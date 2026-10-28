//! The pairing window's camera in scripted runs of the whole app
//! (docs/design/20261028_02_PAIRING_QR_SCAN_PLAN.md, section 5): libmorph's
//! scripted camera (`camera-qr`, `camera-deny`, `camera-deny-forever`,
//! `tap`, `touch`, `suspend`, `resume`) drives [`App`] at the Galaxy S24's
//! size, started with `--camera` to have the Android window on the host.
//! What the app did is read back from [`probe`]: the camera events in order
//! and the window's texts. The scripted viewfinder draws nothing (libmorph
//! gives scripted runs no preview frames), so these check state and
//! controls, not the image. Each run needs a GPU adapter; without one it
//! says so and passes.

use std::sync::mpsc;
use std::time::Duration;

use libflankr_proto::link::{
    self, Incoming, OFFER_LEN, PairAck, PairRequest, PairResult, PairingPayload, StaticKeypair,
};
use libmorph::script::{OutDir, Script, ScriptRun};
use libmorph::{GpuError, MorphError};

use super::App;
use super::probe::{self, Probe};
use crate::ui::Hit;

/// Runs `script` on a fresh app with `args`; `None` without a GPU. Its
/// keys are files in a directory of its own (`--keys`), so the host's key
/// store, and any remote paired there, is neither read nor changed.
fn run(script: &str, args: &[&str]) -> Option<Probe> {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys");
    let run = ScriptRun {
        spec: libmorph::libdevice::GALAXY_S24,
        out: OutDir::Exact(dir.path().join("out")),
        overlay: false,
        fallback_adapter: false,
        args: std::iter::once("companion")
            .chain(args.iter().copied())
            .chain(["--keys", keys.to_str().unwrap()])
            .map(String::from)
            .collect(),
    };
    probe::take();
    let script = Script::parse(script).unwrap();
    match libmorph::script::run::<App>(&script, run) {
        Err(MorphError::Gpu(GpuError::Adapter(error))) => {
            eprintln!("skipped: no GPU adapter ({error})");
            None
        }
        result => {
            result.unwrap();
            Some(probe::take())
        }
    }
}

/// The window as it opens with the camera, for its controls' places.
fn opened() -> Option<Probe> {
    run("frames 2\n", &["--camera"])
}

fn tap((x, y): (f32, f32)) -> String {
    format!("tap {x} {y}\n")
}

/// The camera events of a run, without the others.
fn camera(p: &Probe) -> Vec<&str> {
    p.log
        .iter()
        .filter_map(|l| l.strip_prefix("camera "))
        .collect()
}

/// Run 1: the placeholder, and no camera until the tap; then a code that
/// is not a pairing link is ignored while the scan goes on.
#[test]
fn the_window_waits_for_a_tap_then_ignores_other_codes() {
    let Some(first) = opened() else { return };
    assert!(first.shows("Tap to scan"), "{:?}", first.texts);
    assert!(first.shows("Scan the QR code"), "{:?}", first.texts);
    assert!(first.shows("Paste a link"));
    assert!(!first.shows("paste a pairing URL"), "no field yet");
    assert!(!first.shows("not paired"), "no status lines");
    assert!(first.log.is_empty(), "nothing started: {:?}", first.log);
    let area = first.at(Hit::Area);
    let script = format!(
        "frames 2\n{}frames 1\ncamera-qr https://example.com\nframes 1\n",
        tap(area)
    );
    let p = run(&script, &["--camera"]).unwrap();
    assert_eq!(camera(&p), ["opened 1", "qr 1 19 bytes"], "{:?}", p.log);
    assert_eq!(p.state, "Running(ScanId(1))");
    assert!(p.shows("Not a flankr pairing code."), "{:?}", p.texts);
    assert!(!p.shows("Tap to scan"));
}

/// What the test daemon does with a pairing request once its words are
/// out: holds it until the phone closes it (the owner has not typed yes),
/// adds the phone (yes), or refuses it (no).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Answer {
    Hold,
    Add,
    Refuse,
}

/// The test daemon in pair mode: a `flankr pair` run on 127.0.0.1, as
/// flankr-companion's tests/link.rs builds one. It hands each pairing
/// request's words to the test, then answers it as `answer` says.
struct Daemon {
    url: String,
    /// `127.0.0.1:<port>`.
    addr: String,
    held: mpsc::Receiver<String>,
}

impl Daemon {
    fn start() -> Daemon {
        Daemon::answering(Answer::Hold)
    }

    fn answering(answer: Answer) -> Daemon {
        let key = link::generate_keypair().unwrap();
        let offer = link::generate_offer().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let url = PairingPayload {
            server_key: key.public,
            addresses: vec![addr.clone()],
            offer: Some(offer),
        }
        .url()
        .unwrap();
        let (held_tx, held) = mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                while let Ok((stream, _)) = listener.accept().await {
                    let (key, held) = (key.clone(), held_tx.clone());
                    tokio::spawn(serve(stream, key, offer, held, answer));
                }
            });
        });
        Daemon { url, addr, held }
    }
}

/// One connection: a pairing request with the right offer is held (its
/// words go to the test) until the phone closes it; anything else is
/// dropped.
async fn serve(
    stream: tokio::net::TcpStream,
    key: StaticKeypair,
    offer: [u8; OFFER_LEN],
    held: mpsc::Sender<String>,
    answer: Answer,
) {
    let Ok(Incoming::Pair(pair)) = link::accept_incoming(stream, &key, |_| false).await else {
        return;
    };
    let request: PairRequest = match &pair.request {
        Ok(request) if request.offer == offer => request.clone(),
        _ => return,
    };
    let Ok(mut stream) = pair
        .respond(PairAck::Accepted {
            expires_in_secs: 60,
        })
        .await
    else {
        return;
    };
    let words = link::pairing_words_text(&key.public, &offer, &request.device_key);
    let _ = held.send(words);
    let result = match answer {
        // Held, as the owner has not typed yes; until the phone closes it.
        Answer::Hold => {
            let _ = stream.recv_message().await;
            PairResult::Refused
        }
        Answer::Add => PairResult::Added,
        Answer::Refuse => PairResult::Refused,
    };
    let _ = link::send_pair_result(&mut stream, result).await;
    // Kept open until the phone has read the result.
    let _ = stream.recv_message().await;
}

/// Scans the daemon's code: a tap on the area, the code, and `frames`.
fn scan(first: &Probe, daemon: &Daemon, frames: u32) -> String {
    format!(
        "frames 2\n{}frames 1\ncamera-qr {}\nframes {frames}\n",
        tap(first.at(Hit::Area)),
        daemon.url
    )
}

/// Run 2: a real pairing URL from a daemon in pair mode, scanned: the scan
/// stops, the window says the code was read and where it connects, then
/// the six words the daemon has show in a modal.
#[test]
fn a_scanned_pairing_url_stops_the_scan_and_shows_the_daemons_words() {
    let Some(first) = opened() else { return };
    let daemon = Daemon::start();
    let script = format!(
        "frames 2\n{}frames 1\ncamera-qr {}\nframes 45\n",
        tap(first.at(Hit::Area)),
        daemon.url
    );
    let p = run(&script, &["--camera"]).unwrap();
    let words = daemon
        .held
        .recv_timeout(Duration::from_secs(5))
        .expect("the daemon held a pairing request");
    let qr = format!("qr 1 {} bytes", daemon.url.len());
    assert_eq!(
        camera(&p),
        ["opened 1", qr.as_str(), "closed 1"],
        "{:?}",
        p.log
    );
    assert_eq!(p.state, "Idle");
    let words: Vec<&str> = words.split_whitespace().collect();
    assert_eq!(words.len(), 6);
    let (one, two) = (words[..3].join(" "), words[3..].join(" "));
    assert!(
        p.texts.contains(&one) && p.texts.contains(&two),
        "the daemon's words {words:?} on {:?}",
        p.texts
    );
    assert!(p.shows("If flankr pair on 127.0.0.1"), "{:?}", p.texts);
    assert!(p.shows("Details") && p.shows("Cancel"));
    // The camera is not shown again during the pairing.
    assert!(!p.shows("Tap to scan") && !p.shows("Paste a link"));
    // The words are in the modal, titled, with Details and Cancel only.
    assert!(p.shows("Confirm on your Mac"), "{:?}", p.texts);
    let hits: Vec<Hit> = p.hits.iter().map(|&(h, _)| h).collect();
    assert_eq!(hits, [Hit::Details, Hit::Cancel]);

    // Cancel in the modal: the pairing ends, and the window says so with
    // Tap to scan.
    let script = format!(
        "frames 2\n{}frames 1\ncamera-qr {}\nframes 45\n{}frames 2\n",
        tap(first.at(Hit::Area)),
        daemon.url,
        tap(p.at(Hit::Cancel))
    );
    let c = run(&script, &["--camera"]).unwrap();
    assert!(
        c.shows("Pairing cancelled: nothing was added."),
        "{:?}",
        c.texts
    );
    assert!(c.shows("Tap to scan") && !c.shows("Confirm on your Mac"));
}

/// A scanned code whose daemon has not answered yet (a port that takes
/// the connection and says nothing): the camera stops and the window says
/// the code was read and where it is connecting, with Cancel, which ends it.
#[test]
fn a_scanned_code_shows_read_and_connecting_until_the_daemon_answers() {
    let Some(first) = opened() else { return };
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = PairingPayload {
        server_key: link::generate_keypair().unwrap().public,
        addresses: vec![silent.local_addr().unwrap().to_string()],
        offer: Some(link::generate_offer().unwrap()),
    }
    .url()
    .unwrap();
    let script = format!(
        "frames 2\n{}frames 1\ncamera-qr {url}\nframes 4\n",
        tap(first.at(Hit::Area))
    );
    let p = run(&script, &["--camera"]).unwrap();
    assert_eq!(p.state, "Idle", "the camera stopped");
    assert!(p.shows("Code read"), "{:?}", p.texts);
    assert!(p.shows("Connecting to 127.0.0.1\u{2026}"), "{:?}", p.texts);
    assert!(!p.shows("pairing\u{2026}") && !p.shows("Tap to scan"));
    let hits: Vec<Hit> = p.hits.iter().map(|&(h, _)| h).collect();
    assert_eq!(hits, [Hit::Cancel], "no Tap to scan on the check");
    let script = format!("{script}{}frames 2\n", tap(p.at(Hit::Cancel)));
    let c = run(&script, &["--camera"]).unwrap();
    assert!(
        c.shows("Pairing cancelled: nothing was added."),
        "{:?}",
        c.texts
    );
    assert!(c.shows("Tap to scan"));
    drop(silent);
}

/// The owner types yes: the modal closes, and the window says it is
/// paired, with the address and the fingerprint, until Done, which goes on
/// to the overview.
#[test]
fn a_confirmed_pairing_shows_paired_until_done() {
    let Some(first) = opened() else { return };
    let daemon = Daemon::answering(Answer::Add);
    let p = run(&scan(&first, &daemon, 60), &["--camera"]).unwrap();
    daemon
        .held
        .recv_timeout(Duration::from_secs(5))
        .expect("the daemon had a pairing request");
    assert!(p.shows("Paired with 127.0.0.1"), "{:?}", p.texts);
    let reached = format!("{} \u{b7} fingerprint ", daemon.addr);
    assert!(p.shows(&reached), "{:?}", p.texts);
    assert!(!p.shows("Confirm on your Mac"));
    let hits: Vec<Hit> = p.hits.iter().map(|&(h, _)| h).collect();
    assert_eq!(hits, [Hit::Details, Hit::Done], "no Tap to scan");
    // Still there many frames later: it waits for Done.
    let script = format!(
        "{}{}frames 2\n",
        scan(&first, &daemon, 60),
        tap(p.at(Hit::Details))
    );
    let d = run(&script, &["--camera"]).unwrap();
    assert!(d.shows("Paired with 127.0.0.1"), "{:?}", d.texts);
    assert!(d.shows("Daemon fingerprint"), "{:?}", d.texts);
    assert!(d.shows("BLAKE2s-256 fingerprint"), "{:?}", d.texts);
    assert!(d.shows("Hide details"));
    // Done: on to the overview.
    let script = format!(
        "{}{}frames 2\n",
        scan(&first, &daemon, 60),
        tap(p.at(Hit::Done))
    );
    let done = run(&script, &["--camera"]).unwrap();
    assert!(
        done.log.iter().any(|l| l == "pairing done, overview"),
        "{:?}",
        done.log
    );
}

/// The owner types no: the modal closes and the window keeps the reason,
/// with Tap to scan again.
#[test]
fn a_refused_pairing_keeps_the_window_with_its_reason() {
    let Some(first) = opened() else { return };
    let daemon = Daemon::answering(Answer::Refuse);
    let p = run(&scan(&first, &daemon, 60), &["--camera"]).unwrap();
    assert!(
        p.shows("not confirmed on 127.0.0.1: nothing was added"),
        "{:?}",
        p.texts
    );
    assert!(p.shows("Tap to scan again"), "{:?}", p.texts);
    assert!(!p.shows("Confirm on your Mac") && !p.shows("Paired with"));
    assert!(p.hits.iter().any(|(h, _)| *h == Hit::Area));
}

/// Runs 3 and 4: refusals, with the reason, Open Settings only when
/// Android will not ask again, and a tap asking again.
#[test]
fn a_refusal_shows_why_and_open_settings_only_when_android_wont_ask() {
    let Some(first) = opened() else { return };
    let area = tap(first.at(Hit::Area));
    let script = format!("frames 2\ncamera-deny\n{area}frames 1\n");
    let p = run(&script, &["--camera"]).unwrap();
    assert_eq!(camera(&p), ["denied 1 settings_only=false"]);
    assert!(p.shows("Camera permission was refused"), "{:?}", p.texts);
    assert!(p.shows("Tap to scan"));
    assert!(!p.shows("Open Settings"));
    let script = format!("frames 2\ncamera-deny\n{area}frames 1\n{area}frames 1\n");
    let p = run(&script, &["--camera"]).unwrap();
    assert_eq!(
        camera(&p),
        ["denied 1 settings_only=false", "opened 2"],
        "a tap asks again"
    );
    let script = format!("frames 2\ncamera-deny-forever\n{area}frames 1\n");
    let p = run(&script, &["--camera"]).unwrap();
    assert_eq!(camera(&p), ["denied 1 settings_only=true"]);
    assert!(p.shows("Camera permission is refused"), "{:?}", p.texts);
    assert!(p.shows("Open Settings"));
    // Open Settings: libmorph has no settings page off Android.
    let script = format!(
        "frames 2\ncamera-deny-forever\n{area}frames 1\n{}frames 1\n",
        tap(p.at(Hit::OpenSettings))
    );
    let p = run(&script, &["--camera"]).unwrap();
    assert!(
        p.log
            .iter()
            .any(|l| l == "open_app_settings Err(Unsupported)"),
        "{:?}",
        p.log
    );
}

/// Run 5: two fingers moving apart zoom in, through `zoom_by`, and the
/// ratio shows.
#[test]
fn a_pinch_zooms_the_scan() {
    let Some(first) = opened() else { return };
    let script = format!(
        "frames 2\n{}frames 1\n\
         touch 1 start 400 1000\ntouch 2 start 600 1000\nframes 1\n\
         touch 2 move 650 1000\nframes 1\ntouch 2 move 700 1000\nframes 1\n\
         touch 1 end 400 1000\ntouch 2 end 700 1000\nframes 1\n",
        tap(first.at(Hit::Area))
    );
    let p = run(&script, &["--camera"]).unwrap();
    let zooms: Vec<f32> = camera(&p)
        .iter()
        .filter_map(|l| l.strip_prefix("zoom 1 "))
        .map(|l| {
            assert!(l.ends_with(" app"), "{l}");
            l.split(' ').next().unwrap().parse().unwrap()
        })
        .collect();
    assert_eq!(zooms.len(), 2, "{:?}", p.log);
    assert!(zooms[0] > 1.0 && zooms[1] > zooms[0], "{zooms:?}");
    assert!((zooms[1] - 1.5).abs() < 0.01, "300 / 200 apart: {zooms:?}");
    assert!(p.shows("1.5\u{d7}"), "{:?}", p.texts);
    // The pinch's touches were no tap: the scan still runs.
    assert_eq!(p.state, "Running(ScanId(1))");
}

/// Run 6: the scan keeps running across a suspend; Back on the first-run
/// window stops it and returns to the placeholder; Paste a link stops it
/// and shows the field.
#[test]
fn the_scan_survives_a_suspend_and_back_or_paste_stops_it() {
    let Some(first) = opened() else { return };
    let area = tap(first.at(Hit::Area));
    let script = format!("frames 2\n{area}frames 1\nsuspend\nresume\nframes 2\n");
    let p = run(&script, &["--camera"]).unwrap();
    assert_eq!(camera(&p), ["opened 1", "suspended 1", "opened 1"]);
    assert_eq!(p.state, "Running(ScanId(1))");
    assert!(p.shows("Point at the QR code"));
    let script = format!("frames 2\n{area}frames 1\nkey back\nframes 1\n");
    let p = run(&script, &["--camera"]).unwrap();
    assert_eq!(camera(&p), ["opened 1", "closed 1"]);
    assert_eq!(p.state, "Idle");
    assert!(p.shows("Tap to scan"));
    let script = format!(
        "frames 2\n{area}frames 1\n{}frames 1\n",
        tap(first.at(Hit::PasteLink))
    );
    let p = run(&script, &["--camera"]).unwrap();
    assert_eq!(camera(&p), ["opened 1", "closed 1"]);
    assert!(
        p.shows("paste a pairing URL") && p.shows("Pair"),
        "{:?}",
        p.texts
    );
    assert!(p.shows("Tap to scan"), "the placeholder over the field");
}

/// Run 7: without `--camera` (the desktop and the web), today's field and
/// Pair, and nothing of the camera.
#[test]
fn without_a_camera_the_window_is_the_paste_screen() {
    let Some(p) = run("frames 2\n", &[]) else {
        return;
    };
    assert!(p.shows("paste a pairing URL"), "{:?}", p.texts);
    assert!(p.texts.iter().any(|t| t == "Pair"));
    assert!(!p.shows("Tap to scan") && !p.shows("Paste a link"));
    assert!(p.shows("not paired"), "the status lines");
    assert!(!p.hits.iter().any(|(h, _)| *h == Hit::Area));
    assert!(p.log.is_empty());
}
