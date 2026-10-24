//! The settings (docs/design/20261024_02_SETTINGS_SCREEN_PLAN.md): what
//! they are, where they are kept, and what each one turns off.
//!
//! - [`Settings`] is the screen's toggles and the scrollbar's visibility
//!   (the 11:32 EDT amendment), kept in `settings.toml` in the app's data
//!   directory (decision 5): written on every change, read at start, the
//!   defaults when missing or unreadable. On the desktop that is libmorph's
//!   data directory since 0.4.0; [`move_old`] brings the file over once
//!   from `~/.config/handlr-companion`.
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
pub mod screen;
pub mod toggle_row;
pub mod voice;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use libmsdf::drawlist::DrawList;
use serde::{Deserialize, Serialize};

use crate::agent::{AgentHit, ScrollbarVisibility};

pub use screen::{SettingsHit, SettingsScreen};

/// The file the settings are kept in, in the data directory.
pub const FILE: &str = "settings.toml";

/// How opaque a disabled control is drawn (decision 4).
pub const DISABLED_ALPHA: f32 = 0.35;

/// The settings screen's values. Each toggle is on by default but
/// *Alert when an agent needs you*, the scrollbar *Hidden*, and the voice
/// *Default*.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// *All sound*.
    pub sound: bool,
    /// *Ding when an agent is done*; only while `sound` is on.
    pub ding: bool,
    /// *Microphone*.
    pub microphone: bool,
    /// *General agent (Gemini)*.
    pub general_agent: bool,
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound: true,
            ding: true,
            microphone: true,
            general_agent: true,
            scrollbar: ScrollbarVisibility::Hidden,
            background: true,
            alerts: false,
            voice: None,
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
    #[serde(default = "on")]
    microphone: bool,
    #[serde(default = "on")]
    general_agent: bool,
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
    pub fn to_client(&self) -> handlr_companion::Settings {
        handlr_companion::Settings {
            sound: self.sound,
            ding: self.ding,
            microphone: self.microphone,
            general_agent: self.general_agent,
        }
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
            microphone: file.microphone,
            general_agent: file.general_agent,
            scrollbar,
            background: file.background,
            alerts: file.alerts,
            voice: file.voice.filter(|v| !v.is_empty()),
        }
    }

    /// Writes `settings.toml` in `dir`, through a temporary file renamed
    /// into place.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let file = SettingsFile {
            sound: self.sound,
            ding: self.ding,
            microphone: self.microphone,
            general_agent: self.general_agent,
            scrollbar: scrollbar_name(self.scrollbar).to_string(),
            background: self.background,
            alerts: self.alerts,
            voice: self.voice.clone(),
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

thread_local! {
    /// The settings as the UI thread last set them.
    static CURRENT: RefCell<Settings> = RefCell::new(Settings::default());
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
/// (decision 4): play with sound on; record, a card's reply, the dictation
/// controls and *discuss* (which takes a message) with the microphone on.
pub fn allows(hit: AgentHit) -> bool {
    let s = current();
    match hit {
        AgentHit::ReadAloud | AgentHit::Replay(_) => s.sound,
        AgentHit::Message
        | AgentHit::Reply
        | AgentHit::Submit
        | AgentHit::Scratch
        | AgentHit::Clear
        | AgentHit::Suggest(handlr_companion::Suggestion::Discuss) => s.microphone,
        _ => true,
    }
}

/// As [`allows`], on the overview: there, record is also the general
/// agent's, and is disabled with it off (the 11:37 EDT amendment).
pub fn allows_on_overview(hit: AgentHit) -> bool {
    allows(hit) && (hit != AgentHit::Message || current().general_agent)
}

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
        // Background on, alerts off, by default.
        let d = Settings::default();
        assert!(d.background && !d.alerts);
    }

    #[test]
    fn settings_toml_round_trips_and_fills_what_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            sound: false,
            ding: true,
            microphone: false,
            general_agent: false,
            scrollbar: ScrollbarVisibility::Always,
            background: false,
            alerts: true,
            voice: Some("alba".into()),
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(text.contains("scrollbar = \"always\""), "{text}");
        assert!(!dir.path().join(format!("{FILE}.tmp")).exists());
        assert!(text.contains("background = false") && text.contains("alerts = true"));
        assert!(text.contains("voice = \"alba\""), "{text}");
        // *Default* is left out of the file.
        Settings::default().save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(!text.contains("voice"), "{text}");

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
    fn each_toggle_gates_its_controls() {
        set_current(Settings::default());
        assert!(allows(AgentHit::ReadAloud) && allows(AgentHit::Message));
        set_current(Settings {
            sound: false,
            ..Settings::default()
        });
        assert!(!allows(AgentHit::ReadAloud));
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
        ] {
            assert!(!allows(hit), "{hit:?}");
        }
        assert!(allows(AgentHit::Approve(1)), "Yes and No still work");
        set_current(Settings {
            general_agent: false,
            ..Settings::default()
        });
        assert!(allows(AgentHit::Message), "in the agent view");
        assert!(!allows_on_overview(AgentHit::Message));
        assert!(allows_on_overview(AgentHit::ReadAloud));
        set_current(Settings::default());
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
