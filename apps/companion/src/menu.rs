//! The ⋮ in the title bars and its popup menu
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.6 and step 7).
//!
//! The ⋮ ([`Kebab`]) sits at the right of the overview's title (left of
//! the gear), of the agent view's header and of the settings' title: a
//! 42 pt target, its three dots in the theme's text colour, drawn dim and
//! taking no taps while its menu would be empty. A tap opens the one
//! [`Menu`], a libcontrols `PopupMenu` (libcontrols 0.10.8) on its own
//! router's modal stack, shared by the screens and drawn over them.
//!
//! The menu holds, top to bottom ([`items`]): the enabled plugins' items
//! for the screen, under each plugin's name as a heading when more than
//! one plugin has items; then, after a separator, the title icons that
//! did not fit in the title bar (the plan's 22:03 EDT amendment, item 3):
//! plugin icons under their labels, then **Settings** last
//! ([`crate::titlebar`]). Its **Read aloud** is gone: the volume menu
//! holds both speak options (plan 20261028_06, decision 6). Ids are
//! `<ns>/<item>` for a plugin's and `app/...` for the app's ([`Choice`]).
//! The same menu offers **Unpin** for a plugin card held for 2 s on the
//! overview ([`UNPIN`]), is a plugin's popup menu for an `opens: "menu"`
//! title icon ([`crate::plugins::POPUP`]), the volume button's menu
//! ([`volume_items`]), the text input's + menu ([`attach_items`]) and the
//! agent view's agent menu (the subagents plan's ⊙,
//! [`crate::agent::subagent_menu`]), the one menu drawn with radio marks
//! ([`Menu::open_radio`]).

use std::sync::Arc;

use libcontrols::{
    DrawContext, ModalStack, ModalState, MouseButton, MouseEvent, PopupItem, PopupMarkStyle,
    PopupMenu, PopupMenuEvent, PopupMenuOptions, PopupMenuState, PopupRow, Rect, Router,
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
pub const SETTINGS: &str = "app/settings";
/// The volume menu's (plan 20261028_06, decision 3).
pub const SPEAK_CODING: &str = "app/speak_coding";
pub const SPEAK_GEMINI: &str = "app/speak_gemini";
pub const SKIP: &str = "app/skip";
/// The + menu's (decision 8 and the 23:39 EDT amendment).
pub const PASTE_TEXT: &str = "app/paste_text";
pub const ATTACH_IMAGE: &str = "app/attach_image";
/// The agent view's **Show uncommitted changes** (plan 20261028_05,
/// decision 10.1).
pub const SHOW_CHANGES: &str = "app/show_changes";
/// A held card's Unpin; the card's index follows the slash.
pub const UNPIN: &str = "app/unpin/";

/// The menu's entry on its modal stack.
const ENTRY: &str = "title_menu";

/// What a chosen item asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Settings,
    /// The volume menu: *Speak coding responses* or *Speak agent (Gemini)
    /// responses* toggled in place, to the state given (the menu stays
    /// open); or Skip.
    SpeakCoding(bool),
    SpeakGemini(bool),
    Skip,
    /// The + menu: the clipboard's text at the cursor, or the media
    /// chooser.
    PasteText,
    AttachImage,
    /// The session's `git diff` in the preview.
    ShowChanges,
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
    /// The agent menu: show the main agent (`None`) or the subagent with
    /// this id (the subagents plan, the 20:46 EDT amendment, item 4).
    Session(Option<String>),
}

impl Choice {
    /// The choice a toggle item's `Toggled` names: its id and its new
    /// state.
    pub fn toggled(id: &str, checked: bool) -> Option<Choice> {
        match id {
            SPEAK_CODING => Some(Choice::SpeakCoding(checked)),
            SPEAK_GEMINI => Some(Choice::SpeakGemini(checked)),
            _ => None,
        }
    }

    /// The choice an item id names.
    pub fn parse(id: &str) -> Option<Choice> {
        match id {
            SETTINGS => Some(Choice::Settings),
            SKIP => Some(Choice::Skip),
            PASTE_TEXT => Some(Choice::PasteText),
            ATTACH_IMAGE => Some(Choice::AttachImage),
            SHOW_CHANGES => Some(Choice::ShowChanges),
            _ => {
                if let Some(session) = crate::agent::subagent_menu::parse(id) {
                    return Some(Choice::Session(session));
                }
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
/// the title icons `moved` into it, Settings last (the plan's 22:03 EDT
/// amendment, item 3). Settings is here only when moved. Decision 3.6's
/// Read aloud is removed (plan 20261028_06, decision 6).
pub fn items(_screen: Screen, plugins: &[MenuEntry], moved: &[Icon]) -> Vec<PopupItem> {
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
    let moved: Vec<PopupItem> = moved
        .iter()
        .map(|icon| match &icon.source {
            Source::Settings => PopupItem::action(SETTINGS, copy::MENU_SETTINGS.text),
            _ => PopupItem::action(icon.id(), icon.label.clone()),
        })
        .collect();
    if !moved.is_empty() {
        if !out.is_empty() {
            out.push(PopupItem::separator());
        }
        out.extend(moved);
    }
    out
}

/// `items` with **Show uncommitted changes** first when `changes` (the
/// agent view, with the daemon serving `git`), apart from the rest by a
/// separator.
pub fn with_changes(items: Vec<PopupItem>, changes: bool) -> Vec<PopupItem> {
    if !changes {
        return items;
    }
    let mut out = vec![PopupItem::action(
        SHOW_CHANGES,
        copy::MENU_SHOW_CHANGES.text,
    )];
    if !items.is_empty() {
        out.push(PopupItem::separator());
    }
    out.extend(items);
    out
}

/// A speak option of the volume menu: a libcontrols toggle item (0.10.6),
/// checked while `on`, dim while not `enabled`. Choosing it flips it in
/// place and keeps the menu open, and [`Menu::take_choice`] gives its
/// `Toggled` (plan 20261028_06, decision 3).
fn speak_item(id: &'static str, label: &str, on: bool, enabled: bool) -> PopupItem {
    PopupItem::toggle(id, label, on).with_enabled(enabled)
}

/// The volume menu (plan 20261028_06, decision 3), top to bottom: *Speak
/// coding responses*, *Speak agent (Gemini) responses* (dim while
/// `gemini_offered` is not: the general agent off or not offered), and,
/// while `speaking`, a separator and Skip, last, nearest the finger.
/// While `muted` (the 2026-10-29 00:26 EDT amendment, item 3), both speak
/// items are dim with their checks kept, showing what unmuting brings
/// back, and there is no Skip.
pub fn volume_items(
    speak_coding: bool,
    speak_gemini: bool,
    gemini_offered: bool,
    speaking: bool,
    muted: bool,
) -> Vec<PopupItem> {
    let mut out = vec![
        speak_item(SPEAK_CODING, copy::SPEAK_CODING.text, speak_coding, !muted),
        speak_item(
            SPEAK_GEMINI,
            copy::SPEAK_GEMINI.text,
            speak_gemini,
            gemini_offered && !muted,
        ),
    ];
    let speaking = speaking && !muted;
    if speaking {
        out.push(PopupItem::separator());
        out.push(PopupItem::action(SKIP, copy::MENU_SKIP.text));
    }
    out
}

/// The + menu (plan 20261028_06, decision 8 and the 23:39 EDT amendment):
/// **Paste text**, enabled while the clipboard holds text (`clipboard`,
/// libmorph's `clipboard_has_text`), and, where `images` are taken (the message card, while
/// the daemon offers `images`), **Attach image…**.
pub fn attach_items(images: bool, clipboard: bool) -> Vec<PopupItem> {
    let mut out =
        vec![PopupItem::action(PASTE_TEXT, copy::MENU_PASTE_TEXT.text).with_enabled(clipboard)];
    if images {
        out.push(PopupItem::action(
            ATTACH_IMAGE,
            copy::MENU_ATTACH_IMAGE.text,
        ));
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
    /// The mark column's style: checks, or the agent menu's radio marks
    /// (libcontrols 0.10.7). Per menu in libcontrols, so the menu is
    /// attached again when it changes.
    marks: PopupMarkStyle,
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
            marks: PopupMarkStyle::Check,
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
            let open = self.state.is_open().then(|| self.state.rows());
            self.state.close();
            let _ = self.state.take_events();
            self.layout = layout;
            self.build();
            if let Some(rows) = open {
                self.show(self.state.anchor(), rows);
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
            .dim_text_color(theme().dim)
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
            .column_gap(10.0 * s)
            .check_width(28.0 * s)
            .check_size(14.0 * s)
            .check_thickness(2.0 * s)
            .marks(self.marks)
            .radio_radius(6.0 * s)
            .radio_thickness(1.5 * s)
            .radio_dot_radius(3.0 * s)
            .radio_empty_dot_radius(1.25 * s)
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
        self.open_rows(anchor, items.into_iter().map(PopupRow::from).collect());
    }

    /// Opens it below `anchor` with `rows`: items, with cells in shared
    /// columns or not (a plugin's popup, such as the plan table).
    pub fn open_rows(&mut self, anchor: Rect, rows: Vec<PopupRow>) {
        self.set_marks(PopupMarkStyle::Check);
        self.show(anchor, rows);
    }

    /// Opens it below `anchor` with `items`, their marks drawn as radio
    /// marks: ⦿ on the item checked, ⊙ on the others (the agent menu; the
    /// subagents plan's 20:51 EDT amendment, items 2 and 4).
    pub fn open_radio(&mut self, anchor: Rect, items: Vec<PopupItem>) {
        self.set_marks(PopupMarkStyle::Radio);
        self.show(anchor, items.into_iter().map(PopupRow::from).collect());
    }

    /// The mark style its items are drawn with.
    pub fn marks(&self) -> PopupMarkStyle {
        self.marks
    }

    fn set_marks(&mut self, marks: PopupMarkStyle) {
        if marks != self.marks {
            self.state.close();
            let _ = self.state.take_events();
            self.marks = marks;
            self.build();
        }
    }

    fn show(&mut self, anchor: Rect, rows: Vec<PopupRow>) {
        if rows.is_empty() {
            return;
        }
        self.state.open_rows(anchor, rows);
        self.dirty = true;
    }

    /// While open, its items replaced by `items` (the volume menu as it
    /// changes); unchanged items are left as they are.
    pub fn refresh(&mut self, items: Vec<PopupItem>) {
        self.refresh_rows(items.into_iter().map(PopupRow::from).collect());
    }

    /// While open, its rows replaced by `rows` (a plugin's popup after an
    /// answer); unchanged rows are left as they are.
    pub fn refresh_rows(&mut self, rows: Vec<PopupRow>) {
        if self.state.is_open() && !rows.is_empty() && self.state.rows() != rows {
            self.state.open_rows(self.state.anchor(), rows);
            self.dirty = true;
        }
    }

    /// The rows shown, while open.
    pub fn rows(&self) -> Vec<PopupRow> {
        if self.state.is_open() {
            self.state.rows()
        } else {
            Vec::new()
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
            PopupMenuEvent::Toggled { id, checked } => Choice::toggled(id.as_str(), checked),
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
        // No plugin items: nothing (Read aloud is the volume menu's now);
        // Settings is in the title bar.
        assert!(items(Screen::Overview, &[], &[]).is_empty());
        assert!(items(Screen::Agent, &[], &[]).is_empty());
        assert!(items(Screen::Settings, &[], &[]).is_empty());
        // One plugin: no heading.
        assert_eq!(
            ids(&items(
                Screen::Overview,
                std::slice::from_ref(&planning),
                &[]
            )),
            ["dev.flankr.plugins.planning/plans"]
        );
        // Two: each under its name.
        let other = entry("dev.flankr.plugins.test", "Test", "go");
        assert_eq!(
            ids(&items(Screen::Agent, &[planning.clone(), other], &[])),
            [
                "# Planning",
                "dev.flankr.plugins.planning/plans",
                "# Test",
                "dev.flankr.plugins.test/go",
            ]
        );
    }

    #[test]
    fn the_volume_menu_checks_each_option_and_skips_only_while_speaking() {
        let i = volume_items(false, true, true, false, false);
        assert_eq!(ids(&i), [SPEAK_CODING, SPEAK_GEMINI]);
        assert_eq!(i[0].label, "Speak coding responses");
        assert_eq!(i[1].label, "Speak agent (Gemini) responses");
        assert_eq!((i[0].checked, i[1].checked), (Some(false), Some(true)));
        assert!(i[0].enabled && i[1].enabled);
        assert_eq!(i[0].kind, libcontrols::PopupItemKind::Toggle);
        // Gemini's dim without the general agent.
        assert!(!volume_items(true, true, false, false, false)[1].enabled);
        // Speaking: a separator and Skip, last.
        let i = volume_items(true, false, true, true, false);
        assert_eq!(ids(&i), [SPEAK_CODING, SPEAK_GEMINI, "---", SKIP]);
        assert_eq!(i[3].label, "Skip");
        assert_eq!(
            Choice::toggled(SPEAK_CODING, true),
            Some(Choice::SpeakCoding(true))
        );
        assert_eq!(
            Choice::toggled(SPEAK_GEMINI, false),
            Some(Choice::SpeakGemini(false))
        );
        // Muted: both dim with their checks kept, and no Skip while
        // speaking.
        let i = volume_items(true, false, true, true, true);
        assert_eq!(ids(&i), [SPEAK_CODING, SPEAK_GEMINI]);
        assert_eq!((i[0].checked, i[1].checked), (Some(true), Some(false)));
        assert!(!i[0].enabled && !i[1].enabled);
        assert_eq!(Choice::toggled(SKIP, true), None);
        assert_eq!(Choice::parse(SKIP), Some(Choice::Skip));
    }

    #[test]
    fn show_uncommitted_changes_comes_first_while_git_is_served() {
        let base = items(Screen::Agent, &[], &moved(Screen::Agent, 9));
        assert_eq!(with_changes(base.clone(), false), base);
        let i = with_changes(base.clone(), true);
        assert_eq!(i[0].label, "Show uncommitted changes");
        assert_eq!(Choice::parse(SHOW_CHANGES), Some(Choice::ShowChanges));
        assert_eq!(i.len(), base.len() + 1 + usize::from(!base.is_empty()));
        assert_eq!(ids(&with_changes(Vec::new(), true)), [SHOW_CHANGES]);
    }

    #[test]
    fn the_plus_menu_has_paste_text_disabled_until_the_clipboard_call() {
        let i = attach_items(false, false);
        assert_eq!(ids(&i), [PASTE_TEXT]);
        assert_eq!(i[0].label, "Paste text");
        assert!(!i[0].enabled);
        assert!(attach_items(false, true)[0].enabled);
        assert_eq!(ids(&attach_items(true, false)), [PASTE_TEXT, ATTACH_IMAGE]);
        assert_eq!(Choice::parse(PASTE_TEXT), Some(Choice::PasteText));
        assert_eq!(Choice::parse(ATTACH_IMAGE), Some(Choice::AttachImage));
    }

    #[test]
    fn the_volume_menu_opens_above_a_bottom_button() {
        let mut m = Menu::new();
        m.set_layout(Layout {
            w: 822.0,
            h: 1800.0,
            scale: 2.0,
            top: 80.0,
            bottom: 40.0,
        });
        let anchor = Rect::new(24.0, 1600.0, 104.0, 104.0);
        m.open(anchor, volume_items(false, true, true, true, false));
        let mut list = DrawList::new();
        m.draw(&mut list, &DrawContext::default());
        let last = m.item_rect(3).expect("laid out");
        assert!(last.y + last.h <= anchor.y, "above its anchor");
        // A tap on Speak coding responses toggles it and the menu stays
        // open; Skip then chooses and closes it.
        let [x, y] = m.item_rect(0).unwrap().center();
        m.touch(1, TouchPhase::Started, x, y);
        m.touch(1, TouchPhase::Ended, x, y);
        assert_eq!(m.take_choice(), Some(Choice::SpeakCoding(true)));
        assert!(m.is_open(), "a toggle keeps the menu open");
        assert_eq!(m.items()[0].checked, Some(true));
        m.draw(&mut DrawList::new(), &DrawContext::default());
        let [x, y] = m.item_rect(3).unwrap().center();
        m.touch(2, TouchPhase::Started, x, y);
        m.touch(2, TouchPhase::Ended, x, y);
        assert_eq!(m.take_choice(), Some(Choice::Skip));
        assert!(!m.is_open());
    }

    #[test]
    fn muted_the_speak_items_take_no_taps() {
        let mut m = Menu::new();
        m.set_layout(Layout {
            w: 822.0,
            h: 1800.0,
            scale: 2.0,
            top: 80.0,
            bottom: 40.0,
        });
        let anchor = Rect::new(24.0, 1600.0, 104.0, 104.0);
        m.open(anchor, volume_items(true, true, true, false, true));
        m.draw(&mut DrawList::new(), &DrawContext::default());
        let [x, y] = m.item_rect(0).expect("laid out").center();
        m.touch(1, TouchPhase::Started, x, y);
        m.touch(1, TouchPhase::Ended, x, y);
        assert_eq!(m.take_choice(), None);
        assert_eq!(m.items()[0].checked, Some(true), "the check is kept");
    }

    #[test]
    fn settings_is_in_the_menu_only_when_moved_there() {
        // Up to three plugin icons and Settings: all in the bar.
        for n in 0..=3 {
            assert!(moved(Screen::Overview, n).is_empty());
            let i = items(Screen::Agent, &[], &moved(Screen::Agent, n));
            assert!(!ids(&i).contains(&SETTINGS.to_string()), "{n}");
        }
        // Five registered: Settings last, after a separator.
        let m = moved(Screen::Agent, 4);
        assert_eq!(ids(&items(Screen::Agent, &[], &m)), [SETTINGS]);
        assert_eq!(
            ids(&items(Screen::Overview, &[], &m)),
            [SETTINGS],
            "nothing else: no separator"
        );
        // Six: the last plugin icon above it, under its label.
        let m = moved(Screen::Overview, 5);
        let planning = entry("dev.flankr.plugins.planning", "Planning", "plans");
        let i = items(Screen::Overview, &[planning], &m);
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
            items(Screen::Overview, &[], &moved(Screen::Overview, 4)),
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
            items(Screen::Overview, &[], &moved(Screen::Overview, 4)),
        );
        m.touch(2, TouchPhase::Started, 10.0, 2000.0);
        m.touch(2, TouchPhase::Ended, 10.0, 2000.0);
        assert_eq!(m.take_choice(), None);
        assert!(!m.is_open());
        // Back closes it.
        m.open(
            anchor,
            items(Screen::Overview, &[], &moved(Screen::Overview, 4)),
        );
        assert!(m.back());
        assert!(!m.back());
    }

    /// The subagents plan, the 20:51 EDT amendment: the agent menu is
    /// drawn with radio marks, the other menus with checks; choosing main
    /// or a subagent names it.
    #[test]
    fn the_agent_menu_has_radio_marks_and_chooses_a_session() {
        use crate::agent::subagent_menu::{self, tests::NOW, tests::children};
        let mut m = Menu::new();
        m.set_layout(Layout {
            w: 1080.0,
            h: 2340.0,
            scale: 2.0,
            top: 80.0,
            bottom: 60.0,
        });
        let anchor = Rect::new(660.0, 100.0, 84.0, 84.0);
        m.open_radio(anchor, subagent_menu::items(&children(), Some("a1"), NOW));
        assert_eq!(m.marks(), PopupMarkStyle::Radio);
        assert!(m.is_open());
        m.draw(&mut DrawList::new(), &DrawContext::default());
        let r = m.item_rect(0).expect("laid out");
        assert!(r.y >= anchor.y + anchor.h, "below its ⊙");
        // main.
        let [x, y] = r.center();
        m.touch(1, TouchPhase::Started, x, y);
        m.touch(1, TouchPhase::Ended, x, y);
        assert_eq!(m.take_choice(), Some(Choice::Session(None)));
        assert!(!m.is_open());
        // The finished one, under its heading.
        let items = subagent_menu::items(&children(), None, NOW);
        let at = items
            .iter()
            .position(|i| i.label == "libtypedmd licences")
            .unwrap();
        m.open_radio(anchor, items);
        m.draw(&mut DrawList::new(), &DrawContext::default());
        let [x, y] = m.item_rect(at).unwrap().center();
        m.touch(2, TouchPhase::Started, x, y);
        m.touch(2, TouchPhase::Ended, x, y);
        assert_eq!(m.take_choice(), Some(Choice::Session(Some("c3".into()))));
        // A heading chooses nothing.
        m.open_radio(anchor, subagent_menu::items(&children(), None, NOW));
        m.draw(&mut DrawList::new(), &DrawContext::default());
        let [x, y] = m.item_rect(1).unwrap().center();
        m.touch(3, TouchPhase::Started, x, y);
        m.touch(3, TouchPhase::Ended, x, y);
        assert_eq!(m.take_choice(), None);
        // The ⋮ menu after it: checks again.
        m.close();
        m.open(anchor, volume_items(true, false, true, false, false));
        assert_eq!(m.marks(), PopupMarkStyle::Check);
        assert_eq!(
            Choice::parse("app/agent/child/a1"),
            Some(Choice::Session(Some("a1".into())))
        );
        assert_eq!(
            Choice::parse(subagent_menu::MAIN),
            Some(Choice::Session(None))
        );
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
    fn rows_with_cells_are_opened_refreshed_and_kept_across_a_resize() {
        let mut m = Menu::new();
        let anchor = Rect::new(900.0, 100.0, 84.0, 84.0);
        m.open(anchor, vec![PopupItem::heading("Reading plans…")]);
        let rows = vec![PopupItem::action("p", "Plugins").with_cells(vec![
                libcontrols::PopupCell::new("4")
                    .align(libcontrols::TextAlign::Right)
                    .scale(0.8),
                libcontrols::PopupCell::new("Plugins").flex(),
            ])];
        m.refresh_rows(rows.clone());
        assert_eq!(m.rows(), rows);
        m.set_layout(Layout {
            w: 1200.0,
            h: 800.0,
            ..Layout::default()
        });
        assert_eq!(m.rows(), rows, "the cells survive a resize");
        m.close();
        m.open_rows(anchor, Vec::new());
        assert!(!m.is_open(), "no rows, no menu");
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
