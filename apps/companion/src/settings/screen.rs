//! The settings screen (plan 20261024_02, decisions 1–3, and the 11:32 EDT
//! amendment): a header with "‹" and the title, and under it one vertical
//! libcontrols `ScrollPane` of sections, each a small dim caps title and
//! its rows, 52 pt high. In order:
//!
//! - **Sound**: *All sound*, and *Ding when an agent is done*, inactive
//!   while all sound is off; then *Voice*, "‹ Default ›", stepping
//!   through Phonon's voices with a spoken preview, inactive until Phonon
//!   has loaded and while all sound is off ([`super::voice`]);
//! - **Voice**: *Microphone*, *General agent (Gemini)*, and *Tab follows
//!   focus* (off), inactive while the general agent is off or the daemon
//!   offers none (plan 20261024_02, the 2026-10-27 11:27 EDT amendment);
//! - **Display**: *Scrollbar*, "‹ Hidden ›"; then *Appearance* (*System*,
//!   *Always light*, *Always dark*), *Light theme* and *Dark theme*, the
//!   themes of each kind by name, each step applied at once; with *Always
//!   light* or *Always dark*, the other kind's row is drawn dim (plan
//!   20261024_05, the 15:33 EDT amendment, item 3);
//! - **Background**: *Stay connected in the background* (on), which runs
//!   libmorph's foreground service (CHANGES.md 2026-10-24 12:38 EDT), and
//!   "Needs an app update" while it is on where there is no service;
//! - **Notifications**: *Alert when an agent needs you* (off), dim with
//!   "Needs *Stay connected in the background*" while that is off; turning
//!   it on is the only place notifications are asked for;
//! - **Battery**, on Android only: Android's battery optimization, the
//!   vendor's battery page when the phone has one, and a note when battery
//!   use is restricted, from libmorph's battery pages (read again on
//!   resume, [`SettingsScreen::refresh_battery`]); where the state cannot
//!   be known, one note, "Battery settings need an app update";
//! - **Remotes** (plan 20261028_01, 3.10; plan 20261024_02, the
//!   2026-10-28 13:03 EDT amendment): an ⓘ after the title and a **+** at
//!   the title line's right, dim at [`MAX_REMOTES`], when a tap on it, as
//!   on the ⓘ, says why; one [`RemoteRow`] per paired daemon, in pairing
//!   order, its first line the remote's shown name, with an X that asks
//!   first ([`Confirm`]); with none, the one standard row *No remotes
//!   paired*. Standard rows only, no note under them (the 2026-10-28
//!   17:48 EDT amendment): a tap on a row opens its details
//!   ([`DetailsModal`]: nickname, status, hostname, when paired, the
//!   collapsible fingerprint, Remove and Close), and a removal shows
//!   *Removing…* in its confirm modal until the daemon answers, then
//!   closes, or says it was removed on this phone only until closed;
//! - **Device**, last: *Clear cache (n MB)* in the normal text colour,
//!   with no confirmation (the 12:12 EDT amendment), then *Delete data* in
//!   red, behind a confirming modal ([`Confirm`]). *Unpair this device* is
//!   gone: each remote's X replaces it.
//!
//! Rows with a long explanation have an ⓘ that opens it in a modal
//! ([`InfoModal`]): *Voice*, *General agent*, *Tab follows focus*, *Appearance*, the two above, *Battery optimization*,
//! *Samsung battery (app sleeping)*, and the Remotes title. All the copy is in
//! `copy/SETTINGS_COPY.md` ([`copy`]), drawn as Markdown where it is more than a
//! label; a link in it is tappable, and the app opens it with libmorph's
//! `ctx.open_link` ([`SettingsScreen::take_links`]).
//!
//! The screen is a libcontrols `Router`, rebuilt when the layout changes.
//! Touches go to it as presses, drags and clicks: the pane tells a tap
//! from a scroll, and only a tap (a `Click`) changes anything.

use std::sync::Arc;
use std::time::Duration;

use libcontrols::{
    ControlAction, ControlPart, DismissReason, DrawContext, EventResponse, Label, ModalDialog,
    ModalEvent, ModalStack, ModalState, ModalTransition, MouseButton, MouseEvent, NodeFlags,
    NodeId, Rect, Router, ScrollAxis, ScrollPane, ScrollState, TextAlign,
};
use libmorph::{Key, TouchPhase};
use libmsdf::drawlist::DrawList;

use flankr_companion::{MAX_REMOTES, RemoteId, RemoteInfo};
use flankr_markdown::Style;

use super::background::{self, Background};
use super::battery::{Battery, BatteryState, VendorPage};
use super::build_info::BuildInfo;
use super::confirm::Confirm;
use super::copy::{self, CopyText};
use super::danger_row::DangerRow;
use super::info::{self, INFO_PART, InfoModal};
use super::link_row::{ACTION_PART, LinkRow};
use super::option_row::OptionRow;
use super::remote_details::{DetailsModal, DetailsState};
use super::remote_row::{REMOVE_PART, RemoteRow};
use super::toggle_row::ToggleRow;
use super::voice;
use super::{Settings, dim_since, scrollbar_label, step_scrollbar};
use crate::agent::{PAD, Row, Terminal, draw_row, fill, line_h, term_h};
use crate::markdown_view::{self, Faces, MarkdownView};
use crate::theme::{self, theme};
use crate::ui::Layout;

/// After *Enable*, how long to watch for the exemption, and how often to
/// look, in seconds.
const BATTERY_WATCH: f32 = 60.0;
const BATTERY_LOOK: f32 = 1.0;

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
const TITLE_PT: f32 = 18.0;
/// The back target's width, in points.
const BACK_W: f32 = 56.0;
const BACK_GLYPH: &str = "\u{2039}";

/// The battery notes (decision 3, and step 5 until the wrapper is out).
pub const BATTERY_UPDATE: &str = copy::BATTERY_UPDATE.text;
pub const BATTERY_RESTRICTED: &str = copy::BATTERY_RESTRICTED.text;

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
    Background,
    Notifications,
    Battery,
    Remotes,
    Device,
}

impl Section {
    const ALL: [Section; 8] = [
        Section::Sound,
        Section::Voice,
        Section::Display,
        Section::Background,
        Section::Notifications,
        Section::Battery,
        Section::Remotes,
        Section::Device,
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Sound => copy::SECTION_SOUND.text,
            Section::Voice => copy::SECTION_VOICE.text,
            Section::Display => copy::SECTION_DISPLAY.text,
            Section::Background => copy::SECTION_BACKGROUND.text,
            Section::Notifications => copy::SECTION_NOTIFICATIONS.text,
            Section::Battery => copy::SECTION_BATTERY.text,
            Section::Remotes => copy::SECTION_REMOTES.text,
            Section::Device => copy::SECTION_DEVICE.text,
        }
    }
}

/// The rows with an ⓘ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Info {
    Voice,
    GeneralAgent,
    TabFollowsFocus,
    Background,
    Alerts,
    Optimization,
    Vendor,
    Appearance,
    Remotes,
}

impl Info {
    fn copy(self) -> CopyText {
        match self {
            Info::Voice => copy::VOICE,
            Info::GeneralAgent => copy::GENERAL_AGENT,
            Info::TabFollowsFocus => copy::TAB_FOLLOWS_FOCUS,
            Info::Background => copy::BACKGROUND,
            Info::Alerts => copy::ALERTS,
            Info::Optimization => copy::BATTERY_OPTIMIZATION,
            Info::Vendor => copy::SAMSUNG_BATTERY,
            Info::Appearance => copy::APPEARANCE,
            Info::Remotes => copy::REMOTES,
        }
    }
}

/// What a touch on the screen did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsHit {
    /// The header's "‹": back to the overview.
    Back,
    /// A toggle or the scrollbar changed: the settings as they are now.
    Changed(Settings),
    /// The *Voice* arrows stepped: the settings as they are now, and the
    /// new voice to preview.
    Voice(Settings),
    /// The Remotes title's **+**: the pairing screen, in its add mode.
    AddRemote,
    /// *Remove* in a remote's modal, after its X: the modal stays, saying
    /// *Removing…*, until [`SettingsScreen::remote_removed`].
    RemoveRemote(RemoteId),
    /// A remote's nickname, set or cleared (`None`: the default) in its
    /// details.
    SetNickname {
        id: RemoteId,
        nickname: Option<String>,
    },
    /// *Delete* in its modal.
    DeleteData,
    /// *Clear cache*, with something cached: the cached sessions go.
    ClearCache,
}

/// An arrow's step: forward for the right one.
fn step(action: ControlAction) -> Option<bool> {
    match action {
        ControlAction::StepRight => Some(true),
        ControlAction::StepLeft => Some(false),
        _ => None,
    }
}

/// *Clear cache (n MB)*'s label for a cache of `bytes`: n in MB, one
/// decimal under 10, and "0 MB" when empty.
pub fn clear_cache_label(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    let n = if bytes == 0 {
        "0".to_string()
    } else if mb < 10.0 {
        format!("{mb:.1}")
    } else {
        format!("{mb:.0}")
    };
    copy::CLEAR_CACHE.fill(None, &[("size", &n)])
}

/// Which modal is open, and its buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asking {
    RemoveRemote(RemoteId),
    /// After *Remove*: waiting for the daemon's answer.
    Removing(RemoteId),
    /// The daemon did not answer: removed on this phone only.
    RemovedHere(RemoteId),
    /// A remote's details.
    Details(RemoteId),
    Delete,
    /// An ⓘ's explanation.
    Info(Info),
}

/// What a tap in the open modal can be, besides its close controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// The details' nickname field.
    Field,
    /// The details' Fingerprint row.
    Fingerprint,
    /// *Remove* in a remote's confirm modal, which stays open.
    Remove,
}

/// What a key did to the nickname being typed ([`SettingsScreen::nickname_key`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Typed {
    /// Nothing is being typed, or the key does not edit.
    No,
    /// The text changed.
    Edited,
    /// Enter or Escape: typing ended, with the change to save, if any.
    Done(Option<SettingsHit>),
}

impl Asking {
    /// Its entry on the modal stack.
    fn id(self) -> &'static str {
        match self {
            Asking::RemoveRemote(_) => "remove_remote",
            Asking::Removing(_) => "removing_remote",
            Asking::RemovedHere(_) => "removed_here",
            Asking::Details(_) => "remote_details",
            Asking::Delete => "delete",
            Asking::Info(Info::Voice) => "info_voice",
            Asking::Info(Info::GeneralAgent) => "info_general_agent",
            Asking::Info(Info::TabFollowsFocus) => "info_tab_follows_focus",
            Asking::Info(Info::Background) => "info_background",
            Asking::Info(Info::Alerts) => "info_alerts",
            Asking::Info(Info::Optimization) => "info_optimization",
            Asking::Info(Info::Vendor) => "info_vendor",
            Asking::Info(Info::Appearance) => "info_appearance",
            Asking::Info(Info::Remotes) => "info_remotes",
        }
    }
}

/// The screen's nodes.
struct Nodes {
    pane: NodeId,
    titles: Vec<(Section, NodeId)>,
    sound: NodeId,
    ding: NodeId,
    voice: NodeId,
    microphone: NodeId,
    general_agent: NodeId,
    tab_follows_focus: NodeId,
    scrollbar: NodeId,
    appearance: NodeId,
    light_theme: NodeId,
    dark_theme: NodeId,
    background: NodeId,
    background_note: NodeId,
    alerts: NodeId,
    alerts_note: NodeId,
    battery_note: NodeId,
    optimization: NodeId,
    vendor: NodeId,
    restricted: NodeId,
    remotes_title: NodeId,
    /// [`MAX_REMOTES`] rows, those past the list hidden.
    remotes: Vec<NodeId>,
    /// *No remotes paired*, shown with none.
    remotes_empty: NodeId,
    clear_cache: NodeId,
    delete: NodeId,
}

/// The Remotes title line's ⓘ and **+** (plan 20261028_01, decision 25).
/// The title itself is the section's `Label`; this draws the ⓘ after it
/// and the + at the line's right, in the accent colour, a 44 pt target;
/// dim and inactive when the list is full. It takes taps only on those
/// two.
#[derive(Clone, Debug, PartialEq)]
pub struct RemotesTitle {
    /// The title line: the section title's height, the screen's width.
    pub rect: Rect,
    /// The title's width, for the ⓘ after it.
    pub title_w: f32,
    pub scale: f32,
    /// [`MAX_REMOTES`] are paired: the + is dim and takes no taps.
    pub full: bool,
}

impl RemotesTitle {
    /// The + glyph's half size, in points.
    const PLUS: f32 = 8.0;

    /// The title text's vertical centre.
    fn cy(&self) -> f32 {
        let s = self.scale;
        self.rect.y + self.rect.h - 6.0 * s - line_h(SECTION_PT * s) * 0.5
    }

    pub fn info_rect(&self) -> Rect {
        let s = self.scale;
        info::target(self.rect.x + PAD * s + self.title_w, self.cy(), s)
    }

    /// The +'s 44 pt target, at the line's right.
    pub fn add_rect(&self) -> Rect {
        let s = self.scale;
        let t = info::TARGET * s;
        Rect::new(
            self.rect.x + self.rect.w - PAD * s * 0.5 - t,
            self.cy() - t * 0.5,
            t,
            t,
        )
    }

    fn part(&self, pos: [f32; 2]) -> Option<ControlPart> {
        if self.info_rect().contains(pos) {
            Some(INFO_PART)
        } else if self.add_rect().contains(pos) {
            Some(ControlPart::Body)
        } else {
            None
        }
    }
}

impl libcontrols::DrawControl for RemotesTitle {
    fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        let s = self.scale;
        info::draw(list, ctx, self.info_rect(), s);
        let start = list.instances.len();
        let [cx, cy] = self.add_rect().center();
        let h = Self::PLUS * s;
        let t = 2.0 * s;
        list.push_line([cx - h, cy], [cx + h, cy], t, theme().accent);
        list.push_line([cx, cy - h], [cx, cy + h], t, theme().accent);
        if self.full {
            dim_since(list, start);
        }
    }
}

impl libcontrols::HostedControl for RemotesTitle {
    fn bounds(&self) -> Rect {
        let a = self.add_rect();
        let i = self.info_rect();
        let top = self.rect.y.min(a.y).min(i.y);
        let bottom = (self.rect.y + self.rect.h).max(a.y + a.h).max(i.y + i.h);
        Rect::new(self.rect.x, top, self.rect.w, bottom - top)
    }

    fn hit_test(&self, pos: [f32; 2]) -> Option<(ControlPart, f32)> {
        self.part(pos)
            .map(|part| (part, self.rect.normalized_x(pos[0])))
    }

    /// On the ⓘ, the explanation; on the +, pairing another, or while
    /// full (dim), the explanation saying why it is full.
    fn handle_mouse(
        &self,
        event: &MouseEvent,
        _ctx: &libcontrols::EventContext,
    ) -> Option<ControlAction> {
        let MouseEvent::Click {
            pos,
            button: MouseButton::Left,
        } = event
        else {
            return None;
        };
        let part = self.part(*pos)?;
        Some(ControlAction::Activated { part })
    }

    fn handle_keyboard(
        &self,
        _event: &libcontrols::KeyboardEvent,
        _ctx: &libcontrols::EventContext,
    ) -> Option<ControlAction> {
        None
    }
}

/// The remote in use after `id` is removed: the most recently paired of
/// the rest, by `paired_at` and then list order (plan 20261028_01,
/// decision 8; `Remotes::most_recent` in the client).
pub fn next_remote(remotes: &[RemoteInfo], id: RemoteId) -> Option<&RemoteInfo> {
    remotes
        .iter()
        .enumerate()
        .filter(|(_, r)| r.id != id)
        .max_by_key(|(i, r)| (r.paired_at.unwrap_or(0), *i))
        .map(|(_, r)| r)
}

pub struct SettingsScreen {
    router: Router,
    nodes: Option<Nodes>,
    scroll: Arc<ScrollState>,
    modals: Arc<ModalState>,
    settings: Settings,
    /// Phonon's voices, once it has loaded; until then *Voice* is inactive.
    voices: Option<Vec<String>>,
    battery: Box<dyn Battery>,
    battery_state: Option<BatteryState>,
    /// After *Enable*, seconds left to watch for the exemption, and since
    /// the last look. Android's Allow dialog sits over the app without
    /// taking its surface, so no resume says it closed.
    battery_watch: Option<(f32, f32)>,
    /// The service and the notification permission (libmorph 0.4.0's).
    service: Box<dyn Background>,
    /// Roboto Bold and Italic and the terminal face, for the Markdown.
    faces: Option<Arc<Faces>>,
    /// The phone's vendor, for the copy's vendor additions.
    vendor: Option<String>,
    /// Android: the Battery section shows.
    android: bool,
    /// The vendor's page answered that it does not exist.
    no_vendor: bool,
    /// The paired remotes, in pairing order, as the client last gave them.
    remotes: Vec<RemoteInfo>,
    /// The remote whose details are open, and what they show.
    details: Option<(RemoteId, DetailsState)>,
    /// The nickname last asked for in them, so closing does not ask again.
    nickname_sent: Option<Option<String>>,
    /// The name of the remote being removed, for its modal.
    removing: String,
    /// The open modal's taps that are not its close controls.
    targets: Vec<(Target, Rect)>,
    /// The name this phone paired as: its `[[client]]` entry's, for the
    /// removal modal.
    device: String,
    /// The cached sessions' size, in bytes, for *Clear cache (n MB)*.
    cache: u64,
    /// The open modal, and its Cancel and action buttons (an ⓘ's: OK
    /// twice).
    asking: Option<(Asking, NodeId, NodeId)>,
    /// The open modal's Markdown, for its links.
    modal_text: Option<NodeId>,
    /// A hit left by Back closing a modal ([`SettingsScreen::take_hit`]).
    pending: Option<SettingsHit>,
    /// Links tapped, for the app to open with `ctx.open_link`.
    links: Vec<String>,
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
            // Open and closed at once, never faded: a fading panel showed
            // the rows through it, and a frame left mid-fade stayed see-through
            // (owner, 2026-10-24: "no transparency on settings dialogs"; and
            // "closing a confirm modal shouldn't have a delay, no fade out").
            modals: Arc::new(ModalState::new().with_transition(ModalTransition::Instant)),
            settings,
            voices: None,
            battery,
            battery_state,
            battery_watch: None,
            // Ready, with a request no one reads, until the app gives its
            // own (`with_background`).
            service: Box::new(background::Morph(Default::default())),
            faces: None,
            vendor: None,
            android,
            no_vendor: false,
            remotes: Vec::new(),
            details: None,
            nickname_sent: None,
            removing: String::new(),
            targets: Vec::new(),
            device: "this phone".to_string(),
            cache: 0,
            asking: None,
            modal_text: None,
            pending: None,
            links: Vec::new(),
            layout: Layout::default(),
            touch: None,
            on_back: false,
            dirty: true,
        }
    }

    /// The Background and Notifications sections' source: the app's
    /// [`background::Morph`], whose requests it reads.
    pub fn with_background(mut self, service: Box<dyn Background>) -> Self {
        self.service = service;
        self.nodes = None;
        self
    }

    /// The faces the Markdown is drawn in.
    pub fn with_faces(mut self, faces: Option<Arc<Faces>>) -> Self {
        self.faces = faces;
        self.nodes = None;
        self
    }

    /// The phone's build info, for the copy's vendor additions (`None`
    /// off Android: libmorph 0.4.1's `ctx.build_info()`).
    pub fn with_build_info(mut self, info: Option<BuildInfo>) -> Self {
        self.vendor = info.map(|i| i.vendor.to_ascii_lowercase());
        self.nodes = None;
        self
    }

    /// Phonon's voices, as `AudioEvent::TtsLoaded` named them.
    pub fn with_voices(mut self, voices: Option<Vec<String>>) -> Self {
        self.voices = voices;
        self.nodes = None;
        self
    }

    /// Phonon has loaded with `voices`: *Voice* steps through them.
    pub fn set_voices(&mut self, voices: Option<Vec<String>>) {
        if voices != self.voices {
            self.voices = voices;
            if self.nodes.is_some() {
                self.place();
            }
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.clone()
    }

    /// The voice the *Voice* row shows: the saved one if Phonon has it,
    /// else *Default*.
    fn shown_voice(&self) -> Option<String> {
        let names = self.voices.as_deref().unwrap_or_default();
        voice::effective(self.settings.voice.as_deref(), names)
    }

    /// The settings changed elsewhere, as when the alerts toggle follows
    /// the permission: shown as they are now.
    pub fn set_settings(&mut self, settings: Settings) {
        if settings != self.settings {
            self.settings = settings;
            if self.nodes.is_some() {
                self.place();
            }
        }
    }

    /// The links tapped since the last call, oldest first, for the app to
    /// open (`ctx.open_link`, which needs the context).
    pub fn take_links(&mut self) -> Vec<String> {
        std::mem::take(&mut self.links)
    }

    /// Reads the battery state again, as on returning from its pages.
    /// After *Enable*: the state read again once a second, for a minute or
    /// until flankr is exempt.
    fn watch_battery(&mut self, dt: f32) {
        let Some((left, since)) = self.battery_watch else {
            return;
        };
        let (left, since) = (left - dt, since + dt);
        if since < BATTERY_LOOK {
            self.battery_watch = Some((left, since));
            return;
        }
        self.refresh_battery();
        let exempt = self
            .battery_state
            .as_ref()
            .is_some_and(|b| b.ignoring_optimizations);
        self.battery_watch = (!exempt && left > 0.0).then_some((left, 0.0));
        if exempt {
            log::info!("battery: exempt after Enable");
        }
    }

    pub fn refresh_battery(&mut self) {
        if !self.android {
            return;
        }
        let state = self.battery.state();
        if state != self.battery_state {
            self.battery_state = state;
            if self.nodes.is_some() {
                self.place();
            }
        }
    }

    /// A note's Markdown view, laid out for the screen's width.
    fn note(&self, text: &str) -> MarkdownView {
        let s = self.scale();
        MarkdownView::new(
            text,
            PAD * s,
            0.0,
            (self.layout.w - 2.0 * PAD * s).max(1.0),
            SUBTITLE_PT * s,
            theme().dim,
            self.faces.clone(),
        )
    }

    /// `label`'s width as a row draws it.
    fn label_w(&self, label: &str) -> f32 {
        markdown_view::measure(
            self.faces.as_deref(),
            label,
            Style::Regular,
            LABEL_PT * self.scale(),
        )
    }

    /// The paired remotes ([`flankr_companion::Event::Remotes`]), shown as
    /// they are now.
    pub fn set_remotes(&mut self, remotes: Vec<RemoteInfo>) {
        if remotes == self.remotes {
            return;
        }
        self.remotes = remotes;
        if self.nodes.is_some() {
            self.place();
        }
        // Open details show the remote as it is now, or close with it.
        if let Some((Asking::Details(id), ..)) = self.asking {
            if self.remotes.iter().any(|r| r.id == id) {
                self.open(Asking::Details(id));
            } else {
                self.close_modal();
            }
        }
    }

    /// The name this phone paired as, for the removal modal's copy.
    pub fn set_device(&mut self, name: &str) {
        self.device = name.to_string();
    }

    /// The client removed `id` (`Event::RemoteRemoved`): its modal, while
    /// it says *Removing…*, closes when `daemon` answered, and otherwise
    /// says it was removed on this phone only until it is closed.
    pub fn remote_removed(&mut self, id: RemoteId, name: &str, daemon: bool) {
        if !matches!(self.asking, Some((Asking::Removing(r), ..)) if r == id) {
            return;
        }
        if daemon {
            self.close_modal();
        } else {
            self.removing = name.to_string();
            self.open(Asking::RemovedHere(id));
        }
    }

    /// The nickname being typed in the open details, if it is.
    pub fn nickname_editing(&self) -> Option<&str> {
        match &self.details {
            Some((id, d))
                if d.editing
                    && matches!(self.asking, Some((Asking::Details(a), ..)) if a == *id) =>
            {
                Some(d.draft.as_str())
            }
            _ => None,
        }
    }

    /// The soft keyboard's text for the nickname: shown as typed, at most
    /// [`flankr_companion::NICKNAME_MAX`] characters on one line.
    pub fn set_nickname_text(&mut self, text: &str) {
        let Some((id, d)) = &mut self.details else {
            return;
        };
        if !d.editing {
            return;
        }
        let text: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(flankr_companion::NICKNAME_MAX)
            .collect();
        if text != d.draft {
            d.draft = text;
            let id = *id;
            self.open(Asking::Details(id));
        }
    }

    /// A physical key while the nickname is typed: it edits it, Enter
    /// saves it and Escape ends typing.
    pub fn nickname_key(&mut self, key: Key) -> Typed {
        let Some((_, d)) = &self.details else {
            return Typed::No;
        };
        if self.nickname_editing().is_none() {
            return Typed::No;
        }
        let mut text = d.draft.clone();
        match key {
            Key::Character('\r' | '\n') | Key::Escape => return Typed::Done(self.nickname_done()),
            Key::Character('\u{8}' | '\u{7f}') => {
                text.pop();
            }
            Key::Character(c) if !c.is_control() => text.push(c),
            _ => return Typed::No,
        }
        self.set_nickname_text(&text);
        Typed::Edited
    }

    /// Typing the nickname ended (Enter, the keyboard's Done): the field
    /// stops, and a changed nickname is to be saved.
    pub fn nickname_done(&mut self) -> Option<SettingsHit> {
        let (id, d) = self.details.as_mut()?;
        let was = std::mem::replace(&mut d.editing, false);
        let id = *id;
        if was {
            self.open(Asking::Details(id));
        }
        self.commit_nickname()
    }

    /// The soft keyboard was put away: the field stops; the text stays,
    /// and is saved when the details close.
    pub fn nickname_dismissed(&mut self) {
        if let Some((id, d)) = &mut self.details
            && std::mem::replace(&mut d.editing, false)
        {
            let id = *id;
            self.open(Asking::Details(id));
        }
    }

    /// The nickname typed, if it differs from the remote's and from the
    /// one last asked for.
    fn commit_nickname(&mut self) -> Option<SettingsHit> {
        let (id, d) = self.details.as_ref()?;
        let info = self.remotes.iter().find(|r| r.id == *id)?;
        let wanted = flankr_companion::keys::nickname(Some(&d.draft));
        let asked = self.nickname_sent.as_ref().unwrap_or(&info.nickname);
        if &wanted == asked {
            return None;
        }
        self.nickname_sent = Some(wanted.clone());
        Some(SettingsHit::SetNickname {
            id: *id,
            nickname: wanted,
        })
    }

    /// The cached sessions' size, in bytes: *Clear cache (n MB)*.
    pub fn set_cache(&mut self, bytes: u64) {
        if bytes != self.cache {
            self.cache = bytes;
            if self.nodes.is_some() {
                self.place();
            }
        }
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
                    .with_color(theme().dim)
                    .with_align(TextAlign::Left);
                (section, router.add(under, label))
            })
            .collect();
        let toggle = |router: &mut Router, label: &str, info: bool| {
            let mut row = ToggleRow::new(label, s);
            row.info = info;
            row.label_w = self.label_w(label);
            router.add(under, row)
        };
        let sound = toggle(&mut router, copy::SOUND.label(), false);
        let ding = toggle(&mut router, copy::DING.label(), false);
        let voice = {
            let mut row = OptionRow::new(
                copy::VOICE.label(),
                voice::label(self.shown_voice().as_deref()),
                s,
            );
            row.info = true;
            row.label_w = self.label_w(copy::VOICE.label());
            router.add(under, row)
        };
        let microphone = toggle(&mut router, copy::MICROPHONE.label(), false);
        let general_agent = toggle(&mut router, copy::GENERAL_AGENT.label(), true);
        let tab_follows_focus = toggle(&mut router, copy::TAB_FOLLOWS_FOCUS.label(), true);
        let scrollbar = router.add(
            under,
            OptionRow::new(
                copy::SCROLLBAR.label(),
                scrollbar_label(self.settings.scrollbar),
                s,
            ),
        );
        let option = |router: &mut Router, label: &str, info: bool| {
            let mut row = OptionRow::new(label, "", s);
            row.info = info;
            row.label_w = self.label_w(label);
            router.add(under, row)
        };
        let appearance = option(&mut router, copy::APPEARANCE.label(), true);
        let light_theme = option(&mut router, copy::LIGHT_THEME.label(), false);
        let dark_theme = option(&mut router, copy::DARK_THEME.label(), false);
        let background = toggle(&mut router, copy::BACKGROUND.label(), true);
        let alerts = toggle(&mut router, copy::ALERTS.label(), true);
        let background_note = router.add(under, self.note(copy::NEEDS_UPDATE.text));
        let alerts_note = router.add(under, self.note(copy::ALERTS_NEEDS_BACKGROUND.text));
        let battery_note = router.add(under, self.note(BATTERY_UPDATE));
        let restricted = router.add(under, self.note(BATTERY_RESTRICTED));
        let link = |router: &mut Router, label: &str| {
            let mut row = LinkRow::new(label, s);
            row.info = true;
            row.label_w = self.label_w(label);
            router.add(under, row)
        };
        let optimization = link(&mut router, copy::BATTERY_OPTIMIZATION.label());
        let vendor = link(&mut router, copy::SAMSUNG_BATTERY.label());
        // The title's ⓘ and +, then the rows, which win where the +'s
        // target reaches into the first.
        let remotes_title = router.add(
            under,
            RemotesTitle {
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                title_w: markdown_view::measure(
                    self.faces.as_deref(),
                    copy::SECTION_REMOTES.text,
                    Style::Regular,
                    SECTION_PT * s,
                ),
                scale: s,
                full: false,
            },
        );
        let remotes = (0..MAX_REMOTES)
            .map(|_| router.add(under, RemoteRow::new(s)))
            .collect();
        let remotes_empty = router.add(
            under,
            RemoteRow {
                placeholder: Some(copy::REMOTES_EMPTY.text),
                ..RemoteRow::new(s)
            },
        );
        let clear_cache = router.add(
            under,
            DangerRow::new(&clear_cache_label(self.cache), s).with_color(theme().fg),
        );
        let delete = router.add(under, DangerRow::new(copy::DELETE_DATA.label(), s));
        let _ = ModalStack::install(
            &mut router,
            Arc::clone(&self.modals),
            Rect::new(0.0, 0.0, l.w, l.h),
        );
        self.router = router;
        // The open modal, made again in the new router (a new layout, as
        // the soft keyboard coming up for the nickname makes).
        let open = self.asking.take().map(|(what, ..)| what);
        self.modal_text = None;
        self.nodes = Some(Nodes {
            pane,
            titles,
            sound,
            ding,
            voice,
            microphone,
            general_agent,
            tab_follows_focus,
            scrollbar,
            appearance,
            light_theme,
            dark_theme,
            background,
            background_note,
            alerts,
            alerts_note,
            battery_note,
            optimization,
            vendor,
            restricted,
            remotes_title,
            remotes,
            remotes_empty,
            clear_cache,
            delete,
        });
        self.place();
        if let Some(what) = open {
            self.open(what);
        }
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
        let settings = self.settings.clone();
        let themes = theme::themes();
        let shown_voice = self.shown_voice();
        let voices_loaded = self.voices.is_some();
        let ready = self.service.ready();
        // The notes: the background's where there is no service, and the
        // alerts' while background is off or there is no service.
        let background_note = (settings.background && !ready).then_some(copy::NEEDS_UPDATE.text);
        let alerts_note = if !settings.background {
            Some(copy::ALERTS_NEEDS_BACKGROUND.text)
        } else {
            (settings.alerts && !ready).then_some(copy::NEEDS_UPDATE.text)
        };
        let mut notes = [
            (n.background_note, background_note.map(|t| self.note(t))),
            (n.alerts_note, alerts_note.map(|t| self.note(t))),
        ];
        let remotes = &self.remotes;
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
            if section == Section::Remotes
                && let Some(t) = router.get_mut::<RemotesTitle>(n.remotes_title)
            {
                t.rect = Rect::new(0.0, y, w, SECTION_H * s);
                t.full = remotes.len() >= MAX_REMOTES;
            }
            y += SECTION_H * s;
            let row = |y: &mut f32| {
                let r = Rect::new(0.0, *y, w, row_h);
                *y += row_h;
                r
            };
            // A note under the rows, placed.
            let note = |router: &mut Router, id: NodeId, y: &mut f32| {
                if let Some(view) = router.get_mut::<MarkdownView>(id) {
                    view.move_to(PAD * s, *y + 4.0 * s);
                    *y += view.height() + 14.0 * s;
                }
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
                    let rect = row(&mut y);
                    if let Some(o) = router.get_mut::<OptionRow>(n.voice) {
                        o.set_rect(rect);
                        o.set_value(voice::label(shown_voice.as_deref()));
                        o.enabled = settings.sound && voices_loaded;
                    }
                }
                Section::Voice => {
                    // *General agent* is dim while the daemon offers none;
                    // its ⓘ says why (plan 20261027_02, decision 9).
                    // *Tab follows focus* is dim while the general agent is
                    // off either way (plan 20261024_02, the 2026-10-27
                    // 11:27 EDT amendment).
                    let offered = super::llm().general_agent;
                    for (id, on, enabled) in [
                        (n.microphone, settings.microphone, true),
                        (n.general_agent, settings.general_agent, offered),
                        (
                            n.tab_follows_focus,
                            settings.tab_follows_focus,
                            offered && settings.general_agent,
                        ),
                    ] {
                        let rect = row(&mut y);
                        if let Some(t) = router.get_mut::<ToggleRow>(id) {
                            t.rect = rect;
                            t.on = on;
                            t.enabled = enabled;
                        }
                    }
                }
                Section::Display => {
                    let rect = row(&mut y);
                    if let Some(o) = router.get_mut::<OptionRow>(n.scrollbar) {
                        o.set_rect(rect);
                        o.set_value(scrollbar_label(settings.scrollbar));
                    }
                    let rect = row(&mut y);
                    if let Some(o) = router.get_mut::<OptionRow>(n.appearance) {
                        o.set_rect(rect);
                        o.set_value(settings.appearance.label());
                    }
                    for (id, dark) in [(n.light_theme, false), (n.dark_theme, true)] {
                        let rect = row(&mut y);
                        if let Some(o) = router.get_mut::<OptionRow>(id) {
                            o.set_rect(rect);
                            o.set_value(&settings.chosen(&themes, dark).name);
                            o.enabled = settings.appearance.row_active(dark);
                        }
                    }
                }
                Section::Background | Section::Notifications => {
                    let background = section == Section::Background;
                    let (id, on, enabled) = if background {
                        (n.background, settings.background, true)
                    } else {
                        (n.alerts, settings.alerts, settings.background)
                    };
                    let rect = row(&mut y);
                    if let Some(t) = router.get_mut::<ToggleRow>(id) {
                        t.rect = rect;
                        t.on = on;
                        t.enabled = enabled;
                    }
                    let (note_id, view) = &mut notes[usize::from(!background)];
                    let note_id = *note_id;
                    let view = view.take();
                    show(router, note_id, view.is_some());
                    if let Some(view) = view {
                        if let Some(old) = router.get_mut::<MarkdownView>(note_id) {
                            *old = view;
                        }
                        note(router, note_id, &mut y);
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
                    match battery {
                        None => note(router, n.battery_note, &mut y),
                        Some(b) => {
                            if let Some(link) = router.get_mut::<LinkRow>(n.optimization) {
                                link.rect = Rect::new(0.0, y, w, row_h);
                                link.subtitle = copy::BATTERY_OPTIMIZATION
                                    .status(b.ignoring_optimizations)
                                    .map(str::to_string);
                                // Not yet exempt: *Enable* asks the system.
                                link.action = (!b.ignoring_optimizations)
                                    .then(|| copy::BATTERY_ENABLE.text.to_string());
                                // Exempt: nothing on the right.
                                link.chevron = false;
                                // Taller when its status wraps, beside the button.
                                link.rect.h = link.height(row_h);
                                y += link.rect.h;
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
                Section::Remotes => {
                    for (i, &id) in n.remotes.iter().enumerate() {
                        let info = remotes.get(i).cloned();
                        show(router, id, info.is_some());
                        let rect = if info.is_some() {
                            row(&mut y)
                        } else {
                            Rect::new(0.0, y, w, 0.0)
                        };
                        if let Some(r) = router.get_mut::<RemoteRow>(id) {
                            r.rect = rect;
                            r.info = info;
                        }
                    }
                    // None paired: one standard row says so.
                    show(router, n.remotes_empty, remotes.is_empty());
                    let rect = if remotes.is_empty() {
                        row(&mut y)
                    } else {
                        Rect::new(0.0, y, w, 0.0)
                    };
                    if let Some(r) = router.get_mut::<RemoteRow>(n.remotes_empty) {
                        r.rect = rect;
                    }
                }
                Section::Device => {
                    for id in [n.clear_cache, n.delete] {
                        let rect = row(&mut y);
                        if let Some(d) = router.get_mut::<DangerRow>(id) {
                            d.rect = rect;
                        }
                    }
                    if let Some(d) = router.get_mut::<DangerRow>(n.clear_cache) {
                        d.label = clear_cache_label(self.cache);
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
        [
            n.sound,
            n.ding,
            n.microphone,
            n.general_agent,
            n.tab_follows_focus,
            n.background,
            n.alerts,
        ]
        .into_iter()
        .filter_map(|id| {
            r.get::<ToggleRow>(id)
                .filter(|_| shown(id))
                .map(|t| (t.label.clone(), t.rect))
        })
        .chain(
            [
                n.voice,
                n.scrollbar,
                n.appearance,
                n.light_theme,
                n.dark_theme,
            ]
            .into_iter()
            .filter_map(|id| r.get::<OptionRow>(id).map(|o| (o.label.clone(), o.rect))),
        )
        .chain([n.optimization, n.vendor].into_iter().filter_map(|id| {
            r.get::<LinkRow>(id)
                .filter(|_| shown(id))
                .map(|l| (l.label.clone(), l.rect))
        }))
        .chain(
            [n.clear_cache, n.delete]
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

    /// The Markdown of the notes `ids` that are shown.
    fn shown_notes(&self, ids: &[NodeId]) -> Vec<String> {
        ids.iter()
            .filter(|&&id| !self.router.flags(id).contains(NodeFlags::HIDDEN))
            .filter_map(|&id| {
                self.router
                    .get::<MarkdownView>(id)
                    .map(|v| v.source().to_string())
            })
            .collect()
    }

    /// The notes shown under the Battery title.
    pub fn battery_notes(&self) -> Vec<String> {
        let Some(n) = &self.nodes else {
            return Vec::new();
        };
        self.shown_notes(&[n.battery_note, n.restricted])
    }

    /// The notes shown under the Background and Notifications titles.
    pub fn background_notes(&self) -> Vec<String> {
        let Some(n) = &self.nodes else {
            return Vec::new();
        };
        self.shown_notes(&[n.background_note, n.alerts_note])
    }

    /// The scrollbar row's value as shown.
    pub fn scrollbar_shown(&self) -> Option<String> {
        let n = self.nodes.as_ref()?;
        self.router
            .get::<OptionRow>(n.scrollbar)
            .map(OptionRow::value)
    }

    /// The value shown in the option row `label`, and whether it is
    /// active.
    pub fn option_shown(&self, label: &str) -> Option<(String, bool)> {
        let n = self.nodes.as_ref()?;
        [
            n.appearance,
            n.light_theme,
            n.dark_theme,
            n.scrollbar,
            n.voice,
        ]
        .into_iter()
        .filter_map(|id| self.router.get::<OptionRow>(id))
        .find(|o| o.label == label)
        .map(|o| (o.value(), o.enabled))
    }

    /// The *Voice* row's value as shown.
    pub fn voice_shown(&self) -> Option<String> {
        let n = self.nodes.as_ref()?;
        self.router.get::<OptionRow>(n.voice).map(OptionRow::value)
    }

    /// Whether a modal is open.
    pub fn asking(&self) -> bool {
        self.asking.is_some()
    }

    pub fn scroll(&self) -> &Arc<ScrollState> {
        &self.scroll
    }

    /// The system Back: an open modal is cancelled, and the screen stays.
    /// Returns whether Back was the screen's; if not, it closes.
    /// What it left for the app, a nickname to save, is
    /// [`SettingsScreen::take_hit`]'s.
    pub fn back(&mut self) -> bool {
        if self.asking.is_some() {
            self.modals.dismiss(DismissReason::Back);
            if let Some(hit) = self.settle_modal() {
                self.pending = Some(hit);
            }
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
                self.clicked(response, pos)
            }
        }
    }

    /// A tap at `pos` the router gave to `response.target`.
    fn clicked(&mut self, response: EventResponse, pos: [f32; 2]) -> Option<SettingsHit> {
        if self.asking.is_some() {
            if let Some(url) = self.modal_link(pos) {
                self.open_link(url);
            }
            let target = self
                .targets
                .iter()
                .find(|(_, r)| r.contains(pos))
                .map(|(t, _)| *t);
            if let Some(target) = target
                && self.modals.top_takes_input()
            {
                return self.modal_target(target);
            }
            return self.settle_modal();
        }
        let n = self.nodes.as_ref()?;
        let target = response.target?;
        let action = response.action?;
        if action == (ControlAction::Activated { part: INFO_PART }) {
            let info = [
                (n.voice, Info::Voice),
                (n.general_agent, Info::GeneralAgent),
                (n.tab_follows_focus, Info::TabFollowsFocus),
                (n.background, Info::Background),
                (n.alerts, Info::Alerts),
                (n.optimization, Info::Optimization),
                (n.vendor, Info::Vendor),
                (n.appearance, Info::Appearance),
                (n.remotes_title, Info::Remotes),
            ]
            .into_iter()
            .find(|(id, _)| *id == target)
            .map(|(_, info)| info)?;
            self.explain(info);
            return None;
        }
        if let ControlAction::Activated {
            part: ControlPart::Custom(i),
        } = action
        {
            let url = self
                .router
                .get::<MarkdownView>(target)
                .and_then(|v| v.links().get(i as usize).cloned())?;
            self.open_link(url);
            return None;
        }
        let mut s = self.settings.clone();
        if target == n.sound {
            s.sound = !s.sound;
        } else if target == n.ding {
            s.ding = !s.ding;
        } else if target == n.microphone {
            s.microphone = !s.microphone;
        } else if target == n.general_agent {
            s.general_agent = !s.general_agent;
        } else if target == n.tab_follows_focus {
            s.tab_follows_focus = !s.tab_follows_focus;
        } else if target == n.background {
            s.background = !s.background;
        } else if target == n.alerts {
            s.alerts = !s.alerts;
            // The only place notifications are asked for.
            if s.alerts {
                self.service.request_notifications();
            }
        } else if target == n.scrollbar {
            let forward = match action {
                ControlAction::StepRight => true,
                ControlAction::StepLeft => false,
                _ => return None,
            };
            s.scrollbar = step_scrollbar(s.scrollbar, forward);
        } else if target == n.appearance {
            s.appearance = s.appearance.step(step(action)?);
        } else if target == n.light_theme || target == n.dark_theme {
            let dark = target == n.dark_theme;
            let id = s.step_theme(&theme::themes(), dark, step(action)?);
            if dark {
                s.dark_theme = Some(id);
            } else {
                s.light_theme = Some(id);
            }
        } else if target == n.voice {
            let forward = match action {
                ControlAction::StepRight => true,
                ControlAction::StepLeft => false,
                _ => return None,
            };
            let names = self.voices.as_deref()?;
            s.voice = voice::step(s.voice.as_deref(), names, forward);
            self.settings = s.clone();
            self.place();
            return Some(SettingsHit::Voice(s));
        } else if target == n.optimization {
            // *Enable* asks the system to exempt flankr; the row itself
            // opens the list of every app's setting.
            let enable = matches!(action, ControlAction::Activated { part } if part == ACTION_PART);
            self.battery.open_optimization(!enable);
            if enable {
                self.battery_watch = Some((BATTERY_WATCH, 0.0));
            }
            return None;
        } else if target == n.vendor {
            if self.battery.open_vendor() == VendorPage::None {
                log::info!("battery: the phone has no vendor battery page");
                self.no_vendor = true;
                self.place();
            }
            return None;
        } else if target == n.clear_cache {
            // No confirmation; nothing to do when empty.
            return (self.cache > 0).then_some(SettingsHit::ClearCache);
        } else if target == n.remotes_title {
            // A tap on the +: pairing another; at 8, dim, it says why.
            if self.remotes.len() < MAX_REMOTES {
                return Some(SettingsHit::AddRemote);
            }
            self.explain(Info::Remotes);
            return None;
        } else if let Some(i) = n.remotes.iter().position(|&id| id == target) {
            // The X asks first; the row itself opens the details.
            let id = self.remotes.get(i).map(|r| r.id)?;
            if action == (ControlAction::Activated { part: REMOVE_PART }) {
                self.ask(Asking::RemoveRemote(id));
            } else {
                self.open_details(id);
            }
            return None;
        } else if target == n.delete {
            self.ask(Asking::Delete);
            return None;
        } else {
            return None;
        }
        self.settings = s.clone();
        self.place();
        Some(SettingsHit::Changed(s))
    }

    /// A link in the copy was tapped: the app opens it.
    fn open_link(&mut self, url: String) {
        log::info!("settings: a link was tapped");
        self.links.push(url);
        self.dirty = true;
    }

    /// The link at `pos` in the open modal's Markdown, if any.
    fn modal_link(&self, pos: [f32; 2]) -> Option<String> {
        let (what, ..) = self.asking?;
        let text = self.modal_text?;
        let stack = self.router.find(ModalStack::NAME)?;
        let stack = self.router.get::<ModalStack>(stack)?;
        let dialog = stack.modal::<ModalDialog>(what.id())?;
        let view = dialog.content().get::<MarkdownView>(text)?;
        let i = view.link_at(pos)?;
        view.links().get(i).cloned()
    }

    /// The modals' viewport: the screen between its bars.
    fn modal_viewport(&self) -> Rect {
        let l = self.layout;
        Rect::new(0.0, l.top, l.w, l.h - l.top - l.bottom)
    }

    /// Opens the modal for `what`: a confirming one, a removal's progress
    /// or result, a remote's details, or an explanation.
    fn ask(&mut self, what: Asking) {
        self.open(what);
    }

    /// Opens `id`'s details, the nickname field showing its nickname.
    fn open_details(&mut self, id: RemoteId) {
        let Some(info) = self.remotes.iter().find(|r| r.id == id) else {
            return;
        };
        let state = DetailsState {
            draft: info.nickname.clone().unwrap_or_default(),
            ..DetailsState::default()
        };
        self.details = Some((id, state));
        self.nickname_sent = None;
        self.open(Asking::Details(id));
    }

    /// Closes the open modal at once, with nothing for the app.
    fn close_modal(&mut self) {
        if self.asking.take().is_some() || self.modals.depth() > 0 {
            self.modals.dismiss_all(DismissReason::App);
        }
        let _ = self.modals.take_events();
        self.modal_text = None;
        self.targets.clear();
        self.dirty = true;
    }

    /// (Re)builds `what`'s modal from the screen's state and shows it in
    /// place of any open one. Settings modals open and close at once, so
    /// building one again (a nickname typed, the fingerprint expanded, a
    /// new layout) shows no change but its own.
    fn open(&mut self, what: Asking) {
        let s = self.scale();
        let viewport = self.modal_viewport();
        let faces = self.faces.clone();
        let vendor = self.vendor.as_deref();
        let confirm = |c: Confirm| (c.dialog, c.cancel, c.action, c.text);
        let mut targets = Vec::new();
        let built = match what {
            Asking::RemoveRemote(id) => self.removal_question(id).map(|q| {
                let c = Confirm::new(
                    viewport,
                    &q,
                    copy::REMOVE_REMOTE_ACTION.text,
                    false,
                    s,
                    faces,
                );
                targets.push((Target::Remove, c.action_rect));
                confirm(c)
            }),
            Asking::Removing(_) => {
                let text = copy::REMOTE_REMOVING.fill(None, &[("name", &self.removing)]);
                Some(confirm(Confirm::notice(viewport, &text, None, s, faces)))
            }
            Asking::RemovedHere(_) => {
                let text = copy::REMOTE_REMOVED_HERE
                    .fill(None, &[("name", &self.removing), ("device", &self.device)]);
                Some(confirm(Confirm::notice(
                    viewport,
                    &text,
                    Some(copy::OK.text),
                    s,
                    faces,
                )))
            }
            Asking::Details(id) => {
                let info = self.remotes.iter().find(|r| r.id == id);
                let state = self.details.as_ref().filter(|(d, _)| *d == id);
                info.zip(state).map(|(info, (_, state))| {
                    let m = DetailsModal::new(viewport, info, state, s, faces);
                    targets.push((Target::Field, m.field));
                    targets.push((Target::Fingerprint, m.fingerprint));
                    (m.dialog, m.close, m.remove, m.text)
                })
            }
            Asking::Delete => Some(confirm(Confirm::new(
                viewport,
                &copy::DELETE_DATA.body(vendor),
                copy::DELETE_ACTION.text,
                true,
                s,
                faces,
            ))),
            Asking::Info(info) => {
                let m = self.info_modal(info);
                Some((m.dialog, m.ok, m.ok, m.text))
            }
        };
        self.close_modal();
        let Some((dialog, cancel, action, text)) = built else {
            return;
        };
        let id = what.id();
        match self.router.push_modal(id, dialog) {
            Ok(()) => {
                let _ = self.modals.take_events();
                self.asking = Some((what, cancel, action));
                self.modal_text = Some(text);
                self.targets = targets;
                self.dirty = true;
            }
            Err(e) => log::warn!("settings: the {id} modal: {e:?}"),
        }
    }

    /// The removal modal's text for `id` (plan 20261028_01, 3.11): what is
    /// deleted; then whether its daemon is told (over the open link, by
    /// one short connection, or not at all while the one in use is
    /// offline); then, for the one in use, what comes next.
    fn removal_question(&self, id: RemoteId) -> Option<String> {
        let r = self.remotes.iter().find(|r| r.id == id)?;
        let name = r.display_name.as_str();
        let device = self.device.as_str();
        let mut parts = vec![copy::REMOVE_REMOTE.fill(
            None,
            &[
                ("name", name),
                ("host", &r.host),
                ("fingerprint", &r.fingerprint),
            ],
        )];
        let told = match (r.active, r.online) {
            (true, true) => copy::REMOVE_REMOTE_TELL,
            (true, false) => copy::REMOVE_REMOTE_OFFLINE,
            (false, _) => copy::REMOVE_REMOTE_TRY,
        };
        parts.push(told.fill(None, &[("name", name), ("device", device)]));
        if r.active {
            parts.push(match next_remote(&self.remotes, id) {
                Some(next) => copy::REMOVE_REMOTE_NEXT.fill(None, &[("next", &next.display_name)]),
                None => copy::REMOVE_REMOTE_LAST.text.to_string(),
            });
        }
        Some(
            parts
                .iter()
                .map(|p| p.trim())
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    }

    /// The daemon's `[llm]` changed: *General agent* is shown as it is now.
    pub fn llm_changed(&mut self) {
        if self.nodes.is_some() {
            self.place();
        }
        self.invalidate();
    }

    /// Opens `info`'s explanation.
    fn explain(&mut self, info: Info) {
        self.open(Asking::Info(info));
    }

    /// `info`'s explanation modal.
    fn info_modal(&self, info: Info) -> InfoModal {
        let copy = info.copy();
        let vendor = self.vendor.as_deref();
        let mut body = copy.body(vendor);
        // Without the daemon's general agent: why, and where to turn it on.
        let llm = super::llm();
        if info == Info::GeneralAgent && !llm.general_agent {
            let reason = llm.reason.as_deref().unwrap_or("not given");
            let why = copy::GENERAL_AGENT_UNAVAILABLE.fill(vendor, &[("reason", reason)]);
            body = format!("{why}\n\n{body}");
        }
        // At 8 remotes: why the + is dim, first.
        if info == Info::Remotes && self.remotes.len() >= MAX_REMOTES {
            body = format!("{}\n\n{body}", copy::REMOTES_FULL.text.trim());
        }
        InfoModal::new(
            self.modal_viewport(),
            copy.label(),
            &body,
            self.scale(),
            self.faces.clone(),
        )
    }

    /// A tap in the open modal on one of its [`Target`]s.
    fn modal_target(&mut self, target: Target) -> Option<SettingsHit> {
        let (what, ..) = self.asking?;
        match (what, target) {
            (Asking::Details(id), Target::Field) => {
                if let Some((_, d)) = self.details.as_mut().filter(|(d, _)| *d == id)
                    && !d.editing
                {
                    d.editing = true;
                    self.open(what);
                }
                None
            }
            (Asking::Details(id), Target::Fingerprint) => {
                if let Some((_, d)) = self.details.as_mut().filter(|(d, _)| *d == id) {
                    d.fingerprint_open = !d.fingerprint_open;
                    self.open(what);
                }
                None
            }
            // *Remove*: the modal stays, saying so, until the client
            // answers ([`SettingsScreen::remote_removed`]).
            (Asking::RemoveRemote(id), Target::Remove) => {
                self.removing = self
                    .remotes
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| r.display_name.clone())
                    .unwrap_or_default();
                self.open(Asking::Removing(id));
                Some(SettingsHit::RemoveRemote(id))
            }
            _ => None,
        }
    }

    /// What the modal stack says happened to the open modal: its action
    /// button closed it, or anything else (*Cancel*, *Close*, a tap
    /// outside, Back). Details closing save a changed nickname; their
    /// *Remove* opens the X's confirm modal.
    fn settle_modal(&mut self) -> Option<SettingsHit> {
        let (what, _, action) = self.asking?;
        let by = self
            .modals
            .take_events()
            .into_iter()
            .find_map(|e| match e {
                ModalEvent::Dismissed { by, .. } if by != DismissReason::App => Some(by),
                _ => None,
            })?;
        self.asking = None;
        self.modal_text = None;
        self.targets.clear();
        self.dirty = true;
        let chosen = by == DismissReason::Control(action);
        match what {
            Asking::Details(id) => {
                if let Some((_, d)) = &mut self.details {
                    d.editing = false;
                }
                let hit = (!chosen).then(|| self.commit_nickname()).flatten();
                self.details = None;
                if chosen {
                    self.open(Asking::RemoveRemote(id));
                }
                hit
            }
            Asking::Delete if chosen => Some(SettingsHit::DeleteData),
            _ => None,
        }
    }

    /// What closing a modal by Back left for the app (a nickname to save).
    pub fn take_hit(&mut self) -> Option<SettingsHit> {
        self.pending.take()
    }

    /// Carries the pane's fling and the modal's transition on. Returns
    /// whether anything moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let dt = dt.as_secs_f32();
        self.watch_battery(dt);
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
        let dialog = stack.modal::<ModalDialog>(what.id())?;
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
                .filter_map(|id| c.get::<MarkdownView>(id).map(|v| v.text()))
                .collect::<Vec<_>>()
                .join(" ")
                .replace('\n', " ")
        })
        .unwrap_or_default()
    }

    /// The toggle row `label`, as it is now.
    #[cfg(test)]
    fn toggle(&self, label: &str) -> Option<ToggleRow> {
        let n = self.nodes.as_ref()?;
        [
            n.sound,
            n.ding,
            n.microphone,
            n.general_agent,
            n.tab_follows_focus,
            n.background,
            n.alerts,
        ]
        .into_iter()
        .filter_map(|id| self.router.get::<ToggleRow>(id))
        .find(|t| t.label == label)
        .cloned()
    }

    /// The centre of row `label`'s ⓘ, if it has one.
    #[cfg(test)]
    fn info_point(&self, label: &str) -> Option<[f32; 2]> {
        let n = self.nodes.as_ref()?;
        if let Some(t) = self.toggle(label) {
            return t.info.then(|| t.info_rect().center());
        }
        if let Some(o) = [n.voice, n.appearance]
            .into_iter()
            .filter_map(|id| self.router.get::<OptionRow>(id))
            .find(|o| o.label == label)
        {
            return o.info.then(|| o.info_rect().center());
        }
        [n.optimization, n.vendor]
            .into_iter()
            .filter(|&id| !self.router.flags(id).contains(NodeFlags::HIDDEN))
            .filter_map(|id| self.router.get::<LinkRow>(id))
            .find(|l| l.label == label && l.info)
            .map(|l| l.info_rect().center())
    }

    /// Where the open modal's first link is.
    #[cfg(test)]
    fn modal_link_point(&self) -> Option<[f32; 2]> {
        let text = self.modal_text?;
        self.dialog(|c| {
            c.get::<MarkdownView>(text)
                .and_then(|v| v.link_rect(0))
                .map(|r| r.center())
        })
        .flatten()
    }

    /// The theme changed: the rows, whose controls hold their colours,
    /// are made again (as for a new layout), and the screen drawn again.
    pub fn restyle(&mut self) {
        if self.nodes.is_some() {
            self.build();
        }
        self.dirty = true;
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
        fill(list, Rect::new(0.0, 0.0, l.w, l.h), theme().bg, 0.0);
        // The header first: the modal's dim goes over it.
        fill(
            list,
            Rect::new(0.0, 0.0, l.w, self.header_bottom()),
            theme().header,
            0.0,
        );
        let px = TITLE_PT * s;
        let back = Row {
            text: if term.is_some() { BACK_GLYPH } else { "<" }.to_string(),
            x: 0.0,
            y: 0.0,
            px: 1.6 * px,
            color: theme().fg,
            mono: true,
        };
        let back_y = l.top + (HEADER_H * s - term_h(back.px)) * 0.5;
        draw_row(list, ctx, term, &back, [PAD * s, back_y]);
        ctx.draw_text(
            list,
            copy::SETTINGS_TITLE.text,
            [BACK_W * s, l.top + (HEADER_H * s - line_h(px)) * 0.5],
            px,
            theme().fg,
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

    /// A host whose battery state cannot be known.
    struct NoBattery;

    impl Battery for NoBattery {
        fn state(&self) -> Option<BatteryState> {
            None
        }
        fn open_optimization(&self, _exempt: bool) {}
        fn open_vendor(&self) -> VendorPage {
            VendorPage::None
        }
    }

    /// A phone's battery, as libmorph reports it, with what was opened.
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

    /// A Background, counting what is asked.
    struct Service {
        ready: bool,
        asked: Rc<RefCell<usize>>,
    }

    impl Background for Service {
        fn ready(&self) -> bool {
            self.ready
        }
        fn request_notifications(&self) {
            *self.asked.borrow_mut() += 1;
        }
    }

    fn screen(android: bool, battery: Box<dyn Battery>) -> SettingsScreen {
        let mut s = SettingsScreen::new(Settings::default(), battery, android);
        s.set_layout(PHONE);
        s
    }

    fn plain() -> SettingsScreen {
        screen(false, Box::new(NoBattery))
    }

    fn tap(s: &mut SettingsScreen, x: f32, y: f32) -> Option<SettingsHit> {
        s.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        s.touch(1, TouchPhase::Ended, x, y, Duration::from_millis(16))
    }

    impl SettingsScreen {
        /// The *Enable* button's target in the row `label`, if it has one.
        fn enable_rect(&self, label: &str) -> Option<Rect> {
            let n = self.nodes.as_ref()?;
            [n.optimization, n.vendor]
                .into_iter()
                .filter_map(|id| self.router.get::<LinkRow>(id))
                .find(|l| l.label == label)
                .and_then(|l| {
                    // From the row's place in the pane to its place on screen.
                    let a = l.action_target()?;
                    let r = self.row_rect(label)?;
                    Some(Rect::new(
                        a.x - l.rect.x + r.x,
                        a.y - l.rect.y + r.y,
                        a.w,
                        a.h,
                    ))
                })
        }
    }

    #[test]
    fn switching_to_light_leaves_no_dark_gray_colour_drawn() {
        use crate::theme::testing::{dark_gray_only, draw_context, use_theme};
        use_theme("dark_gray");
        let ctx = draw_context();
        let mut s = plain();
        let mut list = DrawList::new();
        assert!(s.draw(&mut list, &ctx, None));
        assert!(!dark_gray_only(&list).is_empty(), "drawn in Dark (gray)");

        use_theme("light_black");
        s.restyle();
        assert!(s.draw(&mut list, &ctx, None), "drawn again");
        assert_eq!(dark_gray_only(&list), Vec::<[f32; 4]>::new());
        use_theme("dark_gray");
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
                Section::Background,
                Section::Notifications,
                Section::Remotes,
                Section::Device
            ]
        );
        let s = screen(true, Box::new(NoBattery));
        let order: Vec<Section> = s.sections().into_iter().map(|(x, _)| x).collect();
        assert_eq!(order, Section::ALL);
        // With the state unknown, one note.
        assert_eq!(s.battery_notes(), [BATTERY_UPDATE]);
        assert!(s.row_rect("Battery optimization").is_none());
        // Rows are 52 pt, under the header, the danger rows last.
        let sound = s.row_rect("All sound").unwrap();
        assert_eq!(sound.h, ROW_H * 2.0);
        assert!(sound.y >= s.viewport().y);
        let delete = s.row_rect("Delete data").unwrap();
        let clear = s.row_rect("Clear cache (0 MB)").unwrap();
        assert!(clear.y < delete.y && delete.y > sound.y);
        // *Unpair this device* is gone: each remote's X replaces it (the
        // 2026-10-28 13:03 EDT amendment).
        assert!(s.row_rect("Unpair this device").is_none());
        assert!(!copy::DELETE_DATA.text.contains("Unpair"));
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

    // *Tab follows focus* (plan 20261027_04, step 6; plan 20261024_02, the
    // 2026-10-27 11:27 EDT amendment).

    #[test]
    fn tab_follows_focus_follows_the_general_agent_in_voice() {
        let s = plain();
        let mic = s.row_rect("Microphone").unwrap();
        let ga = s.row_rect("General agent (Gemini)").unwrap();
        let row = s
            .row_rect("Tab follows focus")
            .expect("a Tab follows focus row");
        assert_eq!(ga.y, mic.y + mic.h);
        assert_eq!(row.y, ga.y + ga.h, "right after General agent, in Voice");
        assert!(row.y < s.row_rect("Scrollbar").unwrap().y, "before Display");
        let t = s.toggle("Tab follows focus").unwrap();
        assert!(!t.on, "off by default");
        assert!(t.enabled && t.info);
    }

    #[test]
    fn tab_follows_focus_toggles_and_is_dim_without_the_general_agent() {
        let label = "Tab follows focus";
        let mut s = plain();
        assert_eq!(
            tap_row(&mut s, label),
            Some(SettingsHit::Changed(Settings {
                tab_follows_focus: true,
                ..Settings::default()
            }))
        );
        assert!(s.toggle(label).unwrap().on);
        // General agent off: dim, it takes no taps, and keeps its value.
        tap_row(&mut s, "General agent (Gemini)");
        let t = s.toggle(label).unwrap();
        assert!(!t.enabled && t.on);
        assert_eq!(tap_row(&mut s, label), None, "dim, it takes no taps");
        // Its ⓘ still opens.
        let [x, y] = s.info_point(label).unwrap();
        tap(&mut s, x, y);
        assert!(s.asking());
        assert!(s.back());
        // On again: live, and it toggles off.
        tap_row(&mut s, "General agent (Gemini)");
        assert!(s.toggle(label).unwrap().enabled);
        let Some(SettingsHit::Changed(now)) = tap_row(&mut s, label) else {
            panic!("tab follows focus toggles");
        };
        assert!(!now.tab_follows_focus && now.general_agent);
        // The daemon offering no general agent dims it too.
        crate::settings::set_llm(flankr_companion::wire::LlmInfo {
            general_agent: false,
            summaries: true,
            reason: Some("off".into()),
        });
        s.llm_changed();
        assert!(!s.toggle(label).unwrap().enabled);
        assert_eq!(tap_row(&mut s, label), None);
        crate::settings::set_llm(flankr_companion::wire::LlmInfo::available());
        s.llm_changed();
        assert!(s.toggle(label).unwrap().enabled);
    }

    #[test]
    fn tab_follows_focus_copy_names_herdr_and_its_tools() {
        let text = copy::TAB_FOLLOWS_FOCUS.text;
        assert_eq!(copy::TAB_FOLLOWS_FOCUS.label(), "Tab follows focus");
        for word in ["herdr", "`focus_project`", "`focus`", "desktop"] {
            assert!(text.contains(word), "{word}: {text}");
        }
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
        assert_eq!(s.scrollbar_shown().as_deref(), Some("None"));
        // A tap on the label is not an arrow.
        assert_eq!(tap(&mut s, r.x + 40.0, r.y + r.h * 0.5), None);
        // Kept across a new layout.
        s.set_layout(Layout { w: 1200.0, ..PHONE });
        assert_eq!(s.scrollbar_shown().as_deref(), Some("None"));
    }

    #[test]
    fn appearance_steps_and_dims_the_other_kinds_row() {
        let mut s = plain();
        let (right, left) = arrows(&s, "Appearance");
        let shown = |s: &SettingsScreen| {
            ["Appearance", "Light theme", "Dark theme"].map(|l| s.option_shown(l).unwrap())
        };
        assert_eq!(
            shown(&s),
            [
                ("System".to_string(), true),
                ("Light (black text)".to_string(), true),
                ("Dark (navy)".to_string(), true),
            ]
        );
        let mut seen = Vec::new();
        for _ in 0..3 {
            let Some(SettingsHit::Changed(now)) = tap(&mut s, right[0], right[1]) else {
                panic!("the right arrow steps");
            };
            seen.push((now.appearance, shown(&s).map(|(_, on)| on)));
        }
        use super::super::Appearance::*;
        assert_eq!(
            seen,
            [
                (Light, [true, true, false]),
                (Dark, [true, false, true]),
                (System, [true, true, true]),
            ]
        );
        let Some(SettingsHit::Changed(now)) = tap(&mut s, left[0], left[1]) else {
            panic!("the left arrow steps");
        };
        assert_eq!(now.appearance, Dark);
        // The inactive row is drawn dim, and its arrows do nothing.
        let ctx = crate::theme::testing::draw_context();
        let mut list = DrawList::new();
        s.draw(&mut list, &ctx, None);
        let light = s.row_rect("Light theme").unwrap();
        let drawn: Vec<f32> = list
            .instances
            .iter()
            .filter(|i| light.contains([i.position[0] + 1.0, i.position[1] + 1.0]))
            .map(|i| i.color[3])
            .collect();
        assert!(!drawn.is_empty());
        assert!(
            drawn
                .iter()
                .all(|&a| a <= crate::settings::DISABLED_ALPHA + 1e-6),
            "Light theme is dim with Always dark: {drawn:?}"
        );
        let (lr, _) = arrows(&s, "Light theme");
        assert_eq!(tap(&mut s, lr[0], lr[1]), None);
    }

    #[test]
    fn the_theme_rows_step_with_wrapping_and_apply_at_once() {
        use crate::settings::put_theme_in_force;
        use crate::theme::testing::{draw_context, use_theme};
        use_theme("dark_navy");
        let themes = theme::themes();
        let ctx = draw_context();
        let mut s = plain();
        for (label, dark) in [("Dark theme", true), ("Light theme", false)] {
            let names: Vec<String> = themes.of_kind(dark).map(|t| t.name.clone()).collect();
            let (right, left) = arrows(&s, label);
            let mut seen = Vec::new();
            for _ in 0..names.len() {
                let Some(SettingsHit::Changed(now)) = tap(&mut s, right[0], right[1]) else {
                    panic!("{label} steps");
                };
                seen.push(s.option_shown(label).unwrap().0);
                // As the app does with the hit: the theme of the kind in
                // force (System is dark) applies at once.
                let changed = put_theme_in_force(&now);
                assert_eq!(changed.is_some(), dark, "{label}");
                s.restyle();
                let mut list = DrawList::new();
                assert!(s.draw(&mut list, &ctx, None));
                assert_eq!(list.instances[0].color, now.theme(&themes).colors.bg);
            }
            let mut forward = names[1..].to_vec();
            forward.push(names[0].clone());
            assert_eq!(seen, forward, "{label}: wraps forward");
            let Some(SettingsHit::Changed(now)) = tap(&mut s, left[0], left[1]) else {
                panic!("{label} steps back");
            };
            assert_eq!(
                s.option_shown(label).unwrap().0,
                *names.last().unwrap(),
                "and back"
            );
            put_theme_in_force(&now);
        }
        // Always light: the light row's step applies.
        let (right, _) = arrows(&s, "Appearance");
        let Some(SettingsHit::Changed(now)) = tap(&mut s, right[0], right[1]) else {
            panic!("Appearance steps");
        };
        let t = put_theme_in_force(&now).expect("a light theme now");
        assert!(!t.dark);
        assert_eq!(t.name, *s.option_shown("Light theme").unwrap().0);
        assert_eq!(theme(), t.colors);
        use_theme("dark_navy");
    }

    #[test]
    fn the_appearance_and_themes_are_kept_across_a_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = plain();
        let mut last = None;
        // The theme rows first: with Always light, the dark row is inactive.
        for label in ["Light theme", "Dark theme", "Appearance"] {
            let (right, _) = arrows(&s, label);
            if let Some(SettingsHit::Changed(now)) = tap(&mut s, right[0], right[1]) {
                last = Some(now);
            }
        }
        let saved = last.expect("each row steps");
        saved.save(dir.path()).unwrap();
        let loaded = Settings::load(dir.path());
        assert_eq!(loaded, saved);
        assert_eq!(loaded.appearance, super::super::Appearance::Light);
        assert_eq!(loaded.light_theme.as_deref(), Some("light_paper"));
        assert_eq!(loaded.dark_theme.as_deref(), Some("dark_gray"));
        // A new screen with them shows them.
        let mut again = SettingsScreen::new(loaded, Box::new(NoBattery), false);
        again.set_layout(PHONE);
        assert_eq!(
            again.option_shown("Light theme").unwrap().0,
            "Light (paper)"
        );
        assert_eq!(
            again.option_shown("Dark theme"),
            Some(("Dark (gray)".to_string(), false)),
            "kept, and dim with Always light"
        );
    }

    #[test]
    fn each_modal_opens_and_cancels_and_its_action_confirms() {
        // *Delete data*'s; *Unpair this device*'s is gone, and a remote's X
        // has its own test below.
        let (row, hit) = ("Delete data", SettingsHit::DeleteData);
        {
            // Tall enough that Device, last, is in view unscrolled.
            let mut s = with_remotes(Vec::new());
            assert_eq!(tap_row(&mut s, row), None);
            assert!(s.asking(), "{row} asks first");
            s.tick(Duration::from_secs(1));
            // Under the modal (its text, at its centre), the rows take no
            // taps.
            let [cx, cy] = s.modal_viewport().center();
            assert_eq!(tap(&mut s, cx, cy), None);
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

    // The Remotes section (plan 20261028_01, step 5; plan 20261024_02, the
    // 2026-10-28 17:46, 17:48, 17:56 and 17:57 EDT amendments).

    /// A remote, paired `n` seconds after 28 Oct 2026 12:00 UTC.
    fn remote(n: u8, active: bool, online: bool) -> RemoteInfo {
        RemoteInfo {
            id: RemoteId([n; 8]),
            name: format!("m{n}"),
            host: format!("m{n}.tail1a2b3c.ts.net:7347"),
            fingerprint: format!("{n}{n}{n}{n} 12c4"),
            paired_at: Some(1_793_188_800 + u64::from(n)),
            active,
            online,
            fingerprint_hex: format!("{n}{n}{n}{n}12c4").repeat(8),
            fingerprint_words: "zoo zone youth wrist wrong yellow".into(),
            nickname: None,
            display_name: format!("m{n}"),
        }
    }

    /// A screen tall enough that the Remotes and Device sections are in
    /// view, with `remotes`.
    fn with_remotes(remotes: Vec<RemoteInfo>) -> SettingsScreen {
        let mut s = SettingsScreen::new(Settings::default(), Box::new(NoBattery), false);
        s.set_layout(Layout { h: 4000.0, ..PHONE });
        s.set_device("s23");
        s.set_remotes(remotes);
        s
    }

    impl SettingsScreen {
        /// The remote rows shown: name, status, second line and place.
        fn remote_rows(&self) -> Vec<(String, Option<&'static str>, String, Rect)> {
            let Some(n) = &self.nodes else {
                return Vec::new();
            };
            n.remotes
                .iter()
                .filter(|&&id| !self.router.flags(id).contains(NodeFlags::HIDDEN))
                .filter_map(|&id| self.router.get::<RemoteRow>(id))
                .map(|r| (r.name().to_string(), r.status(), r.detail(), r.rect))
                .collect()
        }

        /// The standard row shown with none paired, if it is.
        fn empty_row(&self) -> Option<(String, Rect)> {
            let n = self.nodes.as_ref()?;
            let id = n.remotes_empty;
            (!self.router.flags(id).contains(NodeFlags::HIDDEN))
                .then(|| self.router.get::<RemoteRow>(id))
                .flatten()
                .map(|r| (r.name().to_string(), r.rect))
        }

        /// The centre of remote row `i`'s X.
        fn remove_point(&self, i: usize) -> [f32; 2] {
            let n = self.nodes.as_ref().unwrap();
            self.router
                .get::<RemoteRow>(n.remotes[i])
                .unwrap()
                .remove_rect()
                .center()
        }

        /// The centre of remote row `i`, away from its X.
        fn row_point(&self, i: usize) -> [f32; 2] {
            let r = self.remote_rows()[i].3;
            [r.x + 40.0, r.y + r.h * 0.5]
        }

        /// The Remotes title's + and ⓘ, and whether the + is dim.
        fn remotes_title(&self) -> RemotesTitle {
            let n = self.nodes.as_ref().unwrap();
            self.router
                .get::<RemotesTitle>(n.remotes_title)
                .unwrap()
                .clone()
        }

        /// Every Markdown note shown in the Remotes section's place: there
        /// are none now.
        fn remotes_has_note(&self) -> bool {
            let Some(n) = &self.nodes else {
                return false;
            };
            let (top, bottom) = {
                let at = |x| {
                    self.sections()
                        .into_iter()
                        .find(|(t, _)| *t == x)
                        .map(|(_, y)| y)
                };
                (at(Section::Remotes).unwrap(), at(Section::Device).unwrap())
            };
            let pane = n.pane;
            self.router.children(Some(pane)).any(|id| {
                !self.router.flags(id).contains(NodeFlags::HIDDEN)
                    && self
                        .router
                        .get::<MarkdownView>(id)
                        .is_some_and(|v| v.rect().y > top && v.rect().y < bottom)
            })
        }

        /// The open modal's kind.
        fn open_modal(&self) -> Option<Asking> {
            self.asking.map(|(what, ..)| what)
        }

        /// Where the open modal's `target` is.
        fn target_point(&self, target: Target) -> [f32; 2] {
            self.targets
                .iter()
                .find(|(t, _)| *t == target)
                .map(|(_, r)| r.center())
                .unwrap_or_else(|| panic!("no {target:?}"))
        }

        /// The labels of the open modal's buttons.
        fn modal_buttons(&self) -> Vec<String> {
            self.dialog(|c| {
                c.children(None)
                    .filter_map(|id| c.get::<libcontrols::Button>(id))
                    .map(|b| b.text.clone())
                    .collect()
            })
            .unwrap_or_default()
        }

        /// The open details' nickname field.
        fn field(&self) -> Option<super::super::remote_details::NicknameField> {
            self.dialog(|c| {
                c.children(None)
                    .find_map(|id| c.get::<super::super::remote_details::NicknameField>(id))
                    .cloned()
            })
            .flatten()
        }
    }

    #[test]
    fn the_remotes_section_sits_between_battery_and_device_with_a_row_each() {
        let mut s = screen(true, Box::new(NoBattery));
        let order: Vec<Section> = s.sections().into_iter().map(|(x, _)| x).collect();
        let at = |x| order.iter().position(|&o| o == x).unwrap();
        assert_eq!(at(Section::Remotes), at(Section::Battery) + 1);
        assert_eq!(at(Section::Device), at(Section::Remotes) + 1);
        assert_eq!(Section::Remotes.title(), "REMOTES");

        s.set_remotes(vec![remote(1, true, false), remote(2, false, false)]);
        let rows = s.remote_rows();
        assert_eq!(rows.len(), 2, "one row per remote, the rest hidden");
        let (name, status, detail, rect) = &rows[0];
        assert_eq!(name, "m1");
        assert_eq!(*status, Some("in use · offline"));
        assert_eq!(
            detail,
            "m1.tail1a2b3c.ts.net:7347 · 1111 12c4 · paired 28 Oct 2026"
        );
        assert_eq!(rect.h, ROW_H * 2.0, "52 pt");
        assert_eq!(rows[1].1, None, "only the one in use says so");
        assert_eq!(rows[1].3.y, rect.y + rect.h, "in pairing order");
        assert!(s.empty_row().is_none());
        // Under the Remotes title, above Device.
        let title_y = |x| s.sections().into_iter().find(|(t, _)| *t == x).unwrap().1;
        assert!(rect.y > title_y(Section::Remotes));
        assert!(rows[1].3.y + rows[1].3.h <= title_y(Section::Device));
        // Online: "in use".
        s.set_remotes(vec![remote(1, true, true), remote(2, false, false)]);
        assert_eq!(s.remote_rows()[0].1, Some("in use"));
        // A row is drawn: its two lines and its X.
        let mut list = DrawList::new();
        assert!(s.draw(&mut list, &crate::theme::testing::draw_context(), None));
        assert!(!s.remotes_has_note(), "standard rows only");
    }

    #[test]
    fn a_rows_first_line_is_the_display_name() {
        let mut named = remote(2, false, false);
        named.nickname = Some("Office".into());
        named.display_name = "Office".into();
        let mut clash = remote(3, false, false);
        clash.display_name = "m3:7400".into();
        let s = with_remotes(vec![remote(1, true, true), named, clash]);
        let names: Vec<String> = s.remote_rows().into_iter().map(|r| r.0).collect();
        assert_eq!(names, ["m1", "Office", "m3:7400"]);
    }

    #[test]
    fn no_remote_is_one_standard_row_and_the_title_explains_remotes() {
        let mut s = with_remotes(Vec::new());
        assert!(s.remote_rows().is_empty());
        let (label, rect) = s.empty_row().expect("the empty row");
        assert_eq!(label, "No remotes paired");
        assert_eq!(rect.h, ROW_H * 2.0, "a standard 52 pt row");
        assert!(!s.remotes_has_note(), "no note under it");
        // It takes no taps.
        assert_eq!(tap(&mut s, rect.x + 40.0, rect.y + rect.h * 0.5), None);
        assert!(!s.asking());
        // The ⓘ beside the title opens the explanation.
        let [x, y] = s.remotes_title().info_rect().center();
        assert_eq!(tap(&mut s, x, y), None);
        assert!(s.asking());
        s.tick(Duration::from_secs(1));
        let text = s.modal_text();
        assert!(text.starts_with("Remotes"), "{text}");
        for word in [
            "`Noise_IK`",
            "BLAKE2s-256",
            "*in use*",
            "`remotes/<id>/state.redb`",
        ] {
            assert!(
                text.contains(word.trim_matches(['`', '*'])),
                "{word}: {text}"
            );
        }
        assert!(!text.contains("most this phone keeps"), "not full: {text}");
        assert!(s.back());
        // A remote paired: the empty row goes.
        s.set_remotes(vec![remote(1, true, true)]);
        assert!(s.empty_row().is_none());
        assert_eq!(s.remote_rows().len(), 1);
    }

    #[test]
    fn the_plus_opens_the_add_mode_and_at_eight_is_dim_and_says_why() {
        let mut s = with_remotes(vec![remote(1, true, true)]);
        let t = s.remotes_title();
        let add = t.add_rect();
        assert_eq!(add.w, 44.0 * 2.0, "a 44 pt target");
        assert!(
            add.x + add.w <= PHONE.w && add.x > PHONE.w * 0.8,
            "at the right"
        );
        assert!(!t.full);
        assert_eq!(
            tap(&mut s, add.center()[0], add.center()[1]),
            Some(SettingsHit::AddRemote)
        );
        assert!(!s.asking());
        // At 8: dim, and a tap on it, as on the ⓘ, says why.
        let eight: Vec<_> = (1..=8).map(|n| remote(n, n == 1, true)).collect();
        assert_eq!(eight.len(), MAX_REMOTES);
        s.set_remotes(eight);
        let t = s.remotes_title();
        assert!(t.full);
        assert!(!s.remotes_has_note(), "no note: the ⓘ says it");
        let [x, y] = t.add_rect().center();
        assert_eq!(tap(&mut s, x, y), None, "no add mode");
        assert_eq!(s.open_modal(), Some(Asking::Info(Info::Remotes)));
        let text = s.modal_text();
        assert!(
            text.starts_with("Remotes 8 remotes is the most this phone keeps."),
            "why, first: {text}"
        );
        assert!(s.back());
        let [x, y] = t.info_rect().center();
        tap(&mut s, x, y);
        assert!(s.modal_text().contains("8 remotes is the most"));
        assert!(s.back());
        // Drawn dim.
        let ctx = crate::theme::testing::draw_context();
        let alpha = |full| {
            let mut t = t.clone();
            t.full = full;
            let mut list = DrawList::new();
            libcontrols::DrawControl::draw(&t, &mut list, &ctx);
            list.instances.last().unwrap().color[3]
        };
        assert!(
            (alpha(true) - alpha(false) * crate::settings::DISABLED_ALPHA).abs() < 1e-6,
            "the + at 35%"
        );
    }

    #[test]
    fn a_remotes_x_asks_first_and_remove_shows_removing_in_the_modal() {
        let remotes = vec![
            remote(1, true, true),
            remote(2, false, false),
            remote(3, false, false),
        ];
        let mut s = with_remotes(remotes.clone());

        // Another remote: the revoke probe's paragraph, nothing on what is
        // next.
        let [x, y] = s.remove_point(1);
        assert_eq!(tap(&mut s, x, y), None);
        assert_eq!(s.open_modal(), Some(Asking::RemoveRemote(RemoteId([2; 8]))));
        let text = s.modal_text();
        assert!(
            text.contains("Remove m2 (m2.tail1a2b3c.ts.net:7347, 2222 12c4)"),
            "{text}"
        );
        assert!(text.contains("remotes/<id>/"), "{text}");
        assert!(text.contains("connects to m2 once"), "{text}");
        assert!(text.contains("(s23)"), "the auth.toml entry: {text}");
        assert!(
            !text.contains("next") && !text.contains("last remote"),
            "{text}"
        );
        assert_eq!(s.modal_buttons(), ["Cancel", "Remove"]);
        // Cancel: nothing.
        let (cancel, action) = s.modal_points();
        assert_eq!(tap(&mut s, cancel[0], cancel[1]), None);
        assert!(!s.asking());
        // Remove: the remote's id, and the modal stays, saying so.
        tap(&mut s, x, y);
        assert_eq!(
            tap(&mut s, action[0], action[1]),
            Some(SettingsHit::RemoveRemote(RemoteId([2; 8])))
        );
        assert_eq!(s.open_modal(), Some(Asking::Removing(RemoteId([2; 8]))));
        let text = s.modal_text();
        assert!(
            text.starts_with("Removing m2: asking it to forget this phone"),
            "{text}"
        );
        assert!(s.modal_buttons().is_empty(), "nothing to press meanwhile");
        // Another remote's answer leaves it.
        s.remote_removed(RemoteId([3; 8]), "m3", true);
        assert!(s.asking());
        // The daemon answered: it closes.
        s.remote_removed(RemoteId([2; 8]), "m2", true);
        assert!(!s.asking());
        assert!(!s.modals.contains(Asking::Removing(RemoteId([2; 8])).id()));

        // Not answered: "removed on this phone only" stays until closed.
        tap(&mut s, x, y);
        let (_, action) = s.modal_points();
        tap(&mut s, action[0], action[1]);
        s.set_remotes(vec![remotes[0].clone(), remotes[2].clone()]);
        assert_eq!(s.open_modal(), Some(Asking::Removing(RemoteId([2; 8]))));
        s.remote_removed(RemoteId([2; 8]), "m2", false);
        assert_eq!(s.open_modal(), Some(Asking::RemovedHere(RemoteId([2; 8]))));
        let text = s.modal_text();
        assert!(text.contains("Removed m2 on this phone only"), "{text}");
        assert!(text.contains("`s23`".trim_matches('`')), "{text}");
        s.tick(Duration::from_secs(5));
        assert!(s.asking(), "it stays");
        assert_eq!(s.modal_buttons(), ["OK"]);
        let (ok, _) = s.modal_points();
        assert_eq!(tap(&mut s, ok[0], ok[1]), None);
        assert!(!s.asking(), "closed by OK");
        // A result with no modal open (as for Delete data) does nothing.
        s.remote_removed(RemoteId([3; 8]), "m3", false);
        assert!(!s.asking());
        assert!(!s.remotes_has_note());

        // The one in use, online: told over the open link; the most
        // recently paired of the rest is next.
        let mut s = with_remotes(remotes.clone());
        let [x, y] = s.remove_point(0);
        tap(&mut s, x, y);
        let text = s.modal_text();
        assert!(text.contains("over the open link"), "{text}");
        assert!(text.contains("connects to m3 next"), "{text}");
        assert!(s.back(), "Back cancels");

        // The one in use, offline: it won't be told, and the entry left
        // there is named, with where to delete it.
        let mut offline = remotes.clone();
        offline[0].online = false;
        s.set_remotes(offline);
        tap(&mut s, x, y);
        let text = s.modal_text();
        assert!(
            text.contains("m1 is offline, so it won't be told."),
            "{text}"
        );
        assert!(text.contains("~/.config/flankr/auth.toml"), "{text}");
        assert!(text.contains("(s23)"), "{text}");
        assert!(!text.contains("over the open link"), "{text}");
        let (_, action) = s.modal_points();
        assert_eq!(
            tap(&mut s, action[0], action[1]),
            Some(SettingsHit::RemoveRemote(RemoteId([1; 8])))
        );

        // The last one: back to the pairing screen.
        let mut s = with_remotes(vec![remote(1, true, true)]);
        let [x, y] = s.remove_point(0);
        tap(&mut s, x, y);
        let text = s.modal_text();
        assert!(text.contains("the last remote"), "{text}");
    }

    #[test]
    fn a_rows_tap_opens_its_details_and_close_closes_them() {
        let mut s = with_remotes(vec![remote(1, true, true), remote(2, false, false)]);
        let [x, y] = s.row_point(1);
        assert_eq!(tap(&mut s, x, y), None);
        assert_eq!(s.open_modal(), Some(Asking::Details(RemoteId([2; 8]))));
        let text = s.modal_text();
        for part in [
            "m2",
            "not in use",
            "Nickname",
            "Hostname",
            "m2 at m2.tail1a2b3c.ts.net:7347",
            "Paired 28 Oct 2026 at ",
        ] {
            assert!(text.contains(part), "{part}: {text}");
        }
        // The fingerprint is a collapsed "Fingerprint" section.
        let row = s
            .dialog(|c| {
                c.children(None)
                    .find_map(|id| c.get::<super::super::remote_details::Disclosure>(id))
                    .cloned()
            })
            .flatten()
            .expect("the Fingerprint row");
        assert_eq!(row.label, "Fingerprint");
        assert!(!row.open);
        assert!(!text.contains("zoo zone youth"), "{text}");
        assert!(!text.contains("2222 12c4 2222 12c4"), "{text}");
        assert_eq!(s.modal_buttons(), ["Close", "Remove"]);
        // The field shows the default, as there is no nickname.
        let f = s.field().expect("the nickname field");
        assert_eq!(f.text, "");
        assert_eq!(f.placeholder, "m2 (default)");
        assert!(!f.editing);
        // Close: nothing to save.
        let (close, _) = s.modal_points();
        assert_eq!(tap(&mut s, close[0], close[1]), None);
        assert!(!s.asking());
        // The one in use says so; one paired before the list says that.
        let mut old = remote(1, true, false);
        old.paired_at = None;
        s.set_remotes(vec![old, remote(2, false, false)]);
        let [x, y] = s.row_point(0);
        tap(&mut s, x, y);
        let text = s.modal_text();
        assert!(text.contains("in use · offline"), "{text}");
        assert!(text.contains("Paired before this list"), "{text}");
        // Back closes them too.
        assert!(s.back());
        assert!(!s.asking());
        assert_eq!(s.take_hit(), None);
    }

    #[test]
    fn the_fingerprint_expands_to_the_words_and_the_hex_in_the_mono_face() {
        let mut s = with_remotes(vec![remote(1, true, true)]);
        let [x, y] = s.row_point(0);
        tap(&mut s, x, y);
        let [fx, fy] = s.target_point(Target::Fingerprint);
        assert_eq!(tap(&mut s, fx, fy), None);
        assert!(s.asking(), "still open");
        let text = s.modal_text();
        assert!(text.contains("zoo zone youth wrist wrong yellow"), "{text}");
        assert!(text.contains("1111 12c4 1111 12c4"), "{text}");
        assert!(text.contains("BLAKE2s-256 of its key"), "{text}");
        // In code spans: the terminal face.
        let sources: Vec<String> = s
            .dialog(|c| {
                c.children(None)
                    .filter_map(|id| c.get::<MarkdownView>(id).map(|v| v.source().to_string()))
                    .collect()
            })
            .unwrap();
        let hex = sources
            .iter()
            .find(|m| m.contains("12c4"))
            .expect("the hex");
        assert_eq!(hex.matches("`1111 12c4 1111 12c4`").count(), 4, "{hex}");
        // Collapsed again.
        let [fx, fy] = s.target_point(Target::Fingerprint);
        tap(&mut s, fx, fy);
        assert!(!s.modal_text().contains("zoo zone"));
    }

    #[test]
    fn the_nickname_is_typed_in_the_details_and_saved() {
        let mut s = with_remotes(vec![remote(1, true, true), remote(2, false, false)]);
        let [x, y] = s.row_point(1);
        tap(&mut s, x, y);
        assert_eq!(s.nickname_editing(), None);
        // A tap on the field: typing, and the keyboard is for it.
        let [fx, fy] = s.target_point(Target::Field);
        assert_eq!(tap(&mut s, fx, fy), None);
        assert_eq!(s.nickname_editing(), Some(""));
        assert!(s.field().unwrap().editing);
        // The soft keyboard's text, one line.
        s.set_nickname_text("Off\nice");
        assert_eq!(s.nickname_editing(), Some("Office"));
        assert_eq!(s.field().unwrap().text, "Office");
        // A new layout (the keyboard coming up) keeps the modal and draft.
        s.set_layout(Layout {
            h: 4000.0,
            bottom: 900.0,
            ..PHONE
        });
        assert_eq!(s.open_modal(), Some(Asking::Details(RemoteId([2; 8]))));
        assert_eq!(s.nickname_editing(), Some("Office"));
        // The keyboard's Done: saved.
        assert_eq!(
            s.nickname_done(),
            Some(SettingsHit::SetNickname {
                id: RemoteId([2; 8]),
                nickname: Some("Office".into())
            })
        );
        assert_eq!(s.nickname_editing(), None);
        assert!(s.asking(), "the details stay");
        // Closing them does not ask again.
        let (close, _) = s.modal_points();
        assert_eq!(tap(&mut s, close[0], close[1]), None);

        // The client's rows come back with it: shown, and in the field.
        let mut named = remote(2, false, false);
        named.nickname = Some("Office".into());
        named.display_name = "Office".into();
        s.set_remotes(vec![remote(1, true, true), named]);
        assert_eq!(s.remote_rows()[1].0, "Office");
        let [x, y] = s.row_point(1);
        tap(&mut s, x, y);
        assert!(s.modal_text().contains("Office"));
        let f = s.field().unwrap();
        assert_eq!(f.text, "Office");
        assert_eq!(f.placeholder, "m2 (default)");

        // Cleared with a physical keyboard, then closed by a tap outside:
        // back to the default.
        let [fx, fy] = s.target_point(Target::Field);
        tap(&mut s, fx, fy);
        for _ in 0..6 {
            assert_eq!(s.nickname_key(Key::Character('\u{8}')), Typed::Edited);
        }
        assert_eq!(s.nickname_editing(), Some(""));
        assert_eq!(
            s.nickname_key(Key::Escape),
            Typed::Done(Some(SettingsHit::SetNickname {
                id: RemoteId([2; 8]),
                nickname: None
            }))
        );
        // Typed again, then the details closed by Back: saved then.
        let [fx, fy] = s.target_point(Target::Field);
        tap(&mut s, fx, fy);
        assert_eq!(s.nickname_key(Key::Character('M')), Typed::Edited);
        s.nickname_dismissed();
        assert_eq!(s.nickname_editing(), None, "the keyboard put away");
        assert_eq!(s.nickname_key(Key::Character('x')), Typed::No);
        assert!(s.back());
        assert_eq!(
            s.take_hit(),
            Some(SettingsHit::SetNickname {
                id: RemoteId([2; 8]),
                nickname: Some("M".into())
            })
        );
        // Spaces only: the default.
        let [x, y] = s.row_point(1);
        tap(&mut s, x, y);
        let [fx, fy] = s.target_point(Target::Field);
        tap(&mut s, fx, fy);
        s.set_nickname_text("   ");
        assert_eq!(
            s.nickname_done(),
            Some(SettingsHit::SetNickname {
                id: RemoteId([2; 8]),
                nickname: None
            })
        );
    }

    #[test]
    fn remove_in_the_details_opens_the_xs_confirm_modal() {
        let mut s = with_remotes(vec![remote(1, true, true), remote(2, false, false)]);
        let [x, y] = s.row_point(1);
        tap(&mut s, x, y);
        let (_, remove) = s.modal_points();
        assert_eq!(tap(&mut s, remove[0], remove[1]), None);
        assert_eq!(s.open_modal(), Some(Asking::RemoveRemote(RemoteId([2; 8]))));
        assert!(s.modal_text().contains("Remove m2"));
        let (_, action) = s.modal_points();
        assert_eq!(
            tap(&mut s, action[0], action[1]),
            Some(SettingsHit::RemoveRemote(RemoteId([2; 8])))
        );
        assert_eq!(s.open_modal(), Some(Asking::Removing(RemoteId([2; 8]))));
        // The details of a remote that goes close with it.
        s.remote_removed(RemoteId([2; 8]), "m2", true);
        let [x, y] = s.row_point(1);
        tap(&mut s, x, y);
        s.set_remotes(vec![remote(1, true, true)]);
        assert!(!s.asking());
    }

    #[test]
    fn the_next_remote_is_the_most_recently_paired_of_the_rest() {
        let mut list = vec![
            remote(1, true, true),
            remote(3, false, false),
            remote(2, false, false),
        ];
        assert_eq!(next_remote(&list, RemoteId([1; 8])).unwrap().name, "m3");
        // Undated ones go by list order, after dated ones.
        list[1].paired_at = None;
        assert_eq!(next_remote(&list, RemoteId([1; 8])).unwrap().name, "m2");
        assert!(next_remote(&list[..1], RemoteId([1; 8])).is_none());
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
        // Optimized: an *Enable* button on the right asks the system; the
        // row itself opens the list of every app's setting.
        let enable = s
            .enable_rect("Battery optimization")
            .expect("an Enable button");
        let row = s.row_rect("Battery optimization").unwrap();
        assert!(enable.x > row.x + row.w * 0.6, "on the right");
        assert_eq!(
            tap(&mut s, enable.x + enable.w * 0.5, enable.y + enable.h * 0.5),
            None
        );
        assert_eq!(tap_row(&mut s, "Battery optimization"), None);
        assert_eq!(
            opened.borrow().as_slice(),
            ["optimization false", "optimization true"]
        );
        // No vendor page after all: the row goes, for good.
        assert!(s.row_rect("Samsung battery (app sleeping)").is_some());
        tap_row(&mut s, "Samsung battery (app sleeping)");
        assert!(s.row_rect("Samsung battery (app sleeping)").is_none());
        s.set_layout(Layout { w: 1200.0, ..PHONE });
        assert!(s.row_rect("Samsung battery (app sleeping)").is_none());

        let s = screen(true, Box::new(phone(true, false, VendorPage::Opened)));
        assert!(s.battery_notes().is_empty());
        // Exempt: no button, the "›" as before.
        assert!(s.enable_rect("Battery optimization").is_none());
    }

    #[test]
    fn a_drag_scrolls_and_is_not_a_tap() {
        let mut s = SettingsScreen::new(Settings::default(), Box::new(NoBattery), false);
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
    fn clear_cache_shows_its_size_and_clears_with_no_confirmation() {
        assert_eq!(clear_cache_label(0), "Clear cache (0 MB)");
        assert_eq!(clear_cache_label(2_621_440), "Clear cache (2.5 MB)");
        assert_eq!(clear_cache_label(15 * 1024 * 1024), "Clear cache (15 MB)");
        let mut s = plain();
        // Empty: shown, and does nothing.
        let empty = s.row_rect("Clear cache (0 MB)").unwrap();
        let delete = s.row_rect("Delete data").unwrap();
        assert!(empty.y < delete.y, "before Delete data, in Device");
        assert_eq!(tap_row(&mut s, "Clear cache (0 MB)"), None);
        assert!(!s.asking());
        s.set_cache(2_621_440);
        assert!(s.row_rect("Clear cache (0 MB)").is_none());
        assert_eq!(
            tap_row(&mut s, "Clear cache (2.5 MB)"),
            Some(SettingsHit::ClearCache)
        );
        assert!(!s.asking(), "no confirmation");
        // Kept across a new layout.
        s.set_layout(Layout { w: 1200.0, ..PHONE });
        assert!(s.row_rect("Clear cache (2.5 MB)").is_some());
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

    const BACKGROUND: &str = "Stay connected in the background";
    const ALERTS: &str = "Alert when an agent needs you";

    #[test]
    fn background_and_notifications_sit_after_display_with_their_defaults() {
        let s = screen(true, Box::new(NoBattery));
        let order: Vec<Section> = s.sections().into_iter().map(|(x, _)| x).collect();
        let at = |x| order.iter().position(|&o| o == x).unwrap();
        assert_eq!(at(Section::Background), at(Section::Display) + 1);
        assert_eq!(at(Section::Notifications), at(Section::Background) + 1);
        assert_eq!(at(Section::Battery), at(Section::Notifications) + 1);
        let background = s.toggle(BACKGROUND).expect("the background row");
        let alerts = s.toggle(ALERTS).expect("the alerts row");
        assert!(background.on && background.enabled && background.info);
        assert!(!alerts.on && alerts.enabled && alerts.info);
        assert!(s.row_rect(BACKGROUND).unwrap().y < s.row_rect(ALERTS).unwrap().y);
        // On, with the service there: no note.
        assert!(s.background_notes().is_empty());
    }

    #[test]
    fn without_the_service_the_sections_say_they_need_an_update() {
        let mut s = SettingsScreen::new(Settings::default(), Box::new(NoBattery), true)
            .with_background(Box::new(Service {
                ready: false,
                asked: Rc::new(RefCell::new(0)),
            }));
        s.set_layout(PHONE);
        assert_eq!(s.background_notes(), [copy::NEEDS_UPDATE.text]);
        tap_row(&mut s, ALERTS);
        assert_eq!(
            s.background_notes(),
            [copy::NEEDS_UPDATE.text, copy::NEEDS_UPDATE.text]
        );
    }

    #[test]
    fn the_alerts_toggle_follows_settings_given_to_it() {
        let mut s = plain();
        tap_row(&mut s, ALERTS);
        assert!(s.toggle(ALERTS).unwrap().on);
        // Refused, or revoked while away: the app turns it off.
        s.set_settings(Settings::default());
        assert!(!s.toggle(ALERTS).unwrap().on);
        assert_eq!(s.settings(), Settings::default());
    }

    #[test]
    fn the_alerts_row_is_dim_while_background_is_off() {
        let mut s = plain();
        let Some(SettingsHit::Changed(now)) = tap_row(&mut s, BACKGROUND) else {
            panic!("background toggles");
        };
        assert!(!now.background && !now.alerts);
        let alerts = s.toggle(ALERTS).unwrap();
        assert!(!alerts.enabled, "dim");
        assert_eq!(s.background_notes(), [copy::ALERTS_NEEDS_BACKGROUND.text]);
        assert_eq!(tap_row(&mut s, ALERTS), None, "it takes no taps");
        // Its ⓘ still opens.
        let [x, y] = s.info_point(ALERTS).unwrap();
        assert_eq!(tap(&mut s, x, y), None);
        assert!(s.asking());
        assert!(s.back());
        // Background on again: the row is live, with no note.
        tap_row(&mut s, BACKGROUND);
        assert!(s.toggle(ALERTS).unwrap().enabled);
        assert!(s.background_notes().is_empty());
    }

    #[test]
    fn nothing_is_asked_at_start_and_alerts_ask_once_when_turned_on() {
        let asked = Rc::new(RefCell::new(0));
        let opened = Rc::new(RefCell::new(Vec::new()));
        let phone = Phone {
            state: BatteryState {
                ignoring_optimizations: false,
                background_restricted: false,
                manufacturer: "samsung".into(),
            },
            vendor: VendorPage::Opened,
            opened: Rc::clone(&opened),
        };
        let mut s = SettingsScreen::new(Settings::default(), Box::new(phone), true)
            .with_background(Box::new(Service {
                ready: true,
                asked: Rc::clone(&asked),
            }));
        s.set_layout(PHONE);
        s.tick(Duration::from_millis(16));
        let mut list = DrawList::new();
        s.draw(&mut list, &DrawContext::default(), None);
        assert_eq!(*asked.borrow(), 0, "no permission is asked at start");
        assert!(opened.borrow().is_empty(), "no battery page opens at start");
        // Ready: no notes.
        assert!(s.background_notes().is_empty());
        let Some(SettingsHit::Changed(now)) = tap_row(&mut s, ALERTS) else {
            panic!("alerts toggle");
        };
        assert!(now.alerts);
        assert_eq!(*asked.borrow(), 1, "asked once, on turning it on");
        tap_row(&mut s, ALERTS);
        assert_eq!(*asked.borrow(), 1, "not on turning it off");
        // An ⓘ opens nothing but its modal.
        let [x, y] = s.info_point("Battery optimization").unwrap();
        tap(&mut s, x, y);
        assert!(s.asking());
        assert!(opened.borrow().is_empty());
    }

    #[test]
    fn without_the_daemons_general_agent_its_toggle_is_dim_and_its_info_says_why() {
        use flankr_companion::wire::LlmInfo;
        let label = "General agent (Gemini)";
        let mut s = plain();
        // Offered: the toggle flips.
        assert!(matches!(
            tap_row(&mut s, label),
            Some(SettingsHit::Changed(_))
        ));
        tap_row(&mut s, label);
        // The daemon stops offering it, while the screen is open.
        crate::settings::set_llm(LlmInfo {
            general_agent: false,
            summaries: true,
            reason: Some("[llm.uses.general_agent] enabled = false".into()),
        });
        s.llm_changed();
        assert_eq!(tap_row(&mut s, label), None, "dim, it takes no taps");
        let [x, y] = s.info_point(label).unwrap();
        tap(&mut s, x, y);
        assert!(s.asking());
        s.tick(Duration::from_secs(1));
        let text = s.modal_text();
        assert!(
            text.contains("[llm.uses.general_agent] enabled = false"),
            "{text}"
        );
        assert!(
            text.contains("[llm]") && text.contains("config.toml"),
            "{text}"
        );
        let (ok, _) = s.modal_points();
        tap(&mut s, ok[0], ok[1]);
        // Offered again: live again, and the ⓘ has no reason.
        crate::settings::set_llm(LlmInfo::available());
        s.llm_changed();
        assert!(matches!(
            tap_row(&mut s, label),
            Some(SettingsHit::Changed(_))
        ));
        tap(&mut s, x, y);
        s.tick(Duration::from_secs(1));
        assert!(!s.modal_text().contains("offers no general agent"));
    }

    #[test]
    fn each_info_opens_its_modal_which_closes_at_once() {
        let phone = || Phone {
            state: BatteryState {
                ignoring_optimizations: true,
                background_restricted: false,
                manufacturer: "samsung".into(),
            },
            vendor: VendorPage::Opened,
            opened: Rc::new(RefCell::new(Vec::new())),
        };
        for (label, info) in [
            ("Voice", Info::Voice),
            ("General agent (Gemini)", Info::GeneralAgent),
            ("Tab follows focus", Info::TabFollowsFocus),
            (BACKGROUND, Info::Background),
            (ALERTS, Info::Alerts),
            ("Battery optimization", Info::Optimization),
            ("Samsung battery (app sleeping)", Info::Vendor),
            ("Appearance", Info::Appearance),
        ] {
            let mut s = screen(true, Box::new(phone()));
            let before = s.settings();
            let [x, y] = s.info_point(label).unwrap_or_else(|| panic!("{label}'s ⓘ"));
            assert_eq!(tap(&mut s, x, y), None);
            assert!(s.asking(), "{label}'s ⓘ opens its modal");
            // Its 0.18 s opening.
            s.tick(Duration::from_secs(1));
            assert_eq!(s.settings(), before, "{label}: the ⓘ is not the toggle");
            let text = s.modal_text();
            assert!(text.starts_with(label), "{text}");
            let first = info.copy().text.split_whitespace().next().unwrap();
            assert!(text.contains(first), "{text}");
            // OK closes it at once: the same frame, with no fade.
            let (ok, _) = s.modal_points();
            assert_eq!(tap(&mut s, ok[0], ok[1]), None);
            assert!(!s.asking(), "{label}: OK closes it");
            assert!(
                !s.modals.contains(Asking::Info(info).id()),
                "{label}: at once"
            );
            // A tap outside it closes it too.
            tap(&mut s, x, y);
            assert!(s.asking());
            s.tick(Duration::from_secs(1));
            let top = s.viewport().y + 5.0;
            tap(&mut s, 5.0, top);
            assert!(!s.asking(), "{label}: a tap outside closes it");
            assert!(!s.modals.contains(Asking::Info(info).id()));
        }
    }

    #[test]
    fn a_link_in_an_explanation_is_handed_to_the_app_once() {
        let mut s = plain();
        let [x, y] = s.info_point(BACKGROUND).unwrap();
        tap(&mut s, x, y);
        s.tick(Duration::from_secs(1));
        assert!(s.take_links().is_empty());
        let at = s.modal_link_point().expect("the explanation has a link");
        assert_eq!(tap(&mut s, at[0], at[1]), None);
        assert_eq!(
            s.take_links(),
            ["https://developer.android.com/develop/background-work/services/fgs"]
        );
        assert!(s.take_links().is_empty(), "once");
        assert!(s.asking(), "the modal stays open");
    }

    #[test]
    fn the_battery_rows_follow_the_state_when_it_is_read_again() {
        let state = Rc::new(RefCell::new(BatteryState {
            ignoring_optimizations: false,
            background_restricted: true,
            manufacturer: "samsung".into(),
        }));
        struct Changing(Rc<RefCell<BatteryState>>);
        impl Battery for Changing {
            fn state(&self) -> Option<BatteryState> {
                Some(self.0.borrow().clone())
            }
            fn open_optimization(&self, _exempt: bool) {}
            fn open_vendor(&self) -> VendorPage {
                VendorPage::Opened
            }
        }
        let mut s = screen(true, Box::new(Changing(Rc::clone(&state))));
        assert_eq!(s.battery_notes(), [BATTERY_RESTRICTED]);
        let subtitle = |s: &SettingsScreen| {
            let n = s.nodes.as_ref().unwrap();
            s.router
                .get::<LinkRow>(n.optimization)
                .and_then(|l| l.subtitle.clone())
        };
        assert_eq!(
            subtitle(&s).as_deref(),
            copy::BATTERY_OPTIMIZATION.status(false)
        );
        // Back from Android's dialog and the battery page.
        *state.borrow_mut() = BatteryState {
            ignoring_optimizations: true,
            background_restricted: false,
            manufacturer: "samsung".into(),
        };
        s.refresh_battery();
        assert!(s.battery_notes().is_empty());
        assert_eq!(
            subtitle(&s).as_deref(),
            copy::BATTERY_OPTIMIZATION.status(true)
        );
    }

    #[test]
    fn after_enable_the_exemption_is_seen_without_a_resume() {
        let state = Rc::new(RefCell::new(BatteryState {
            ignoring_optimizations: false,
            background_restricted: false,
            manufacturer: "samsung".into(),
        }));
        struct Changing(Rc<RefCell<BatteryState>>);
        impl Battery for Changing {
            fn state(&self) -> Option<BatteryState> {
                Some(self.0.borrow().clone())
            }
            fn open_optimization(&self, _exempt: bool) {}
            fn open_vendor(&self) -> VendorPage {
                VendorPage::Opened
            }
        }
        let mut s = screen(true, Box::new(Changing(Rc::clone(&state))));
        let enable = s.enable_rect("Battery optimization").unwrap();
        tap(&mut s, enable.x + enable.w * 0.5, enable.y + enable.h * 0.5);
        // The user allows it in Android's dialog, which takes no surface.
        state.borrow_mut().ignoring_optimizations = true;
        s.tick(Duration::from_millis(500));
        assert!(
            s.enable_rect("Battery optimization").is_some(),
            "not looked yet"
        );
        s.tick(Duration::from_millis(600));
        assert!(
            s.enable_rect("Battery optimization").is_none(),
            "seen, button gone"
        );
        assert!(s.battery_watch.is_none(), "and the watch ends");
    }

    #[test]
    fn the_vendor_addition_shows_on_that_vendors_phones_only() {
        for (info, freecess) in [
            (None, false),
            (
                Some(BuildInfo {
                    vendor: "google".into(),
                }),
                false,
            ),
            (
                Some(BuildInfo {
                    vendor: "samsung".into(),
                }),
                true,
            ),
        ] {
            let mut s = SettingsScreen::new(Settings::default(), Box::new(NoBattery), false)
                .with_build_info(info);
            s.set_layout(PHONE);
            let [x, y] = s.info_point(BACKGROUND).unwrap();
            tap(&mut s, x, y);
            let text = s.modal_text();
            assert!(
                text.contains("cached apps freezer"),
                "the base text: {text}"
            );
            assert_eq!(text.contains("Freecess"), freecess, "{text}");
        }
    }

    fn voices() -> Option<Vec<String>> {
        Some(
            ["marius", "alba", "cosette"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
    }

    /// The centres of row `label`'s right and left arrows.
    fn arrows(s: &SettingsScreen, label: &str) -> ([f32; 2], [f32; 2]) {
        let r = s.row_rect(label).unwrap();
        (
            [r.x + r.w - PAD * 2.0 - 20.0, r.y + r.h * 0.5],
            [
                r.x + r.w - PAD * 2.0 - OptionRow::ARROWS_W * 2.0 + 20.0,
                r.y + r.h * 0.5,
            ],
        )
    }

    fn voice_row(s: &SettingsScreen) -> OptionRow {
        let n = s.nodes.as_ref().unwrap();
        s.router.get::<OptionRow>(n.voice).unwrap().clone()
    }

    #[test]
    fn the_voice_row_follows_ding_and_is_inactive_before_load_and_without_sound() {
        let mut s = plain();
        let ding = s.row_rect("Ding when an agent is done").unwrap();
        let row = s.row_rect("Voice").expect("a Voice row");
        assert_eq!(row.y, ding.y + ding.h, "right after Ding, in Sound");
        assert!(row.y < s.row_rect("Microphone").unwrap().y);
        assert_eq!(s.voice_shown().as_deref(), Some("Default"));
        // Phonon has not loaded: dim, and the arrows do nothing.
        assert!(!voice_row(&s).enabled);
        let (right, left) = arrows(&s, "Voice");
        assert_eq!(tap(&mut s, right[0], right[1]), None);
        assert_eq!(tap(&mut s, left[0], left[1]), None);
        s.set_voices(voices());
        assert!(voice_row(&s).enabled);
        // All sound off: dim again, and no step.
        tap_row(&mut s, "All sound");
        assert!(!voice_row(&s).enabled);
        assert_eq!(tap(&mut s, right[0], right[1]), None);
        assert_eq!(s.voice_shown().as_deref(), Some("Default"));
        // Its ⓘ still opens.
        let [x, y] = s.info_point("Voice").unwrap();
        tap(&mut s, x, y);
        assert!(s.asking());
        assert!(s.back());
        tap_row(&mut s, "All sound");
        assert!(voice_row(&s).enabled);
    }

    #[test]
    fn the_voice_arrows_step_from_default_both_ways_and_wrap_each_asking_a_preview() {
        let mut s = plain().with_voices(voices());
        s.set_layout(PHONE);
        let (right, left) = arrows(&s, "Voice");
        let mut seen = Vec::new();
        for _ in 0..4 {
            let Some(SettingsHit::Voice(now)) = tap(&mut s, right[0], right[1]) else {
                panic!("each step is a preview");
            };
            seen.push(voice::label(now.voice.as_deref()).to_string());
            assert_eq!(
                s.voice_shown().as_deref(),
                Some(seen.last().unwrap().as_str())
            );
        }
        assert_eq!(
            seen,
            ["alba", "cosette", "marius", "Default"],
            "Default first, wrapping"
        );
        let Some(SettingsHit::Voice(now)) = tap(&mut s, left[0], left[1]) else {
            panic!("the left arrow steps");
        };
        assert_eq!(
            now.voice.as_deref(),
            Some("marius"),
            "left from Default wraps"
        );
        assert_eq!(s.settings().voice.as_deref(), Some("marius"));
        // A tap on the label is not an arrow.
        let r = s.row_rect("Voice").unwrap();
        assert_eq!(tap(&mut s, r.x + 200.0, r.y + r.h * 0.5), None);
        // Kept across a new layout.
        s.set_layout(Layout { w: 1200.0, ..PHONE });
        assert_eq!(s.voice_shown().as_deref(), Some("marius"));
    }

    #[test]
    fn a_saved_voice_no_longer_installed_shows_as_default_and_is_kept() {
        let saved = Settings {
            voice: Some("javert".into()),
            ..Settings::default()
        };
        let mut s =
            SettingsScreen::new(saved.clone(), Box::new(NoBattery), false).with_voices(voices());
        s.set_layout(PHONE);
        assert_eq!(s.voice_shown().as_deref(), Some("Default"));
        assert_eq!(s.settings(), saved, "kept until another is chosen");
        // Another toggle keeps it.
        let Some(SettingsHit::Changed(now)) = tap_row(&mut s, "Microphone") else {
            panic!("the microphone toggles");
        };
        assert_eq!(now.voice.as_deref(), Some("javert"));
        // It steps as Default.
        let (right, _) = arrows(&s, "Voice");
        let Some(SettingsHit::Voice(now)) = tap(&mut s, right[0], right[1]) else {
            panic!("a step");
        };
        assert_eq!(now.voice.as_deref(), Some("alba"));
    }

    #[test]
    fn the_copy_comes_from_settings_copy_md() {
        let s = plain();
        for label in [
            "All sound",
            "Ding when an agent is done",
            "Voice",
            "Microphone",
            "General agent (Gemini)",
            "Tab follows focus",
            "Scrollbar",
            "Delete data",
        ] {
            assert!(s.row_rect(label).is_some(), "{label}");
        }
        assert_eq!(copy::SOUND.label(), "All sound");
        assert_eq!(Section::Sound.title(), "SOUND");
        assert_eq!(BATTERY_UPDATE, "Battery settings need an app update");
    }
}
