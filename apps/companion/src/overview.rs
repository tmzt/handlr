//! The overview, the home screen (plan 20261023_04, as amended at 17:04
//! and 18:07 EDT). From the top: the title and the status lines; then the
//! agent cards ([`CardGrid`]), scrolling as one list; then the newest
//! Gemini line ([`TurnStrip`]) while there is one; then, above the bottom
//! row, the confirmation card while one is shown and the dictation card
//! while a message is taken; and the bottom row of the agent view
//! (decision 5): play at the left, record at the right, record becoming
//! send with a cancel in the middle while a message is taken.
//!
//! A tap on the strip slides the Gemini turns up over the cards
//! ([`TurnOverlay`]); a tap on the dimmed cards above it, a swipe down on
//! it, or Back ([`Overview::back`]) closes it. The bottom row, and the
//! confirmation and dictation cards over it, stay live while it is open.
//!
//! Tapping a card opens the agent view on that pane (decision 4). Play
//! reads replies from an agent view's pane only, so it is dim and does
//! nothing until a view has been opened; record takes a message for
//! herdr's focused agent. A tap on nothing is the main screen's tap.
//!
//! The gear ([`Gear`]) at the right of the title opens the settings
//! (plan 20261024_02). What they turn off is off here too (decision 4):
//! play without sound, record without the microphone or the general agent,
//! and the dictation card without the microphone are drawn dim and take no
//! taps, and without the general agent the strip and the overlay are
//! hidden.

pub mod agent_card;
pub mod card_grid;
pub mod gear;
pub mod panel_shader;
pub mod turn_log;
pub mod turn_overlay;
pub mod turn_strip;

use std::time::Duration;

use flankr_companion::PromptMode;
use flankr_companion::Speaker;
use libcontrols::{DrawContext, DrawControl, Rect, TextArea};
use libflankr_proto::common::AgentInfo;
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use crate::agent::{
    ACTION_H, AgentHit, BUTTON_H, Fill, Flow, FlowButton, PAD, Row, Terminal, columns, fill, line_h,
};
use crate::controls::scroll::Touched;
use crate::controls::{ControlRow, SquareButton};
use crate::settings;
use crate::theme::theme;
use crate::ui::{DictationPrompt, Layout, Line, wrap};
pub use card_grid::CardGrid;
pub use gear::Gear;
pub use panel_shader::{PanelInstance, PanelShader};
pub use turn_log::TurnLog;
pub use turn_overlay::TurnOverlay;
pub use turn_strip::TurnStrip;

/// What a touch on the overview did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverviewHit {
    /// A card: open the agent view on this pane.
    Agent(String),
    /// The bottom row, or a confirmation's Approve or Deny.
    Bottom(AgentHit),
    /// A tap on no control.
    Tap,
    /// The gear: open the settings.
    Settings,
}

/// A card above the bottom row: its lines and, for a confirmation being
/// asked, the id its Approve and Deny answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panel {
    pub lines: Vec<Line>,
    pub confirm: Option<u64>,
}

/// Where a touch started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Grid,
    /// The strip: a tap opens the overlay.
    Strip,
    /// The overlay's sheet.
    Sheet,
    /// The dimmed cards above the open overlay: a tap closes it.
    Shade,
    /// The gear.
    Gear,
    /// A control, or nothing.
    Other,
}

/// One line at the top: optional fixed x (left-aligned), or centered when None.
#[derive(Clone, Debug, PartialEq)]
struct Heading {
    text: String,
    x: Option<f32>,
    y: f32,
    px: f32,
    color: [f32; 4],
}

/// The type sizes at the top, in points, as the main screen's.
const TITLE_PT: f32 = 32.0;
const STATUS_PT: f32 = 15.0;

pub struct Overview {
    layout: Layout,
    status: Vec<String>,
    note: Option<(String, [f32; 4])>,
    agents: Vec<AgentInfo>,
    pending: Vec<String>,
    grid: CardGrid,
    strip: TurnStrip,
    overlay: TurnOverlay,
    card: Option<Panel>,
    dictation: Option<DictationPrompt>,
    read_aloud: bool,
    speaking: bool,
    /// An agent view has been opened: play reads its replies.
    play_live: bool,
    headings: Vec<Heading>,
    divider_y: Option<f32>,
    gear: Gear,
    /// The confirmation and dictation cards as placed.
    panels: Vec<Flow>,
    /// The first of `panels` is the dictation card.
    dictation_shown: bool,
    buttons: Vec<FlowButton>,
    hits: Vec<(AgentHit, Rect)>,
    touch: Option<(u64, Target)>,
    loaded: bool,
    dirty: bool,
    /// The height other sessions' cards take above the panels, and where
    /// they end (plan 20261022_03, the L3 amendment of 2026-10-24 15:56
    /// EDT, item 2).
    cross_h: f32,
    cross_bottom: f32,
    /// Whether the custom GPU panel shader is rendering the navy panels.
    pub shader_panels: bool,
}

impl Default for Overview {
    fn default() -> Self {
        Self::new()
    }
}

impl Overview {
    pub fn new() -> Self {
        Overview {
            layout: Layout::default(),
            status: Vec::new(),
            note: None,
            agents: Vec::new(),
            pending: Vec::new(),
            grid: CardGrid::new(),
            strip: TurnStrip::new(),
            overlay: TurnOverlay::new(),
            card: None,
            dictation: None,
            read_aloud: false,
            speaking: false,
            play_live: false,
            headings: Vec::new(),
            divider_y: None,
            gear: Gear::default(),
            panels: Vec::new(),
            dictation_shown: false,
            buttons: Vec::new(),
            hits: Vec::new(),
            touch: None,
            loaded: false,
            dirty: true,
            cross_h: 0.0,
            cross_bottom: 0.0,
            shader_panels: false,
        }
    }

    /// The agents as listed, and the panes with a confirmation waiting.
    pub fn set_agents(&mut self, agents: Vec<AgentInfo>, pending: &[String]) {
        if self.loaded && agents == self.agents && pending == self.pending {
            return;
        }
        self.loaded = true;
        self.agents = agents;
        self.pending = pending.to_vec();
        if self.grid.set_agents(&self.agents, &self.pending) {
            self.dirty = true;
        }
    }

    pub fn set_layout(&mut self, layout: Layout) {
        if layout != self.layout {
            self.layout = layout;
            self.settle();
        }
    }

    /// The status lines (the link and the sound), and a pairing note or an
    /// error under them.
    pub fn set_status(&mut self, status: Vec<String>, note: Option<(String, [f32; 4])>) {
        if status != self.status || note != self.note {
            self.status = status;
            self.note = note;
            self.settle();
        }
    }

    /// The confirmation card, while one is shown.
    pub fn set_card(&mut self, card: Option<Panel>) {
        if card != self.card {
            self.card = card;
            self.settle();
        }
    }

    /// The height other sessions' cards need: the cards make room for
    /// them above the panels.
    pub fn set_cross_height(&mut self, h: f32) {
        if (h - self.cross_h).abs() > 0.01 {
            self.cross_h = h;
            self.settle();
        }
    }

    /// Where other sessions' cards end: above the panels and the bottom
    /// row.
    pub fn cross_bottom(&self) -> f32 {
        self.cross_bottom
    }

    /// The dictation card's prompt, while a message is taken.
    pub fn set_dictation(&mut self, prompt: Option<DictationPrompt>) {
        if prompt != self.dictation {
            self.dictation = prompt;
            self.settle();
        }
    }

    /// The play button: on, something being said, and whether a view has
    /// been opened for it to read.
    pub fn set_play(&mut self, read_aloud: bool, speaking: bool, live: bool) {
        if (read_aloud, speaking, live) != (self.read_aloud, self.speaking, self.play_live) {
            self.read_aloud = read_aloud;
            self.speaking = speaking;
            self.play_live = live;
            self.dirty = true;
        }
    }

    /// A line of the conversation with Gemini.
    pub fn push_turn(&mut self, speaker: Speaker, text: String) {
        let shown = self.strip.visible();
        self.strip.push(speaker, &text);
        self.overlay.log_mut().push(speaker, text);
        self.strip_changed(shown);
    }

    /// What STT is hearing, at the end of the conversation and on the
    /// strip.
    pub fn set_partial(&mut self, partial: Option<String>) {
        let shown = self.strip.visible();
        let strip = self.strip.set_partial(partial.clone());
        if self.overlay.log_mut().set_partial(partial) || strip {
            self.strip_changed(shown);
        }
    }

    /// Whether the strip shows: a line of Gemini's, and the general agent
    /// on (plan 20261024_02, decision 4).
    fn strip_shown(&self) -> bool {
        self.strip.visible() && settings::current().general_agent
    }

    /// The settings changed: what they turn off is redrawn, and without the
    /// general agent the strip and the overlay go.
    pub fn settings_changed(&mut self) {
        if !settings::current().general_agent {
            self.overlay.close();
        }
        self.settle();
    }

    /// The theme changed: the cards, the conversation and the headings
    /// are laid out again in its colours.
    pub fn restyle(&mut self) {
        self.grid.restyle();
        self.overlay.log_mut().restyle();
        self.settle();
        self.dirty = true;
    }

    pub fn gear(&self) -> Gear {
        self.gear
    }

    /// The strip may have changed: when it shows or hides, the cards' room
    /// changes with it.
    fn strip_changed(&mut self, shown: bool) {
        if self.strip.visible() != shown {
            self.settle();
        } else {
            self.dirty = true;
        }
    }

    pub fn grid(&self) -> &CardGrid {
        &self.grid
    }

    pub fn strip(&self) -> &TurnStrip {
        &self.strip
    }

    pub fn overlay(&self) -> &TurnOverlay {
        &self.overlay
    }

    pub fn log(&self) -> &TurnLog {
        self.overlay.log()
    }

    /// Opens the overlay, as a tap on the strip does.
    pub fn open_turns(&mut self) {
        if self.overlay.open() {
            self.dirty = true;
        }
    }

    /// The system Back: closes the overlay. Returns whether it was open;
    /// if not, Back is not the overview's.
    pub fn back(&mut self) -> bool {
        let was = self.overlay.close();
        if was {
            self.dirty = true;
        }
        was
    }

    fn scale(&self) -> f32 {
        if self.layout.scale > 0.0 {
            self.layout.scale
        } else {
            1.0
        }
    }

    /// Places everything for the layout and the cards shown.
    fn settle(&mut self) {
        let l = self.layout;
        let s = self.scale();
        let pad = PAD * s;
        let mut headings = Vec::new();
        let mut y = l.top + 24.0 * s;
        self.gear = Gear::place(l.w, pad, y, TITLE_PT * s * 1.35, s);
        headings.push(Heading {
            text: "flankr".to_string(),
            x: Some(pad),
            y,
            px: TITLE_PT * s,
            color: theme().text,
        });
        y += TITLE_PT * s * 1.25;
        for line in &self.status {
            headings.push(Heading {
                text: line.clone(),
                x: None,
                y,
                px: STATUS_PT * s,
                color: theme().dim,
            });
            y += STATUS_PT * s * 1.35;
        }
        if let Some((note, color)) = &self.note {
            headings.push(Heading {
                text: note.clone(),
                x: None,
                y,
                px: STATUS_PT * s,
                color: *color,
            });
            y += STATUS_PT * s * 1.35;
        }
        let divider_y = y + 4.0 * s;
        self.divider_y = Some(divider_y);
        self.headings = headings;
        let top = divider_y + 12.0 * s;

        // The bottom row, then the cards above it.
        let row_bottom = l.h - l.bottom - 16.0 * s;
        let dictating = self.dictation.is_some();
        self.buttons = ControlRow::lay_out(pad, l.w - pad, row_bottom, s, dictating);
        let mut above = row_bottom - BUTTON_H * s - 12.0 * s;
        let mut panels = Vec::new();
        // While a confirmation asks, the message makes way for it.
        let asking = self.card.as_ref().is_some_and(|c| c.confirm.is_some());
        self.dictation_shown = false;
        if let Some(prompt) = self.dictation.as_ref().filter(|_| !asking) {
            let p = panel(
                &prompt.lines,
                None,
                prompt.mode == PromptMode::Keyboard,
                pad,
                l.w - 2.0 * pad,
                above,
                s,
            );
            above -= p.height + 8.0 * s;
            panels.push(p);
            self.dictation_shown = true;
        }
        if let Some(card) = &self.card {
            let p = panel(
                &card.lines,
                card.confirm,
                false,
                pad,
                l.w - 2.0 * pad,
                above,
                s,
            );
            above -= p.height + 8.0 * s;
            panels.push(p);
        }
        self.panels = panels;
        // Other sessions' cards above them, at the bottom of the cards'
        // room.
        self.cross_bottom = above;
        if self.cross_h > 0.0 {
            above -= self.cross_h + 8.0 * s;
        }

        // The strip above them while it shows, and the cards on the rest.
        let bottom = above.max(top);
        let strip_h = if self.strip_shown() {
            TurnStrip::HEIGHT * s
        } else {
            0.0
        };
        let cards_bottom = (bottom - strip_h).max(top);
        self.strip
            .set_rect(Rect::new(0.0, cards_bottom, l.w, bottom - cards_bottom), s);
        self.grid
            .set_viewport(Rect::new(0.0, top, l.w, cards_bottom - top), s);

        // The overlay, over the cards and the strip: two thirds of the
        // screen, or what there is under the status lines.
        let sheet_top = (l.h * (1.0 - TurnOverlay::SHARE)).max(top).min(bottom);
        self.overlay
            .set_area(Rect::new(0.0, sheet_top, l.w, bottom - sheet_top), s);

        self.hits = self
            .buttons
            .iter()
            .chain(self.panels.iter().flat_map(|p| &p.buttons))
            .map(|b| (b.hit, b.rect))
            .collect();
        self.dirty = true;
    }

    /// The control at `(x, y)`, if any.
    pub fn hit(&self, x: f32, y: f32) -> Option<AgentHit> {
        self.hits
            .iter()
            .find(|(_, r)| r.contains([x, y]))
            .map(|&(h, _)| h)
    }

    /// Where `hit` is, if it is shown.
    pub fn rect_of(&self, hit: AgentHit) -> Option<Rect> {
        self.hits.iter().find(|(h, _)| *h == hit).map(|&(_, r)| r)
    }

    /// A touch at `now` (any monotonic clock). A drag scrolls the cards,
    /// or the overlay while it is open; a tap returns what it hit. The
    /// controls come first, so the bottom row works under the overlay.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now: Duration,
    ) -> Option<OverviewHit> {
        if phase == TouchPhase::Started {
            let target = if self.hit(x, y).is_some() {
                Target::Other
            } else if self.gear.contains(x, y) {
                Target::Gear
            } else if self.overlay.is_open() {
                if self.overlay.contains(x, y) {
                    Target::Sheet
                } else if y < self.overlay.rect().y {
                    Target::Shade
                } else {
                    Target::Other
                }
            } else if self.strip_shown() && self.strip.contains(x, y) {
                Target::Strip
            } else if self.grid.viewport().contains([x, y]) {
                Target::Grid
            } else {
                Target::Other
            };
            self.touch = Some((id, target));
        }
        let (held, target) = self.touch?;
        if held != id {
            return None;
        }
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.touch = None;
        }
        let touched = match target {
            Target::Grid => self.grid.touch(id, phase, x, y, now),
            Target::Sheet => self.overlay.touch(id, phase, x, y, now),
            Target::Strip | Target::Shade | Target::Gear | Target::Other => match phase {
                TouchPhase::Ended => Touched::Tap,
                _ => Touched::Nothing,
            },
        };
        match touched {
            Touched::Nothing => None,
            Touched::Scrolled => {
                self.dirty = true;
                None
            }
            Touched::Tap => match target {
                Target::Grid => Some(
                    self.grid
                        .pane_at(x, y)
                        .map_or(OverviewHit::Tap, |p| OverviewHit::Agent(p.to_string())),
                ),
                Target::Strip => {
                    self.open_turns();
                    None
                }
                Target::Shade => {
                    self.back();
                    None
                }
                Target::Sheet => Some(OverviewHit::Tap),
                Target::Gear => self.gear.contains(x, y).then_some(OverviewHit::Settings),
                Target::Other => match self.hit(x, y) {
                    // Nothing to read until a view has been opened.
                    Some(AgentHit::ReadAloud) if !self.play_live => None,
                    // Turned off in the settings: no tap.
                    Some(hit) if !settings::allows_on_overview(hit) => None,
                    Some(hit) => Some(OverviewHit::Bottom(hit)),
                    None => Some(OverviewHit::Tap),
                },
            },
        }
    }

    /// Carries the cards' fling, and the overlay's slide and fling, on.
    /// Returns whether anything moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let moved = self.grid.tick(dt) | self.overlay.tick(dt);
        if moved {
            self.dirty = true;
        }
        moved
    }

    /// Marks the screen to be drawn again, as after another screen drew
    /// over the list.
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
        if !self.shader_panels {
            fill(list, Rect::new(0.0, 0.0, l.w, l.h), theme().bg, 0.0);
        }
        if let Some(div_y) = self.divider_y {
            fill(list, Rect::new(0.0, div_y, l.w, 1.0 * s), theme().band, 0.0);
        }
        for h in &self.headings {
            let px = h.px;
            let x = match h.x {
                Some(fixed_x) => fixed_x,
                None => {
                    let w = ctx.text_advance(&h.text, px);
                    (l.w - w) * 0.5
                }
            };
            ctx.draw_text(list, &h.text, [x, h.y], px, h.color);
        }
        self.gear.draw(list);
        self.grid.draw(list, ctx, self.shader_panels);
        if self.strip_shown() {
            self.strip.draw(list, ctx);
        }
        if settings::current().general_agent {
            self.overlay.draw(list, ctx, l.w);
        }
        let recording = self.dictation.is_some();
        let read_aloud = self.read_aloud && self.play_live;
        let draw_button = |list: &mut DrawList, b: &FlowButton| {
            SquareButton::draw_enabled(
                list,
                ctx,
                term,
                b,
                b.rect,
                s,
                read_aloud,
                self.speaking,
                recording,
                settings::allows_on_overview(b.hit),
            );
        };
        let microphone = settings::current().microphone;
        for (i, p) in self.panels.iter().enumerate() {
            let start = list.instances.len();
            if let Some(ta) = &p.text_area {
                for f in &p.fills {
                    if f.rect != ta.rect {
                        fill(list, f.rect, f.color, f.radius);
                    }
                }
                for r in &p.rows {
                    if !ta.rect.contains([r.x, r.y]) {
                        ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
                    }
                }
                ta.draw(list, ctx);
            } else {
                for f in &p.fills {
                    fill(list, f.rect, f.color, f.radius);
                }
                for r in &p.rows {
                    ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
                }
            }
            // The dictation card without the microphone: dim.
            if i == 0 && self.dictation_shown && !microphone {
                settings::dim_since(list, start);
            }
            for b in &p.buttons {
                draw_button(list, b);
            }
        }
        for b in &self.buttons {
            draw_button(list, b);
        }
        self.dirty = false;
        true
    }

    /// The panel instances for the custom GPU panel shader.
    pub fn panel_instances(&self) -> Vec<PanelInstance> {
        let mut out = self.grid.panel_instances();
        for p in &self.panels {
            if let Some(f) = p.fills.first() {
                out.push(PanelInstance {
                    rect: [f.rect.x, f.rect.y, f.rect.w, f.rect.h],
                    color: f.color,
                    border_color: theme().band,
                    clip: [0.0; 4],
                    params: [f.radius, 1.0 * self.scale(), 1.0, 0.0],
                });
            }
        }
        out
    }
}

/// `lines` on a card `w` wide from `x`, its bottom at `bottom`, with
/// Approve and Deny under them when `confirm` is given. In keyboard mode,
/// the input line sits inside a shaded input area.
fn panel(
    lines: &[Line],
    confirm: Option<u64>,
    is_keyboard: bool,
    x: f32,
    w: f32,
    bottom: f32,
    s: f32,
) -> Flow {
    let pad = PAD * s;
    let inner_x = x + pad;
    let inner_w = w - 2.0 * pad;
    let actions = if confirm.is_some() {
        ACTION_H * s + pad
    } else {
        0.0
    };

    if is_keyboard {
        let (input_line, head_lines) = match lines.split_last() {
            Some((last, rest)) => (Some(last), rest),
            None => (None, lines),
        };

        let mut rows: Vec<(String, f32, [f32; 4], f32)> = Vec::new();
        for (n, line) in head_lines.iter().enumerate() {
            let px = line.size * s;
            for (i, t) in wrap(&line.text, columns(inner_w, px))
                .into_iter()
                .enumerate()
            {
                let gap = if i == 0 && n > 0 {
                    line.gap.min(8.0) * s
                } else {
                    0.0
                };
                rows.push((t, px, line.color, gap));
            }
        }

        let box_pad_x = 10.0 * s;
        let box_pad_y = 8.0 * s;
        let box_inner_w = inner_w - 2.0 * box_pad_x;
        let px = input_line.map_or(20.0 * s, |l| l.size * s);
        let input_text = input_line.map_or("", |l| l.text.as_str());
        let input_color = input_line.map_or(theme().muted, |l| l.color);
        let mut input_rows = wrap(input_text, columns(box_inner_w, px));
        if input_rows.is_empty() {
            input_rows.push(String::new());
        }
        let input_text_h = input_rows.len() as f32 * line_h(px);
        let box_h = input_text_h + 2.0 * box_pad_y;
        let gap_before_box = if !rows.is_empty() { 8.0 * s } else { 0.0 };

        let rows_h: f32 = rows.iter().map(|(_, px, _, gap)| line_h(*px) + gap).sum();
        let h = rows_h + gap_before_box + box_h + 2.0 * pad + actions;
        let top = bottom - h;
        let mut flow = Flow::default();

        flow.fills.push(Fill {
            rect: Rect::new(x, top, w, h),
            color: theme().card,
            radius: 10.0 * s,
        });

        let mut y = top + pad;
        for (text, px, color, gap) in rows {
            y += gap;
            flow.rows.push(Row {
                text,
                x: inner_x,
                y,
                px,
                color,
                mono: false,
            });
            y += line_h(px);
        }

        let box_y = y + gap_before_box;
        let box_rect = Rect::new(inner_x, box_y, inner_w, box_h);
        let is_placeholder = input_text == "Type your message";
        let actual_text = if is_placeholder { "" } else { input_text };

        let text_area = TextArea::new(actual_text, box_rect)
            .with_padding([box_pad_x, box_pad_y])
            .with_font_size(px)
            .with_text_color(theme().text)
            .with_bg(theme().input)
            .with_corner_radius(8.0 * s)
            .with_placeholder("Type your message")
            .with_active(true);
        text_area.set_cursor(actual_text.len());
        flow.text_area = Some(text_area);

        flow.fills.push(Fill {
            rect: box_rect,
            color: theme().input,
            radius: 8.0 * s,
        });

        for (i, text) in input_rows.into_iter().enumerate() {
            flow.rows.push(Row {
                text,
                x: inner_x + box_pad_x,
                y: box_y + box_pad_y + i as f32 * line_h(px),
                px,
                color: input_color,
                mono: false,
            });
        }

        flow.height = h;
        return flow;
    }

    let mut rows: Vec<(String, f32, [f32; 4], f32)> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let px = line.size * s;
        for (i, t) in wrap(&line.text, columns(inner_w, px))
            .into_iter()
            .enumerate()
        {
            let gap = if i == 0 && n > 0 {
                line.gap.min(8.0) * s
            } else {
                0.0
            };
            rows.push((t, px, line.color, gap));
        }
    }
    let h = rows
        .iter()
        .map(|(_, px, _, gap)| line_h(*px) + gap)
        .sum::<f32>()
        + 2.0 * pad
        + actions;
    let top = bottom - h;
    let mut flow = Flow::default();
    flow.fills.push(Fill {
        rect: Rect::new(x, top, w, h),
        color: theme().card,
        radius: 10.0 * s,
    });
    let mut y = top + pad;
    for (text, px, color, gap) in rows {
        y += gap;
        flow.rows.push(Row {
            text,
            x: inner_x,
            y,
            px,
            color,
            mono: false,
        });
        y += line_h(px);
    }
    if let Some(id) = confirm {
        let gap = 12.0 * s;
        let bw = (inner_w - gap) / 2.0;
        let by = bottom - pad - ACTION_H * s;
        for (i, (hit, label, bg)) in [
            (AgentHit::Approve(id), "Approve", theme().approve),
            (AgentHit::Deny(id), "Deny", theme().deny),
        ]
        .into_iter()
        .enumerate()
        {
            flow.buttons.push(FlowButton {
                hit,
                rect: Rect::new(inner_x + i as f32 * (bw + gap), by, bw, ACTION_H * s),
                label,
                bg,
            });
        }
    }
    flow.height = h;
    flow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overview::agent_card::tests::{agent, asking};

    const PHONE: Layout = Layout {
        w: 1080.0,
        h: 2340.0,
        scale: 2.0,
        top: 80.0,
        bottom: 40.0,
    };

    const MS: Duration = Duration::from_millis(16);

    fn overview(n: u32) -> Overview {
        let mut o = Overview::new();
        o.set_layout(PHONE);
        o.set_status(vec!["connected to m4".into(), "tap to talk".into()], None);
        let agents = (1..=n)
            .map(|t| agent(&format!("p{t}"), "w1", t, None))
            .collect();
        o.set_agents(agents, &[]);
        o
    }

    fn tap(o: &mut Overview, x: f32, y: f32) -> Option<OverviewHit> {
        o.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        o.touch(1, TouchPhase::Ended, x, y, MS)
    }

    /// Drags up by `by` pixels from `(x, y)`, then lifts after a pause:
    /// what the lift returned.
    fn drag_up(o: &mut Overview, x: f32, y: f32, by: f32) -> Option<OverviewHit> {
        o.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        let mut now = Duration::ZERO;
        for i in 1..=10 {
            now += MS;
            o.touch(1, TouchPhase::Moved, x, y - by * i as f32 / 10.0, now);
        }
        o.touch(
            1,
            TouchPhase::Ended,
            x,
            y - by,
            now + Duration::from_millis(300),
        )
    }

    #[test]
    fn switching_to_light_leaves_no_dark_gray_colour_drawn() {
        use crate::theme::testing::{dark_gray_only, draw_context, use_theme};
        use crate::ui::{Card, CardState};
        use_theme("dark_gray");
        let ctx = draw_context();
        let mut o = overview(3);
        let mut agents: Vec<AgentInfo> = (1..=3)
            .map(|t| agent(&format!("p{t}"), "w1", t, None))
            .collect();
        agents[1].turn = asking("Go on?");
        o.set_agents(agents, &["p3".to_string()]);
        o.push_turn(Speaker::User, "what is p2 doing".into());
        o.push_turn(Speaker::Gemini, "It is asking.".into());
        o.set_partial(Some("and p3".into()));
        // What the app gives on every update, in the theme in force.
        let card = Card {
            confirm_id: 7,
            command: "continue".into(),
            agent: "Claude".into(),
            project: Some("flankr".into()),
            handle: Some("blue fox".into()),
            state: CardState::Waiting,
        };
        let give = |o: &mut Overview| {
            o.set_status(
                vec!["connected to m4".into(), "tap to talk".into()],
                Some((
                    "pairing failed".into(),
                    theme().get(crate::theme::Role::Warn),
                )),
            );
            o.set_card(Some(Panel {
                lines: card.lines(),
                confirm: Some(7),
            }));
        };
        give(&mut o);
        o.open_turns();
        let mut list = DrawList::new();
        assert!(o.draw(&mut list, &ctx, None));
        assert!(!dark_gray_only(&list).is_empty(), "drawn in Dark (gray)");

        use_theme("light_black");
        o.restyle();
        give(&mut o);
        assert!(o.draw(&mut list, &ctx, None), "drawn again");
        assert_eq!(dark_gray_only(&list), Vec::<[f32; 4]>::new());
        use_theme("dark_gray");
    }

    #[test]
    fn other_sessions_cards_take_room_under_the_cards_above_the_panels() {
        let mut o = overview(6);
        let grid = o.grid().viewport();
        let bottom = o.cross_bottom();
        assert!(bottom >= grid.y + grid.h, "under the cards");
        o.set_cross_height(100.0);
        assert_eq!(o.cross_bottom(), bottom, "the panels stay");
        let after = o.grid().viewport();
        assert!(after.y + after.h <= bottom - 100.0, "the cards make room");
        o.set_cross_height(0.0);
        assert_eq!(o.grid().viewport(), grid);
    }

    #[test]
    fn a_tap_on_a_card_gives_its_pane() {
        let mut o = overview(3);
        let r = o.grid().card_rect("p3").unwrap();
        assert_eq!(
            tap(&mut o, r.x + 10.0, r.y + 10.0),
            Some(OverviewHit::Agent("p3".into()))
        );
        // An asking card's buttons are part of the card.
        let mut agents: Vec<AgentInfo> = (1..=3)
            .map(|t| agent(&format!("p{t}"), "w1", t, None))
            .collect();
        agents[1].turn = asking("Push it?");
        o.set_agents(agents, &[]);
        let (_, card) = o.grid().cards().iter().find(|(p, _)| p == "p2").unwrap();
        let yes = card.buttons[0].rect;
        let dy = o.grid().card_rect("p2").unwrap().y - card.rect.y;
        assert_eq!(
            tap(&mut o, yes.x + 5.0, yes.y + dy + 5.0),
            Some(OverviewHit::Agent("p2".into()))
        );
        // Between the cards is no card: a tap.
        let r = o.grid().card_rect("p1").unwrap();
        assert_eq!(
            tap(&mut o, r.x + r.w + 4.0, r.y + 10.0),
            Some(OverviewHit::Tap)
        );
    }

    /// Some turns of the conversation.
    fn talk(o: &mut Overview, turns: usize) {
        for i in 0..turns {
            o.push_turn(Speaker::User, format!("question {i}"));
            o.push_turn(Speaker::Gemini, format!("Answer {i}, at some length."));
        }
    }

    /// Opens the overlay with a tap on the strip, and lets it slide.
    fn open(o: &mut Overview) {
        let strip = o.strip().rect();
        assert_eq!(tap(o, 540.0, strip.y + strip.h * 0.5), None);
        assert!(o.overlay().is_open());
        o.tick(TurnOverlay::SLIDE);
    }

    #[test]
    fn the_cards_fill_the_screen_and_scroll_as_one_list() {
        let mut o = overview(24);
        let record = o.rect_of(AgentHit::Message).unwrap();
        let grid = o.grid().viewport();
        // From under the status lines to above the bottom row: no half.
        let status = o.headings.last().unwrap();
        assert!(grid.y > status.y && grid.y < status.y + 100.0);
        assert!((record.y - (grid.y + grid.h) - 12.0 * 2.0).abs() < 0.01);
        assert!(grid.h > 0.75 * PHONE.h);
        assert!(o.grid().scroll().max() > 0.0);
        // A drag moves the cards; ending on a card, it is a scroll, not a
        // tap.
        assert_eq!(drag_up(&mut o, 540.0, grid.y + grid.h - 40.0, 250.0), None);
        assert!((o.grid().scroll().offset() - 250.0).abs() < 1.0);
        // With turns, the strip takes one line off their bottom.
        talk(&mut o, 1);
        let strip = o.strip().rect();
        let grid = o.grid().viewport();
        assert_eq!(strip.h, TurnStrip::HEIGHT * 2.0);
        assert_eq!(grid.y + grid.h, strip.y);
        assert!(strip.y + strip.h <= record.y);
    }

    #[test]
    fn the_strip_shows_the_newest_line_and_hides_with_no_turns() {
        let mut o = overview(2);
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        o.draw(&mut list, &ctx, None);
        assert!(!o.strip().visible());
        let full = o.grid().viewport();
        // A tap where the strip would be is a card's place, not the strip.
        assert_eq!(
            tap(&mut o, 540.0, full.y + full.h - 10.0),
            Some(OverviewHit::Tap)
        );
        assert!(!o.overlay().is_open());
        o.push_turn(Speaker::User, "what is claude doing".into());
        assert!(!o.strip().visible(), "the strip is Gemini's");
        o.push_turn(Speaker::Gemini, "Claude is running the tests.".into());
        assert_eq!(
            o.strip().line().as_deref(),
            Some("< Claude is running the tests.")
        );
        assert!(o.draw(&mut list, &ctx, None));
        // The live words while speaking, then Gemini's line again.
        o.set_partial(Some("and co".into()));
        assert_eq!(o.strip().line().as_deref(), Some("> and co\u{2026}"));
        assert!(o.draw(&mut list, &ctx, None));
        o.set_partial(None);
        assert_eq!(
            o.strip().line().as_deref(),
            Some("< Claude is running the tests.")
        );
        // Above the confirmation and dictation cards' place.
        o.set_dictation(Some(
            vec![Line {
                text: "Taking a message".into(),
                size: 15.0,
                color: theme().muted,
                gap: 28.0,
            }]
            .into(),
        ));
        let card = &o.panels[0].fills[0].rect;
        let strip = o.strip().rect();
        assert!(strip.y + strip.h <= card.y);
    }

    #[test]
    fn a_tap_on_the_strip_opens_the_overlay() {
        let mut o = overview(6);
        talk(&mut o, 30);
        open(&mut o);
        let sheet = o.overlay().rect();
        let record = o.rect_of(AgentHit::Message).unwrap();
        // About two thirds of the screen, over the cards and the strip,
        // ending above the bottom row; the log in it, newest at the bottom.
        assert!((sheet.y - PHONE.h / 3.0).abs() < 1.0);
        assert!(sheet.y + sheet.h <= record.y);
        assert!(sheet.y + sheet.h >= o.strip().rect().y + o.strip().rect().h);
        let log = o.log().viewport();
        assert!(sheet.contains([log.x + 1.0, log.y + 1.0]));
        assert!(o.log().scroll().max() > 0.0);
        assert_eq!(o.log().scroll().offset(), o.log().scroll().max());
        // A drag on it scrolls the turns, not the cards, and leaves it open.
        o.touch(2, TouchPhase::Started, 540.0, log.y + 200.0, Duration::ZERO);
        let mut now = Duration::ZERO;
        for i in 1..=10 {
            now += MS;
            o.touch(
                2,
                TouchPhase::Moved,
                540.0,
                log.y + 200.0 + 30.0 * i as f32,
                now,
            );
        }
        o.touch(
            2,
            TouchPhase::Ended,
            540.0,
            log.y + 500.0,
            now + Duration::from_millis(300),
        );
        assert!(o.overlay().is_open());
        assert!((o.log().scroll().max() - o.log().scroll().offset() - 300.0).abs() < 1.0);
        assert_eq!(o.grid().scroll().offset(), 0.0);
        // A card under it is not a tap away.
        let under = o.grid().card_rect("p5").unwrap();
        assert!(sheet.contains([under.x + 5.0, under.y + 5.0]));
        assert_eq!(
            tap(&mut o, under.x + 5.0, under.y + 5.0),
            Some(OverviewHit::Tap)
        );
        // It never opens by itself: Gemini speaking leaves it closed.
        let mut o = overview(2);
        talk(&mut o, 3);
        o.tick(TurnOverlay::SLIDE);
        assert!(!o.overlay().is_shown());
    }

    #[test]
    fn each_way_of_closing_the_overlay() {
        let mut o = overview(2);
        talk(&mut o, 1);
        // A tap on the dimmed cards above it.
        open(&mut o);
        let sheet = o.overlay().rect();
        let above = o.grid().card_rect("p1").unwrap();
        assert!(above.y + 5.0 < sheet.y);
        assert_eq!(tap(&mut o, above.x + 5.0, above.y + 5.0), None);
        assert!(!o.overlay().is_open());
        o.tick(TurnOverlay::SLIDE);
        assert!(!o.overlay().is_shown());
        // A swipe down on it.
        open(&mut o);
        o.touch(
            1,
            TouchPhase::Started,
            540.0,
            sheet.y + 100.0,
            Duration::ZERO,
        );
        let mut now = Duration::ZERO;
        for i in 1..=10 {
            now += MS;
            o.touch(
                1,
                TouchPhase::Moved,
                540.0,
                sheet.y + 100.0 + 40.0 * i as f32,
                now,
            );
        }
        assert_eq!(
            o.touch(1, TouchPhase::Ended, 540.0, sheet.y + 500.0, now + MS),
            None
        );
        assert!(!o.overlay().is_open());
        o.tick(TurnOverlay::SLIDE);
        assert!(!o.overlay().is_shown());
        // Back; and Back with it closed is not the overview's.
        open(&mut o);
        assert!(o.back());
        o.tick(TurnOverlay::SLIDE);
        assert!(!o.overlay().is_shown());
        assert!(!o.back());
        // Closed, the cards are a tap away again.
        assert_eq!(
            tap(&mut o, above.x + 5.0, above.y + 5.0),
            Some(OverviewHit::Agent("p1".into()))
        );
    }

    #[test]
    fn the_bottom_row_works_with_the_overlay_open() {
        let mut o = overview(2);
        talk(&mut o, 1);
        o.set_play(false, false, true);
        open(&mut o);
        let play = o.rect_of(AgentHit::ReadAloud).unwrap();
        let record = o.rect_of(AgentHit::Message).unwrap();
        let keys = o.rect_of(AgentHit::Keyboard).unwrap();
        assert_eq!((keys.w, keys.h), (record.w, record.h), "record's hit box");
        assert!(keys.x + keys.w < record.x, "left of record");
        assert_eq!(
            tap(&mut o, record.x + 5.0, record.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Message))
        );
        assert_eq!(
            tap(&mut o, play.x + 5.0, play.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::ReadAloud))
        );
        assert!(o.overlay().is_open(), "the row leaves it open");
        // Taking a message: the dictation card and send are live too, the
        // sheet ending above the card.
        o.set_dictation(Some(
            vec![Line {
                text: "Taking a message".into(),
                size: 15.0,
                color: theme().muted,
                gap: 28.0,
            }]
            .into(),
        ));
        let card = o.panels[0].fills[0].rect;
        let sheet = o.overlay().rect();
        assert!(sheet.y + sheet.h <= card.y);
        let send = o.rect_of(AgentHit::Submit).unwrap();
        assert_eq!(
            tap(&mut o, send.x + 5.0, send.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Submit))
        );
        assert!(o.overlay().is_open());
    }

    #[test]
    fn the_bottom_rows_hits() {
        let mut o = overview(2);
        // Play at the left, record at the right, square.
        let play = o.rect_of(AgentHit::ReadAloud).unwrap();
        let record = o.rect_of(AgentHit::Message).unwrap();
        assert!(play.x < 100.0 && record.x + record.w > 980.0);
        assert_eq!(play.w, play.h);
        assert!(record.y + record.h <= PHONE.h - PHONE.bottom);
        assert!(o.grid().viewport().y + o.grid().viewport().h <= record.y);
        assert_eq!(
            tap(&mut o, record.x + 5.0, record.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Message))
        );
        // Play does nothing until a view has been opened.
        assert_eq!(tap(&mut o, play.x + 5.0, play.y + 5.0), None);
        o.set_play(false, false, true);
        assert_eq!(
            tap(&mut o, play.x + 5.0, play.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::ReadAloud))
        );
        // Taking a message: cancel in the middle, send at the right, and
        // the dictation card above them.
        o.set_dictation(Some(
            vec![Line {
                text: "Taking a message".into(),
                size: 15.0,
                color: theme().muted,
                gap: 28.0,
            }]
            .into(),
        ));
        let send = o.rect_of(AgentHit::Submit).unwrap();
        assert!(send.x + send.w > 980.0);
        assert!(o.rect_of(AgentHit::Clear).is_some());
        assert!(o.rect_of(AgentHit::Message).is_none());
        let card = &o.panels[0].fills[0].rect;
        assert!(card.y + card.h <= send.y);
        assert!(o.grid().viewport().y + o.grid().viewport().h <= card.y);
    }

    #[test]
    fn a_confirmation_card_answers_its_own_id() {
        let mut o = overview(1);
        let line = |text: &str| Line {
            text: text.into(),
            size: 22.0,
            color: theme().text,
            gap: 4.0,
        };
        o.set_dictation(Some(vec![line("Please run the tests.")].into()));
        o.set_card(Some(Panel {
            lines: vec![line("Approve?"), line("\"continue\" to Claude in flankr")],
            confirm: Some(7),
        }));
        // The message makes way for the question.
        assert_eq!(o.panels.len(), 1);
        let approve = o.rect_of(AgentHit::Approve(7)).unwrap();
        let deny = o.rect_of(AgentHit::Deny(7)).unwrap();
        assert!(approve.x + approve.w < deny.x);
        assert_eq!(
            tap(&mut o, deny.x + 5.0, deny.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Deny(7)))
        );
        // Answered: the card stays without buttons, and the message is back.
        o.set_card(Some(Panel {
            lines: vec![line("Sent")],
            confirm: None,
        }));
        assert!(o.rect_of(AgentHit::Approve(7)).is_none());
        assert_eq!(o.panels.len(), 2);
    }

    #[test]
    fn overview_keyboard_prompt_card_has_shaded_input_area() {
        let mut o = overview(1);
        let lines = vec![
            Line {
                text: "Message for Claude".into(),
                size: 18.0,
                color: theme().text,
                gap: 4.0,
            },
            Line {
                text: "Type your message".into(),
                size: 20.0,
                color: theme().muted,
                gap: 8.0,
            },
        ];
        o.set_dictation(Some((lines, PromptMode::Keyboard).into()));
        assert_eq!(o.panels.len(), 1);
        let panel = &o.panels[0];
        assert!(panel.fills.iter().any(|f| f.color == theme().card));
        assert!(panel.fills.iter().any(|f| f.color == theme().input));
        assert!(panel.rows.iter().any(|r| r.text == "Type your message"));
        assert!(panel.text_area.is_some());
        let ta = panel.text_area.as_ref().unwrap();
        assert!(ta.is_active());
    }

    #[test]
    fn it_is_drawn_only_when_something_changed() {
        let mut o = overview(2);
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        assert!(o.draw(&mut list, &ctx, None));
        assert!(!o.draw(&mut list, &ctx, None));
        o.set_agents(
            vec![agent("p1", "w1", 1, None), agent("p2", "w1", 2, None)],
            &[],
        );
        assert!(!o.draw(&mut list, &ctx, None), "the same list");
        o.set_agents(vec![agent("p1", "w1", 1, None)], &["p1".to_string()]);
        assert!(o.draw(&mut list, &ctx, None));
        o.push_turn(Speaker::User, "hello".into());
        assert!(o.draw(&mut list, &ctx, None));
    }

    /// The opacity of what is drawn inside `r`, at most.
    fn alpha_in(list: &DrawList, r: Rect) -> f32 {
        list.instances
            .iter()
            .filter(|i| r.contains(i.position))
            .map(|i| i.color[3])
            .fold(0.0, f32::max)
    }

    fn with(sound: bool, microphone: bool, general_agent: bool) -> settings::Settings {
        settings::Settings {
            sound,
            microphone,
            general_agent,
            ..settings::Settings::default()
        }
    }

    #[test]
    fn the_gear_is_at_the_right_of_the_title_and_opens_the_settings() {
        let mut o = overview(2);
        let g = o.gear().rect;
        let title = &o.headings[0];
        assert_eq!(title.text, "flankr");
        assert!((g.x + g.w - (PHONE.w - PAD * 2.0)).abs() < 0.01);
        assert_eq!(g.w, Gear::SIZE * 2.0);
        assert!(g.y <= title.y + title.px && g.y + g.h >= title.y);
        assert_eq!(
            tap(&mut o, g.x + g.w * 0.5, g.y + g.h * 0.5),
            Some(OverviewHit::Settings)
        );
    }

    #[test]
    fn controls_the_settings_turn_off_are_dim_and_take_no_taps() {
        let mut o = overview(2);
        o.set_play(true, false, true);
        let ctx = DrawContext::default();
        let play = o.rect_of(AgentHit::ReadAloud).unwrap();
        let record = o.rect_of(AgentHit::Message).unwrap();
        let keys = o.rect_of(AgentHit::Keyboard).unwrap();
        let mut list = DrawList::new();
        o.draw(&mut list, &ctx, None);
        assert_eq!(alpha_in(&list, play), 1.0);
        assert_eq!(alpha_in(&list, record), 1.0);
        assert_eq!(
            alpha_in(&list, keys),
            1.0,
            "keyboard is live with text input"
        );
        assert_eq!(
            tap(&mut o, keys.x + 5.0, keys.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Keyboard))
        );

        for (settings, off) in [
            (with(false, true, true), AgentHit::ReadAloud),
            (with(true, false, true), AgentHit::Message),
            // Record on the overview is the general agent's too.
            (with(true, true, false), AgentHit::Message),
        ] {
            settings::set_current(settings);
            o.settings_changed();
            let mut list = DrawList::new();
            o.draw(&mut list, &ctx, None);
            let r = o.rect_of(off).unwrap();
            assert!(
                (alpha_in(&list, r) - settings::DISABLED_ALPHA).abs() < 1e-6,
                "{off:?} dim"
            );
            assert_eq!(
                tap(&mut o, r.x + 5.0, r.y + 5.0),
                None,
                "{off:?} takes no tap"
            );
        }
        // The dictation card, without the microphone.
        settings::set_current(with(true, false, true));
        o.set_dictation(Some(
            vec![Line {
                text: "Taking a message".into(),
                size: 15.0,
                color: theme().muted,
                gap: 28.0,
            }]
            .into(),
        ));
        let mut list = DrawList::new();
        o.draw(&mut list, &ctx, None);
        let card = o.panels[0].fills[0].rect;
        assert!((alpha_in(&list, card) - settings::DISABLED_ALPHA).abs() < 1e-6);
        let send = o.rect_of(AgentHit::Submit).unwrap();
        assert_eq!(tap(&mut o, send.x + 5.0, send.y + 5.0), None);
        settings::set_current(settings::Settings::default());
    }

    #[test]
    fn without_the_general_agent_the_strip_and_overlay_are_hidden() {
        let mut o = overview(4);
        talk(&mut o, 2);
        open(&mut o);
        let strip = o.strip().rect();
        settings::set_current(with(true, true, false));
        o.settings_changed();
        assert!(!o.overlay().is_open());
        // The cards take the strip's room, and a tap there is no strip.
        let grid = o.grid().viewport();
        assert!(grid.y + grid.h >= strip.y + strip.h - 0.01);
        assert!(tap(&mut o, 540.0, strip.y + strip.h * 0.5).is_some());
        assert!(!o.overlay().is_open());
        // Gemini's lines come in hidden, and show again with it on.
        o.push_turn(Speaker::Gemini, "Still here.".into());
        let mut list = DrawList::new();
        o.draw(&mut list, &DrawContext::default(), None);
        assert!(o.grid().viewport().y + o.grid().viewport().h >= strip.y + strip.h - 0.01);
        settings::set_current(settings::Settings::default());
        o.settings_changed();
        let grid = o.grid().viewport();
        assert!((grid.y + grid.h - o.strip().rect().y).abs() < 0.01);
        assert_eq!(o.strip().rect().h, TurnStrip::HEIGHT * 2.0);
    }
}
