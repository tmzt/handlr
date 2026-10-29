//! The settings (docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md): what
//! they are, where they are kept, and what each one turns off.
//!
//! - [`Settings`] is the screen's toggles and the scrollbar's visibility
//!   (the 11:32 EDT amendment), kept in `settings.toml` in the app's data
//!   directory (decision 5): written on every change, read at start, the
//!   defaults when missing or unreadable. On the desktop that is libmorph's
//!   data directory since 0.4.0; [`move_old`] brings the file over once
//!   from `~/.config/flankr-companion`.
//! - A toggle that is off disables its UI throughout the app (decision 4).
//!   Each control asks [`allows`] whether it is live, so no screen keeps
//!   its own switch: a disabled control is drawn at [`DISABLED_ALPHA`]
//!   ([`dim_since`]), shows nothing live, and takes no taps. The values
//!   asked are the UI thread's, set with [`set_current`].
//! - The screen is `settings/screen.rs`, one control per file beside it,
//!   and the gear that opens it is `overview/gear.rs`.
//! - [`delete_data`] is *Delete data*'s part on disk: everything in the
//!   data directory but the voice models.

pub mod background;
pub mod battery;
pub mod build_info;
pub mod confirm;
pub mod copy;
pub mod danger_row;
pub mod info;
pub mod link_row;
pub mod option_row;
pub mod plugin_row;
pub mod remote_details;
pub mod remote_row;
pub mod screen;
pub mod toggle_row;
pub mod voice;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use libmsdf::drawlist::DrawList;
use serde::{Deserialize, Serialize};

use flankr_companion::wire::LlmInfo;

use crate::agent::{AgentHit, ScrollbarVisibility};
use crate::theme::{self, Theme, Themes};

pub use screen::{SettingsHit, SettingsScreen};

/// The file the settings are kept in, in the data directory.
pub const FILE: &str = "settings.toml";

/// How opaque a disabled control is drawn (decision 4).
pub const DISABLED_ALPHA: f32 = 0.35;

/// The settings screen's values. Each toggle is on by default but
/// *Alert when an agent needs you* and *Tab follows focus*, the scrollbar
/// *Hidden*, and the voice *Default*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// *All sound*.
    pub sound: bool,
    /// *Ding when an agent is done*; only while `sound` is on.
    pub ding: bool,
    /// *Speak coding responses* (plan 20261028_06, decision 6): the
    /// agent's replies read aloud (`Input::ReadAloud`); off until turned
    /// on, as the play button's toggle was at every start.
    pub speak_coding: bool,
    /// *Speak agent (Gemini) responses*: Gemini's `turn.say` lines spoken;
    /// on by default, as they always were.
    pub speak_gemini: bool,
    /// The volume button's mute (plan 20261028_06, its 2026-10-29 00:26
    /// EDT amendment): above both speak options, which keep their values;
    /// muted, the client is told as if both were off. No Settings row;
    /// off by default.
    pub muted: bool,
    /// *Microphone*.
    pub microphone: bool,
    /// *General agent (Gemini)*.
    pub general_agent: bool,
    /// *Tab follows focus* (plan 20261027_04, step 6): off until turned
    /// on; on, Gemini's `focus_project` also moves herdr's focused tab.
    pub tab_follows_focus: bool,
    /// *Scrollbar*, the agent view's.
    pub scrollbar: ScrollbarVisibility,
    /// *Stay connected in the background* (CHANGES.md 2026-10-24 12:38
    /// EDT): the foreground service, once libmorph 0.4.0 is in.
    pub background: bool,
    /// *Alert when an agent needs you*: off until turned on.
    pub alerts: bool,
    /// *Voice* (CHANGES.md 2026-10-24 14:58 EDT): a Phonon voice's name,
    /// or `None` for *Default*. Kept as saved when it is not installed;
    /// [`voice::effective`] says what is spoken.
    pub voice: Option<String>,
    /// *Appearance* (plan 20261024_05, the 15:33 EDT amendment, item 3).
    pub appearance: Appearance,
    /// *Light theme* and *Dark theme*: a theme's id, or `None` for the
    /// first of its kind. Kept as saved when no theme has it.
    pub light_theme: Option<String>,
    pub dark_theme: Option<String>,
}

/// Which kind of theme is in force: the phone's (*System*), or always one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    /// As `settings.toml` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    /// As the screen shows it.
    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => copy::APPEARANCE_SYSTEM.text,
            Appearance::Light => copy::APPEARANCE_LIGHT.text,
            Appearance::Dark => copy::APPEARANCE_DARK.text,
        }
    }

    /// The next one the right arrow (`forward`) or the left gives,
    /// wrapping: *System*, *Always light*, *Always dark*.
    pub fn step(self, forward: bool) -> Self {
        let i = Self::ALL.iter().position(|&a| a == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }

    /// Whether a dark theme is in force, with the phone in dark mode or
    /// not (`system_dark`, `None` when it is not known: then dark).
    pub fn is_dark(self, system_dark: Option<bool>) -> bool {
        match self {
            Appearance::System => system_dark.unwrap_or(true),
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }

    /// Whether the row for themes of the kind `dark` is active: both are
    /// with *System*, and only its own kind's otherwise.
    pub fn row_active(self, dark: bool) -> bool {
        self == Appearance::System || self.is_dark(None) == dark
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound: true,
            ding: true,
            speak_coding: false,
            speak_gemini: true,
            muted: false,
            microphone: true,
            general_agent: true,
            tab_follows_focus: false,
            scrollbar: ScrollbarVisibility::Hidden,
            background: true,
            alerts: false,
            voice: None,
            appearance: Appearance::System,
            light_theme: None,
            dark_theme: None,
        }
    }
}

/// `settings.toml`'s layout. A key left out is its default.
#[derive(Debug, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default = "on")]
    sound: bool,
    #[serde(default = "on")]
    ding: bool,
    #[serde(default)]
    speak_coding: bool,
    #[serde(default = "on")]
    speak_gemini: bool,
    #[serde(default)]
    muted: bool,
    #[serde(default = "on")]
    microphone: bool,
    #[serde(default = "on")]
    general_agent: bool,
    #[serde(default)]
    tab_follows_focus: bool,
    /// `disabled`, `hidden` or `always`.
    #[serde(default = "hidden")]
    scrollbar: String,
    #[serde(default = "on")]
    background: bool,
    #[serde(default)]
    alerts: bool,
    /// Left out for *Default*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    voice: Option<String>,
    /// `system`, `light` or `dark`.
    #[serde(default = "system")]
    appearance: String,
    /// Theme ids; left out for the first of their kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    light_theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dark_theme: Option<String>,
}

fn system() -> String {
    Appearance::System.name().to_string()
}

fn on() -> bool {
    true
}

fn hidden() -> String {
    scrollbar_name(ScrollbarVisibility::Hidden).to_string()
}

/// The scrollbar's visibility as `settings.toml` writes it.
pub fn scrollbar_name(mode: ScrollbarVisibility) -> &'static str {
    match mode {
        ScrollbarVisibility::Disabled => "disabled",
        ScrollbarVisibility::Hidden => "hidden",
        ScrollbarVisibility::Always => "always",
    }
}

/// The scrollbar's visibility as the screen shows it.
pub fn scrollbar_label(mode: ScrollbarVisibility) -> &'static str {
    match mode {
        ScrollbarVisibility::Disabled => copy::SCROLLBAR_NONE.text,
        ScrollbarVisibility::Hidden => copy::SCROLLBAR_ON_SCROLL.text,
        ScrollbarVisibility::Always => copy::SCROLLBAR_ALWAYS.text,
    }
}

/// The next visibility the right arrow (`forward`) or the left gives,
/// wrapping at both ends: *None*, *On Scroll*, *Always*.
pub fn step_scrollbar(mode: ScrollbarVisibility, forward: bool) -> ScrollbarVisibility {
    use ScrollbarVisibility::*;
    match (mode, forward) {
        (Disabled, true) | (Always, false) => Hidden,
        (Hidden, true) | (Disabled, false) => Always,
        (Always, true) | (Hidden, false) => Disabled,
    }
}

impl Settings {
    /// What the client acts on: everything but the scrollbar.
    pub fn to_client(&self) -> flankr_companion::Settings {
        flankr_companion::Settings {
            sound: self.sound,
            ding: self.ding,
            microphone: self.microphone,
            general_agent: self.general_agent,
            tab_follows_focus: self.tab_follows_focus,
            speak_gemini: self.speak_gemini && !self.muted,
        }
    }

    /// *Speak coding responses* as the client's read aloud
    /// (`Input::ReadAloud`): off while muted.
    pub fn read_aloud(&self) -> bool {
        self.speak_coding && !self.muted
    }

    /// `settings.toml` in `dir`. Missing or unreadable, the defaults; an
    /// unknown scrollbar value is *Hidden*.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                log::warn!("settings: {}: {e}; the defaults", path.display());
                return Self::default();
            }
        };
        let file: SettingsFile = match toml::from_str(&text) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("settings: {}: {e}; the defaults", path.display());
                return Self::default();
            }
        };
        let scrollbar = [
            ScrollbarVisibility::Disabled,
            ScrollbarVisibility::Hidden,
            ScrollbarVisibility::Always,
        ]
        .into_iter()
        .find(|&m| scrollbar_name(m) == file.scrollbar)
        .unwrap_or_default();
        Settings {
            sound: file.sound,
            ding: file.ding,
            speak_coding: file.speak_coding,
            speak_gemini: file.speak_gemini,
            muted: file.muted,
            microphone: file.microphone,
            general_agent: file.general_agent,
            tab_follows_focus: file.tab_follows_focus,
            scrollbar,
            background: file.background,
            alerts: file.alerts,
            voice: file.voice.filter(|v| !v.is_empty()),
            appearance: Appearance::ALL
                .into_iter()
                .find(|a| a.name() == file.appearance)
                .unwrap_or_default(),
            light_theme: file.light_theme.filter(|t| !t.is_empty()),
            dark_theme: file.dark_theme.filter(|t| !t.is_empty()),
        }
    }

    /// Writes `settings.toml` in `dir`, through a temporary file renamed
    /// into place.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let file = SettingsFile {
            sound: self.sound,
            ding: self.ding,
            speak_coding: self.speak_coding,
            speak_gemini: self.speak_gemini,
            muted: self.muted,
            microphone: self.microphone,
            general_agent: self.general_agent,
            tab_follows_focus: self.tab_follows_focus,
            scrollbar: scrollbar_name(self.scrollbar).to_string(),
            background: self.background,
            alerts: self.alerts,
            voice: self.voice.clone(),
            appearance: self.appearance.name().to_string(),
            light_theme: self.light_theme.clone(),
            dark_theme: self.dark_theme.clone(),
        };
        let text = toml::to_string(&file).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(FILE);
        let tmp = dir.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, text)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

impl Settings {
    /// The saved theme of the kind `dark`, or the first of its kind when
    /// `themes` has no theme of that kind with its id.
    pub fn chosen<'a>(&self, themes: &'a Themes, dark: bool) -> &'a Theme {
        let id = if dark {
            &self.dark_theme
        } else {
            &self.light_theme
        };
        id.as_deref()
            .and_then(|id| themes.of_kind(dark).find(|t| t.id == id))
            .unwrap_or_else(|| themes.first(dark))
    }

    /// The theme in force with these settings and `themes`, by
    /// *Appearance* and the phone's dark mode ([`theme::system_dark_mode`]).
    pub fn theme<'a>(&self, themes: &'a Themes) -> &'a Theme {
        self.chosen(themes, self.appearance.is_dark(theme::system_dark_mode()))
    }

    /// The next theme of the kind `dark` that the right arrow (`forward`)
    /// or the left gives, wrapping, from the one shown.
    pub fn step_theme(&self, themes: &Themes, dark: bool, forward: bool) -> String {
        let list: Vec<&Theme> = themes.of_kind(dark).collect();
        let now = self.chosen(themes, dark);
        let i = list.iter().position(|t| t.id == now.id).unwrap_or(0);
        let n = list.len();
        list[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
        .id
        .clone()
    }
}

/// Puts `settings`' theme, from this thread's themes, in force, and gives
/// the system bars its colours. Returns it when it was not in force
/// already: then the screens are to be restyled.
pub fn put_theme_in_force(settings: &Settings) -> Option<Theme> {
    let themes = theme::themes();
    let t = settings.theme(&themes);
    if theme::current_id() == t.id && theme::theme() == t.colors {
        return None;
    }
    theme::set_theme(t);
    theme::apply_system_bars(theme::SystemBars::of(&t.colors));
    Some(t.clone())
}

thread_local! {
    /// The settings as the UI thread last set them.
    static CURRENT: RefCell<Settings> = RefCell::new(Settings::default());
    /// What the daemon's `[llm]` makes available, as last told (plan
    /// 20261027_02, decision 9); both until a daemon says otherwise.
    static LLM: RefCell<LlmInfo> = RefCell::new(LlmInfo::available());
}

/// What the daemon's `[llm]` makes available, as the UI thread last set it.
pub fn llm() -> LlmInfo {
    LLM.with(|l| l.borrow().clone())
}

/// Sets what [`llm`] gives on this thread.
pub fn set_llm(info: LlmInfo) {
    LLM.with(|l| *l.borrow_mut() = info);
}

/// Whether the general agent is there to use: the user's toggle is on and
/// the daemon offers it.
pub fn general_agent_on() -> bool {
    current().general_agent && llm().general_agent
}

/// The settings every control reads.
pub fn current() -> Settings {
    CURRENT.with(|c| c.borrow().clone())
}

/// Sets what [`current`] gives on this thread.
pub fn set_current(settings: Settings) {
    CURRENT.with(|c| *c.borrow_mut() = settings);
}

/// Whether the control `hit` is live with the settings as they are
/// (decision 4): volume with sound on; record, a card's reply, the dictation
/// controls and *discuss* (which takes a message) with the microphone on.
pub fn allows(hit: AgentHit) -> bool {
    let s = current();
    match hit {
        // Typing needs no microphone, only text input.
        AgentHit::PromptText(_) | AgentHit::PromptIme(_) | AgentHit::PromptAttach(_) => TEXT_INPUT,
        AgentHit::Volume | AgentHit::Replay(_) => s.sound,
        AgentHit::Message
        | AgentHit::Reply
        | AgentHit::Submit
        | AgentHit::Scratch
        | AgentHit::Clear
        | AgentHit::Suggest(flankr_companion::Suggestion::Discuss)
        | AgentHit::PromptMic(_)
        | AgentHit::Answer { opens: true, .. } => s.microphone,
        // Hidden while the daemon offers none (plan 20261027_02, decision 9).
        AgentHit::GeneralAgent => llm().general_agent,
        _ => true,
    }
}

/// As [`allows`], on the overview: there, record is also the general
/// agent's, and is disabled with it off (the 11:37 EDT amendment), or
/// while the daemon offers none (plan 20261027_02).
pub fn allows_on_overview(hit: AgentHit) -> bool {
    allows(hit) && (hit != AgentHit::Message || general_agent_on())
}

/// Whether the app can show the soft keyboard (libmorph text input plan
/// 20261024_06, step 1).
pub const TEXT_INPUT: bool = true;

/// Draws everything pushed to `list` since `start` at
/// [`DISABLED_ALPHA`] of its opacity.
pub fn dim_since(list: &mut DrawList, start: usize) {
    for instance in list.instances.iter_mut().skip(start) {
        instance.color[3] *= DISABLED_ALPHA;
    }
}

/// Moves `settings.toml` from `old` (the desktop's directory before
/// libmorph 0.4.0) into `dir`, once: only when `old` has one and `dir`
/// does not, so a file already in `dir` is never replaced. Logged either
/// way. Returns whether it moved.
pub fn move_old(old: &Path, dir: &Path) -> bool {
    let from = old.join(FILE);
    if !from.is_file() || old == dir {
        return false;
    }
    let to = dir.join(FILE);
    if to.exists() {
        log::info!(
            "settings: {} is also at the old place, {}; keeping {}",
            FILE,
            from.display(),
            to.display()
        );
        return false;
    }
    // A rename, or across file systems a copy and a delete.
    let moved = std::fs::create_dir_all(dir)
        .and_then(|()| {
            std::fs::rename(&from, &to).or_else(|_| {
                std::fs::copy(&from, &to)?;
                std::fs::remove_file(&from)
            })
        })
        .map_err(|e| e.to_string());
    match moved {
        Ok(()) => {
            log::info!("settings: moved {} to {}", from.display(), to.display());
            true
        }
        Err(e) => {
            log::warn!(
                "settings: moving {} to {}: {e}",
                from.display(),
                to.display()
            );
            false
        }
    }
}

/// *Delete data* on disk (decision 3): removes everything in `dir` but the
/// paths in `keep`, the voice models. Returns how many entries went.
pub fn delete_data(dir: &Path, keep: &[PathBuf]) -> Result<usize, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut removed = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if keep.iter().any(|k| k == &path) {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let gone = if kind.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        gone.map_err(|e| format!("{}: {e}", path.display()))?;
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_unreadable_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        std::fs::write(dir.path().join(FILE), "sound = 3\n").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        // Background on, alerts and tab follows focus off, by default.
        let d = Settings::default();
        assert!(d.background && !d.alerts && !d.tab_follows_focus);
    }

    #[test]
    fn a_file_without_the_speak_keys_loads_as_today() {
        // Read aloud was off at every start and Gemini always spoke (plan
        // 20261028_06, finding 1): there is nothing else to migrate.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "sound = true\nding = false\n").unwrap();
        let s = Settings::load(dir.path());
        assert!(!s.speak_coding && s.speak_gemini && !s.ding);
        let d = Settings::default();
        assert!(!d.speak_coding && d.speak_gemini);
        // Kept as left, and Gemini's sent to the client.
        let s = Settings {
            speak_coding: true,
            speak_gemini: false,
            ..d
        };
        s.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(text.contains("speak_coding = true"), "{text}");
        assert!(text.contains("speak_gemini = false"), "{text}");
        assert_eq!(Settings::load(dir.path()), s);
        assert!(!s.to_client().speak_gemini);
        assert!(Settings::default().to_client().speak_gemini);
    }

    #[test]
    fn muted_is_kept_and_tells_the_client_both_options_are_off() {
        // Off by default and in a file without it.
        let dir = tempfile::tempdir().unwrap();
        assert!(!Settings::default().muted);
        std::fs::write(dir.path().join(FILE), "speak_coding = true\n").unwrap();
        let s = Settings::load(dir.path());
        assert!(!s.muted && s.read_aloud() && s.to_client().speak_gemini);
        // Muted: both options keep their values, kept across restarts,
        // and the client hears neither.
        let m = Settings { muted: true, ..s };
        m.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(text.contains("muted = true"), "{text}");
        let m = Settings::load(dir.path());
        assert!(m.muted && m.speak_coding && m.speak_gemini);
        assert!(!m.read_aloud());
        assert!(!m.to_client().speak_gemini);
    }

    #[test]
    fn tab_follows_focus_reaches_the_client() {
        assert!(!Settings::default().to_client().tab_follows_focus);
        let s = Settings {
            tab_follows_focus: true,
            ..Settings::default()
        };
        let client = s.to_client();
        assert!(client.tab_follows_focus && client.general_agent);
    }

    #[test]
    fn settings_toml_round_trips_and_fills_what_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            sound: false,
            ding: true,
            speak_coding: true,
            speak_gemini: false,
            muted: true,
            microphone: false,
            general_agent: false,
            tab_follows_focus: true,
            scrollbar: ScrollbarVisibility::Always,
            background: false,
            alerts: true,
            voice: Some("alba".into()),
            appearance: Appearance::Light,
            light_theme: Some("solarized_light".into()),
            dark_theme: Some("nord".into()),
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(text.contains("scrollbar = \"always\""), "{text}");
        assert!(!dir.path().join(format!("{FILE}.tmp")).exists());
        assert!(text.contains("background = false") && text.contains("alerts = true"));
        assert!(text.contains("tab_follows_focus = true"), "{text}");
        assert!(text.contains("voice = \"alba\""), "{text}");
        assert!(text.contains("appearance = \"light\""), "{text}");
        assert!(text.contains("light_theme = \"solarized_light\""), "{text}");
        assert!(text.contains("dark_theme = \"nord\""), "{text}");
        // *Default* is left out of the file.
        Settings::default().save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(!text.contains("voice"), "{text}");
        assert!(
            !text.contains("_theme"),
            "the first of each kind is left out: {text}"
        );
        assert!(text.contains("appearance = \"system\""), "{text}");

        std::fs::write(
            dir.path().join(FILE),
            "ding = false\nscrollbar = \"sideways\"\n",
        )
        .unwrap();
        assert_eq!(
            Settings::load(dir.path()),
            Settings {
                ding: false,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn the_old_desktop_file_moves_once_and_never_over_a_new_one() {
        let root = tempfile::tempdir().unwrap();
        let (old, dir) = (root.path().join("old"), root.path().join("new"));
        assert!(!move_old(&old, &dir), "nothing to move");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join(FILE), "sound = false\n").unwrap();
        // The new directory need not exist yet.
        assert!(move_old(&old, &dir));
        assert!(!old.join(FILE).exists(), "moved, not copied");
        assert!(!Settings::load(&dir).sound);
        assert!(!move_old(&old, &dir), "once");
        // A file in both places: the new one is kept, the old one left.
        std::fs::write(old.join(FILE), "ding = false\n").unwrap();
        assert!(!move_old(&old, &dir));
        assert!(old.join(FILE).exists());
        let kept = Settings::load(&dir);
        assert!(!kept.sound && kept.ding);
        // The same directory is not a move.
        assert!(!move_old(&dir, &dir));
        assert!(dir.join(FILE).exists());
    }

    #[test]
    fn the_arrows_step_through_the_visibilities_and_wrap() {
        use ScrollbarVisibility::*;
        let mut m = Hidden;
        let mut seen = Vec::new();
        for _ in 0..3 {
            m = step_scrollbar(m, true);
            seen.push(m);
        }
        assert_eq!(seen, [Always, Disabled, Hidden]);
        for _ in 0..3 {
            m = step_scrollbar(m, false);
            seen.push(m);
        }
        assert_eq!(&seen[3..], [Disabled, Always, Hidden]);
    }

    #[test]
    fn appearance_steps_both_ways_and_wraps_and_picks_the_kind() {
        let mut a = Appearance::default();
        assert_eq!(a, Appearance::System);
        let mut seen = Vec::new();
        for _ in 0..3 {
            a = a.step(true);
            seen.push(a);
        }
        use Appearance::*;
        assert_eq!(seen, [Light, Dark, System]);
        for _ in 0..3 {
            a = a.step(false);
            seen.push(a);
        }
        assert_eq!(&seen[3..], [Dark, Light, System]);
        // Until libmorph says, System is dark.
        assert!(System.is_dark(None) && System.is_dark(Some(true)));
        assert!(!System.is_dark(Some(false)));
        assert!(!Light.is_dark(Some(true)) && Dark.is_dark(Some(false)));
        assert!(System.row_active(true) && System.row_active(false));
        assert!(Light.row_active(false) && !Light.row_active(true));
        assert!(Dark.row_active(true) && !Dark.row_active(false));
        // An unknown appearance in the file is System.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "appearance = \"sepia\"\n").unwrap();
        assert_eq!(Settings::load(dir.path()).appearance, System);
    }

    #[test]
    fn the_theme_rows_step_through_their_kind_and_wrap() {
        let themes = Themes::shipped();
        for dark in [false, true] {
            let kind: Vec<&str> = themes.of_kind(dark).map(|t| t.id.as_str()).collect();
            let mut s = Settings::default();
            assert_eq!(s.chosen(&themes, dark).id, kind[0], "the first by default");
            let mut seen = Vec::new();
            for _ in 0..kind.len() {
                let id = s.step_theme(&themes, dark, true);
                if dark {
                    s.dark_theme = Some(id.clone());
                } else {
                    s.light_theme = Some(id.clone());
                }
                seen.push(id);
            }
            let mut forward: Vec<&str> = kind[1..].to_vec();
            forward.push(kind[0]);
            assert_eq!(seen, forward, "dark {dark}: wraps forward");
            assert_eq!(
                s.step_theme(&themes, dark, false),
                *kind.last().unwrap(),
                "and back"
            );
        }
    }

    #[test]
    fn a_missing_theme_id_falls_back_to_the_first_of_its_kind() {
        let themes = Themes::shipped();
        let s = Settings {
            light_theme: Some("gone".into()),
            dark_theme: Some("light_paper".into()),
            ..Settings::default()
        };
        assert_eq!(s.chosen(&themes, false).id, "light_black");
        assert_eq!(
            s.chosen(&themes, true).id,
            "dark_navy",
            "a light id is not a dark theme"
        );
        let s = Settings {
            appearance: Appearance::Light,
            light_theme: Some("solarized_light".into()),
            dark_theme: Some("nord".into()),
            ..Settings::default()
        };
        assert_eq!(s.theme(&themes).id, "solarized_light");
        let s = Settings {
            appearance: Appearance::System,
            ..s
        };
        assert_eq!(s.theme(&themes).id, "nord", "System is dark for now");
    }

    #[test]
    fn each_toggle_gates_its_controls() {
        set_current(Settings::default());
        assert!(allows(AgentHit::Volume) && allows(AgentHit::Message));
        set_current(Settings {
            sound: false,
            ..Settings::default()
        });
        assert!(!allows(AgentHit::Volume));
        assert!(allows(AgentHit::Message));
        set_current(Settings {
            microphone: false,
            ..Settings::default()
        });
        for hit in [
            AgentHit::Message,
            AgentHit::Reply,
            AgentHit::Submit,
            AgentHit::Clear,
            AgentHit::Answer {
                index: 1,
                text: true,
                opens: true,
            },
        ] {
            assert!(!allows(hit), "{hit:?}");
        }
        assert!(allows(AgentHit::Approve(1)), "Yes and No still work");
        assert!(
            allows(AgentHit::Answer {
                index: 0,
                text: true,
                opens: false
            }),
            "an answer that sends still works"
        );
        set_current(Settings {
            general_agent: false,
            ..Settings::default()
        });
        assert!(allows(AgentHit::Message), "in the agent view");
        assert!(!allows_on_overview(AgentHit::Message));
        assert!(allows_on_overview(AgentHit::Volume));
        set_current(Settings::default());
    }

    #[test]
    fn the_daemons_llm_gates_the_general_agent() {
        set_current(Settings::default());
        assert!(general_agent_on() && allows(AgentHit::GeneralAgent));
        set_llm(LlmInfo {
            general_agent: false,
            summaries: true,
            reason: Some("off".into()),
        });
        assert!(!general_agent_on(), "though the toggle is on");
        assert!(current().general_agent);
        assert!(!allows(AgentHit::GeneralAgent));
        assert!(allows(AgentHit::Message), "a message in the agent view");
        assert!(!allows_on_overview(AgentHit::Message));
        set_llm(LlmInfo::available());
        assert!(general_agent_on() && allows_on_overview(AgentHit::Message));
    }

    #[test]
    fn delete_data_keeps_only_the_voice_models() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for model in ["stt-1b-en_fr", "phonon-31m-en"] {
            std::fs::create_dir_all(d.join(model).join("voices")).unwrap();
            std::fs::write(d.join(model).join("model.q8.gguf"), "weights").unwrap();
        }
        std::fs::write(d.join(FILE), "sound = false\n").unwrap();
        std::fs::write(d.join("vocab.txt"), "claude\n").unwrap();
        std::fs::create_dir_all(d.join("store")).unwrap();
        std::fs::write(d.join("store").join("scrollback.redb"), "x").unwrap();
        let keep = vec![d.join("stt-1b-en_fr"), d.join("phonon-31m-en")];
        assert_eq!(delete_data(d, &keep).unwrap(), 3);
        let mut left: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["phonon-31m-en", "stt-1b-en_fr"]);
        assert!(d.join("stt-1b-en_fr").join("model.q8.gguf").exists());
    }
}
