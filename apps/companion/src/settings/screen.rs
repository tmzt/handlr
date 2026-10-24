//! The settings screen (plan 20261024_02, decisions 1–3, and the 11:32 EDT
//! amendment): a header with "‹" and the title, and under it one vertical
//! libcontrols `ScrollPane` of sections, each a small dim caps title and
//! its rows, 52 pt high. In order:
//!
//! - **Sound**: *All sound*, and *Ding when an agent is done*, inactive
//!   while all sound is off;
//! - **Voice**: *Microphone* and *General agent (Gemini)*;
//! - **Display**: *Scrollbar*, "‹ Hidden ›";
//! - **Battery**, on Android only: Android's battery optimization, the
//!   vendor's battery page when the phone has one, and a note when battery
//!   use is restricted; until libmorph's battery wrapper is released, one
//!   note, "Battery settings need an app update";
//! - **Device**, last, in red: *Unpair this device* and *Delete data*,
//!   each behind a confirming modal ([`Confirm`]).
//!
//! The screen is a libcontrols `Router`, rebuilt when the layout changes.
//! Touches go to it as presses, drags and clicks: the pane tells a tap
//! from a scroll, and only a tap (a `Click`) changes anything.

use std::sync::Arc;
use std::time::Duration;

use libcontrols::{
    ControlAction, DismissReason, DrawContext, EventResponse, Label, ModalEvent, ModalStack,
    ModalState, MouseButton, MouseEvent, NodeFlags, NodeId, Rect, Router, ScrollAxis, ScrollPane,
    ScrollState, TextAlign,
};
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use super::battery::{Battery, BatteryState, VendorPage};
use super::confirm::Confirm;
use super::danger_row::DangerRow;
use super::link_row::LinkRow;
use super::option_row::OptionRow;
use super::toggle_row::ToggleRow;
use super::{Settings, scrollbar_label, step_scrollbar};
use crate::agent::{BG, DIM, FG, PAD, Row, Terminal, draw_row, fill, line_h, term_h};
use crate::ui::Layout;

/// A row's height, in points (decision 2).
pub const ROW_H: f32 = 52.0;
/// A row's label and subtitle, and a section's title, in points.
pub const LABEL_PT: f32 = 17.0;
pub const SUBTITLE_PT: f32 = 13.0;
const SECTION_PT: f32 = 12.0;
/// A section title's height, and the space after a section, in points.
const SECTION_H: f32 = 36.0;
const SECTION_GAP: f32 = 12.0;
/// The header, as the agent view's.
const HEADER_H: f32 = 64.0;
const HEADER_BG: [f32; 4] = [0.07, 0.07, 0.09, 1.0];
const TITLE_PT: f32 = 18.0;
/// The back target's width, in points.
const BACK_W: f32 = 56.0;
const BACK_GLYPH: &str = "\u{2039}";

/// The battery notes (decision 3, and step 5 until the wrapper is out).
pub const BATTERY_UPDATE: &str = "Battery settings need an app update";
pub const BATTERY_RESTRICTED: &str =
    "Battery use is Restricted: handlr can't stay connected in the background";

/// `px` text centred down `rect`.
pub fn label_y(rect: Rect, px: f32) -> f32 {
    rect.y + (rect.h - line_h(px)) * 0.5
}

/// The sections, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Sound,
    Voice,
    Display,
    Battery,
    Device,
}

impl Section {
    const ALL: [Section; 5] = [
        Section::Sound,
        Section::Voice,
        Section::Display,
        Section::Battery,
        Section::Device,
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Sound => "SOUND",
            Section::Voice => "VOICE",
            Section::Display => "DISPLAY",
            Section::Battery => "BATTERY",
            Section::Device => "DEVICE",
        }
    }
}

/// What a touch on the screen did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsHit {
    /// The header's "‹": back to the overview.
    Back,
    /// A toggle or the scrollbar changed: the settings as they are now.
    Changed(Settings),
    /// *Unpair* in its modal.
    Unpair,
    /// *Delete* in its modal.
    DeleteData,
}

/// Which modal is open, and its buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asking {
    Unpair,
    Delete,
}

impl Asking {
    /// Its entry on the modal stack.
    fn id(self) -> &'static str {
        match self {
            Asking::Unpair => "unpair",
            Asking::Delete => "delete",
        }
    }
}

/// The screen's nodes.
struct Nodes {
    pane: NodeId,
    titles: Vec<(Section, NodeId)>,
    sound: NodeId,
    ding: NodeId,
    microphone: NodeId,
    general_agent: NodeId,
    scrollbar: NodeId,
    battery_note: NodeId,
    optimization: NodeId,
    vendor: NodeId,
    restricted: NodeId,
    unpair: NodeId,
    delete: NodeId,
}

pub struct SettingsScreen {
    router: Router,
    nodes: Option<Nodes>,
    scroll: Arc<ScrollState>,
    modals: Arc<ModalState>,
    settings: Settings,
    battery: Box<dyn Battery>,
    battery_state: Option<BatteryState>,
    /// Android: the Battery section shows.
    android: bool,
    /// The vendor's page answered that it does not exist.
    no_vendor: bool,
    /// The daemon's name, and whether it is online, for the unpair modal.
    daemon: String,
    online: bool,
    /// The open modal, and its Cancel and action buttons.
    asking: Option<(Asking, NodeId, NodeId)>,
    layout: Layout,
    /// The finger held: its id and where it came down.
    touch: Option<(u64, [f32; 2])>,
    /// It came down on the header's "‹".
    on_back: bool,
    dirty: bool,
}

impl SettingsScreen {
    /// A screen showing `settings`; the Battery section from `battery`
    /// when `android`.
    pub fn new(settings: Settings, battery: Box<dyn Battery>, android: bool) -> Self {
        let battery_state = if android { battery.state() } else { None };
        SettingsScreen {
            router: Router::new(),
            nodes: None,
            scroll: ScrollState::new(),
            modals: Arc::new(ModalState::new()),
            settings,
            battery,
            battery_state,
            android,
            no_vendor: false,
            daemon: "handlr".to_string(),
            online: false,
            asking: None,
            layout: Layout::default(),
            touch: None,
            on_back: false,
            dirty: true,
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// The paired daemon's name, and whether it is online now.
    pub fn set_daemon(&mut self, name: &str, online: bool) {
        self.daemon = name.to_string();
        self.online = online;
    }

    pub fn set_layout(&mut self, layout: Layout) {
        if layout != self.layout || self.nodes.is_none() {
            self.layout = layout;
            self.build();
        }
    }

    fn scale(&self) -> f32 {
        if self.layout.scale > 0.0 {
            self.layout.scale
        } else {
            1.0
        }
    }

    /// The header's bottom.
    fn header_bottom(&self) -> f32 {
        self.layout.top + HEADER_H * self.scale()
    }

    /// The pane's window: under the header, above the system bar.
    pub fn viewport(&self) -> Rect {
        let l = self.layout;
        let top = self.header_bottom();
        Rect::new(0.0, top, l.w, (l.h - l.bottom - top).max(0.0))
    }

    /// The header's "‹".
    pub fn back_rect(&self) -> Rect {
        Rect::new(
            0.0,
            self.layout.top,
            BACK_W * self.scale(),
            HEADER_H * self.scale(),
        )
    }

    /// Makes the router's nodes for the layout, then places them.
    fn build(&mut self) {
        let s = self.scale();
        let l = self.layout;
        let mut router = Router::new();
        let viewport = self.viewport();
        let pane = router.add(
            None,
            ScrollPane::new(viewport, ScrollAxis::Vertical).with_state(Arc::clone(&self.scroll)),
        );
        let under = Some(pane);
        let titles = Section::ALL
            .iter()
            .map(|&section| {
                let label = Label::new(section.title(), Rect::new(0.0, 0.0, 1.0, 1.0))
                    .with_font_size(SECTION_PT * s)
                    .with_color(DIM)
                    .with_align(TextAlign::Left);
                (section, router.add(under, label))
            })
            .collect();
        let toggle = |router: &mut Router, label| router.add(under, ToggleRow::new(label, s));
        let sound = toggle(&mut router, "All sound");
        let ding = toggle(&mut router, "Ding when an agent is done");
        let microphone = toggle(&mut router, "Microphone");
        let general_agent = toggle(&mut router, "General agent (Gemini)");
        let scrollbar = router.add(
            under,
            OptionRow::new("Scrollbar", scrollbar_label(self.settings.scrollbar), s),
        );
        let note = |router: &mut Router, text: &str| {
            router.add(
                under,
                Label::new(text, Rect::new(0.0, 0.0, 1.0, 1.0))
                    .with_font_size(SUBTITLE_PT * s)
                    .with_color(DIM)
                    .with_align(TextAlign::Left),
            )
        };
        let battery_note = note(&mut router, BATTERY_UPDATE);
        let restricted = note(&mut router, BATTERY_RESTRICTED);
        let optimization = router.add(under, LinkRow::new("Battery optimization", s));
        let vendor = router.add(under, LinkRow::new("Vendor battery", s));
        let unpair = router.add(under, DangerRow::new("Unpair this device", s));
        let delete = router.add(under, DangerRow::new("Delete data", s));
        let _ = ModalStack::install(
            &mut router,
            Arc::clone(&self.modals),
            Rect::new(0.0, 0.0, l.w, l.h),
        );
        self.router = router;
        self.asking = None;
        self.nodes = Some(Nodes {
            pane,
            titles,
            sound,
            ding,
            microphone,
            general_agent,
            scrollbar,
            battery_note,
            optimization,
            vendor,
            restricted,
            unpair,
            delete,
        });
        self.place();
    }

    /// Sets each node's values and place, and the content's height.
    fn place(&mut self) {
        let Some(n) = &self.nodes else {
            return;
        };
        let s = self.scale();
        let w = self.layout.w;
        let viewport = self.viewport();
        let row_h = ROW_H * s;
        let mut y = viewport.y + 8.0 * s;
        let settings = self.settings;
        let router = &mut self.router;
        let show = |router: &mut Router, id: NodeId, on: bool| {
            router.set_flags(id, NodeFlags::HIDDEN, !on);
        };
        let battery = self.battery_state.as_ref().filter(|_| self.android);
        let vendor_label = battery
            .and_then(BatteryState::vendor_label)
            .filter(|_| !self.no_vendor);
        for &(section, title) in &n.titles {
            let shown = section != Section::Battery || self.android;
            show(router, title, shown);
            if !shown {
                for id in [n.battery_note, n.optimization, n.vendor, n.restricted] {
                    show(router, id, false);
                }
                continue;
            }
            if let Some(label) = router.get_mut::<Label>(title) {
                label.rect = Rect::new(
                    PAD * s,
                    y + SECTION_H * s - line_h(SECTION_PT * s) - 6.0 * s,
                    w - 2.0 * PAD * s,
                    line_h(SECTION_PT * s),
                );
            }
            y += SECTION_H * s;
            let row = |y: &mut f32| {
                let r = Rect::new(0.0, *y, w, row_h);
                *y += row_h;
                r
            };
            match section {
                Section::Sound => {
                    for (id, on, enabled) in [
                        (n.sound, settings.sound, true),
                        (n.ding, settings.ding, settings.sound),
                    ] {
                        let rect = row(&mut y);
                        if let Some(t) = router.get_mut::<ToggleRow>(id) {
                            t.rect = rect;
                            t.on = on;
                            t.enabled = enabled;
                        }
                    }
                }
                Section::Voice => {
                    for (id, on) in [
                        (n.microphone, settings.microphone),
                        (n.general_agent, settings.general_agent),
                    ] {
                        let rect = row(&mut y);
                        if let Some(t) = router.get_mut::<ToggleRow>(id) {
                            t.rect = rect;
                            t.on = on;
                        }
                    }
                }
                Section::Display => {
                    let rect = row(&mut y);
                    if let Some(o) = router.get_mut::<OptionRow>(n.scrollbar) {
                        o.set_rect(rect);
                        o.set_value(scrollbar_label(settings.scrollbar));
                    }
                }
                Section::Battery => {
                    show(router, n.battery_note, battery.is_none());
                    show(router, n.optimization, battery.is_some());
                    show(router, n.vendor, vendor_label.is_some());
                    show(
                        router,
                        n.restricted,
                        battery.is_some_and(|b| b.background_restricted),
                    );
                    let note = |router: &mut Router, id: NodeId, y: &mut f32| {
                        if let Some(label) = router.get_mut::<Label>(id) {
                            label.rect = Rect::new(PAD * s, *y, w - 2.0 * PAD * s, row_h);
                        }
                        *y += row_h;
                    };
                    match battery {
                        None => note(router, n.battery_note, &mut y),
                        Some(b) => {
                            let rect = row(&mut y);
                            if let Some(link) = router.get_mut::<LinkRow>(n.optimization) {
                                link.rect = rect;
                                link.subtitle = Some(
                                    if b.ignoring_optimizations {
                                        "Exempt"
                                    } else {
                                        "Optimized"
                                    }
                                    .to_string(),
                                );
                            }
                            if let Some(label) = vendor_label {
                                let rect = row(&mut y);
                                if let Some(link) = router.get_mut::<LinkRow>(n.vendor) {
                                    link.rect = rect;
                                    link.label = label.to_string();
                                }
                            }
                            if b.background_restricted {
                                note(router, n.restricted, &mut y);
                            }
                        }
                    }
                }
                Section::Device => {
                    for id in [n.unpair, n.delete] {
                        let rect = row(&mut y);
                        if let Some(d) = router.get_mut::<DangerRow>(id) {
                            d.rect = rect;
                        }
                    }
                }
            }
            y += SECTION_GAP * s;
        }
        if let Some(pane) = router.get::<ScrollPane>(n.pane) {
            pane.set_viewport(viewport);
            pane.set_content_size(Some([w, y + 8.0 * s - viewport.y]));
        }
        self.dirty = true;
    }

    /// Where a row is now, in content coordinates (as at offset zero).
    pub fn row_rect(&self, label: &str) -> Option<Rect> {
        let n = self.nodes.as_ref()?;
        let r = &self.router;
        let shown = |id| !r.flags(id).contains(NodeFlags::HIDDEN);
        [n.sound, n.ding, n.microphone, n.general_agent]
            .into_iter()
            .filter_map(|id| {
                r.get::<ToggleRow>(id)
                    .filter(|_| shown(id))
                    .map(|t| (t.label.clone(), t.rect))
            })
            .chain(
                r.get::<OptionRow>(n.scrollbar)
                    .map(|o| (o.label.clone(), o.rect)),
            )
            .chain([n.optimization, n.vendor].into_iter().filter_map(|id| {
                r.get::<LinkRow>(id)
                    .filter(|_| shown(id))
                    .map(|l| (l.label.clone(), l.rect))
            }))
            .chain(
                [n.unpair, n.delete]
                    .into_iter()
                    .filter_map(|id| r.get::<DangerRow>(id).map(|d| (d.label.clone(), d.rect))),
            )
            .find(|(l, _)| l == label)
            .map(|(_, rect)| rect)
    }

    /// The sections shown, in order from the top, with their titles' tops.
    pub fn sections(&self) -> Vec<(Section, f32)> {
        let Some(n) = &self.nodes else {
            return Vec::new();
        };
        let mut shown: Vec<(Section, f32)> = n
            .titles
            .iter()
            .filter(|(_, id)| !self.router.flags(*id).contains(NodeFlags::HIDDEN))
            .filter_map(|&(section, id)| self.router.get::<Label>(id).map(|l| (section, l.rect.y)))
            .collect();
        shown.sort_by(|a, b| a.1.total_cmp(&b.1));
        shown
    }

    /// The notes shown under the Battery title.
    pub fn battery_notes(&self) -> Vec<String> {
        let Some(n) = &self.nodes else {
            return Vec::new();
        };
        [n.battery_note, n.restricted]
            .into_iter()
            .filter(|&id| !self.router.flags(id).contains(NodeFlags::HIDDEN))
            .filter_map(|id| self.router.get::<Label>(id).map(|l| l.state.text()))
            .collect()
    }

    /// The scrollbar row's value as shown.
    pub fn scrollbar_shown(&self) -> Option<String> {
        let n = self.nodes.as_ref()?;
        self.router
            .get::<OptionRow>(n.scrollbar)
            .map(OptionRow::value)
    }

    /// Whether a confirming modal is open.
    pub fn asking(&self) -> bool {
        self.asking.is_some()
    }

    pub fn scroll(&self) -> &Arc<ScrollState> {
        &self.scroll
    }

    /// The system Back: an open modal is cancelled, and the screen stays.
    /// Returns whether Back was the screen's; if not, it closes.
    pub fn back(&mut self) -> bool {
        if self.asking.is_some() {
            self.modals.dismiss(DismissReason::Back);
            self.settle_modal();
            return true;
        }
        false
    }

    /// A touch at `now`. A drag scrolls the sections; a tap returns what it
    /// did, if anything the app must act on.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        _now: Duration,
    ) -> Option<SettingsHit> {
        let pos = [x, y];
        let left = MouseButton::Left;
        match phase {
            TouchPhase::Started => {
                if self.touch.is_some() {
                    return None;
                }
                self.touch = Some((id, pos));
                self.on_back = self.asking.is_none() && self.back_rect().contains(pos);
                if !self.on_back {
                    self.router
                        .deliver_mouse(MouseEvent::Down { pos, button: left });
                }
                self.dirty = true;
                None
            }
            TouchPhase::Moved => {
                let (held, start) = self.touch?;
                if held != id || self.on_back {
                    return None;
                }
                self.router.deliver_mouse(MouseEvent::Drag {
                    start_pos: start,
                    current_pos: pos,
                    delta: [0.0, 0.0],
                    button: left,
                });
                self.dirty = true;
                None
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let (held, _) = self.touch?;
                if held != id {
                    return None;
                }
                self.touch = None;
                self.dirty = true;
                if self.on_back {
                    self.on_back = false;
                    let tapped = phase == TouchPhase::Ended && self.back_rect().contains(pos);
                    return tapped.then_some(SettingsHit::Back);
                }
                self.router
                    .deliver_mouse(MouseEvent::Up { pos, button: left });
                if phase == TouchPhase::Cancelled {
                    return None;
                }
                let response = self
                    .router
                    .deliver_mouse(MouseEvent::Click { pos, button: left });
                self.clicked(response)
            }
        }
    }

    /// A tap the router gave to `response.target`.
    fn clicked(&mut self, response: EventResponse) -> Option<SettingsHit> {
        if self.asking.is_some() {
            return self.settle_modal();
        }
        let n = self.nodes.as_ref()?;
        let target = response.target?;
        let action = response.action?;
        let mut s = self.settings;
        if target == n.sound {
            s.sound = !s.sound;
        } else if target == n.ding {
            s.ding = !s.ding;
        } else if target == n.microphone {
            s.microphone = !s.microphone;
        } else if target == n.general_agent {
            s.general_agent = !s.general_agent;
        } else if target == n.scrollbar {
            let forward = match action {
                ControlAction::StepRight => true,
                ControlAction::StepLeft => false,
                _ => return None,
            };
            s.scrollbar = step_scrollbar(s.scrollbar, forward);
        } else if target == n.optimization {
            let exempt = self
                .battery_state
                .as_ref()
                .is_some_and(|b| b.ignoring_optimizations);
            self.battery.open_optimization(exempt);
            return None;
        } else if target == n.vendor {
            if self.battery.open_vendor() == VendorPage::None {
                log::info!("battery: the phone has no vendor battery page");
                self.no_vendor = true;
                self.place();
            }
            return None;
        } else if target == n.unpair {
            self.ask(Asking::Unpair);
            return None;
        } else if target == n.delete {
            self.ask(Asking::Delete);
            return None;
        } else {
            return None;
        }
        self.settings = s;
        self.place();
        Some(SettingsHit::Changed(s))
    }

    /// Opens the confirming modal for `what`.
    fn ask(&mut self, what: Asking) {
        let s = self.scale();
        let l = self.layout;
        let viewport = Rect::new(0.0, l.top, l.w, l.h - l.top - l.bottom);
        let (question, action) = match what {
            Asking::Unpair => {
                let mut q = vec![format!(
                    "Unpair this phone from {}? You will need to pair again to use handlr.",
                    self.daemon
                )];
                if !self.online {
                    q.insert(
                        0,
                        "handlr is offline: it will still list this phone until it is removed from auth.toml."
                            .to_string(),
                    );
                }
                (q, "Unpair")
            }
            Asking::Delete => (
                vec![
                    "Delete handlr's data on this phone? Settings, stored scrollback and pairing are removed. Voice models stay."
                        .to_string(),
                ],
                "Delete",
            ),
        };
        let confirm = Confirm::new(viewport, &question, action, s);
        let id = what.id();
        match self.router.push_modal(id, confirm.dialog) {
            Ok(()) => {
                self.asking = Some((what, confirm.cancel, confirm.action));
                self.dirty = true;
            }
            Err(e) => log::warn!("settings: the {id} modal: {e:?}"),
        }
    }

    /// What the modal stack says happened to the open modal: its action
    /// button closed it, or anything else (*Cancel*, a tap outside, Back).
    fn settle_modal(&mut self) -> Option<SettingsHit> {
        let (what, _, action) = self.asking?;
        let mut hit = None;
        for event in self.modals.take_events() {
            if let ModalEvent::Dismissed { by, .. } = event {
                self.asking = None;
                self.dirty = true;
                if by == DismissReason::Control(action) {
                    hit = Some(match what {
                        Asking::Unpair => SettingsHit::Unpair,
                        Asking::Delete => SettingsHit::DeleteData,
                    });
                }
            }
        }
        hit
    }

    /// Carries the pane's fling and the modal's transition on. Returns
    /// whether anything moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let dt = dt.as_secs_f32();
        let moved = self.router.advance(dt) | self.modals.advance(dt);
        if moved {
            self.dirty = true;
        }
        moved
    }

    /// Reads the open modal's content.
    #[cfg(test)]
    fn dialog<R>(&self, read: impl FnOnce(&Router) -> R) -> Option<R> {
        let (what, ..) = self.asking?;
        let stack = self.router.find(ModalStack::NAME)?;
        let stack = self.router.get::<ModalStack>(stack)?;
        let dialog = stack.modal::<libcontrols::ModalDialog>(what.id())?;
        Some(read(dialog.content()))
    }

    /// The centres of the open modal's Cancel and action buttons.
    #[cfg(test)]
    fn modal_points(&self) -> ([f32; 2], [f32; 2]) {
        let (_, cancel, action) = self.asking.expect("a modal");
        self.dialog(|c| {
            let at = |id| {
                c.get::<libcontrols::Button>(id)
                    .expect("a button")
                    .rect
                    .center()
            };
            (at(cancel), at(action))
        })
        .expect("its dialog")
    }

    /// The open modal's text.
    #[cfg(test)]
    fn modal_text(&self) -> String {
        self.dialog(|c| {
            c.children(None)
                .filter_map(|id| c.get::<Label>(id).map(|l| l.state.text()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
    }

    /// Marks the screen to be drawn again.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Rebuilds `list` if anything changed. Returns whether it did.
    pub fn draw(
        &mut self,
        list: &mut DrawList,
        ctx: &DrawContext,
        term: Option<&Terminal>,
    ) -> bool {
        if !self.dirty {
            return false;
        }
        list.clear();
        let l = self.layout;
        let s = self.scale();
        fill(list, Rect::new(0.0, 0.0, l.w, l.h), BG, 0.0);
        // The header first: the modal's dim goes over it.
        fill(
            list,
            Rect::new(0.0, 0.0, l.w, self.header_bottom()),
            HEADER_BG,
            0.0,
        );
        let px = TITLE_PT * s;
        let back = Row {
            text: if term.is_some() { BACK_GLYPH } else { "<" }.to_string(),
            x: 0.0,
            y: 0.0,
            px: 1.6 * px,
            color: FG,
            mono: true,
        };
        let back_y = l.top + (HEADER_H * s - term_h(back.px)) * 0.5;
        draw_row(list, ctx, term, &back, [PAD * s, back_y]);
        ctx.draw_text(
            list,
            "Settings",
            [BACK_W * s, l.top + (HEADER_H * s - line_h(px)) * 0.5],
            px,
            FG,
        );
        self.router.render(list, ctx);
        self.dirty = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ScrollbarVisibility;
    use std::cell::RefCell;
    use std::rc::Rc;

    const PHONE: Layout = Layout {
        w: 1080.0,
        h: 2340.0,
        scale: 2.0,
        top: 80.0,
        bottom: 40.0,
    };

    /// A phone's battery, as the wrapper will report it, with what was
    /// opened.
    struct Phone {
        state: BatteryState,
        vendor: VendorPage,
        opened: Rc<RefCell<Vec<String>>>,
    }

    impl Battery for Phone {
        fn state(&self) -> Option<BatteryState> {
            Some(self.state.clone())
        }
        fn open_optimization(&self, exempt: bool) {
            self.opened
                .borrow_mut()
                .push(format!("optimization {exempt}"));
        }
        fn open_vendor(&self) -> VendorPage {
            self.opened.borrow_mut().push("vendor".into());
            self.vendor
        }
    }

    fn screen(android: bool, battery: Box<dyn Battery>) -> SettingsScreen {
        let mut s = SettingsScreen::new(Settings::default(), battery, android);
        s.set_layout(PHONE);
        s
    }

    fn plain() -> SettingsScreen {
        screen(false, Box::new(super::super::battery::Unreleased))
    }

    fn tap(s: &mut SettingsScreen, x: f32, y: f32) -> Option<SettingsHit> {
        s.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        s.touch(1, TouchPhase::Ended, x, y, Duration::from_millis(16))
    }

    /// Taps the row `label`, scrolled to the top.
    fn tap_row(s: &mut SettingsScreen, label: &str) -> Option<SettingsHit> {
        let r = s
            .row_rect(label)
            .unwrap_or_else(|| panic!("no row {label}"));
        tap(s, r.x + 40.0, r.y + r.h * 0.5)
    }

    #[test]
    fn the_sections_are_in_order_and_battery_only_on_android() {
        let s = plain();
        let order: Vec<Section> = s.sections().into_iter().map(|(x, _)| x).collect();
        assert_eq!(
            order,
            [
                Section::Sound,
                Section::Voice,
                Section::Display,
                Section::Device
            ]
        );
        let s = screen(true, Box::new(super::super::battery::Unreleased));
        let order: Vec<Section> = s.sections().into_iter().map(|(x, _)| x).collect();
        assert_eq!(order, Section::ALL);
        // Until the wrapper is released, one note.
        assert_eq!(s.battery_notes(), [BATTERY_UPDATE]);
        assert!(s.row_rect("Battery optimization").is_none());
        // Rows are 52 pt, under the header, the danger rows last.
        let sound = s.row_rect("All sound").unwrap();
        assert_eq!(sound.h, ROW_H * 2.0);
        assert!(sound.y >= s.viewport().y);
        let delete = s.row_rect("Delete data").unwrap();
        let unpair = s.row_rect("Unpair this device").unwrap();
        assert!(unpair.y < delete.y && delete.y > sound.y);
    }

    #[test]
    fn each_toggle_flips_and_ding_is_inactive_without_sound() {
        let mut s = plain();
        assert_eq!(
            tap_row(&mut s, "All sound"),
            Some(SettingsHit::Changed(Settings {
                sound: false,
                ..Settings::default()
            }))
        );
        // Ding is dim and takes no taps while sound is off.
        assert_eq!(tap_row(&mut s, "Ding when an agent is done"), None);
        tap_row(&mut s, "All sound");
        assert_eq!(
            tap_row(&mut s, "Ding when an agent is done"),
            Some(SettingsHit::Changed(Settings {
                ding: false,
                ..Settings::default()
            }))
        );
        let Some(SettingsHit::Changed(now)) = tap_row(&mut s, "Microphone") else {
            panic!("the microphone toggles");
        };
        assert!(!now.microphone);
        let Some(SettingsHit::Changed(now)) = tap_row(&mut s, "General agent (Gemini)") else {
            panic!("the general agent toggles");
        };
        assert!(!now.general_agent && !now.microphone && !now.ding);
        assert_eq!(s.settings(), now);
    }

    #[test]
    fn the_scrollbar_arrows_step_both_ways_and_keep_the_value() {
        let mut s = plain();
        let r = s.row_rect("Scrollbar").unwrap();
        let (right, left) = (
            [r.x + r.w - PAD * 2.0 - 20.0, r.y + r.h * 0.5],
            [
                r.x + r.w - PAD * 2.0 - OptionRow::ARROWS_W * 2.0 + 20.0,
                r.y + r.h * 0.5,
            ],
        );
        assert_eq!(s.scrollbar_shown().as_deref(), Some("On Scroll"));
        let mut seen = Vec::new();
        for _ in 0..3 {
            if let Some(SettingsHit::Changed(now)) = tap(&mut s, right[0], right[1]) {
                seen.push(now.scrollbar);
            }
        }
        use ScrollbarVisibility::*;
        assert_eq!(
            seen,
            [Always, Disabled, Hidden],
            "one step per tap, wrapping"
        );
        let Some(SettingsHit::Changed(now)) = tap(&mut s, left[0], left[1]) else {
            panic!("the left arrow steps");
        };
        assert_eq!(now.scrollbar, Disabled);
        assert_eq!(s.scrollbar_shown().as_deref(), Some("Disabled"));
        // A tap on the label is not an arrow.
        assert_eq!(tap(&mut s, r.x + 40.0, r.y + r.h * 0.5), None);
        // Kept across a new layout.
        s.set_layout(Layout { w: 1200.0, ..PHONE });
        assert_eq!(s.scrollbar_shown().as_deref(), Some("Disabled"));
    }

    #[test]
    fn each_modal_opens_and_cancels_and_its_action_confirms() {
        for (row, hit) in [
            ("Unpair this device", SettingsHit::Unpair),
            ("Delete data", SettingsHit::DeleteData),
        ] {
            let mut s = plain();
            s.set_daemon("m4", true);
            assert_eq!(tap_row(&mut s, row), None);
            assert!(s.asking(), "{row} asks first");
            s.tick(Duration::from_secs(1));
            let r = s.row_rect(row).unwrap();
            // Under the modal, the rows take no taps.
            assert_eq!(tap(&mut s, r.x + 40.0, r.y + r.h * 0.5), None);
            assert!(s.asking());
            let (cancel, action) = s.modal_points();
            assert_eq!(tap(&mut s, cancel[0], cancel[1]), None);
            assert!(!s.asking(), "{row}: Cancel closes it");
            s.tick(Duration::from_secs(1));
            tap_row(&mut s, row);
            s.tick(Duration::from_secs(1));
            let (_, action2) = s.modal_points();
            assert_eq!(action, action2);
            assert_eq!(tap(&mut s, action[0], action[1]), Some(hit));
            assert!(!s.asking());
            s.tick(Duration::from_secs(1));
            // Back cancels too, and stays on the screen.
            tap_row(&mut s, row);
            s.tick(Duration::from_secs(1));
            assert!(s.back());
            assert!(!s.asking());
            assert!(!s.back(), "then Back is the app's");
        }
    }

    #[test]
    fn the_unpair_modal_says_when_handlr_is_offline() {
        let mut s = plain();
        s.set_daemon("m4", false);
        tap_row(&mut s, "Unpair this device");
        s.tick(Duration::from_secs(1));
        let text = s.modal_text();
        assert!(text.contains("handlr is offline"), "{text}");
        assert!(text.contains("from m4?"), "{text}");
    }

    #[test]
    fn the_battery_rows_from_the_wrapper() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let phone = |exempt: bool, restricted: bool, vendor| Phone {
            state: BatteryState {
                ignoring_optimizations: exempt,
                background_restricted: restricted,
                manufacturer: "samsung".into(),
            },
            vendor,
            opened: Rc::clone(&opened),
        };
        let mut s = screen(true, Box::new(phone(false, true, VendorPage::None)));
        assert!(s.row_rect("Battery optimization").is_some());
        assert_eq!(s.battery_notes(), [BATTERY_RESTRICTED]);
        assert_eq!(tap_row(&mut s, "Battery optimization"), None);
        assert_eq!(opened.borrow().as_slice(), ["optimization false"]);
        // No vendor page after all: the row goes, for good.
        assert!(s.row_rect("Samsung battery (app sleeping)").is_some());
        tap_row(&mut s, "Samsung battery (app sleeping)");
        assert!(s.row_rect("Samsung battery (app sleeping)").is_none());
        s.set_layout(Layout { w: 1200.0, ..PHONE });
        assert!(s.row_rect("Samsung battery (app sleeping)").is_none());

        let s = screen(true, Box::new(phone(true, false, VendorPage::Opened)));
        assert!(s.battery_notes().is_empty());
    }

    #[test]
    fn a_drag_scrolls_and_is_not_a_tap() {
        let mut s = SettingsScreen::new(
            Settings::default(),
            Box::new(super::super::battery::Unreleased),
            false,
        );
        // A short screen, so the sections scroll.
        s.set_layout(Layout { h: 900.0, ..PHONE });
        // A frame: the pane learns its range.
        s.tick(Duration::from_millis(16));
        let r = s.row_rect("All sound").unwrap();
        let (x, y) = (r.x + 40.0, r.y + r.h * 0.5);
        s.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        for i in 1..=10 {
            s.touch(1, TouchPhase::Moved, x, y - 20.0 * i as f32, Duration::ZERO);
        }
        assert_eq!(
            s.touch(1, TouchPhase::Ended, x, y - 200.0, Duration::ZERO),
            None
        );
        assert!(s.scroll().offset()[1] > 0.0);
        assert_eq!(s.settings(), Settings::default());
    }

    #[test]
    fn the_back_glyph_is_back() {
        let mut s = plain();
        let b = s.back_rect();
        assert_eq!(
            tap(&mut s, b.x + b.w * 0.5, b.y + b.h * 0.5),
            Some(SettingsHit::Back)
        );
        let mut list = DrawList::new();
        assert!(s.draw(&mut list, &DrawContext::default(), None));
        assert!(!s.draw(&mut list, &DrawContext::default(), None));
    }
}
