//! The bottom bar in scripted runs of the whole app
//! (docs/design/20261028_06_BOTTOM_BAR_UPDATE_PLAN.md, steps 3 and 4), as
//! `drawer_tests` drives it: libmorph's scripted runs at the Galaxy S24's
//! size, the client's events given by the test, and what the app sent read
//! back from [`probe`]. Each run needs a GPU adapter; without one it says
//! so and passes.
//!
//! The app's clock is real time (15 fps), so a 2 s press is about 30
//! frames.

use libcontrols::Rect;
use libmorph::script::{OutDir, Script, ScriptRun};
use libmorph::{GpuError, MorphError};

use flankr_companion::{Event, RemoteId, RemoteInfo};

use super::App;
use super::probe::{self, Probe};
use crate::agent::{AgentHit, BUTTON_H};
use crate::menu;

/// The update on which the sessions are given.
const READY: u32 = 4;
/// Frames before the first touch.
const SETTLE: u32 = 8;
/// Frames for a 2 s press, at 15 fps, and some.
const HOLD: u32 = 36;

fn run(script: &str) -> Option<Probe> {
    run_with(script, None)
}

/// As [`run`], from `settings` as `settings.toml` when given.
fn run_with(script: &str, settings: Option<&str>) -> Option<Probe> {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys");
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    if let Some(text) = settings {
        std::fs::write(data.join("settings.toml"), text).unwrap();
    }
    let run = ScriptRun {
        spec: libmorph::libdevice::GALAXY_S24,
        out: OutDir::Exact(dir.path().join("out")),
        overlay: false,
        fallback_adapter: false,
        args: ["companion", "--keys", keys.to_str().unwrap()]
            .into_iter()
            .chain(["--data", data.to_str().unwrap()])
            .map(String::from)
            .collect(),
    };
    probe::take();
    probe::injected(u32::MAX);
    // Paired and connected, so the overview shows.
    probe::inject(
        READY,
        Event::Remotes(vec![RemoteInfo {
            id: RemoteId([1; 8]),
            name: "m4".into(),
            host: "m4.tail.ts.net:7347".into(),
            fingerprint: "0101 0101".into(),
            paired_at: Some(1_761_600_000),
            active: true,
            online: true,
            fingerprint_hex: String::new(),
            fingerprint_words: String::new(),
            nickname: None,
            display_name: "m4".into(),
        }]),
    );
    probe::inject(
        READY,
        Event::Connected {
            server: "m4".into(),
        },
    );
    probe::inject(READY, Event::Agents(Vec::new()));
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

fn inputs_with(p: &Probe, part: &str) -> usize {
    p.inputs.iter().filter(|i| i.contains(part)).count()
}

/// The overview's bar, from a run that only settles.
fn bar() -> Option<(Rect, Rect, Rect)> {
    let p = run(&format!("frames {SETTLE}\n"))?;
    let at = |hit| {
        p.drawer
            .bar
            .iter()
            .find(|(h, _)| *h == hit)
            .map(|&(_, r)| r)
            .unwrap_or_else(|| panic!("{hit:?} in {:?}", p.drawer.bar))
    };
    Some((
        at(AgentHit::Volume),
        at(AgentHit::GeneralAgent),
        at(AgentHit::Message),
    ))
}

#[test]
fn a_tap_on_the_centre_starts_listening_on_the_release_not_on_touch_down() {
    let Some((_, centre, _)) = bar() else { return };
    let [x, y] = centre.center();
    let down = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(
        inputs_with(&down, "StartGeneralAgentPrompt"),
        0,
        "{:?}",
        down.inputs
    );
    assert!(down.drawer.holding);
    let tap = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes 4\ntouch 5 end {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(inputs_with(&tap, "StartGeneralAgentPrompt(Voice)"), 1);
    assert!(!tap.drawer.holding);
}

#[test]
fn a_2_s_press_on_the_centre_opens_geminis_card_typed_without_listening() {
    let Some((_, centre, _)) = bar() else { return };
    let [x, y] = centre.center();
    let p = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {HOLD}\ntouch 5 end {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(
        inputs_with(&p, "StartGeneralAgentPrompt(Keyboard)"),
        1,
        "{:?}",
        p.inputs
    );
    assert_eq!(inputs_with(&p, "StartGeneralAgentPrompt(Voice)"), 0);
}

#[test]
fn a_2_s_press_on_the_right_opens_the_message_card_typed() {
    let Some((_, _, right)) = bar() else { return };
    let [x, y] = right.center();
    let p = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {HOLD}\ntouch 5 end {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(inputs_with(&p, "StartTextPrompt"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "StartDictation"), 0, "a hold is no tap");
    // Released early: a tap, a dictated message.
    let p = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes 4\ntouch 5 end {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(inputs_with(&p, "StartDictation"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "StartTextPrompt"), 0);
}

#[test]
fn a_move_of_11_pt_cancels_both() {
    let Some((_, centre, right)) = bar() else {
        return;
    };
    let s = centre.w / BUTTON_H;
    for r in [centre, right] {
        let [x, y] = r.center();
        let moved = y - 11.0 * s;
        let p = run(&format!(
            "frames {SETTLE}\ntouch 5 start {x} {y}\nframes 2\ntouch 5 move {x} {moved}\nframes {HOLD}\ntouch 5 end {x} {moved}\nframes 4\n"
        ))
        .unwrap();
        for part in [
            "StartGeneralAgentPrompt",
            "StartTextPrompt",
            "StartDictation",
        ] {
            assert_eq!(inputs_with(&p, part), 0, "{part}: {:?}", p.inputs);
        }
    }
}

#[test]
fn a_tap_on_volume_mutes_and_unmutes_without_its_menu() {
    let Some((volume, _, _)) = bar() else { return };
    let [x, y] = volume.center();
    let tap = format!("touch 5 start {x} {y}\nframes 2\ntouch 5 end {x} {y}\nframes 4\n");
    // Both options on: muting stops what is said and tells the client
    // both are off; no menu opens.
    let on = "speak_coding = true\nspeak_gemini = true\n";
    let p = run_with(&format!("frames {SETTLE}\n{tap}"), Some(on)).unwrap();
    assert!(p.drawer.menu_items.is_empty(), "{:?}", p.drawer.menu_items);
    assert_eq!(inputs_with(&p, "Skip"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "ReadAloud(false)"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "speak_gemini: false"), 1, "{:?}", p.inputs);
    // A second tap unmutes: both come back, and nothing is skipped.
    let p = run_with(&format!("frames {SETTLE}\n{tap}{tap}"), Some(on)).unwrap();
    assert_eq!(inputs_with(&p, "Skip"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "ReadAloud(true)"), 1, "{:?}", p.inputs);
    let last = p
        .inputs
        .iter()
        .rfind(|i| i.starts_with("Settings"))
        .unwrap();
    assert!(last.contains("speak_gemini: true"), "{last}");
    assert!(p.drawer.menu_items.is_empty());
    // Muted in the file, kept across restarts: a tap unmutes.
    let p = run_with(
        &format!("frames {SETTLE}\n{tap}"),
        Some("speak_coding = true\nmuted = true\n"),
    )
    .unwrap();
    assert_eq!(inputs_with(&p, "Skip"), 0, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "ReadAloud(true)"), 1, "{:?}", p.inputs);
}

#[test]
fn a_2_s_press_on_volume_opens_its_menu_of_the_two_options() {
    let Some((volume, _, _)) = bar() else { return };
    let [x, y] = volume.center();
    let p = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {HOLD}\ntouch 5 end {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(
        p.drawer.menu_items,
        [menu::SPEAK_CODING, menu::SPEAK_GEMINI],
        "{:?}",
        p.inputs
    );
    assert_eq!(
        inputs_with(&p, "Skip"),
        0,
        "a hold is no tap: {:?}",
        p.inputs
    );
    // Released early: a tap, which mutes, and no menu.
    let p = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert!(p.drawer.holding, "timed for its hold");
    assert!(p.drawer.menu_items.is_empty());
}
