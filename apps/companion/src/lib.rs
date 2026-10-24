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
//! The home screen is the overview (plan 20261023_04, `overview.rs`): the
//! agents' cards on the whole screen, the newest Gemini line in a strip
//! under them, and the confirmation and dictation cards and the bottom row
//! under that. A tap on the strip slides the conversation with Gemini up
//! over the cards; the system Back closes it. The pairing screen (`ui.rs`) shows instead while
//! the phone is neither paired nor connected, or shows its device code.
//!
//! Dictation (plan 20261023_02): "take a message" or the record button
//! collects utterances into one message for herdr's focused agent, shown
//! on its card as it grows; send and cancel are buttons, and "scratch
//! that" removes the last utterance. It is submitted after
//! `--dictation-silence <seconds>` (default and least 8) of silence, set
//! on Android by `dictation_silence.txt` in the data directory, holding
//! the seconds.
//!
//! The agent view (plan 20261022_03, step 4, `agent.rs`): tapping an
//! agent's card on the overview opens its transcript as terminal
//! scrollback with its priority cards; Back, or the system Back, returns
//! to the overview. "Take a message" there dictates to that agent, with
//! its card in the view.
//!
//! The settings (plan 20261024_02, `settings.rs`): the gear on the
//! overview opens them; Back, or the system Back, returns. They are kept in
//! `settings.toml` in the data directory (on the desktop, the companion's
//! own, `~/.config/handlr-companion`), given to the client as
//! [`Input::Settings`], and read by every control they turn off. *Unpair
//! this device* sends [`Input::Unpair`]; once [`Event::Unpaired`] comes,
//! the app is back on the pairing screen, and *Delete data* then removes
//! everything in the data directory but the voice models.

pub mod agent;
pub mod controls;
pub mod keys;
pub mod markdown_view;
pub mod overview;
pub mod settings;
pub mod ui;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use handlr_audio::libvoicexn::config::SttQuant;
use handlr_audio::libvoicexn::vad::GateState;
use handlr_audio::{AudioConfig, AudioEvent, AudioSession};
use handlr_companion::{
    Companion, CompanionConfig, Dictation, Event, FileKeyStore, Input, KeyStore, Pressure,
    Suggestion,
};
use libcontrols::DrawContext;
use libhandlr_proto::common::AgentInfo;
use libmorph::{
    AudioSetup, Frame, Key, KeyStatus, LimitsRequest, MorphApp, MorphContext, Touch, TouchPhase,
    wgpu,
};
use libmsdf::GpuSdfRenderer;
use libmsdf::drawlist::{AtlasId, AtlasSet, DrawList};
use libmsdf::font::{
    BUNDLED_ATLAS_PX_RANGE, FontAtlas, NOTO_SANS_MONO_ATLAS, ROBOTO_BOLD_ASCII, ROBOTO_BOLD_ATLAS,
    ROBOTO_ITALIC_ASCII, ROBOTO_ITALIC_ATLAS, ROBOTO_REGULAR_ASCII, ROBOTO_REGULAR_ATLAS,
    TERMINAL_SYMBOLS_ATLAS, TerminalShaper, TextShaper,
};

use crate::agent::{AgentHit, AgentScreen, Terminal};
use crate::keys::VaultKeyStore;
use crate::markdown_view::{CodeFace, Face, Faces};
use crate::overview::{Overview, OverviewHit, Panel};
use crate::settings::battery::Unreleased;
use crate::settings::{Settings, SettingsHit, SettingsScreen};
use crate::ui::{
    CODE, Card, CardState, Hit, Layout, Line, MUTED, Screen, View, WARN, chunk, dictation_lines,
    wrap,
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

/// The dictation card's lines, with the tail of a long message.
fn message_lines(d: &Dictation, layout: &Layout) -> Vec<Line> {
    dictation_lines(d)
        .into_iter()
        .map(|l| {
            if l.text != d.text {
                return l;
            }
            let lines = wrap(&l.text, layout.columns(l.size));
            let skip = lines.len().saturating_sub(MESSAGE_LINES);
            if skip == 0 {
                return l;
            }
            Line {
                text: format!("\u{2026} {}", lines[skip..].join(" ")),
                ..l
            }
        })
        .collect()
}

/// What was drawn last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shown {
    Pairing,
    Overview,
    Agent,
    Settings,
}

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
    /// The faces the settings' Markdown is drawn in (plan 20261024_04).
    faces: std::sync::Arc<Faces>,
    sdf: GpuSdfRenderer,
    list: DrawList,
    /// The pairing screen.
    screen: Screen,
    /// The home screen, once paired or connected.
    overview: Overview,
    /// What was drawn last: a change of screen draws the new one in full.
    shown: Option<Shown>,
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
    /// The agents, as last listed, and the panes waiting for an approval.
    agents: Vec<AgentInfo>,
    pending: Vec<String>,
    /// The message being dictated, while the mode is on.
    dictation: Option<Dictation>,
    /// The agent view, while it is open.
    agent: Option<AgentScreen>,
    /// The agent view's play button: replies are read aloud. Kept while
    /// the view is closed.
    read_aloud: bool,
    /// Something is being said.
    speaking: bool,
    /// An agent view has been opened: the overview's play has replies to
    /// read.
    viewed: bool,
    /// The settings, as last saved, and where `settings.toml` is.
    settings: Settings,
    data_dir: Option<PathBuf>,
    /// The settings screen, while it is open.
    settings_screen: Option<SettingsScreen>,
    /// The paired server's name, for the unpair modal.
    server_name: Option<String>,
    /// *Delete data* waits for the unpairing to finish.
    deleting: bool,
    /// The voice models, which *Delete data* keeps.
    models: Vec<PathBuf>,
    /// The cached sessions' size, in bytes, for *Clear cache (n MB)*.
    cache: u64,
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
    /// `--pair`, `pair.txt`, and an opened `handlr://` link. The text is
    /// never logged.
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
                self.server_name = Some(server.clone());
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
            Event::Agents(agents) => {
                self.agents = agents;
                self.overview.set_agents(self.agents.clone(), &self.pending);
            }
            Event::PendingPanes(panes) => {
                self.pending = panes;
                self.overview.set_agents(self.agents.clone(), &self.pending);
            }
            Event::Conversation { speaker, text } => self.overview.push_turn(speaker, text),
            Event::Dictation(d) => {
                if d.active != self.dictation.is_some() {
                    log::info!("dictation: {}", if d.active { "on" } else { "off" });
                }
                self.dictation = d.active.then_some(d);
            }
            Event::AgentView {
                name,
                status,
                sections,
                pinned,
                changed,
                loading,
                typed,
                loading_earlier,
                start,
                terminal_on,
                new_content,
                held_end,
                held_newer,
                bookends,
            } => {
                if let Some(agent) = &mut self.agent {
                    agent.set_loading(loading);
                    agent.set_typed(typed);
                    agent.set_terminal_on(terminal_on);
                    agent.set_content(name, status, sections, pinned, &changed);
                    agent.set_history(loading_earlier, start);
                    agent.set_new_content(new_content);
                    agent.set_held_end(held_end);
                    agent.set_held_newer(held_newer);
                    agent.set_bookends(bookends);
                }
            }
            // The audio played the chime.
            Event::Ding => log::info!("ding: an agent is done"),
            Event::Refreshed { new } => {
                if let Some(agent) = &mut self.agent {
                    agent.refreshed(new);
                }
            }
            Event::Cache { bytes } => {
                self.cache = bytes;
                if let Some(screen) = &mut self.settings_screen {
                    screen.set_cache(bytes);
                }
            }
            Event::Unpaired { daemon } => self.unpaired(daemon),
            // herdr's focus shows on the cards, from the list it refreshes.
            Event::Say(_)
            | Event::Focus(_)
            | Event::StopSpeaking
            | Event::Arm
            | Event::Vocabulary(_) => {}
        }
    }

    /// Opens the view of the agent in `pane_id`, from its card.
    fn open_agent(&mut self, pane_id: String) {
        log::info!("agent view: open");
        let name = self
            .agents
            .iter()
            .find(|a| a.pane_id == pane_id)
            .and_then(|a| a.name.clone())
            .unwrap_or_else(|| "the agent".to_string());
        self.send(Input::OpenAgent(pane_id));
        let mut agent = AgentScreen::new();
        agent.name = name;
        agent.set_scrollbar(self.settings.scrollbar);
        agent.set_sound(self.settings.sound);
        agent.set_read_aloud(self.read_aloud);
        agent.set_speaking(self.speaking);
        self.agent = Some(agent);
        self.viewed = true;
    }

    /// The phone is short of memory (plan 20261022_03, the L3 amendment
    /// of 2026-10-24 12:21 EDT): the client drops cached views and far
    /// contents, and the agent view frees the rows of blocks off screen.
    // TODO(libmorph 0.4.0): call from `on_trim_memory` (`UI_HIDDEN`,
    // `BACKGROUND`) and `on_low_memory` once libmorph delivers them.
    pub fn memory_pressure(&mut self, pressure: Pressure) {
        log::info!("memory: {pressure:?}");
        self.send(Input::Memory(pressure));
        if let Some(agent) = &mut self.agent {
            agent.set_pressure(Some(pressure));
        }
    }

    /// Closes the agent view, back to the overview.
    fn close_agent(&mut self) {
        if self.agent.take().is_some() {
            log::info!("agent view: closed");
            self.send(Input::CloseAgent);
            self.screen.invalidate();
        }
    }

    /// Opens the settings screen, from the overview's gear.
    fn open_settings(&mut self) {
        log::info!("settings: open");
        let mut screen = SettingsScreen::new(
            self.settings,
            Box::new(Unreleased),
            cfg!(target_os = "android"),
        )
        .with_faces(Some(std::sync::Arc::clone(&self.faces)))
        .with_build_info(settings::build_info::build_info());
        screen.set_daemon(self.server_name.as_deref().unwrap_or("handlr"), self.online);
        screen.set_cache(self.cache);
        self.settings_screen = Some(screen);
    }

    /// Closes the settings screen, back to the overview.
    fn close_settings(&mut self) {
        if self.settings_screen.take().is_some() {
            log::info!("settings: closed");
            self.overview.invalidate();
        }
    }

    /// A tap on the settings screen.
    fn settings_hit(&mut self, hit: SettingsHit) {
        match hit {
            SettingsHit::Back => self.close_settings(),
            SettingsHit::Changed(settings) => self.apply_settings(settings),
            SettingsHit::Unpair => self.unpair(false),
            SettingsHit::DeleteData => self.unpair(true),
            SettingsHit::ClearCache => {
                log::info!("settings: clearing the cache");
                self.send(Input::ClearCache);
            }
        }
    }

    /// The settings changed: saved, given to the client, and shown by every
    /// control they turn off.
    fn apply_settings(&mut self, settings: Settings) {
        self.settings = settings;
        settings::set_current(settings);
        if let Some(dir) = &self.data_dir
            && let Err(e) = settings.save(dir)
        {
            log::warn!("settings: {e}");
        }
        self.send(Input::Settings(settings.to_client()));
        if let Some(agent) = &mut self.agent {
            agent.set_scrollbar(settings.scrollbar);
            agent.set_sound(settings.sound);
            agent.invalidate();
        }
        self.overview.settings_changed();
    }

    /// *Unpair this device*, and *Delete data* (`delete`) after it.
    fn unpair(&mut self, delete: bool) {
        log::info!(
            "settings: {}",
            if delete {
                "deleting the data"
            } else {
                "unpairing"
            }
        );
        self.deleting = delete;
        if self.companion.is_some() {
            self.send(Input::Unpair);
        } else {
            // No key storage, so no pairing to forget.
            self.unpaired(false);
        }
    }

    /// The pairing is forgotten: back to the pairing screen, and the data
    /// deleted if that was asked.
    fn unpaired(&mut self, daemon: bool) {
        log::info!(
            "pairing: unpaired{}",
            if daemon { "" } else { " here only" }
        );
        self.paired = false;
        self.online = false;
        self.code = None;
        self.server_name = None;
        self.card = None;
        self.dictation = None;
        self.agents.clear();
        self.pending.clear();
        if self.agent.take().is_some() {
            self.send(Input::CloseAgent);
        }
        self.viewed = false;
        self.settings_screen = None;
        self.overview = Overview::new();
        self.screen.invalidate();
        self.link = "not paired \u{b7} paste a pairing URL below".into();
        self.note = Some(if daemon {
            ("unpaired".into(), MUTED)
        } else {
            (
                "unpaired on this phone; handlr lists it until it is removed from auth.toml".into(),
                WARN,
            )
        });
        if std::mem::take(&mut self.deleting) {
            self.delete_data();
        }
    }

    /// *Delete data*: everything in the data directory but the voice
    /// models, and the settings back to their defaults.
    fn delete_data(&mut self) {
        if let Some(dir) = &self.data_dir {
            match settings::delete_data(dir, &self.models) {
                Ok(n) => log::info!("settings: deleted {n} entries of the data directory"),
                Err(e) => {
                    log::error!("settings: deleting the data: {e}");
                    self.note = Some((format!("deleting the data: {e}"), WARN));
                }
            }
        }
        self.settings = Settings::default();
        settings::set_current(self.settings);
        self.send(Input::Settings(self.settings.to_client()));
        self.overview.settings_changed();
    }

    /// A tap on one of the agent view's controls, or the overview's.
    fn agent_hit(&mut self, hit: AgentHit) {
        match hit {
            AgentHit::Back => self.close_agent(),
            AgentHit::Message => self.send(Input::StartDictation),
            AgentHit::Reply => {
                if let Some(agent) = &mut self.agent {
                    agent.begin_reply();
                }
                self.send(Input::StartDictation);
            }
            AgentHit::Approve(confirm_id) => self.send(Input::Answer {
                confirm_id,
                approve: true,
            }),
            AgentHit::Deny(confirm_id) => self.send(Input::Answer {
                confirm_id,
                approve: false,
            }),
            AgentHit::Expand => {
                if let Some(agent) = &mut self.agent {
                    agent.toggle_expand();
                }
            }
            AgentHit::Fold(key) => {
                if let Some(agent) = &mut self.agent {
                    agent.toggle_fold(key);
                }
            }
            AgentHit::Replay(key) => {
                if let Some(text) = self.agent.as_ref().and_then(|a| a.spoken_of(key)) {
                    log::info!("agent view: replaying a card's spoken text");
                    self.send(Input::Replay(text));
                }
            }
            AgentHit::Pin => {
                if let Some(agent) = &mut self.agent {
                    agent.jump_to_pin();
                }
            }
            AgentHit::JumpToEnd => {
                // A 2 s scroll to the end. A new outline waiting: the
                // client adds its newer sections below the end, the scroll
                // carries on into them, and the rest is merged once the
                // view lands at the bottom, following.
                if self.agent.as_mut().is_some_and(|a| a.jump_to_end()) {
                    self.send(Input::ApplyOutline);
                }
            }
            AgentHit::Suggest(Suggestion::Discuss) => {
                // A message in reply to the plan's card, its start already
                // in it.
                if let Some(agent) = &mut self.agent {
                    agent.begin_reply();
                }
                self.send(Input::Respond(Suggestion::Discuss));
            }
            AgentHit::Suggest(suggestion) => self.send(Input::Respond(suggestion)),
            AgentHit::Link(index) => {
                let _url = self
                    .agent
                    .as_ref()
                    .and_then(|a| a.link(index))
                    .map(|l| l.url.clone())
                    .unwrap_or_default();
                // TODO(libmorph): open `_url` in the phone's browser once
                // libmorph (and libmobile) have an outbound "open this link"
                // call; they only receive links today.
                // Never the URL itself: a query or fragment may be a secret.
                log::info!("link: opening is not available yet");
            }
            AgentHit::ReadAloud if self.read_aloud && self.speaking => {
                log::info!("read aloud: skip");
                self.send(Input::Skip);
            }
            AgentHit::ReadAloud => {
                self.read_aloud = !self.read_aloud;
                log::info!(
                    "agent view: read aloud {}",
                    if self.read_aloud { "on" } else { "off" }
                );
                self.send(Input::ReadAloud(self.read_aloud));
                if let Some(agent) = &mut self.agent {
                    agent.set_read_aloud(self.read_aloud);
                }
            }
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
                if let Input::Speaking(on) = input {
                    self.speaking = on;
                    if let Some(agent) = &mut self.agent {
                        agent.set_speaking(on);
                    }
                }
                match &input {
                    Input::Utterance(_) => {
                        self.overview.set_partial(None);
                        // A new utterance: an answered card has had its say.
                        if self
                            .card
                            .as_ref()
                            .is_some_and(|c| matches!(c.state, CardState::Done(_)))
                        {
                            self.card = None;
                        }
                    }
                    Input::Listening(false) => self.overview.set_partial(None),
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
            AudioEvent::Partial(text) => self.overview.set_partial(Some(text)),
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

    /// Whether the pairing screen shows: neither paired nor connected, or a
    /// device code to paste.
    fn pairing(&self) -> bool {
        self.code.is_some() || (!self.paired && !self.online)
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
        if let Some(agent) = &mut self.agent {
            agent.set_layout(layout);
            agent.set_dictation(self.dictation.as_ref().map(dictation_lines));
        }
        if let Some(screen) = &mut self.settings_screen {
            screen.set_layout(layout);
            screen.set_daemon(self.server_name.as_deref().unwrap_or("handlr"), self.online);
        }
        // Without the microphone, nothing listens: the sound line says so.
        let sound = if self.settings.microphone {
            self.sound.clone()
        } else {
            "microphone off".to_string()
        };
        if !self.pairing() {
            let o = &mut self.overview;
            o.set_layout(layout);
            o.set_status(vec![self.link.clone(), sound], self.note.clone());
            o.set_card(self.card.as_ref().map(|c| Panel {
                lines: c.lines(),
                confirm: c.asks().then_some(c.confirm_id),
            }));
            o.set_dictation(self.dictation.as_ref().map(|d| message_lines(d, &layout)));
            o.set_play(self.read_aloud, self.speaking, self.viewed);
            return;
        }

        let line = |text: String, size: f32, color, gap| Line {
            text,
            size,
            color,
            gap,
        };
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
        let view = View {
            status: vec![self.link.clone(), sound],
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

    /// The app left the screen: the link sleeps (one socket, no polling).
    fn on_suspend(&mut self, _ctx: &mut MorphContext) {
        self.send(Input::Background(true));
    }

    /// Back on screen: the link wakes and catches up, and memory pressure
    /// is over.
    fn on_resume(&mut self, _ctx: &mut MorphContext) {
        self.send(Input::Background(false));
        if let Some(agent) = &mut self.agent {
            agent.set_pressure(None);
        }
    }

    /// A `handlr://` link opened the app (libmorph 0.3.2). Only a pairing
    /// link (`handlr://<host>:<port>/pair#…`) is acted on; its fragment is
    /// the pairing string, so nothing past the path is logged.
    fn on_open_url(&mut self, _ctx: &mut MorphContext, url: &str) {
        let before_fragment = url.split(['#', '?']).next().unwrap_or("");
        let is_pairing = before_fragment
            .strip_prefix("handlr://")
            .is_some_and(|rest| rest.ends_with("/pair"));
        if is_pairing {
            log::info!("link: a pairing link was opened");
            self.open_url(url);
        } else {
            log::warn!("link: ignoring an opened link that is not a pairing link");
        }
    }

    fn init(ctx: &mut MorphContext) -> Self {
        let parse = |bytes| FontAtlas::from_bytes(bytes).expect("bundled atlas parses");
        let (text, page) = (parse(NOTO_SANS_MONO_ATLAS), parse(TERMINAL_SYMBOLS_ATLAS));
        let terminal = TerminalShaper::new(&text, &page).expect("bundled terminal atlases agree");
        let mut fonts = AtlasSet::new();
        let mut add = |atlas| fonts.register(atlas).expect("bundled atlases fit one set");
        let (roboto, mono, symbols) = (add(parse(ROBOTO_REGULAR_ATLAS)), add(text), add(page));
        let (bold, italic) = (
            add(parse(ROBOTO_BOLD_ATLAS)),
            add(parse(ROBOTO_ITALIC_ATLAS)),
        );
        let shaper = TextShaper::new(ROBOTO_REGULAR_ASCII.to_vec()).expect("bundled face parses");
        // The Markdown's faces keep the atlases' metrics; their texels are
        // in the set above.
        let metrics = |bytes: &'static [u8]| Faces::metrics(parse(bytes));
        let face = |id, atlas: &'static [u8], ttf: &[u8]| Face {
            id,
            atlas: metrics(atlas),
            shaper: TextShaper::new(ttf.to_vec()).expect("bundled face parses"),
        };
        let (code_text, code_page) = (
            metrics(NOTO_SANS_MONO_ATLAS),
            metrics(TERMINAL_SYMBOLS_ATLAS),
        );
        let faces = std::sync::Arc::new(Faces {
            regular: face(roboto, ROBOTO_REGULAR_ATLAS, ROBOTO_REGULAR_ASCII),
            bold: face(bold, ROBOTO_BOLD_ATLAS, ROBOTO_BOLD_ASCII),
            italic: face(italic, ROBOTO_ITALIC_ATLAS, ROBOTO_ITALIC_ASCII),
            code: CodeFace {
                shaper: TerminalShaper::new(&code_text, &code_page)
                    .expect("bundled terminal atlases agree"),
                text: (mono, code_text),
                symbols: (symbols, code_page),
            },
            px_range: BUNDLED_ATLAS_PX_RANGE,
        });
        let sdf = GpuSdfRenderer::new_with_atlas_set(ctx.device(), ctx.surface_format(), &fonts);
        sdf.upload_atlas_set(ctx.queue(), &fonts);

        let args = Args::parse(ctx.args());
        let audio_config = audio_config(ctx, &args);
        // The app's data directory; on the desktop, the companion's own.
        let data_dir = ctx
            .data_dir()
            .map(std::path::Path::to_path_buf)
            .or_else(|| Some(FileKeyStore::default_dir()));
        let settings = data_dir.as_deref().map(Settings::load).unwrap_or_default();
        settings::set_current(settings);
        log::info!("settings: {settings:?}");
        // The voice models, where they are in the data directory.
        let models = [&audio_config.stt.model_dir, &audio_config.tts.model_dir]
            .into_iter()
            .filter(|m| {
                data_dir
                    .as_ref()
                    .is_some_and(|d| m.parent() == Some(d.as_path()))
            })
            .cloned()
            .collect();
        let mut app = Self {
            fonts,
            roboto,
            mono,
            symbols,
            shaper,
            terminal,
            faces,
            sdf,
            list: DrawList::new(),
            screen: Screen::new(),
            overview: Overview::new(),
            shown: None,
            next_frame: Instant::now(),
            companion: None,
            audio: None,
            audio_config,
            #[cfg(not(target_os = "android"))]
            _mic: None,
            link: "starting\u{2026}".into(),
            sound: "waiting for the microphone\u{2026}".into(),
            paired: false,
            card: None,
            code: None,
            note: None,
            field: String::new(),
            field_focused: false,
            online: false,
            agents: Vec::new(),
            pending: Vec::new(),
            dictation: None,
            agent: None,
            read_aloud: false,
            speaking: false,
            viewed: false,
            settings,
            data_dir,
            settings_screen: None,
            server_name: None,
            deleting: false,
            models,
            cache: 0,
            started: Instant::now(),
            last_update: Instant::now(),
        };

        match key_store(ctx, &args) {
            Ok((keys, server)) => {
                app.paired = server.is_some();
                app.server_name = server.clone();
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
                let companion = Companion::spawn(config, keys);
                companion.send(Input::Settings(app.settings.to_client()));
                app.companion = Some(companion);
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
        if let Some(screen) = &mut self.settings_screen {
            screen.tick(dt);
        } else if let Some(agent) = &mut self.agent {
            agent.tick(dt);
        } else {
            self.overview.tick(dt);
        }
        self.lay_out(ctx);
        // The view's flow came near its top: page in earlier sections
        // (plan 20261023_03, A7).
        if self.agent.as_mut().is_some_and(|a| a.take_load_earlier()) {
            self.send(Input::LoadEarlier);
        }
        // Pulled past the end and let go: refresh (plan 20261022_03, the
        // L3 amendment of 2026-10-24 12:22 EDT).
        if self.agent.as_mut().is_some_and(|a| a.take_refresh()) {
            self.send(Input::Refresh);
        }
        // Where the window is, for the contents fetched around it and an
        // outline held aside while scrolled back (plan 20261024_01).
        if let Some(window) = self.agent.as_mut().and_then(|a| a.take_window()) {
            self.send(Input::Window(window));
        }
    }

    /// The controls, and a tap anywhere else. On the overview and in the
    /// agent view, every phase goes to the screen: a drag scrolls, a tap
    /// hits. On the pairing screen, only a touch's end counts.
    fn on_touch(&mut self, _ctx: &mut MorphContext, touch: Touch) {
        let now = self.started.elapsed();
        if let Some(screen) = &mut self.settings_screen {
            if let Some(hit) = screen.touch(touch.id, touch.phase, touch.x, touch.y, now) {
                self.settings_hit(hit);
            }
            return;
        }
        if let Some(agent) = &mut self.agent {
            if let Some(hit) = agent.touch(touch.id, touch.phase, touch.x, touch.y, now) {
                self.agent_hit(hit);
            }
            return;
        }
        if !self.pairing() {
            match self
                .overview
                .touch(touch.id, touch.phase, touch.x, touch.y, now)
            {
                Some(OverviewHit::Agent(pane_id)) => self.open_agent(pane_id),
                Some(OverviewHit::Bottom(hit)) => self.agent_hit(hit),
                Some(OverviewHit::Tap) => self.send(Input::Tap),
                Some(OverviewHit::Settings) => self.open_settings(),
                None => {}
            }
            return;
        }
        if touch.phase != TouchPhase::Ended {
            return;
        }
        match self.screen.hit(touch.x, touch.y) {
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
        // The system Back cancels the settings' modal, or closes them.
        if matches!(key, Key::Back | Key::Escape)
            && let Some(screen) = &mut self.settings_screen
        {
            if !screen.back() {
                self.close_settings();
            }
            return KeyStatus::Handled;
        }
        // The system Back closes the agent view.
        if self.agent.is_some() && matches!(key, Key::Back | Key::Escape) {
            self.close_agent();
            return KeyStatus::Handled;
        }
        // On the overview, it closes the Gemini turns' overlay.
        if self.agent.is_none()
            && !self.pairing()
            && matches!(key, Key::Back | Key::Escape)
            && self.overview.back()
        {
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
        let shown = match (&self.agent, self.pairing()) {
            _ if self.settings_screen.is_some() => Shown::Settings,
            (Some(_), _) => Shown::Agent,
            (None, true) => Shown::Pairing,
            (None, false) => Shown::Overview,
        };
        // Another screen drew over the list: the one shown now draws it all.
        if self.shown != Some(shown) {
            self.shown = Some(shown);
            self.screen.invalidate();
            self.overview.invalidate();
            if let Some(agent) = &mut self.agent {
                agent.invalidate();
            }
            if let Some(screen) = &mut self.settings_screen {
                screen.invalidate();
            }
        }
        match (&mut self.settings_screen, &mut self.agent) {
            (Some(screen), _) => screen.draw(&mut self.list, &draw, Some(&term)),
            (None, Some(agent)) => agent.draw(&mut self.list, &draw, Some(&term)),
            (None, None) if shown == Shown::Pairing => self.screen.draw(&mut self.list, &draw),
            (None, None) => self.overview.draw(&mut self.list, &draw, Some(&term)),
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
