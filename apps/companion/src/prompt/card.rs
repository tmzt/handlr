//! The prompt card (plan 20261027_03, decisions 1 to 6), built from the
//! agent view's `DictationCard`, the overview's `panel()` and its
//! `TurnStrip`: one control in two modes.
//!
//! - *General agent*: header "✦ Gemini" in the Gemini accent; the latest
//!   exchange (the user's words, or what is being heard, or "Listening…";
//!   and Gemini's newest line); the text area while a typed prompt is open.
//! - *Message*: header "Message to <agent>" in that session's colour (the
//!   L3 amendment of 2026-10-24 15:56 EDT); where it stands and any
//!   warning; the text being said, or the text area while typing. Replying
//!   to a card, the card's title and text come first and the controls are
//!   inside it (owner, 2026-10-23).
//!
//! The typed input is the L3 prompt card's: a shaded box (`theme().input`)
//! at least 3 lines of 12 pt, growing with its text and dropping to 11 and
//! 10 pt to fit (the amendments of 2026-10-25 17:53 and 23:41 EDT), with
//! the placeholder while it is empty. An up arrow (˄), drawn as strokes, at
//! the top right expands the card; nothing else does (decision 4). Under
//! the box, a row of the + and the keyboard toggle ([`InputRow`], plan
//! 20261028_06, decision 8); the card grows by 48 pt while it shows.

use flankr_companion::Speaker;
use libcontrols::{DrawContext, DrawControl, Rect, TextArea};
use libmsdf::drawlist::DrawList;

use super::bars::Bars;
use super::history::{History, Sent};
use super::input_row::{InputRow, InputTools, ROW_GAP};
use super::{PromptInput, PromptKind};
use crate::agent::{
    AgentHit, BUTTON_H, Fill, FlowButton, PAD, Row, columns, fill, line_h, one_line, wrap_hard,
};
use crate::controls::{ControlRow, RowState};
use crate::settings;
use crate::settings::copy;
use crate::theme::theme;

/// The accent edge at a card's left, and the type sizes of a replied
/// card's title and text, as the agent view's cards have them.
const EDGE: f32 = 6.0;
const TITLE_PT: f32 = 15.0;
const CARD_PT: f32 = 19.0;
/// Lines of a replied card's text, as the pinned card's.
const PIN_LINES: usize = 3;
/// The header's and the latest exchange's type sizes, in points.
const HEADER_PT: f32 = 13.0;
const EXCHANGE_PT: f32 = 15.0;
/// Lines of Gemini's newest reply shown pinned.
const REPLY_LINES: usize = 2;
/// Dictated text: its size, and the lines shown at most (its tail).
const SAID_PT: f32 = 20.0;
const SAID_LINES: usize = 6;
/// The typed input: its size, and the lines its box holds at least.
const TYPED_PT: f32 = 12.0;
const TYPED_LINES: usize = 3;
/// The up arrow's square at the card's top right, in points.
const ARROW: f32 = 36.0;

/// The card a message replies to: its title and text, and its accent.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub title: String,
    pub text: String,
    pub accent: [f32; 4],
}

/// Drops the type size as the text wraps onto more lines, so a multi-line
/// message fits its box: 12 pt for 3 lines, 11 for 4, then 10.
fn fit_font_size(text: &str, base_pt: f32, box_w: f32, s: f32) -> (f32, Vec<String>) {
    let base = base_pt.min(12.0);
    for pt in [base, 11.0, 10.0] {
        if pt > base {
            continue;
        }
        let mut rows = wrap_hard(text, columns(box_w, pt * s));
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
    (10.0, vec![String::new()])
}

/// A prompt card: what it shows, whether it is expanded, and where it is.
pub struct PromptCard {
    kind: PromptKind,
    /// The user's choice: expanded into the overlay, else pinned. Kept
    /// while the card is closed or hidden, and opened that way again.
    expanded: bool,
    /// *General agent*: shown by the 2 s hold with nothing in it yet.
    opened: bool,
    /// *Message*: the agent it goes to, as shown, and its session colour.
    target: Option<(String, [f32; 4])>,
    reply: Option<Reply>,
    input: Option<PromptInput>,
    /// *General agent*: the user's last words, Gemini's newest line, what
    /// STT is hearing, and whether it listens.
    user: Option<String>,
    gemini: Option<String>,
    partial: Option<String>,
    listening: bool,
    history: History,
    // As laid out.
    rect: Rect,
    arrow: Rect,
    fills: Vec<Fill>,
    rows: Vec<Row>,
    text_area: Option<TextArea>,
    buttons: Vec<FlowButton>,
    bars: Bars,
    scale: f32,
    /// Typing's tools as the app gives them, and the row under the typed
    /// box as laid out (plan 20261028_06, decision 8).
    tools: InputTools,
    row: Option<InputRow>,
}

impl PromptCard {
    pub fn new(kind: PromptKind) -> Self {
        let mut history = History::new();
        if kind == PromptKind::Message {
            history.set_empty(Some(copy::PROMPT_NO_MESSAGES.text.to_string()));
        }
        PromptCard {
            kind,
            expanded: false,
            opened: false,
            target: None,
            reply: None,
            input: None,
            user: None,
            gemini: None,
            partial: None,
            listening: false,
            history,
            rect: Rect::default(),
            arrow: Rect::default(),
            fills: Vec::new(),
            rows: Vec::new(),
            text_area: None,
            buttons: Vec::new(),
            bars: Bars::default(),
            scale: 1.0,
            tools: InputTools::default(),
            row: None,
        }
    }

    /// Typing's tools: whether the row has the + and the toggle, and the
    /// toggle's state. Returns whether they changed.
    pub fn set_tools(&mut self, tools: InputTools) -> bool {
        let changed = tools != self.tools;
        self.tools = tools;
        changed
    }

    pub fn tools(&self) -> InputTools {
        self.tools
    }

    /// The row under the typed box, as laid out.
    pub fn row(&self) -> Option<&InputRow> {
        self.row.as_ref()
    }

    pub fn kind(&self) -> PromptKind {
        self.kind
    }

    /// Whether the card has anything to show: a message being taken; for
    /// the general agent, any exchange, listening, a typed prompt, or the
    /// hold having opened it.
    pub fn has_content(&self) -> bool {
        match self.kind {
            PromptKind::Message => self.input.is_some(),
            PromptKind::GeneralAgent => {
                self.opened
                    || self.input.is_some()
                    || self.listening
                    || self.partial.is_some()
                    || self.user.is_some()
                    || self.gemini.is_some()
            }
        }
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// The user's choice of pinned or expanded, remembered.
    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    /// Shows the general agent's card pinned, even with nothing in it.
    pub fn open_pinned(&mut self) {
        self.opened = true;
        self.expanded = false;
    }

    /// The header and its colour (decision 1).
    pub fn header(&self) -> (String, [f32; 4]) {
        match (self.kind, &self.target) {
            (PromptKind::GeneralAgent, _) => (copy::PROMPT_GEMINI.text.to_string(), theme().accent),
            (PromptKind::Message, Some((agent, colour))) => (
                copy::PROMPT_MESSAGE
                    .fill(None, &[("agent", agent.as_str())])
                    .trim()
                    .to_string(),
                *colour,
            ),
            (PromptKind::Message, None) => (
                copy::PROMPT_MESSAGE_UNTARGETED.text.to_string(),
                theme().text,
            ),
        }
    }

    /// The typed input's placeholder.
    pub fn placeholder(&self) -> String {
        match self.kind {
            PromptKind::GeneralAgent => copy::PROMPT_ASK_GEMINI.text,
            PromptKind::Message => copy::PROMPT_TYPE_MESSAGE.text,
        }
        .to_string()
    }

    /// *Message*: the agent it goes to, as shown, and its colour. Returns
    /// whether it changed.
    pub fn set_target(&mut self, target: Option<(String, [f32; 4])>) -> bool {
        let changed = target != self.target;
        self.target = target;
        changed
    }

    /// *Message*: the card it replies to. Returns whether it changed.
    pub fn set_reply(&mut self, reply: Option<Reply>) -> bool {
        let changed = reply != self.reply;
        self.reply = reply;
        changed
    }

    pub fn reply(&self) -> Option<&Reply> {
        self.reply.as_ref()
    }

    /// What is being said or typed; `None` once sent or cancelled.
    /// Returns whether it changed.
    pub fn set_input(&mut self, input: Option<PromptInput>) -> bool {
        let changed = input != self.input;
        self.input = input;
        changed
    }

    pub fn input(&self) -> Option<&PromptInput> {
        self.input.as_ref()
    }

    pub fn input_text(&self) -> String {
        self.input
            .as_ref()
            .map(|i| i.text.clone())
            .unwrap_or_default()
    }

    /// A line of the conversation with Gemini: in the history, and the
    /// newest of its speaker in the latest exchange.
    pub fn push_turn(&mut self, speaker: Speaker, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match speaker {
            Speaker::User => self.user = Some(text.to_string()),
            Speaker::Gemini => self.gemini = Some(text.to_string()),
        }
        self.history.push(speaker, text.to_string());
    }

    /// The whole conversation at once, as the app keeps it: for a screen's
    /// card as the screen opens.
    pub fn set_turns(&mut self, turns: &[(Speaker, String)]) {
        let newest = |who: Speaker| {
            turns
                .iter()
                .rev()
                .find(|(s, t)| *s == who && !t.trim().is_empty())
                .map(|(_, t)| t.trim().to_string())
        };
        self.user = newest(Speaker::User);
        self.gemini = newest(Speaker::Gemini);
        let lines = turns
            .iter()
            .filter(|(_, t)| !t.trim().is_empty())
            .map(|(s, t)| super::history::Line::Turn(*s, t.trim().to_string()))
            .collect();
        self.history.set_lines(lines);
    }

    /// Whether it listens for the general agent.
    pub fn listening(&self) -> bool {
        self.listening
    }

    /// What STT is hearing. Returns whether it changed.
    pub fn set_partial(&mut self, partial: Option<String>) -> bool {
        let partial = partial.filter(|p| !p.trim().is_empty());
        let changed = partial != self.partial;
        self.partial = partial.clone();
        self.history.set_partial(partial) || changed
    }

    /// Whether it listens for the general agent. Returns whether it
    /// changed.
    pub fn set_listening(&mut self, listening: bool) -> bool {
        let changed = listening != self.listening;
        self.listening = listening;
        changed
    }

    /// *Message*: a message sent, in the history.
    pub fn push_sent(&mut self, sent: Sent) {
        self.history.push_sent(sent);
    }

    /// *Message*: the session's messages, as the app keeps them. Returns
    /// whether they changed.
    pub fn set_sent(&mut self, sent: &[Sent]) -> bool {
        self.history.set_lines(
            sent.iter()
                .cloned()
                .map(super::history::Line::Sent)
                .collect(),
        )
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    /// The input bar's line: what is heard or typed, "Listening…", else
    /// the user's last words.
    fn input_line(&self) -> Option<String> {
        if let Some(p) = &self.partial {
            return Some(format!("> {}\u{2026}", p.trim()));
        }
        if self.listening {
            return Some(copy::PROMPT_LISTENING.text.to_string());
        }
        if let Some(i) = self.input.as_ref().filter(|i| !i.text.trim().is_empty()) {
            return Some(format!("> {}", i.text.trim().replace('\n', " ")));
        }
        self.user.as_ref().map(|u| format!("> {u}"))
    }

    /// The output bar's line: Gemini's newest.
    fn output_line(&self) -> Option<String> {
        self.gemini
            .as_ref()
            .map(|g| format!("< {}", g.replace('\n', " ")))
    }

    /// Nothing laid out: the card is not pinned (closed, hidden or
    /// expanded).
    pub fn clear_layout(&mut self) {
        self.rect = Rect::default();
        self.arrow = Rect::default();
        self.fills.clear();
        self.rows.clear();
        self.text_area = None;
        self.buttons.clear();
        self.bars.clear();
        self.row = None;
    }

    /// *General agent*, hidden: its two bars, the lower's bottom at
    /// `bottom`. Returns the height taken.
    pub fn lay_out_bars(&mut self, x: f32, w: f32, bottom: f32, s: f32) -> f32 {
        self.scale = s;
        let (input, output) = (self.input_line(), self.output_line());
        self.bars.lay_out(input, output, x, w, bottom, s)
    }

    pub fn bars(&self) -> &Bars {
        &self.bars
    }

    /// Lays the pinned card out, `x` to `x + w`, its bottom at `bottom`.
    /// Returns its height.
    pub fn lay_out(&mut self, x: f32, w: f32, bottom: f32, s: f32) -> f32 {
        let cursor = self.text_area.as_ref().map(|ta| (ta.text(), ta.cursor()));
        self.clear_layout();
        self.scale = s;
        let pad = PAD * s;
        let inner_x = x + EDGE * s + pad;
        let inner_w = w - EDGE * s - 2.0 * pad;
        let (header, colour) = self.header();
        // (text, px, colour, gap above)
        let mut rows: Vec<(String, f32, [f32; 4], f32)> = Vec::new();
        let head_w = inner_w - ARROW * s;
        rows.push((
            one_line(&header, columns(head_w, HEADER_PT * s)),
            HEADER_PT * s,
            colour,
            0.0,
        ));
        let wrapped = |rows: &mut Vec<(String, f32, [f32; 4], f32)>,
                       text: &str,
                       pt: f32,
                       color: [f32; 4],
                       gap: f32,
                       most: usize| {
            let px = pt * s;
            let mut lines = wrap_hard(text.trim(), columns(inner_w, px));
            if lines.len() > most {
                lines.truncate(most);
                if let Some(last) = lines.last_mut() {
                    last.push('\u{2026}');
                }
            }
            for (i, t) in lines.into_iter().enumerate() {
                rows.push((t, px, color, if i == 0 { gap * s } else { 0.0 }));
            }
        };
        if let Some(r) = &self.reply {
            wrapped(&mut rows, &r.title, TITLE_PT, r.accent, 6.0, 1);
            wrapped(&mut rows, &r.text, CARD_PT, theme().fg, 4.0, PIN_LINES);
        }
        if self.kind == PromptKind::GeneralAgent {
            let first = if let Some(p) = &self.partial {
                Some((format!("> {}\u{2026}", p.trim()), theme().fg))
            } else if self.listening {
                Some((copy::PROMPT_LISTENING.text.to_string(), theme().muted))
            } else {
                self.user
                    .as_ref()
                    .map(|u| (format!("> {}", u.replace('\n', " ")), theme().text))
            };
            if let Some((line, color)) = first {
                wrapped(&mut rows, &line, EXCHANGE_PT, color, 4.0, 1);
            }
            if let Some(g) = &self.gemini {
                wrapped(
                    &mut rows,
                    &format!("< {g}"),
                    EXCHANGE_PT,
                    theme().fg,
                    2.0,
                    REPLY_LINES,
                );
            }
        }
        let mut typed = None;
        if let Some(input) = &self.input {
            if let Some((status, color)) = &input.status {
                wrapped(&mut rows, status, EXCHANGE_PT, *color, 4.0, 2);
            }
            for (note, color) in &input.notes {
                wrapped(&mut rows, note, EXCHANGE_PT, *color, 2.0, 2);
            }
            if input.keyboard() {
                typed = Some(input.text.clone());
            } else if self.kind == PromptKind::Message {
                // What is being said, its tail; the placeholder while
                // nothing is.
                let (text, color) = if input.text.trim().is_empty() {
                    ("(say your message)".to_string(), theme().muted)
                } else {
                    (input.text.clone(), theme().text)
                };
                // As the dictation card had it: fitted from 12 pt down.
                let (pt, _) = fit_font_size(&text, SAID_PT, inner_w, s);
                let px = pt * s;
                let lines = wrap_hard(text.trim(), columns(inner_w, px));
                let skip = lines.len().saturating_sub(SAID_LINES);
                for (i, t) in lines.into_iter().skip(skip).enumerate() {
                    rows.push((t, px, color, if i == 0 { 8.0 * s } else { 0.0 }));
                }
            }
        } else if self.kind == PromptKind::GeneralAgent && self.opened && rows.len() == 1 {
            let placeholder = self.placeholder();
            wrapped(&mut rows, &placeholder, EXCHANGE_PT, theme().muted, 4.0, 1);
        }
        // The typed input's box: at least three lines.
        let box_pad = [10.0 * s, 8.0 * s];
        let typed_box = typed.map(|text| {
            let (pt, lines) = fit_font_size(&text, TYPED_PT, inner_w - 2.0 * box_pad[0], s);
            let px = pt * s;
            let h = lines.len().max(TYPED_LINES) as f32 * line_h(px) + 2.0 * box_pad[1];
            (text, px, h)
        });
        let controls = if self.reply.is_some() {
            BUTTON_H * s + pad
        } else {
            0.0
        };
        let rows_h: f32 = rows.iter().map(|(_, px, _, gap)| line_h(*px) + gap).sum();
        // The row under the typed box grows the card by 48 pt.
        let row_h = if typed_box.is_some() {
            self.tools.grows(self.kind, s)
        } else {
            0.0
        };
        let box_h = typed_box.as_ref().map_or(0.0, |(_, _, h)| h + 8.0 * s) + row_h;
        let h = rows_h + box_h + 2.0 * pad + controls;
        let top = bottom - h;
        let rect = Rect::new(x, top, w, h);
        if self.reply.is_some() {
            self.fills.push(Fill {
                rect: Rect::new(x + 3.0 * s, top + 4.0 * s, w, h),
                color: theme().shadow,
                radius: 10.0 * s,
            });
        }
        self.fills.push(Fill {
            rect,
            color: theme().card,
            radius: 10.0 * s,
        });
        let edge = self.reply.as_ref().map_or(colour, |r| r.accent);
        self.fills.push(Fill {
            rect: Rect::new(x, top, EDGE * s, h),
            color: edge,
            radius: 3.0 * s,
        });
        let mut y = top + pad;
        for (text, px, color, gap) in rows {
            y += gap;
            self.rows.push(Row {
                text,
                x: inner_x,
                y,
                px,
                color,
                mono: false,
            });
            y += line_h(px);
        }
        if let Some((text, px, bh)) = typed_box {
            let r = Rect::new(inner_x, y + 8.0 * s, inner_w, bh);
            self.fills.push(Fill {
                rect: r,
                color: theme().input,
                radius: 8.0 * s,
            });
            let ta = TextArea::new(text.clone(), r)
                .with_padding(box_pad)
                .with_font_size(px)
                .with_text_color(theme().text)
                .with_bg(theme().input)
                .with_corner_radius(8.0 * s)
                .with_placeholder(self.placeholder())
                .with_active(true);
            // The cursor stays where it was put while the text is the same.
            match cursor {
                Some((was, at)) if was == text => ta.set_cursor(at),
                _ => ta.set_cursor(text.len()),
            }
            self.text_area = Some(ta);
            self.row = InputRow::lay_out(
                self.kind,
                inner_x,
                r.y + bh + ROW_GAP * s,
                inner_w,
                s,
                &self.tools,
            );
        }
        let a = ARROW * s;
        self.arrow = Rect::new(x + w - a - 2.0 * s, top + 2.0 * s, a, a);
        if self.reply.is_some() {
            self.buttons = ControlRow::lay_out(
                inner_x,
                x + w - pad,
                bottom - pad,
                s,
                &RowState::three(true, false),
            );
        }
        self.rect = rect;
        h
    }

    /// The card as placed; empty while it is not pinned.
    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn fills(&self) -> &[Fill] {
        &self.fills
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn text_area(&self) -> Option<&TextArea> {
        self.text_area.as_ref()
    }

    /// A reply's controls, inside the card: the screen draws them.
    pub fn buttons(&self) -> &[FlowButton] {
        &self.buttons
    }

    fn pinned(&self) -> bool {
        self.rect.h > 0.0
    }

    /// Whether `(x, y)` is on the pinned card or the bars.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        (self.pinned() && self.rect.contains([x, y])) || self.bars.contains(x, y)
    }

    /// The control at `(x, y)` on the pinned card: the arrow, the text
    /// area (its cursor placed there), or a reply's controls.
    pub fn hit(&self, x: f32, y: f32) -> Option<AgentHit> {
        if !self.pinned() {
            return None;
        }
        if let Some(ta) = &self.text_area
            && ta.rect.contains([x, y])
        {
            ta.set_cursor_from_pos([x, y]);
            return Some(AgentHit::PromptText(self.kind));
        }
        if let Some(hit) = self.row.as_ref().and_then(|r| r.hit(x, y, 0.0)) {
            return Some(hit);
        }
        if self.arrow.contains([x, y]) {
            return Some(AgentHit::PromptExpand(self.kind));
        }
        self.buttons
            .iter()
            .find(|b| b.rect.contains([x, y]))
            .map(|b| b.hit)
    }

    /// Where `hit` is on the pinned card, if it shows.
    pub fn rect_of(&self, hit: AgentHit) -> Option<Rect> {
        if !self.pinned() {
            return None;
        }
        match hit {
            AgentHit::PromptExpand(kind) if kind == self.kind => Some(self.arrow),
            AgentHit::PromptText(kind) if kind == self.kind => {
                self.text_area.as_ref().map(|ta| ta.rect)
            }
            AgentHit::PromptAttach(_) | AgentHit::PromptIme(_) => {
                self.row.as_ref().and_then(|r| r.rect_of(hit, 0.0))
            }
            _ => self.buttons.iter().find(|b| b.hit == hit).map(|b| b.rect),
        }
    }

    /// Draws the pinned card, or the bars; not its buttons. A message being
    /// said without the microphone is dim (voice only).
    pub fn draw(&self, list: &mut DrawList, ctx: &DrawContext) {
        self.bars.draw(list, ctx);
        if !self.pinned() {
            return;
        }
        let start = list.instances.len();
        let s = self.scale;
        for f in &self.fills {
            fill(list, f.rect, f.color, f.radius);
        }
        for r in &self.rows {
            ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
        }
        if let Some(ta) = &self.text_area {
            ta.draw(list, ctx);
        }
        if let Some(row) = &self.row {
            row.draw(list, 0.0);
        }
        up_arrow(list, self.arrow, s, theme().dim);
        let voice = self.input.as_ref().is_some_and(|i| !i.keyboard());
        if self.kind == PromptKind::Message && voice && !settings::current().microphone {
            settings::dim_since(list, start);
        }
    }
}

/// An up arrow (˄) centred in `rect`, as two strokes: the faces may lack
/// the glyph.
pub fn up_arrow(list: &mut DrawList, rect: Rect, s: f32, color: [f32; 4]) {
    let (cx, cy) = (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    let (half, rise) = (7.0 * s, 4.0 * s);
    let t = 2.0 * s;
    list.push_line([cx - half, cy + rise], [cx, cy - rise], t, color);
    list.push_line([cx, cy - rise], [cx + half, cy + rise], t, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use flankr_companion::PromptMode;

    fn laid(card: &mut PromptCard) -> f32 {
        card.lay_out(24.0, 1032.0, 2000.0, 2.0)
    }

    fn texts(card: &PromptCard) -> Vec<&str> {
        card.rows().iter().map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn the_general_agent_pinned_shows_its_header_and_the_latest_exchange() {
        let mut c = PromptCard::new(PromptKind::GeneralAgent);
        assert!(!c.has_content());
        c.push_turn(Speaker::User, "what is claude doing");
        c.push_turn(Speaker::Gemini, "Claude is running the tests.");
        laid(&mut c);
        assert_eq!(
            texts(&c),
            [
                "\u{2726} Gemini",
                "> what is claude doing",
                "< Claude is running the tests."
            ]
        );
        assert_eq!(c.rows()[0].color, theme().accent);
        assert!((c.rect().y + c.rect().h - 2000.0).abs() < 0.01);
        // The arrow at its top right.
        let arrow = c
            .rect_of(AgentHit::PromptExpand(PromptKind::GeneralAgent))
            .unwrap();
        assert!(arrow.x + arrow.w <= c.rect().x + c.rect().w);
        assert!(arrow.x > c.rect().x + c.rect().w * 0.8);
        assert!((arrow.y - c.rect().y).abs() < 10.0);
        assert_eq!(
            c.hit(arrow.x + 5.0, arrow.y + 5.0),
            Some(AgentHit::PromptExpand(PromptKind::GeneralAgent))
        );
        // Heard, then listening with nothing heard.
        c.set_partial(Some("and co".into()));
        laid(&mut c);
        assert_eq!(texts(&c)[1], "> and co\u{2026}");
        c.set_partial(None);
        c.set_listening(true);
        laid(&mut c);
        assert_eq!(texts(&c)[1], "Listening\u{2026}");
        assert_eq!(c.rows()[1].color, theme().muted);
        let mut list = DrawList::new();
        c.draw(&mut list, &DrawContext::default());
        assert!(!list.instances.is_empty());
    }

    #[test]
    fn a_message_is_headed_in_its_sessions_colour() {
        let mut c = PromptCard::new(PromptKind::Message);
        assert!(!c.has_content());
        let colour = theme().session(3);
        c.set_target(Some(("Claude".into(), colour)));
        c.set_input(Some(PromptInput {
            status: Some(("Taking a message".into(), theme().muted)),
            ..PromptInput::said("please go on")
        }));
        assert!(c.has_content());
        laid(&mut c);
        assert_eq!(
            texts(&c),
            ["Message to Claude", "Taking a message", "please go on"]
        );
        assert_eq!(c.rows()[0].color, colour);
        assert!(c.fills().iter().any(|f| f.color == colour), "its edge");
        assert!(c.text_area().is_none());
        // Nothing said yet: the placeholder, dim.
        c.set_input(Some(PromptInput::said("")));
        laid(&mut c);
        assert_eq!(texts(&c).last(), Some(&"(say your message)"));
        // No agent to go to.
        c.set_target(None);
        laid(&mut c);
        assert_eq!(texts(&c)[0], "Message");
    }

    #[test]
    fn typing_has_a_shaded_box_of_three_lines_at_12_pt_that_grows() {
        for kind in [PromptKind::Message, PromptKind::GeneralAgent] {
            let mut c = PromptCard::new(kind);
            c.set_input(Some(PromptInput::typed("")));
            laid(&mut c);
            let ta = c.text_area().expect("the text area");
            assert!(c.fills().iter().any(|f| f.color == theme().input));
            assert!(c.fills().iter().any(|f| f.color == theme().card));
            assert!(ta.is_active());
            assert_eq!(ta.font_size, Some(12.0 * 2.0));
            let three = ta.rect.h;
            assert!(three >= 3.0 * (12.0 * 1.3 * 2.0));
            assert_eq!(
                c.hit(ta.rect.x + 10.0, ta.rect.y + 10.0),
                Some(AgentHit::PromptText(kind))
            );
            // Six lines: smaller type, a taller box.
            c.set_input(Some(PromptInput {
                mode: PromptMode::Keyboard,
                text: "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\nLine 6".into(),
                ..Default::default()
            }));
            laid(&mut c);
            let ta = c.text_area().unwrap();
            assert!(ta.font_size.unwrap() <= 12.0 * 2.0);
            assert!(ta.rect.h > three);
        }
    }

    const PHONE: InputTools = InputTools {
        mobile: true,
        images: false,
        keyboard_up: true,
        physical: false,
    };

    #[test]
    fn a_typed_card_has_the_row_under_its_box_and_grows_by_48_pt() {
        for kind in [PromptKind::Message, PromptKind::GeneralAgent] {
            let mut c = PromptCard::new(kind);
            c.set_input(Some(PromptInput::typed("hello")));
            let desktop = laid(&mut c);
            c.set_tools(PHONE);
            let with = laid(&mut c);
            if kind == PromptKind::Message {
                // The desktop: no row on the message card.
                assert!((with - desktop - 48.0 * 2.0).abs() < 0.01);
            } else {
                // Gemini's + shows on the desktop too, in a row of its own.
                assert!((with - desktop).abs() < 0.01);
            }
            c.set_tools(InputTools::default());
            laid(&mut c);
            assert_eq!(
                c.row()
                    .is_some_and(|r| r.attach.is_some() && r.ime.is_none()),
                kind == PromptKind::GeneralAgent,
                "{kind:?} on the desktop"
            );
            c.set_tools(PHONE);
            laid(&mut c);
            let row = *c.row().expect("the row");
            let ta = c.text_area().unwrap().rect;
            let ime = row.ime.expect("the toggle on both cards");
            assert!((ime.y - (ta.y + ta.h + 4.0 * 2.0)).abs() < 0.01);
            assert_eq!(ime.h, 44.0 * 2.0);
            assert!(c.rect().contains([ime.x + 1.0, ime.y + ime.h - 1.0]));
            assert_eq!(
                c.hit(ime.x + 5.0, ime.y + 5.0),
                Some(AgentHit::PromptIme(kind))
            );
            assert_eq!(c.rect_of(AgentHit::PromptIme(kind)), Some(ime));
            // The +: Gemini's for Paste text; the message's only with
            // the daemon's images.
            assert_eq!(
                row.attach.is_some(),
                kind == PromptKind::GeneralAgent,
                "{kind:?}"
            );
            // A dictated message has no row.
            if kind == PromptKind::Message {
                c.set_input(Some(PromptInput::said("hello")));
                laid(&mut c);
                assert!(c.row().is_none());
            }
        }
    }

    #[test]
    fn replying_puts_the_card_and_the_controls_inside() {
        let mut c = PromptCard::new(PromptKind::Message);
        c.set_reply(Some(Reply {
            title: "Claude is asking".into(),
            text: "Which one?".into(),
            accent: theme().question,
        }));
        c.set_input(Some(PromptInput::said("the second")));
        laid(&mut c);
        let t = texts(&c);
        assert!(t.contains(&"Claude is asking") && t.contains(&"the second"));
        let hits: Vec<AgentHit> = c.buttons().iter().map(|b| b.hit).collect();
        assert_eq!(hits, [AgentHit::Volume, AgentHit::Clear, AgentHit::Submit]);
        let send = c.buttons()[2].rect;
        assert!(c.rect().contains([send.x + 1.0, send.y + 1.0]));
        assert_eq!(c.hit(send.x + 5.0, send.y + 5.0), Some(AgentHit::Submit));
    }

    #[test]
    fn the_hold_shows_an_empty_general_agent_card() {
        let mut c = PromptCard::new(PromptKind::GeneralAgent);
        c.set_expanded(true);
        c.open_pinned();
        assert!(c.has_content() && !c.is_expanded());
        laid(&mut c);
        assert_eq!(texts(&c), ["\u{2726} Gemini", "Ask Gemini\u{2026}"]);
    }
}
