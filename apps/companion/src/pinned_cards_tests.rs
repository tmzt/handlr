//! The pinned card in scripted runs of the whole app (plan 20261022_03,
//! the L3 amendment of 2026-10-30 15:20 EDT): *More* and *Less*, a link in
//! its text, and a sideways drag that removes it, by kind. As
//! `subagents_tests` drives it: libmorph's scripted runs at the Galaxy
//! S24's size, the client's events given by the test, and what the app
//! did read back from [`probe`]. Each run needs a GPU adapter; without one
//! it says so and passes.
//!
//! The view of p1 is opened from the session drawer; its `Event::AgentView`
//! comes with a few entries and the one card under test, pinned.

use flankr_companion::{AgentCard, CardKind, Event, RemoteId, RemoteInfo, Shows, ViewSection};
use libcontrols::Rect;
use libflankr_proto::common::{
    AgentInfo, ChoiceOption, ChoiceQuestion, ChoiceSet, Entry, EntryKind, Role, Serial,
};
use libmorph::script::{OutDir, Script, ScriptRun};
use libmorph::{GpuError, MorphError};

use super::App;
use super::probe::{self, PinProbe, Probe};
use crate::agent::{
    AgentHit, BUTTON_H, DENIED_BY_DISMISSING, DISMISSED_THE_QUESTION, PIN_LESS, PIN_MORE,
};

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
        last_session: None,
    }
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
        status: "idle".into(),
        focused: false,
        session: None,
        turn: None,
        idle_ms: None,
        subagents_running: None,
        summary: None,
        children: Vec::new(),
    }
}

fn entry(role: Role, text: &str) -> Entry {
    Entry {
        ts: "2026-10-30T19:00:00Z".into(),
        role,
        kind: EntryKind::Text,
        tool: None,
        summary: text.chars().take(80).collect(),
        links: Vec::new(),
        origin: None,
        text: Some(text.into()),
    }
}

/// The card's section serial.
fn card_serial() -> Serial {
    Serial::new(7, 9, 194)
}

fn live(kind: CardKind, title: &str, text: &str) -> AgentCard {
    AgentCard {
        kind,
        live: true,
        at: 0,
        title: title.into(),
        text: text.into(),
        confirm_id: (kind == CardKind::Approval).then_some(7),
        about_plan: false,
        links: Vec::new(),
        points: Vec::new(),
        spoken: None,
        answers: Vec::new(),
        choices: None,
    }
}

/// The view as the client gives it: a user turn, a reply, and `card`
/// after them, pinned.
fn view(card: AgentCard) -> Event {
    let mut sections: Vec<ViewSection> = [
        entry(Role::User, "build the cards"),
        entry(Role::Assistant, "Landed the pinned cards.\nThen pushed."),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, e)| ViewSection {
        index: i as u64,
        serial: Serial::new(7, 2 * i as u32 + 2, 0),
        shows: Shows::Entries(vec![e]),
    })
    .collect();
    sections.push(ViewSection {
        index: 2,
        serial: card_serial(),
        shows: Shows::Card(card),
    });
    let changed = sections.iter().map(|s| s.serial).collect();
    Event::AgentView {
        name: "Claude p1".into(),
        status: "idle".into(),
        sections,
        pinned: Some(card_serial()),
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
        child: None,
        children: Vec::new(),
        cached: false,
        ended: false,
    }
}

fn run(script: &str, card: &AgentCard) -> Option<Probe> {
    run_with(script, card, Vec::new())
}

/// [`run`], with `after`'s events given as the client's once the app has
/// sent an input starting with their text.
fn run_with(script: &str, card: &AgentCard, after: Vec<(&'static str, Event)>) -> Option<Probe> {
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
    probe::inject_after("OpenAgent", view(card.clone()));
    // The test's choice results stand in for the daemon's.
    if after
        .iter()
        .any(|(_, e)| matches!(e, Event::ChoiceAnswered { .. }))
    {
        probe::daemon_choices();
    }
    for (input, event) in after {
        probe::inject_after(input, event);
    }
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

fn inputs_with(p: &Probe, part: &str) -> usize {
    p.inputs.iter().filter(|i| i.contains(part)).count()
}

/// The script that opens p1's view with `card` pinned, the probe after
/// it, and the screen's scale.
fn open_view(card: &AgentCard) -> Option<(String, Probe, f32)> {
    let at = run(&format!("frames {SETTLE}\n"), card)?;
    let script = format!(
        "frames {SETTLE}\n{}frames {SLIDE}\n{}frames {SLIDE}\n",
        tap(at.drawer.menu.expect("the overview's ≡")),
        tap(*at.drawer.rows.first().expect("p1's row")),
    );
    let p = run(&script, card)?;
    assert_eq!(p.drawer.agent_pane.as_deref(), Some("p1"), "{:?}", p.inputs);
    let message = p
        .drawer
        .bar
        .iter()
        .find(|(h, _)| *h == AgentHit::Message)
        .map(|&(_, r)| r)
        .expect("the bar");
    Some((script, p, message.w / BUTTON_H))
}

fn pin(p: &Probe) -> &PinProbe {
    p.drawer.pin.as_ref().expect("the pinned card")
}

/// A finger on the pinned card's title row, moved `dx` across in `steps`
/// a frame apart, then held still `hold` frames and lifted.
fn sideways(p: &Probe, dx: f32, steps: u32, hold: u32) -> String {
    let r = pin(p).rect;
    let (x, y) = (r.x + r.w * 0.5, r.y + 30.0);
    let mut script = format!("touch 9 start {x} {y}\nframes 1\n");
    for i in 1..=steps {
        let to = x + dx * i as f32 / steps as f32;
        script.push_str(&format!("touch 9 move {to} {y}\nframes 1\n"));
    }
    if hold > 0 {
        script.push_str(&format!("frames {hold}\n"));
    }
    script.push_str(&format!("touch 9 end {} {y}\nframes 6\n", x + dx));
    script
}

#[test]
fn more_and_less_alone_change_the_cards_size() {
    let long = "Should I go on with the next step, which moves the readers onto the record \
                trait, changes every test fixture's loader, and then rebuilds the daemon and \
                the app before the check on the phone, or stop here so you can look first?";
    let card = live(CardKind::Question, "Claude is asking", long);
    let Some((open, at, _)) = open_view(&card) else {
        return;
    };
    let folded = pin(&at).clone();
    assert!(
        folded.texts.iter().any(|t| t == PIN_MORE),
        "{:?}",
        folded.texts
    );
    let more = folded.more.expect("More");
    assert!(more.x > folded.rect.x + folded.rect.w * 0.6, "at its right");
    // A tap on its text changes nothing.
    let text = format!(
        "tap {} {}\nframes 4\n",
        folded.rect.x + 60.0,
        folded.rect.y + 60.0
    );
    let p = run(&format!("{open}{text}"), &card).unwrap();
    assert_eq!(pin(&p).rect, folded.rect, "{:?}", p.inputs);
    // More: all of it, and Less.
    let script = format!("{open}{}frames 4\n", tap(more));
    let p = run(&script, &card).unwrap();
    let open_pin = pin(&p).clone();
    assert!(open_pin.rect.h > folded.rect.h);
    assert!(
        open_pin.texts.iter().any(|t| t == PIN_LESS),
        "{:?}",
        open_pin.texts
    );
    // Less: folded again.
    let p = run(
        &format!("{script}{}frames 4\n", tap(open_pin.more.unwrap())),
        &card,
    )
    .unwrap();
    assert_eq!(pin(&p).rect.h, folded.rect.h);
    assert_eq!(
        inputs_with(&p, "Respond"),
        0,
        "nothing answered: {:?}",
        p.inputs
    );
}

#[test]
fn a_tap_on_a_link_in_the_cards_markdown_opens_it() {
    let card = live(
        CardKind::Question,
        "Claude is asking",
        "Built **the cards**: see [the plan](https://example.com/plan) and \
         `docs/design/x.md`.\n\n- one\n- two",
    );
    let Some((open, at, _)) = open_view(&card) else {
        return;
    };
    let link = *pin(&at).links.first().expect("the link is spanned");
    let p = run(&format!("{open}{}frames 4\n", tap(link)), &card).unwrap();
    assert_eq!(p.opened, ["https://example.com/plan"], "{:?}", p.inputs);
    assert_eq!(pin(&p).rect, pin(&at).rect, "the card's size is as it was");
}

#[test]
fn a_finished_card_dragged_away_is_one_line_and_its_speech_dropped() {
    let card = live(
        CardKind::Finished,
        "Finished",
        "Landed the pinned cards.\nThen pushed.",
    );
    let Some((open, at, s)) = open_view(&card) else {
        return;
    };
    let p = run(&format!("{open}{}", sideways(&at, -60.0 * s, 6, 0)), &card).unwrap();
    let bar = pin(&p);
    assert!(bar.compact, "{:?}", p.inputs);
    assert_eq!(bar.texts.len(), 1);
    assert!(
        bar.texts[0].starts_with("Finished \u{b7} Landed the pinned cards."),
        "{:?}",
        bar.texts
    );
    assert_eq!(inputs_with(&p, "DropSpeech(Card("), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Respond"), 0);
}

#[test]
fn an_approval_dragged_away_is_denied_with_its_line_in_the_flow() {
    let card = live(
        CardKind::Approval,
        "Approval needed",
        "cargo test -p companion",
    );
    let Some((open, at, s)) = open_view(&card) else {
        return;
    };
    let p = run(&format!("{open}{}", sideways(&at, 60.0 * s, 6, 0)), &card).unwrap();
    assert_eq!(
        inputs_with(&p, "Answer { confirm_id: 7, approve: false }"),
        1,
        "{:?}",
        p.inputs
    );
    assert_eq!(inputs_with(&p, "approve: true"), 0);
    assert_eq!(inputs_with(&p, "DropSpeech(Card("), 1, "{:?}", p.inputs);
    assert!(p.drawer.pin.is_none(), "out of the pinned place");
    assert!(
        p.drawer
            .flow_notes
            .contains(&format!("-- {DENIED_BY_DISMISSING}")),
        "{:?}",
        p.drawer.flow_notes
    );
}

#[test]
fn a_short_drag_springs_back_and_does_nothing() {
    let card = live(
        CardKind::Approval,
        "Approval needed",
        "cargo test -p companion",
    );
    let Some((open, at, s)) = open_view(&card) else {
        return;
    };
    // 25 pt, held still before lifting: no fling.
    let p = run(
        &format!("{open}{}frames 15\n", sideways(&at, -25.0 * s, 5, 4)),
        &card,
    )
    .unwrap();
    let back = pin(&p);
    assert_eq!(back.offset, 0.0, "back in place");
    assert_eq!(back.rect, pin(&at).rect);
    assert!(!back.compact);
    for part in ["Answer {", "DropSpeech", "Respond"] {
        assert_eq!(inputs_with(&p, part), 0, "{part}: {:?}", p.inputs);
    }
}

/// A live choice question's card (plan 20261029_01): one single-select
/// question.
fn choice_card() -> AgentCard {
    let set = ChoiceSet::new(
        "toolu_1",
        vec![ChoiceQuestion::new(
            "Colour",
            "Which colour should the badge be?",
            false,
            true,
            vec![
                ChoiceOption::new("Teal", "matches the bar", None),
                ChoiceOption::new("Amber", "stands out", None),
            ],
        )],
        "claude_code/AskUserQuestion",
    );
    AgentCard {
        choices: Some(set),
        ..live(
            CardKind::Question,
            "Claude is asking",
            "Which colour should the badge be?",
        )
    }
}

fn cancel_answered(outcome: &str, got: Vec<String>) -> Event {
    Event::ChoiceAnswered {
        id: "toolu_1".into(),
        pane_id: "p1".into(),
        outcome: outcome.into(),
        got,
        code: None,
    }
}

#[test]
fn a_choice_question_dragged_away_is_cancelled_with_its_line() {
    let card = choice_card();
    let Some((open, at, s)) = open_view(&card) else {
        return;
    };
    let p = run_with(
        &format!("{open}{}frames 4\n", sideways(&at, -60.0 * s, 6, 0)),
        &card,
        vec![("CancelChoice", cancel_answered("cancelled", Vec::new()))],
    )
    .unwrap();
    assert_eq!(
        inputs_with(&p, "CancelChoice { id: \"toolu_1\" }"),
        1,
        "{:?}",
        p.inputs
    );
    assert_eq!(inputs_with(&p, "Choose {"), 0, "not answered");
    assert_eq!(inputs_with(&p, "DropSpeech(Card("), 1, "{:?}", p.inputs);
    assert!(p.drawer.pin.is_none(), "out of the pinned place");
    assert!(
        p.drawer
            .flow_notes
            .contains(&format!("-- {DISMISSED_THE_QUESTION}")),
        "{:?}",
        p.drawer.flow_notes
    );
}

#[test]
fn a_choice_question_not_cancelled_springs_back_with_a_note() {
    let card = choice_card();
    let Some((open, at, s)) = open_view(&card) else {
        return;
    };
    let p = run_with(
        &format!("{open}{}frames 20\n", sideways(&at, -60.0 * s, 6, 0)),
        &card,
        vec![("CancelChoice", cancel_answered("not_cancelled", Vec::new()))],
    )
    .unwrap();
    assert_eq!(inputs_with(&p, "CancelChoice {"), 1, "{:?}", p.inputs);
    let back = pin(&p);
    assert_eq!(back.offset, 0.0, "sprung back into place");
    assert!(!back.compact);
    assert!(
        back.texts
            .iter()
            .any(|t| t == "Couldn't dismiss the question"),
        "{:?}",
        back.texts
    );
    assert!(
        !p.drawer
            .flow_notes
            .contains(&format!("-- {DISMISSED_THE_QUESTION}")),
        "{:?}",
        p.drawer.flow_notes
    );
}
