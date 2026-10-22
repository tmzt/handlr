//! The mapping between libvoicexn's workers and `handlr-companion`, kept
//! pure: no threads, devices or models, so it is tested on its own.
//!
//! - [`Event::Say`], [`Event::StopSpeaking`], [`Event::Arm`] and
//!   [`Event::Vocabulary`] become an [`Action`] for the workers.
//! - [`SttEvent`]s and [`TtsEvent`]s become [`AudioEvent`]s, among them
//!   [`Input::Utterance`], [`Input::Listening`] and [`Input::Speaking`] for
//!   [`Companion::send`](handlr_companion::Companion::send).

use handlr_companion::{Event, Input};
use libvoicexn::stt::{SttEvent, SttRequest};
use libvoicexn::tts::{TtsEvent, TtsRequest};
use libvoicexn::vad::GateState;

/// What a companion [`Event`] asks of the audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// [`TtsRequest::Speak`].
    Speak(String),
    /// [`TtsRequest::Stop`], and flushing what is queued for the speaker.
    Stop,
    /// [`SttRequest::Arm`].
    Arm,
    /// Write `vocab.txt`, read when the STT model next loads.
    Vocabulary(Vec<String>),
}

/// The [`Action`] for `event`, if it is one of the audio's.
pub fn action(event: &Event) -> Option<Action> {
    match event {
        Event::Say(text) => Some(Action::Speak(text.clone())),
        Event::StopSpeaking => Some(Action::Stop),
        Event::Arm => Some(Action::Arm),
        Event::Vocabulary(words) => Some(Action::Vocabulary(words.clone())),
        _ => None,
    }
}

/// The TTS worker's request for `action`, spoken in `voice`.
pub fn tts_request(action: &Action, voice: Option<&str>) -> Option<TtsRequest> {
    match action {
        Action::Speak(text) => Some(TtsRequest::Speak {
            text: text.clone(),
            voice: voice.map(str::to_string),
        }),
        Action::Stop => Some(TtsRequest::Stop),
        Action::Arm | Action::Vocabulary(_) => None,
    }
}

/// The STT worker's request for `action`.
pub fn stt_request(action: &Action) -> Option<SttRequest> {
    match action {
        Action::Arm => Some(SttRequest::Arm),
        _ => None,
    }
}

/// What the audio reports, to the companion (as [`AudioEvent::Input`]) or
/// to the front end.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    /// For [`Companion::send`](handlr_companion::Companion::send):
    /// `Utterance`, `Listening` or `Speaking`.
    Input(Input),
    /// The STT model is loaded; [`Event::Arm`] works from here.
    SttLoaded { load_ms: u32 },
    /// The STT model did not load. Nothing will be heard.
    SttLoadFailed(String),
    /// Phonon is loaded, with this many voices.
    TtsLoaded { load_ms: u32, voices: usize },
    /// Phonon did not load. Nothing will be said.
    TtsLoadFailed(String),
    /// The utterance so far.
    Partial(String),
    /// The earshot gate changed.
    Gate(GateState),
    /// The STT model's mean step over the last 25 frames (budget 80 ms),
    /// and the samples dropped since it loaded.
    Speed { step_ms: f32, dropped: u64 },
    /// One `Say` was synthesized.
    Spoken { audio_ms: u32, synth_ms: u32 },
    /// One `Say` failed.
    SpeakFailed(String),
    /// `vocab.txt` was written, with `phrases` lines; it is read at the next
    /// start.
    VocabularySaved {
        path: std::path::PathBuf,
        phrases: usize,
    },
    /// `vocab.txt` could not be written.
    VocabularyFailed(String),
}

/// Whether the gate counts as listening for [`Input::Listening`]: from the
/// tap (`Armed`) until the utterance has ended (`Idle`), as
/// [`Gate::listening`](libvoicexn::vad::Gate::listening) has it.
pub fn gate_listening(state: GateState) -> bool {
    state != GateState::Idle
}

/// The state behind [`Input::Speaking`] and [`Input::Listening`], so each
/// is given only when it changes.
///
/// Speaking starts when a `Say` is sent to the TTS worker and lasts until
/// the worker has nothing queued or playing *and* the speaker's ring has
/// run dry, so a notice is not released while the end of a reply is still
/// in the ring.
#[derive(Debug, Default)]
pub struct Pump {
    /// `Speak`s sent and not yet started.
    queued: usize,
    /// An utterance between `Started` and its last event.
    active: bool,
    /// The last `Speaking` given.
    speaking: bool,
    /// The last `Listening` given.
    listening: bool,
}

impl Pump {
    pub fn new() -> Self {
        Self::default()
    }

    /// Notes `action` going to the workers. `Speak` gives `Speaking(true)`
    /// at once if it is not in effect.
    pub fn on_action(&mut self, action: &Action) -> Option<AudioEvent> {
        match action {
            Action::Speak(_) => {
                self.queued += 1;
                self.set_speaking(true)
            }
            // The worker drops what it had queued; the utterance playing
            // ends with `Stopped`.
            Action::Stop => {
                self.queued = 0;
                None
            }
            Action::Arm | Action::Vocabulary(_) => None,
        }
    }

    /// Maps one STT event.
    pub fn on_stt(&mut self, event: SttEvent) -> Vec<AudioEvent> {
        match event {
            SttEvent::Loaded { load_ms } => vec![AudioEvent::SttLoaded { load_ms }],
            SttEvent::LoadFailed(e) => vec![AudioEvent::SttLoadFailed(e)],
            SttEvent::Partial(text) => vec![AudioEvent::Partial(text)],
            SttEvent::Utterance(text) => vec![AudioEvent::Input(Input::Utterance(text))],
            SttEvent::Speed { step_ms, dropped } => vec![AudioEvent::Speed { step_ms, dropped }],
            SttEvent::Gate(state) => {
                let mut out = vec![AudioEvent::Gate(state)];
                let listening = gate_listening(state);
                if listening != self.listening {
                    self.listening = listening;
                    out.push(AudioEvent::Input(Input::Listening(listening)));
                }
                out
            }
        }
    }

    /// Maps one TTS event. `Speaking(false)` waits for [`tick`](Self::tick).
    pub fn on_tts(&mut self, event: TtsEvent) -> Option<AudioEvent> {
        match event {
            TtsEvent::Loaded { voices, load_ms } => Some(AudioEvent::TtsLoaded {
                load_ms,
                voices: voices.len(),
            }),
            TtsEvent::LoadFailed(e) => {
                self.queued = 0;
                self.active = false;
                Some(AudioEvent::TtsLoadFailed(e))
            }
            TtsEvent::Started { .. } => {
                self.queued = self.queued.saturating_sub(1);
                self.active = true;
                // A `Speak` sent before a `Stop` can still start.
                self.set_speaking(true)
            }
            TtsEvent::FirstAudio { .. } => None,
            TtsEvent::Finished {
                audio_ms, synth_ms, ..
            } => {
                self.active = false;
                Some(AudioEvent::Spoken { audio_ms, synth_ms })
            }
            TtsEvent::Stopped { .. } => {
                self.active = false;
                None
            }
            TtsEvent::Failed { error, .. } => {
                self.active = false;
                Some(AudioEvent::SpeakFailed(error))
            }
        }
    }

    /// After the events of one poll: `Speaking(false)` once nothing is
    /// queued, nothing is being made and the ring is not `playing`.
    pub fn tick(&mut self, playing: bool) -> Option<AudioEvent> {
        let speaking = self.queued > 0 || self.active || playing;
        if !speaking {
            return self.set_speaking(false);
        }
        None
    }

    fn set_speaking(&mut self, speaking: bool) -> Option<AudioEvent> {
        if speaking == self.speaking {
            return None;
        }
        self.speaking = speaking;
        Some(AudioEvent::Input(Input::Speaking(speaking)))
    }
}

/// `vocab.txt` for `words`: a comment, then one phrase per line, in order,
/// without repeats. Line breaks inside a phrase become spaces; empty
/// phrases, and phrases that would read as comments, are left out.
pub fn vocab_text(words: &[String]) -> (String, usize) {
    let mut text = String::from("# The companion's vocabulary, written by handlr-audio.\n# libvoicexn reads it when the STT model loads.\n");
    let mut seen = std::collections::HashSet::new();
    for word in words {
        let phrase = word
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if phrase.is_empty() || phrase.starts_with('#') || !seen.insert(phrase.clone()) {
            continue;
        }
        text.push_str(&phrase);
        text.push('\n');
    }
    (text, seen.len())
}

/// Writes `vocab.txt` for `words` at `path`, through a temporary file and a
/// rename, creating its folder. Returns the phrases written; a file that
/// already says the same is left alone.
pub fn write_vocab(path: &std::path::Path, words: &[String]) -> std::io::Result<usize> {
    let (text, phrases) = vocab_text(words);
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(phrases);
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(phrases)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(events: &[AudioEvent]) -> Vec<Input> {
        events
            .iter()
            .filter_map(|e| match e {
                AudioEvent::Input(i) => Some(i.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn companion_events_become_actions() {
        assert_eq!(
            action(&Event::Say("Hello.".into())),
            Some(Action::Speak("Hello.".into()))
        );
        assert_eq!(action(&Event::StopSpeaking), Some(Action::Stop));
        assert_eq!(action(&Event::Arm), Some(Action::Arm));
        assert_eq!(
            action(&Event::Vocabulary(vec!["claude".into()])),
            Some(Action::Vocabulary(vec!["claude".into()]))
        );
        assert_eq!(
            action(&Event::Connected {
                server: "mac".into()
            }),
            None
        );
        assert_eq!(action(&Event::Error("x".into())), None);
    }

    #[test]
    fn actions_become_worker_requests() {
        let speak = tts_request(&Action::Speak("Hi.".into()), Some("alba"));
        assert!(matches!(
            speak,
            Some(TtsRequest::Speak { ref text, voice: Some(ref v) }) if text == "Hi." && v == "alba"
        ));
        assert!(matches!(
            tts_request(&Action::Stop, None),
            Some(TtsRequest::Stop)
        ));
        assert!(tts_request(&Action::Arm, None).is_none());
        assert!(matches!(stt_request(&Action::Arm), Some(SttRequest::Arm)));
        assert!(stt_request(&Action::Stop).is_none());
        assert!(stt_request(&Action::Vocabulary(vec![])).is_none());
    }

    #[test]
    fn an_utterance_is_an_input() {
        let mut pump = Pump::new();
        let out = pump.on_stt(SttEvent::Utterance("send continue to claude".into()));
        assert_eq!(
            inputs(&out),
            vec![Input::Utterance("send continue to claude".into())]
        );
        assert_eq!(
            pump.on_stt(SttEvent::Partial("send".into())),
            vec![AudioEvent::Partial("send".into())]
        );
    }

    #[test]
    fn listening_follows_the_gate_from_the_tap_to_idle() {
        let mut pump = Pump::new();
        let armed = pump.on_stt(SttEvent::Gate(GateState::Armed));
        assert_eq!(armed[0], AudioEvent::Gate(GateState::Armed));
        assert_eq!(inputs(&armed), vec![Input::Listening(true)]);
        assert!(inputs(&pump.on_stt(SttEvent::Gate(GateState::Open))).is_empty());
        assert!(inputs(&pump.on_stt(SttEvent::Gate(GateState::Closing))).is_empty());
        let idle = pump.on_stt(SttEvent::Gate(GateState::Idle));
        assert_eq!(inputs(&idle), vec![Input::Listening(false)]);
        // An arm that times out is still a change, and back.
        assert_eq!(
            inputs(&pump.on_stt(SttEvent::Gate(GateState::Armed))),
            vec![Input::Listening(true)]
        );
        assert_eq!(
            inputs(&pump.on_stt(SttEvent::Gate(GateState::Idle))),
            vec![Input::Listening(false)]
        );
    }

    #[test]
    fn speaking_lasts_until_the_ring_runs_dry() {
        let mut pump = Pump::new();
        let say = Action::Speak("Hello.".into());
        assert_eq!(
            pump.on_action(&say),
            Some(AudioEvent::Input(Input::Speaking(true)))
        );
        // Not started yet: still speaking.
        assert_eq!(pump.tick(false), None);
        assert_eq!(pump.on_tts(TtsEvent::Started { id: 1 }), None);
        assert_eq!(pump.on_tts(TtsEvent::FirstAudio { id: 1, ms: 40 }), None);
        assert_eq!(
            pump.on_tts(TtsEvent::Finished {
                id: 1,
                audio_ms: 900,
                synth_ms: 120
            }),
            Some(AudioEvent::Spoken {
                audio_ms: 900,
                synth_ms: 120
            })
        );
        // The end of the reply is still in the ring.
        assert_eq!(pump.tick(true), None);
        assert_eq!(
            pump.tick(false),
            Some(AudioEvent::Input(Input::Speaking(false)))
        );
        assert_eq!(pump.tick(false), None);
    }

    #[test]
    fn two_says_in_a_row_are_one_stretch_of_speaking() {
        let mut pump = Pump::new();
        assert!(pump.on_action(&Action::Speak("One.".into())).is_some());
        assert_eq!(pump.on_action(&Action::Speak("Two.".into())), None);
        pump.on_tts(TtsEvent::Started { id: 1 });
        pump.on_tts(TtsEvent::Finished {
            id: 1,
            audio_ms: 1,
            synth_ms: 1,
        });
        // "Two." is queued: the gap between them is not the end.
        assert_eq!(pump.tick(false), None);
        pump.on_tts(TtsEvent::Started { id: 2 });
        pump.on_tts(TtsEvent::Failed {
            id: 2,
            error: "no voice".into(),
        });
        assert_eq!(
            pump.tick(false),
            Some(AudioEvent::Input(Input::Speaking(false)))
        );
    }

    #[test]
    fn stop_drops_the_queue_and_speaking_ends_when_flushed() {
        let mut pump = Pump::new();
        pump.on_action(&Action::Speak("One.".into()));
        pump.on_action(&Action::Speak("Two.".into()));
        pump.on_tts(TtsEvent::Started { id: 1 });
        assert_eq!(pump.on_action(&Action::Stop), None);
        // Until the worker says `Stopped`, it may still be pushing.
        assert_eq!(pump.tick(false), None);
        assert_eq!(pump.on_tts(TtsEvent::Stopped { id: 1 }), None);
        assert_eq!(
            pump.tick(false),
            Some(AudioEvent::Input(Input::Speaking(false)))
        );
    }

    #[test]
    fn a_stop_while_idle_ends_speaking_once_the_ring_is_flushed() {
        let mut pump = Pump::new();
        pump.on_action(&Action::Speak("Long reply.".into()));
        pump.on_tts(TtsEvent::Started { id: 1 });
        pump.on_tts(TtsEvent::Finished {
            id: 1,
            audio_ms: 5000,
            synth_ms: 400,
        });
        pump.on_action(&Action::Stop);
        assert_eq!(pump.tick(true), None);
        assert_eq!(
            pump.tick(false),
            Some(AudioEvent::Input(Input::Speaking(false)))
        );
    }

    #[test]
    fn loading_events_are_reported() {
        let mut pump = Pump::new();
        assert_eq!(
            pump.on_stt(SttEvent::Loaded { load_ms: 900 }),
            vec![AudioEvent::SttLoaded { load_ms: 900 }]
        );
        assert_eq!(
            pump.on_tts(TtsEvent::Loaded {
                voices: vec!["a".into(), "b".into()],
                load_ms: 300
            }),
            Some(AudioEvent::TtsLoaded {
                load_ms: 300,
                voices: 2
            })
        );
        assert_eq!(
            pump.on_stt(SttEvent::Speed {
                step_ms: 30.5,
                dropped: 0
            }),
            vec![AudioEvent::Speed {
                step_ms: 30.5,
                dropped: 0
            }]
        );
    }

    #[test]
    fn vocab_text_is_one_clean_phrase_per_line() {
        let words: Vec<String> = [
            "claude",
            " codex ",
            "",
            "claude",
            "# not a comment",
            "two\nlines",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (text, n) = vocab_text(&words);
        let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(lines, vec!["claude", "codex", "two lines"]);
        assert_eq!(n, 3);
    }

    #[test]
    fn write_vocab_creates_the_folder_and_replaces_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handlr-companion").join("vocab.txt");
        assert_eq!(
            write_vocab(&path, &["claude".into(), "hr".into()]).unwrap(),
            2
        );
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.ends_with("claude\nhr\n"), "{first}");
        assert_eq!(write_vocab(&path, &["codex".into()]).unwrap(), 1);
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .ends_with("\ncodex\n"));
        assert!(!path.with_extension("txt.tmp").exists());
    }
}
