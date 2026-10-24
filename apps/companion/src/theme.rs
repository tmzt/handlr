//! Themes (docs/design/20261024_05_THEMES_PLAN.md and its 15:33 EDT
//! amendment): named sets of colour roles.
//!
//! - The shipped `themes.toml` is compiled in ([`SHIPPED`]), checked by
//!   `build.rs`, and parsed at start, so a valid set is always there.
//! - `themes.toml` in the data directory, beside `settings.toml`, is read
//!   at start and each time Settings opens ([`Themes::load`]): a theme
//!   with a new id is added, one with a shipped id replaces it, and an
//!   invalid one is skipped and logged. Roles a theme leaves out are the
//!   shipped default's of its kind (*Dark (gray)* or *Light (black
//!   text)*).
//! - The UI thread's themes and the one in force are thread-locals, as the
//!   settings are: [`theme`] gives its colours.

use std::cell::{Cell, RefCell};
use std::path::Path;

#[path = "../build/themes_check.rs"]
mod check;

pub use check::{Colors, ROLES, Role, Theme};

/// The shipped themes.
pub const SHIPPED: &str = include_str!("../themes.toml");

/// The user's themes' file, in the data directory.
pub const FILE: &str = "themes.toml";

/// The themes there are, in file order: the shipped ones, then the user's
/// new ones.
#[derive(Clone, Debug, PartialEq)]
pub struct Themes {
    pub list: Vec<Theme>,
    /// The shipped defaults, for the user's themes' missing roles.
    dark_default: Colors,
    light_default: Colors,
}

impl Themes {
    /// The shipped themes alone.
    pub fn shipped() -> Self {
        let (themes, _) = check::parse(SHIPPED).expect("build.rs checked themes.toml");
        let (dark, light) = check::defaults(&themes).expect("build.rs checked themes.toml");
        let list = check::check(SHIPPED).expect("build.rs checked themes.toml");
        Themes {
            list,
            dark_default: dark.colors,
            light_default: light.colors,
        }
    }

    /// The shipped themes and those in `<dir>/themes.toml`, if it exists.
    pub fn load(dir: Option<&Path>) -> Self {
        let mut themes = Self::shipped();
        let Some(dir) = dir else {
            return themes;
        };
        let path = dir.join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => themes.add(&text, &path.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("themes: {}: {e}; the shipped themes", path.display()),
        }
        themes
    }

    /// Adds the themes in `text` (from `source`, for the log): a new id is
    /// added, a known one replaced, an invalid theme skipped and logged.
    pub fn add(&mut self, text: &str, source: &str) {
        let (partials, errors) = match check::parse(text) {
            Ok(parsed) => parsed,
            Err(e) => {
                log::warn!("themes: {source}: {e}; none of its themes");
                return;
            }
        };
        for e in errors {
            log::warn!("themes: {source}: {e}; skipped");
        }
        for p in partials {
            let default = if p.dark {
                &self.dark_default
            } else {
                &self.light_default
            };
            let theme = p.complete(default);
            if self
                .list
                .iter()
                .any(|t| t.name == theme.name && t.id != theme.id)
            {
                log::warn!(
                    "themes: {source}: theme.{}: the name {:?} is taken; skipped",
                    theme.id,
                    theme.name
                );
                continue;
            }
            match self.list.iter_mut().find(|t| t.id == theme.id) {
                Some(t) => *t = theme,
                None => self.list.push(theme),
            }
        }
    }

    /// The theme `id`.
    pub fn find(&self, id: &str) -> Option<&Theme> {
        self.list.iter().find(|t| t.id == id)
    }

    /// The dark themes, or the light ones, in file order.
    pub fn of_kind(&self, dark: bool) -> impl Iterator<Item = &Theme> {
        self.list.iter().filter(move |t| t.dark == dark)
    }

    /// The first theme of a kind: its default.
    pub fn first(&self, dark: bool) -> &Theme {
        self.of_kind(dark)
            .next()
            .expect("the shipped themes have both kinds")
    }
}

thread_local! {
    /// The themes as the UI thread last loaded them.
    static THEMES: RefCell<Themes> = RefCell::new(Themes::shipped());
    /// The theme in force: its id and colours.
    static CURRENT: RefCell<String> = RefCell::new(THEMES.with(|t| t.borrow().first(true).id.clone()));
    static COLORS: Cell<Colors> = Cell::new(THEMES.with(|t| t.borrow().first(true).colors));
}

/// The colours of the theme in force on this thread (at first, *Dark
/// (gray)*).
pub fn theme() -> Colors {
    COLORS.with(Cell::get)
}

/// The themes this thread has.
pub fn themes() -> Themes {
    THEMES.with(|t| t.borrow().clone())
}

/// Sets what [`themes`] gives on this thread. The theme in force keeps its
/// colours until [`set_theme`].
pub fn set_themes(themes: Themes) {
    THEMES.with(|t| *t.borrow_mut() = themes);
}

/// The id of the theme in force.
pub fn current_id() -> String {
    CURRENT.with(|c| c.borrow().clone())
}

/// Puts `theme` in force on this thread.
pub fn set_theme(theme: &Theme) {
    CURRENT.with(|c| *c.borrow_mut() = theme.id.clone());
    COLORS.with(|c| c.set(theme.colors));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each colour constant the app had before themes, by its file and
    /// name, as it was written, and the role that now holds it.
    const OLD: &[(&str, [f32; 4], Role)] = &[
        ("agent::BG", [0.03, 0.03, 0.04, 1.0], Role::Bg),
        ("agent::FG", [0.94, 0.95, 0.96, 1.0], Role::Fg),
        ("agent::DIM", [0.50, 0.53, 0.58, 1.0], Role::Dim),
        ("agent::GREEN", [0.36, 0.93, 0.45, 1.0], Role::Green),
        ("agent::RED", [1.0, 0.40, 0.40, 1.0], Role::Red),
        ("agent::YELLOW", [1.0, 0.82, 0.30, 1.0], Role::Yellow),
        ("agent::BAND", [0.18, 0.19, 0.21, 1.0], Role::Band),
        ("agent::CARD_BG", [0.11, 0.12, 0.17, 1.0], Role::Card),
        ("agent::SHADOW", [0.0, 0.0, 0.0, 0.55], Role::Shadow),
        ("agent::HEADER_BG", [0.07, 0.07, 0.09, 1.0], Role::Header),
        ("agent::APPROVE_BG", [0.16, 0.45, 0.25, 1.0], Role::Approve),
        ("agent::DENY_BG", [0.55, 0.18, 0.18, 1.0], Role::Deny),
        ("agent::MESSAGE_BG", [0.20, 0.32, 0.55, 1.0], Role::Message),
        ("agent::BACK_BG", [0.30, 0.30, 0.36, 1.0], Role::Back),
        ("agent::DICTATION_BG", [0.09, 0.10, 0.13, 1.0], Role::Input),
        (
            "agent::accent(Approval)",
            [1.0, 0.74, 0.25, 1.0],
            Role::Accent,
        ),
        (
            "agent::accent(Question)",
            [0.42, 0.72, 1.0, 1.0],
            Role::Question,
        ),
        ("agent::RECORDING", [1.0, 0.35, 0.35, 1.0], Role::Recording),
        (
            "agent::PLACEHOLDER",
            [0.94, 0.95, 0.96, 0.06],
            Role::Placeholder,
        ),
        (
            "agent::SHIMMER",
            [0.94, 0.95, 0.96, 0.04 / 3.0],
            Role::Shimmer,
        ),
        ("ui::TEXT", [0.92, 0.93, 0.95, 1.0], Role::Text),
        ("ui::MUTED", [0.60, 0.62, 0.68, 1.0], Role::Muted),
        ("ui::SPOKEN", [0.55, 0.78, 1.0, 1.0], Role::Spoken),
        ("ui::WARN", [1.0, 0.62, 0.45, 1.0], Role::Warn),
        ("ui::CODE", [1.0, 0.86, 0.45, 1.0], Role::Code),
        ("ui::PAIR_BG", [0.20, 0.32, 0.55, 1.0], Role::Message),
        (
            "settings::screen::HEADER_BG",
            [0.07, 0.07, 0.09, 1.0],
            Role::Header,
        ),
        (
            "settings::link_row::BUTTON_BG",
            [0.24, 0.25, 0.31, 1.0],
            Role::Button,
        ),
        (
            "settings::toggle_row::TRACK_ON",
            [0.22, 0.62, 0.36, 1.0],
            Role::Toggle,
        ),
        (
            "agent::link_spans::LINK",
            [0.45, 0.78, 1.0, 1.0],
            Role::Link,
        ),
        (
            "overview::turn_overlay::SHEET_BG",
            [0.07, 0.07, 0.09, 1.0],
            Role::Header,
        ),
        (
            "overview::turn_overlay::SHADE",
            [0.0, 0.0, 0.0, 0.55],
            Role::Overlay,
        ),
        (
            "libcontrols ScrollTheme::indicator_color",
            [0.55, 0.62, 0.70, 0.5],
            Role::Scrollbar,
        ),
    ];

    #[test]
    fn dark_gray_is_the_colours_before_themes_exactly() {
        let themes = Themes::shipped();
        let gray = themes.find("dark_gray").expect("shipped");
        assert_eq!(gray.name, "Dark (gray)");
        assert!(gray.dark);
        assert_eq!(themes.first(true).id, "dark_gray", "the dark default");
        assert_eq!(themes.first(false).id, "light_black", "the light default");
        for &(name, value, role) in OLD {
            assert_eq!(gray.colors.get(role), value, "{name} as {role:?}");
        }
        // Every role holds one of them.
        for &role in Role::ALL {
            assert!(OLD.iter().any(|o| o.2 == role), "{role:?}");
        }
        assert_eq!(theme(), gray.colors, "in force at start");
    }

    #[test]
    fn the_constants_are_dark_grays_roles() {
        let c = Themes::shipped().find("dark_gray").unwrap().colors;
        use crate::agent;
        assert_eq!(agent::BG, c.bg);
        assert_eq!(agent::FG, c.fg);
        assert_eq!(agent::DIM, c.dim);
        assert_eq!(agent::CARD_BG, c.card);
        assert_eq!(agent::APPROVE_BG, c.approve);
        assert_eq!(agent::DENY_BG, c.deny);
        assert_eq!(agent::MESSAGE_BG, c.message);
        assert_eq!(agent::BACK_BG, c.back);
        assert_eq!(agent::RECORDING, c.recording);
        assert_eq!(
            agent::accent(handlr_companion::CardKind::Approval),
            c.accent
        );
        assert_eq!(
            agent::accent(handlr_companion::CardKind::Question),
            c.question
        );
        assert_eq!(crate::ui::TEXT, c.text);
        assert_eq!(crate::ui::MUTED, c.muted);
        assert_eq!(crate::ui::SPOKEN, c.spoken);
        assert_eq!(crate::ui::WARN, c.warn);
        assert_eq!(crate::ui::CODE, c.code);
        assert_eq!(agent::link_spans::LINK, c.link);
    }

    #[test]
    fn the_shipped_themes_are_in_file_order_with_their_sources_named() {
        let themes = Themes::shipped();
        let names: Vec<&str> = themes.list.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Dark (gray)",
                "Dark (black)",
                "Dark (navy)",
                "Light (black text)",
                "Light (paper)",
                "Solarized Dark",
                "Solarized Light",
                "Gruvbox Dark",
                "Nord",
                "Dracula",
                "Tokyo Night",
                "High contrast",
            ]
        );
        for source in [
            "Ethan Schoonover",
            "morhetz",
            "Arctic Ice Studio",
            "draculatheme.com",
            "enkia",
        ] {
            assert!(SHIPPED.contains(source), "{source}");
        }
    }

    #[test]
    fn a_user_file_adds_replaces_and_skips() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE),
            "[theme.mine]\nname = \"Mine\"\ndark = false\nbg = \"#FFEEDD\"\n\
             [theme.nord]\nname = \"Nord (mine)\"\ndark = true\nbg = \"#101010\"\n\
             [theme.broken]\nname = \"Broken\"\ndark = true\ntext = \"blue\"\n\
             [theme.undecided]\nname = \"Undecided\"\nbg = \"#000000\"\n",
        )
        .unwrap();
        let shipped = Themes::shipped();
        let themes = Themes::load(Some(dir.path()));
        assert_eq!(themes.list.len(), shipped.list.len() + 1, "one added");
        let mine = themes.find("mine").expect("added");
        assert_eq!(themes.list.last().unwrap().id, "mine", "after the shipped");
        assert_eq!(mine.colors.bg, [1.0, 238.0 / 255.0, 221.0 / 255.0, 1.0]);
        let light = shipped.find("light_black").unwrap().colors;
        assert_eq!(mine.colors.text, light.text, "from the light default");
        let nord = themes.find("nord").unwrap();
        assert_eq!(nord.name, "Nord (mine)", "replaced");
        assert_eq!(
            nord.colors.bg,
            [16.0 / 255.0, 16.0 / 255.0, 16.0 / 255.0, 1.0]
        );
        let gray = shipped.find("dark_gray").unwrap().colors;
        assert_eq!(nord.colors.fg, gray.fg, "from the dark default, not Nord's");
        assert!(themes.find("broken").is_none() && themes.find("undecided").is_none());
        // No file, or a file that is not TOML: the shipped themes.
        assert_eq!(Themes::load(None), shipped);
        std::fs::write(dir.path().join(FILE), "= nonsense").unwrap();
        assert_eq!(Themes::load(Some(dir.path())), shipped);
    }

    #[test]
    fn set_theme_puts_it_in_force() {
        let themes = Themes::shipped();
        let light = themes.find("light_black").unwrap();
        set_theme(light);
        assert_eq!(theme(), light.colors);
        assert_eq!(current_id(), "light_black");
        set_theme(themes.first(true));
        assert_eq!(current_id(), "dark_gray");
    }
}
