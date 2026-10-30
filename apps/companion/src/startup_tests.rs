//! *At startup* in scripted runs of the whole app (plan 20261024_02 and
//! plan 20261023_03, their 2026-10-30 15:20 EDT amendments), as
//! `bottom_bar_tests` drives it: libmorph's scripted runs at the Galaxy
//! S24's size, and what the app did read back from [`probe`]. Unlike
//! those, the remotes, their kept agent lists, kept views and last
//! sessions are written before the run into the files the client reads
//! (`keys/` and each remote's `state.redb`), so the client itself gives
//! them; its links reach no daemon (nothing listens at their addresses),
//! so everything shown is from the files. A delivery, or a session gone,
//! is given by the test ([`probe::inject`]). Each run needs a GPU
//! adapter; without one it says so and passes.

use std::path::Path;
use std::time::{Duration, Instant};

use flankr_companion::{
    Event, FileKeyStore, KeptView, KeyStore, LastSession, RemoteState, Remotes, ServerRecord,
    Shows, ViewSection,
};
use libflankr_proto::common::{AgentInfo, Entry, EntryKind, Role, Serial};
use libmorph::script::{OutDir, Script, ScriptRun};
use libmorph::{GpuError, MorphError};

use super::App;
use super::probe::{self, Probe};
use crate::agent::REFRESHING;

/// Frames for the client to load the files and the view to open.
const SETTLE: u32 = 20;
/// The update a test's own event is given on: after the view opened.
const LATER: u32 = 30;

/// An address where nothing listens.
fn closed_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// A remote to write: its key's byte, its kept list's panes, and its last
/// session (pane, shown) if any.
struct Kept {
    key: u8,
    panes: &'static [&'static str],
    last: Option<(&'static str, u64)>,
}

fn agent(pane: &str) -> AgentInfo {
    serde_json::from_value(
        serde_json::json!({"pane_id": pane, "name": "the flankr agent",
        "spoken": "the flankr agent", "agent": "claude", "project": "flankr",
        "tab_label": "17-fl", "tab_number": 17, "status": "idle", "focused": false}),
    )
    .unwrap()
}

/// The kept view of `pane`: one reply.
fn kept_view(pane: &str) -> KeptView {
    serde_json::from_value(serde_json::json!({
        "pane_id": pane, "session_id": "s1", "status": "idle",
        "sections": [{"section": {"serial": "3fa9c21e:1.0", "kind": "text", "entries": [
            {"ts": "1", "role": "assistant", "kind": "text", "summary": "Kept from the last run."}]}}]}))
    .unwrap()
}

/// Writes the remotes (the first in use) into `keys`, and each one's kept
/// list, view and last session into its `state.redb` in `data`.
fn write_remotes(keys: &Path, data: &Path, remotes: &[Kept]) -> Vec<ServerRecord> {
    let mut store = FileKeyStore::new(keys);
    let records: Vec<ServerRecord> = remotes
        .iter()
        .map(|k| {
            let mut r = ServerRecord::new(format!("m{}", k.key), [k.key; 32], vec![closed_addr()]);
            r.paired_at = Some(1_761_600_000 + u64::from(k.key));
            store.generate_static_key(&r.key_name).unwrap();
            r
        })
        .collect();
    store
        .set_remotes(&Remotes {
            active: records.first().map(|r| r.id),
            list: records.clone(),
        })
        .unwrap();
    for (k, r) in remotes.iter().zip(&records) {
        let state = RemoteState::open(data, r).unwrap();
        let list: Vec<AgentInfo> = k.panes.iter().map(|p| agent(p)).collect();
        state.set_agents(&list).unwrap();
        if let Some((pane, shown)) = k.last {
            state.set_view(&kept_view(pane)).unwrap();
            state
                .set_last_session(Some(&LastSession {
                    pane_id: pane.into(),
                    shown,
                }))
                .unwrap();
        }
    }
    records
}

/// Runs `script` with `remotes` written and `settings` as
/// `settings.toml`, `events` given on [`LATER`]; the probe, and the data
/// directory (kept until the returned guard goes).
fn run(
    script: &str,
    settings: &str,
    remotes: &[Kept],
    events: Vec<Event>,
) -> Option<(Probe, tempfile::TempDir, Vec<ServerRecord>)> {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys");
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.toml"), settings).unwrap();
    let records = write_remotes(&keys, &data, remotes);
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
    probe::speaking(u32::MAX);
    for event in events {
        probe::inject(LATER, event);
    }
    let script = Script::parse(script).unwrap();
    match libmorph::script::run::<App>(&script, run) {
        Err(MorphError::Gpu(GpuError::Adapter(error))) => {
            eprintln!("skipped: no GPU adapter ({error})");
            None
        }
        result => {
            result.unwrap();
            Some((probe::take(), dir, records))
        }
    }
}

const LAST_SESSION: &str = "startup = \"last_session\"\n";

fn inputs_with(p: &Probe, part: &str) -> Vec<usize> {
    p.inputs
        .iter()
        .enumerate()
        .filter(|(_, i)| i.contains(part))
        .map(|(n, _)| n)
        .collect()
}

/// The view as the client gives it after its first delivery: no longer
/// cached, or its session gone.
fn view(cached: bool, ended: bool) -> Event {
    let sections = if ended {
        Vec::new()
    } else {
        vec![ViewSection {
            index: 0,
            serial: Serial::new(0x3fa9_c21e, 1, 0),
            shows: Shows::Entries(vec![Entry {
                ts: "1".into(),
                role: Role::Assistant,
                kind: EntryKind::Text,
                tool: None,
                summary: "Kept from the last run.".into(),
                links: Vec::new(),
                origin: None,
                text: None,
            }]),
        }]
    };
    let changed = sections.iter().map(|s| s.serial).collect();
    Event::AgentView {
        name: "the flankr agent".into(),
        status: "idle".into(),
        sections,
        pinned: None,
        changed,
        loading: false,
        typed: None,
        loading_earlier: false,
        start: false,
        terminal_on: None,
        new_content: false,
        held_end: Vec::new(),
        held_newer: Vec::new(),
        bookends: Vec::new(),
        child: None,
        children: Vec::new(),
        cached,
        ended,
    }
}

#[test]
fn last_session_opens_the_agent_view_from_the_cache_refreshing_until_the_first_delivery() {
    let remotes = [Kept {
        key: 1,
        panes: &["w2:pZ"],
        last: Some(("w2:pZ", 1_700_000_000)),
    }];
    // Up to the first delivery: the kept view, refreshing.
    let Some((p, _dir, _)) = run(
        &format!("frames {SETTLE}\n"),
        LAST_SESSION,
        &remotes,
        Vec::new(),
    ) else {
        return;
    };
    assert_eq!(
        inputs_with(&p, "OpenAgent(\"w2:pZ\")").len(),
        1,
        "{:?}",
        p.inputs
    );
    assert!(
        inputs_with(&p, "SwitchRemote").is_empty(),
        "the remote in use"
    );
    assert_eq!(p.drawer.agent_pane.as_deref(), Some("w2:pZ"));
    assert!(p.drawer.agent_cached, "the kept view, from the client");
    assert!(!p.drawer.agent_ended);
    assert_eq!(
        p.pull_lines.last(),
        Some(&Some(REFRESHING.to_string())),
        "{:?}",
        p.pull_lines
    );
    // The first delivery: the indicator goes.
    let (p, _dir, _) = run(
        &format!("frames {}\n", LATER + 10),
        LAST_SESSION,
        &remotes,
        vec![view(false, false)],
    )
    .unwrap();
    assert!(!p.drawer.agent_cached);
    assert!(
        p.pull_lines.contains(&Some(REFRESHING.to_string())),
        "{:?}",
        p.pull_lines
    );
    assert_eq!(p.pull_lines.last(), Some(&None), "{:?}", p.pull_lines);
    assert_eq!(p.drawer.agent_pane.as_deref(), Some("w2:pZ"));

    // With *Overview*, the default, nothing opens.
    let (p, _dir, _) = run(&format!("frames {SETTLE}\n"), "", &remotes, Vec::new()).unwrap();
    assert!(inputs_with(&p, "OpenAgent").is_empty(), "{:?}", p.inputs);
    assert_eq!(p.drawer.agent_pane, None);
}

#[test]
fn last_session_falls_back_to_the_most_recent_other_remote() {
    // The remote in use's last session is no longer listed; of the others,
    // the most recently shown is opened, its remote coming into use.
    let remotes = [
        Kept {
            key: 1,
            panes: &["w1:pA"],
            last: Some(("w1:pGone", 1_700_000_500)),
        },
        Kept {
            key: 2,
            panes: &["w3:pB"],
            last: Some(("w3:pB", 1_700_000_200)),
        },
        Kept {
            key: 3,
            panes: &["w4:pC"],
            last: Some(("w4:pC", 1_700_000_100)),
        },
    ];
    let Some((p, _dir, records)) = run(
        &format!("frames {SETTLE}\n"),
        LAST_SESSION,
        &remotes,
        Vec::new(),
    ) else {
        return;
    };
    let switch = inputs_with(&p, &format!("SwitchRemote({:?})", records[1].id));
    let open = inputs_with(&p, "OpenAgent(\"w3:pB\")");
    assert_eq!(switch.len(), 1, "{:?}", p.inputs);
    assert_eq!(open.len(), 1, "{:?}", p.inputs);
    assert!(switch[0] < open[0], "switched, then opened");
    assert!(inputs_with(&p, "OpenAgent(\"w1").is_empty());
    assert!(inputs_with(&p, "OpenAgent(\"w4").is_empty());
    assert_eq!(p.drawer.agent_pane.as_deref(), Some("w3:pB"));
    assert!(p.drawer.agent_cached, "b's kept view");
}

#[test]
fn last_session_falls_back_to_the_overview() {
    // No last session kept anywhere, or none listed: the overview.
    let remotes = [
        Kept {
            key: 1,
            panes: &["w1:pA"],
            last: None,
        },
        Kept {
            key: 2,
            panes: &["w3:pB"],
            last: Some(("w3:pGone", 1_700_000_200)),
        },
    ];
    let Some((p, _dir, _)) = run(
        &format!("frames {SETTLE}\n"),
        LAST_SESSION,
        &remotes,
        Vec::new(),
    ) else {
        return;
    };
    assert!(inputs_with(&p, "OpenAgent").is_empty(), "{:?}", p.inputs);
    assert!(inputs_with(&p, "SwitchRemote").is_empty(), "{:?}", p.inputs);
    assert_eq!(p.drawer.agent_pane, None);
}

#[test]
fn a_session_gone_says_it_has_ended_and_its_button_goes_back_to_the_overview() {
    let remotes = [Kept {
        key: 1,
        panes: &["w2:pZ"],
        last: Some(("w2:pZ", 1_700_000_000)),
    }];
    let Some((p, _dir, _)) = run(
        &format!("frames {}\n", LATER + 6),
        LAST_SESSION,
        &remotes,
        vec![view(false, true)],
    ) else {
        return;
    };
    assert!(p.drawer.agent_ended);
    assert!(!p.drawer.agent_cached);
    assert_eq!(p.pull_lines.last(), Some(&None), "no refreshing line");
    let button = p.drawer.ended_button.expect("Back to the overview");
    let [x, y] = button.center();
    let (p, dir, records) = run(
        &format!(
            "frames {}\ntouch 1 start {x} {y}\nframes 2\ntouch 1 end {x} {y}\nframes 6\n",
            LATER + 6
        ),
        LAST_SESSION,
        &remotes,
        vec![view(false, true)],
    )
    .unwrap();
    assert_eq!(inputs_with(&p, "CloseAgent").len(), 1, "{:?}", p.inputs);
    assert_eq!(p.drawer.agent_pane, None, "back on the overview");
    // Closed back to the overview: the remote's last session is gone, so
    // the overview opens next time.
    let data = dir.path().join("data");
    let deadline = Instant::now() + Duration::from_secs(5);
    let state = loop {
        match RemoteState::open(&data, &records[0]) {
            Ok(state) => break state,
            Err(e) => {
                assert!(Instant::now() < deadline, "the client still holds it: {e}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert_eq!(state.last_session().unwrap(), None);
}
