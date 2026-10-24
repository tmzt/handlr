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

/// `c` at opacity `a`.
pub fn with_alpha(c: [f32; 4], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
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

/// Whether the phone is in dark mode, or `None` where that is not known.
///
/// TODO(libmorph): libmorph has no dark-mode API yet; handlr asked for
/// `ctx.dark_mode()` and a change event (plan 20261024_05's 15:33 EDT
/// amendment, item 5). Until then this is `None`, and *System* acts as
/// dark; with the event, the app applies the theme again as it changes.
pub fn system_dark_mode() -> Option<bool> {
    None
}

/// Android's status and navigation bars in a theme (CHANGES.md
/// 2026-10-24 15:36 EDT, plan 20261024_05 decision 6): the status bar in
/// the theme's `header`, the navigation bar in its `bg`, each with dark
/// icons on a light colour and light icons on a dark one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemBars {
    pub status: [f32; 4],
    pub status_dark_icons: bool,
    pub navigation: [f32; 4],
    pub navigation_dark_icons: bool,
}

/// WCAG 2's relative luminance of `c`'s sRGB colour, from 0 (black) to 1
/// (white).
pub fn luminance(c: [f32; 4]) -> f32 {
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
}

/// Whether dark icons contrast more with `c` than light ones: above
/// about 0.179, black has the higher contrast ratio against it than white.
pub fn wants_dark_icons(c: [f32; 4]) -> bool {
    // (L + 0.05) / 0.05 against black, 1.05 / (L + 0.05) against white.
    (luminance(c) + 0.05) / 0.05 > 1.05 / (luminance(c) + 0.05)
}

impl SystemBars {
    /// The bars for `colors`.
    pub fn of(colors: &Colors) -> Self {
        SystemBars {
            status: colors.header,
            status_dark_icons: wants_dark_icons(colors.header),
            navigation: colors.bg,
            navigation_dark_icons: wants_dark_icons(colors.bg),
        }
    }
}

/// Gives the system bars their colours.
///
/// TODO(libmorph): libmorph has no call for the system bars' colours and
/// icons yet; handlr asked for one with its dark-mode request
/// (`ctx.dark_mode()` and a change event, plan 20261024_05's 15:33 EDT
/// amendment, item 5). Until then this does nothing.
pub fn apply_system_bars(bars: SystemBars) {
    log::debug!("theme: system bars {bars:?} (no libmorph call yet)");
}

/// What the screens' theme tests share.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use libcontrols::DrawContext;
    use libmsdf::drawlist::DrawList;
    use libmsdf::font::{FontAtlas, ROBOTO_REGULAR_ASCII, ROBOTO_REGULAR_ATLAS, TextShaper};

    /// A draw context with Roboto, so text is drawn as glyphs in its
    /// colours.
    pub fn draw_context() -> DrawContext<'static> {
        let atlas: &'static FontAtlas = Box::leak(Box::new(
            FontAtlas::from_bytes(ROBOTO_REGULAR_ATLAS).unwrap(),
        ));
        let shaper: &'static TextShaper = Box::leak(Box::new(
            TextShaper::new(ROBOTO_REGULAR_ASCII.to_vec()).unwrap(),
        ));
        DrawContext::new(Some(atlas), Some(shaper))
    }

    /// Puts the shipped theme `id` in force.
    pub fn use_theme(id: &str) {
        set_theme(Themes::shipped().find(id).expect("shipped"));
    }

    /// The colours drawn in `list` whose red, green and blue are one of
    /// *Dark (gray)*'s and none of *Light (black text)*'s, at any opacity.
    pub fn dark_gray_only(list: &DrawList) -> Vec<[f32; 4]> {
        let themes = Themes::shipped();
        let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
        let of = |id: &str| -> Vec<[f32; 3]> {
            let c = themes.find(id).unwrap().colors;
            Role::ALL.iter().map(|&r| rgb(c.get(r))).collect()
        };
        let (gray, light) = (of("dark_gray"), of("light_black"));
        let mut out: Vec<[f32; 4]> = list
            .instances
            .iter()
            .map(|i| i.color)
            .filter(|&c| gray.contains(&rgb(c)) && !light.contains(&rgb(c)))
            .collect();
        out.dedup();
        out
    }
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
        // Every role holds one of them, but the session colours, which are
        // new (plan 20261022_03, the L3 amendment of 2026-10-24 15:56 EDT).
        for &role in Role::ALL
            .iter()
            .filter(|r| !check::SESSION_ROLES.contains(r))
        {
            assert!(OLD.iter().any(|o| o.2 == role), "{role:?}");
        }
        assert_eq!(theme(), gray.colors, "in force at start");
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
    fn the_system_bars_take_header_and_bg_with_contrasting_icons() {
        let themes = Themes::shipped();
        let light = SystemBars::of(&themes.find("light_black").unwrap().colors);
        assert_eq!(light.navigation, [1.0, 1.0, 1.0, 1.0], "bg");
        assert!(light.status_dark_icons && light.navigation_dark_icons);
        let gray = themes.find("dark_gray").unwrap().colors;
        let dark = SystemBars::of(&gray);
        assert_eq!((dark.status, dark.navigation), (gray.header, gray.bg));
        assert!(!dark.status_dark_icons && !dark.navigation_dark_icons);
        // Every shipped theme's bars contrast with its kind.
        for t in &themes.list {
            let bars = SystemBars::of(&t.colors);
            assert_eq!(bars.navigation_dark_icons, !t.dark, "{}", t.name);
            assert_eq!(bars.status_dark_icons, !t.dark, "{}", t.name);
        }
        assert!(wants_dark_icons([0.5, 0.5, 0.5, 1.0]) && !wants_dark_icons([0.4, 0.4, 0.4, 1.0]));
    }

    #[test]
    fn session_colours_are_inherited_unless_a_theme_sets_its_own() {
        let themes = Themes::shipped();
        let gray = themes.find("dark_gray").unwrap().colors;
        let light = themes.find("light_black").unwrap().colors;
        // Both defaults set all 16, each its own.
        for i in 0..16 {
            assert_ne!(gray.session(i), light.session(i), "session_{}", i + 1);
        }
        // Themes without their own take their kind's default's.
        for (id, default) in [
            ("dark_black", gray),
            ("dark_navy", gray),
            ("high_contrast", gray),
            ("light_paper", light),
        ] {
            let c = themes.find(id).unwrap().colors;
            for i in 0..16 {
                assert_eq!(c.session(i), default.session(i), "{id} session_{}", i + 1);
            }
        }
        // The palette themes set their own.
        for id in [
            "solarized_dark",
            "solarized_light",
            "gruvbox_dark",
            "nord",
            "dracula",
            "tokyo_night",
        ] {
            let t = themes.find(id).unwrap();
            let default = if t.dark { gray } else { light };
            assert!(
                (0..16).all(|i| t.colors.session(i) != default.session(i)),
                "{id}"
            );
        }
        // A user's theme that leaves them out inherits them.
        let mut mine = themes.clone();
        mine.add(
            "[theme.mine]\nname = \"Mine\"\ndark = false\nsession_3 = \"#123456\"\n",
            "test",
        );
        let m = mine.find("mine").unwrap().colors;
        assert_eq!(
            m.session(2),
            [
                0x12 as f32 / 255.0,
                0x34 as f32 / 255.0,
                0x56 as f32 / 255.0,
                1.0
            ]
        );
        assert_eq!(m.session(0), light.session(0));
    }

    #[test]
    fn every_session_colour_shows_on_its_themes_background_and_cards() {
        let contrast = |a: [f32; 4], b: [f32; 4]| {
            let (x, y) = (luminance(a), luminance(b));
            (x.max(y) + 0.05) / (x.min(y) + 0.05)
        };
        for t in &Themes::shipped().list {
            for i in 0..16 {
                let c = t.colors.session(i);
                for (what, under) in [("bg", t.colors.bg), ("card", t.colors.card)] {
                    let r = contrast(c, under);
                    assert!(r >= 1.5, "{} session_{} on {what}: {r:.2}", t.name, i + 1);
                }
            }
        }
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
