//! Audio for the companion front ends
//! (docs/design/20261022_04_COMPANION_APP_PLAN.md, 18:10 EDT amendment).
//!
//! An [`AudioSession`] owns libvoicexn's TTS and STT workers, the earshot
//! gate behind the STT worker, and the speaker's ring. It joins them to
//! `handlr-companion`:
//!
//! - [`AudioSession::handle`] takes the companion's [`Event`]s: `Say` is
//!   spoken, `StopSpeaking` stops Phonon and flushes the ring, `Arm` arms
//!   the gate, `Vocabulary` writes `vocab.txt`, read when the STT model
//!   next loads, and `Ding` plays a short built-in chime over whatever the
//!   speaker plays (plan 20261024_02, as approved at 11:46 EDT).
//! - [`AudioSession::poll`] gives [`AudioEvent`]s. Those in
//!   [`AudioEvent::Input`] (`Utterance`, `Listening`, `Speaking`) go to
//!   [`Companion::send`](handlr_companion::Companion::send); the others are
//!   for the front end to show or log.
//!
//! The devices are joined through two callbacks, [`Mic::push`] and
//! [`Playback::fill`]. [`AudioSession::spawn`] hands them to the caller;
//! with `cpal`, [`AudioSession::open_default`] opens the default devices
//! with them, and with `morph`, [`AudioSession::spawn_morph`] turns them
//! into a libmorph `Sink` and `Source` (the adapters in `morph.rs`).
//!
//! The mapping itself is in [`pump`], with no devices or models.

#[cfg(feature = "cpal")]
mod devices;
#[cfg(feature = "morph")]
mod morph;
mod playback;
pub mod pump;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use handlr_companion::{Event, FileKeyStore};
use libvoicexn::capture::Mic;
use libvoicexn::config::{SttConfig, TtsConfig};
use libvoicexn::stt::SttWorker;
use libvoicexn::tts::TtsWorker;

pub use libvoicexn;
pub use playback::Playback;
pub use pump::{Action, AudioEvent};

/// The speaker's ring, in seconds of output.
const PLAYBACK_SECONDS: usize = 2;
/// The microphone's ring, in seconds of 24 kHz audio.
const CAPTURE_SECONDS: u32 = 4;

/// What the session loads, and where it writes the vocabulary.
#[derive(Clone, Debug)]
pub struct AudioConfig {
    /// Kyutai STT: the model folder, the quantization and the boost.
    pub stt: SttConfig,
    /// Phonon: the checkpoint folder and quantization.
    pub tts: TtsConfig,
    /// The Phonon voice for every `Say`; `None` is the checkpoint's own.
    pub voice: Option<String>,
    /// `vocab.txt`, written on `Vocabulary`. If it exists when the session
    /// starts, the STT model is boosted with it, in place of the list
    /// [`SttConfig::for_platform`] found. `None` writes nothing.
    pub vocab_file: Option<PathBuf>,
}

impl AudioConfig {
    /// libvoicexn's layout under `models`: `phonon-31m-en` and
    /// `stt-1b-en_fr`, with the platform's defaults and the `VOICEXN_*`
    /// overrides, and the vocabulary next to the companion's keys
    /// (`~/.config/handlr-companion/vocab.txt`).
    pub fn new(models: impl AsRef<Path>) -> Self {
        let models = models.as_ref();
        Self {
            stt: SttConfig::for_platform(models.join("stt-1b-en_fr")),
            tts: TtsConfig::for_platform(models.join("phonon-31m-en")),
            voice: None,
            vocab_file: Some(FileKeyStore::default_dir().join("vocab.txt")),
        }
    }
}

impl Default for AudioConfig {
    /// [`AudioConfig::new`] with `models` in the working directory, as
    /// libvoicexn's examples run.
    fn default() -> Self {
        Self::new("models")
    }
}

/// A device's rate and interleaved channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
}

/// The workers, and the session's half of the device callbacks. With
/// `cpal` from [`open_default`](Self::open_default) it also holds the
/// streams, which are not `Send` on every platform.
pub struct AudioSession {
    tts: TtsWorker,
    stt: SttWorker,
    pump: pump::Pump,
    voice: Option<String>,
    vocab_file: Option<PathBuf>,
    /// Moved on every `Stop`; [`Playback`] drains the ring when it moves.
    flush: Arc<AtomicU32>,
    /// Moved on every `Chime`; [`Playback`] starts the chime when it moves.
    chimes: Arc<AtomicU32>,
    /// Set while the speaker's last buffer played something.
    playing: Arc<AtomicBool>,
    /// Events made by [`handle`](Self::handle), given by the next poll.
    pending: Vec<AudioEvent>,
    #[cfg(feature = "cpal")]
    _streams: Option<devices::Streams>,
}

impl AudioSession {
    /// Starts the workers for a microphone in `input` and a speaker in
    /// `output`, and returns the two device callbacks' ends: feed the input
    /// callback's samples to [`Mic::push`], and fill the output callback's
    /// buffer with [`Playback::fill`]. The models load on the workers'
    /// threads; [`poll`](Self::poll) says when.
    pub fn spawn(config: AudioConfig, input: Format, output: Format) -> (Self, Mic, Playback) {
        let capacity = output.rate as usize * PLAYBACK_SECONDS;
        let (producer, consumer) =
            libvoicexn::playback::ring(capacity, output.channels, output.rate);
        let playing = consumer.speaking();
        // Muted while the speaker plays, so STT does not hear Phonon.
        let (mic, capture) = Mic::new(input.rate, input.channels, playing.clone(), CAPTURE_SECONDS);
        let flush = Arc::new(AtomicU32::new(0));
        let chimes = Arc::new(AtomicU32::new(0));
        let playback = Playback::new(
            consumer,
            capacity * usize::from(output.channels.max(1)),
            flush.clone(),
        )
        .with_chime(output, chimes.clone());

        let mut stt_config = config.stt;
        if let Some(path) = config.vocab_file.as_ref().filter(|p| p.is_file()) {
            stt_config.vocab = Some(path.clone());
        }
        log::info!(
            "audio: stt {} ({:?}), vocab {:?}; tts {}",
            stt_config.model_dir.display(),
            stt_config.quant,
            stt_config.vocab,
            config.tts.model_dir.display()
        );
        let tts = TtsWorker::spawn(config.tts, Some(producer));
        let stt = SttWorker::spawn(stt_config, capture);
        let session = Self {
            tts,
            stt,
            pump: pump::Pump::new(),
            voice: config.voice,
            vocab_file: config.vocab_file,
            flush,
            chimes,
            playing,
            pending: Vec::new(),
            #[cfg(feature = "cpal")]
            _streams: None,
        };
        (session, mic, playback)
    }

    /// Opens the default input and output devices with cpal and starts the
    /// session on them.
    #[cfg(feature = "cpal")]
    pub fn open_default(config: AudioConfig) -> Result<Self, String> {
        let (input, output) = (devices::Device::input()?, devices::Device::output()?);
        let (mut session, mic, playback) = Self::spawn(config, input.format(), output.format());
        session._streams = Some(devices::Streams {
            _output: output.play(playback)?,
            _input: Some(input.capture(mic)?),
        });
        Ok(session)
    }

    /// Opens only the default output device with cpal, for a microphone the
    /// caller feeds through the returned [`Mic`] in `input`'s format: a
    /// machine with no input device, or audio from a file.
    #[cfg(feature = "cpal")]
    pub fn open_default_output(config: AudioConfig, input: Format) -> Result<(Self, Mic), String> {
        let output = devices::Device::output()?;
        let (mut session, mic, playback) = Self::spawn(config, input, output.format());
        session._streams = Some(devices::Streams {
            _output: output.play(playback)?,
            _input: None,
        });
        Ok((session, mic))
    }

    /// Starts the session for libmorph's audio: a `Sink` for
    /// `Audio::add_sink` in `capture`'s format, and a `Source` for a speaker
    /// in `output`.
    #[cfg(feature = "morph")]
    pub fn spawn_morph(
        config: AudioConfig,
        capture: libmorph::CaptureFormat,
        output: Format,
    ) -> (Self, libmorph::Sink, libmorph::Source) {
        let input = Format {
            rate: capture.sample_rate,
            channels: capture.channels,
        };
        let (session, mic, playback) = Self::spawn(config, input, output);
        (session, morph::sink(mic), morph::source(playback))
    }

    /// Acts on a companion event. Events that are not the audio's are
    /// ignored, so every event can be passed here.
    pub fn handle(&mut self, event: &Event) {
        if let Some(action) = pump::action(event) {
            self.act(action);
        }
    }

    /// Carries out one [`Action`].
    pub fn act(&mut self, action: Action) {
        if let Some(e) = self.pump.on_action(&action) {
            self.pending.push(e);
        }
        if let Some(req) = pump::tts_request(&action, self.voice.as_deref()) {
            self.tts.send(req);
        }
        if let Some(req) = pump::stt_request(&action) {
            self.stt.send(req);
        }
        match action {
            Action::Stop => {
                self.flush.fetch_add(1, Ordering::Release);
            }
            Action::Vocabulary(words) => self.pending.push(self.save_vocabulary(&words)),
            Action::Chime => {
                self.chimes.fetch_add(1, Ordering::Release);
            }
            Action::Speak(_) | Action::Arm => {}
        }
    }

    /// What has happened since the last poll, without waiting. Call it
    /// often (every 20 ms or so): `Speaking(false)` is noticed here.
    pub fn poll(&mut self) -> Vec<AudioEvent> {
        let mut out = std::mem::take(&mut self.pending);
        for event in self.stt.poll() {
            out.extend(self.pump.on_stt(event));
        }
        for event in self.tts.poll() {
            out.extend(self.pump.on_tts(event));
        }
        out.extend(self.pump.tick(self.playing.load(Ordering::Relaxed)));
        out
    }

    /// Where `Vocabulary` is written, if anywhere.
    pub fn vocab_file(&self) -> Option<&Path> {
        self.vocab_file.as_deref()
    }

    fn save_vocabulary(&self, words: &[String]) -> AudioEvent {
        let Some(path) = &self.vocab_file else {
            return AudioEvent::VocabularyFailed("no vocabulary file configured".into());
        };
        match pump::write_vocab(path, words) {
            Ok(phrases) => AudioEvent::VocabularySaved {
                path: path.clone(),
                phrases,
            },
            Err(e) => AudioEvent::VocabularyFailed(format!("{}: {e}", path.display())),
        }
    }
}
