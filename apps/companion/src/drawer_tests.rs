//! The session drawer in scripted runs of the whole app
//! (docs/design/20261028_03_SESSION_DRAWER_PLAN.md, section 5, "App
//! (host)", and steps 7-8): libmorph's scripted runs drive [`App`] at the
//! Galaxy S24's size, with the client's events given by the test
//! ([`probe::inject`]) instead of a daemon. What the app did is read back
//! from [`probe`]: the screen shown and the drawer's layer plan on every
//! frame, the inputs it sent the client, and the drawer's places. Each run
//! needs a GPU adapter; without one it says so and passes.
//!
//! The app's clock is real time (it holds each frame to 15 fps), so a
//! 2 s press is about 30 frames.

use libcontrols::Rect;
use libflankr_proto::common::{AgentInfo, Turn};
use libmorph::script::{OutDir, Script, ScriptRun};
use libmorph::{GpuError, MorphError};

use flankr_companion::{Dictation, DictationPhase, Event, PromptMode, RemoteId, RemoteInfo};

use super::probe::{self, Probe};
use super::{App, Shown};

/// The update on which the remotes and the sessions are given: after the
/// client's own first events (an empty list, with no remote paired in the
/// test's key directory).
const READY: u32 = 4;
/// Frames before the first tap: the overview drawn with the sessions.
const SETTLE: u32 = 8;
/// Frames for the drawer's slide (0.3 s) and more.
const SLIDE: u32 = 10;
/// Frames for a 2 s press, at 15 fps, and some.
const HOLD: u32 = 36;

fn remote(n: u8, name: &str, active: bool) -> RemoteInfo {
    RemoteInfo {
        id: RemoteId([n; 8]),
        name: name.into(),
        host: format!("{name}.tail.ts.net:7347"),
        fingerprint: format!("{n:02x}{n:02x} {n:02x}{n:02x}"),
        paired_at: Some(1_761_600_000),
        active,
        online: true,
        fingerprint_hex: String::new(),
        fingerprint_words: String::new(),
        nickname: None,
        display_name: name.into(),
    }
}

fn agent(pane: &str, tab: u32) -> AgentInfo {
    AgentInfo {
        pane_id: pane.into(),
        name: Some(format!("Claude {pane}")),
        spoken: None,
        agent: "claude".into(),
        project: Some("flankr".into()),
        cwd: None,
        workspace_id: Some("w1".into()),
        tab_id: None,
        tab_label: Some(format!("{tab}-fl")),
        tab_number: Some(tab),
        status: "idle".into(),
        focused: false,
        session: None,
        turn: Some(Turn::Stopped {
            line: "done".into(),
        }),
        idle_ms: Some(120_000),
        subagents_running: None,
        children: Vec::new(),
        summary: None,
    }
}

/// m4 in use with sessions p1 and p2; mini in the background.
fn connected() {
    probe::inject(
        READY,
        Event::Remotes(vec![remote(1, "m4", true), remote(2, "mini", false)]),
    );
    probe::inject(
        READY,
        Event::Connected {
            server: "m4".into(),
        },
    );
    probe::inject(READY, Event::Agents(vec![agent("p1", 1), agent("p2", 2)]));
}

/// Runs `script` on a fresh app with [`connected`]'s events; `None`
/// without a GPU. Its keys and data are in a directory of its own.
fn run(script: &str, more: impl FnOnce()) -> Option<Probe> {
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
    connected();
    more();
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

/// The places, from a run that only settles: the overview's ≡, the
/// cards and the rows.
fn places() -> Option<Probe> {
    run(&format!("frames {SETTLE}\n"), || {})
}

fn inputs_with(p: &Probe, part: &str) -> usize {
    p.inputs.iter().filter(|i| i.contains(part)).count()
}

#[test]
fn closed_it_draws_straight_to_the_surface_with_no_texture() {
    let Some(p) = places() else { return };
    assert!(!p.frames.is_empty());
    assert!(
        p.frames
            .iter()
            .all(|&(_, direct, textures)| direct && !textures),
        "{:?}",
        p.frames
    );
    assert_eq!(p.frames.last().unwrap().0, Shown::Overview);
    assert_eq!(p.drawer.panes, ["p1", "p2"]);
    assert_eq!(p.drawer.cards.len(), 3, "m4, mini and the +");
    assert!(!p.drawer.shown);
    assert!(inputs_with(&p, "DrawerShown") == 0);
}

#[test]
fn the_menu_opens_it_layered_and_back_closes_it_freeing_the_textures() {
    let Some(at) = places() else { return };
    let menu = at.drawer.menu.unwrap();
    let script = format!(
        "frames {SETTLE}\n{}frames {SLIDE}\nkey back\nframes {SLIDE}\n",
        tap(menu)
    );
    let p = run(&script, || {}).unwrap();
    let before = SETTLE as usize;
    assert!(
        p.frames[..before].iter().all(|f| f.1 && !f.2),
        "closed first"
    );
    let open = &p.frames[before + 2..before + SLIDE as usize];
    assert!(open.iter().all(|f| !f.1 && f.2), "open: layered, {open:?}");
    let last = p.frames.last().unwrap();
    assert!(last.1 && !last.2, "closed again: direct, no texture");
    assert!(!p.drawer.shown);
    assert_eq!(inputs_with(&p, "DrawerShown(true)"), 1);
    assert_eq!(inputs_with(&p, "DrawerShown(false)"), 1);
}

#[test]
fn a_tap_on_the_scrim_closes_it() {
    let Some(at) = places() else { return };
    let menu = at.drawer.menu.unwrap();
    // Right of the sheet: the scrim.
    let scrim = Rect::new(1000.0, 1200.0, 10.0, 10.0);
    let script = format!(
        "frames {SETTLE}\n{}frames {SLIDE}\n{}frames {SLIDE}\n",
        tap(menu),
        tap(scrim)
    );
    let p = run(&script, || {}).unwrap();
    assert!(!p.drawer.shown);
    assert_eq!(inputs_with(&p, "DrawerShown(false)"), 1);
    assert!(
        p.frames[SETTLE as usize..]
            .iter()
            .all(|f| f.0 == Shown::Overview)
    );
}

#[test]
fn a_row_opens_its_view_at_once_from_the_overview_and_from_another_view() {
    let Some(at) = places() else { return };
    let menu = at.drawer.menu.unwrap();
    let (row1, row2) = (at.drawer.rows[0], at.drawer.rows[1]);
    // The overview, the drawer, p2's row: p2's view. Then from p2's view
    // the drawer again, p1's row: p1's view, with no overview between.
    // Then p1's own row: nothing more.
    let first = format!(
        "frames {SETTLE}\n{}frames {SLIDE}\n{}frames {SLIDE}\n",
        tap(menu),
        tap(row2)
    );
    let p = run(&first, || {}).unwrap();
    let agent_menu = p.drawer.menu.unwrap();
    assert_eq!(p.drawer.agent_pane.as_deref(), Some("p2"));
    let script = format!(
        "{first}{}frames {SLIDE}\n{}frames {SLIDE}\n{}frames {SLIDE}\n{}frames {SLIDE}\n",
        tap(agent_menu),
        tap(row1),
        tap(agent_menu),
        tap(row1)
    );
    let p = run(&script, || {}).unwrap();
    assert_eq!(p.drawer.agent_pane.as_deref(), Some("p1"));
    let shown: Vec<Shown> = p.frames.iter().map(|f| f.0).collect();
    let first_agent = shown.iter().position(|s| *s == Shown::Agent).unwrap();
    assert!(
        shown[first_agent..].iter().all(|s| *s == Shown::Agent),
        "the overview never drawn between views: {shown:?}"
    );
    assert_eq!(inputs_with(&p, "OpenAgent(\"p2\")"), 1);
    assert_eq!(
        inputs_with(&p, "OpenAgent(\"p1\")"),
        1,
        "its own row: nothing more"
    );
    assert_eq!(inputs_with(&p, "CloseAgent"), 1);
    assert!(!p.drawer.shown);
    // While the view was open under the drawer, the list was asked for
    // (DrawerShown each time it showed).
    assert_eq!(inputs_with(&p, "DrawerShown(true)"), 3);
}

#[test]
fn a_tap_on_a_card_switches_with_a_view_and_a_message_open_and_ends_on_the_new_list() {
    let Some(at) = places() else { return };
    let menu = at.drawer.menu.unwrap();
    let row1 = at.drawer.rows[0];
    let mini = at.drawer.cards[1];
    let open_view = format!(
        "frames {SETTLE}\n{}frames {SLIDE}\n{}frames {SLIDE}\n",
        tap(menu),
        tap(row1)
    );
    let first = run(&open_view, || {}).unwrap();
    let agent_menu = first.drawer.menu.unwrap();
    let before_switch = format!("{open_view}frames 2\n{}frames {SLIDE}\n", tap(agent_menu));
    let script = format!("{before_switch}{}frames {SLIDE}\n", tap(mini));
    let p = run(&script, || {
        // A typed message to p1, open in its view.
        probe::inject_after(
            "OpenAgent",
            Event::Dictation(Dictation {
                active: true,
                mode: PromptMode::Keyboard,
                text: "hello".into(),
                target: None,
                focus_moved: None,
                phase: DictationPhase::Collecting,
                submit_in: None,
                general_agent: false,
            }),
        );
        // The switch's replay, as the client gives it: mini's sessions,
        // then the rows with mini in use.
        let replay = "SwitchRemote";
        probe::inject_after(
            replay,
            Event::Connected {
                server: "mini".into(),
            },
        );
        probe::inject_after(replay, Event::Agents(vec![agent("q1", 1)]));
        probe::inject_after(
            replay,
            Event::Remotes(vec![remote(1, "m4", false), remote(2, "mini", true)]),
        );
    })
    .unwrap();
    // The view and the message closed first, then the switch.
    let close = p
        .inputs
        .iter()
        .rposition(|i| i == "CloseAgent")
        .unwrap_or_else(|| panic!("{:?} {:?}", p.inputs, p.drawer));
    let switch = p
        .inputs
        .iter()
        .position(|i| i.starts_with("SwitchRemote"))
        .expect("switched");
    assert!(close < switch, "{:?}", p.inputs);
    assert_eq!(
        p.inputs[switch],
        format!("SwitchRemote({:?})", RemoteId([2; 8]))
    );
    assert_eq!(p.drawer.agent_pane, None);
    assert!(
        !p.drawer.message,
        "the message to the old remote's pane dropped"
    );
    assert_eq!(p.frames.last().unwrap().0, Shown::Overview);
    assert_eq!(p.drawer.agents, ["q1"]);
    assert_eq!(
        p.drawer.panes,
        ["q1"],
        "the drawer's list is the new remote's"
    );
    assert!(p.drawer.shown, "the drawer stays open");
}

#[test]
fn a_tap_on_the_card_in_use_does_nothing_and_on_the_plus_opens_the_add_mode() {
    let Some(at) = places() else { return };
    let menu = at.drawer.menu.unwrap();
    let (m4, plus) = (at.drawer.cards[0], at.drawer.cards[2]);
    let script = format!(
        "frames {SETTLE}\n{}frames {SLIDE}\n{}frames 4\n",
        tap(menu),
        tap(m4)
    );
    let p = run(&script, || {}).unwrap();
    assert_eq!(inputs_with(&p, "SwitchRemote"), 0);
    assert!(p.drawer.shown && !p.drawer.details);
    let script = format!("{script}{}frames 4\n", tap(plus));
    let p = run(&script, || {}).unwrap();
    assert!(p.drawer.adding, "the pairing window's add mode");
    assert!(!p.drawer.shown, "closed at once");
}

#[test]
fn a_2_s_press_on_any_card_opens_its_details_and_a_press_released_early_is_a_tap() {
    let Some(at) = places() else { return };
    let menu = at.drawer.menu.unwrap();
    for card in [at.drawer.cards[0], at.drawer.cards[1]] {
        let [x, y] = card.center();
        let script = format!(
            "frames {SETTLE}\n{}frames {SLIDE}\ntouch 5 start {x} {y}\nframes {HOLD}\ntouch 5 end {x} {y}\nframes 4\n",
            tap(menu)
        );
        let p = run(&script, || {}).unwrap();
        assert!(
            p.drawer.settings && p.drawer.details,
            "the details open: {:?} {:?}",
            p.drawer,
            p.inputs
        );
        assert!(!p.drawer.shown);
        assert_eq!(inputs_with(&p, "SwitchRemote"), 0, "a hold is no tap");
        // Back closes the details, and the Settings they opened over.
        let p = run(&format!("{script}key back\nframes 4\n"), || {}).unwrap();
        assert!(!p.drawer.settings);
    }
    // Released after 1 s: a tap, which switches.
    let [x, y] = at.drawer.cards[1].center();
    let script = format!(
        "frames {SETTLE}\n{}frames {SLIDE}\ntouch 5 start {x} {y}\nframes 12\ntouch 5 end {x} {y}\nframes 4\n",
        tap(menu)
    );
    let p = run(&script, || {}).unwrap();
    assert!(!p.drawer.settings);
    assert_eq!(inputs_with(&p, "SwitchRemote"), 1);
}

#[test]
fn a_background_remotes_question_and_approval_show_with_its_name_and_are_answered_through_it() {
    let mini = RemoteId([2; 8]);
    let bg = move |event: Event| Event::Background {
        remote: mini,
        event: Box::new(event),
    };
    let script = format!("frames {SETTLE}\nframes 4\n");
    let p = run(&script, || {
        probe::inject(
            READY + 1,
            bg(Event::SessionCards(vec![flankr_companion::SessionCard {
                pane_id: "q1".into(),
                session: "s1".into(),
                kind: flankr_companion::SessionCardKind::Question,
                agent: "Claude".into(),
                colour: 3,
                text: "Push it?".into(),
                at: std::time::Duration::from_secs(1),
            }])),
        );
        probe::inject(
            READY + 1,
            bg(Event::Confirm {
                confirm_id: 41,
                text: "Approve?".into(),
                label: None,
                phrase: String::new(),
                command: "continue".into(),
                agent: "Claude".into(),
                project: None,
            }),
        );
    })
    .unwrap();
    assert_eq!(p.drawer.waiting, [mini], "mini's card has a dot");
    assert_eq!(p.drawer.session_cards, ["mini \u{b7} Claude"]);
    assert_eq!(
        p.drawer.card,
        Some(("mini \u{b7} Claude".to_string(), Some(mini)))
    );
    // Approve answers through mini's link.
    let approve = p.drawer.approve.expect("the card's Approve");
    let p = run(&format!("{script}{}frames 2\n", tap(approve)), || {
        probe::inject(
            READY + 1,
            bg(Event::Confirm {
                confirm_id: 41,
                text: "Approve?".into(),
                label: None,
                phrase: String::new(),
                command: "continue".into(),
                agent: "Claude".into(),
                project: None,
            }),
        );
    })
    .unwrap();
    let answer = p
        .inputs
        .iter()
        .find(|i| i.starts_with("ToRemote"))
        .unwrap_or_else(|| panic!("{:?}", p.inputs));
    assert!(answer.contains(&format!("{mini:?}")), "{answer}");
    assert!(
        answer.contains("Answer { confirm_id: 41, approve: true }"),
        "{answer}"
    );
}
