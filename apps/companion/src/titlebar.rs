//! The title-bar icons (docs/design/20261028_04_PLUGINS_PLAN.md, decision
//! 3.6 and its 2026-10-28 22:03 EDT amendment).
//!
//! The app and the plugins register icons for a screen ([`registered`]),
//! left to right: the **agent menu** (the subagents plan's ⊙, on the agent
//! view while the pane has subagents), the plugins' icons in the
//! registry's order (the planning plugin's `plans` is ☑, U+2611, from the
//! terminal face), then **Settings** (the gear, on the overview and the
//! agent view). The ⋮ ([`Kebab`]) is always last, at the right. Each icon
//! is a 42 pt square target in the title's text colour. The drawer's ≡ is
//! not one of them and is not counted.
//!
//! At most [`MAX_ICONS`] stand before the ⋮ ([`overflow`]): past four,
//! Settings leaves the bar for the ⋮ menu's last item; past four still,
//! the last plugin icons, in reverse registry order, follow it, above
//! Settings, under their labels.
//!
//! A plugin cannot send a glyph: its icon names one of the app's
//! ([`glyph_of`]), and a name the app does not know is drawn as the
//! label's first letter in a ring.

use libcontrols::{DrawContext, Rect};
use libmsdf::drawlist::DrawList;

use crate::agent::{Row, Terminal, draw_row, fill, term_h};
use crate::menu::Kebab;
use crate::overview::Gear;
use crate::plugins::{MenuEntry, Screen};
use crate::theme::theme;

/// The most icons before the ⋮.
pub const MAX_ICONS: usize = 4;
/// The planning plugin's icon, from the terminal face's symbol page.
pub const PLANS_GLYPH: &str = "\u{2611}";
/// The agent menu's icon (plan 20261023_07), from the terminal face.
pub const AGENT_MENU_GLYPH: &str = "\u{2299}";

/// What an icon is drawn as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// ⊙.
    AgentMenu,
    /// ☑.
    Plans,
    /// The gear.
    Settings,
    /// A name the app does not know: this letter in a ring.
    Letter(char),
}

/// The glyph a plugin's icon `name` is drawn as; an unknown name is the
/// first letter of `label` (else of `name`) in a ring.
pub fn glyph_of(name: &str, label: &str) -> Glyph {
    match name {
        "plans" => Glyph::Plans,
        _ => Glyph::Letter(
            label
                .chars()
                .chain(name.chars())
                .find(|c| c.is_alphanumeric())
                .map_or('?', |c| c.to_uppercase().next().unwrap_or(c)),
        ),
    }
}

/// Who registered an icon, and what a tap does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The subagents plan's menu.
    AgentMenu,
    /// Opens the settings.
    Settings,
    /// A plugin's item, with what it opens.
    Plugin {
        ns: String,
        item: String,
        opens: String,
    },
}

/// One registered icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub source: Source,
    pub glyph: Glyph,
    /// Its item's label in the ⋮ menu, when it moves there.
    pub label: String,
}

impl Icon {
    /// Its id, as the ⋮ menu's item when moved: `<ns>/<item>` for a
    /// plugin's, as its ⋮ items are.
    pub fn id(&self) -> String {
        match &self.source {
            Source::AgentMenu => "app/agent_menu".to_string(),
            Source::Settings => crate::menu::SETTINGS.to_string(),
            Source::Plugin { ns, item, .. } => format!("{ns}/{item}"),
        }
    }

    fn is_plugin(&self) -> bool {
        matches!(self.source, Source::Plugin { .. })
    }

    fn settings() -> Icon {
        Icon {
            source: Source::Settings,
            glyph: Glyph::Settings,
            label: crate::settings::copy::MENU_SETTINGS.text.to_string(),
        }
    }
}

/// The icons registered for `screen`, left to right: the agent menu when
/// `agent_menu` (the agent view only), `plugins`' icons in their order,
/// then Settings on the overview and the agent view.
pub fn registered(screen: Screen, agent_menu: bool, plugins: &[MenuEntry]) -> Vec<Icon> {
    let mut icons = Vec::new();
    if agent_menu && screen == Screen::Agent {
        icons.push(Icon {
            source: Source::AgentMenu,
            glyph: Glyph::AgentMenu,
            label: String::new(),
        });
    }
    for p in plugins {
        let Some(name) = &p.icon else { continue };
        icons.push(Icon {
            source: Source::Plugin {
                ns: p.ns.clone(),
                item: p.item.clone(),
                opens: p.opens.clone(),
            },
            glyph: glyph_of(name, &p.label),
            label: p.label.clone(),
        });
    }
    if matches!(screen, Screen::Overview | Screen::Agent) {
        icons.push(Icon::settings());
    }
    icons
}

/// `icons` split into the title bar's, left to right, and those moved into
/// the ⋮ menu, in its order: plugin icons in registry order, then
/// Settings last.
pub fn overflow(mut icons: Vec<Icon>) -> (Vec<Icon>, Vec<Icon>) {
    let mut settings = None;
    if icons.len() > MAX_ICONS
        && let Some(i) = icons.iter().position(|i| i.source == Source::Settings)
    {
        settings = Some(icons.remove(i));
    }
    let mut moved = Vec::new();
    while icons.len() > MAX_ICONS {
        let Some(i) = icons.iter().rposition(Icon::is_plugin) else {
            break;
        };
        moved.insert(0, icons.remove(i));
    }
    moved.extend(settings);
    (icons, moved)
}

/// A screen's title bar: its icons as placed, the ⋮ and what moved into
/// its menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bar {
    pub icons: Vec<(Icon, Rect)>,
    pub kebab: Kebab,
    pub moved: Vec<Icon>,
}

impl Bar {
    /// The bar of `registered` icons, placed right to left from the left
    /// of `kebab`, on its line, at scale `s`.
    pub fn new(kebab: Kebab, registered: Vec<Icon>, s: f32) -> Bar {
        let (shown, moved) = overflow(registered);
        let side = Kebab::SIZE * s;
        let r = kebab.rect;
        let y = r.y + (r.h - side) * 0.5;
        let n = shown.len();
        let icons = shown
            .into_iter()
            .enumerate()
            .map(|(i, icon)| {
                let x = r.x - (n - i) as f32 * side;
                (icon, Rect::new(x, y, side, side))
            })
            .collect();
        Bar {
            icons,
            kebab,
            moved,
        }
    }

    /// The icon a tap at `(x, y)` is on.
    pub fn icon_at(&self, x: f32, y: f32) -> Option<usize> {
        self.icons.iter().position(|(_, r)| r.contains([x, y]))
    }

    /// Where `source`'s icon is, if it is in the bar.
    pub fn rect_of(&self, source: &Source) -> Option<Rect> {
        self.icons
            .iter()
            .find(|(i, _)| i.source == *source)
            .map(|&(_, r)| r)
    }

    /// The icons, then the ⋮, in `color` on `under` (the bar's
    /// background, for the gear's hole and the ring's inside).
    pub fn draw(
        &self,
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
        s: f32,
        under: [f32; 4],
    ) {
        let color = theme().text;
        for (icon, rect) in &self.icons {
            draw_icon(list, ctx, term, &icon.glyph, *rect, s, color, under);
        }
        self.kebab.draw(list, s);
    }
}

/// One icon's glyph, centred in its target.
#[allow(clippy::too_many_arguments)]
pub fn draw_icon(
    list: &mut DrawList,
    ctx: &DrawContext,
    term: Option<&Terminal>,
    glyph: &Glyph,
    rect: Rect,
    s: f32,
    color: [f32; 4],
    under: [f32; 4],
) {
    let [cx, cy] = rect.center();
    let symbol = |list: &mut DrawList, text: &str| {
        let px = 24.0 * s;
        let w = text.chars().count() as f32 * px * crate::agent::CELL_EM;
        let row = Row {
            text: text.to_string(),
            x: 0.0,
            y: 0.0,
            px,
            color,
            mono: true,
        };
        draw_row(list, ctx, term, &row, [cx - w * 0.5, cy - term_h(px) * 0.5]);
    };
    match glyph {
        Glyph::Plans => symbol(list, PLANS_GLYPH),
        Glyph::AgentMenu => symbol(list, AGENT_MENU_GLYPH),
        Glyph::Settings => Gear { rect }.draw_in(list, color, under),
        Glyph::Letter(c) => {
            let outer = 11.0 * s;
            let inner = outer - 2.0 * s;
            fill(
                list,
                Rect::new(cx - outer, cy - outer, 2.0 * outer, 2.0 * outer),
                color,
                outer,
            );
            fill(
                list,
                Rect::new(cx - inner, cy - inner, 2.0 * inner, 2.0 * inner),
                under,
                inner,
            );
            let px = 13.0 * s;
            let text = c.to_string();
            let w = ctx.text_advance(&text, px);
            ctx.draw_text(list, &text, [cx - w * 0.5, cy - px * 0.62], px, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(n: usize) -> MenuEntry {
        MenuEntry {
            ns: format!("dev.flankr.plugins.p{n}"),
            name: format!("P{n}"),
            item: "go".into(),
            label: format!("Plugin {n}"),
            opens: "modal".into(),
            icon: Some(format!("p{n}")),
        }
    }

    fn plans() -> MenuEntry {
        MenuEntry {
            ns: "dev.flankr.plugins.planning".into(),
            name: "Planning".into(),
            item: "plans".into(),
            label: "Plans in this project".into(),
            opens: "menu".into(),
            icon: Some("plans".into()),
        }
    }

    fn ids(icons: &[Icon]) -> Vec<String> {
        icons.iter().map(Icon::id).collect()
    }

    const KEBAB: Kebab = Kebab {
        rect: Rect {
            x: 996.0,
            y: 100.0,
            w: 84.0,
            h: 84.0,
        },
        dim: false,
    };

    #[test]
    fn the_slots_per_screen_left_to_right() {
        let p = [plans(), plugin(1)];
        let pl = "dev.flankr.plugins.planning/plans";
        let p1 = "dev.flankr.plugins.p1/go";
        assert_eq!(
            ids(&registered(Screen::Overview, false, &p)),
            [pl, p1, crate::menu::SETTINGS]
        );
        // The agent menu first, on the agent view only.
        assert_eq!(
            ids(&registered(Screen::Agent, true, &p)),
            ["app/agent_menu", pl, p1, crate::menu::SETTINGS]
        );
        assert_eq!(
            ids(&registered(Screen::Overview, true, &p)),
            [pl, p1, crate::menu::SETTINGS]
        );
        // The settings screen: no Settings.
        assert!(registered(Screen::Settings, false, &[]).is_empty());
        // A ⋮ item without an icon is not a title icon.
        let mut plain = plugin(2);
        plain.icon = None;
        assert_eq!(
            ids(&registered(Screen::Overview, false, &[plain])),
            [crate::menu::SETTINGS]
        );
        // Placed right to left from the ⋮, each 42 pt, on its line.
        let bar = Bar::new(KEBAB, registered(Screen::Agent, true, &p), 2.0);
        let rects: Vec<Rect> = bar.icons.iter().map(|(_, r)| *r).collect();
        assert_eq!(rects.len(), 4);
        assert_eq!(rects[3].x + rects[3].w, KEBAB.rect.x);
        for w in rects.windows(2) {
            assert_eq!(w[0].x + w[0].w, w[1].x);
        }
        for r in &rects {
            assert_eq!((r.w, r.h), (84.0, 84.0));
            assert_eq!(r.y, KEBAB.rect.y);
        }
        assert_eq!(bar.icons[0].0.source, Source::AgentMenu);
        assert_eq!(bar.icons[3].0.source, Source::Settings);
        assert_eq!(bar.icon_at(rects[1].x + 1.0, rects[1].y + 1.0), Some(1));
        assert_eq!(bar.icon_at(KEBAB.rect.x + 1.0, 101.0), None);
        assert!(bar.moved.is_empty());
    }

    #[test]
    fn the_overflow_at_four_five_and_six() {
        let s = crate::menu::SETTINGS;
        // Four: all in the bar.
        let four: Vec<MenuEntry> = (1..=3).map(plugin).collect();
        let (bar, moved) = overflow(registered(Screen::Overview, false, &four));
        assert_eq!(bar.len(), 4);
        assert_eq!(bar[3].source, Source::Settings);
        assert!(moved.is_empty());
        // Five: Settings moves into the ⋮ menu.
        let five: Vec<MenuEntry> = (1..=4).map(plugin).collect();
        let (bar, moved) = overflow(registered(Screen::Overview, false, &five));
        assert_eq!(
            ids(&bar),
            (1..=4)
                .map(|n| format!("dev.flankr.plugins.p{n}/go"))
                .collect::<Vec<_>>()
        );
        assert_eq!(ids(&moved), [s]);
        // Six: Settings, then the last plugin icon, above it.
        let six: Vec<MenuEntry> = (1..=5).map(plugin).collect();
        let (bar, moved) = overflow(registered(Screen::Overview, false, &six));
        assert_eq!(bar.len(), 4);
        assert_eq!(ids(&moved), ["dev.flankr.plugins.p5/go", s]);
        // Seven, with the agent menu: it stays; the last two plugins go,
        // in registry order above Settings.
        let (bar, moved) = overflow(registered(Screen::Agent, true, &six));
        assert_eq!(bar[0].source, Source::AgentMenu);
        assert_eq!(bar.len(), 4);
        assert_eq!(
            ids(&moved),
            ["dev.flankr.plugins.p4/go", "dev.flankr.plugins.p5/go", s]
        );
    }

    #[test]
    fn plans_is_the_checkbox_and_an_unknown_name_a_letter_in_a_ring() {
        assert_eq!(glyph_of("plans", "Plans in this project"), Glyph::Plans);
        assert_eq!(PLANS_GLYPH, "\u{2611}");
        assert_eq!(glyph_of("weather", "forecast"), Glyph::Letter('F'));
        assert_eq!(glyph_of("weather", ""), Glyph::Letter('W'));
        assert_eq!(glyph_of("", ""), Glyph::Letter('?'));
        let ctx = DrawContext::default();
        let r = Rect::new(0.0, 0.0, 84.0, 84.0);
        // ☑ is drawn from the terminal face's symbol page.
        {
            use libmsdf::drawlist::AtlasSet;
            use libmsdf::font::{FontAtlas, NOTO_SANS_MONO_ATLAS, TERMINAL_SYMBOLS_ATLAS};
            use libmsdf::font::{GlyphStyle, TerminalShaper, TerminalSource};
            let text = FontAtlas::from_bytes(NOTO_SANS_MONO_ATLAS).expect("parses");
            let page = FontAtlas::from_bytes(TERMINAL_SYMBOLS_ATLAS).expect("parses");
            let shaper = TerminalShaper::new(&text, &page).expect("the atlases agree");
            let line = shaper.line(PLANS_GLYPH, GlyphStyle::Regular);
            assert!(
                line.segments
                    .iter()
                    .any(|g| g.source == TerminalSource::Symbols),
                "☑ is on the symbol page"
            );
            let mut set = AtlasSet::new();
            let (t, p) = (set.register(text).unwrap(), set.register(page).unwrap());
            let term = Terminal {
                shaper: &shaper,
                text: set.atlas(t).unwrap(),
                symbols: set.atlas(p).unwrap(),
                px_range: 4.0,
            };
            let mut list = DrawList::new();
            let glyph = glyph_of("plans", "Plans in this project");
            draw_icon(
                &mut list,
                &ctx,
                Some(&term),
                &glyph,
                r,
                2.0,
                theme().text,
                theme().bg,
            );
            assert!(!list.lower_with(&set).draws.is_empty());
        }
        // The ring: two discs, the outer in the text colour, then the
        // letter.
        let mut ring = DrawList::new();
        draw_icon(
            &mut ring,
            &ctx,
            None,
            &Glyph::Letter('F'),
            r,
            2.0,
            theme().text,
            theme().bg,
        );
        assert!(ring.instances.len() >= 2);
        assert_eq!(ring.instances[0].color, theme().text);
        assert_eq!(ring.instances[1].color, theme().bg);
        assert!(ring.instances[0].size[0] > ring.instances[1].size[0]);
        // The gear in the title's text colour.
        let mut gear = DrawList::new();
        draw_icon(
            &mut gear,
            &ctx,
            None,
            &Glyph::Settings,
            r,
            2.0,
            theme().text,
            theme().bg,
        );
        assert_eq!(gear.instances[0].color, theme().text);
    }
}
