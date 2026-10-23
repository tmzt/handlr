//! The overview, the home screen (plan 20261023_04, as amended at 17:04
//! EDT). From the top: the title and the status lines; then two halves
//! that scroll on their own, the agent cards above ([`CardGrid`]) and the
//! Gemini turns below ([`TurnLog`]); then, above the bottom row, the
//! confirmation card while one is shown and the dictation card while a
//! message is taken; and the bottom row of the agent view (decision 5):
//! play at the left, record at the right, record becoming send with a
//! cancel in the middle while a message is taken.
//!
//! Tapping a card opens the agent view on that pane (decision 4). Play
//! reads replies from an agent view's pane only, so it is dim and does
//! nothing until a view has been opened; record takes a message for
//! herdr's focused agent. A tap on nothing is the main screen's tap.

pub mod agent_card;
pub mod card_grid;
pub mod turn_log;

use std::time::Duration;

use handlr_companion::Speaker;
use libcontrols::{DrawContext, Rect};
use libhandlr_proto::common::AgentInfo;
use libmorph::TouchPhase;
use libmsdf::drawlist::DrawList;

use crate::agent::{
    ACTION_H, APPROVE_BG, AgentHit, BAND, BG, BUTTON_H, CARD_BG, DENY_BG, Fill, Flow, FlowButton,
    PAD, Row, Terminal, columns, fill, line_h,
};
use crate::controls::scroll::Touched;
use crate::controls::{ControlRow, SquareButton};
use crate::ui::{Layout, Line, MUTED, TEXT, wrap};
pub use card_grid::CardGrid;
pub use turn_log::TurnLog;

/// What a touch on the overview did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverviewHit {
    /// A card: open the agent view on this pane.
    Agent(String),
    /// The bottom row, or a confirmation's Approve or Deny.
    Bottom(AgentHit),
    /// A tap on no control.
    Tap,
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
    Log,
    /// A control, or no half.
    Other,
}

/// One centred line at the top.
#[derive(Clone, Debug, PartialEq)]
struct Heading {
    text: String,
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
    log: TurnLog,
    card: Option<Panel>,
    dictation: Option<Vec<Line>>,
    read_aloud: bool,
    speaking: bool,
    /// An agent view has been opened: play reads its replies.
    play_live: bool,
    headings: Vec<Heading>,
    /// The confirmation and dictation cards as placed.
    panels: Vec<Flow>,
    buttons: Vec<FlowButton>,
    /// The line between the halves.
    divider: Rect,
    hits: Vec<(AgentHit, Rect)>,
    touch: Option<(u64, Target)>,
    loaded: bool,
    dirty: bool,
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
            log: TurnLog::new(),
            card: None,
            dictation: None,
            read_aloud: false,
            speaking: false,
            play_live: false,
            headings: Vec::new(),
            panels: Vec::new(),
            buttons: Vec::new(),
            divider: Rect::default(),
            hits: Vec::new(),
            touch: None,
            loaded: false,
            dirty: true,
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

    /// The dictation card's lines, while a message is taken.
    pub fn set_dictation(&mut self, lines: Option<Vec<Line>>) {
        if lines != self.dictation {
            self.dictation = lines;
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
        self.log.push(speaker, text);
        self.dirty = true;
    }

    /// What STT is hearing, at the end of the conversation.
    pub fn set_partial(&mut self, partial: Option<String>) {
        if self.log.set_partial(partial) {
            self.dirty = true;
        }
    }

    pub fn grid(&self) -> &CardGrid {
        &self.grid
    }

    pub fn log(&self) -> &TurnLog {
        &self.log
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
        let mut heading = |text: &str, pt: f32, color, y: &mut f32| {
            headings.push(Heading {
                text: text.to_string(),
                y: *y,
                px: pt * s,
                color,
            });
            *y += pt * s * 1.35;
        };
        heading("handlr", TITLE_PT, TEXT, &mut y);
        for line in &self.status {
            heading(line, STATUS_PT, MUTED, &mut y);
        }
        if let Some((note, color)) = &self.note {
            heading(note, STATUS_PT, *color, &mut y);
        }
        self.headings = headings;
        let top = y + 8.0 * s;

        // The bottom row, then the cards above it.
        let row_bottom = l.h - l.bottom - 16.0 * s;
        let dictating = self.dictation.is_some();
        self.buttons = ControlRow::lay_out(pad, l.w - pad, row_bottom, s, dictating);
        let mut above = row_bottom - BUTTON_H * s - 12.0 * s;
        let mut panels = Vec::new();
        // While a confirmation asks, the message makes way for it.
        let asking = self.card.as_ref().is_some_and(|c| c.confirm.is_some());
        if let Some(lines) = self.dictation.as_ref().filter(|_| !asking) {
            let p = panel(lines, None, pad, l.w - 2.0 * pad, above, s);
            above -= p.height + 8.0 * s;
            panels.push(p);
        }
        if let Some(card) = &self.card {
            let p = panel(&card.lines, card.confirm, pad, l.w - 2.0 * pad, above, s);
            above -= p.height + 8.0 * s;
            panels.push(p);
        }
        self.panels = panels;

        // The halves, split in the middle.
        let bottom = above.max(top);
        let mid = (top + bottom) * 0.5;
        let gap = 4.0 * s;
        self.grid
            .set_viewport(Rect::new(0.0, top, l.w, (mid - gap - top).max(0.0)), s);
        self.log.set_viewport(
            Rect::new(0.0, mid + gap, l.w, (bottom - mid - gap).max(0.0)),
            s,
        );
        self.divider = Rect::new(pad, mid, l.w - 2.0 * pad, 1.0);

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

    /// A touch at `now` (any monotonic clock). A drag scrolls the half it
    /// started in; a tap returns what it hit.
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
            } else if self.grid.viewport().contains([x, y]) {
                Target::Grid
            } else if self.log.viewport().contains([x, y]) {
                Target::Log
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
            Target::Log => self.log.touch(id, phase, x, y, now),
            Target::Other => match phase {
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
                Target::Log => Some(OverviewHit::Tap),
                Target::Other => match self.hit(x, y) {
                    // Nothing to read until a view has been opened.
                    Some(AgentHit::ReadAloud) if !self.play_live => None,
                    Some(hit) => Some(OverviewHit::Bottom(hit)),
                    None => Some(OverviewHit::Tap),
                },
            },
        }
    }

    /// Carries the halves' flings on. Returns whether either moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let moved = self.grid.tick(dt) | self.log.tick(dt);
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
        fill(list, Rect::new(0.0, 0.0, l.w, l.h), BG, 0.0);
        for h in &self.headings {
            let w = ctx.text_advance(&h.text, h.px);
            ctx.draw_text(list, &h.text, [(l.w - w) * 0.5, h.y], h.px, h.color);
        }
        self.grid.draw(list, ctx);
        fill(list, self.divider, BAND, 0.0);
        self.log.draw(list, ctx);
        let recording = self.dictation.is_some();
        let read_aloud = self.read_aloud && self.play_live;
        let draw_button = |list: &mut DrawList, b: &FlowButton| {
            SquareButton::draw(
                list,
                ctx,
                term,
                b,
                b.rect,
                s,
                read_aloud,
                self.speaking,
                recording,
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
        for b in &self.buttons {
            draw_button(list, b);
        }
        self.dirty = false;
        true
    }
}

/// `lines` on a card `w` wide from `x`, its bottom at `bottom`, with
/// Approve and Deny under them when `confirm` is given.
fn panel(lines: &[Line], confirm: Option<u64>, x: f32, w: f32, bottom: f32, s: f32) -> Flow {
    let pad = PAD * s;
    let inner_x = x + pad;
    let inner_w = w - 2.0 * pad;
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
    let actions = if confirm.is_some() {
        ACTION_H * s + pad
    } else {
        0.0
    };
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
        color: CARD_BG,
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
            (AgentHit::Approve(id), "Approve", APPROVE_BG),
            (AgentHit::Deny(id), "Deny", DENY_BG),
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
        // Between the halves is no card: a tap.
        let divider = o.divider;
        assert_eq!(tap(&mut o, 540.0, divider.y + 1.0), Some(OverviewHit::Tap));
    }

    #[test]
    fn the_two_halves_scroll_apart() {
        let mut o = overview(16);
        for i in 0..30 {
            o.push_turn(Speaker::User, format!("question {i}"));
            o.push_turn(Speaker::Gemini, format!("Answer {i}, at some length."));
        }
        let grid = o.grid().viewport();
        let log = o.log().viewport();
        assert!(grid.y + grid.h <= log.y, "cards above the turns");
        assert!(o.grid().scroll().max() > 0.0 && o.log().scroll().max() > 0.0);
        let log_at = o.log().scroll().offset();
        assert_eq!(log_at, o.log().scroll().max(), "following the newest");
        // A drag in the cards moves only the cards; ending on a card, it
        // is a scroll, not a tap.
        assert_eq!(drag_up(&mut o, 540.0, grid.y + grid.h - 40.0, 250.0), None);
        assert!((o.grid().scroll().offset() - 250.0).abs() < 1.0);
        assert_eq!(o.log().scroll().offset(), log_at);
        // One in the turns moves only the turns, back from the newest.
        o.touch(2, TouchPhase::Started, 540.0, log.y + 40.0, Duration::ZERO);
        let mut now = Duration::ZERO;
        for i in 1..=10 {
            now += MS;
            o.touch(
                2,
                TouchPhase::Moved,
                540.0,
                log.y + 40.0 + 30.0 * i as f32,
                now,
            );
        }
        o.touch(
            2,
            TouchPhase::Ended,
            540.0,
            log.y + 340.0,
            now + Duration::from_millis(300),
        );
        assert!((log_at - o.log().scroll().offset() - 300.0).abs() < 1.0);
        assert!((o.grid().scroll().offset() - 250.0).abs() < 1.0);
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
        assert!(o.log().viewport().y + o.log().viewport().h <= record.y);
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
        o.set_dictation(Some(vec![Line {
            text: "Taking a message".into(),
            size: 15.0,
            color: MUTED,
            gap: 28.0,
        }]));
        let send = o.rect_of(AgentHit::Submit).unwrap();
        assert!(send.x + send.w > 980.0);
        assert!(o.rect_of(AgentHit::Clear).is_some());
        assert!(o.rect_of(AgentHit::Message).is_none());
        let card = &o.panels[0].fills[0].rect;
        assert!(card.y + card.h <= send.y);
        assert!(o.log().viewport().y + o.log().viewport().h <= card.y);
    }

    #[test]
    fn a_confirmation_card_answers_its_own_id() {
        let mut o = overview(1);
        let line = |text: &str| Line {
            text: text.into(),
            size: 22.0,
            color: TEXT,
            gap: 4.0,
        };
        o.set_dictation(Some(vec![line("Please run the tests.")]));
        o.set_card(Some(Panel {
            lines: vec![line("Approve?"), line("\"continue\" to Claude in handlr")],
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
}
