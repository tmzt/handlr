//! The agent view (plan 20261022_03, decision 9, as amended at 02:34 EDT:
//! "the agent view as terminal scrollback with priority cards", and "L3-1:
//! pinned to the bottom, scrolling overrides").
//!
//! One flow per agent: its transcript laid out at the phone's width in a
//! terminal style (a dark background, bright colours, the user's turns on a
//! grey band, replies plain, tool calls as dim one-liners, diff lines green
//! and red, errors red), with the priority cards at their places in it.
//! Live cards are raised panels with an accent edge, larger text and their
//! actions; resolved ones are one dim line.
//!
//! **Scrolling** is the app's own: libcontrols has no scroll view. A finger
//! drags the flow, and a fling carries on with momentum that decays. The
//! view follows the newest entry while it is at the bottom; scrolled back,
//! it stays where it is as entries arrive (kept on the entry at its top),
//! and follows again once scrolled back to the bottom.
//!
//! **The pinned card** (the highest live one) sits above the buttons. At
//! the bottom it shows in full; while the finger scrolls back, or the view
//! is scrolled back, it is a one-line bar, and tapping the bar jumps to the
//! card in the flow. The pin never moves the flow: the flow's bottom is
//! always laid out above the full card, so changing between the card and
//! the bar only uncovers or covers space below the flow.

use std::time::Duration;

use handlr_companion::{AgentCard, CardKind, Suggestion};
use libcontrols::{Button, DrawContext, DrawControl, Rect};
use libhandlr_proto::common::{Entry, EntryKind, Role};
use libmorph::TouchPhase;
use libmsdf::drawlist::{AtlasRef, DrawList, SdfInstance, SdfKind};
#[cfg(test)]
use libmsdf::font::TerminalSource;
use libmsdf::font::{GlyphStyle, TerminalShaper};

use crate::ui::{Layout, Line};

pub const BG: [f32; 4] = [0.03, 0.03, 0.04, 1.0];
pub const FG: [f32; 4] = [0.94, 0.95, 0.96, 1.0];
pub const DIM: [f32; 4] = [0.50, 0.53, 0.58, 1.0];
pub const GREEN: [f32; 4] = [0.36, 0.93, 0.45, 1.0];
pub const RED: [f32; 4] = [1.0, 0.40, 0.40, 1.0];
pub const YELLOW: [f32; 4] = [1.0, 0.82, 0.30, 1.0];
pub const BAND: [f32; 4] = [0.18, 0.19, 0.21, 1.0];
const CARD_BG: [f32; 4] = [0.11, 0.12, 0.17, 1.0];
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const HEADER_BG: [f32; 4] = [0.07, 0.07, 0.09, 1.0];
const APPROVE_BG: [f32; 4] = [0.16, 0.45, 0.25, 1.0];
const DENY_BG: [f32; 4] = [0.55, 0.18, 0.18, 1.0];
const MESSAGE_BG: [f32; 4] = [0.20, 0.32, 0.55, 1.0];
const BACK_BG: [f32; 4] = [0.30, 0.30, 0.36, 1.0];
const DICTATION_BG: [f32; 4] = [0.09, 0.10, 0.13, 1.0];

/// A card kind's accent colour.
pub fn accent(kind: CardKind) -> [f32; 4] {
    match kind {
        CardKind::Approval => [1.0, 0.74, 0.25, 1.0],
        CardKind::Question => [0.42, 0.72, 1.0, 1.0],
        CardKind::Broke => RED,
        CardKind::Finished => GREEN,
    }
}

/// Type sizes, in points.
const TEXT_PT: f32 = 15.0;
const TOOL_PT: f32 = 13.0;
const TITLE_PT: f32 = 15.0;
const CARD_PT: f32 = 19.0;
const NAME_PT: f32 = 18.0;
/// The side margin, the accent edge, a card's padding and its buttons'
/// height, in points.
const PAD: f32 = 12.0;
const EDGE: f32 = 6.0;
const CARD_PAD: f32 = 12.0;
const ACTION_H: f32 = 44.0;
const BUTTON_H: f32 = 52.0;
const HEADER_H: f32 = 64.0;
const BAR_H: f32 = 40.0;
/// The suggestion row's height and type size, and the back glyph's tap
/// width at the header's left.
const SUGGEST_H: f32 = 36.0;
const SUGGEST_PT: f32 = 18.0;
const BACK_W: f32 = 44.0;
/// The take-a-message button's width at the bottom right, and its record
/// dot. A centred dot stands in for a microphone glyph, which no bundled
/// atlas has yet (a microphone icon is for later).
const MIC_W: f32 = 96.0;
const DOT: f32 = 18.0;
const DOT_RED: [f32; 4] = [0.95, 0.25, 0.25, 1.0];
/// The back glyph, from the terminal face (Roboto here is ASCII only).
const BACK_GLYPH: &str = "\u{2039}";
/// Lines of a tool result shown: one, or a diff's or an error's first few.
const RESULT_LINES: usize = 12;
const ERROR_LINES: usize = 4;
/// Lines of the pinned card's text.
const PIN_LINES: usize = 3;
/// Lines of the dictation card shown at most: its tail.
const DICTATION_LINES: usize = 6;

/// A line height for type of `px` pixels.
fn line_h(px: f32) -> f32 {
    px * 1.35
}

/// The terminal face's cell and line pitch, in em: Noto Sans Mono's 0.6,
/// and the 1.3 em at which the symbol page's box drawing joins (libmsdf's
/// `TerminalShaper::cell_em` and `line_pitch_em`).
const CELL_EM: f32 = 0.6;
const PITCH_EM: f32 = 1.3;

/// A scrollback line's height for type of `px` pixels.
fn term_h(px: f32) -> f32 {
    px * PITCH_EM
}

/// How many cells of `px`-pixel terminal type fit in `width`.
fn term_columns(width: f32, px: f32) -> usize {
    (width / (px * CELL_EM)).max(8.0) as usize
}

/// How many characters of `px`-pixel type fit in `width`: roughly half an
/// em each in Roboto, as the main screen counts.
fn columns(width: f32, px: f32) -> usize {
    (width / (px * 0.5)).max(8.0) as usize
}

/// Splits `text` into lines of at most `max` characters: at its newlines,
/// then at spaces, breaking words longer than a line (paths, hashes). An
/// empty line stays empty.
pub fn wrap_hard(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.trim_end();
        if raw.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in raw.split(' ') {
            let mut word: Vec<char> = word.chars().collect();
            loop {
                let used = line.chars().count();
                let sep = usize::from(used > 0);
                if used + sep + word.len() <= max {
                    if sep == 1 {
                        line.push(' ');
                    }
                    line.extend(word.iter());
                    break;
                }
                if word.len() <= max {
                    // It fits on a line of its own.
                    out.push(std::mem::take(&mut line));
                    continue;
                }
                // Longer than a line: fill the rest of this one.
                let room = max.saturating_sub(used + sep);
                if room == 0 {
                    out.push(std::mem::take(&mut line));
                    continue;
                }
                if sep == 1 {
                    line.push(' ');
                }
                line.extend(word[..room].iter());
                word.drain(..room);
                out.push(std::mem::take(&mut line));
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

/// One line of text in the flow, in content coordinates (y from the top
/// of the flow).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub text: String,
    pub x: f32,
    pub y: f32,
    /// Type size in pixels.
    pub px: f32,
    pub color: [f32; 4],
    /// Scrollback, on the terminal grid; otherwise a card's or the
    /// screen's text, in Roboto.
    pub mono: bool,
}

/// A filled rectangle: a band, a card's panel, its shadow or its edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub rect: Rect,
    pub color: [f32; 4],
    pub radius: f32,
}

/// A control a touch landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentHit {
    Back,
    /// Take a message to this agent.
    Message,
    Approve(u64),
    Deny(u64),
    /// A suggested response, in the row above the buttons or a card.
    Suggest(Suggestion),
    /// The pinned bar: jump to its card.
    Pin,
    /// The dictation buttons, while a message is taken from the view.
    Submit,
    Scratch,
    Clear,
}

/// A button, in the flow or on the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowButton {
    pub hit: AgentHit,
    pub rect: Rect,
    pub label: &'static str,
    pub bg: [f32; 4],
}

/// The flow laid out at one width, in content coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flow {
    pub rows: Vec<Row>,
    pub fills: Vec<Fill>,
    pub buttons: Vec<FlowButton>,
    /// The top of each entry.
    pub entry_y: Vec<f32>,
    /// The top of each card.
    pub card_y: Vec<f32>,
    pub height: f32,
}

/// How an entry is drawn: its lines, their size and colours, and whether
/// it sits on the grey band.
fn entry_lines(e: &Entry, cols: impl Fn(f32) -> usize) -> (Vec<(String, f32, [f32; 4])>, bool) {
    let mut lines = Vec::new();
    match (e.role, e.kind) {
        (Role::User, EntryKind::Text) => {
            for l in wrap_hard(e.summary.trim(), cols(TEXT_PT)) {
                lines.push((l, TEXT_PT, FG));
            }
            return (lines, true);
        }
        (_, EntryKind::Text) => {
            // Replies are plain; inside a fenced block, diff lines are green
            // and red and the fences dim.
            let mut fenced = false;
            for raw in e.summary.trim().split('\n') {
                let fence = raw.trim_start().starts_with("```");
                let color = if fence {
                    DIM
                } else if fenced {
                    diff_colour(raw).unwrap_or(FG)
                } else {
                    FG
                };
                if fence {
                    fenced = !fenced;
                }
                for l in wrap_hard(raw, cols(TEXT_PT)) {
                    lines.push((l, TEXT_PT, color));
                }
            }
        }
        (_, EntryKind::ToolUse) => {
            let first = e.summary.lines().next().unwrap_or("");
            lines.push((one_line(&format!("> {first}"), cols(TOOL_PT)), TOOL_PT, DIM));
        }
        (_, EntryKind::ToolResult) => {
            if let Some(error) = e.summary.strip_prefix("error: ") {
                let wrapped = wrap_hard(&format!("! {}", error.trim()), cols(TOOL_PT));
                for l in wrapped.into_iter().take(ERROR_LINES) {
                    lines.push((l, TOOL_PT, RED));
                }
            } else if is_diff(&e.summary) {
                for raw in e.summary.lines().take(RESULT_LINES) {
                    let color = diff_colour(raw).unwrap_or(DIM);
                    lines.push((one_line(&format!("  {raw}"), cols(TOOL_PT)), TOOL_PT, color));
                }
            } else {
                let first = e
                    .summary
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("");
                lines.push((one_line(&format!("  {first}"), cols(TOOL_PT)), TOOL_PT, DIM));
            }
        }
    }
    (lines, false)
}

/// A diff line's colour: `+` green and `-` red, but not `+++` or `---`.
pub fn diff_colour(line: &str) -> Option<[f32; 4]> {
    if line.starts_with("+++") || line.starts_with("---") {
        None
    } else if line.starts_with('+') {
        Some(GREEN)
    } else if line.starts_with('-') {
        Some(RED)
    } else {
        None
    }
}

/// Whether a tool result reads as a diff: a hunk header, or `+` and `-`
/// lines both.
pub fn is_diff(text: &str) -> bool {
    let has = |c: char| {
        text.lines()
            .any(|l| l.starts_with(c) && !l.starts_with("---"))
    };
    text.lines().any(|l| l.starts_with("@@")) || (has('+') && has('-'))
}

/// `text` on one line of `cols` characters, cut with "…".
fn one_line(text: &str, cols: usize) -> String {
    if text.chars().count() <= cols {
        return text.to_string();
    }
    let kept: String = text.chars().take(cols.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

/// A card's actions.
fn actions(card: &AgentCard) -> Vec<(AgentHit, &'static str, [f32; 4])> {
    match (card.kind, card.confirm_id) {
        (CardKind::Approval, Some(id)) => vec![
            (AgentHit::Approve(id), "Approve", APPROVE_BG),
            (AgentHit::Deny(id), "Deny", DENY_BG),
        ],
        (CardKind::Question, _) => vec![
            (AgentHit::Suggest(Suggestion::Yes), "Yes", APPROVE_BG),
            (AgentHit::Suggest(Suggestion::No), "No", DENY_BG),
            (AgentHit::Message, "", MESSAGE_BG),
        ],
        _ => vec![],
    }
}

/// A live card's block at `y`, `w` wide from `x`: its fills, rows and
/// buttons, and its height. At most `max_lines` lines of its text.
fn card_block(
    card: &AgentCard,
    x: f32,
    y: f32,
    w: f32,
    s: f32,
    max_lines: usize,
    flow: &mut Flow,
) -> f32 {
    let pad = CARD_PAD * s;
    let inner_x = x + EDGE * s + pad;
    let inner_w = w - EDGE * s - 2.0 * pad;
    let (title_px, text_px) = (TITLE_PT * s, CARD_PT * s);
    let mut lines = wrap_hard(card.text.trim(), columns(inner_w, text_px));
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            last.push('\u{2026}');
        }
    }
    let acts = actions(card);
    let mut h = pad + line_h(title_px) + 4.0 * s + lines.len() as f32 * line_h(text_px) + pad;
    if !acts.is_empty() {
        h += ACTION_H * s + pad * 0.5;
    }
    let panel = Rect::new(x, y, w, h);
    flow.fills.push(Fill {
        rect: Rect::new(x + 3.0 * s, y + 4.0 * s, w, h),
        color: SHADOW,
        radius: 10.0 * s,
    });
    flow.fills.push(Fill {
        rect: panel,
        color: CARD_BG,
        radius: 10.0 * s,
    });
    flow.fills.push(Fill {
        rect: Rect::new(x, y, EDGE * s, h),
        color: accent(card.kind),
        radius: 3.0 * s,
    });
    let mut ty = y + pad;
    flow.rows.push(Row {
        text: card.title.clone(),
        x: inner_x,
        y: ty,
        px: title_px,
        color: accent(card.kind),
        mono: false,
    });
    ty += line_h(title_px) + 4.0 * s;
    for l in lines {
        flow.rows.push(Row {
            text: l,
            x: inner_x,
            y: ty,
            px: text_px,
            color: FG,
            mono: false,
        });
        ty += line_h(text_px);
    }
    if !acts.is_empty() {
        let gap = 12.0 * s;
        let n = acts.len().max(2) as f32;
        let bw = (inner_w - gap * (n - 1.0)) / n;
        for (i, (hit, label, bg)) in acts.into_iter().enumerate() {
            flow.buttons.push(FlowButton {
                hit,
                rect: Rect::new(
                    inner_x + i as f32 * (bw + gap),
                    ty + pad * 0.5,
                    bw,
                    ACTION_H * s,
                ),
                label,
                bg,
            });
        }
    }
    h
}

/// Lays out `entries` and `cards` (in flow order, as the client gives
/// them) at `width` pixels and scale `s`.
pub fn lay_out_flow(entries: &[Entry], cards: &[AgentCard], width: f32, s: f32) -> Flow {
    let mut flow = Flow::default();
    let pad = PAD * s;
    let text_w = width - 2.0 * pad;
    let cols = |pt: f32| term_columns(text_w - 8.0 * s, pt * s);
    let mut y = 8.0 * s;
    let mut next_card = 0;
    for i in 0..=entries.len() {
        while let Some(card) = cards.get(next_card).filter(|c| c.at <= i) {
            flow.card_y.push(y);
            if card.live {
                y += 8.0 * s;
                *flow.card_y.last_mut().expect("pushed") = y;
                y += card_block(card, pad, y, text_w, s, usize::MAX, &mut flow) + 12.0 * s;
            } else {
                // Resolved: ordinary scrollback, one dim line.
                let text = format!("-- {}: {}", card.title, card.text.replace('\n', " "));
                flow.rows.push(Row {
                    text: one_line(&text, cols(TOOL_PT)),
                    x: pad,
                    y,
                    px: TOOL_PT * s,
                    color: DIM,
                    mono: true,
                });
                y += term_h(TOOL_PT * s) + 4.0 * s;
            }
            next_card += 1;
        }
        let Some(e) = entries.get(i) else { break };
        let (lines, band) = entry_lines(e, cols);
        let gap = match (e.role, e.kind) {
            (_, EntryKind::Text) => 10.0 * s,
            _ => 2.0 * s,
        };
        y += gap;
        flow.entry_y.push(y);
        let h: f32 = lines
            .iter()
            .map(|(text, pt, _)| {
                let lh = term_h(pt * s);
                if text.is_empty() { lh * 0.5 } else { lh }
            })
            .sum();
        if band {
            flow.fills.push(Fill {
                rect: Rect::new(0.0, y - 4.0 * s, width, h + 8.0 * s),
                color: BAND,
                radius: 0.0,
            });
        }
        let mut ly = y;
        for (text, pt, color) in lines {
            let px = pt * s;
            let lh = term_h(px);
            if text.is_empty() {
                ly += lh * 0.5;
                continue;
            }
            flow.rows.push(Row {
                text,
                x: pad,
                y: ly,
                px,
                color,
                mono: true,
            });
            ly += lh;
        }
        y = ly + if band { 8.0 * s } else { 0.0 };
    }
    flow.height = y + 16.0 * s;
    flow
}

/// A finger on the flow.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    id: u64,
    start: [f32; 2],
    last_y: f32,
    last_t: Duration,
    /// Past the slop: a scroll, not a tap.
    moved: bool,
    /// Pixels per second, smoothed.
    velocity: f32,
}

/// A movement under this many points is a tap.
const SLOP: f32 = 10.0;
/// The momentum's decay per second: the speed falls to `e^-DECAY` of
/// itself each second.
const DECAY: f32 = 3.2;
/// Slower than this (points per second), a fling stops.
const MIN_FLING: f32 = 30.0;

/// Where things are on the screen, for the current layout and scroll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    pub header: Rect,
    /// The flow's window.
    pub viewport: Rect,
    /// The flow's window with the pinned card in full: what the bottom is.
    pub full_viewport_h: f32,
    /// The pinned card, in full or as the bar, and its parts.
    pub pin: Option<Pinned>,
    /// The dictation card, while a message is taken from the view.
    pub dictation: Option<Flow>,
    pub buttons: Vec<FlowButton>,
    /// The back glyph's tap area, at the header's left.
    pub back: Rect,
    /// The suggested responses, as plain coloured text above the buttons.
    pub suggestions: Vec<Suggested>,
}

/// A suggested response as placed: its tap area and its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggested {
    pub hit: AgentHit,
    pub rect: Rect,
    pub label: &'static str,
    pub color: [f32; 4],
}

/// A suggestion's colour: yes green, no red, the commands yellow.
pub fn suggestion_colour(s: Suggestion) -> [f32; 4] {
    match s {
        Suggestion::Yes => GREEN,
        Suggestion::No => RED,
        Suggestion::Continue | Suggestion::Compact => YELLOW,
    }
}

/// The pinned card as placed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pinned {
    pub compact: bool,
    pub rect: Rect,
    /// Laid out at the screen's coordinates.
    pub flow: Flow,
}

/// The terminal face for the scrollback: the shaper and its two atlases,
/// in the renderer's atlas set.
pub struct Terminal<'a> {
    pub shaper: &'a TerminalShaper,
    pub text: AtlasRef<'a>,
    pub symbols: AtlasRef<'a>,
    pub px_range: f32,
}

/// Draws `row` at `pos`: on the terminal grid when it is scrollback and
/// the face is there, otherwise in the context's font.
fn draw_row(
    list: &mut DrawList,
    ctx: &DrawContext,
    term: Option<&Terminal>,
    row: &Row,
    pos: [f32; 2],
) {
    match term {
        Some(t) if row.mono => {
            let line = t.shaper.line(&row.text, GlyphStyle::Regular);
            list.push_terminal_line(&line, t.text, t.symbols, pos, row.px, t.px_range, row.color);
        }
        _ => {
            ctx.draw_text(list, &row.text, pos, row.px, row.color);
        }
    }
}

/// Whether `a` and `b` are the same entry of the session: its time, role and
/// kind. Its summary may change between peeks, as a tool call's result is
/// linked to it.
fn same_entry(a: &Entry, b: &Entry) -> bool {
    a.ts == b.ts && a.role == b.role && a.kind == b.kind
}

/// The agent view's screen: content, scroll state and layout.
#[derive(Default)]
pub struct AgentScreen {
    pub name: String,
    pub status: String,
    entries: Vec<Entry>,
    cards: Vec<AgentCard>,
    pinned: Option<usize>,
    layout: Layout,
    flow: Flow,
    frame: Frame,
    /// From the top of the flow, in pixels.
    offset: f32,
    /// Following the newest entry at the bottom.
    follow: bool,
    drag: Option<Drag>,
    /// A fling's speed, in pixels per second (positive: the finger moved
    /// down, scrolling back).
    fling: f32,
    hits: Vec<(AgentHit, Rect)>,
    /// The dictation card's lines (as the main screen gives them), while a
    /// message is taken.
    dictation: Option<Vec<Line>>,
    dirty: bool,
}

impl AgentScreen {
    pub fn new() -> Self {
        AgentScreen {
            follow: true,
            dirty: true,
            ..Default::default()
        }
    }

    /// New content. Scrolled back, the view stays on the entry at its top.
    pub fn set_content(
        &mut self,
        name: String,
        status: String,
        entries: Vec<Entry>,
        cards: Vec<AgentCard>,
        pinned: Option<usize>,
    ) {
        if self.name == name
            && self.status == status
            && self.entries == entries
            && self.cards == cards
            && self.pinned == pinned
        {
            return;
        }
        // Scrolled back, the view holds still: it stays on the entry at its
        // top, or, when that one has left the window, on the next one that
        // is still there, at the same place on screen.
        let keep = if self.follow {
            Vec::new()
        } else {
            self.anchors()
        };
        self.name = name;
        self.status = status;
        self.entries = entries;
        self.cards = cards;
        self.pinned = pinned;
        self.relayout();
        if let Some((i, delta)) = keep.iter().find_map(|(entry, delta)| {
            self.entries
                .iter()
                .rposition(|e| same_entry(e, entry))
                .map(|i| (i, *delta))
        }) {
            self.offset = self.flow.entry_y[i] + delta;
        }
        self.settle();
    }

    /// The entries from the one at the top of the window on, each with how
    /// far the window's top is from it.
    fn anchors(&self) -> Vec<(Entry, f32)> {
        // Above the first entry (the flow's top padding), the first.
        let top = self
            .flow
            .entry_y
            .iter()
            .rposition(|&y| y <= self.offset)
            .unwrap_or(0);
        self.entries
            .iter()
            .zip(&self.flow.entry_y)
            .skip(top)
            .map(|(e, &y)| (e.clone(), self.offset - y))
            .collect()
    }

    /// The entry at the top of the window, and how far into it the window
    /// starts.
    #[cfg(test)]
    fn top_entry(&self) -> Option<(Entry, f32)> {
        let i = self.flow.entry_y.iter().rposition(|&y| y <= self.offset)?;
        Some((
            self.entries.get(i)?.clone(),
            self.offset - self.flow.entry_y[i],
        ))
    }

    /// The dictation card, or `None` once the message is sent or cleared.
    /// The buttons are Submit, Scratch and Clear while it shows.
    pub fn set_dictation(&mut self, lines: Option<Vec<Line>>) {
        if lines == self.dictation {
            return;
        }
        self.dictation = lines;
        self.settle();
    }

    pub fn set_layout(&mut self, layout: Layout) {
        if layout == self.layout {
            return;
        }
        self.layout = layout;
        self.relayout();
        self.settle();
    }

    fn relayout(&mut self) {
        self.flow = lay_out_flow(&self.entries, &self.cards, self.layout.w, self.scale());
    }

    fn scale(&self) -> f32 {
        if self.layout.scale > 0.0 {
            self.layout.scale
        } else {
            1.0
        }
    }

    /// The pinned card, if any.
    pub fn pinned_card(&self) -> Option<&AgentCard> {
        self.pinned.and_then(|i| self.cards.get(i))
    }

    /// The largest offset: the flow's bottom above the full pinned card.
    pub fn max_offset(&self) -> f32 {
        (self.flow.height - self.frame.full_viewport_h).max(0.0)
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn following(&self) -> bool {
        self.follow
    }

    /// Whether the pinned card is the bar: while a finger scrolls, or the
    /// view is scrolled back.
    pub fn compact(&self) -> bool {
        !self.follow || self.drag.is_some_and(|d| d.moved)
    }

    /// Clamps the offset, follows the bottom when following, and places
    /// everything.
    fn settle(&mut self) {
        self.frame = self.arrange(false);
        let full_h = self.frame.full_viewport_h;
        let max = (self.flow.height - full_h).max(0.0);
        if self.follow {
            self.offset = max;
        }
        self.offset = self.offset.clamp(0.0, max);
        self.frame = self.arrange(self.compact());
        self.place_hits();
        self.dirty = true;
    }

    /// The screen's parts, with the pinned card in full or as the bar.
    fn arrange(&self, compact: bool) -> Frame {
        let l = &self.layout;
        let s = self.scale();
        let pad = PAD * s;
        let header = Rect::new(0.0, 0.0, l.w, l.top + HEADER_H * s);
        let buttons_top = l.h - l.bottom - 16.0 * s - BUTTON_H * s;
        let gap = 12.0 * s;
        // The take-a-message button, always at the bottom right; while
        // dictating, Submit, Scratch and Clear share the rest of the row.
        let mic_w = MIC_W * s;
        let mut buttons = vec![FlowButton {
            hit: AgentHit::Message,
            rect: Rect::new(l.w - pad - mic_w, buttons_top, mic_w, BUTTON_H * s),
            label: "",
            bg: MESSAGE_BG,
        }];
        if self.dictation.is_some() {
            let row = [
                (AgentHit::Submit, "Submit", APPROVE_BG),
                (AgentHit::Scratch, "Scratch", BACK_BG),
                (AgentHit::Clear, "Clear", DENY_BG),
            ];
            let n = row.len() as f32;
            let w = (l.w - 2.0 * pad - mic_w - gap * n) / n;
            for (i, (hit, label, bg)) in row.into_iter().enumerate() {
                buttons.push(FlowButton {
                    hit,
                    rect: Rect::new(pad + i as f32 * (w + gap), buttons_top, w, BUTTON_H * s),
                    label,
                    bg,
                });
            }
        }
        let back = Rect::new(0.0, l.top, BACK_W * s, HEADER_H * s);
        let mut above = buttons_top - 12.0 * s;
        // The suggestions, just above the buttons.
        let offered = self.suggestions();
        let slot = (l.w - 2.0 * pad) / 4.0;
        let row_top = buttons_top - 8.0 * s - SUGGEST_H * s;
        let suggestions: Vec<Suggested> = offered
            .into_iter()
            .enumerate()
            .map(|(i, sg)| Suggested {
                hit: AgentHit::Suggest(sg),
                rect: Rect::new(pad + i as f32 * slot, row_top, slot, SUGGEST_H * s),
                label: sg.label(),
                color: suggestion_colour(sg),
            })
            .collect();
        if !suggestions.is_empty() {
            above = row_top - 8.0 * s;
        }
        // The dictation card, above the buttons.
        let dictation = self.dictation.as_ref().map(|lines| {
            let inner = l.w - 4.0 * pad;
            let mut rows: Vec<(String, f32, [f32; 4], f32)> = Vec::new();
            for line in lines {
                let px = line.size * s;
                for (i, t) in wrap_hard(&line.text, columns(inner, px))
                    .into_iter()
                    .enumerate()
                {
                    let gap = if i == 0 { line.gap.min(8.0) * s } else { 0.0 };
                    rows.push((t, px, line.color, gap));
                }
            }
            let skip = rows.len().saturating_sub(DICTATION_LINES);
            let rows = &rows[skip..];
            let h: f32 = rows
                .iter()
                .map(|(_, px, _, gap)| line_h(*px) + gap)
                .sum::<f32>()
                + 2.0 * pad;
            let top = above - h;
            let mut flow = Flow::default();
            flow.fills.push(Fill {
                rect: Rect::new(pad, top, l.w - 2.0 * pad, h),
                color: DICTATION_BG,
                radius: 10.0 * s,
            });
            let mut y = top + pad;
            for (text, px, color, gap) in rows {
                y += gap;
                flow.rows.push(Row {
                    text: text.clone(),
                    x: 2.0 * pad,
                    y,
                    px: *px,
                    color: *color,
                    mono: false,
                });
                y += line_h(*px);
            }
            flow.height = h;
            above = top - 8.0 * s;
            flow
        });
        let mut full_top = above;
        let mut pin = None;
        if let Some(card) = self.pinned_card() {
            // The full card, laid out once to know its height.
            let mut full = Flow::default();
            let h = card_block(card, pad, 0.0, l.w - 2.0 * pad, s, PIN_LINES, &mut full);
            full_top = above - h;
            if compact {
                let bar = Rect::new(pad, above - BAR_H * s, l.w - 2.0 * pad, BAR_H * s);
                let mut flow = Flow::default();
                flow.fills.push(Fill {
                    rect: bar,
                    color: CARD_BG,
                    radius: 8.0 * s,
                });
                flow.fills.push(Fill {
                    rect: Rect::new(bar.x, bar.y, EDGE * s, bar.h),
                    color: accent(card.kind),
                    radius: 3.0 * s,
                });
                let px = TITLE_PT * s;
                let text = format!("{}: {}", card.title, card.text.replace('\n', " "));
                flow.rows.push(Row {
                    text: one_line(&text, columns(bar.w - 2.0 * pad - EDGE * s, px)),
                    x: bar.x + EDGE * s + pad,
                    y: bar.y + (bar.h - line_h(px)) * 0.5,
                    px,
                    color: FG,
                    mono: false,
                });
                pin = Some(Pinned {
                    compact: true,
                    rect: bar,
                    flow,
                });
            } else {
                let mut flow = Flow::default();
                card_block(
                    card,
                    pad,
                    full_top,
                    l.w - 2.0 * pad,
                    s,
                    PIN_LINES,
                    &mut flow,
                );
                pin = Some(Pinned {
                    compact: false,
                    rect: Rect::new(pad, full_top, l.w - 2.0 * pad, h),
                    flow,
                });
            }
        }
        let view_top = header.y + header.h;
        let view_bottom = match &pin {
            Some(p) => p.rect.y - 8.0 * s,
            None => above,
        };
        let full_bottom = if pin.is_some() {
            full_top - 8.0 * s
        } else {
            above
        };
        Frame {
            header,
            viewport: Rect::new(0.0, view_top, l.w, (view_bottom - view_top).max(0.0)),
            full_viewport_h: (full_bottom - view_top).max(0.0),
            pin,
            dictation,
            buttons,
            back,
            suggestions,
        }
    }

    /// The suggested responses for the row, by the pinned card and the
    /// status. Those that belong to a card are in it: a question's yes and
    /// no, an approval's own buttons. None while working or dictating.
    pub fn suggestions(&self) -> Vec<Suggestion> {
        if self.dictation.is_some() || self.status == "working" {
            return Vec::new();
        }
        match self.pinned_card().filter(|c| c.live).map(|c| c.kind) {
            Some(CardKind::Broke | CardKind::Finished) => vec![Suggestion::Continue],
            _ => Vec::new(),
        }
    }

    /// The controls where they are now: the screen's buttons, the pin's,
    /// and the flow's that are in the window.
    fn place_hits(&mut self) {
        self.hits.clear();
        for b in &self.frame.buttons {
            self.hits.push((b.hit, b.rect));
        }
        self.hits.push((AgentHit::Back, self.frame.back));
        for g in &self.frame.suggestions {
            self.hits.push((g.hit, g.rect));
        }
        if let Some(p) = &self.frame.pin {
            if p.compact {
                self.hits.push((AgentHit::Pin, p.rect));
            } else {
                for b in &p.flow.buttons {
                    self.hits.push((b.hit, b.rect));
                }
            }
        }
        let v = self.frame.viewport;
        for b in &self.flow.buttons {
            let rect = Rect::new(b.rect.x, v.y + b.rect.y - self.offset, b.rect.w, b.rect.h);
            if rect.y >= v.y && rect.y + rect.h <= v.y + v.h {
                self.hits.push((b.hit, rect));
            }
        }
    }

    /// The control at `(x, y)`, if any.
    pub fn hit(&self, x: f32, y: f32) -> Option<AgentHit> {
        self.hits
            .iter()
            .find(|(_, rect)| rect.contains([x, y]))
            .map(|&(hit, _)| hit)
    }

    /// Where `hit` is now, if it is on the screen.
    pub fn rect_of(&self, hit: AgentHit) -> Option<Rect> {
        self.hits.iter().find(|(h, _)| *h == hit).map(|&(_, r)| r)
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    pub fn flow(&self) -> &Flow {
        &self.flow
    }

    /// Scrolls to `offset`, following again if that is the bottom.
    fn scroll_to(&mut self, offset: f32) {
        let max = self.max_offset();
        self.offset = offset.clamp(0.0, max);
        self.follow = self.offset >= max - 1.0;
        self.frame = self.arrange(self.compact());
        self.place_hits();
        self.dirty = true;
    }

    /// Jumps to the pinned card in the flow: its top a third of the way
    /// down the window.
    pub fn jump_to_pin(&mut self) {
        let Some(y) = self.pinned.and_then(|i| self.flow.card_y.get(i).copied()) else {
            return;
        };
        self.fling = 0.0;
        let target = y - self.frame.viewport.h / 3.0;
        self.scroll_to(target);
    }

    /// A touch at `now` (any monotonic clock). Returns the control tapped,
    /// if the touch was a tap; a drag scrolls and returns nothing.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
        now: Duration,
    ) -> Option<AgentHit> {
        let s = self.scale();
        match phase {
            TouchPhase::Started => {
                self.fling = 0.0;
                self.drag = Some(Drag {
                    id,
                    start: [x, y],
                    last_y: y,
                    last_t: now,
                    moved: false,
                    velocity: 0.0,
                });
                None
            }
            TouchPhase::Moved => {
                let mut d = self.drag.filter(|d| d.id == id)?;
                if !d.moved
                    && ((y - d.start[1]).abs() > SLOP * s || (x - d.start[0]).abs() > SLOP * s)
                {
                    d.moved = true;
                }
                let dy = y - d.last_y;
                let dt = now.saturating_sub(d.last_t).as_secs_f32();
                if dt > 0.0 {
                    let v = dy / dt;
                    d.velocity = if d.velocity == 0.0 {
                        v
                    } else {
                        0.6 * v + 0.4 * d.velocity
                    };
                }
                d.last_y = y;
                d.last_t = now;
                self.drag = Some(d);
                if d.moved {
                    self.scroll_to(self.offset - dy);
                }
                None
            }
            TouchPhase::Ended => {
                let d = self.drag.take().filter(|d| d.id == id)?;
                if d.moved {
                    // A finger that stopped before lifting does not fling.
                    let still = now.saturating_sub(d.last_t) > Duration::from_millis(100);
                    if !still && d.velocity.abs() > MIN_FLING * s {
                        self.fling = d.velocity;
                    }
                    self.scroll_to(self.offset);
                    return None;
                }
                self.scroll_to(self.offset);
                self.hit(x, y)
            }
            TouchPhase::Cancelled => {
                if self.drag.take().is_some() {
                    self.scroll_to(self.offset);
                }
                None
            }
        }
    }

    /// Carries a fling on by `dt`. Returns whether the view moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        if self.fling == 0.0 || self.drag.is_some() {
            return false;
        }
        let dt = dt.as_secs_f32().min(0.1);
        let before = self.offset;
        self.scroll_to(self.offset - self.fling * dt);
        self.fling *= (-DECAY * dt).exp();
        let stopped = self.offset == before
            || self.fling.abs() < MIN_FLING * self.scale()
            || self.offset <= 0.0
            || self.follow;
        if stopped {
            self.fling = 0.0;
        }
        self.offset != before
    }

    /// Rebuilds `list` if anything changed. Returns whether it did. The
    /// scrollback is drawn with `term` when given, otherwise in `ctx`'s font.
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

        // The flow, in its window.
        let v = self.frame.viewport;
        list.push_clip(v.pos(), v.size());
        let dy = v.y - self.offset;
        let visible = |y: f32, h: f32| y + h + dy >= v.y && y + dy <= v.y + v.h;
        for f in &self.flow.fills {
            if visible(f.rect.y, f.rect.h) {
                let r = Rect::new(f.rect.x, f.rect.y + dy, f.rect.w, f.rect.h);
                fill(list, r, f.color, f.radius);
            }
        }
        for r in &self.flow.rows {
            let h = if r.mono { term_h(r.px) } else { line_h(r.px) };
            if visible(r.y, h) {
                draw_row(list, ctx, term, r, [r.x, r.y + dy]);
            }
        }
        for b in &self.flow.buttons {
            if visible(b.rect.y, b.rect.h) {
                let rect = Rect::new(b.rect.x, b.rect.y + dy, b.rect.w, b.rect.h);
                button(list, ctx, b.label, rect, b.bg, s);
            }
        }
        if self.entries.is_empty() && self.cards.is_empty() {
            let px = TEXT_PT * s;
            ctx.draw_text(
                list,
                "(no session to show)",
                [PAD * s, v.y + 16.0 * s],
                px,
                DIM,
            );
        }
        list.push_clip_end();

        // The header: the agent and its status.
        fill(list, self.frame.header, HEADER_BG, 0.0);
        let name_px = NAME_PT * s;
        let top = l.top + 8.0 * s;
        let back = Row {
            text: if term.is_some() { BACK_GLYPH } else { "<" }.to_string(),
            x: 0.0,
            y: 0.0,
            px: 1.6 * name_px,
            color: FG,
            mono: true,
        };
        let back_y = l.top + (HEADER_H * s - term_h(back.px)) * 0.5;
        draw_row(list, ctx, term, &back, [PAD * s, back_y]);
        let name_x = BACK_W * s;
        ctx.draw_text(list, &self.name, [name_x, top], name_px, FG);
        ctx.draw_text(
            list,
            &self.status,
            [name_x, top + line_h(name_px)],
            TOOL_PT * s,
            DIM,
        );

        // The pinned card, the dictation card, then the buttons.
        if let Some(d) = &self.frame.dictation {
            for f in &d.fills {
                fill(list, f.rect, f.color, f.radius);
            }
            for r in &d.rows {
                ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
            }
        }
        if let Some(p) = &self.frame.pin {
            for f in &p.flow.fills {
                fill(list, f.rect, f.color, f.radius);
            }
            for r in &p.flow.rows {
                ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
            }
            for b in &p.flow.buttons {
                button(list, ctx, b.label, b.rect, b.bg, s);
            }
        }
        let sg_px = SUGGEST_PT * s;
        for g in &self.frame.suggestions {
            let y = g.rect.y + (g.rect.h - line_h(sg_px)) * 0.5;
            ctx.draw_text(list, g.label, [g.rect.x + 4.0 * s, y], sg_px, g.color);
        }
        for b in &self.frame.buttons {
            button(list, ctx, b.label, b.rect, b.bg, s);
        }
        self.dirty = false;
        true
    }

    /// Marks the screen to be drawn again.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
}

fn fill(list: &mut DrawList, rect: Rect, color: [f32; 4], radius: f32) {
    let kind = if radius > 0.0 {
        SdfKind::RoundedBox { radius }
    } else {
        SdfKind::Box
    };
    list.push(SdfInstance {
        kind,
        position: rect.pos(),
        size: rect.size(),
        color,
        anim: 0,
    });
}

fn button(list: &mut DrawList, ctx: &DrawContext, label: &str, rect: Rect, bg: [f32; 4], s: f32) {
    if label.is_empty() {
        // The take-a-message button: its shape, and the record dot centred.
        fill(list, rect, bg, 10.0 * s);
        let d = DOT * s;
        let dot = Rect::new(
            rect.x + (rect.w - d) * 0.5,
            rect.y + (rect.h - d) * 0.5,
            d,
            d,
        );
        fill(list, dot, DOT_RED, d * 0.5);
        return;
    }
    let mut b = Button::new(label, rect)
        .with_bg(bg)
        .with_text_color(FG)
        .with_font_size(17.0 * s);
    b.corner_radius = Some(10.0 * s);
    b.draw(list, ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: Layout = Layout {
        w: 1080.0,
        h: 2340.0,
        scale: 2.0,
        top: 80.0,
        bottom: 40.0,
    };

    fn entry(role: Role, kind: EntryKind, summary: &str) -> Entry {
        Entry {
            ts: summary.chars().take(12).collect(),
            role,
            kind,
            tool: None,
            summary: summary.to_string(),
        }
    }

    fn conversation(n: usize) -> Vec<Entry> {
        (0..n)
            .flat_map(|i| {
                [
                    entry(Role::User, EntryKind::Text, &format!("question {i}")),
                    entry(Role::Assistant, EntryKind::ToolUse, &format!("Bash {{\"n\":{i}}}")),
                    entry(Role::Tool, EntryKind::ToolResult, &format!("ok {i}")),
                    entry(
                        Role::Assistant,
                        EntryKind::Text,
                        &format!("Answer {i}.\n\nA second paragraph, long enough to wrap at the phone's width at least once or twice."),
                    ),
                ]
            })
            .collect()
    }

    fn card(kind: CardKind, at: usize, live: bool) -> AgentCard {
        AgentCard {
            kind,
            live,
            at,
            title: "Claude is asking".to_string(),
            text: "Shall I go on?".to_string(),
            confirm_id: (kind == CardKind::Approval).then_some(7),
        }
    }

    fn screen(entries: Vec<Entry>, cards: Vec<AgentCard>, pinned: Option<usize>) -> AgentScreen {
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        a.set_content("Claude".into(), "idle".into(), entries, cards, pinned);
        a
    }

    const MS: Duration = Duration::from_millis(16);

    /// Drags from `from` to `to` in `steps`, 16 ms apart, starting at `t`.
    fn drag(a: &mut AgentScreen, from: f32, to: f32, steps: u32, t: Duration) -> Duration {
        a.touch(1, TouchPhase::Started, 540.0, from, t);
        let mut now = t;
        for i in 1..=steps {
            now += MS;
            let y = from + (to - from) * i as f32 / steps as f32;
            a.touch(1, TouchPhase::Moved, 540.0, y, now);
        }
        now
    }

    #[test]
    fn hard_wrapping_keeps_lines_and_breaks_long_words() {
        assert_eq!(wrap_hard("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap_hard("a\n\nb", 7), ["a", "", "b"]);
        assert_eq!(wrap_hard("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap_hard("x abcdefghij", 4), ["x ab", "cdef", "ghij"]);
        for l in wrap_hard(&"word ".repeat(50), 13) {
            assert!(l.chars().count() <= 13, "{l}");
        }
    }

    #[test]
    fn entries_are_drawn_terminal_style() {
        let entries = vec![
            entry(Role::User, EntryKind::Text, "please fix it"),
            entry(
                Role::Assistant,
                EntryKind::ToolUse,
                "Edit {\"file_path\":\"src/a.rs\"}\nmore",
            ),
            entry(Role::Tool, EntryKind::ToolResult, "@@ -1 +1 @@\n-old\n+new"),
            entry(Role::Tool, EntryKind::ToolResult, "error: exit status 1"),
            entry(
                Role::Assistant,
                EntryKind::Text,
                "Fixed:\n```diff\n-a\n+b\n```\n- a bullet",
            ),
        ];
        let flow = lay_out_flow(&entries, &[], 1080.0, 2.0);
        let colour = |text: &str| {
            flow.rows
                .iter()
                .find(|r| r.text.trim() == text)
                .unwrap_or_else(|| panic!("no row {text:?} in {:?}", flow.rows))
                .color
        };
        // The user's turn on the grey band, full width.
        let band = flow.fills.iter().find(|f| f.color == BAND).expect("a band");
        assert_eq!((band.rect.x, band.rect.w), (0.0, 1080.0));
        assert!(band.rect.y <= flow.entry_y[0] && band.rect.y + band.rect.h > flow.entry_y[0]);
        assert_eq!(colour("please fix it"), FG);
        // A tool call is one dim line.
        let call: Vec<&Row> = flow
            .rows
            .iter()
            .filter(|r| r.text.starts_with("> Edit"))
            .collect();
        assert_eq!(call.len(), 1);
        assert_eq!(call[0].color, DIM);
        assert!(!flow.rows.iter().any(|r| r.text.trim() == "more"));
        // A diff result: green and red.
        assert_eq!(colour("-old"), RED);
        assert_eq!(colour("+new"), GREEN);
        // An error, red.
        assert_eq!(colour("! exit status 1"), RED);
        // In a reply, diff lines only inside a fence; a bullet stays plain.
        assert_eq!(colour("-a"), RED);
        assert_eq!(colour("+b"), GREEN);
        assert_eq!(colour("- a bullet"), FG);
        assert_eq!(colour("```diff"), DIM);
        // Nothing runs past the phone's width.
        for r in &flow.rows {
            assert!(r.mono, "{r:?}");
            let cols = term_columns(1080.0 - 2.0 * PAD * 2.0 - 16.0, r.px);
            assert!(r.text.chars().count() <= cols + 1, "{r:?}");
        }
    }

    #[test]
    fn a_live_card_is_a_raised_panel_with_its_actions_and_a_resolved_one_a_line() {
        let entries = conversation(1);
        let cards = vec![
            card(CardKind::Question, 4, false),
            card(CardKind::Approval, 4, true),
        ];
        let flow = lay_out_flow(&entries, &cards, 1080.0, 2.0);
        assert_eq!(flow.card_y.len(), 2);
        // The resolved one: a dim line, no panel.
        let faded = flow
            .rows
            .iter()
            .find(|r| r.text.starts_with("-- "))
            .unwrap();
        assert_eq!(faded.color, DIM);
        // The live one: a shadow, a panel and its accent edge, larger text.
        assert!(flow.fills.iter().any(|f| f.color == CARD_BG));
        assert!(flow.fills.iter().any(|f| f.color == SHADOW));
        assert!(
            flow.fills
                .iter()
                .any(|f| f.color == accent(CardKind::Approval))
        );
        let text = flow
            .rows
            .iter()
            .find(|r| r.text == "Shall I go on?")
            .unwrap();
        assert!(text.px > TEXT_PT * 2.0);
        let hits: Vec<AgentHit> = flow.buttons.iter().map(|b| b.hit).collect();
        assert_eq!(hits, [AgentHit::Approve(7), AgentHit::Deny(7)]);
        // After the last entry.
        assert!(flow.card_y[1] > *flow.entry_y.last().unwrap());

        let asking = lay_out_flow(&entries, &[card(CardKind::Question, 4, true)], 1080.0, 2.0);
        let hits: Vec<AgentHit> = asking.buttons.iter().map(|b| b.hit).collect();
        assert_eq!(
            hits,
            [
                AgentHit::Suggest(Suggestion::Yes),
                AgentHit::Suggest(Suggestion::No),
                AgentHit::Message
            ]
        );
        let finished = lay_out_flow(&entries, &[card(CardKind::Finished, 4, true)], 1080.0, 2.0);
        assert!(finished.buttons.is_empty());
    }

    #[test]
    fn it_follows_the_bottom_until_scrolled_back() {
        let mut a = screen(conversation(20), vec![], None);
        assert!(a.following());
        assert!(a.max_offset() > 0.0);
        assert_eq!(a.offset(), a.max_offset());
        // New entries: still at the bottom.
        a.set_content(
            "Claude".into(),
            "idle".into(),
            conversation(21),
            vec![],
            None,
        );
        assert_eq!(a.offset(), a.max_offset());

        // Scroll back: the finger moves down.
        let t = drag(&mut a, 1000.0, 1400.0, 10, Duration::ZERO);
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1400.0,
            t + Duration::from_millis(200),
        );
        assert!(!a.following());
        let back = a.offset();
        assert!((a.max_offset() - back - 400.0).abs() < 1.0, "{back}");
        // New entries do not move it.
        a.set_content(
            "Claude".into(),
            "idle".into(),
            conversation(22),
            vec![],
            None,
        );
        assert_eq!(a.offset(), back);
        // Back down to the bottom: it follows again.
        let t = drag(&mut a, 1400.0, 200.0, 10, t + Duration::from_secs(1));
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            200.0,
            t + Duration::from_millis(200),
        );
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
    }

    #[test]
    fn scrolled_back_the_view_stays_on_its_entry_as_old_ones_leave() {
        let all = conversation(30);
        let mut a = screen(all[..100].to_vec(), vec![], None);
        let t = drag(&mut a, 600.0, 1800.0, 10, Duration::ZERO);
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1800.0,
            t + Duration::from_millis(200),
        );
        let (top, delta) = a.top_entry().unwrap();
        // The window moves on by four entries: the same entry stays on top.
        a.set_content(
            "Claude".into(),
            "idle".into(),
            all[4..104].to_vec(),
            vec![],
            None,
        );
        let (now_top, now_delta) = a.top_entry().unwrap();
        assert_eq!(now_top, top);
        assert!((now_delta - delta).abs() < 0.5);
    }

    #[test]
    fn a_fling_carries_on_and_slows() {
        let mut a = screen(conversation(40), vec![], None);
        let t = drag(&mut a, 800.0, 1100.0, 5, Duration::ZERO);
        let lifted = a.offset();
        a.touch(1, TouchPhase::Ended, 540.0, 1100.0, t + MS);
        let mut moved = Vec::new();
        let mut last = a.offset();
        for _ in 0..30 {
            a.tick(MS);
            moved.push(last - a.offset());
            last = a.offset();
        }
        assert!(a.offset() < lifted - 100.0, "it carried on");
        assert!(moved[0] > moved[20], "and slowed: {moved:?}");
        // A finger that stops before lifting does not fling.
        let mut b = screen(conversation(40), vec![], None);
        let t = drag(&mut b, 800.0, 1100.0, 5, Duration::ZERO);
        b.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1100.0,
            t + Duration::from_millis(300),
        );
        let at = b.offset();
        assert!(!b.tick(MS));
        assert_eq!(b.offset(), at);
    }

    #[test]
    fn a_tap_is_a_tap_and_a_drag_is_not() {
        let mut a = screen(conversation(5), vec![], None);
        let back = a.rect_of(AgentHit::Back).unwrap();
        let (x, y) = (back.x + 4.0, back.y + 4.0);
        a.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        a.touch(1, TouchPhase::Moved, x + 3.0, y + 3.0, MS);
        assert_eq!(
            a.touch(1, TouchPhase::Ended, x + 3.0, y + 3.0, MS * 2),
            Some(AgentHit::Back)
        );
        let message = a.rect_of(AgentHit::Message).unwrap();
        assert!(message.x > back.x + back.w, "side by side");
        // Started on Back, dragged away: no tap.
        a.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        a.touch(1, TouchPhase::Moved, x, y - 200.0, MS);
        assert_eq!(a.touch(1, TouchPhase::Ended, x, y - 200.0, MS * 2), None);
    }

    #[test]
    fn the_pinned_card_is_full_at_the_bottom_and_a_bar_scrolled_back() {
        let entries = conversation(20);
        let n = entries.len();
        let cards = vec![card(CardKind::Approval, n, true)];
        let mut a = screen(entries.clone(), cards.clone(), Some(0));
        let frame = a.frame().clone();
        let pin = frame.pin.as_ref().expect("pinned");
        assert!(!pin.compact);
        let buttons_top = frame.buttons[0].rect.y;
        assert!(pin.rect.y + pin.rect.h <= buttons_top, "above the buttons");
        assert!(
            frame.viewport.y + frame.viewport.h <= pin.rect.y,
            "the flow ends above it"
        );
        // Its Approve and Deny are on the screen.
        let approve = a.rect_of(AgentHit::Approve(7)).expect("Approve on the pin");
        assert!(approve.y >= pin.rect.y);
        let full_offset = a.offset();

        // The finger scrolls back: the bar, and the scroll is the finger's.
        a.touch(1, TouchPhase::Started, 540.0, 900.0, Duration::ZERO);
        a.touch(1, TouchPhase::Moved, 540.0, 960.0, MS);
        let bar = a.frame().pin.clone().unwrap();
        assert!(bar.compact);
        assert!(bar.rect.h < pin.rect.h);
        assert!(
            (a.offset() - (full_offset - 60.0)).abs() < 0.5,
            "exactly the finger's movement: {} vs {}",
            a.offset(),
            full_offset
        );
        a.touch(1, TouchPhase::Moved, 540.0, 1300.0, MS * 2);
        a.touch(1, TouchPhase::Ended, 540.0, 1300.0, MS * 30);
        assert!(
            a.frame().pin.as_ref().unwrap().compact,
            "scrolled back: the bar"
        );
        // The flow's window reaches down to the bar.
        let f = a.frame().clone();
        assert!(f.viewport.y + f.viewport.h > pin.rect.y);

        // Tapping the bar jumps to the card in the flow.
        let bar_rect = a.rect_of(AgentHit::Pin).expect("the bar");
        let tapped = a.touch(
            2,
            TouchPhase::Started,
            bar_rect.x + 10.0,
            bar_rect.y + 5.0,
            Duration::from_secs(2),
        );
        assert_eq!(tapped, None);
        let tap = a.touch(
            2,
            TouchPhase::Ended,
            bar_rect.x + 10.0,
            bar_rect.y + 5.0,
            Duration::from_secs(2) + MS,
        );
        assert_eq!(tap, Some(AgentHit::Pin));
        a.jump_to_pin();
        let card_y = a.flow().card_y[0];
        let v = a.frame().viewport;
        let on_screen = v.y + card_y - a.offset();
        assert!(
            on_screen >= v.y && on_screen < v.y + v.h,
            "the card is in the window"
        );
    }

    #[test]
    fn the_pin_never_moves_the_view() {
        let entries = conversation(20);
        let n = entries.len();
        let mut a = screen(entries.clone(), vec![], None);
        let t = drag(&mut a, 600.0, 1200.0, 10, Duration::ZERO);
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1200.0,
            t + Duration::from_millis(200),
        );
        let back = a.offset();
        // A card arrives and is pinned: the scrolled-back view stays.
        let cards = vec![card(CardKind::Question, n, true)];
        a.set_content(
            "Claude".into(),
            "idle".into(),
            entries.clone(),
            cards,
            Some(0),
        );
        assert_eq!(a.offset(), back);
        assert!(a.frame().pin.as_ref().unwrap().compact);
        // It resolves and leaves the pin: still there.
        let resolved = vec![card(CardKind::Question, n, false)];
        a.set_content("Claude".into(), "idle".into(), entries, resolved, None);
        assert_eq!(a.offset(), back);
        assert!(a.frame().pin.is_none());
    }

    #[test]
    fn flow_buttons_are_hit_only_in_the_window() {
        let entries = conversation(3);
        let cards = vec![card(CardKind::Question, entries.len(), true)];
        // Not pinned, so its only button is the one in the flow.
        let a = screen(entries, cards, None);
        let take = a
            .hits
            .iter()
            .filter(|(h, _)| *h == AgentHit::Message)
            .count();
        assert_eq!(take, 2, "the screen's and the card's");
        let v = a.frame().viewport;
        for (hit, rect) in &a.hits {
            if *hit == AgentHit::Message && rect.y < v.y + v.h {
                assert!(rect.y >= v.y);
            }
        }
    }

    #[test]
    fn taking_a_message_shows_its_card_and_buttons_in_the_view() {
        let mut a = screen(conversation(20), vec![], None);
        let bottom = a.offset();
        assert!(a.rect_of(AgentHit::Back).is_some());
        let lines = vec![
            Line {
                text: "Message for Claude".into(),
                size: 18.0,
                color: FG,
                gap: 4.0,
            },
            Line {
                text: "please go on".into(),
                size: 20.0,
                color: FG,
                gap: 8.0,
            },
        ];
        a.set_dictation(Some(lines));
        let hits: Vec<AgentHit> = a.frame().buttons.iter().map(|b| b.hit).collect();
        assert_eq!(
            hits,
            [
                AgentHit::Message,
                AgentHit::Submit,
                AgentHit::Scratch,
                AgentHit::Clear
            ]
        );
        let card = a.frame().dictation.clone().expect("the card");
        assert!(card.rows.iter().any(|r| r.text == "please go on"));
        let v = a.frame().viewport;
        assert!(v.y + v.h <= card.fills[0].rect.y, "the flow ends above it");
        assert!(a.following());
        assert!(a.offset() > bottom, "still at the bottom, now higher");
        a.set_dictation(None);
        assert!(a.rect_of(AgentHit::Message).is_some());
        assert!(a.frame().dictation.is_none());
    }

    #[test]
    fn the_scrollback_grid_is_the_terminal_face_s() {
        use libmsdf::drawlist::AtlasSet;
        use libmsdf::font::{FontAtlas, NOTO_SANS_MONO_ATLAS, TERMINAL_SYMBOLS_ATLAS};
        let text = FontAtlas::from_bytes(NOTO_SANS_MONO_ATLAS).expect("parses");
        let page = FontAtlas::from_bytes(TERMINAL_SYMBOLS_ATLAS).expect("parses");
        let shaper = TerminalShaper::new(&text, &page).expect("the atlases agree");
        assert!((shaper.cell_em() - CELL_EM).abs() < 1e-4);
        assert!((shaper.line_pitch_em() - PITCH_EM).abs() < 1e-6);

        // Box drawing and Braille come from the symbol page, on the grid.
        let mut set = AtlasSet::new();
        let (t, p) = (set.register(text).unwrap(), set.register(page).unwrap());
        let term = Terminal {
            shaper: &shaper,
            text: set.atlas(t).unwrap(),
            symbols: set.atlas(p).unwrap(),
            px_range: 4.0,
        };
        let line = shaper.line("\u{256d}\u{2500} ok \u{280b}", GlyphStyle::Regular);
        assert!(
            line.segments
                .iter()
                .any(|g| g.source == TerminalSource::Symbols)
        );
        assert_eq!(line.columns, 7);
        let row = Row {
            text: "\u{256d}\u{2500} ok \u{280b}".into(),
            x: 0.0,
            y: 0.0,
            px: 30.0,
            color: FG,
            mono: true,
        };
        let mut list = DrawList::new();
        draw_row(
            &mut list,
            &DrawContext::default(),
            Some(&term),
            &row,
            [0.0, 0.0],
        );
        assert!(!list.lower_with(&set).draws.is_empty());
    }

    #[test]
    fn drawing_rebuilds_only_on_change() {
        let mut a = screen(conversation(3), vec![], None);
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        assert!(a.draw(&mut list, &ctx, None));
        assert!(!a.draw(&mut list, &ctx, None));
        a.set_content(
            "Claude".into(),
            "idle".into(),
            conversation(3),
            vec![],
            None,
        );
        assert!(!a.draw(&mut list, &ctx, None), "the same content");
        a.set_content(
            "Claude".into(),
            "working".into(),
            conversation(3),
            vec![],
            None,
        );
        assert!(a.draw(&mut list, &ctx, None));
    }

    #[test]
    fn back_is_in_the_header_and_the_message_button_bottom_right() {
        let a = screen(conversation(3), vec![], None);
        let f = a.frame();
        assert!(f.back.y + f.back.h <= f.header.y + f.header.h + 0.5);
        assert_eq!(
            a.hit(f.back.x + 10.0, f.back.y + 10.0),
            Some(AgentHit::Back)
        );
        assert_eq!(f.buttons.len(), 1);
        let mic = f.buttons[0].rect;
        assert_eq!(f.buttons[0].hit, AgentHit::Message);
        assert!((mic.x + mic.w - (PHONE.w - PAD * PHONE.scale)).abs() < 0.5);
        assert!(mic.y + mic.h <= PHONE.h - PHONE.bottom);
    }

    #[test]
    fn suggestions_follow_the_pinned_card() {
        // Finished: cont, in yellow, above the buttons.
        let mut a = screen(
            conversation(3),
            vec![card(CardKind::Finished, 3, true)],
            Some(0),
        );
        a.status = "idle".into();
        a.settle();
        let row = &a.frame().suggestions;
        assert_eq!(row.len(), 1);
        assert_eq!(row[0].hit, AgentHit::Suggest(Suggestion::Continue));
        assert_eq!((row[0].label, row[0].color), ("cont", YELLOW));
        assert!(row[0].rect.y + row[0].rect.h <= a.frame().buttons[0].rect.y);
        let r = row[0].rect;
        assert_eq!(
            a.hit(r.x + 5.0, r.y + 5.0),
            Some(AgentHit::Suggest(Suggestion::Continue))
        );
        // A question's yes and no are in its card, not the row.
        let a = screen(
            conversation(3),
            vec![card(CardKind::Question, 3, true)],
            Some(0),
        );
        assert!(a.frame().suggestions.is_empty());
        // Working: none.
        let mut a = screen(
            conversation(3),
            vec![card(CardKind::Broke, 3, true)],
            Some(0),
        );
        assert_eq!(a.suggestions(), [Suggestion::Continue]);
        a.status = "working".into();
        assert!(a.suggestions().is_empty());
        assert_eq!(suggestion_colour(Suggestion::Yes), GREEN);
        assert_eq!(suggestion_colour(Suggestion::No), RED);
    }

    #[test]
    fn the_back_glyph_is_in_the_terminal_face() {
        use libmsdf::font::{FontAtlas, NOTO_SANS_MONO_ATLAS, TERMINAL_SYMBOLS_ATLAS};
        let text = FontAtlas::from_bytes(NOTO_SANS_MONO_ATLAS).expect("parses");
        let page = FontAtlas::from_bytes(TERMINAL_SYMBOLS_ATLAS).expect("parses");
        let shaper = TerminalShaper::new(&text, &page).expect("the atlases agree");
        let line = shaper.line(BACK_GLYPH, GlyphStyle::Regular);
        assert_eq!(line.columns, 1);
        assert!(!line.segments.is_empty(), "{BACK_GLYPH} has a glyph");
    }

    #[test]
    fn scrolled_back_the_view_holds_when_its_top_entry_leaves() {
        let all = conversation(30);
        let mut a = screen(all[..100].to_vec(), vec![], None);
        // Scrolled back to the very top.
        let t = drag(&mut a, 300.0, 2200.0, 40, Duration::ZERO);
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            2200.0,
            t + Duration::from_millis(200),
        );
        a.scroll_to(0.0);
        assert!(!a.following());
        let second = a.entries[1].clone();
        // The window moves on: the top entry leaves, and the next one is at
        // the top, not the old offset into the new content.
        a.set_content(
            "Claude".into(),
            "idle".into(),
            all[1..101].to_vec(),
            vec![],
            None,
        );
        assert!(!a.following());
        assert_eq!((a.entries[0].clone(), a.offset()), (second, 0.0));
        // In the middle, entries leaving above keep the view on the same
        // entry, at the same place, even when its summary changes (a
        // result linked to its call).
        a.scroll_to(a.flow.entry_y[20] + 7.0);
        let (top, delta) = a.top_entry().unwrap();
        let mut changed = all[5..105].to_vec();
        for e in changed.iter_mut() {
            e.summary.push_str(" (linked)");
        }
        a.set_content("Claude".into(), "idle".into(), changed, vec![], None);
        let (now_top, now_delta) = a.top_entry().unwrap();
        assert!(same_entry(&now_top, &top));
        assert!((now_delta - delta).abs() < 0.5);
        assert!(!a.following());
    }
}
