//! The overview, the home screen (plan 20261023_04, as amended at 17:04
//! and 18:07 EDT). From the top: the title and the status lines; then the
//! agent cards ([`CardGrid`]), scrolling as one list; then, above the
//! bottom row, its prompt cards ([`Prompts`], plan 20261027_03): Gemini's
//! pinned card, or its two bars while a message is taken, and the message
//! card; the confirmation card while one is shown; and the bottom row of
//! the agent view (decision 5): play at the left, record at the right,
//! record becoming send with a cancel in the middle while a message is
//! taken.
//!
//! A prompt card's up arrow expands it into an overlay over the cards; its
//! grab bar, a swipe down on it, a tap on the dimmed cards above it, or
//! Back ([`Overview::back`]) collapses it. The bottom row and the
//! confirmation card stay live while it is open.
//!
//! Tapping a card opens the agent view on that pane (decision 4). Play
//! reads replies from an agent view's pane only, so it is dim and does
//! nothing until a view has been opened; record takes a message for
//! herdr's focused agent. A tap on nothing is the main screen's tap.
//!
//! The ≡ button ([`Menu`]) at the left of the title opens the session
//! drawer (plan 20261028_03, 3.10; the 18:48 EDT amendment, item 1).
//!
//! The title-bar icons at the right of the title, the gear ([`Gear`],
//! which opens the settings, plan 20261024_02) among them, then the ⋮,
//! are placed and drawn by the app ([`crate::titlebar`]) from
//! [`Overview::kebab`]. What the settings turn off is off here too (decision 4):
//! play without sound, record without the microphone or the general agent,
//! and the message card being said without the microphone are drawn dim
//! and take no taps, and without the general agent Gemini's card and its
//! overlay are hidden.

pub mod agent_card;
pub mod card_grid;
pub mod gear;
pub mod panel_shader;

use std::time::Duration;

use flankr_companion::Speaker;
use libcontrols::{DrawContext, Rect};
use libflankr_proto::common::AgentInfo;
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use crate::agent::{
    ACTION_H, AgentHit, BUTTON_H, Fill, Flow, FlowButton, PAD, Row, Terminal, columns, fill, line_h,
};
use crate::controls::scroll::Touched;
use crate::controls::square_button::Volume;
use crate::controls::{ControlRow, RowState, SquareButton};
use crate::drawer::Menu;
use crate::prompt::{MessagePrompt, PromptInput, PromptTouched, Prompts};
use crate::settings;
use crate::theme::theme;
use crate::ui::{Layout, Line, wrap};
pub use card_grid::CardGrid;
pub use gear::Gear;
pub use panel_shader::{PanelInstance, PanelShader};

/// What a touch on the overview did.
#[derive(Clone, Debug, PartialEq)]
pub enum OverviewHit {
    /// A card: open the agent view on this pane.
    Agent(String),
    /// The bottom row, a confirmation's Approve or Deny, or a prompt
    /// card's part.
    Bottom(AgentHit),
    /// A tap on no control.
    Tap,
    /// The ≡ button: open the drawer.
    Menu,
    /// A pinned plugin card (plan 20261028_04, decision 3.7): its index,
    /// where it was tapped on the screen, and whether it was held for
    /// [`crate::plugins::card::HOLD`] (Unpin) rather than tapped.
    PluginCard {
        index: usize,
        x: f32,
        y: f32,
        held: bool,
    },
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
    /// A prompt card, its bars or its overlay ([`Prompts::touch`]).
    Prompt,
    /// The ≡ button.
    Menu,
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
    /// Gemini's card and the message card, and their overlay.
    prompts: Prompts,
    /// The message being taken, as given: held back while a confirmation
    /// asks.
    message: Option<MessagePrompt>,
    card: Option<Panel>,
    /// The volume glyph's state: muted, and either speak option on (plan
    /// 20261028_06, decision 4, and its 2026-10-29 00:26 EDT amendment).
    volume: Volume,
    speaking: bool,
    /// A 2 s press timed on a control, and its progress (decision 7).
    hold: Option<(AgentHit, f32)>,
    headings: Vec<Heading>,
    divider_y: Option<f32>,
    /// The ⋮, at the title's right; the title icons are left of it.
    kebab: crate::menu::Kebab,
    menu: Menu,
    /// The confirmation card as placed.
    panels: Vec<Flow>,
    buttons: Vec<FlowButton>,
    hits: Vec<(AgentHit, Rect)>,
    touch: Option<(u64, Target)>,
    /// When the touch held now came down.
    touch_at: Duration,
    loaded: bool,
    dirty: bool,
    /// The height other sessions' cards take above the panels, and where
    /// they end (plan 20261022_03, the L3 amendment of 2026-10-24 15:56
    /// EDT, item 2).
    cross_h: f32,
    cross_bottom: f32,
    /// Whether the custom GPU panel shader is rendering the navy panels.
    pub shader_panels: bool,
    /// Whether voice recognition is active for the general agent.
    general_agent_listening: bool,
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
            prompts: Prompts::new(),
            message: None,
            card: None,
            volume: Volume::default(),
            speaking: false,
            hold: None,
            headings: Vec::new(),
            divider_y: None,
            kebab: crate::menu::Kebab::default(),
            menu: Menu::default(),
            panels: Vec::new(),
            buttons: Vec::new(),
            hits: Vec::new(),
            touch: None,
            touch_at: Duration::ZERO,
            loaded: false,
            dirty: true,
            cross_h: 0.0,
            cross_bottom: 0.0,
            shader_panels: false,
            general_agent_listening: false,
        }
    }

    /// No list (a switch to a remote with none kept): no cards and no
    /// line until one is given.
    pub fn unload(&mut self) {
        self.loaded = false;
        self.agents.clear();
        self.pending.clear();
        self.grid.unload();
        self.dirty = true;
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

    /// The message card, while a message is taken.
    pub fn set_dictation(&mut self, message: Option<MessagePrompt>) {
        if message != self.message {
            self.message = message;
            self.settle();
        }
    }

    /// Gemini's typed prompt, while one is open.
    pub fn set_gemini_input(&mut self, input: Option<PromptInput>) {
        if self.prompts.set_gemini_input(input) {
            self.settle();
        }
    }

    /// Whether voice recognition is active for the general agent.
    pub fn set_general_agent_listening(&mut self, listening: bool) {
        if self.general_agent_listening != listening {
            self.general_agent_listening = listening;
            self.settle();
        }
    }

    /// The volume button: muted, either speak option on, and something
    /// being said (its waves step). The same menu as the agent view's opens
    /// from it (plan 20261028_06, open question 6).
    pub fn set_volume(&mut self, volume: Volume, speaking: bool) {
        if (volume, speaking) != (self.volume, self.speaking) {
            self.volume = volume;
            self.speaking = speaking;
            self.dirty = true;
        }
    }

    /// The press being timed on `hit`, and its progress, for its bar.
    pub fn set_hold(&mut self, hold: Option<(AgentHit, f32)>) {
        if hold != self.hold {
            self.hold = hold;
            self.dirty = true;
        }
    }

    /// Typing's tools, for the typed cards' rows (decision 8).
    pub fn set_input_tools(&mut self, tools: crate::prompt::InputTools) {
        if self.prompts.set_tools(tools) {
            self.settle();
        }
    }

    /// A line of the conversation with Gemini.
    pub fn push_turn(&mut self, speaker: Speaker, text: String) {
        self.prompts.gemini_mut().push_turn(speaker, &text);
        self.settle();
    }

    /// What STT is hearing, at the end of the conversation and on Gemini's
    /// card.
    pub fn set_partial(&mut self, partial: Option<String>) {
        if self.prompts.gemini_mut().set_partial(partial) {
            self.settle();
        }
    }

    /// Whether voice recognition is active for the general agent.
    pub fn set_listening(&mut self, listening: bool) {
        if self.prompts.gemini_mut().set_listening(listening) {
            self.settle();
        }
    }

    /// The settings changed: what they turn off is redrawn, and without the
    /// general agent Gemini's card and its overlay go.
    pub fn settings_changed(&mut self) {
        self.settle();
    }

    /// The theme changed: the cards, the histories and the headings are
    /// laid out again in its colours.
    pub fn restyle(&mut self) {
        self.grid.restyle();
        self.prompts.restyle();
        self.settle();
        self.dirty = true;
    }

    /// The ≡ button.
    pub fn menu(&self) -> Menu {
        self.menu
    }

    /// The ⋮, at the title's right, always last (plan 20261028_04,
    /// decision 3.6, item 1, and its 22:03 EDT amendment); the title
    /// icons, Settings' gear among them, are placed left of it.
    pub fn kebab(&self) -> crate::menu::Kebab {
        self.kebab
    }

    /// The pinned plugin cards' heights, in order, at the end of the grid.
    pub fn set_plugin_cards(&mut self, heights: Vec<f32>) {
        if self.grid.set_extra(heights) {
            self.dirty = true;
        }
    }

    /// The width a plugin card is laid out at.
    pub fn plugin_card_width(&self) -> f32 {
        (self.layout.w - 2.0 * CardGrid::GAP * self.scale()).max(0.0)
    }

    /// The plugin cards in the grid's window, and the window to clip them
    /// to.
    pub fn plugin_cards(&self) -> (Rect, Vec<(usize, Rect)>) {
        (self.grid.viewport(), self.grid.extra_shown())
    }

    pub fn grid(&self) -> &CardGrid {
        &self.grid
    }

    /// The prompt cards and their overlay.
    pub fn prompts(&self) -> &Prompts {
        &self.prompts
    }

    /// The prompt cards, to act on a hit (expanding, collapsing, the
    /// hold): the screen is laid out again.
    pub fn with_prompts(&mut self, f: impl FnOnce(&mut Prompts)) {
        f(&mut self.prompts);
        self.settle();
    }

    /// The system Back: collapses an expanded prompt card. Returns whether
    /// one was; if not, Back is not the overview's.
    pub fn back(&mut self) -> bool {
        let was = self.prompts.back();
        if was {
            self.settle();
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
        self.kebab = crate::menu::Kebab::place(l.w - pad, y, TITLE_PT * s * 1.35, s);
        // The ≡ at the left, its bars where the title began; the title
        // after it.
        self.menu = Menu::place(pad * 0.5, y, TITLE_PT * s * 1.35, s);
        headings.push(Heading {
            text: "flankr".to_string(),
            x: Some(self.menu.rect.x + self.menu.rect.w + 4.0 * s),
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

        // While a confirmation asks, the message makes way for it.
        let asking = self.card.as_ref().is_some_and(|c| c.confirm.is_some());
        let message = self.message.clone().filter(|_| !asking);
        self.prompts.set_message(message);

        // The bottom row, then the cards above it.
        let row_bottom = l.h - l.bottom - 16.0 * s;
        self.buttons = ControlRow::lay_out(
            pad,
            l.w - pad,
            row_bottom,
            s,
            &RowState::three(self.prompts.taking_input(), self.general_agent_listening),
        );
        let mut above = row_bottom - BUTTON_H * s - 12.0 * s;
        let mut panels = Vec::new();
        if let Some(card) = &self.card {
            let p = panel(&card.lines, card.confirm, pad, l.w - 2.0 * pad, above, s);
            above -= p.height + 8.0 * s;
            panels.push(p);
        }
        self.panels = panels;

        // The prompt cards above them, pinned, and their overlay over the
        // cards: two thirds of the screen, or what there is under the
        // status lines.
        above = self
            .prompts
            .lay_out(pad, l.w - 2.0 * pad, above, top, l.h, s);

        // Other sessions' cards above the prompt cards.
        self.cross_bottom = above;
        if self.cross_h > 0.0 {
            above -= self.cross_h + 8.0 * s;
        }

        let cards_bottom = above.max(top);
        self.grid
            .set_viewport(Rect::new(0.0, top, l.w, cards_bottom - top), s);

        self.hits = self
            .buttons
            .iter()
            .chain(self.panels.iter().flat_map(|p| &p.buttons))
            .map(|b| (b.hit, b.rect))
            .collect();
        self.dirty = true;
    }

    /// The control at `(x, y)`, if any: the bottom row, the confirmation's
    /// buttons, then the prompt cards' parts.
    pub fn hit(&self, x: f32, y: f32) -> Option<AgentHit> {
        self.control_at(x, y).or_else(|| self.prompts.hit(x, y))
    }

    /// The bottom row's or the confirmation's control at `(x, y)`.
    fn control_at(&self, x: f32, y: f32) -> Option<AgentHit> {
        self.hits
            .iter()
            .find(|(_, r)| r.contains([x, y]))
            .map(|&(h, _)| h)
    }

    /// Where `hit` is, if it is shown.
    pub fn rect_of(&self, hit: AgentHit) -> Option<Rect> {
        self.hits
            .iter()
            .find(|(h, _)| *h == hit)
            .map(|&(_, r)| r)
            .or_else(|| self.prompts.rect_of(hit))
    }

    /// A touch at `now` (any monotonic clock). A drag scrolls the cards,
    /// or the overlay's history while it is open; a tap returns what it
    /// hit. The controls come first, so the bottom row works under the
    /// overlay.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now: Duration,
    ) -> Option<OverviewHit> {
        if phase == TouchPhase::Started {
            self.touch_at = now;
            let target = if self.control_at(x, y).is_some() {
                Target::Other
            } else if self.menu.contains(x, y) && !self.prompts.overlay().is_open() {
                Target::Menu
            } else if self.prompts.claims(x, y) {
                Target::Prompt
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
        if target == Target::Prompt {
            let touched = self.prompts.touch(id, phase, x, y, now)?;
            self.dirty = true;
            return match touched {
                PromptTouched::Held => None,
                PromptTouched::Hit(hit) => {
                    if matches!(
                        hit,
                        AgentHit::PromptExpand(_) | AgentHit::PromptCollapse | AgentHit::PromptBars
                    ) {
                        self.settle();
                    }
                    Some(OverviewHit::Bottom(hit))
                }
            };
        }
        let touched = match target {
            Target::Grid => self.grid.touch(id, phase, x, y, now),
            Target::Prompt | Target::Menu | Target::Other => match phase {
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
                Target::Grid => match self.grid.extra_at(x, y) {
                    Some(index) => Some(OverviewHit::PluginCard {
                        index,
                        x,
                        y,
                        held: now.saturating_sub(self.touch_at) >= crate::plugins::card::HOLD,
                    }),
                    None => self
                        .grid
                        .pane_at(x, y)
                        .map(|p| OverviewHit::Agent(p.to_string())),
                },
                Target::Menu => self.menu.contains(x, y).then_some(OverviewHit::Menu),
                Target::Prompt => None,
                Target::Other => match self.control_at(x, y) {
                    // Turned off in the settings: no tap.
                    Some(hit) if !settings::allows_on_overview(hit) => None,
                    Some(hit) => Some(OverviewHit::Bottom(hit)),
                    None => None,
                },
            },
        }
    }

    /// Carries the cards' fling, and the overlay's slide and fling, on.
    /// Returns whether anything moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let moved = self.grid.tick(dt) | self.prompts.tick(dt);
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
        self.menu.draw(list, s, theme().muted);
        self.grid.draw(list, ctx, self.shader_panels);
        self.prompts.draw_pinned(list, ctx);
        let recording = self.prompts.message_open();
        let volume = self.volume;
        let draw_button = |list: &mut DrawList, b: &FlowButton| {
            SquareButton::draw_enabled(
                list,
                ctx,
                term,
                b,
                b.rect,
                s,
                volume,
                self.speaking,
                recording,
                settings::allows_on_overview(b.hit),
            );
        };
        for p in &self.panels {
            for f in &p.fills {
                fill(list, f.rect, f.color, f.radius);
            }
            for r in &p.rows {
                ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
            }
            for b in &p.buttons {
                draw_button(list, b);
            }
        }
        self.prompts.draw_overlay(list, ctx, l.w);
        for b in &self.buttons {
            draw_button(list, b);
        }
        // A 2 s press's progress along its control's bottom (plan
        // 20261028_06, decision 7).
        if let Some((hit, progress)) = self.hold
            && let Some(r) = self.rect_of(hit)
        {
            fill(
                list,
                crate::controls::hold::bar(r, progress, s),
                crate::agent::hold_colour(hit),
                0.0,
            );
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

/// Drops the type size as the text wraps onto more lines, so a long last
/// line fits its card.
fn fit_font_size(text: &str, base_pt: f32, box_w: f32, s: f32) -> (f32, Vec<String>) {
    let base = base_pt.min(12.0);
    let sizes = [base, 11.0, 10.0];
    for &pt in &sizes {
        if pt > base {
            continue;
        }
        let px = pt * s;
        let mut rows = wrap(text, columns(box_w, px));
        if rows.is_empty() {
            rows.push(String::new());
        }
        let max_lines = match pt as usize {
            12.. => 3,
            11.. => 4,
            _ => usize::MAX,
        };
        if rows.len() <= max_lines || pt <= 10.0 {
            return (pt, rows);
        }
    }
    let pt = 10.0;
    let mut rows = wrap(text, columns(box_w, pt * s));
    if rows.is_empty() {
        rows.push(String::new());
    }
    (pt, rows)
}

/// The confirmation card: `lines` on a card `w` wide from `x`, its bottom
/// at `bottom`, with Approve and Deny under them when `confirm` is given.
/// (The message card is the prompt card's now, plan 20261027_03.)
fn panel(lines: &[Line], confirm: Option<u64>, x: f32, w: f32, bottom: f32, s: f32) -> Flow {
    let pad = PAD * s;
    let inner_x = x + pad;
    let inner_w = w - 2.0 * pad;
    let actions = if confirm.is_some() {
        ACTION_H * s + pad
    } else {
        0.0
    };
    let mut rows: Vec<(String, f32, [f32; 4], f32)> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        let is_last = n == lines.len() - 1;
        let (fit_pt, line_rows) = if is_last && line.size >= 18.0 {
            fit_font_size(&line.text, line.size, inner_w, s)
        } else {
            let px = line.size * s;
            (line.size, wrap(&line.text, columns(inner_w, px)))
        };
        let px = fit_pt * s;
        for (i, t) in line_rows.into_iter().enumerate() {
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
                label: label.into(),
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
    use crate::prompt::{PromptKind, PromptOverlay};

    /// Unmuted, an option on.
    const ON: Volume = Volume {
        muted: false,
        on: true,
    };

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
        o.with_prompts(|p| p.expand(PromptKind::GeneralAgent));
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
        // Between the cards is no card: no hit.
        let r = o.grid().card_rect("p1").unwrap();
        assert_eq!(tap(&mut o, r.x + r.w + 4.0, r.y + 10.0), None);
    }

    /// Some turns of the conversation.
    fn talk(o: &mut Overview, turns: usize) {
        for i in 0..turns {
            o.push_turn(Speaker::User, format!("question {i}"));
            o.push_turn(Speaker::Gemini, format!("Answer {i}, at some length."));
        }
    }

    /// Gemini's card as pinned.
    fn gemini(o: &Overview) -> Rect {
        o.prompts().gemini().rect()
    }

    /// The message card as pinned.
    fn message(o: &Overview) -> Rect {
        o.prompts().message().rect()
    }

    /// A message being said.
    fn said(text: &str) -> Option<MessagePrompt> {
        Some(PromptInput::said(text).into())
    }

    /// A message being typed.
    fn typed(text: &str) -> Option<MessagePrompt> {
        Some(PromptInput::typed(text).into())
    }

    /// Opens the overlay with a tap on Gemini's up arrow, and lets it
    /// slide.
    fn open(o: &mut Overview) {
        let arrow = o
            .rect_of(AgentHit::PromptExpand(PromptKind::GeneralAgent))
            .expect("the arrow");
        assert_eq!(
            tap(o, arrow.x + arrow.w * 0.5, arrow.y + arrow.h * 0.5),
            Some(OverviewHit::Bottom(AgentHit::PromptExpand(
                PromptKind::GeneralAgent
            )))
        );
        assert!(o.prompts().overlay().is_open());
        o.tick(PromptOverlay::SLIDE);
    }

    fn sheet(o: &Overview) -> Rect {
        o.prompts().overlay().rect()
    }

    fn history(o: &Overview) -> &crate::prompt::History {
        o.prompts().gemini().history()
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
        // With turns, Gemini's card takes its room off their bottom.
        talk(&mut o, 1);
        let card = gemini(&o);
        let grid = o.grid().viewport();
        assert!(card.h > 0.0);
        assert!((grid.y + grid.h + Prompts::GAP * 2.0 - card.y).abs() < 0.01);
        assert!(card.y + card.h <= record.y);
    }

    #[test]
    fn geminis_card_shows_the_latest_exchange_and_hides_with_no_turns() {
        let mut o = overview(2);
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        o.draw(&mut list, &ctx, None);
        assert_eq!(gemini(&o).h, 0.0);
        let full = o.grid().viewport();
        // A tap where the card would be is a card's place.
        assert_eq!(tap(&mut o, 540.0, full.y + full.h - 10.0), None);
        assert!(!o.prompts().overlay().is_open());
        o.push_turn(Speaker::Gemini, "Claude is running the tests.".into());
        let rows = |o: &Overview| -> Vec<String> {
            o.prompts()
                .gemini()
                .rows()
                .iter()
                .map(|r| r.text.clone())
                .collect()
        };
        assert_eq!(
            rows(&o),
            ["\u{2726} Gemini", "< Claude is running the tests."]
        );
        assert!(o.draw(&mut list, &ctx, None));
        // The live words while speaking, then the exchange again.
        o.set_partial(Some("and co".into()));
        assert_eq!(rows(&o)[1], "> and co\u{2026}");
        assert!(o.draw(&mut list, &ctx, None));
        o.set_partial(None);
        assert_eq!(rows(&o)[1], "< Claude is running the tests.");
        // A message: Gemini's card goes to its bars, above the message
        // card, and comes back when it is sent.
        o.set_dictation(said("please go on"));
        assert_eq!(gemini(&o).h, 0.0);
        let bars = o.prompts().gemini().bars().rect().expect("the bars");
        assert!(bars.y + bars.h <= message(&o).y);
        o.set_dictation(None);
        assert!(gemini(&o).h > 0.0);
        assert!(o.prompts().gemini().bars().rect().is_none());
    }

    #[test]
    fn a_tap_on_the_arrow_opens_the_overlay() {
        let mut o = overview(6);
        talk(&mut o, 30);
        // A tap on the card but not its arrow leaves it pinned.
        let card = gemini(&o);
        assert_eq!(tap(&mut o, card.x + 40.0, card.y + card.h - 8.0), None);
        assert!(!o.prompts().overlay().is_open());
        open(&mut o);
        let sheet = sheet(&o);
        let record = o.rect_of(AgentHit::Message).unwrap();
        // About two thirds of the screen, over the cards, ending above the
        // bottom row; the history in it, newest at the bottom.
        assert!((sheet.y - PHONE.h / 3.0).abs() < 1.0);
        assert!(sheet.y + sheet.h <= record.y);
        assert_eq!(gemini(&o).h, 0.0, "expanded, not pinned");
        let log = history(&o).viewport();
        assert!(sheet.contains([log.x + 1.0, log.y + 1.0]));
        assert!(history(&o).scroll().max() > 0.0);
        assert_eq!(history(&o).scroll().offset(), history(&o).scroll().max());
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
        assert!(o.prompts().overlay().is_open());
        let scroll = history(&o).scroll();
        assert!((scroll.max() - scroll.offset() - 300.0).abs() < 1.0);
        assert_eq!(o.grid().scroll().offset(), 0.0);
        // A card under it is not a tap away.
        let y = log.y + 10.0;
        let under = (1..=6)
            .filter_map(|t| o.grid().card_rect(&format!("p{t}")))
            .find(|r| r.y <= y && y <= r.y + r.h)
            .expect("a card there");
        assert_eq!(tap(&mut o, under.x + 5.0, y), None);
        // It never opens by itself: Gemini speaking leaves it closed.
        let mut o = overview(2);
        talk(&mut o, 3);
        o.tick(PromptOverlay::SLIDE);
        assert!(!o.prompts().overlay().is_shown());
    }

    #[test]
    fn each_way_of_closing_the_overlay() {
        let mut o = overview(2);
        talk(&mut o, 1);
        // A tap on the dimmed cards above it.
        open(&mut o);
        let sheet = sheet(&o);
        let above = o.grid().card_rect("p1").unwrap();
        assert!(above.y + 5.0 < sheet.y);
        assert_eq!(
            tap(&mut o, above.x + 5.0, above.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::PromptCollapse))
        );
        assert!(!o.prompts().overlay().is_open());
        o.tick(PromptOverlay::SLIDE);
        assert!(!o.prompts().overlay().is_shown());
        assert!(gemini(&o).h > 0.0, "pinned again");
        // A tap on the grab bar.
        open(&mut o);
        let bar = o.rect_of(AgentHit::PromptCollapse).unwrap();
        assert_eq!(
            tap(&mut o, bar.x + bar.w * 0.5, bar.y + bar.h * 0.5),
            Some(OverviewHit::Bottom(AgentHit::PromptCollapse))
        );
        o.tick(PromptOverlay::SLIDE);
        assert!(!o.prompts().overlay().is_shown());
        // A swipe down on it.
        open(&mut o);
        let sheet = self::sheet(&o);
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
            Some(OverviewHit::Bottom(AgentHit::PromptCollapse))
        );
        assert!(!o.prompts().overlay().is_open());
        o.tick(PromptOverlay::SLIDE);
        assert!(!o.prompts().overlay().is_shown());
        // Back; and Back with it closed is not the overview's.
        open(&mut o);
        assert!(o.back());
        o.tick(PromptOverlay::SLIDE);
        assert!(!o.prompts().overlay().is_shown());
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
        open(&mut o);
        let play = o.rect_of(AgentHit::Volume).unwrap();
        let record = o.rect_of(AgentHit::Message).unwrap();
        assert_eq!(
            tap(&mut o, record.x + 5.0, record.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Message))
        );
        assert_eq!(
            tap(&mut o, play.x + 5.0, play.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Volume))
        );
        assert!(o.prompts().overlay().is_open(), "the row leaves it open");
        // Taking a message: one card at a time. Gemini's goes to its bars,
        // the message card is pinned, and send is live.
        o.set_dictation(said("Taking a message"));
        assert!(!o.prompts().overlay().is_open());
        assert!(message(&o).h > 0.0);
        let send = o.rect_of(AgentHit::Submit).unwrap();
        assert_eq!(
            tap(&mut o, send.x + 5.0, send.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Submit))
        );
        // Sent: Gemini's card comes back as it was, expanded.
        o.set_dictation(None);
        assert!(o.prompts().overlay().is_open());
    }

    #[test]
    fn the_bottom_rows_hits() {
        let mut o = overview(2);
        // Volume at the left, record at the right, square.
        let play = o.rect_of(AgentHit::Volume).unwrap();
        let record = o.rect_of(AgentHit::Message).unwrap();
        assert!(play.x < 100.0 && record.x + record.w > 980.0);
        assert_eq!(play.w, play.h);
        assert!(record.y + record.h <= PHONE.h - PHONE.bottom);
        assert!(o.grid().viewport().y + o.grid().viewport().h <= record.y);
        assert_eq!(
            tap(&mut o, record.x + 5.0, record.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Message))
        );
        // Centre button: General agent is in the middle, slot 1 of 3.
        let ga = o.rect_of(AgentHit::GeneralAgent).unwrap();
        assert!(ga.x > play.x && ga.x + ga.w < record.x);
        assert!((ga.x - (play.x + record.x) * 0.5).abs() < 0.01, "evenly");
        assert_eq!(
            tap(&mut o, ga.x + 5.0, ga.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::GeneralAgent))
        );
        // The volume menu opens from the overview too (open question 6).
        assert_eq!(
            tap(&mut o, play.x + 5.0, play.y + 5.0),
            Some(OverviewHit::Bottom(AgentHit::Volume))
        );
        // Taking a message: cancel ('X') replaces the general agent button in
        // the middle, send at the right, and the message card above them.
        o.set_dictation(said("Taking a message"));
        let send = o.rect_of(AgentHit::Submit).unwrap();
        assert!(send.x + send.w > 980.0);
        assert!(o.rect_of(AgentHit::Clear).is_some());
        assert!(o.rect_of(AgentHit::GeneralAgent).is_none());
        assert!(o.rect_of(AgentHit::Message).is_none());
        let card = message(&o);
        assert!(card.y + card.h <= send.y);
        assert!(o.grid().viewport().y + o.grid().viewport().h <= card.y);
        // Typing to Gemini: the X and send too.
        o.set_dictation(None);
        o.set_gemini_input(Some(PromptInput::typed("")));
        assert!(o.rect_of(AgentHit::Clear).is_some());
        assert!(o.rect_of(AgentHit::Submit).is_some());
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
        o.set_dictation(said("Please run the tests."));
        o.set_card(Some(Panel {
            lines: vec![line("Approve?"), line("\"continue\" to Claude in flankr")],
            confirm: Some(7),
        }));
        // The message makes way for the question.
        assert_eq!(o.panels.len(), 1);
        assert!(!o.prompts().message_open());
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
        assert!(o.prompts().message_open());
        assert!(message(&o).y + message(&o).h <= o.panels[0].fills[0].rect.y);
    }

    #[test]
    fn a_message_card_opens_as_its_session_left_it() {
        let mut o = overview(2);
        // Left expanded for this session: it opens expanded, with what was
        // sent to it.
        o.set_dictation(Some(MessagePrompt {
            expanded: true,
            sent: vec![crate::prompt::Sent {
                text: "run the tests".into(),
                time: "09:41".into(),
            }],
            ..PromptInput::said("and push").into()
        }));
        o.tick(PromptOverlay::SLIDE);
        assert_eq!(o.prompts().expanded(), Some(PromptKind::Message));
        let rows = o.prompts().message().history().rows();
        assert_eq!(rows[1].text, "> run the tests");
        // Collapsed by the user, it stays collapsed while it is open.
        assert!(o.back());
        o.set_dictation(Some(MessagePrompt {
            expanded: true,
            ..PromptInput::said("and push it").into()
        }));
        assert_eq!(o.prompts().expanded(), None);
        // Another session, left pinned: pinned.
        o.set_dictation(None);
        o.set_dictation(said("hello"));
        assert_eq!(o.prompts().expanded(), None);
        assert!(message(&o).h > 0.0);
    }

    #[test]
    fn the_keyboards_height_lifts_the_card_and_the_overlays_input() {
        let mut o = overview(3);
        o.set_dictation(typed("hello"));
        let down = o
            .rect_of(AgentHit::PromptText(PromptKind::Message))
            .unwrap();
        // The soft keyboard up: its inset is in the layout's bottom.
        let inset = 700.0;
        o.set_layout(Layout {
            bottom: PHONE.bottom + inset,
            ..PHONE
        });
        let up = o
            .rect_of(AgentHit::PromptText(PromptKind::Message))
            .unwrap();
        assert!((down.y - up.y - inset).abs() < 0.01, "lifted by the inset");
        assert!(up.y + up.h <= PHONE.h - PHONE.bottom - inset);
        // Expanded: the overlay's input sits above the keyboard too.
        o.with_prompts(|p| p.expand(PromptKind::Message));
        o.tick(PromptOverlay::SLIDE);
        let input = o
            .rect_of(AgentHit::PromptText(PromptKind::Message))
            .unwrap();
        assert!(input.y + input.h <= PHONE.h - PHONE.bottom - inset);
        let sheet = o.prompts().overlay().rect();
        assert!(sheet.y >= o.grid().viewport().y, "under the status lines");
    }

    #[test]
    fn overview_keyboard_prompt_card_has_shaded_input_area() {
        let mut o = overview(1);
        o.set_dictation(typed(""));
        let card = o.prompts().message();
        assert!(card.fills().iter().any(|f| f.color == theme().card));
        assert!(card.fills().iter().any(|f| f.color == theme().input));
        let ta = card.text_area().expect("the text area");
        assert!(ta.is_active());
        assert!(ta.text().is_empty(), "the placeholder shows");
    }

    #[test]
    fn keyboard_prompt_multiline_drops_font_size_to_fit() {
        let mut o = overview(1);
        o.set_dictation(typed("Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6"));
        let ta = o.prompts().message().text_area().unwrap();
        assert!(ta.font_size.unwrap() <= 12.0 * o.scale());
        assert!(ta.rect.h >= 3.0 * (12.0 * 1.3 * o.scale()));
    }

    #[test]
    fn tapping_text_area_hits_text_area_and_positions_cursor() {
        let mut o = overview(1);
        o.set_dictation(typed("Hello world"));
        let rect = o
            .rect_of(AgentHit::PromptText(PromptKind::Message))
            .expect("the text area");
        assert_eq!(
            o.hit(rect.x + 10.0, rect.y + 10.0),
            Some(AgentHit::PromptText(PromptKind::Message))
        );
        assert_eq!(
            tap(&mut o, rect.x + 10.0, rect.y + 10.0),
            Some(OverviewHit::Bottom(AgentHit::PromptText(
                PromptKind::Message
            )))
        );
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
    fn the_kebab_is_at_the_right_of_the_title_and_not_the_overviews_to_tap() {
        let mut o = overview(2);
        let k = o.kebab().rect;
        let title = &o.headings[0];
        assert_eq!(title.text, "flankr");
        assert!((k.x + k.w - (PHONE.w - PAD * 2.0)).abs() < 0.01);
        assert_eq!(k.w, crate::menu::Kebab::SIZE * PHONE.scale);
        assert!(k.y <= title.y + title.px && k.y + k.h >= title.y);
        // The app takes the title bar's taps before the overview.
        assert_eq!(tap(&mut o, k.x + k.w * 0.5, k.y + k.h * 0.5), None);
    }

    #[test]
    fn the_menu_is_at_the_left_of_the_title_and_opens_the_drawer() {
        let mut o = overview(2);
        let m = o.menu().rect;
        let title = &o.headings[0];
        assert_eq!(m.w, Menu::SIZE * 2.0);
        assert!(m.x < PAD * 2.0, "at the left edge");
        assert!(title.x.unwrap() >= m.x + m.w, "the title after it");
        assert!(m.y <= title.y + title.px && m.y + m.h >= title.y);
        assert_eq!(
            tap(&mut o, m.x + m.w * 0.5, m.y + m.h * 0.5),
            Some(OverviewHit::Menu)
        );
    }

    #[test]
    fn controls_the_settings_turn_off_are_dim_and_take_no_taps() {
        let mut o = overview(2);
        o.set_volume(ON, false);
        let ctx = DrawContext::default();
        let play = o.rect_of(AgentHit::Volume).unwrap();
        let record = o.rect_of(AgentHit::Message).unwrap();
        let mut list = DrawList::new();
        o.draw(&mut list, &ctx, None);
        assert_eq!(alpha_in(&list, play), 1.0);
        assert_eq!(alpha_in(&list, record), 1.0);

        for (settings, off) in [
            (with(false, true, true), AgentHit::Volume),
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
        // The message card being said, without the microphone.
        settings::set_current(with(true, false, true));
        o.set_dictation(said("Taking a message"));
        let mut list = DrawList::new();
        o.draw(&mut list, &ctx, None);
        let card = message(&o);
        assert!((alpha_in(&list, card) - settings::DISABLED_ALPHA).abs() < 1e-6);
        let send = o.rect_of(AgentHit::Submit).unwrap();
        assert_eq!(tap(&mut o, send.x + 5.0, send.y + 5.0), None);
        settings::set_current(settings::Settings::default());
    }

    #[test]
    fn without_the_general_agent_geminis_card_and_overlay_are_hidden() {
        let mut o = overview(4);
        talk(&mut o, 2);
        let card = gemini(&o);
        open(&mut o);
        settings::set_current(with(true, true, false));
        o.settings_changed();
        assert!(!o.prompts().overlay().is_open());
        // The cards take the card's room, and tapping there opens nothing.
        let grid = o.grid().viewport();
        assert!(grid.y + grid.h >= card.y + card.h - 0.01);
        tap(&mut o, 540.0, card.y + 10.0);
        assert!(!o.prompts().overlay().is_open());
        // Gemini's lines come in hidden, and show again with it on, as
        // the card was left: expanded.
        o.push_turn(Speaker::Gemini, "Still here.".into());
        let mut list = DrawList::new();
        o.draw(&mut list, &DrawContext::default(), None);
        assert!(o.grid().viewport().y + o.grid().viewport().h >= card.y + card.h - 0.01);
        settings::set_current(settings::Settings::default());
        o.settings_changed();
        assert!(o.prompts().overlay().is_open());
        o.back();
        assert!(gemini(&o).h > 0.0);
    }

    #[test]
    fn the_centre_button_and_geminis_card_follow_the_daemons_llm() {
        use flankr_companion::wire::LlmInfo;
        let mut o = overview(4);
        talk(&mut o, 2);
        let ctx = DrawContext::default();
        let mut list = DrawList::new();
        o.draw(&mut list, &ctx, None);
        assert!(o.rect_of(AgentHit::GeneralAgent).is_some());
        assert!(gemini(&o).h > 0.0);
        // The daemon turns it off while connected: both go.
        settings::set_llm(LlmInfo {
            general_agent: false,
            summaries: false,
            reason: Some("off".into()),
        });
        o.settings_changed();
        let mut list = DrawList::new();
        o.draw(&mut list, &ctx, None);
        assert!(o.rect_of(AgentHit::GeneralAgent).is_none(), "hidden");
        assert_eq!(gemini(&o).h, 0.0);
        assert!(!o.prompts().overlay().is_open());
        // And back.
        settings::set_llm(LlmInfo::available());
        o.settings_changed();
        let mut list = DrawList::new();
        o.draw(&mut list, &ctx, None);
        assert!(o.rect_of(AgentHit::GeneralAgent).is_some());
        assert!(gemini(&o).h > 0.0);
    }

    #[test]
    fn a_plugin_card_follows_the_agents_and_a_tap_or_a_hold_is_its() {
        let mut o = overview(2);
        o.set_plugin_cards(vec![160.0]);
        let (_, cards) = o.plugin_cards();
        assert_eq!(cards.len(), 1);
        let (i, r) = cards[0];
        assert_eq!(i, 0);
        let [x, y] = r.center();
        assert_eq!(
            tap(&mut o, x, y),
            Some(OverviewHit::PluginCard {
                index: 0,
                x,
                y,
                held: false
            })
        );
        o.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        assert_eq!(
            o.touch(1, TouchPhase::Ended, x, y, Duration::from_millis(2100)),
            Some(OverviewHit::PluginCard {
                index: 0,
                x,
                y,
                held: true
            })
        );
        // The agents' cards are still theirs.
        let p1 = o.grid().card_rect("p1").unwrap().center();
        assert_eq!(
            tap(&mut o, p1[0], p1[1]),
            Some(OverviewHit::Agent("p1".into()))
        );
    }
}
