//! handlr's companion app (docs/design/20261022_04_COMPANION_APP_PLAN.md,
//! decision 8 and step 4 of the 18:10 EDT amendment): one libmorph app for
//! a desktop window and for Android, started as a copy of libvoicexn's
//! `examples/voicexn` at v0.2.0.
//!
//! - `handlr-companion` is the client: pairing, the link, turns and
//!   confirmations. The app sends it [`Input`]s and shows its [`Event`]s.
//! - `handlr-audio` is the voice: libvoicexn's STT and TTS, with cpal on the
//!   desktop and libmorph's audio on Android. Every event is handed to it,
//!   and its `Utterance`, `Listening` and `Speaking` go back to the client.
//! - Keys are in libmorph's key store and secure store (`keys.rs`). Where
//!   libmorph has neither (Linux), the desktop uses `FileKeyStore`; Android
//!   never falls back, and shows an error instead.
//!
//! A tap on no control is [`Input::Tap`]. Pairing takes a `handlr://` URL,
//! a `handlr1…` string or a `data:image/png;base64,` URL, through
//! [`App::open_url`]: typed into the field (desktop), given as `--pair`
//! (desktop), or pushed as `pair.txt` into the data directory (Android,
//! the plan's 16:23 EDT amendment), which the app reads and deletes at
//! start.
//!
//! Dictation (plan 20261023_02): "take a message" or the Take a message
//! button collects utterances into one message for herdr's focused agent,
//! shown on its card as it grows; Submit, Scratch and Clear are buttons. It
//! is submitted after `--dictation-silence <seconds>` (default and least 8)
//! of silence, set on Android by `dictation_silence.txt` in the data
//! directory, holding the seconds.
//!
//! The agent view (plan 20261022_03, step 4, `agent.rs`): tapping the focus
//! line opens the focused agent's transcript as terminal scrollback with
//! its priority cards; Back, or the system Back, returns. "Take a message"
//! there dictates to that agent, with its card in the view.

pub mod agent;
pub mod keys;
pub mod ui;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use handlr_audio::libvoicexn::config::SttQuant;
use handlr_audio::libvoicexn::vad::GateState;
use handlr_audio::{AudioConfig, AudioEvent, AudioSession};
use handlr_companion::{
    Companion, CompanionConfig, Dictation, Event, FileKeyStore, Input, KeyStore,
};
use libcontrols::DrawContext;
use libhandlr_proto::common::FocusInfo;
use libmorph::{
    AudioSetup, Frame, Key, KeyStatus, LimitsRequest, MorphApp, MorphContext, Touch, TouchPhase,
    wgpu,
};
use libmsdf::GpuSdfRenderer;
use libmsdf::drawlist::{AtlasId, AtlasSet, DrawList};
use libmsdf::font::{
    BUNDLED_ATLAS_PX_RANGE, FontAtlas, NOTO_SANS_MONO_ATLAS, ROBOTO_REGULAR_ASCII,
    ROBOTO_REGULAR_ATLAS, TERMINAL_SYMBOLS_ATLAS, TerminalShaper, TextShaper,
};

use crate::agent::{AgentHit, AgentScreen, Terminal};
use crate::keys::VaultKeyStore;
use crate::ui::{
    Buttons, CODE, Card, CardState, Hit, Layout, Line, MUTED, SPOKEN, Screen, TEXT, View, WARN,
    chunk, dictation_lines, focus_words, wrap,
};

/// The frame rate: enough for text, and far less heat than the display's
/// 120 Hz while STT listens.
const FPS: u32 = 15;

/// The longest pairing input the field takes: a data URL of a QR code PNG
/// is a few kilobytes.
const FIELD_MAX: usize = 16 * 1024;

/// The name the client gives in `hello`.
const CLIENT: &str = "companion";

/// The most lines of a dictated message's text the card shows: its tail.
const MESSAGE_LINES: usize = 8;

/// The desktop's command line (Android has none).
#[derive(Debug, Default)]
struct Args {
    models: Option<PathBuf>,
    quant: Option<SttQuant>,
    voice: Option<String>,
    pair: Option<String>,
    keys: Option<PathBuf>,
    name: Option<String>,
    /// Seconds of silence that submit a dictated message.
    dictation_silence: Option<Duration>,
}

/// Seconds as a silence: a positive number, else `None`.
fn parse_silence(text: &str) -> Option<Duration> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(Duration::from_secs_f64)
}

impl Args {
    fn parse(args: &[String]) -> Self {
        let mut out = Self::default();
        let mut args = args.iter().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().cloned();
            match arg.as_str() {
                "--models" => out.models = value().map(PathBuf::from),
                "--quant" => {
                    out.quant = value().and_then(|q| {
                        let quant = SttQuant::parse(&q);
                        if quant.is_none() {
                            log::error!("--quant {q:?}: one of q4k, q4_0, q8");
                        }
                        quant
                    })
                }
                "--voice" => out.voice = value(),
                "--pair" => out.pair = value(),
                "--keys" => out.keys = value().map(PathBuf::from),
                "--name" => out.name = value(),
                "--dictation-silence" => {
                    out.dictation_silence = value().and_then(|v| {
                        let silence = parse_silence(&v);
                        if silence.is_none() {
                            log::error!("--dictation-silence {v:?}: a number of seconds");
                        }
                        silence
                    })
                }
                other => log::warn!("unknown argument {other:?}"),
            }
        }
        out
    }
}

pub struct App {
    /// Roboto first, so text pushed with a plain atlas (libcontrols') draws
    /// from it; then the terminal face and its symbol page.
    fonts: AtlasSet,
    roboto: AtlasId,
    mono: AtlasId,
    symbols: AtlasId,
    shaper: TextShaper,
    terminal: TerminalShaper,
    sdf: GpuSdfRenderer,
    list: DrawList,
    screen: Screen,
    next_frame: Instant,
    /// `None` when there is no key storage (Android without a key store).
    companion: Option<Companion>,
    /// Started at init on the desktop, and once capture opens on Android.
    audio: Option<AudioSession>,
    audio_config: AudioConfig,
    /// The microphone's end when the desktop has no input device: kept, and
    /// never fed.
    #[cfg(not(target_os = "android"))]
    _mic: Option<handlr_audio::libvoicexn::capture::Mic>,
    /// The connection, for the first status line.
    link: String,
    /// The audio, for the second.
    sound: String,
    /// A server is stored, so a disconnect is a retry, not "not paired".
    paired: bool,
    heard: String,
    partial: String,
    said: String,
    /// The confirmation being asked, or the last one, with its outcome,
    /// until the next utterance (plan 20261023_01, section 4).
    card: Option<Card>,
    /// The device code to paste into `handlr pair`, and the server's name.
    code: Option<(String, String)>,
    /// A pairing note or an error, and its colour.
    note: Option<(String, [f32; 4])>,
    field: String,
    field_focused: bool,
    /// `hello` was answered on the current connection.
    online: bool,
    /// herdr's focus, as last told.
    focus: Option<FocusInfo>,
    /// The message being dictated, while the mode is on.
    dictation: Option<Dictation>,
    /// The agent view, while it is open.
    agent: Option<AgentScreen>,
    /// The clock touches and flings are timed by.
    started: Instant,
    /// When the last frame was updated, for a fling's step.
    last_update: Instant,
}

/// The models: `<data dir>/<name>` on Android, `--models <dir>` (default
/// `models/`) on the desktop.
fn audio_config(ctx: &MorphContext, args: &Args) -> AudioConfig {
    let models = match ctx.data_dir() {
        Some(dir) => dir.to_path_buf(),
        None => args
            .models
            .clone()
            .unwrap_or_else(|| PathBuf::from("models")),
    };
    let mut config = AudioConfig::new(&models);
    if let Some(dir) = ctx.data_dir() {
        // Next to the keys, which on Android are not files.
        config.vocab_file = Some(dir.join("vocab.txt"));
    }
    if let Some(quant) = args.quant {
        config.stt.quant = quant;
    }
    config.voice = args.voice.clone();
    config
}

/// The key storage, and the name of the paired server if there is one.
fn key_store(
    ctx: &MorphContext,
    args: &Args,
) -> Result<(Box<dyn KeyStore>, Option<String>), String> {
    let mut keys: Box<dyn KeyStore> = match (ctx.key_store(), ctx.secure_store(), &args.keys) {
        // `--keys <dir>` (desktop only: Android has no arguments).
        (_, _, Some(dir)) => {
            log::info!("keys: files in {}", dir.display());
            Box::new(FileKeyStore::new(dir))
        }
        (Some(vault), Some(store), None) => {
            let keys = VaultKeyStore::new(vault.clone(), store.clone());
            log::info!("keys: key store and secure store ({})", keys.describe());
            Box::new(keys)
        }
        _ if cfg!(target_os = "android") => {
            return Err("this device has no key store; the companion cannot keep its key".into());
        }
        _ => {
            let dir = FileKeyStore::default_dir();
            log::info!(
                "keys: no key store on this host; files in {}",
                dir.display()
            );
            Box::new(FileKeyStore::new(dir))
        }
    };
    let server = keys.server()?.map(|s| s.name);
    let key = keys.static_key()?.is_some();
    log::info!(
        "keys: static key {}, paired server {}",
        if key { "present" } else { "none yet" },
        if server.is_some() { "present" } else { "none" },
    );
    Ok((keys, server))
}

/// The dictation silence: `--dictation-silence` on the desktop, or
/// `dictation_silence.txt` in the data directory (Android's setting until
/// the app has a settings screen). The client raises anything under 8 s.
fn dictation_silence(ctx: &MorphContext, args: &Args) -> Option<Duration> {
    if args.dictation_silence.is_some() {
        return args.dictation_silence;
    }
    let path = ctx.data_dir()?.join("dictation_silence.txt");
    let text = std::fs::read_to_string(&path).ok()?;
    let silence = parse_silence(&text);
    if silence.is_none() {
        log::warn!("{}: not a number of seconds", path.display());
    }
    silence
}

/// The device name to pair as: the phone's model on Android, the hostname
/// on the desktop.
fn device_name(args: &Args) -> String {
    if let Some(name) = &args.name {
        return name.clone();
    }
    if cfg!(target_os = "android") {
        let model = std::process::Command::new("getprop")
            .arg("ro.product.model")
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok());
        if let Some(name) = model
            .as_deref()
            .and_then(handlr_companion::device_name_from)
        {
            return name;
        }
    }
    handlr_companion::default_device_name()
}

impl App {
    /// The app's handling of a pairing URL, however it arrives: the field,
    /// `--pair`, `pair.txt`, and, once libmorph has URL schemes, an opened
    /// `handlr://` link. The text is never logged.
    pub fn open_url(&mut self, url: &str) {
        let text = url.trim();
        if text.is_empty() {
            return;
        }
        match &self.companion {
            Some(companion) => {
                companion.send(Input::Pair(text.to_string()));
                self.note = Some(("pairing\u{2026}".into(), MUTED));
            }
            None => self.note = Some(("no key storage: cannot pair".into(), WARN)),
        }
    }

    fn send(&self, input: Input) {
        if let Some(companion) = &self.companion {
            companion.send(input);
        }
    }

    /// Starts the audio on libmorph's capture and output (Android).
    #[cfg(target_os = "android")]
    fn start_morph_audio(&mut self, ctx: &mut MorphContext) {
        let (Some(capture), Some(output)) = (ctx.audio().capture_format(), ctx.audio().format())
        else {
            log::warn!("audio: no capture or no output");
            self.sound = "no audio device".into();
            return;
        };
        log::info!(
            "audio: capture {} Hz, {} channels; playback {} Hz, {} channels",
            capture.sample_rate,
            capture.channels,
            output.sample_rate,
            output.channels
        );
        let output = handlr_audio::Format {
            rate: output.sample_rate,
            channels: output.channels,
        };
        let (session, sink, source) =
            AudioSession::spawn_morph(self.audio_config.clone(), capture, output);
        ctx.audio().add_sink(sink);
        ctx.audio().add_source(source);
        self.audio = Some(session);
        self.sound = "loading the voice\u{2026}".into();
    }

    fn on_event(&mut self, event: Event) {
        if let Some(audio) = &mut self.audio {
            audio.handle(&event);
        }
        match event {
            Event::Connected { server } => {
                log::info!("link: connected to {server}");
                self.link = format!("connected to {server}");
                self.online = true;
            }
            Event::Disconnected { retry_in_ms } => {
                self.online = false;
                self.link = if self.paired {
                    format!(
                        "disconnected \u{b7} retrying in {} s",
                        retry_in_ms.div_ceil(1000)
                    )
                } else {
                    "not paired \u{b7} paste a pairing URL below".into()
                };
            }
            Event::PairingCode { code, server } => {
                // A public key, safe to log, and readable over adb.
                log::info!("pairing: device code {code}");
                self.paired = true;
                self.code = Some((code, server));
                self.note = None;
            }
            Event::Paired { name } => {
                log::info!("pairing: paired as {name}");
                self.code = None;
                self.note = Some((format!("paired as {name}"), MUTED));
            }
            Event::PairFailed(why) => {
                log::warn!("pairing: refused");
                self.note = Some((format!("pairing failed: {why}"), WARN));
            }
            Event::Say(text) => self.said = text,
            // The card shows the handle; only `text` is spoken.
            Event::Confirm {
                confirm_id,
                label,
                command,
                agent,
                project,
                ..
            } => {
                self.card = Some(Card {
                    confirm_id,
                    command,
                    agent,
                    project,
                    handle: label,
                    state: CardState::Waiting,
                })
            }
            Event::Answered {
                confirm_id,
                approve,
            } => self.set_card(confirm_id, CardState::Answering { approve }),
            Event::ReplyFailed {
                confirm_id,
                message,
            } => {
                log::warn!("confirmation {confirm_id}: {message}");
                self.set_card(confirm_id, CardState::Refused(message));
            }
            Event::Resolved {
                confirm_id,
                outcome,
            } => {
                log::info!("confirmation {confirm_id}: {outcome}");
                self.set_card(confirm_id, CardState::Done(outcome));
            }
            Event::TurnEnded { outcome, .. } => log::info!("turn: {outcome}"),
            Event::Error(e) => {
                log::warn!("companion: {e}");
                self.note = Some((e, WARN));
            }
            Event::Focus(focus) => self.focus = focus,
            Event::Dictation(d) => {
                if d.active != self.dictation.is_some() {
                    log::info!("dictation: {}", if d.active { "on" } else { "off" });
                }
                self.dictation = d.active.then_some(d);
            }
            Event::AgentView {
                name,
                status,
                entries,
                cards,
                pinned,
            } => {
                if let Some(agent) = &mut self.agent {
                    agent.set_content(name, status, entries, cards, pinned);
                }
            }
            Event::StopSpeaking | Event::Arm | Event::Vocabulary(_) => {}
        }
    }

    /// Opens the view of the focused agent.
    fn open_agent(&mut self) {
        let Some(focus) = &self.focus else {
            return;
        };
        log::info!("agent view: open");
        self.send(Input::OpenAgent(focus.pane_id.clone()));
        let mut agent = AgentScreen::new();
        agent.name = focus.name.clone();
        self.agent = Some(agent);
    }

    /// Closes the agent view, back to the main screen.
    fn close_agent(&mut self) {
        if self.agent.take().is_some() {
            log::info!("agent view: closed");
            self.send(Input::CloseAgent);
            self.screen.invalidate();
        }
    }

    /// A tap on one of the agent view's controls.
    fn agent_hit(&mut self, hit: AgentHit) {
        match hit {
            AgentHit::Back => self.close_agent(),
            AgentHit::Message => self.send(Input::StartDictation),
            AgentHit::Approve(confirm_id) => self.send(Input::Answer {
                confirm_id,
                approve: true,
            }),
            AgentHit::Deny(confirm_id) => self.send(Input::Answer {
                confirm_id,
                approve: false,
            }),
            AgentHit::Pin => {
                if let Some(agent) = &mut self.agent {
                    agent.jump_to_pin();
                }
            }
            AgentHit::Suggest(suggestion) => self.send(Input::Respond(suggestion)),
            AgentHit::Submit => self.send(Input::Submit),
            AgentHit::Scratch => self.send(Input::Scratch),
            AgentHit::Clear => self.send(Input::Clear),
        }
    }

    /// Moves the card for `confirm_id` to `state`; another's is left alone.
    fn set_card(&mut self, confirm_id: u64, state: CardState) {
        if let Some(card) = self.card.as_mut().filter(|c| c.confirm_id == confirm_id) {
            card.state = state;
        }
    }

    fn on_audio(&mut self, event: AudioEvent) {
        match event {
            AudioEvent::Input(input) => {
                match &input {
                    Input::Utterance(text) => {
                        self.heard = text.clone();
                        self.partial.clear();
                        // A new utterance: an answered card has had its say.
                        if self
                            .card
                            .as_ref()
                            .is_some_and(|c| matches!(c.state, CardState::Done(_)))
                        {
                            self.card = None;
                        }
                    }
                    Input::Listening(false) => self.partial.clear(),
                    _ => {}
                }
                self.send(input);
            }
            AudioEvent::SttLoaded { load_ms } => {
                log::info!("audio: STT loaded in {load_ms} ms");
                self.sound = "tap to talk".into();
            }
            AudioEvent::SttLoadFailed(e) => {
                log::error!("audio: STT: {e}");
                self.sound = format!("STT model missing: {e}");
            }
            AudioEvent::TtsLoaded { load_ms, voices } => {
                log::info!("audio: TTS loaded in {load_ms} ms, {voices} voices");
            }
            AudioEvent::TtsLoadFailed(e) => {
                log::error!("audio: TTS: {e}");
                self.sound = format!("TTS model missing: {e}");
            }
            AudioEvent::Partial(text) => self.partial = text,
            AudioEvent::Gate(state) => {
                self.sound = match state {
                    GateState::Idle => "tap to talk",
                    GateState::Armed => "listening\u{2026}",
                    GateState::Open => "hearing",
                    GateState::Closing => "finishing",
                }
                .into();
            }
            AudioEvent::Speed { step_ms, dropped } => {
                log::debug!("audio: STT step {step_ms:.1} ms, dropped {dropped}");
            }
            AudioEvent::Spoken { audio_ms, synth_ms } => {
                log::info!("audio: said {audio_ms} ms in {synth_ms} ms");
            }
            AudioEvent::SpeakFailed(e) => log::warn!("audio: TTS: {e}"),
            AudioEvent::VocabularySaved { phrases, .. } => {
                log::info!("audio: vocabulary of {phrases} phrases, for the next start");
            }
            AudioEvent::VocabularyFailed(e) => log::warn!("audio: vocabulary: {e}"),
        }
    }

    fn pair(&mut self) {
        let text = std::mem::take(&mut self.field);
        self.field_focused = false;
        self.open_url(&text);
    }

    /// Gives the screen what it shows now; it changes only what differs.
    fn lay_out(&mut self, ctx: &MorphContext) {
        let (w, h) = ctx.surface_size();
        let safe = ctx.safe_area();
        let layout = Layout {
            w: w as f32,
            h: h as f32,
            scale: ctx.scale_factor(),
            top: safe.top,
            bottom: safe.bottom,
        };
        let line = |text: String, size: f32, color, gap| Line {
            text,
            size,
            color,
            gap,
        };
        let mut body = Vec::new();
        let mut push = |text: &str, size: f32, color, gap: f32| {
            for (i, t) in wrap(text, layout.columns(size)).into_iter().enumerate() {
                body.push(line(t, size, color, if i == 0 { gap } else { 0.0 }));
            }
        };
        // The focus line: tapping it opens the agent view.
        let mut focus_lines = 0;
        if self.dictation.is_none()
            && let Some(focus) = &self.focus
        {
            let text = format!("focus: {}", focus_words(focus));
            focus_lines = wrap(&text, layout.columns(15.0)).len();
            push(&text, 15.0, MUTED, 0.0);
        }
        if !self.partial.is_empty() {
            push(&format!("> {}\u{2026}", self.partial), 20.0, MUTED, 24.0);
        } else if !self.heard.is_empty() {
            push(&format!("> {}", self.heard), 20.0, TEXT, 24.0);
        }
        if !self.said.is_empty() {
            push(&format!("< {}", self.said), 19.0, SPOKEN, 18.0);
        }
        let asking = self.card.as_ref().is_some_and(Card::asks);
        if let Some(d) = self.dictation.as_ref().filter(|_| !asking) {
            for l in dictation_lines(d) {
                let text = if l.text == d.text {
                    // The tail of a long message.
                    let lines = wrap(&l.text, layout.columns(l.size));
                    let skip = lines.len().saturating_sub(MESSAGE_LINES);
                    if skip > 0 {
                        format!("\u{2026} {}", lines[skip..].join(" "))
                    } else {
                        l.text
                    }
                } else {
                    l.text
                };
                push(&text, l.size, l.color, l.gap);
            }
        }
        if let Some(card) = &self.card {
            for l in card.lines() {
                push(&l.text, l.size, l.color, l.gap);
            }
        }

        let mut code = Vec::new();
        if let Some((device_code, server)) = &self.code {
            code.push(line(
                format!("Paste this code into handlr pair on {server}:"),
                15.0,
                MUTED,
                0.0,
            ));
            // Base58 is mostly capitals and wide letters: wider than the
            // half an em `columns` assumes.
            for part in chunk(device_code, layout.columns(26.0 * 1.3)) {
                code.push(line(part, 26.0, CODE, 4.0));
            }
        }
        if let Some((note, color)) = &self.note {
            for (i, t) in wrap(note, layout.columns(15.0)).into_iter().enumerate() {
                code.push(line(t, 15.0, *color, if i == 0 { 8.0 } else { 0.0 }));
            }
        }

        if let Some(agent) = &mut self.agent {
            agent.set_layout(layout);
            agent.set_dictation(self.dictation.as_ref().map(dictation_lines));
        }
        let view = View {
            status: vec![self.link.clone(), self.sound.clone()],
            body,
            focus_lines,
            buttons: if asking {
                Buttons::Confirm
            } else if self.dictation.is_some() {
                Buttons::Dictation
            } else if self.online {
                Buttons::Start
            } else {
                Buttons::None
            },
            code,
            field: self.field.clone(),
            field_focused: self.field_focused,
            paired: self.paired,
        };
        self.screen.set(layout, view);
    }
}

impl MorphApp for App {
    /// The namespace of the key store and the secure store:
    /// `libmorph.handlr-companion`, on Android and in the macOS Keychain.
    const SECURE_STORE_ID: &'static str = "handlr-companion";

    /// libmsdf's fragment stage binds five storage buffers; the downlevel
    /// defaults allow four.
    fn limits(base: &wgpu::Limits) -> LimitsRequest {
        let required = wgpu::Limits {
            max_storage_buffers_per_shader_stage: base.max_storage_buffers_per_shader_stage.max(8),
            ..base.clone()
        };
        LimitsRequest {
            preferred: required.clone(),
            required,
        }
    }

    /// On Android, the microphone, in the preset for speech recognition.
    /// The desktop opens its devices with cpal instead.
    fn audio_setup(setup: &mut AudioSetup) {
        #[cfg(target_os = "android")]
        {
            setup.capture = true;
            setup.capture_mode = libmorph::CaptureMode::VoiceRecognition;
        }
        #[cfg(not(target_os = "android"))]
        let _ = setup;
    }

    fn init(ctx: &mut MorphContext) -> Self {
        let parse = |bytes| FontAtlas::from_bytes(bytes).expect("bundled atlas parses");
        let (text, page) = (parse(NOTO_SANS_MONO_ATLAS), parse(TERMINAL_SYMBOLS_ATLAS));
        let terminal = TerminalShaper::new(&text, &page).expect("bundled terminal atlases agree");
        let mut fonts = AtlasSet::new();
        let mut add = |atlas| fonts.register(atlas).expect("bundled atlases fit one set");
        let (roboto, mono, symbols) = (add(parse(ROBOTO_REGULAR_ATLAS)), add(text), add(page));
        let shaper = TextShaper::new(ROBOTO_REGULAR_ASCII.to_vec()).expect("bundled face parses");
        let sdf = GpuSdfRenderer::new_with_atlas_set(ctx.device(), ctx.surface_format(), &fonts);
        sdf.upload_atlas_set(ctx.queue(), &fonts);

        let args = Args::parse(ctx.args());
        let audio_config = audio_config(ctx, &args);
        let mut app = Self {
            fonts,
            roboto,
            mono,
            symbols,
            shaper,
            terminal,
            sdf,
            list: DrawList::new(),
            screen: Screen::new(),
            next_frame: Instant::now(),
            companion: None,
            audio: None,
            audio_config,
            #[cfg(not(target_os = "android"))]
            _mic: None,
            link: "starting\u{2026}".into(),
            sound: "waiting for the microphone\u{2026}".into(),
            paired: false,
            heard: String::new(),
            partial: String::new(),
            said: String::new(),
            card: None,
            code: None,
            note: None,
            field: String::new(),
            field_focused: false,
            online: false,
            focus: None,
            dictation: None,
            agent: None,
            started: Instant::now(),
            last_update: Instant::now(),
        };

        match key_store(ctx, &args) {
            Ok((keys, server)) => {
                app.paired = server.is_some();
                app.link = match server {
                    Some(name) => format!("connecting to {name}\u{2026}"),
                    None => "not paired \u{b7} paste a pairing URL below".into(),
                };
                let mut config = CompanionConfig::new(CLIENT);
                config.device_name = device_name(&args);
                config.voice = args.voice.clone();
                if let Some(silence) = dictation_silence(ctx, &args) {
                    config.dictation_silence = silence;
                }
                log::info!(
                    "companion: device name {}, dictation silence {:.1} s",
                    config.device_name,
                    config
                        .dictation_silence
                        .max(handlr_companion::DICTATION_SILENCE)
                        .as_secs_f64()
                );
                app.companion = Some(Companion::spawn(config, keys));
            }
            Err(e) => {
                log::error!("keys: {e}");
                app.link = "no key storage".into();
                app.note = Some((e, WARN));
            }
        }

        #[cfg(not(target_os = "android"))]
        match AudioSession::open_default(app.audio_config.clone()) {
            Ok(session) => {
                app.audio = Some(session);
                app.sound = "loading the voice\u{2026}".into();
            }
            // No microphone (a Mac mini): the replies can still be heard.
            Err(e) => {
                log::error!("audio: {e}; trying the speaker alone");
                let silent = handlr_audio::Format {
                    rate: 24_000,
                    channels: 1,
                };
                match AudioSession::open_default_output(app.audio_config.clone(), silent) {
                    Ok((session, mic)) => {
                        app.audio = Some(session);
                        app._mic = Some(mic);
                        app.sound = format!("no microphone ({e})");
                    }
                    Err(e) => {
                        log::error!("audio: {e}");
                        app.sound = format!("no audio: {e}");
                    }
                }
            }
        }

        if let Some(text) = args.pair.as_deref() {
            app.open_url(text);
        }
        // Android: a pairing input pushed with scripts/push-pair.sh. It is
        // read once and deleted, whatever comes of it.
        if let Some(path) = ctx.data_dir().map(|dir| dir.join("pair.txt"))
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            log::info!("pairing: read {}", path.display());
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("pairing: deleting {}: {e}", path.display());
            }
            app.open_url(&text);
        }
        // The launch Intent's data (`am start -d <url>`) cannot reach the app
        // yet: libmobile delivers it as `MobileEvent::Intent`, but libmorph
        // 0.3.1's Android runner drops that event. Once libmorph releases its
        // URL schemes (plan 20261022_07, Draft), `on_open_url` calls
        // `open_url`, and build.rs adds `.url_scheme("handlr")`.
        app
    }

    #[cfg(target_os = "android")]
    fn on_capture_ready(&mut self, ctx: &mut MorphContext) {
        self.start_morph_audio(ctx);
    }

    fn update(&mut self, ctx: &mut MorphContext, _dt: Duration) {
        // Hold the frame: libmorph renders as fast as presentation allows,
        // and the CPU and GPU heat the phone.
        let now = Instant::now();
        if now < self.next_frame {
            std::thread::sleep(self.next_frame - now);
        }
        self.next_frame = self.next_frame.max(now) + Duration::from_secs(1) / FPS;

        let events: Vec<Event> = match &self.companion {
            Some(companion) => companion.poll().collect(),
            None => Vec::new(),
        };
        for event in events {
            self.on_event(event);
        }
        let sounds = match &mut self.audio {
            Some(audio) => audio.poll(),
            None => Vec::new(),
        };
        for event in sounds {
            self.on_audio(event);
        }
        let dt = now.saturating_duration_since(self.last_update);
        self.last_update = now;
        if let Some(agent) = &mut self.agent {
            agent.tick(dt);
        }
        self.lay_out(ctx);
    }

    /// The buttons and the field, and a tap anywhere else. In the agent
    /// view, every phase goes to it: a drag scrolls, a tap hits.
    fn on_touch(&mut self, _ctx: &mut MorphContext, touch: Touch) {
        if let Some(agent) = &mut self.agent {
            let now = self.started.elapsed();
            if let Some(hit) = agent.touch(touch.id, touch.phase, touch.x, touch.y, now) {
                self.agent_hit(hit);
            }
            return;
        }
        if touch.phase != TouchPhase::Ended {
            return;
        }
        match self.screen.hit(touch.x, touch.y) {
            Some(Hit::Focus) => self.open_agent(),
            Some(Hit::Approve) => self.send(Input::Approve),
            Some(Hit::Deny) => self.send(Input::Deny),
            Some(Hit::Start) => self.send(Input::StartDictation),
            Some(Hit::Submit) => self.send(Input::Submit),
            Some(Hit::Scratch) => self.send(Input::Scratch),
            Some(Hit::Clear) => self.send(Input::Clear),
            Some(Hit::Field) => self.field_focused = true,
            Some(Hit::Pair) => self.pair(),
            None => {
                self.field_focused = false;
                self.send(Input::Tap);
            }
        }
    }

    /// Typing into the focused field (desktop): Enter pairs, Escape clears.
    /// libmorph has no Backspace, paste or on-screen keyboard yet.
    fn on_key(&mut self, _ctx: &mut MorphContext, key: Key) -> KeyStatus {
        // The system Back closes the agent view.
        if self.agent.is_some() && matches!(key, Key::Back | Key::Escape) {
            self.close_agent();
            return KeyStatus::Handled;
        }
        if !self.field_focused {
            return KeyStatus::Unhandled;
        }
        match key {
            Key::Character('\r' | '\n') => self.pair(),
            Key::Character('\u{8}' | '\u{7f}') => {
                self.field.pop();
            }
            Key::Character(c) if !c.is_control() => {
                if self.field.len() < FIELD_MAX {
                    self.field.push(c);
                }
            }
            Key::Escape | Key::Back => {
                self.field.clear();
                self.field_focused = false;
            }
            _ => return KeyStatus::Unhandled,
        }
        KeyStatus::Handled
    }

    fn render(&mut self, ctx: &mut MorphContext, frame: &Frame<'_>) {
        let font = |id| self.fonts.atlas(id).expect("registered in this set");
        let draw = DrawContext {
            px_range: BUNDLED_ATLAS_PX_RANGE,
            ..DrawContext::new(Some(font(self.roboto).atlas), Some(&self.shaper))
        };
        let term = Terminal {
            shaper: &self.terminal,
            text: font(self.mono),
            symbols: font(self.symbols),
            px_range: BUNDLED_ATLAS_PX_RANGE,
        };
        match &mut self.agent {
            Some(agent) => agent.draw(&mut self.list, &draw, Some(&term)),
            None => self.screen.draw(&mut self.list, &draw),
        };
        let (w, h) = frame.size;
        self.sdf.render_draw_list_in(
            ctx.device(),
            ctx.queue(),
            frame.view,
            w,
            h,
            1.0,
            &self.list,
            &self.fonts,
            0.0,
        );
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: libmorph::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("handlr"),
    );
    // Before any worker starts: rayon's default of one thread per core is
    // too many on phones that mix fast and slow cores, and STT needs 4 on
    // the S23 to keep up with 80 ms frames (as voicexn).
    // SAFETY: no other thread has started yet.
    unsafe {
        std::env::set_var(
            "RAYON_NUM_THREADS",
            option_env!("VOICEXN_THREADS").unwrap_or("4"),
        )
    };
    libmorph::run_android::<App>(app);
}
