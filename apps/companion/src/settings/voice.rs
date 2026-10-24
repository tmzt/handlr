//! The *Voice* setting (plan 20261024_02, CHANGES.md 2026-10-24 14:58 EDT):
//! which of Phonon's voices handlr speaks in.
//!
//! - **The choices:** *Default* (Phonon's own, `None`) first, then the voices
//!   Phonon reported on load (`AudioEvent::TtsLoaded`'s names), in
//!   alphabetical order; the row's arrows step through them, wrapping at
//!   both ends ([`step`]).
//! - **A saved voice that is no longer installed** is spoken, and shown, as
//!   *Default* ([`effective`]), and stays in `settings.toml` until another
//!   is chosen.
//! - **The preview:** each step says "This is <voice>." in that voice,
//!   through the app's audio, while *All sound* is on. A step while
//!   something is being said cuts it off first ([`preview`]).

use handlr_audio::Action;

use super::copy;

/// The voices to step through: *Default*, then `names` sorted, each once.
pub fn choices(names: &[String]) -> Vec<Option<String>> {
    let mut sorted: Vec<&String> = names.iter().filter(|n| !n.is_empty()).collect();
    sorted.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b)));
    sorted.dedup();
    std::iter::once(None)
        .chain(sorted.into_iter().cloned().map(Some))
        .collect()
}

/// The voice spoken for `saved`: itself when Phonon has it, else *Default*.
pub fn effective(saved: Option<&str>, names: &[String]) -> Option<String> {
    saved
        .filter(|v| names.iter().any(|n| n == v))
        .map(str::to_string)
}

/// The voice after `voice` when stepping right (`forward`) or left,
/// wrapping at both ends. A voice not among `names` steps as *Default*.
pub fn step(voice: Option<&str>, names: &[String], forward: bool) -> Option<String> {
    let all = choices(names);
    let now = effective(voice, names);
    let i = all.iter().position(|v| *v == now).unwrap_or(0);
    let n = all.len();
    let next = if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    };
    all[next].clone()
}

/// The voice as the row shows it.
pub fn label(voice: Option<&str>) -> &str {
    voice.unwrap_or(copy::VOICE_DEFAULT.text)
}

/// The preview's text for `voice`: "This is <voice>.", or "This is the
/// default voice." for *Default*.
pub fn preview_text(voice: Option<&str>) -> String {
    match voice {
        // Not "This is Default." (owner, 2026-10-24).
        None => copy::VOICE_PREVIEW_DEFAULT.text.to_string(),
        Some(_) => copy::VOICE_PREVIEW.fill(None, &[("voice", label(voice))]),
    }
}

/// What the audio is asked for a step's preview of `voice`, already set
/// as the session's voice: a `Stop` first while anything is being said
/// (`speaking`), so a further step cuts the last preview off, then the
/// preview.
pub fn preview(voice: Option<&str>, speaking: bool) -> Vec<Action> {
    let mut out = Vec::with_capacity(2);
    if speaking {
        out.push(Action::Stop);
    }
    out.push(Action::Speak(preview_text(voice)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["marius", "alba", "Cosette", "alba"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn default_first_then_the_voices_in_alphabetical_order() {
        assert_eq!(
            choices(&names()),
            [
                None,
                Some("alba".to_string()),
                Some("Cosette".to_string()),
                Some("marius".to_string())
            ]
        );
        assert_eq!(choices(&[]), [None], "no voices: Default alone");
    }

    #[test]
    fn stepping_goes_both_ways_and_wraps() {
        let names = names();
        let mut v: Option<String> = None;
        let mut seen = Vec::new();
        for _ in 0..4 {
            v = step(v.as_deref(), &names, true);
            seen.push(v.clone());
        }
        let s = |x: &str| Some(x.to_string());
        assert_eq!(seen, [s("alba"), s("Cosette"), s("marius"), None]);
        assert_eq!(step(None, &names, false), s("marius"), "left from Default");
        assert_eq!(step(Some("alba"), &names, false), None);
        assert_eq!(step(None, &[], true), None, "only Default");
    }

    #[test]
    fn an_unknown_voice_is_default() {
        let names = names();
        assert_eq!(effective(Some("javert"), &names), None);
        assert_eq!(effective(Some("alba"), &names), Some("alba".into()));
        assert_eq!(effective(None, &names), None);
        // It steps as Default.
        assert_eq!(step(Some("javert"), &names, true), Some("alba".into()));
    }

    #[test]
    fn the_preview_names_the_voice_and_a_further_step_cuts_it_off() {
        assert_eq!(preview_text(Some("alba")), "This is alba.");
        assert_eq!(preview_text(None), "This is the default voice.");
        assert_eq!(
            preview(Some("alba"), false),
            [Action::Speak("This is alba.".into())]
        );
        assert_eq!(
            preview(Some("Cosette"), true),
            [Action::Stop, Action::Speak("This is Cosette.".into())]
        );
    }
}
