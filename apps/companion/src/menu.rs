//! The ⋮ in the title bars and its popup menu
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.6 and step 7).
//!
//! The ⋮ ([`Kebab`]) sits at the right of the overview's title (left of
//! the gear), of the agent view's header and of the settings' title: a
//! 42 pt target, its three dots in the theme's text colour, drawn dim and
//! taking no taps while its menu would be empty. A tap opens the one
//! [`Menu`], a libcontrols `PopupMenu` (libcontrols 0.10.3) on its own
//! router's modal stack, shared by the screens and drawn over them.
//!
//! The menu holds, top to bottom ([`items`]): the enabled plugins' items
//! for the screen, under each plugin's name as a heading when more than
//! one plugin has items; a separator; the app's own: **Read aloud** (the
//! agent view only, checked while on: the play button's toggle); then,
//! after a separator, the title icons that did not fit in the title bar
//! (the plan's 22:03 EDT amendment, item 3): plugin icons under their
//! labels, then **Settings** last ([`crate::titlebar`]). Ids are
//! `<ns>/<item>` for a plugin's and `app/read_aloud`, `app/settings` for
//! the app's ([`Choice`]). The same menu offers **Unpin** for a plugin
//! card held for 2 s on the overview ([`UNPIN`]), and is a plugin's popup
//! menu for an `opens: "menu"` title icon ([`crate::plugins::POPUP`]).

use std::sync::Arc;

use libcontrols::{
    DrawContext, ModalStack, ModalState, MouseButton, MouseEvent, PopupItem, PopupMenu,
    PopupMenuEvent, PopupMenuOptions, PopupMenuState, Rect, Router,
};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use crate::agent::fill;
use crate::plugins::{MenuEntry, POPUP, Screen};
use crate::settings::copy;
use crate::theme::theme;
use crate::titlebar::{Icon, Source};
use crate::ui::Layout;

/// The app's own items' ids.
pub const READ_ALOUD: &str = "app/read_aloud";
pub const SETTINGS: &str = "app/settings";
/// A held card's Unpin; the card's index follows the slash.
pub const UNPIN: &str = "app/unpin/";

/// The menu's entry on its modal stack.
const ENTRY: &str = "title_menu";

/// What a chosen item asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    ReadAloud,
    Settings,
    Plugin {
        ns: String,
        item: String,
    },
    /// Unpin the overview's plugin card at this index.
    Unpin(usize),
    /// The item `id` of `ns`'s popup menu for its `item`.
    Popup {
        ns: String,
        item: String,
        id: String,
    },
}

impl Choice {
    /// The choice an item id names.
    pub fn parse(id: &str) -> Option<Choice> {
        match id {
            READ_ALOUD => Some(Choice::ReadAloud),
            SETTINGS => Some(Choice::Settings),
            _ => {
                if let Some(i) = id.strip_prefix(UNPIN) {
                    return i.parse().ok().map(Choice::Unpin);
                }
                if let Some(rest) = id.strip_prefix(POPUP) {
                    let (ns, rest) = rest.split_once('/')?;
                    let (item, id) = rest.split_once('/')?;
                    return (!ns.is_empty() && !item.is_empty() && !id.is_empty()).then(|| {
                        Choice::Popup {
                            ns: ns.to_string(),
                            item: item.to_string(),
                            id: id.to_string(),
                        }
                    });
                }
                let (ns, item) = id.split_once('/')?;
                (ns != "app" && !ns.is_empty() && !item.is_empty()).then(|| Choice::Plugin {
                    ns: ns.to_string(),
                    item: item.to_string(),
                })
            }
        }
    }
}

/// The items of the ⋮ menu on `screen`: `plugins`' entries for it, then
/// the app's own, with Read aloud checked when `read_aloud` (decision
/// 3.6, item 2), then the title icons `moved` into it, Settings last (the
/// plan's 22:03 EDT amendment, item 3). Settings is here only when moved.
pub fn items(
    screen: Screen,
    plugins: &[MenuEntry],
    read_aloud: bool,
    moved: &[Icon],
) -> Vec<PopupItem> {
    let mut out = Vec::new();
    let mut groups: Vec<&str> = Vec::new();
    for p in plugins {
        if !groups.contains(&p.ns.as_str()) {
            groups.push(&p.ns);
        }
    }
    let headed = groups.len() > 1;
    for ns in &groups {
        let mut entries = plugins.iter().filter(|p| p.ns == *ns).peekable();
        if headed && let Some(first) = entries.peek() {
            out.push(PopupItem::heading(first.name.clone()));
        }
        for e in entries {
            out.push(PopupItem::action(
                format!("{}/{}", e.ns, e.item),
                e.label.clone(),
            ));
        }
    }
    let mut own = Vec::new();
    if screen == Screen::Agent {
        own.push(
            PopupItem::action(READ_ALOUD, copy::MENU_READ_ALOUD.text)
                .with_checked(Some(read_aloud)),
        );
    }
    let moved: Vec<PopupItem> = moved
        .iter()
        .map(|icon| match &icon.source {
            Source::Settings => PopupItem::action(SETTINGS, copy::MENU_SETTINGS.text),
            _ => PopupItem::action(icon.id(), icon.label.clone()),
        })
        .collect();
    for group in [own, moved] {
        if group.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(PopupItem::separator());
        }
        out.extend(group);
    }
    out
}

/// The ⋮ glyph and its target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Kebab {
    pub rect: Rect,
    /// Its menu would be empty: drawn dim, no taps.
    pub dim: bool,
}

impl Kebab {
    /// The target's side, in points.
    pub const SIZE: f32 = 42.0;

    /// A ⋮ whose target's right edge is at `right`, centred down a line
    /// from `y`, `line` high, at scale `s`.
    pub fn place(right: f32, y: f32, line: f32, s: f32) -> Self {
        let side = Self::SIZE * s;
        Kebab {
            rect: Rect::new(right - side, y + (line - side) * 0.5, side, side),
            dim: false,
        }
    }

    /// Whether a tap at `(x, y)` is its.
    pub fn takes(&self, x: f32, y: f32) -> bool {
        !self.dim && self.rect.contains([x, y])
    }

    /// Three dots down its centre.
    pub fn draw(&self, list: &mut DrawList, s: f32) {
        let [cx, cy] = self.rect.center();
        let d = 4.0 * s;
        let mut color = theme().text;
        if self.dim {
            color[3] *= crate::settings::DISABLED_ALPHA;
        }
        for i in [-1.0, 0.0, 1.0] {
            let y = cy + i * 7.0 * s;
            fill(
                list,
                Rect::new(cx - d * 0.5, y - d * 0.5, d, d),
                color,
                d * 0.5,
            );
        }
    }
}

/// The popup menu, shared by the screens: its own router, whose modal
/// stack takes every touch while it is open.
pub struct Menu {
    router: Router,
    modals: Arc<ModalState>,
    state: Arc<PopupMenuState>,
    layout: Layout,
    touch: Option<(u64, [f32; 2])>,
    dirty: bool,
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu {
    pub fn new() -> Self {
        let mut menu = Menu {
            router: Router::new(),
            modals: Arc::new(ModalState::new()),
            state: Arc::new(PopupMenuState::new()),
            layout: Layout::default(),
            touch: None,
            dirty: false,
        };
        menu.build();
        menu
    }

    fn scale(&self) -> f32 {
        if self.layout.scale > 0.0 {
            self.layout.scale
        } else {
            1.0
        }
    }

    /// The screen's size changed: the menu stays inside it.
    pub fn set_layout(&mut self, layout: Layout) {
        if layout != self.layout {
            let open = self.state.is_open().then(|| self.state.items());
            self.state.close();
            let _ = self.state.take_events();
            self.layout = layout;
            self.build();
            if let Some(items) = open {
                self.open(self.state.anchor(), items);
            }
        }
    }

    /// The theme changed: the menu is made again in its colours.
    pub fn restyle(&mut self) {
        self.state.close();
        let _ = self.state.take_events();
        self.build();
    }

    fn build(&mut self) {
        let l = self.layout;
        let s = self.scale();
        let mut router = Router::new();
        let screen = Rect::new(0.0, 0.0, l.w.max(1.0), l.h.max(1.0));
        let _ = ModalStack::install(&mut router, Arc::clone(&self.modals), screen);
        let viewport = Rect::new(0.0, l.top, l.w.max(1.0), (l.h - l.top - l.bottom).max(1.0));
        let options = PopupMenuOptions::new()
            .bg(theme().card)
            .border(theme().band)
            .text_color(theme().text)
            .heading_color(theme().dim)
            .check_color(theme().accent)
            .separator_color(theme().band)
            .highlight(theme().band)
            .font_size(16.0 * s)
            .item_height(44.0 * s)
            .padding(6.0 * s)
            .item_padding(16.0 * s)
            .min_width(200.0 * s)
            .max_width(320.0 * s)
            .separator_height(9.0 * s)
            .check_width(28.0 * s)
            .check_size(14.0 * s)
            .check_thickness(2.0 * s)
            .corner_radius(10.0 * s)
            .offset(4.0 * s)
            .margin(8.0 * s)
            .shadow_offset(3.0 * s);
        let menu = PopupMenu::new(viewport)
            .with_state(Arc::clone(&self.state))
            .with_options(options);
        if let Err(e) = router.attach_popup_menu(ENTRY, menu) {
            log::warn!("menu: attaching it: {e:?}");
        }
        self.router = router;
        self.dirty = true;
    }

    /// Opens it below `anchor` with `items`.
    pub fn open(&mut self, anchor: Rect, items: Vec<PopupItem>) {
        if items.is_empty() {
            return;
        }
        self.state.open(anchor, items);
        self.dirty = true;
    }

    /// While open, its items replaced by `items` (a plugin's popup after
    /// an answer); unchanged items are left as they are.
    pub fn refresh(&mut self, items: Vec<PopupItem>) {
        if self.state.is_open() && !items.is_empty() && self.state.items() != items {
            self.state.open(self.state.anchor(), items);
            self.dirty = true;
        }
    }

    /// The items shown, while open.
    pub fn items(&self) -> Vec<PopupItem> {
        if self.state.is_open() {
            self.state.items()
        } else {
            Vec::new()
        }
    }

    /// Closes it, if open.
    pub fn close(&mut self) {
        if self.state.is_open() {
            self.state.close();
            let _ = self.state.take_events();
            self.dirty = true;
        }
    }

    pub fn is_open(&self) -> bool {
        self.state.is_open()
    }

    /// Back while it is open closes it. Whether it was.
    pub fn back(&mut self) -> bool {
        if self.state.is_open() {
            self.state.close();
            self.dirty = true;
            return true;
        }
        false
    }

    /// The item chosen since last asked, if any.
    pub fn take_choice(&mut self) -> Option<Choice> {
        let events = self.state.take_events();
        if !events.is_empty() {
            self.dirty = true;
        }
        events.into_iter().find_map(|e| match e {
            PopupMenuEvent::Chosen(id) => Choice::parse(id.as_str()),
            _ => None,
        })
    }

    /// A touch while it is open: every phase goes to its router, which
    /// chooses on a press and release on one item and dismisses on a tap
    /// outside.
    pub fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        let pos = [x, y];
        let left = MouseButton::Left;
        match phase {
            TouchPhase::Started => {
                if self.touch.is_some() {
                    return;
                }
                self.touch = Some((id, pos));
                self.router
                    .deliver_mouse(MouseEvent::Down { pos, button: left });
            }
            TouchPhase::Moved => {
                let Some((held, start)) = self.touch else {
                    return;
                };
                if held == id {
                    self.router.deliver_mouse(MouseEvent::Drag {
                        start_pos: start,
                        current_pos: pos,
                        delta: [0.0, 0.0],
                        button: left,
                    });
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let Some((held, _)) = self.touch else {
                    return;
                };
                if held != id {
                    return;
                }
                self.touch = None;
                self.router
                    .deliver_mouse(MouseEvent::Up { pos, button: left });
                if phase == TouchPhase::Ended {
                    self.router
                        .deliver_mouse(MouseEvent::Click { pos, button: left });
                }
            }
        }
        self.dirty = true;
    }

    /// It changed since last asked: the screen under it draws again.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Draws it over what is in `list`, while open.
    pub fn draw(&mut self, list: &mut DrawList, ctx: &DrawContext) {
        if self.state.is_open() {
            self.router.render(list, ctx);
        }
    }

    /// The rect of item `i`, as last drawn.
    #[cfg(test)]
    pub fn item_rect(&self, i: usize) -> Option<Rect> {
        self.state.layout()?.items.get(i).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ns: &str, name: &str, item: &str) -> MenuEntry {
        MenuEntry {
            ns: ns.into(),
            name: name.into(),
            item: item.into(),
            label: format!("{name} {item}"),
            opens: "modal".into(),
            icon: None,
        }
    }

    fn icon(ns: &str, item: &str) -> MenuEntry {
        MenuEntry {
            icon: Some(item.into()),
            ..entry(ns, "Icon", item)
        }
    }

    /// What moved into the ⋮ menu with `n` plugin icons on `screen`.
    fn moved(screen: Screen, n: usize) -> Vec<Icon> {
        let icons: Vec<MenuEntry> = (0..n)
            .map(|i| icon(&format!("dev.flankr.plugins.i{i}"), "go"))
            .collect();
        crate::titlebar::overflow(crate::titlebar::registered(screen, false, &icons)).1
    }

    fn ids(items: &[PopupItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i.kind {
                libcontrols::PopupItemKind::Heading => format!("# {}", i.label),
                libcontrols::PopupItemKind::Separator => "---".into(),
                _ => i.id.as_str().to_string(),
            })
            .collect()
    }

    #[test]
    fn the_items_per_screen_and_plugin_state() {
        let planning = entry("dev.flankr.plugins.planning", "Planning", "plans");
        // No plugin items: the app's own; Settings is in the title bar.
        assert!(items(Screen::Overview, &[], false, &[]).is_empty());
        assert_eq!(ids(&items(Screen::Agent, &[], true, &[])), [READ_ALOUD]);
        assert!(items(Screen::Settings, &[], false, &[]).is_empty());
        // One plugin: no heading.
        assert_eq!(
            ids(&items(
                Screen::Overview,
                std::slice::from_ref(&planning),
                false,
                &[]
            )),
            ["dev.flankr.plugins.planning/plans"]
        );
        // Two: each under its name.
        let other = entry("dev.flankr.plugins.test", "Test", "go");
        assert_eq!(
            ids(&items(
                Screen::Agent,
                &[planning.clone(), other],
                false,
                &[]
            )),
            [
                "# Planning",
                "dev.flankr.plugins.planning/plans",
                "# Test",
                "dev.flankr.plugins.test/go",
                "---",
                READ_ALOUD,
            ]
        );
        // Read aloud's check follows the play button.
        let on = items(Screen::Agent, &[], true, &[]);
        assert_eq!(on[0].checked, Some(true));
        assert_eq!(
            items(Screen::Agent, &[], false, &[])[0].checked,
            Some(false)
        );
    }

    #[test]
    fn settings_is_in_the_menu_only_when_moved_there() {
        // Up to three plugin icons and Settings: all in the bar.
        for n in 0..=3 {
            assert!(moved(Screen::Overview, n).is_empty());
            let i = items(Screen::Agent, &[], false, &moved(Screen::Agent, n));
            assert!(!ids(&i).contains(&SETTINGS.to_string()), "{n}");
        }
        // Five registered: Settings last, after a separator.
        let m = moved(Screen::Agent, 4);
        assert_eq!(
            ids(&items(Screen::Agent, &[], false, &m)),
            [READ_ALOUD, "---", SETTINGS]
        );
        assert_eq!(
            ids(&items(Screen::Overview, &[], false, &m)),
            [SETTINGS],
            "nothing else: no separator"
        );
        // Six: the last plugin icon above it, under its label.
        let m = moved(Screen::Overview, 5);
        let planning = entry("dev.flankr.plugins.planning", "Planning", "plans");
        let i = items(Screen::Overview, &[planning], false, &m);
        assert_eq!(
            ids(&i),
            [
                "dev.flankr.plugins.planning/plans",
                "---",
                "dev.flankr.plugins.i4/go",
                SETTINGS
            ]
        );
        assert_eq!(i[2].label, "Icon go");
        assert_eq!(i[3].label, copy::MENU_SETTINGS.text);
    }

    #[test]
    fn choices_parse_from_ids() {
        assert_eq!(Choice::parse(READ_ALOUD), Some(Choice::ReadAloud));
        assert_eq!(Choice::parse(SETTINGS), Some(Choice::Settings));
        assert_eq!(Choice::parse("app/unpin/2"), Some(Choice::Unpin(2)));
        assert_eq!(
            Choice::parse("dev.flankr.plugins.planning/plans"),
            Some(Choice::Plugin {
                ns: "dev.flankr.plugins.planning".into(),
                item: "plans".into()
            })
        );
        assert_eq!(
            Choice::parse("app/popup/dev.flankr.plugins.planning/plans/plan/20261028_04_X"),
            Some(Choice::Popup {
                ns: "dev.flankr.plugins.planning".into(),
                item: "plans".into(),
                id: "plan/20261028_04_X".into()
            })
        );
        assert_eq!(Choice::parse("app/popup/ns/item"), None);
        assert_eq!(Choice::parse("app/other"), None);
        assert_eq!(Choice::parse("plans"), None);
    }

    #[test]
    fn a_tap_on_an_item_chooses_it_and_outside_dismisses() {
        let mut m = Menu::new();
        m.set_layout(Layout {
            w: 1080.0,
            h: 2340.0,
            scale: 2.0,
            top: 80.0,
            bottom: 60.0,
        });
        let anchor = Rect::new(980.0, 100.0, 84.0, 84.0);
        m.open(
            anchor,
            items(Screen::Overview, &[], false, &moved(Screen::Overview, 4)),
        );
        assert!(m.is_open());
        // Drawn once, so it has a layout.
        let mut list = DrawList::new();
        m.draw(&mut list, &DrawContext::default());
        let r = m.item_rect(0).expect("laid out");
        assert!(r.y >= anchor.y + anchor.h, "below its anchor");
        let [x, y] = r.center();
        m.touch(1, TouchPhase::Started, x, y);
        m.touch(1, TouchPhase::Ended, x, y);
        assert_eq!(m.take_choice(), Some(Choice::Settings));
        assert!(!m.is_open());
        // Outside: dismissed, nothing chosen.
        m.open(
            anchor,
            items(Screen::Overview, &[], false, &moved(Screen::Overview, 4)),
        );
        m.touch(2, TouchPhase::Started, 10.0, 2000.0);
        m.touch(2, TouchPhase::Ended, 10.0, 2000.0);
        assert_eq!(m.take_choice(), None);
        assert!(!m.is_open());
        // Back closes it.
        m.open(
            anchor,
            items(Screen::Overview, &[], false, &moved(Screen::Overview, 4)),
        );
        assert!(m.back());
        assert!(!m.back());
    }

    #[test]
    fn a_popup_is_refreshed_while_open_only() {
        let mut m = Menu::new();
        let anchor = Rect::new(900.0, 100.0, 84.0, 84.0);
        m.open(anchor, vec![PopupItem::heading("Reading plans…")]);
        let next = vec![PopupItem::action("a", "A"), PopupItem::action("b", "B")];
        m.refresh(next.clone());
        assert_eq!(m.items(), next);
        m.close();
        assert!(!m.is_open());
        m.refresh(vec![PopupItem::action("c", "C")]);
        assert!(!m.is_open());
        assert!(m.items().is_empty());
    }

    #[test]
    fn the_kebab_is_42_pt_and_dim_takes_no_taps() {
        let mut k = Kebab::place(1000.0, 100.0, 80.0, 2.0);
        assert_eq!((k.rect.w, k.rect.h), (84.0, 84.0));
        assert_eq!(k.rect.x + k.rect.w, 1000.0);
        assert!(k.takes(950.0, 140.0));
        k.dim = true;
        assert!(!k.takes(950.0, 140.0));
        let mut list = DrawList::new();
        k.draw(&mut list, 2.0);
        assert_eq!(list.instances.len(), 3);
    }
}
