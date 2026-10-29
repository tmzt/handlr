//! Subagents in scripted runs of the whole app
//! (docs/design/20261023_07_SUBAGENTS_PLAN.md, step 5), as `drawer_tests`
//! drives it: libmorph's scripted runs at the Galaxy S24's size, the
//! client's events given by the test ([`probe::inject`]) instead of a
//! daemon, and what the app did read back from [`probe`]. Each run needs a
//! GPU adapter; without one it says so and passes.
//!
//! The view of p1 is opened from the session drawer; its `Event::AgentView`
//! comes with three subagents and the parent's `Agent` call for one.

use flankr_companion::{Event, RemoteId, RemoteInfo, Shows, ViewSection};
use libcontrols::Rect;
use libflankr_proto::common::{AgentInfo, ChildInfo, Entry, EntryKind, Role, Serial, Turn};
use libmorph::script::{OutDir, Script, ScriptRun};
use libmorph::{GpuError, MorphError};

use super::App;
use super::probe::{self, Probe};
use crate::agent::subagent_menu;

const READY: u32 = 4;
const SETTLE: u32 = 8;
/// Frames for the drawer's slide and more.
const SLIDE: u32 = 10;

fn remote() -> RemoteInfo {
    RemoteInfo {
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
    }
}

fn now() -> u64 {
    subagent_menu::now()
}

fn child(id: &str, short: &str, description: &str, finished: bool, minutes: u64) -> ChildInfo {
    ChildInfo {
        id: id.into(),
        short: short.into(),
        description: description.into(),
        kind: "general-purpose".into(),
        background: true,
        finished,
        started: now() - minutes * 60,
        started_by: None,
        turn: None,
    }
}

fn children() -> Vec<ChildInfo> {
    vec![
        child(
            "a1",
            "overview screen",
            "Build the overview screen plan",
            false,
            12,
        ),
        child("b2", "spoken summaries", "Spoken summaries", false, 2),
        child(
            "c3",
            "libtypedmd licences",
            "libtypedmd licences and README",
            true,
            30,
        ),
    ]
}

fn agent() -> AgentInfo {
    AgentInfo {
        pane_id: "p1".into(),
        name: Some("Claude p1".into()),
        spoken: None,
        agent: "claude".into(),
        project: Some("flankr".into()),
        cwd: None,
        workspace_id: Some("w1".into()),
        tab_id: None,
        tab_label: Some("1-fl".into()),
        tab_number: Some(1),
        status: "working".into(),
        focused: false,
        session: None,
        turn: Some(Turn::Working {
            line: "Agent overview screen".into(),
        }),
        idle_ms: None,
        subagents_running: Some(2),
        summary: None,
        children: children(),
    }
}

fn entry(role: Role, kind: EntryKind, tool: Option<&str>, text: &str) -> Entry {
    Entry {
        ts: "2026-10-29T06:00:00Z".into(),
        role,
        kind,
        tool: tool.map(str::to_string),
        summary: text.chars().take(80).collect(),
        links: Vec::new(),
        origin: None,
        text: Some(text.into()),
    }
}

/// The view as the client gives it: a user turn, a reply and the
/// parent's `Agent` call for a1; the subagent `child` shown, if any.
fn view(child: Option<&str>) -> Event {
    let input = serde_json::json!({
        "description": "Build the overview screen plan",
        "prompt": "Read the plan and build it.",
        "subagent_type": "general-purpose",
    });
    let entries = [
        entry(Role::User, EntryKind::Text, None, "build the overview"),
        entry(
            Role::Assistant,
            EntryKind::Text,
            None,
            "Starting a subagent.",
        ),
        entry(
            Role::Assistant,
            EntryKind::ToolUse,
            Some("Agent"),
            &format!("Agent {input}"),
        ),
    ];
    let sections: Vec<ViewSection> = entries
        .into_iter()
        .enumerate()
        .map(|(i, e)| ViewSection {
            index: i as u64,
            serial: Serial::new(7, 2 * i as u32 + 2, 0),
            shows: Shows::Entries(vec![e]),
        })
        .collect();
    let changed = sections.iter().map(|s| s.serial).collect();
    Event::AgentView {
        name: "Claude p1".into(),
        status: "working".into(),
        sections,
        pinned: None,
        changed,
        loading: false,
        typed: None,
        loading_earlier: false,
        start: true,
        terminal_on: None,
        new_content: false,
        held_end: Vec::new(),
        held_newer: Vec::new(),
        bookends: Vec::new(),
        child: child.map(str::to_string),
        children: children(),
    }
}

fn run(script: &str) -> Option<Probe> {
    let dir = tempfile::tempdir().unwrap();
    let keys = dir.path().join("keys");
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
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
    probe::inject(READY, Event::Remotes(vec![remote()]));
    probe::inject(
        READY,
        Event::Connected {
            server: "m4".into(),
        },
    );
    probe::inject(READY, Event::Agents(vec![agent()]));
    probe::inject_after("OpenAgent", view(None));
    probe::inject_after("OpenChild", view(Some("a1")));
    probe::inject_after("CloseChild", view(None));
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

fn tap(r: Rect) -> String {
    let [x, y] = r.center();
    format!("tap {x} {y}\n")
}

fn icon(p: &Probe, id: &str) -> Option<Rect> {
    p.drawer
        .title_icons
        .iter()
        .find(|(i, _)| i == id)
        .map(|&(_, r)| r)
}

fn inputs_with(p: &Probe, part: &str) -> usize {
    p.inputs.iter().filter(|i| i.contains(part)).count()
}

/// The script that opens p1's view from the drawer, and the probe after
/// it.
fn open_view() -> Option<(String, Probe)> {
    let at = run(&format!("frames {SETTLE}\n"))?;
    assert!(
        icon(&at, "app/agent_menu").is_none(),
        "no ⊙ on the overview"
    );
    let script = format!(
        "frames {SETTLE}\n{}frames {SLIDE}\n{}frames {SLIDE}\n",
        tap(at.drawer.menu.expect("the overview's ≡")),
        tap(*at.drawer.rows.first().expect("p1's row")),
    );
    let p = run(&script)?;
    assert_eq!(p.drawer.agent_pane.as_deref(), Some("p1"), "{:?}", p.inputs);
    Some((script, p))
}

#[test]
fn the_agent_menu_is_first_of_the_title_icons_and_opens_a_subagent() {
    let Some((open, at)) = open_view() else {
        return;
    };
    // The ⊙ is the first title icon, left of Settings.
    let ids: Vec<&str> = at
        .drawer
        .title_icons
        .iter()
        .map(|(i, _)| i.as_str())
        .collect();
    assert_eq!(ids.first(), Some(&"app/agent_menu"), "{ids:?}");
    assert_eq!(ids.last(), Some(&crate::menu::SETTINGS));
    let dot = icon(&at, "app/agent_menu").unwrap();
    // Its menu: main, Running, the two running, Finished, the finished.
    let script = format!("{open}{}frames 3\n", tap(dot));
    let menu = run(&script).unwrap();
    assert_eq!(
        menu.drawer.menu_items,
        [
            subagent_menu::MAIN.to_string(),
            String::new(),
            "app/agent/child/a1".into(),
            "app/agent/child/b2".into(),
            String::new(),
            "app/agent/child/c3".into(),
        ],
        "{:?}",
        menu.drawer
    );
    let rects = &menu.drawer.menu_rects;
    assert!(rects.len() >= 3, "{rects:?}");
    assert!(rects[0].y >= dot.y + dot.h, "below the ⊙");
    // Choosing a1 opens it; the header names it after its parent.
    let script = format!("{script}{}frames 4\n", tap(rects[2]));
    let p = run(&script).unwrap();
    assert_eq!(inputs_with(&p, "OpenChild(\"a1\")"), 1, "{:?}", p.inputs);
    assert_eq!(p.drawer.agent_child.as_deref(), Some("a1"));
    assert_eq!(
        p.drawer.agent_title.as_deref(),
        Some("Claude p1 \u{203a} Build the overview screen plan")
    );
    // The system Back returns to the parent, still in its view.
    let script = format!("{script}key back\nframes 4\n");
    let p = run(&script).unwrap();
    assert_eq!(inputs_with(&p, "CloseChild"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "CloseAgent"), 0);
    assert_eq!(p.drawer.agent_child, None);
    assert_eq!(p.drawer.agent_pane.as_deref(), Some("p1"));
}

#[test]
fn the_link_in_the_flow_opens_its_subagent() {
    let Some((open, at)) = open_view() else {
        return;
    };
    let &(i, link) = at
        .drawer
        .child_links
        .first()
        .unwrap_or_else(|| panic!("no link: {:?}", at.drawer));
    assert_eq!(i, 0, "a1's");
    let p = run(&format!("{open}{}frames 4\n", tap(link))).unwrap();
    assert_eq!(inputs_with(&p, "OpenChild(\"a1\")"), 1, "{:?}", p.inputs);
    assert_eq!(p.drawer.agent_child.as_deref(), Some("a1"));
}
