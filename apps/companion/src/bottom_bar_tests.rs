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

use flankr_companion::{Event, RemoteId, RemoteInfo, Speaker};

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
/// Frames for a press past 1 s and short of 2 s, at 15 fps.
const STOP_HOLD: u32 = 20;
/// Frames for a press past the centre's and right's 500 ms and short of
/// 1 s, at 15 fps.
const CARD_HOLD: u32 = 11;

fn run(script: &str) -> Option<Probe> {
    run_with(script, None)
}

/// As [`run`], from `settings` as `settings.toml` when given.
fn run_with(script: &str, settings: Option<&str>) -> Option<Probe> {
    run_full(script, settings, false)
}

/// As [`run`], the speaker speaking from the update the sessions are
/// given on.
fn run_speaking(script: &str) -> Option<Probe> {
    run_full(script, None, true)
}

fn run_full(script: &str, settings: Option<&str>, speaking: bool) -> Option<Probe> {
    run_events(script, settings, speaking, Vec::new())
}

/// As [`run_full`], `events` also given as the client's on the update
/// after the sessions'.
fn run_events(
    script: &str,
    settings: Option<&str>,
    speaking: bool,
    events: Vec<Event>,
) -> Option<Probe> {
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
    probe::speaking(u32::MAX);
    if speaking {
        probe::inject_speaking(READY, true);
    }
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
            last_session: None,
        }]),
    );
    probe::inject(
        READY,
        Event::Connected {
            server: "m4".into(),
        },
    );
    probe::inject(READY, Event::Agents(Vec::new()));
    for event in events {
        probe::inject(READY + 1, event);
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
fn a_500_ms_press_on_the_centre_opens_geminis_card_typed_without_listening() {
    let Some((_, centre, _)) = bar() else { return };
    let [x, y] = centre.center();
    let p = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {CARD_HOLD}\ntouch 5 end {x} {y}\nframes 4\n"
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
fn a_500_ms_press_on_the_right_opens_the_message_card_typed() {
    let Some((_, _, right)) = bar() else { return };
    let [x, y] = right.center();
    let p = run(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {CARD_HOLD}\ntouch 5 end {x} {y}\nframes 4\n"
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
    // Both options on: muting stops what is said, clears the queue and
    // tells the client it is muted, both options left as they are (plan
    // 20261023_02, the 2026-10-30 15:10 EDT amendment); no menu opens.
    let on = "speak_coding = true\nspeak_gemini = true\n";
    let p = run_with(&format!("frames {SETTLE}\n{tap}"), Some(on)).unwrap();
    assert!(p.drawer.menu_items.is_empty(), "{:?}", p.drawer.menu_items);
    assert_eq!(inputs_with(&p, "StopReading"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Skip"), 0, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Muted(true)"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "ReadAloud("), 0, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "speak_gemini: false"), 0, "{:?}", p.inputs);
    // A second tap unmutes, and nothing more is stopped.
    let p = run_with(&format!("frames {SETTLE}\n{tap}{tap}"), Some(on)).unwrap();
    assert_eq!(inputs_with(&p, "StopReading"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Muted(false)"), 1, "{:?}", p.inputs);
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
    assert_eq!(inputs_with(&p, "StopReading"), 0, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Muted(false)"), 1, "{:?}", p.inputs);
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

#[test]
fn while_speaking_a_tap_on_volume_skips_and_does_not_mute() {
    let Some((volume, _, _)) = bar() else { return };
    let [x, y] = volume.center();
    let p = run_speaking(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes 2\ntouch 5 end {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(inputs_with(&p, "Skip"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "StopReading"), 0, "{:?}", p.inputs);
    // Not muted.
    assert_eq!(inputs_with(&p, "Muted(true)"), 0, "{:?}", p.inputs);
    assert!(p.drawer.menu_items.is_empty(), "{:?}", p.drawer.menu_items);
}

#[test]
fn while_speaking_a_1_s_press_on_volume_stops_and_clears_without_muting() {
    let Some((volume, _, _)) = bar() else { return };
    let [x, y] = volume.center();
    // Held past 1 s: stopped before the release, no tap after it.
    let p = run_speaking(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {STOP_HOLD}\n"
    ))
    .unwrap();
    assert_eq!(inputs_with(&p, "StopReading"), 1, "{:?}", p.inputs);
    assert!(p.drawer.holding, "still timed for the menu");
    assert!(p.drawer.menu_items.is_empty());
    let p = run_speaking(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {STOP_HOLD}\ntouch 5 end {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(inputs_with(&p, "StopReading"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Skip"), 0, "no tap: {:?}", p.inputs);
    assert_eq!(
        inputs_with(&p, "Muted(true)"),
        0,
        "not muted: {:?}",
        p.inputs
    );
    assert!(p.drawer.menu_items.is_empty(), "{:?}", p.drawer.menu_items);
}

#[test]
fn while_speaking_a_2_s_press_on_volume_stops_then_opens_its_menu() {
    let Some((volume, _, _)) = bar() else { return };
    let [x, y] = volume.center();
    let p = run_speaking(&format!(
        "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {HOLD}\ntouch 5 end {x} {y}\nframes 4\n"
    ))
    .unwrap();
    assert_eq!(inputs_with(&p, "StopReading"), 1, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Skip"), 0, "{:?}", p.inputs);
    assert!(
        p.drawer.menu_items.iter().any(|i| i == menu::SPEAK_CODING),
        "{:?}",
        p.drawer.menu_items
    );
}

/// What the client asks to say, as each kind comes: a read-aloud reply, a
/// spoken notice, a confirmation's prompt and a Gemini line; and the ding.
fn voices_and_a_ding() -> Vec<Event> {
    vec![
        Event::Say("The tests pass.".into()),
        Event::Say("Claude in flankr is done.".into()),
        Event::Confirm {
            confirm_id: 7,
            text: "Approve running the tests?".into(),
            label: None,
            phrase: "run the tests".into(),
            command: "cargo test".into(),
            agent: "claude".into(),
            project: None,
        },
        Event::Say("Approve running the tests?".into()),
        Event::Conversation {
            speaker: Speaker::Gemini,
            text: "Opening flankr.".into(),
        },
        Event::Say("Opening flankr.".into()),
        Event::Ding,
    ]
}

#[test]
fn muted_nothing_is_spoken_and_the_ding_still_plays() {
    let on = "speak_coding = true\nspeak_gemini = true\n";
    let muted = "speak_coding = true\nspeak_gemini = true\nmuted = true\n";
    let script = format!("frames {SETTLE}\n");
    // Unmuted, each is handed to the audio, the ding as a chime.
    let Some(p) = run_events(&script, Some(on), false, voices_and_a_ding()) else {
        return;
    };
    let speak = |p: &Probe| p.audio.iter().filter(|a| a.starts_with("Speak")).count();
    assert_eq!(speak(&p), 4, "{:?}", p.audio);
    assert_eq!(p.audio.iter().filter(|a| *a == "Chime").count(), 1);
    // Muted: none is spoken, each counts as said at once, and the ding
    // still plays.
    let p = run_events(&script, Some(muted), false, voices_and_a_ding()).unwrap();
    assert_eq!(speak(&p), 0, "{:?}", p.audio);
    assert_eq!(p.audio, ["Chime"], "{:?}", p.audio);
    assert_eq!(inputs_with(&p, "Speaking(false)"), 4, "{:?}", p.inputs);
    // The confirmation's card is still shown.
    assert!(p.drawer.card.is_some(), "{:?}", p.drawer.card);
}

#[test]
fn muted_a_press_on_volume_unmutes_or_opens_the_menu_even_while_speaking() {
    let Some((volume, _, _)) = bar() else { return };
    let [x, y] = volume.center();
    let muted = Some("speak_coding = true\nmuted = true\n");
    // A tap, the client reporting speech: it unmutes and skips nothing.
    let p = run_full(
        &format!(
            "frames {SETTLE}\ntouch 5 start {x} {y}\nframes 2\ntouch 5 end {x} {y}\nframes 4\n"
        ),
        muted,
        true,
    )
    .unwrap();
    assert_eq!(inputs_with(&p, "Skip"), 0, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "StopReading"), 0, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Muted(false)"), 1, "{:?}", p.inputs);
    // Held past 1 s: no stop, still timed for the menu.
    let p = run_full(
        &format!("frames {SETTLE}\ntouch 5 start {x} {y}\nframes {STOP_HOLD}\n"),
        muted,
        true,
    )
    .unwrap();
    assert_eq!(inputs_with(&p, "StopReading"), 0, "{:?}", p.inputs);
    assert!(p.drawer.holding, "timed for the menu");
    // Held 2 s: the menu opens, nothing stopped, still muted.
    let p = run_full(
        &format!(
            "frames {SETTLE}\ntouch 5 start {x} {y}\nframes {HOLD}\ntouch 5 end {x} {y}\nframes 4\n"
        ),
        muted,
        true,
    )
    .unwrap();
    assert_eq!(inputs_with(&p, "StopReading"), 0, "{:?}", p.inputs);
    assert_eq!(inputs_with(&p, "Muted(false)"), 0, "{:?}", p.inputs);
    assert!(
        p.drawer.menu_items.iter().any(|i| i == menu::SPEAK_CODING),
        "{:?}",
        p.drawer.menu_items
    );
}

/// The *While you spoke* card on the overview (plan 20261023_02, the
/// 2026-10-30 15:10 EDT amendment): the client's catch-up pins it, muted
/// or not; the next replaces it, and its ✕ dismisses it.
#[test]
fn a_catch_up_is_pinned_until_its_x_and_the_next_replaces_it() {
    let catch_up = |text: &str| Event::CatchUp { text: text.into() };
    let Some(p) = run_events(
        &format!("frames {SETTLE}\n"),
        None,
        false,
        vec![catch_up("Codex finished.")],
    ) else {
        return;
    };
    assert_eq!(p.drawer.catch_up.as_deref(), Some("Codex finished."));
    let x = p.drawer.catch_up_x.expect("its \u{2715}");
    // Muted, the same card.
    let muted = Some("speak_coding = true\nmuted = true\n");
    let p = run_events(
        &format!("frames {SETTLE}\n"),
        muted,
        false,
        vec![catch_up("Codex finished.")],
    )
    .unwrap();
    assert_eq!(p.drawer.catch_up.as_deref(), Some("Codex finished."));
    assert_eq!(p.drawer.catch_up_x, Some(x));
    // The next replaces it.
    let p = run_events(
        &format!("frames {SETTLE}\n"),
        None,
        false,
        vec![
            catch_up("Codex finished."),
            catch_up("Claude asks a question."),
        ],
    )
    .unwrap();
    assert_eq!(
        p.drawer.catch_up.as_deref(),
        Some("Claude asks a question.")
    );
    // A tap on its ✕: gone, and nothing is sent.
    let [cx, cy] = x.center();
    let p = run_events(
        &format!(
            "frames {SETTLE}\ntouch 5 start {cx} {cy}\nframes 2\ntouch 5 end {cx} {cy}\nframes 4\n"
        ),
        None,
        false,
        vec![catch_up("Codex finished.")],
    )
    .unwrap();
    assert_eq!(p.drawer.catch_up, None);
    assert_eq!(p.drawer.catch_up_x, None);
    assert!(p.drawer.menu_items.is_empty());
}
