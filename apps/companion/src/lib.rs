//! flankr's companion app (docs/design/20261022_04_COMPANION_APP_PLAN.md,
//! decision 8 and step 4 of the 18:10 EDT amendment): one libmorph app for
//! a desktop window and for Android, started as a copy of libvoicexn's
//! `examples/voicexn` at v0.2.0.
//!
//! - `flankr-companion` is the client: pairing, the link, turns and
//!   confirmations. The app sends it [`Input`]s and shows its [`Event`]s.
//! - `flankr-audio` is the voice: libvoicexn's STT and TTS, with cpal on the
//!   desktop and libmorph's audio on Android. Every event is handed to it,
//!   and its `Utterance`, `Listening` and `Speaking` go back to the client.
//! - Keys are in libmorph's key store and secure store (`keys.rs`). Where
//!   libmorph has neither (Linux), the desktop uses `FileKeyStore`; Android
//!   never falls back, and shows an error instead.
//!
//! A tap on no control is [`Input::Tap`]. Pairing takes a `flankr://` URL,
//! a `flankr1…` string or a `data:image/png;base64,` URL, through
//! [`App::open_url`]: typed into the field (desktop), given as `--pair`
//! (desktop), or pushed as `pair.txt` into the data directory (Android,
//! the plan's 16:23 EDT amendment), which the app reads and deletes at
//! start.
//!
//! The home screen is the overview (plan 20261023_04, `overview.rs`): the
//! agents' cards on the whole screen, the prompt cards under them (Gemini's
//! and a message's, `prompt/`, plan 20261027_03), and the confirmation card
//! and the bottom row under that. A prompt card's up arrow expands it over
//! the cards with its history; its grab bar or the system Back collapses
//! it. The pairing screen (`ui.rs`) shows instead while
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
//! `settings.toml` in libmorph's data directory on every host (on macOS,
//! `~/Library/Application Support/dev.flankr.companion`; a file at the old
//! desktop place, `~/.config/flankr-companion`, is moved once), given to the client as
//! [`Input::Settings`], and read by every control they turn off. *Unpair
//! this device* sends [`Input::Unpair`]; once [`Event::Unpaired`] comes,
//! the app is back on the pairing screen, and *Delete data* then removes
//! everything in the data directory but the voice models.

pub mod agent;
pub mod controls;
pub mod keys;
pub mod markdown_view;
pub mod memory;
pub mod notify;
pub mod overview;
pub mod prompt;
pub mod session_cards;
pub mod settings;
pub mod theme;
pub mod ui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use flankr_audio::libvoicexn::config::SttQuant;
use flankr_audio::libvoicexn::vad::GateState;
use flankr_audio::{AudioConfig, AudioEvent, AudioSession};
use flankr_companion::{
    Companion, CompanionConfig, Dictation, DictationPhase, Event, FileKeyStore, Input, KeyStore,
    Pressure, PromptMode, Speaker, Suggestion,
};
use libcontrols::DrawContext;
use libflankr_proto::common::AgentInfo;
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
use crate::notify::Notifier;
use crate::overview::{Overview, OverviewHit, Panel};
use crate::prompt::keyboard::{FocusedPrompt, Ime, KeyboardRoute, PromptScreen};
use crate::prompt::{MessagePrompt, PromptKind, Prompts, Sent};
use crate::session_cards::{SessionCards, SessionHit, Touched};
use crate::settings::background::{self, POST_NOTIFICATIONS, PermissionOs, TurnedOn};
use crate::settings::build_info::BuildInfo;
use crate::settings::{Settings, SettingsHit, SettingsScreen};
use crate::theme::{Role, theme};
use crate::ui::{Card, CardState, Hit, Layout, Line, Screen, View, chunk, prompt_input, wrap};

/// The frame rate: enough for text, and far less heat than the display's
/// 120 Hz while STT listens.
const FPS: u32 = 15;

/// The longest pairing input the field takes: a data URL of a QR code PNG
/// is a few kilobytes.
const FIELD_MAX: usize = 16 * 1024;

/// The name the client gives in `hello`.
const CLIENT: &str = "companion";

/// The app's id: the Android package (build.rs) and the desktop's data
/// directory's name.
pub const APP_ID: &str = "dev.flankr.companion";

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
    panel_shader: Option<overview::PanelShader>,
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
    _mic: Option<flankr_audio::libvoicexn::capture::Mic>,
    /// The connection, for the first status line.
    link: String,
    /// The audio, for the second.
    sound: String,
    /// A server is stored, so a disconnect is a retry, not "not paired".
    paired: bool,
    /// The confirmation being asked, or the last one, with its outcome,
    /// until the next utterance (plan 20261023_01, section 4).
    card: Option<Card>,
    /// The device code to paste into `flankr pair`, and the server's name.
    code: Option<(String, String)>,
    /// A pairing note or an error, and its colour's role (looked up as it
    /// is drawn, so a theme change recolours it).
    note: Option<(String, Role)>,
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
    /// Phonon's voices, once it has loaded (`AudioEvent::TtsLoaded`), for
    /// the *Voice* row.
    voices: Option<Vec<String>>,
    /// A *Voice* preview was asked and is not yet over: the next step cuts
    /// it off.
    previewing: bool,
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
    /// Links tapped in the agent view, opened once the touch is handled.
    links: Vec<String>,
    /// The phone's vendor (libmorph 0.4.1's `build_info`), for the copy.
    build_info: Option<BuildInfo>,
    /// The foreground service's notification and the alerts (`notify.rs`).
    notifier: Notifier,
    /// Where the service is, as the main thread sees it.
    service: Service,
    /// flankr is on screen.
    visible: bool,
    /// Set by the settings screen when the alerts are turned on: the
    /// permission is asked on the next frame, with the context.
    notify_ask: std::rc::Rc<std::cell::Cell<bool>>,
    /// `POST_NOTIFICATIONS` is granted, as last read: at start, on resume
    /// and in its answer. Always on the desktop.
    notifications_granted: bool,
    /// A request for it is out: the toggle waits for the answer.
    asking_permission: bool,
    /// Other sessions' cards at the bottom (plan 20261022_03, the L3
    /// amendments of 2026-10-24 15:56, 16:00 and 16:01 EDT).
    session_cards: SessionCards,
    /// The agent view's pane, while it is open: its session's colour.
    agent_pane: Option<String>,
    /// The display's scale, as last laid out.
    scale: f32,
    /// The one keyboard route to the prompt cards (plan 20261027_03, step
    /// 3): the focused prompt, the soft keyboard dismissed, and a physical
    /// keyboard (always on the desktop; on Android once one types).
    keyboard: KeyboardRoute,
    /// A touch held on the centre button, watched for opening Gemini's card
    /// pinned after 2 seconds (plan 20261027_03, decision 7).
    center_hold: Option<CenterHold>,
    /// The conversation with Gemini, as each screen's card is given it
    /// (plan 20261027_03, decision 1: each screen owns its own card).
    gemini_turns: Vec<(Speaker, String)>,
    /// What each session's message card keeps for the app's run, by pane
    /// (the amendment of 2026-10-27 01:14 EDT, item 2).
    sessions: HashMap<String, SessionPrompt>,
    /// The agent view's Gemini card was last left expanded (decision 5:
    /// per screen); the overview's card keeps its own.
    agent_gemini_expanded: bool,
}

/// A session's message card, kept for the app's run: the session it is
/// for, the messages sent, and whether it was last left expanded.
#[derive(Clone, Debug, Default)]
struct SessionPrompt {
    session_id: Option<String>,
    sent: Vec<Sent>,
    expanded: bool,
}

#[derive(Clone, Copy, Debug)]
struct CenterHold {
    id: u64,
    start: Duration,
    opened: bool,
    started_listening: bool,
}

impl CenterHold {
    /// How long the centre button is held to open Gemini's card.
    const OPENS: Duration = Duration::from_secs(2);

    /// Whether the hold opens the card at `now`: once, at 2 s.
    fn open_at(&mut self, now: Duration) -> bool {
        if self.opened || now.saturating_sub(self.start) < Self::OPENS {
            return false;
        }
        self.opened = true;
        true
    }
}

/// The foreground service, as the app knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    /// Not running: started on the next frame on screen if it is wanted.
    Off,
    /// `start` returned; `ServiceEvent::Started` is awaited.
    Starting,
    Running,
    /// It could not start (no service on this host, or Android refused
    /// it): not tried again until the setting changes or flankr is back on
    /// screen.
    Unavailable,
}

/// The context's permission calls, for [`background::turn_on`].
struct ContextPermissions<'a>(&'a mut MorphContext);

impl PermissionOs for ContextPermissions<'_> {
    fn granted(&self) -> bool {
        self.0.permission_granted(POST_NOTIFICATIONS)
    }

    fn rationale(&self) -> bool {
        self.0.should_show_rationale(POST_NOTIFICATIONS)
    }

    fn request(&mut self) {
        if self.0.request_permissions(&[POST_NOTIFICATIONS]).is_none() {
            log::info!("notifications: no runtime permissions on this host");
        }
    }

    /// The app's settings page, one tap from its notifications.
    fn open_settings(&mut self) {
        if let Err(e) = libmorph::battery::open_app_details() {
            log::warn!("notifications: opening the app's settings: {e}");
        }
    }
}

/// Opens `url` in the phone's browser (libmorph 0.4.0's `open_link`).
/// Never logged past its kind: a query or fragment may be a secret.
fn open_link(ctx: &MorphContext, url: &str) {
    match ctx.open_link(url) {
        Ok(()) => log::info!("link: opened"),
        Err(libmorph::OpenLinkError::NotVisible) => {
            log::warn!("link: not opened: flankr is not on screen (NotVisible)")
        }
        Err(e) => log::warn!("link: not opened: {e}"),
    }
}

/// The models: `<data dir>/<name>` on Android, `--models <dir>` (default
/// `models/`) on the desktop. libmorph 0.4.0 gives a data directory on the
/// desktop too; the models and the vocabulary stay where they were there.
fn audio_config(ctx: &MorphContext, args: &Args) -> AudioConfig {
    let android_dir = ctx.data_dir().filter(|_| cfg!(target_os = "android"));
    let models = match android_dir {
        Some(dir) => dir.to_path_buf(),
        None => args
            .models
            .clone()
            .unwrap_or_else(|| PathBuf::from("models")),
    };
    let mut config = AudioConfig::new(&models);
    if let Some(dir) = android_dir {
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
            .and_then(flankr_companion::device_name_from)
        {
            return name;
        }
    }
    flankr_companion::default_device_name()
}

impl App {
    /// Puts the settings' theme in force (plan 20261024_05): every screen
    /// is made again or laid out in its colours and drawn again, and the
    /// system bars follow it. Nothing when it is in force already.
    fn apply_theme(&mut self) {
        let Some(t) = settings::put_theme_in_force(&self.settings) else {
            return;
        };
        log::info!("theme: {}", t.name);
        self.screen.restyle();
        self.overview.restyle();
        if let Some(a) = self.agent.as_mut() {
            a.restyle();
        }
        if let Some(s) = self.settings_screen.as_mut() {
            s.restyle();
        }
    }

    /// The app's handling of a pairing URL, however it arrives: the field,
    /// `--pair`, `pair.txt`, and an opened `flankr://` link. The text is
    /// never logged.
    pub fn open_url(&mut self, url: &str) {
        let text = url.trim();
        if text.is_empty() {
            return;
        }
        match &self.companion {
            Some(companion) => {
                companion.send(Input::Pair(text.to_string()));
                self.note = Some(("pairing\u{2026}".into(), Role::Muted));
            }
            None => self.note = Some(("no key storage: cannot pair".into(), Role::Warn)),
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
        let output = flankr_audio::Format {
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
                self.note = Some((format!("paired as {name}"), Role::Muted));
            }
            Event::PairFailed(why) => {
                log::warn!("pairing: refused");
                self.note = Some((format!("pairing failed: {why}"), Role::Warn));
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
            Event::TurnEnded { outcome, .. } => {
                log::info!("turn: {outcome}");
                self.set_listening(false);
            }
            Event::Error(e) => {
                log::warn!("companion: {e}");
                self.note = Some((e, Role::Warn));
            }
            Event::Agents(agents) => {
                self.agents = agents;
                self.follow_sessions();
                self.overview.set_agents(self.agents.clone(), &self.pending);
            }
            Event::PendingPanes(panes) => {
                self.pending = panes;
                self.overview.set_agents(self.agents.clone(), &self.pending);
            }
            Event::Conversation { speaker, text } => {
                if let Some(agent) = &mut self.agent {
                    agent.push_turn(speaker, &text);
                }
                self.gemini_turns.push((speaker, text.clone()));
                let drop = self
                    .gemini_turns
                    .len()
                    .saturating_sub(crate::prompt::history::KEPT);
                self.gemini_turns.drain(..drop);
                self.overview.push_turn(speaker, text);
            }
            Event::Dictation(d) => {
                if d.active != self.dictation.is_some() {
                    log::info!("dictation: {}", if d.active { "on" } else { "off" });
                }
                if !d.active {
                    self.keyboard.close();
                    self.message_sent();
                    self.set_listening(false);
                } else if d.general_agent {
                    self.set_listening(d.mode == PromptMode::Voice);
                }
                self.dictation = d.active.then_some(d);
                self.overview.invalidate();
                if let Some(agent) = &mut self.agent {
                    agent.invalidate();
                }
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
            Event::SessionCards(cards) => {
                if let Some(c) = cards.first() {
                    log::info!(
                        "cards: {:?} from another session, {} in all",
                        c.kind,
                        cards.len()
                    );
                }
                self.session_cards.set(cards);
            }
            // Plan 20261027_02, decision 9: the centre button, the Gemini
            // strip and the settings toggle follow the daemon's `[llm]`.
            Event::Llm(info) => {
                settings::set_llm(info);
                self.overview.settings_changed();
                self.overview.invalidate();
                if let Some(agent) = &mut self.agent {
                    agent.settings_changed();
                }
                if let Some(screen) = &mut self.settings_screen {
                    screen.llm_changed();
                }
            }
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
        self.agent_pane = Some(pane_id.clone());
        self.send(Input::OpenAgent(pane_id));
        let mut agent = AgentScreen::new();
        agent.name = name;
        agent.set_scrollbar(self.settings.scrollbar);
        agent.set_sound(self.settings.sound);
        agent.set_read_aloud(self.read_aloud);
        agent.set_speaking(self.speaking);
        let ga_listening = self.dictation.as_ref().is_some_and(|d| {
            d.active && d.general_agent && d.mode == flankr_companion::PromptMode::Voice
        });
        agent.set_general_agent_listening(ga_listening);
        // Its own Gemini card: the conversation so far, and the state the
        // view's card was last left in (plan 20261027_03, decisions 1, 5).
        self.remember_prompts();
        let expanded = self.agent_gemini_expanded;
        let listening = self.overview.prompts().gemini().listening();
        let turns = &self.gemini_turns;
        agent.with_prompts(|p| {
            p.gemini_mut().set_turns(turns);
            p.gemini_mut().set_listening(listening);
            p.gemini_mut().set_expanded(expanded);
        });
        self.agent = Some(agent);
        self.viewed = true;
    }

    /// The prompt cards of the screen shown.
    fn prompts(&self) -> &Prompts {
        match &self.agent {
            Some(agent) => agent.prompts(),
            None => self.overview.prompts(),
        }
    }

    /// Acts on the prompt cards of the screen shown, which is laid out
    /// again; what is to be remembered of them is.
    fn with_prompts(&mut self, f: impl FnOnce(&mut Prompts)) {
        match &mut self.agent {
            Some(agent) => agent.with_prompts(f),
            None => self.overview.with_prompts(f),
        }
        self.remember_prompts();
    }

    /// Remembers whether the shown screen's cards are left pinned or
    /// expanded (decision 5): the message card's for its session, and the
    /// agent view's Gemini card for the next view.
    fn remember_prompts(&mut self) {
        let prompts = self.prompts();
        let message = prompts
            .message_open()
            .then(|| prompts.message().is_expanded());
        let gemini = prompts.gemini().is_expanded();
        if let Some(expanded) = message
            && let Some(pane) = self.message_pane()
        {
            self.sessions.entry(pane).or_default().expanded = expanded;
        }
        if self.agent.is_some() {
            self.agent_gemini_expanded = gemini;
        }
    }

    /// The pane the message being taken goes to.
    fn message_pane(&self) -> Option<String> {
        self.dictation
            .as_ref()
            .filter(|d| !d.general_agent)
            .and_then(|d| d.target.as_ref())
            .map(|t| t.pane_id.clone())
    }

    /// The message card for `d`: the agent it goes to, as shown, in its
    /// session's colour; what was sent to that session this run; and the
    /// state its card was last left in.
    fn message_prompt(&self, d: &Dictation) -> MessagePrompt {
        let pane = d.target.as_ref().map(|t| t.pane_id.as_str());
        let agent = pane.and_then(|p| self.agents.iter().find(|a| a.pane_id == p));
        let target = d.target.as_ref().filter(|t| t.agent.is_some()).map(|t| {
            let name = agent
                .map(flankr_companion::cross::shown_name)
                .unwrap_or_else(|| t.agent.clone().unwrap_or_default());
            let colour = agent.map_or(theme().text, |a| {
                theme().session(flankr_companion::cross::colour_of(a))
            });
            (name, colour)
        });
        let session = pane.and_then(|p| self.sessions.get(p));
        MessagePrompt {
            input: prompt_input(d),
            target,
            sent: session.map(|s| s.sent.clone()).unwrap_or_default(),
            expanded: session.is_some_and(|s| s.expanded),
        }
    }

    /// The message being taken has gone (the dictation turned off): if it
    /// had been submitted, it was sent, and goes into its session's
    /// history with the time.
    fn message_sent(&mut self) {
        let Some(d) = self.dictation.as_ref().filter(|d| {
            !d.general_agent
                && matches!(
                    d.phase,
                    DictationPhase::Submitting | DictationPhase::Confirming
                )
        }) else {
            return;
        };
        let Some(pane) = d.target.as_ref().map(|t| t.pane_id.clone()) else {
            return;
        };
        let sent = Sent::now(&d.text);
        let session_id = self
            .agents
            .iter()
            .find(|a| a.pane_id == pane)
            .and_then(|a| a.session.as_ref())
            .and_then(|s| s.session_id.clone());
        let entry = self.sessions.entry(pane).or_default();
        if entry.session_id.is_none() {
            entry.session_id = session_id;
        }
        entry.sent.push(sent);
        let drop = entry
            .sent
            .len()
            .saturating_sub(crate::prompt::history::KEPT);
        entry.sent.drain(..drop);
    }

    /// The sessions' messages are kept until the session's file changes: a
    /// new session id for the pane clears them.
    fn follow_sessions(&mut self) {
        for (pane, kept) in &mut self.sessions {
            let id = self
                .agents
                .iter()
                .find(|a| &a.pane_id == pane)
                .and_then(|a| a.session.as_ref())
                .and_then(|s| s.session_id.clone());
            let Some(id) = id else {
                continue;
            };
            if kept.session_id.as_ref().is_some_and(|was| *was != id) {
                log::info!("prompt: a new session file; its messages are cleared");
                kept.sent.clear();
            }
            kept.session_id = Some(id);
        }
    }

    /// The screen whose prompt cards are shown.
    fn prompt_screen(&self) -> PromptScreen {
        if self.agent.is_some() {
            PromptScreen::Agent
        } else {
            PromptScreen::Overview
        }
    }

    /// The keyboard types into `kind`'s card on the screen shown, and the
    /// soft keyboard shows (plan 20261027_03, decision 6).
    fn focus_prompt(&mut self, kind: PromptKind) {
        let screen = self.prompt_screen();
        self.keyboard.focus(FocusedPrompt { screen, kind });
    }

    /// The one place the soft keyboard is shown or hidden, each frame: for
    /// the typed prompt open, unless it was dismissed or a physical
    /// keyboard types.
    fn sync_keyboard(&mut self, ctx: &mut MorphContext) {
        let typing = self
            .dictation
            .as_ref()
            .filter(|d| d.mode == PromptMode::Keyboard)
            .map(|d| {
                let kind = if d.general_agent {
                    PromptKind::GeneralAgent
                } else {
                    PromptKind::Message
                };
                (kind, d.text.as_str())
            });
        let screen = self.prompt_screen();
        match self.keyboard.sync(typing, ctx.text_input_active(), screen) {
            Ime::Start(text) => {
                let mut options = libmorph::TextInputOptions::default();
                options.multiline = true;
                options.action = libmorph::TextAction::None;
                ctx.start_text_input(&text, &options);
            }
            Ime::Stop => ctx.stop_text_input(),
            Ime::Keep => {}
        }
    }

    /// Whether it listens for the general agent, on both screens' cards.
    fn set_listening(&mut self, listening: bool) {
        self.overview.set_listening(listening);
        self.overview.invalidate();
        if let Some(agent) = &mut self.agent {
            agent.set_listening(listening);
            agent.invalidate();
        }
    }

    /// What STT is hearing, on both screens' Gemini cards.
    fn set_partial(&mut self, partial: Option<String>) {
        self.overview.set_partial(partial.clone());
        self.overview.invalidate();
        if let Some(agent) = &mut self.agent {
            agent.set_partial(partial);
            agent.invalidate();
        }
    }

    /// The phone is short of memory (plan 20261022_03, the L3 amendments
    /// of 2026-10-24 12:21 and 13:45 EDT): the client drops cached views
    /// and far contents, and the agent view frees the rows of blocks off
    /// screen. From `on_trim_memory` and `on_low_memory` ([`memory`]).
    pub fn memory_pressure(&mut self, pressure: Pressure) {
        log::info!("memory: {pressure:?}");
        self.send(Input::Memory(pressure));
        if let Some(agent) = &mut self.agent {
            agent.set_pressure(Some(pressure));
        }
    }

    /// Closes the agent view, back to the overview.
    fn close_agent(&mut self) {
        self.remember_prompts();
        self.agent_pane = None;
        if self.agent.take().is_some() {
            log::info!("agent view: closed");
            self.send(Input::CloseAgent);
            self.screen.invalidate();
        }
    }

    /// Opens the settings screen, from the overview's gear.
    fn open_settings(&mut self) {
        log::info!("settings: open");
        self.follow_permission();
        // The user's themes.toml is read again: a theme added or changed
        // there shows in the rows, and in force if it is the one chosen.
        theme::set_themes(theme::Themes::load(self.data_dir.as_deref()));
        self.apply_theme();
        let mut screen = SettingsScreen::new(
            self.settings.clone(),
            Box::new(settings::battery::Morph),
            cfg!(target_os = "android"),
        )
        .with_faces(Some(std::sync::Arc::clone(&self.faces)))
        .with_build_info(self.build_info.clone())
        .with_voices(self.voices.clone())
        .with_background(Box::new(background::Morph(std::rc::Rc::clone(
            &self.notify_ask,
        ))));
        screen.set_daemon(self.server_name.as_deref().unwrap_or("flankr"), self.online);
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
            SettingsHit::Voice(settings) => {
                self.apply_settings(settings);
                self.preview_voice();
            }
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
        if settings.background != self.settings.background && self.service == Service::Unavailable {
            self.service = Service::Off;
        }
        let voice_changed = settings.voice != self.settings.voice;
        self.settings = settings.clone();
        settings::set_current(settings.clone());
        if voice_changed {
            self.apply_voice();
        }
        self.notifier
            .set_alerts(settings.alerts && settings.background);
        if let Some(dir) = &self.data_dir
            && let Err(e) = settings.save(dir)
        {
            log::warn!("settings: {e}");
        }
        self.send(Input::Settings(settings.to_client()));
        // *Appearance* and the theme rows apply at once.
        self.apply_theme();
        if let Some(agent) = &mut self.agent {
            agent.set_scrollbar(settings.scrollbar);
            agent.set_sound(settings.sound);
            agent.invalidate();
        }
        self.overview.settings_changed();
    }

    /// The session speaks in the saved voice, or *Default* when Phonon does
    /// not have it. Before Phonon has loaded, nothing: it is set on load.
    fn apply_voice(&mut self) {
        let (Some(audio), Some(names)) = (&mut self.audio, &self.voices) else {
            return;
        };
        let voice = settings::voice::effective(self.settings.voice.as_deref(), names);
        if self.settings.voice.is_some() && voice.is_none() {
            log::info!("settings: the saved voice is not installed; Default");
        }
        audio.set_voice(voice);
    }

    /// A *Voice* step's preview, "This is <voice>.", through the audio as
    /// any `Say` is, and only while *All sound* is on. Anything being said,
    /// as the last step's preview, is cut off first.
    fn preview_voice(&mut self) {
        if !self.settings.sound {
            return;
        }
        let Some(audio) = &mut self.audio else {
            return;
        };
        let voice = audio.voice().map(str::to_string);
        for action in settings::voice::preview(voice.as_deref(), self.previewing || self.speaking) {
            audio.act(action);
        }
        self.previewing = true;
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
        let mut overview = Overview::new();
        overview.shader_panels = self.panel_shader.is_some();
        self.overview = overview;
        self.screen.invalidate();
        self.link = "not paired \u{b7} paste a pairing URL below".into();
        self.note = Some(if daemon {
            ("unpaired".into(), Role::Muted)
        } else {
            (
                "unpaired on this phone; flankr lists it until it is removed from auth.toml".into(),
                Role::Warn,
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
                    self.note = Some((format!("deleting the data: {e}"), Role::Warn));
                }
            }
        }
        self.settings = Settings::default();
        settings::set_current(self.settings.clone());
        self.apply_voice();
        self.send(Input::Settings(self.settings.to_client()));
        self.overview.settings_changed();
    }

    /// A tap on one of the agent view's controls, or the overview's.
    fn agent_hit(&mut self, hit: AgentHit) {
        log::info!("companion: agent_hit: {hit:?}");
        match hit {
            AgentHit::Back => self.close_agent(),
            AgentHit::Message => self.send(Input::StartDictation),
            // The centre button: stops what is being said to Gemini, or
            // starts listening. Its card stays as the user left it (plan
            // 20261027_03, decision 4).
            AgentHit::GeneralAgent => {
                if self
                    .dictation
                    .as_ref()
                    .is_some_and(|d| d.general_agent && d.active)
                {
                    self.keyboard.close();
                    self.dictation = None;
                    self.set_listening(false);
                    self.send(Input::Clear);
                } else {
                    self.dictation = Some(flankr_companion::Dictation {
                        active: true,
                        mode: flankr_companion::PromptMode::Voice,
                        text: String::new(),
                        target: None,
                        focus_moved: None,
                        phase: flankr_companion::DictationPhase::Collecting,
                        submit_in: None,
                        general_agent: true,
                    });
                    self.set_listening(true);
                    self.send(Input::StartGeneralAgentPrompt(
                        flankr_companion::PromptMode::Voice,
                    ));
                }
            }
            // The keyboard glyph: a typed prompt, to Gemini while its card
            // is expanded, else a message. The keyboard route focuses it.
            AgentHit::Keyboard => {
                if self.prompts().expanded() == Some(PromptKind::GeneralAgent) {
                    self.agent_hit(AgentHit::PromptText(PromptKind::GeneralAgent));
                } else {
                    self.send(Input::StartTextPrompt);
                    self.focus_prompt(PromptKind::Message);
                }
            }
            // A tap on a card's text area: focused, and the soft keyboard
            // shown (again, if it was dismissed). Gemini's starts a typed
            // prompt if none is open.
            AgentHit::PromptText(kind) => {
                let typing = self
                    .dictation
                    .as_ref()
                    .is_some_and(|d| d.general_agent && d.mode == PromptMode::Keyboard);
                if kind == PromptKind::GeneralAgent && !typing {
                    self.send(Input::StartGeneralAgentPrompt(PromptMode::Keyboard));
                }
                self.focus_prompt(kind);
            }
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
            // One of the daemon's answers (plan 20261027_06, step 4): one
            // that opens the message card takes it in reply to the card.
            AgentHit::Answer { index, opens, .. } => {
                if let Some(answer) = self.agent.as_ref().and_then(|a| a.answer(index)) {
                    if opens && let Some(agent) = &mut self.agent {
                        agent.begin_reply();
                    }
                    self.send(Input::Suggested(answer));
                }
            }
            AgentHit::Link(index) => {
                // Opened at the end of the touch, with the context.
                if let Some(url) = self
                    .agent
                    .as_ref()
                    .and_then(|a| a.link(index))
                    .map(|l| l.url.clone())
                {
                    self.links.push(url);
                }
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
            // Sent or cancelled: the keyboard goes with the prompt.
            AgentHit::Submit => {
                self.keyboard.close();
                self.send(Input::Submit);
            }
            AgentHit::Scratch => self.send(Input::Scratch),
            AgentHit::Clear => {
                self.keyboard.close();
                self.send(Input::Clear);
            }
            // The prompt cards (plan 20261027_03): the screen expanded or
            // collapsed one; the state is remembered (decision 5).
            AgentHit::PromptExpand(_) | AgentHit::PromptCollapse | AgentHit::PromptBars => {
                self.remember_prompts();
            }
        }
    }

    /// Starts the foreground service when it is wanted (*Stay connected in
    /// the background* on, and paired) and stops it when not. Called on
    /// each frame, so only while flankr is on screen, as Android requires
    /// of a start.
    fn sync_service(&mut self) {
        let wanted = self.settings.background && self.paired && self.companion.is_some();
        match (wanted, self.service) {
            (true, Service::Off) if self.visible => match self.notifier.start() {
                Ok(()) => {
                    log::info!("service: starting");
                    self.service = Service::Starting;
                }
                Err(e) => {
                    log::info!("service: not started: {e}");
                    self.service = Service::Unavailable;
                }
            },
            (false, Service::Starting | Service::Running) => {
                log::info!("service: stopping");
                self.notifier.stop();
                self.service = Service::Off;
            }
            _ => {}
        }
    }

    /// *Alert when an agent needs you* was turned on: the one place
    /// `POST_NOTIFICATIONS` is asked for.
    fn alerts_turned_on(&mut self, ctx: &mut MorphContext) {
        let asked = self
            .data_dir
            .as_deref()
            .is_some_and(background::asked_before);
        match background::turn_on(&mut ContextPermissions(ctx), asked) {
            TurnedOn::Granted => {
                self.notifications_granted = true;
                log::info!("notifications: already granted");
            }
            TurnedOn::Requested => {
                log::info!("notifications: asking");
                self.asking_permission = true;
                if let Some(dir) = &self.data_dir {
                    background::mark_asked(dir);
                }
            }
            TurnedOn::OpenedSettings => {
                log::info!("notifications: Android won't ask again; the app's settings opened")
            }
        }
    }

    /// The alerts toggle shows the real permission: off without it. Not
    /// while a request is out.
    fn follow_permission(&mut self) {
        if self.asking_permission {
            return;
        }
        let alerts = background::follow(self.settings.alerts, self.notifications_granted);
        if alerts != self.settings.alerts {
            log::info!("notifications: not granted; the alerts are off");
            let settings = Settings {
                alerts,
                ..self.settings.clone()
            };
            self.apply_settings(settings.clone());
            if let Some(screen) = &mut self.settings_screen {
                screen.set_settings(settings);
            }
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
                    if !on {
                        self.previewing = false;
                    }
                    if let Some(agent) = &mut self.agent {
                        agent.set_speaking(on);
                    }
                }
                match &input {
                    Input::Utterance(_) => {
                        self.set_partial(None);
                        // A new utterance: an answered card has had its say.
                        if self
                            .card
                            .as_ref()
                            .is_some_and(|c| matches!(c.state, CardState::Done(_)))
                        {
                            self.card = None;
                        }
                    }
                    Input::Listening(false) => {
                        self.set_partial(None);
                        self.set_listening(false);
                    }
                    _ => {}
                }
                self.send(input);
            }
            AudioEvent::SttLoaded { load_ms } => {
                log::info!("audio: STT loaded in {load_ms} ms");
                self.sound = "ready".into();
            }
            AudioEvent::SttLoadFailed(e) => {
                log::error!("audio: STT: {e}");
                self.sound = format!("STT model missing: {e}");
            }
            AudioEvent::TtsLoaded {
                load_ms,
                voices,
                names,
            } => {
                log::info!("audio: TTS loaded in {load_ms} ms, {voices} voices");
                self.voices = Some(names);
                if let Some(screen) = &mut self.settings_screen {
                    screen.set_voices(self.voices.clone());
                }
                // A saved voice is said from here; none leaves `--voice`.
                if self.settings.voice.is_some() {
                    self.apply_voice();
                }
            }
            AudioEvent::TtsLoadFailed(e) => {
                log::error!("audio: TTS: {e}");
                self.sound = format!("TTS model missing: {e}");
            }
            AudioEvent::Partial(text) => self.set_partial(Some(text)),
            AudioEvent::Gate(state) => {
                self.sound = match state {
                    GateState::Idle => "ready",
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

    /// A touch on the screen shown: on the overview and in the agent view,
    /// every phase goes to the screen (a drag scrolls, a tap hits); on the
    /// pairing screen, only a touch's end counts.
    fn touch(&mut self, touch: Touch) {
        let now = self.started.elapsed();
        if let Some(hold) = self.center_hold.as_mut()
            && hold.id == touch.id
        {
            match touch.phase {
                TouchPhase::Ended => {
                    let opened = hold.opened;
                    let started_listening = hold.started_listening;
                    self.center_hold = None;
                    if !opened && !started_listening {
                        self.agent_hit(AgentHit::GeneralAgent);
                    }
                    return;
                }
                TouchPhase::Cancelled => {
                    self.center_hold = None;
                    return;
                }
                _ => return,
            }
        }

        if touch.phase == TouchPhase::Started {
            let on_center = if let Some(agent) = &self.agent {
                agent.hit(touch.x, touch.y) == Some(AgentHit::GeneralAgent)
            } else if !self.pairing() {
                self.overview.hit(touch.x, touch.y) == Some(AgentHit::GeneralAgent)
            } else {
                false
            };
            if on_center {
                let currently_listening = self.dictation.as_ref().is_some_and(|d| {
                    d.general_agent && d.active && d.mode == flankr_companion::PromptMode::Voice
                });
                let started_listening = if !currently_listening {
                    // Activate immediately on press: start voice recognition, animation, and Gemini card!
                    self.dictation = Some(flankr_companion::Dictation {
                        active: true,
                        mode: flankr_companion::PromptMode::Voice,
                        text: String::new(),
                        target: None,
                        focus_moved: None,
                        phase: flankr_companion::DictationPhase::Collecting,
                        submit_in: None,
                        general_agent: true,
                    });
                    self.set_listening(true);
                    self.send(Input::StartGeneralAgentPrompt(
                        flankr_companion::PromptMode::Voice,
                    ));
                    true
                } else {
                    false
                };
                self.center_hold = Some(CenterHold {
                    id: touch.id,
                    start: now,
                    opened: false,
                    started_listening,
                });
                return;
            }
        }

        // Another session's card, over whichever screen is shown, unless a
        // prompt card's overlay is over it.
        let s = self.scale();
        let overlay = self.settings_screen.is_none() && self.prompts().overlay().is_shown();
        if !overlay {
            match self
                .session_cards
                .touch(touch.id, touch.phase, touch.x, touch.y, s)
            {
                Touched::Yes(hit) => return self.session_hit(hit),
                Touched::No => {}
            }
        }
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

    /// The display's scale, as the screens were last laid out at.
    fn scale(&self) -> f32 {
        self.scale
    }

    /// A tap on another session's card opens it; a swipe dismisses it.
    fn session_hit(&mut self, hit: Option<SessionHit>) {
        match hit {
            Some(SessionHit::Open(pane_id)) => {
                log::info!("cards: opening another session");
                self.close_settings();
                if self.agent_pane.as_deref() != Some(pane_id.as_str()) {
                    self.open_agent(pane_id);
                }
            }
            Some(SessionHit::Dismiss(pane_id)) => {
                log::info!("cards: dismissed");
                self.send(Input::DismissCard(pane_id));
            }
            None => {}
        }
    }

    /// Lays other sessions' cards out at the bottom of the screen shown:
    /// the screen makes room for them, except the settings, which they sit
    /// over.
    fn lay_out_session_cards(&mut self, layout: Layout) {
        let s = if layout.scale > 0.0 {
            layout.scale
        } else {
            1.0
        };
        self.scale = s;
        let pad = crate::agent::PAD * s;
        let w = layout.w - 2.0 * pad;
        let h = self.session_cards.height(w, s);
        let bottom = if self.settings_screen.is_some() {
            Some(layout.h - layout.bottom - 16.0 * s)
        } else if let Some(agent) = &mut self.agent {
            agent.set_cross_height(h);
            Some(agent.cross_bottom())
        } else if !self.pairing() {
            self.overview.set_cross_height(h);
            Some(self.overview.cross_bottom())
        } else {
            None
        };
        match bottom {
            Some(bottom) => self.session_cards.place(pad, bottom, w, s),
            None => self.session_cards.place(pad, 0.0, w, s),
        }
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
            bottom: safe.bottom + ctx.keyboard_inset(),
        };
        let ga_listening = self.dictation.as_ref().is_some_and(|d| {
            d.active && d.general_agent && d.mode == flankr_companion::PromptMode::Voice
        });
        // The prompt cards' inputs (plan 20261027_03): a message to a
        // session, and a typed prompt to Gemini.
        let message = self
            .dictation
            .as_ref()
            .filter(|d| !d.general_agent)
            .map(|d| self.message_prompt(d));
        let gemini = self
            .dictation
            .as_ref()
            .filter(|d| d.general_agent && d.mode == PromptMode::Keyboard)
            .map(prompt_input);

        if let Some(screen) = &mut self.settings_screen {
            screen.set_layout(layout);
            screen.set_daemon(self.server_name.as_deref().unwrap_or("flankr"), self.online);
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
            o.set_general_agent_listening(ga_listening);
            let note = self
                .note
                .as_ref()
                .map(|(t, r)| (t.clone(), theme().get(*r)));
            o.set_status(vec![self.link.clone(), sound.clone()], note);
            o.set_card(self.card.as_ref().map(|c| Panel {
                lines: c.lines(),
                confirm: c.asks().then_some(c.confirm_id),
            }));
            o.set_dictation(message.clone());
            o.set_gemini_input(gemini.clone());
            o.set_play(self.read_aloud, self.speaking, self.viewed);
        }

        if let Some(agent) = &mut self.agent {
            agent.set_layout(layout);
            agent.set_general_agent_listening(ga_listening);
            agent.set_dictation(message);
            agent.set_gemini_input(gemini);
            // The band in its session's colour.
            let colour = self
                .agent_pane
                .as_deref()
                .and_then(|p| self.agents.iter().find(|a| a.pane_id == p))
                .map(flankr_companion::cross::colour_of);
            agent.set_session_colour(colour);
            self.lay_out_session_cards(layout);
            return;
        }
        self.lay_out_session_cards(layout);

        if !self.pairing() {
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
                format!("Paste this code into flankr pair on {server}:"),
                15.0,
                theme().muted,
                0.0,
            ));
            // Base58 is mostly capitals and wide letters: wider than the
            // half an em `columns` assumes.
            for part in chunk(device_code, layout.columns(26.0 * 1.3)) {
                code.push(line(part, 26.0, theme().code, 4.0));
            }
        }
        if let Some((note, color)) = &self.note {
            for (i, t) in wrap(note, layout.columns(15.0)).into_iter().enumerate() {
                code.push(line(
                    t,
                    15.0,
                    theme().get(*color),
                    if i == 0 { 8.0 } else { 0.0 },
                ));
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
    /// `libmorph.flankr-companion`, on Android and in the macOS Keychain.
    const SECURE_STORE_ID: &'static str = "flankr-companion";

    /// The desktop's data and cache directories' name (libmorph 0.4.0):
    /// `~/Library/Application Support/dev.flankr.companion` on macOS. Android
    /// uses the package's own, whose id is the same.
    const APP_ID: &'static str = APP_ID;

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
        self.visible = false;
        self.notifier.visible(false);
    }

    /// Back on screen: the link wakes and catches up, and memory pressure
    /// is over.
    fn on_resume(&mut self, ctx: &mut MorphContext) {
        self.send(Input::Background(false));
        self.visible = true;
        self.notifier.visible(true);
        if self.service == Service::Unavailable {
            self.service = Service::Off;
        }
        // The permission may have changed in Android's settings.
        self.notifications_granted = ctx.permission_granted(POST_NOTIFICATIONS);
        self.follow_permission();
        if let Some(agent) = &mut self.agent {
            agent.set_pressure(None);
        }
        // Back from Android's battery dialog or pages, perhaps.
        if let Some(screen) = &mut self.settings_screen {
            screen.refresh_battery();
        }
    }

    /// The answer to the `POST_NOTIFICATIONS` request: refused, the alerts
    /// go off.
    fn on_permissions(&mut self, _ctx: &mut MorphContext, answer: &libmorph::Permissions) {
        self.asking_permission = false;
        self.notifications_granted = answer.granted(POST_NOTIFICATIONS);
        log::info!(
            "notifications: {}",
            if self.notifications_granted {
                "granted"
            } else {
                "refused"
            }
        );
        self.follow_permission();
    }

    /// The foreground service (libmorph 0.4.0), whether or not flankr is on
    /// screen. Exit stops it and closes the app, as libmorph's `Exited`
    /// says.
    fn on_service(&mut self, ctx: &mut MorphContext, event: &libmorph::service::ServiceEvent) {
        use libmorph::service::ServiceEvent as E;
        match event {
            E::Started => {
                log::info!("service: running");
                self.service = Service::Running;
                self.notifier.running(true);
            }
            E::Stopped | E::TimedOut => {
                log::info!("service: stopped ({event:?})");
                self.service = Service::Off;
                self.notifier.running(false);
            }
            E::Failed(why) => {
                log::warn!("service: failed: {why}");
                self.service = Service::Unavailable;
                self.notifier.running(false);
            }
            E::NotificationHidden => log::info!(
                "service: running; without POST_NOTIFICATIONS its notification shows only in the running apps list"
            ),
            E::Exited => {
                log::info!("service: Exit: closing flankr");
                self.service = Service::Off;
                self.notifier.running(false);
                ctx.exit();
            }
            E::WakeLock(held) => log::info!(
                "service: wake lock {}",
                if *held { "held" } else { "released" }
            ),
            E::TaskRemoved => log::info!("service: swiped from Recents; still running"),
            E::Action(id) => log::info!("service: action {id:?}"),
            other => log::debug!("service: {other:?}"),
        }
    }

    /// Android asked the app to trim memory (libmorph 0.4.1): the level's
    /// grade, as the 13:45 EDT L3 amendment maps it ([`memory`]).
    fn on_trim_memory(&mut self, _ctx: &mut MorphContext, level: libmorph::TrimLevel) {
        log::info!("memory: trim {level:?}");
        if let Some(pressure) = memory::pressure_for(level) {
            self.memory_pressure(pressure);
        }
    }

    /// The system is low on memory: the full drop, so the app is not
    /// killed.
    fn on_low_memory(&mut self, _ctx: &mut MorphContext) {
        log::info!("memory: low");
        self.memory_pressure(memory::LOW_MEMORY);
    }

    /// A `flankr://` link opened the app (libmorph 0.3.2). Only a pairing
    /// link (`flankr://<host>:<port>/pair#…`) is acted on; its fragment is
    /// the pairing string, so nothing past the path is logged.
    fn on_open_url(&mut self, _ctx: &mut MorphContext, url: &str) {
        let before_fragment = url.split(['#', '?']).next().unwrap_or("");
        let is_pairing = before_fragment
            .strip_prefix("flankr://")
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
        // libmorph's data directory, on every host with a filesystem; the
        // companion's old one where there is none (the web).
        let data_dir = ctx
            .data_dir()
            .map(std::path::Path::to_path_buf)
            .or_else(|| Some(FileKeyStore::default_dir()));
        // The desktop's settings.toml was in ~/.config/flankr-companion
        // until libmorph 0.4.0: moved once into the data directory. Only
        // into the app's own directory, never a scripted run's `storage/`.
        #[cfg(not(target_os = "android"))]
        if let Some(dir) = data_dir.as_deref()
            && dir.file_name() == Some(std::ffi::OsStr::new(APP_ID))
        {
            settings::move_old(&FileKeyStore::default_dir(), dir);
        }
        let settings = data_dir.as_deref().map(Settings::load).unwrap_or_default();
        settings::set_current(settings.clone());
        // The themes: the shipped ones and the user's themes.toml.
        theme::set_themes(theme::Themes::load(data_dir.as_deref()));
        theme::set_theme(settings.theme(&theme::themes()));
        theme::apply_system_bars(theme::SystemBars::of(&theme()));
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
        let panel_shader = Some(overview::PanelShader::new(
            ctx.device(),
            ctx.surface_format(),
        ));
        let mut overview = Overview::new();
        overview.shader_panels = true;
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
            overview,
            panel_shader,
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
            voices: None,
            previewing: false,
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
            links: Vec::new(),
            build_info: settings::build_info::build_info(ctx),
            notifier: Notifier::spawn(std::sync::Arc::new(notify::MorphHost), notify::MIN_GAP),
            service: Service::Off,
            visible: true,
            notify_ask: Default::default(),
            notifications_granted: ctx.permission_granted(POST_NOTIFICATIONS),
            asking_permission: false,
            session_cards: SessionCards::new(),
            agent_pane: None,
            scale: 1.0,
            keyboard: KeyboardRoute::new(cfg!(not(target_os = "android"))),
            center_hold: None,
            gemini_turns: Vec::new(),
            sessions: HashMap::new(),
            agent_gemini_expanded: false,
        };
        app.notifier
            .set_alerts(app.settings.alerts && app.settings.background);
        // The toggle shows the real permission; nothing is asked at start.
        app.follow_permission();

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
                        .max(flankr_companion::DICTATION_SILENCE)
                        .as_secs_f64()
                );
                let companion = Companion::spawn_tapped(config, keys, app.notifier.tap());
                companion.send(Input::Settings(app.settings.to_client()));
                app.companion = Some(companion);
            }
            Err(e) => {
                log::error!("keys: {e}");
                app.link = "no key storage".into();
                app.note = Some((e, Role::Warn));
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
                let silent = flankr_audio::Format {
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
        // `open_url`, and build.rs adds `.url_scheme("flankr")`.
        app
    }

    #[cfg(target_os = "android")]
    fn on_capture_ready(&mut self, ctx: &mut MorphContext) {
        self.start_morph_audio(ctx);
    }

    fn update(&mut self, ctx: &mut MorphContext, _dt: Duration) {
        if self.notify_ask.take() {
            self.alerts_turned_on(ctx);
        }
        self.sync_service();
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
        self.sync_keyboard(ctx);

        let dt = now.saturating_duration_since(self.last_update);
        self.last_update = now;
        // The 2 s hold opens Gemini's card pinned, not the overlay; the
        // user expands it (plan 20261027_03, decision 7).
        let elapsed = self.started.elapsed();
        if self
            .center_hold
            .as_mut()
            .is_some_and(|h| h.open_at(elapsed))
        {
            self.with_prompts(Prompts::hold_open);
        }
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

    /// The controls, and a tap anywhere else ([`App::touch`]); then any
    /// link it tapped opens.
    fn on_touch(&mut self, ctx: &mut MorphContext, touch: Touch) {
        self.touch(touch);
        // Links tapped in the settings' copy and the agent view.
        let mut links = std::mem::take(&mut self.links);
        if let Some(screen) = &mut self.settings_screen {
            links.extend(screen.take_links());
        }
        for url in links {
            open_link(ctx, &url);
        }
    }

    /// Typing into the focused field (desktop): Enter pairs, Escape clears.
    fn on_key(&mut self, _ctx: &mut MorphContext, key: Key) -> KeyStatus {
        let back = matches!(key, Key::Back | Key::Escape);
        // The system Back cancels the settings' modal, or closes them.
        if back && let Some(screen) = &mut self.settings_screen {
            if !screen.back() {
                self.close_settings();
            }
            return KeyStatus::Handled;
        }
        // Back collapses an expanded prompt card (plan 20261027_03,
        // decision 4), and leaves what is in it.
        if back && !self.pairing() && self.prompts().expanded().is_some() {
            self.with_prompts(|p| {
                p.back();
            });
            return KeyStatus::Handled;
        }
        // Back or Escape cancels an active prompt card / dictation.
        if back && self.dictation.is_some() {
            self.keyboard.close();
            self.send(Input::Clear);
            return KeyStatus::Handled;
        }
        // A physical keyboard types into the typed prompt directly; the
        // soft keyboard stays hidden from then on (the keyboard route).
        if let Some(d) = &self.dictation
            && d.mode == PromptMode::Keyboard
        {
            let Some(text) = self.keyboard.key(&d.text, key) else {
                return KeyStatus::Unhandled;
            };
            self.send(Input::SetText(text));
            return KeyStatus::Handled;
        }
        // The system Back closes the agent view.
        if self.agent.is_some() && back {
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

    fn on_text_input(&mut self, _ctx: &mut MorphContext, event: &libmorph::TextInputEvent) {
        log::info!("companion: on_text_input: {event:?}");
        match event {
            libmorph::TextInputEvent::Changed(text) => {
                self.send(Input::SetText(text.clone()));
            }
            libmorph::TextInputEvent::Action(_) => {
                self.keyboard.close();
                self.send(Input::Submit);
            }
            // The card and its text stay; a tap shows it again.
            libmorph::TextInputEvent::Dismissed => self.keyboard.dismiss(),
            _ => {}
        }
    }

    fn render(&mut self, ctx: &mut MorphContext, frame: &Frame<'_>) {
        let font = |id| self.fonts.atlas(id).expect("registered in this set");
        let mut draw = DrawContext {
            px_range: BUNDLED_ATLAS_PX_RANGE,
            ..DrawContext::new(Some(font(self.roboto).atlas), Some(&self.shaper))
        };
        // libcontrols' own colours that the app draws: the settings' scroll
        // indicator and the insertion cursor, in the theme.
        draw.theme.accent_color = theme().accent;
        draw.theme.scroll.indicator_color = theme().scrollbar;
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
        // Another session's card changed: the screen under it draws again.
        let cards_changed = self.session_cards.take_dirty();
        // Another screen drew over the list: the one shown now draws it all.
        if self.shown != Some(shown) || cards_changed {
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
        let ga_listening = self.dictation.as_ref().is_some_and(|d| {
            d.active && d.general_agent && d.mode == flankr_companion::PromptMode::Voice
        });
        if ga_listening {
            self.overview.invalidate();
            if let Some(agent) = &mut self.agent {
                agent.invalidate();
            }
        }
        let drew = match (&mut self.settings_screen, &mut self.agent) {
            (Some(screen), _) => screen.draw(&mut self.list, &draw, Some(&term)),
            (None, Some(agent)) => agent.draw(&mut self.list, &draw, Some(&term)),
            (None, None) if shown == Shown::Pairing => self.screen.draw(&mut self.list, &draw),
            (None, None) => self.overview.draw(&mut self.list, &draw, Some(&term)),
        };
        // The card goes over what the screen drew, unless a prompt card's
        // overlay is over it (each screen draws its own, plan 20261027_03).
        let overlay = shown != Shown::Settings && self.prompts().overlay().is_shown();
        if drew && shown != Shown::Pairing && !overlay {
            let s = ctx.scale_factor();
            self.session_cards.draw(&mut self.list, &draw, s);
        }
        let (w, h) = frame.size;
        let s = ctx.scale_factor();
        if shown == Shown::Overview && self.panel_shader.is_some() {
            let panels = self.overview.panel_instances();
            if let Some(shader) = &mut self.panel_shader {
                shader.draw(
                    ctx.device(),
                    ctx.queue(),
                    frame.view,
                    ctx.surface_format(),
                    w,
                    h,
                    s,
                    theme().bg,
                    &panels,
                );
            }
            let frame_sdf = self.list.lower_with(&self.fonts);
            self.sdf.upload_chars(ctx.queue(), &frame_sdf.char_buffer);
            self.sdf.upload_params(ctx.queue(), &frame_sdf.param_bank);
            self.sdf.render_full_scaled_load(
                ctx.device(),
                ctx.queue(),
                frame.view,
                w,
                h,
                1.0,
                &frame_sdf.draws,
                0.0,
                &[0; 16],
                &[],
                None,
            );
        } else {
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
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: libmorph::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("flankr"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{CardState, PromptOverlay};

    #[test]
    fn the_2_s_hold_opens_geminis_card_pinned_not_the_overlay() {
        let start = Duration::from_secs(10);
        let mut hold = CenterHold {
            id: 1,
            start,
            opened: false,
            started_listening: true,
        };
        assert!(!hold.open_at(start + Duration::from_millis(1999)));
        assert!(hold.open_at(start + CenterHold::OPENS));
        assert!(!hold.open_at(start + Duration::from_secs(3)), "once");
        // What it opens, on either screen: the card pinned, even with
        // nothing said yet, and the overlay closed, even if it was left
        // expanded.
        let mut o = Overview::new();
        o.set_layout(Layout {
            w: 1080.0,
            h: 2340.0,
            scale: 2.0,
            top: 80.0,
            bottom: 40.0,
        });
        assert_eq!(o.prompts().gemini().rect().h, 0.0);
        o.with_prompts(Prompts::hold_open);
        o.tick(PromptOverlay::SLIDE);
        assert!(o.prompts().gemini().rect().h > 0.0);
        assert!(!o.prompts().overlay().is_shown());
        o.with_prompts(|p| p.expand(PromptKind::GeneralAgent));
        o.tick(PromptOverlay::SLIDE);
        assert!(o.prompts().overlay().is_open());
        o.with_prompts(Prompts::hold_open);
        assert!(!o.prompts().overlay().is_open());
        assert_eq!(
            o.prompts().state(PromptKind::GeneralAgent),
            CardState::Pinned
        );
        let mut a = AgentScreen::new();
        a.set_layout(Layout {
            w: 1080.0,
            h: 2340.0,
            scale: 2.0,
            top: 80.0,
            bottom: 40.0,
        });
        a.with_prompts(Prompts::hold_open);
        assert!(a.prompts().gemini().rect().h > 0.0);
        assert!(!a.prompts().overlay().is_open());
    }
}
