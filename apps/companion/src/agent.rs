//! The agent view (plan 20261022_03, decision 9, as amended at 02:34 EDT:
//! "the agent view as terminal scrollback with priority cards", and "L3-1:
//! pinned to the bottom, scrolling overrides").
//!
//! One flow per agent: its transcript laid out at the phone's width in a
//! terminal style (a dark background, bright colours, the user's turns on a
//! grey band, replies plain, tool calls as dim one-liners, diff lines green
//! and red, errors red, and context such as a finished task or a peer's
//! message as a dim line after an amber dot), with the priority cards at
//! their places in it.
//! Live cards are raised panels with an accent edge, larger text and their
//! actions; resolved ones are one dim line.
//!
//! **Scrolling** is the app's own: libcontrols has no scroll view. A finger
//! drags the flow, and a fling carries on with momentum that decays. The
//! view follows the newest entry while it is at the bottom; scrolled back,
//! it stays where it is as sections arrive, and follows again once
//! scrolled back to the bottom.
//!
//! **The ribbon** (the 23:54 EDT amendment): the scrollback is the view's
//! sections end to end, in serial order, one block each, below a marker row
//! of fixed height. A block is laid out once, when its section arrives or
//! changes, and cached by serial; scrolled back, the place is the serial of
//! the block at the window's top and the distance into it.
//!
//! **The pinned card** (the highest live one) sits above the buttons. At
//! the bottom it shows in full; while the finger scrolls back, or the view
//! is scrolled back, it is a one-line bar, and tapping the bar goes back to
//! the bottom. It is shown only there, never also in the flow (owner,
//! 2026-10-23: "got two cards instead of one with the same content"); the
//! flow keeps its place. The pin never moves the flow: the flow's bottom is
//! always laid out above the full card, so changing between the card and
//! the bar only uncovers or covers space below the flow.

use std::time::Duration;

use std::collections::{HashMap, HashSet};

use handlr_companion::{AgentCard, CardKind, Shows, Suggestion, ViewSection};
use libcontrols::{Button, DrawContext, DrawControl, Rect};
use libhandlr_proto::common::{Entry, EntryKind, Link, Role, Serial};
use libmorph::TouchPhase;
use libmsdf::drawlist::{AtlasRef, DrawList, SdfInstance, SdfKind};
#[cfg(test)]
use libmsdf::font::TerminalSource;
use libmsdf::font::{GlyphStyle, TerminalShaper};

use crate::controls::{ControlRow, SquareButton};
use crate::ui::{Layout, Line};

mod dictation_card;
mod input_box;
pub mod link_spans;
mod notice_line;
use dictation_card::DictationCard;
use input_box::InputBox;
use link_spans::LinkSpan;
use notice_line::NoticeLine;

pub const BG: [f32; 4] = [0.03, 0.03, 0.04, 1.0];
pub const FG: [f32; 4] = [0.94, 0.95, 0.96, 1.0];
pub const DIM: [f32; 4] = [0.50, 0.53, 0.58, 1.0];
pub const GREEN: [f32; 4] = [0.36, 0.93, 0.45, 1.0];
pub const RED: [f32; 4] = [1.0, 0.40, 0.40, 1.0];
pub const YELLOW: [f32; 4] = [1.0, 0.82, 0.30, 1.0];
pub const BAND: [f32; 4] = [0.18, 0.19, 0.21, 1.0];
pub(crate) const CARD_BG: [f32; 4] = [0.11, 0.12, 0.17, 1.0];
const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const HEADER_BG: [f32; 4] = [0.07, 0.07, 0.09, 1.0];
pub(crate) const APPROVE_BG: [f32; 4] = [0.16, 0.45, 0.25, 1.0];
pub(crate) const DENY_BG: [f32; 4] = [0.55, 0.18, 0.18, 1.0];
pub(crate) const MESSAGE_BG: [f32; 4] = [0.20, 0.32, 0.55, 1.0];
pub(crate) const BACK_BG: [f32; 4] = [0.30, 0.30, 0.36, 1.0];
/// A plan question's implement button: approve and build.
const DICTATION_BG: [f32; 4] = [0.09, 0.10, 0.13, 1.0];

/// A card kind's accent colour.
pub fn accent(kind: CardKind) -> [f32; 4] {
    match kind {
        CardKind::Approval => [1.0, 0.74, 0.25, 1.0],
        CardKind::Question => [0.42, 0.72, 1.0, 1.0],
        CardKind::Broke => RED,
        CardKind::Finished => GREEN,
        // A kind from a newer daemon.
        _ => DIM,
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
pub(crate) const PAD: f32 = 12.0;
const EDGE: f32 = 6.0;
const CARD_PAD: f32 = 12.0;
pub(crate) const ACTION_H: f32 = 44.0;
pub(crate) const BUTTON_H: f32 = 52.0;
const HEADER_H: f32 = 64.0;
/// The header's "terminal on: <name>" line, when herdr's focus is on
/// another pane than the view's.
const TERMINAL_ON_H: f32 = 18.0;
/// Entries and cards more than this many screens from the window keep
/// their place but not their laid-out rows (the 22:55 EDT amendment).
pub const KEEP_SCREENS: f32 = 5.0;
const BAR_H: f32 = 40.0;
/// The suggestion row's height and type size, and the back glyph's tap
/// width at the header's left.
const SUGGEST_H: f32 = 36.0;
const SUGGEST_PT: f32 = 18.0;
const BACK_W: f32 = 44.0;
/// The record dot: white on blue, red while recording, centred in place of
/// a microphone glyph, which no bundled atlas has yet (a microphone icon
/// is for later).
pub(crate) const DOT: f32 = 18.0;
pub(crate) const RECORDING: [f32; 4] = [1.0, 0.35, 0.35, 1.0];
/// Send and cancel, from the terminal face's symbol page.
pub(crate) const SEND_GLYPH: &str = "\u{27A4}";
pub(crate) const CANCEL_GLYPH: &str = "\u{2715}";
/// The back glyph, from the terminal face (Roboto here is ASCII only).
const BACK_GLYPH: &str = "\u{2039}";
/// The play button's glyph, from the terminal face, and the skip glyph it
/// shows while a reply is being read (no ⏭ in the face: play and a bar).
pub(crate) const PLAY_GLYPH: &str = "\u{25B6}";
pub(crate) const SKIP_GLYPH: &str = "\u{25B6}\u{2502}";
/// Lines of a tool result shown: one, or a diff's or an error's first few.
const RESULT_LINES: usize = 12;
const ERROR_LINES: usize = 4;
/// Lines of the pinned card's text.
const PIN_LINES: usize = 3;
/// Lines of the dictation card shown at most: its tail.
const DICTATION_LINES: usize = 6;

/// A line height for type of `px` pixels.
pub(crate) fn line_h(px: f32) -> f32 {
    px * 1.35
}

/// The terminal face's cell and line pitch, in em: Noto Sans Mono's 0.6,
/// and the 1.3 em at which the symbol page's box drawing joins (libmsdf's
/// `TerminalShaper::cell_em` and `line_pitch_em`).
pub(crate) const CELL_EM: f32 = 0.6;
const PITCH_EM: f32 = 1.3;

/// A scrollback line's height for type of `px` pixels.
pub(crate) fn term_h(px: f32) -> f32 {
    px * PITCH_EM
}

/// How many cells of `px`-pixel terminal type fit in `width`.
fn term_columns(width: f32, px: f32) -> usize {
    (width / (px * CELL_EM)).max(8.0) as usize
}

/// How many characters of `px`-pixel type fit in `width`: roughly half an
/// em each in Roboto, as the main screen counts.
pub(crate) fn columns(width: f32, px: f32) -> usize {
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
    /// The play button: read the agent's replies aloud, or stop.
    ReadAloud,
    /// A card's record button: take a message in reply to that card.
    Reply,
    /// The pinned bar: jump to its card.
    Pin,
    /// The pinned card's text, cut to its first lines: show it all, or
    /// fold it again.
    Expand,
    /// The dictation buttons, while a message is taken from the view.
    Submit,
    Scratch,
    Clear,
    /// A link's text: its index in the screen's links
    /// ([`AgentScreen::link`]).
    Link(usize),
    /// The jump line: back to the bottom, following.
    JumpToEnd,
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
    /// The links' text in the rows.
    pub links: Vec<LinkSpan>,
    /// The links the spans index: every entry's and card's, once.
    pub targets: Vec<Link>,
    /// The span of the flow's y whose entries and cards have their rows
    /// laid out; `None` for all of them.
    pub kept: Option<(f32, f32)>,
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
pub(crate) fn one_line(text: &str, cols: usize) -> String {
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
        // A question about a plan (plan 20261022_03, the 21:57 EDT
        // amendment): just the three words, as plain coloured text, not
        // buttons (owner, 2026-10-23: "Yes/No are buttons but suggestions
        // (approve, impl) can be plain text"). The colour stands in the
        // background slot; `SquareButton::draw` draws these as text.
        (CardKind::Question, _) if card.about_plan => vec![
            (AgentHit::Suggest(Suggestion::Approve), "approve", GREEN),
            (
                AgentHit::Suggest(Suggestion::Discuss),
                "discuss",
                link_spans::LINK,
            ),
            (
                AgentHit::Suggest(Suggestion::Implement),
                "implement",
                YELLOW,
            ),
            (AgentHit::Reply, "", MESSAGE_BG),
        ],
        (CardKind::Question, _) => vec![
            (AgentHit::Suggest(Suggestion::Yes), "Yes", APPROVE_BG),
            (AgentHit::Suggest(Suggestion::No), "No", DENY_BG),
            (AgentHit::Reply, "", MESSAGE_BG),
        ],
        _ => vec![],
    }
}

/// A live card's block at `y`, `w` wide from `x`: its fills, rows and
/// buttons, and its height. At most `max_lines` lines of its text. Its
/// links are marked by their index in `targets`.
#[allow(clippy::too_many_arguments)]
fn card_block(
    card: &AgentCard,
    x: f32,
    y: f32,
    w: f32,
    s: f32,
    max_lines: usize,
    targets: &[Link],
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
    let first_text = flow.rows.len();
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
    link_spans::mark(flow, first_text, &card.links, targets);
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
/// The pinned card (`pinned`, an index into `cards`) is not drawn in the
/// flow: it shows once, at the bottom, and keeps only its place here.
pub fn lay_out_flow(
    entries: &[Entry],
    cards: &[AgentCard],
    pinned: Option<usize>,
    width: f32,
    s: f32,
) -> Flow {
    lay_out_flow_in(entries, cards, pinned, width, s, None, None)
}

/// The marker row at the flow's top while history pages in, or once the
/// session's start is reached (plan 20261023_03, A7).
pub const LOADING_EARLIER: &str = "loading earlier\u{2026}";
pub const START_OF_SESSION: &str = "start of session";

/// The flow's top padding, in points, above the marker row.
const TOP_PAD: f32 = 8.0;

/// The marker row's fixed height at scale `s`, with the padding around it
/// (the 23:54 EDT amendment, item 4): the ribbon's first section starts
/// this far down, whatever the marker says.
fn ribbon_top(s: f32) -> f32 {
    TOP_PAD * s + line_h(TOOL_PT * s) + 8.0 * s
}

/// The marker row, at the flow's top.
fn marker_row(text: &str, s: f32) -> Row {
    Row {
        text: text.to_string(),
        x: PAD * s,
        y: TOP_PAD * s,
        px: TOOL_PT * s,
        color: DIM,
        mono: false,
    }
}

/// What the scrollback is laid out with: the width, the scale, and the
/// text's margin and width.
#[derive(Clone, Copy)]
struct Pen {
    width: f32,
    s: f32,
    pad: f32,
    text_w: f32,
}

impl Pen {
    fn new(width: f32, s: f32) -> Self {
        let pad = PAD * s;
        Pen {
            width,
            s,
            pad,
            text_w: width - 2.0 * pad,
        }
    }

    /// Terminal cells of `pt`-point type across the text.
    fn cols(&self, pt: f32) -> usize {
        term_columns(self.text_w - 8.0 * self.s, pt * self.s)
    }
}

/// Lays out `card` at `y` into `flow`; the y after it. A live card that
/// is `pinned` shows at the bottom instead, and keeps only its place.
fn lay_out_card(
    card: &AgentCard,
    pinned: bool,
    y: f32,
    pen: Pen,
    targets: &[Link],
    flow: &mut Flow,
) -> f32 {
    let s = pen.s;
    flow.card_y.push(y);
    if card.live && pinned {
        // Shown at the bottom instead.
        y
    } else if card.live {
        let y = y + 8.0 * s;
        *flow.card_y.last_mut().expect("pushed") = y;
        y + card_block(card, pen.pad, y, pen.text_w, s, usize::MAX, targets, flow) + 12.0 * s
    } else {
        // Resolved: ordinary scrollback, one dim line.
        let text = format!("-- {}: {}", card.title, card.text.replace('\n', " "));
        flow.rows.push(Row {
            text: one_line(&text, pen.cols(TOOL_PT)),
            x: pen.pad,
            y,
            px: TOOL_PT * s,
            color: DIM,
            mono: true,
        });
        y + term_h(TOOL_PT * s) + 4.0 * s
    }
}

/// Lays out entry `e` at `y` into `flow`, after an entry that
/// `previous_compacts`; the y after it.
fn lay_out_entry(
    e: &Entry,
    previous_compacts: bool,
    y: f32,
    pen: Pen,
    targets: &[Link],
    flow: &mut Flow,
) -> f32 {
    let s = pen.s;
    let mut y = y;
    if let Some(origin) = e.origin.as_deref() {
        // Context: one dim line or two, after an amber dot.
        if NoticeLine::repeats(e, previous_compacts) {
            flow.entry_y.push(y);
            return y;
        }
        y += 4.0 * s;
        flow.entry_y.push(y);
        return y + NoticeLine::lay_out(e, origin, pen.pad, y, pen.cols(TOOL_PT), s, targets, flow);
    }
    let (lines, band) = entry_lines(e, |pt| pen.cols(pt));
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
            rect: Rect::new(0.0, y - 4.0 * s, pen.width, h + 8.0 * s),
            color: BAND,
            radius: 0.0,
        });
    }
    let mut ly = y;
    let first_row = flow.rows.len();
    for (text, pt, color) in lines {
        let px = pt * s;
        let lh = term_h(px);
        if text.is_empty() {
            ly += lh * 0.5;
            continue;
        }
        flow.rows.push(Row {
            text,
            x: pen.pad,
            y: ly,
            px,
            color,
            mono: true,
        });
        ly += lh;
    }
    link_spans::mark(flow, first_row, &e.links, targets);
    ly + if band { 8.0 * s } else { 0.0 }
}

/// As [`lay_out_flow`], with `marker` (a dim row) at the top, and only the
/// rows, fills, buttons and links of the entries and cards that meet
/// `keep` (a span of the flow's y): the rest keep their place and height
/// but no laid-out rows (the 22:55 EDT amendment's memory guard). With a
/// marker, the entries start where the ribbon's do.
pub fn lay_out_flow_in(
    entries: &[Entry],
    cards: &[AgentCard],
    pinned: Option<usize>,
    width: f32,
    s: f32,
    marker: Option<&str>,
    keep: Option<(f32, f32)>,
) -> Flow {
    let mut flow = Flow {
        kept: keep,
        ..Flow::default()
    };
    let targets = link_spans::targets(
        entries
            .iter()
            .map(|e| &e.links[..])
            .chain(cards.iter().map(|c| &c.links[..])),
    );
    let pen = Pen::new(width, s);
    let mut y = TOP_PAD * s;
    if let Some(text) = marker {
        flow.rows.push(marker_row(text, s));
        y = ribbon_top(s);
    }
    // What has been pushed when an item starts; dropped again when the
    // item is outside `keep`.
    let mark = |flow: &Flow| {
        (
            flow.rows.len(),
            flow.fills.len(),
            flow.buttons.len(),
            flow.links.len(),
        )
    };
    let prune = |flow: &mut Flow, at: (usize, usize, usize, usize), top: f32, bottom: f32| {
        if let Some((lo, hi)) = keep
            && (bottom < lo || top > hi)
        {
            flow.rows.truncate(at.0);
            flow.fills.truncate(at.1);
            flow.buttons.truncate(at.2);
            flow.links.truncate(at.3);
        }
    };
    let mut next_card = 0;
    for i in 0..=entries.len() {
        while let Some(card) = cards.get(next_card).filter(|c| c.at <= i) {
            let at = mark(&flow);
            let top = y;
            y = lay_out_card(card, pinned == Some(next_card), y, pen, &targets, &mut flow);
            prune(&mut flow, at, top, y);
            next_card += 1;
        }
        let Some(e) = entries.get(i) else { break };
        let at = mark(&flow);
        let top = y;
        let previous_compacts = i
            .checked_sub(1)
            .and_then(|p| entries.get(p))
            .is_some_and(NoticeLine::compacts);
        y = lay_out_entry(e, previous_compacts, y, pen, &targets, &mut flow);
        prune(&mut flow, at, top, y);
    }
    flow.height = y + 16.0 * s;
    flow.targets = targets;
    flow
}

/// One section's block of the ribbon (the 23:54 EDT amendment, items 1
/// and 2), laid out at y = 0, `width` pixels wide at scale `s`: its rows,
/// fills, buttons and link spans, its entries' and card's tops, and its
/// height. Its links are its own (`targets`), which the screen's flow maps
/// to its own. A live card that is `pinned` keeps no height; the first
/// entry comes after one that `after_compact`.
pub fn lay_out_block(shows: &Shows, pinned: bool, after_compact: bool, width: f32, s: f32) -> Flow {
    let mut flow = Flow::default();
    let pen = Pen::new(width, s);
    let mut y = 0.0;
    match shows {
        Shows::Entries(entries) => {
            flow.targets = link_spans::targets(entries.iter().map(|e| &e.links[..]));
            let targets = std::mem::take(&mut flow.targets);
            let mut previous_compacts = after_compact;
            for e in entries {
                y = lay_out_entry(e, previous_compacts, y, pen, &targets, &mut flow);
                previous_compacts = NoticeLine::compacts(e);
            }
            flow.targets = targets;
        }
        Shows::Card(card) => {
            let targets = link_spans::targets([&card.links[..]]);
            y = lay_out_card(card, pinned, y, pen, &targets, &mut flow);
            flow.targets = targets;
        }
    }
    flow.height = y;
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
    /// The fling running when the finger came down, if any: a quick flick
    /// the same way adds to it.
    carried: f32,
    /// When the finger came down.
    down: Duration,
    /// The latest samples, oldest first, for the release speed. It is
    /// measured over a span, not between two moves, because moves can
    /// arrive together, microseconds apart, and divide into absurd speeds
    /// (the S23 logged flings of 2.7 million px/s, the "snap back").
    samples: [(Duration, f32); SAMPLES],
    sampled: usize,
}

impl Drag {
    fn sample(&mut self, t: Duration, y: f32) {
        if self.sampled == SAMPLES {
            self.samples.copy_within(1.., 0);
            self.sampled -= 1;
        }
        self.samples[self.sampled] = (t, y);
        self.sampled += 1;
    }
}

/// The finger's speed at release, in pixels per second: over the span
/// from the window's start to the last move, none for a span too short to
/// measure, and at most `MAX_FLING`.
fn release_speed(d: &Drag, s: f32) -> f32 {
    let samples = &d.samples[..d.sampled];
    let Some(&(last_t, _)) = samples.last() else {
        return 0.0;
    };
    // The samples within the window, and at least one before it when the
    // window holds a span too short (moves can come in bursts).
    let mut i = samples
        .iter()
        .position(|&(t, _)| last_t.saturating_sub(t) <= SPEED_WINDOW)
        .unwrap_or(0);
    if last_t.saturating_sub(samples[i].0) < SPEED_SPAN_MIN {
        i = i.saturating_sub(1);
    }
    let window = &samples[i..];
    let span = last_t.saturating_sub(window[0].0);
    if span < SPEED_SPAN_MIN {
        return 0.0;
    }
    // A least-squares line through them, weighted toward the newest, so
    // the speed is the finger's at release, not the flick's average.
    let (mut sw, mut st, mut sy, mut stt, mut sty) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
    for &(t, y) in window {
        let age = last_t.saturating_sub(t).as_secs_f32();
        let x = -age;
        let w = 1.0 - 0.5 * (age / SPEED_WINDOW.as_secs_f32()).min(1.0);
        sw += w;
        st += w * x;
        sy += w * y;
        stt += w * x * x;
        sty += w * x * y;
    }
    let den = sw * stt - st * st;
    if den.abs() < f32::EPSILON {
        return 0.0;
    }
    let v = (sw * sty - st * sy) / den;
    v.clamp(-MAX_FLING * s, MAX_FLING * s)
}

/// A finger speed to a fling's: the same up to `FLING_KNEE`, growing as
/// its power `FLING_POWER` above it.
fn fling_curve(v: f32, s: f32) -> f32 {
    let knee = FLING_KNEE * s;
    if v.abs() <= knee {
        return v;
    }
    v.signum() * knee * (v.abs() / knee).powf(FLING_POWER)
}

/// A movement under this many points is a tap.
pub(crate) const SLOP: f32 = 10.0;
/// The longest frame a fling advances by, in seconds.
pub(crate) const FLING_DT_MAX: f32 = 1.0 / 30.0;
/// The momentum's decay per second: the speed falls to `e^-DECAY` of
/// itself each second.
pub(crate) const DECAY: f32 = 2.0;
/// Slower than this (points per second), a fling stops.
pub(crate) const MIN_FLING: f32 = 30.0;
/// The fastest finger speed measured (points per second), as libcontrols'
/// scroll pane.
pub(crate) const MAX_FLING: f32 = 8000.0;
/// Above this finger speed (points per second), a flick's force counts
/// for more than its speed: the fling grows as the speed to the power
/// `FLING_POWER`, so a harder flick goes disproportionately further, as on
/// Android, where a fling's distance grows as about speed^1.7. Below it, a
/// gentle flick is followed exactly.
const FLING_KNEE: f32 = 1000.0;
const FLING_POWER: f32 = 1.5;
/// The fastest fling after that curve, and after flicks add up.
pub(crate) const MAX_FLING_OUT: f32 = 16000.0;
/// The span the release speed is measured over, back from the last move.
const SPEED_WINDOW: Duration = Duration::from_millis(100);
/// A flick within this long of touching a running fling, the same way,
/// adds to it (as Android and iOS do).
const FLING_BOOST_TOUCH: Duration = Duration::from_millis(400);
/// Samples kept for it.
const SAMPLES: usize = 16;
/// A shorter span gives no speed: too little to measure.
const SPEED_SPAN_MIN: Duration = Duration::from_millis(16);
/// How long the agent must be quiet, in seconds, before continue is
/// suggested after a finished turn.
const QUIET_FOR_CONTINUE: f32 = 30.0;

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
    /// The response just sent, as typed input in its box.
    pub typed: Option<Flow>,
    /// The jump line, while scrolled back.
    pub jump: Option<JumpLine>,
}

/// The one-line "jump to end" message below the scrollback, while
/// scrolled back (plan 20261022_03, the 00:05 EDT amendment): its tap area,
/// the screen's width, and its text, drawn dim and centred.
#[derive(Clone, Debug, PartialEq)]
pub struct JumpLine {
    pub rect: Rect,
    pub text: String,
}

/// The jump line's height, in points, kept whether or not it shows.
const JUMP_H: f32 = 28.0;
/// The jump line with nothing new.
pub const JUMP_TO_END: &str = "-- jump to end --";

/// The first line of what a section shows: its first entry's text (a
/// turn's prompt, a reply, a tool call's summary), or its card's title.
fn first_line(shows: &Shows) -> Option<String> {
    let line = match shows {
        Shows::Entries(entries) => entries
            .iter()
            .find_map(|e| e.summary.lines().map(str::trim).find(|l| !l.is_empty()))?,
        Shows::Card(card) => card.title.trim(),
    };
    (!line.is_empty()).then(|| line.to_string())
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
        Suggestion::Approve | Suggestion::Implement => GREEN,
        Suggestion::Discuss => link_spans::LINK,
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
pub(crate) fn draw_row(
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

/// One section's place in the ribbon (the 23:54 EDT amendment, item 1):
/// its index (increasing with the serial, as the client gives it), its
/// serial, and its block's height in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub index: u64,
    pub serial: Serial,
    pub height: f32,
}

/// A section's block, cached by serial (item 2).
#[derive(Debug)]
struct Block {
    shows: Shows,
    /// A live card shown at the bottom instead: no height here.
    pinned: bool,
    /// The first entry comes after a compaction's line.
    after_compact: bool,
    /// Laid out at y = 0: its height, its entries' and card's tops and its
    /// links always; its rows, fills, buttons and link spans while `rows`.
    flow: Flow,
    rows: bool,
    /// To be laid out again: new or different content, the width or scale
    /// changed, or its pin or neighbour did.
    stale: bool,
}

impl Block {
    fn new(shows: Shows) -> Self {
        Block {
            shows,
            pinned: false,
            after_compact: false,
            flow: Flow::default(),
            rows: false,
            stale: true,
        }
    }

    fn lay_out(&mut self, width: f32, s: f32) {
        self.flow = lay_out_block(&self.shows, self.pinned, self.after_compact, width, s);
        self.rows = true;
        self.stale = false;
    }

    /// The memory guard: only its height, tops and links stay.
    fn drop_rows(&mut self) {
        self.flow.rows = Vec::new();
        self.flow.fills = Vec::new();
        self.flow.buttons = Vec::new();
        self.flow.links = Vec::new();
        self.rows = false;
    }

    fn card(&self) -> Option<&AgentCard> {
        match &self.shows {
            Shows::Card(card) => Some(card),
            _ => None,
        }
    }
}

/// The agent view's screen: content, scroll state and layout.
#[derive(Default)]
pub struct AgentScreen {
    pub name: String,
    pub status: String,
    /// The ribbon: the view's sections in index order (the 23:54 EDT
    /// amendment, item 1).
    slots: Vec<Slot>,
    /// The top of each slot's block in the flow, in pixels: the marker row,
    /// then the heights before it.
    tops: Vec<f32>,
    /// The blocks, by packed serial.
    blocks: HashMap<u64, Block>,
    pinned: Option<Serial>,
    /// The width and scale the blocks were laid out at.
    laid_at: (f32, f32),
    layout: Layout,
    /// The kept blocks' rows put together in flow coordinates, for drawing
    /// and hit testing.
    flow: Flow,
    /// The flow is to be put together again.
    flow_stale: bool,
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
    /// The play button is on: replies are read aloud.
    read_aloud: bool,
    /// Something is being said: the play button skips it.
    speaking: bool,
    /// The session has not answered yet.
    loading: bool,
    /// The pinned card shows all its text (owner, 2026-10-23: "clicking an
    /// abbreviated question should expand the text and card").
    pin_expanded: bool,
    /// Seconds since the entries or the status last changed.
    quiet_for: f32,
    /// The response just sent, until the session shows it.
    typed: Option<String>,
    /// The card a message is being taken in reply to.
    reply: Option<AgentCard>,
    /// herdr's focus, shown, when it is on another pane than the view's.
    terminal_on: Option<String>,
    /// A history page is on its way: "loading earlier…" at the top.
    loading_earlier: bool,
    /// A page reached the session's first line: "start of session".
    history_start: bool,
    /// The flow's top came within a screen of the window's: the client
    /// is to page in earlier sections, once per approach.
    wants_earlier: bool,
    /// Taken by the last approach to the top; armed again once the top is
    /// left, or a page lands.
    earlier_asked: bool,
    /// The newest section held when the view left the bottom: those after
    /// it are new, for the jump line.
    left_at: Option<u64>,
    dirty: bool,
    /// Blocks laid out, counted for the tests.
    #[cfg(test)]
    block_layouts: usize,
}

impl AgentScreen {
    pub fn new() -> Self {
        AgentScreen {
            follow: true,
            dirty: true,
            loading: true,
            flow_stale: true,
            ..Default::default()
        }
    }

    /// New content: the view's sections in serial order, the pinned card's
    /// serial, and the serials `changed` since the last (the 23:54 EDT
    /// amendment, item 5). Only the changed sections' blocks, and those not
    /// held yet, are laid out again. Scrolled back, the view stays on the
    /// section at its top (item 3).
    pub fn set_content(
        &mut self,
        name: String,
        status: String,
        sections: Vec<ViewSection>,
        pinned: Option<Serial>,
        changed: &[Serial],
    ) {
        let same_slots = sections.len() == self.slots.len()
            && sections
                .iter()
                .zip(&self.slots)
                .all(|(v, s)| v.serial == s.serial && v.index == s.index);
        if self.name == name
            && self.status == status
            && changed.is_empty()
            && self.pinned == pinned
            && same_slots
        {
            return;
        }
        let changed: HashSet<u64> = changed.iter().map(|s| s.to_u64()).collect();
        let is_entries = |b: Option<&Block>| matches!(b.map(|b| &b.shows), Some(Shows::Entries(_)));
        let mut entries_changed = changed.iter().any(|k| is_entries(self.blocks.get(k)));
        if self.status != status {
            self.quiet_for = 0.0;
        }
        let keep = self.keep_anchors();
        let pin_was = self.pinned_card().map(|c| (c.kind, c.text.clone()));
        let mut slots = Vec::with_capacity(sections.len());
        for v in sections {
            let key = v.serial.to_u64();
            if changed.contains(&key) || !self.blocks.contains_key(&key) {
                entries_changed |= matches!(v.shows, Shows::Entries(_));
                self.blocks.insert(key, Block::new(v.shows));
            }
            slots.push(Slot {
                index: v.index,
                serial: v.serial,
                height: self.blocks[&key].flow.height,
            });
        }
        self.slots = slots;
        if self.blocks.len() != self.slots.len() {
            // Sections gone: their blocks go too.
            let held: HashSet<u64> = self.slots.iter().map(|s| s.serial.to_u64()).collect();
            self.blocks.retain(|k, _| held.contains(k));
        }
        if entries_changed {
            self.quiet_for = 0.0;
        }
        self.name = name;
        self.status = status;
        self.pinned = pinned;
        // A different pinned card starts folded.
        if self.pinned_card().map(|c| (c.kind, c.text.clone())) != pin_was {
            self.pin_expanded = false;
        }
        self.refresh_blocks();
        self.hold(keep);
        self.flow_stale = true;
        self.settle();
    }

    /// The anchors to hold the view still by, when scrolled back.
    fn keep_anchors(&self) -> Vec<(u64, f32)> {
        if self.follow {
            Vec::new()
        } else {
            self.anchors()
        }
    }

    /// Scrolled back, puts the section that was at the window's top back
    /// at the same place on screen, or, if it went, the next one still
    /// held at its own place (item 3): sections paged in above it, or
    /// replaced below it, never move it.
    fn hold(&mut self, keep: Vec<(u64, f32)>) {
        if let Some((i, delta)) = keep
            .iter()
            .find_map(|&(serial, delta)| self.slot_of(serial).map(|i| (i, delta)))
        {
            let to = self.tops[i] + delta;
            if (to - self.offset).abs() > 2.0 {
                log::info!(
                    "agent view: held on {}, {:.0} -> {:.0}",
                    self.slots[i].serial,
                    self.offset,
                    to
                );
            }
            self.offset = to;
        } else if !keep.is_empty() {
            log::info!("agent view: no anchor held of {}", keep.len());
        }
    }

    /// The slot of the section `serial` (packed), if held.
    fn slot_of(&self, serial: u64) -> Option<usize> {
        self.slots
            .binary_search_by_key(&serial, |s| s.serial.to_u64())
            .ok()
    }

    /// The slot whose block is at `y` of the flow: the last that starts at
    /// or above it.
    fn slot_at(&self, y: f32) -> Option<usize> {
        self.tops.partition_point(|&t| t <= y).checked_sub(1)
    }

    /// Lays out the blocks that are stale (all of them when the width or
    /// scale changed), then the slots' heights and the running tops.
    fn refresh_blocks(&mut self) {
        let (w, s) = (self.layout.w, self.scale());
        let redo = self.laid_at != (w, s);
        self.laid_at = (w, s);
        let pinned = self.pinned.map(Serial::to_u64);
        let mut after_compact = false;
        let mut top = ribbon_top(s);
        self.tops.clear();
        #[cfg(test)]
        let mut laid = 0;
        for slot in &mut self.slots {
            let key = slot.serial.to_u64();
            let b = self.blocks.get_mut(&key).expect("a block for each slot");
            // Only what changes the block counts: the pin for a live card,
            // the neighbour for a first entry that is a compaction's.
            let is_pinned = pinned == Some(key) && b.card().is_some_and(|c| c.live);
            let after = after_compact
                && matches!(&b.shows, Shows::Entries(e) if e.first().is_some_and(NoticeLine::compacts));
            if redo || b.pinned != is_pinned || b.after_compact != after {
                b.stale = true;
            }
            b.pinned = is_pinned;
            b.after_compact = after;
            if b.stale {
                b.lay_out(w, s);
                #[cfg(test)]
                {
                    laid += 1;
                }
            }
            if let Shows::Entries(e) = &b.shows
                && let Some(last) = e.last()
            {
                after_compact = NoticeLine::compacts(last);
            }
            slot.height = b.flow.height;
            self.tops.push(top);
            top += slot.height;
        }
        #[cfg(test)]
        {
            self.block_layouts += laid;
        }
    }

    /// The flow's height: the marker row, the blocks, and the bottom
    /// padding.
    fn ribbon_height(&self) -> f32 {
        let s = self.scale();
        let end = match (self.tops.last(), self.slots.last()) {
            (Some(top), Some(slot)) => top + slot.height,
            _ => ribbon_top(s),
        };
        end + 16.0 * s
    }

    /// The marker row's text: "loading earlier…", "start of session", or
    /// nothing.
    fn marker(&self) -> &'static str {
        if self.loading_earlier {
            LOADING_EARLIER
        } else if self.history_start {
            START_OF_SESSION
        } else {
            ""
        }
    }

    /// Puts the flow together from the blocks: the marker row, then each
    /// block's tops, and the rows, fills, buttons and links of those within
    /// the kept span, moved to the block's top. A block that comes into the
    /// span is laid out again; one that leaves it keeps only its height
    /// (the memory guard, per block).
    fn assemble(&mut self) {
        let s = self.scale();
        let (w, _) = self.laid_at;
        let keep = self.keep_span();
        let (lo, hi) = keep;
        let mut flow = Flow {
            kept: Some(keep),
            ..Flow::default()
        };
        flow.rows.push(marker_row(self.marker(), s));
        let pinned = self.pinned.map(Serial::to_u64);
        #[cfg(test)]
        let mut laid = 0;
        for (slot, &top) in self.slots.iter().zip(&self.tops) {
            let key = slot.serial.to_u64();
            let b = self.blocks.get_mut(&key).expect("a block for each slot");
            let inside = top + slot.height >= lo && top <= hi;
            if inside && !b.rows {
                b.lay_out(w, s);
                #[cfg(test)]
                {
                    laid += 1;
                }
            } else if !inside && b.rows {
                b.drop_rows();
            }
            let f = &b.flow;
            flow.entry_y.extend(f.entry_y.iter().map(|y| y + top));
            flow.card_y.extend(f.card_y.iter().map(|y| y + top));
            if !b.rows && pinned != Some(key) {
                continue;
            }
            // The block's links, as the flow's.
            let map: Vec<usize> = f
                .targets
                .iter()
                .map(|l| match flow.targets.iter().position(|t| t == l) {
                    Some(i) => i,
                    None => {
                        flow.targets.push(l.clone());
                        flow.targets.len() - 1
                    }
                })
                .collect();
            if !b.rows {
                continue;
            }
            let first_row = flow.rows.len();
            flow.rows.extend(f.rows.iter().map(|r| Row {
                y: r.y + top,
                ..r.clone()
            }));
            flow.fills.extend(f.fills.iter().map(|x| Fill {
                rect: Rect::new(x.rect.x, x.rect.y + top, x.rect.w, x.rect.h),
                ..*x
            }));
            flow.buttons.extend(f.buttons.iter().map(|x| FlowButton {
                rect: Rect::new(x.rect.x, x.rect.y + top, x.rect.w, x.rect.h),
                ..x.clone()
            }));
            flow.links.extend(f.links.iter().map(|x| LinkSpan {
                row: x.row + first_row,
                link: map[x.link],
                ..*x
            }));
        }
        #[cfg(test)]
        {
            self.block_layouts += laid;
        }
        flow.height = self.ribbon_height();
        self.flow = flow;
        self.flow_stale = false;
    }

    /// The history markers (A7): a page on its way, or the session's
    /// start reached. The marker row is always there, at a fixed height:
    /// its text changing never moves the view (item 4).
    pub fn set_history(&mut self, loading_earlier: bool, start: bool) {
        if (loading_earlier, start) == (self.loading_earlier, self.history_start) {
            return;
        }
        if self.loading_earlier && !loading_earlier {
            // A page landed: the next approach may ask again.
            self.earlier_asked = false;
        }
        self.loading_earlier = loading_earlier;
        self.history_start = start;
        let marker = self.marker();
        if let Some(row) = self.flow.rows.first_mut() {
            row.text = marker.to_string();
        }
        self.settle();
    }

    /// herdr's focus, when it is on another pane than the view's: a dim
    /// "terminal on: <name>" line under the header's name.
    pub fn set_terminal_on(&mut self, name: Option<String>) {
        if name != self.terminal_on {
            self.terminal_on = name;
            self.settle();
        }
    }

    pub fn terminal_on(&self) -> Option<&str> {
        self.terminal_on.as_deref()
    }

    /// Whether earlier sections are to be paged in: the flow's top came
    /// within a screen of the window's top. Once per approach.
    pub fn take_load_earlier(&mut self) -> bool {
        std::mem::take(&mut self.wants_earlier)
    }

    /// Notes an approach to the flow's top (A7's scroll trigger).
    fn check_earlier(&mut self) {
        let screen = self.frame.viewport.h.max(1.0);
        let near_top = self.offset <= screen;
        if !near_top {
            self.earlier_asked = false;
            return;
        }
        if self.earlier_asked || self.loading_earlier || self.history_start || self.slots.is_empty()
        {
            return;
        }
        self.earlier_asked = true;
        self.wants_earlier = true;
    }

    /// The span of the flow's y to lay rows out for: the window and
    /// `KEEP_SCREENS` screens on each side.
    fn keep_span(&self) -> (f32, f32) {
        let screen = self.frame.viewport.h.max(self.layout.h).max(1.0);
        (
            self.offset - KEEP_SCREENS * screen,
            self.offset + screen + KEEP_SCREENS * screen,
        )
    }

    /// Whether the flow is to be put together again: it is stale, or the
    /// window came within a screen of the edge of the rows laid out (the
    /// memory guard).
    fn rows_short(&self) -> bool {
        if self.flow_stale {
            return true;
        }
        let Some((lo, hi)) = self.flow.kept else {
            return true;
        };
        let screen = self.frame.viewport.h.max(self.layout.h).max(1.0);
        let short_above = lo > 0.0 && self.offset - screen < lo;
        let short_below = hi < self.ribbon_height() && self.offset + 2.0 * screen > hi;
        short_above || short_below
    }

    /// The sections from the one at the top of the window on, each with
    /// how far the window's top is from its block's top.
    fn anchors(&self) -> Vec<(u64, f32)> {
        // Above the first block (the marker row), the first.
        let top = self.slot_at(self.offset).unwrap_or(0);
        self.slots
            .iter()
            .zip(&self.tops)
            .skip(top)
            .map(|(slot, &y)| (slot.serial.to_u64(), self.offset - y))
            .collect()
    }

    /// The ribbon's slots, in index order.
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// The section at the top of the window, and how far into its block
    /// the window starts.
    pub fn place(&self) -> Option<(Serial, f32)> {
        let i = self.slot_at(self.offset)?;
        Some((self.slots[i].serial, self.offset - self.tops[i]))
    }

    /// The entries held, in order, as the flow's `entry_y` places them.
    #[cfg(test)]
    fn entries(&self) -> Vec<Entry> {
        self.slots
            .iter()
            .filter_map(|s| match &self.blocks[&s.serial.to_u64()].shows {
                Shows::Entries(e) => Some(e.clone()),
                Shows::Card(_) => None,
            })
            .flatten()
            .collect()
    }

    /// The entry at the top of the window, and how far into it the window
    /// starts.
    #[cfg(test)]
    fn top_entry(&self) -> Option<(Entry, f32)> {
        let i = self.flow.entry_y.iter().rposition(|&y| y <= self.offset)?;
        Some((
            self.entries().get(i)?.clone(),
            self.offset - self.flow.entry_y[i],
        ))
    }

    /// The dictation card, or `None` once the message is sent or cleared.
    /// While it shows, the controls are send, cancel and record.
    pub fn set_dictation(&mut self, lines: Option<Vec<Line>>) {
        if lines == self.dictation {
            return;
        }
        if lines.is_none() {
            self.reply = None;
        }
        self.dictation = lines;
        self.settle();
    }

    /// A message is to be taken in reply to a card: the pinned card when it
    /// asks, else the newest live question.
    pub fn begin_reply(&mut self) {
        let asking = |c: &&AgentCard| c.live && c.kind == CardKind::Question;
        self.reply = self
            .pinned_card()
            .filter(asking)
            .or_else(|| {
                self.slots
                    .iter()
                    .rev()
                    .filter_map(|s| self.blocks.get(&s.serial.to_u64()).and_then(Block::card))
                    .find(asking)
            })
            .cloned();
        self.settle();
    }

    /// The card a message is being taken in reply to.
    pub fn replying_to(&self) -> Option<&AgentCard> {
        self.reply.as_ref().filter(|_| self.dictation.is_some())
    }

    /// The play button's state: bright when on, dull when off.
    pub fn set_read_aloud(&mut self, on: bool) {
        if on != self.read_aloud {
            self.read_aloud = on;
            self.dirty = true;
        }
    }

    pub fn read_aloud(&self) -> bool {
        self.read_aloud
    }

    /// Whether the session is still loading: "(loading session)" until it
    /// answers, then "(no session to show)" if it has nothing.
    pub fn set_loading(&mut self, on: bool) {
        if on != self.loading {
            self.loading = on;
            self.dirty = true;
        }
    }

    /// The response just sent from the view, shown as typed input until
    /// the session has it.
    pub fn set_typed(&mut self, typed: Option<String>) {
        if typed != self.typed {
            self.typed = typed;
            self.settle();
        }
    }

    /// Shows all of the pinned card's text, or folds it to its first lines.
    pub fn toggle_expand(&mut self) {
        self.pin_expanded = !self.pin_expanded;
        self.settle();
    }

    /// Whether the pinned card's text is longer than its first lines.
    fn pin_is_cut(&self) -> bool {
        let Some(card) = self.pinned_card() else {
            return false;
        };
        let s = self.scale();
        let w = self.layout.w - 2.0 * PAD * s;
        let inner_w = w - EDGE * s - 2.0 * CARD_PAD * s;
        wrap_hard(card.text.trim(), columns(inner_w, CARD_PT * s)).len() > PIN_LINES
    }

    /// Whether something is being said; with read aloud on, the play button
    /// shows skip.
    pub fn set_speaking(&mut self, on: bool) {
        if on != self.speaking {
            self.speaking = on;
            self.dirty = true;
        }
    }

    /// Whether a tap on the play button skips what is being said.
    pub fn skips(&self) -> bool {
        self.read_aloud && self.speaking
    }

    pub fn set_layout(&mut self, layout: Layout) {
        if layout == self.layout {
            return;
        }
        self.layout = layout;
        // Every block again only when the width or scale changed.
        self.refresh_blocks();
        self.flow_stale = true;
        self.settle();
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
        self.pinned
            .and_then(|p| self.blocks.get(&p.to_u64()))
            .and_then(Block::card)
    }

    /// The largest offset: the flow's bottom above the full pinned card.
    pub fn max_offset(&self) -> f32 {
        (self.ribbon_height() - self.frame.full_viewport_h).max(0.0)
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
        let max = (self.ribbon_height() - full_h).max(0.0);
        if self.follow {
            self.offset = max;
        }
        let clamped = self.offset.clamp(0.0, max);
        if !self.follow && (clamped - self.offset).abs() > 2.0 {
            log::info!("agent view: clamped {:.0} -> {:.0}", self.offset, clamped);
        }
        self.offset = clamped;
        if self.rows_short() {
            self.assemble();
        }
        self.frame = self.arrange(self.compact());
        self.place_hits();
        self.check_earlier();
        self.dirty = true;
    }

    /// The screen's parts, with the pinned card in full or as the bar.
    fn arrange(&self, compact: bool) -> Frame {
        let l = &self.layout;
        let s = self.scale();
        let pad = PAD * s;
        let terminal_on = if self.terminal_on.is_some() {
            TERMINAL_ON_H * s
        } else {
            0.0
        };
        let header = Rect::new(0.0, 0.0, l.w, l.top + HEADER_H * s + terminal_on);
        // The bottom row controls the audio: play at the left and record at
        // the right, square. While a message is taken, record becomes send
        // and a cancel sits in the middle. Replying to a card, the row is
        // in that card instead, and the screen's is hidden.
        let row_bottom = l.h - l.bottom - 16.0 * s;
        let dictating = self.dictation.is_some();
        let replying = self.reply.as_ref().filter(|_| dictating);
        let buttons = if replying.is_some() {
            Vec::new()
        } else {
            ControlRow::lay_out(pad, l.w - pad, row_bottom, s, dictating)
        };
        let back = Rect::new(0.0, l.top, BACK_W * s, HEADER_H * s);
        let mut above = if replying.is_some() {
            row_bottom
        } else {
            row_bottom - BUTTON_H * s - 12.0 * s
        };
        // The suggestions, just above the buttons.
        let offered = self.suggestions();
        let slot = (l.w - 2.0 * pad) / 4.0;
        let row_top = above + 4.0 * s - SUGGEST_H * s;
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
        // A response just sent, as typed input, until the session has it.
        let typed = self.typed.as_deref().filter(|_| !dictating).map(|t| {
            let flow = InputBox::lay_out(t, pad, l.w - 2.0 * pad, above, s);
            above -= flow.height + 8.0 * s;
            flow
        });
        // The message being taken, pinned above the buttons: on its own
        // card, or in the card it replies to with the controls.
        let dictation = self.dictation.as_ref().map(|lines| {
            let flow = DictationCard::lay_out(lines, replying, pad, l.w - 2.0 * pad, above, s);
            above -= flow.height + 8.0 * s;
            flow
        });
        let mut full_top = above;
        let mut pin = None;
        // While a message is taken, the pinned card makes way for it.
        if let Some(card) = self.pinned_card().filter(|_| !dictating) {
            // Expanded, as many lines as leave the flow 120 pt; else the
            // first PIN_LINES.
            let view_top = header.y + header.h;
            let fit = ((above - view_top - 120.0 * s - 80.0 * s) / line_h(CARD_PT * s)).max(0.0);
            let max_lines = if self.pin_expanded {
                (fit as usize).max(PIN_LINES)
            } else {
                PIN_LINES
            };
            // The full card, laid out once to know its height.
            let mut full = Flow::default();
            let targets = &self.flow.targets;
            let h = card_block(
                card,
                pad,
                0.0,
                l.w - 2.0 * pad,
                s,
                max_lines,
                targets,
                &mut full,
            );
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
                    max_lines,
                    targets,
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
        // The jump line's height is kept below the scrollback whether or
        // not it shows, so it never moves the scrollback (the 00:05 EDT
        // amendment, item 4).
        let jump_h = JUMP_H * s;
        let (view_bottom, full_bottom) = (view_bottom - jump_h, full_bottom - jump_h);
        let jump = (!self.follow).then(|| {
            let px = TOOL_PT * s;
            let text = self.jump_text(columns(l.w - 2.0 * pad, px));
            JumpLine {
                rect: Rect::new(0.0, view_bottom, l.w, jump_h),
                text,
            }
        });
        Frame {
            header,
            viewport: Rect::new(0.0, view_top, l.w, (view_bottom - view_top).max(0.0)),
            full_viewport_h: (full_bottom - view_top).max(0.0),
            pin,
            dictation,
            buttons,
            back,
            suggestions,
            typed,
            jump,
        }
    }

    /// The jump line's text (the 00:05 EDT amendment, item 2), at most
    /// `cols` characters: the newest section's first line, after how many
    /// sections arrived since the view left the bottom; "jump to end" when
    /// none did.
    fn jump_text(&self, cols: usize) -> String {
        let new = self.left_at.map_or(0, |at| {
            self.slots
                .iter()
                .rev()
                .take_while(|s| s.serial.to_u64() > at)
                .count()
        });
        let message = self
            .slots
            .last()
            .and_then(|s| self.blocks.get(&s.serial.to_u64()))
            .and_then(|b| first_line(&b.shows));
        let Some(message) = message.filter(|_| new > 0) else {
            return JUMP_TO_END.to_string();
        };
        let lead = format!("-- {new} new: ");
        let room = cols.saturating_sub(lead.chars().count() + 3).max(1);
        format!("{lead}{} --", one_line(&message, room))
    }

    /// Jumps to the bottom and follows again: the jump line's tap.
    pub fn jump_to_end(&mut self) {
        self.fling = 0.0;
        let bottom = self.max_offset();
        self.scroll_to(bottom);
    }

    /// The suggested responses for the row, by the pinned card and the
    /// status. Those that belong to a card are in it: a question's yes and
    /// no, an approval's own buttons. None while working or dictating.
    pub fn suggestions(&self) -> Vec<Suggestion> {
        if self.dictation.is_some() || self.status == "working" {
            return Vec::new();
        }
        // Continue after something broke at once; after a finished turn,
        // or with no card, only once the agent has been quiet a while
        // (owner, 2026-10-23: "it should probably also show after an idle
        // period has passed, such as 30s").
        match self.pinned_card().filter(|c| c.live).map(|c| c.kind) {
            Some(CardKind::Broke) => vec![Suggestion::Continue],
            Some(CardKind::Finished) | None if self.quiet_for >= QUIET_FOR_CONTINUE => {
                vec![Suggestion::Continue]
            }
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
        if let Some(d) = &self.frame.dictation {
            for b in &d.buttons {
                self.hits.push((b.hit, b.rect));
            }
        }
        for g in &self.frame.suggestions {
            self.hits.push((g.hit, g.rect));
        }
        if let Some(j) = &self.frame.jump {
            self.hits.push((AgentHit::JumpToEnd, j.rect));
        }
        if let Some(p) = &self.frame.pin {
            if p.compact {
                self.hits.push((AgentHit::Pin, p.rect));
            } else {
                for b in &p.flow.buttons {
                    self.hits.push((b.hit, b.rect));
                }
                for span in &p.flow.links {
                    let rect = link_spans::rect(&p.flow.rows[span.row], span, 0.0);
                    self.hits.push((AgentHit::Link(span.link), rect));
                }
                // The rest of the card, when its text is cut or shown all.
                if self.pin_expanded || self.pin_is_cut() {
                    self.hits.push((AgentHit::Expand, p.rect));
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
        for span in &self.flow.links {
            let rect = link_spans::rect(&self.flow.rows[span.row], span, v.y - self.offset);
            if rect.y + rect.h * 0.5 >= v.y && rect.y + rect.h * 0.5 <= v.y + v.h {
                self.hits.push((AgentHit::Link(span.link), rect));
            }
        }
    }

    /// The link a [`AgentHit::Link`] names.
    pub fn link(&self, index: usize) -> Option<&Link> {
        self.flow.targets.get(index)
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
        let follow = self.offset >= max - 1.0;
        if self.follow && !follow {
            // Leaving the bottom: what arrives from now on is new.
            self.left_at = self.slots.last().map(|s| s.serial.to_u64());
        }
        self.follow = follow;
        if self.rows_short() {
            self.assemble();
        }
        self.frame = self.arrange(self.compact());
        self.place_hits();
        self.check_earlier();
        self.dirty = true;
    }

    /// Jumps to the pinned card in the flow: its top a third of the way
    /// down the window.
    pub fn jump_to_pin(&mut self) {
        if self.pinned.is_none() {
            return;
        }
        // The pinned card is shown only at the bottom: go back there.
        self.fling = 0.0;
        let bottom = self.max_offset();
        self.scroll_to(bottom);
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
                let carried = std::mem::take(&mut self.fling);
                self.drag = Some(Drag {
                    carried,
                    down: now,
                    id,
                    start: [x, y],
                    last_y: y,
                    last_t: now,
                    moved: false,
                    samples: [(now, y); SAMPLES],
                    sampled: 1,
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
                d.sample(now, y);
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
                    let mut velocity = fling_curve(release_speed(&d, s), s);
                    let quick = now.saturating_sub(d.down) <= FLING_BOOST_TOUCH;
                    if quick && velocity * d.carried > 0.0 {
                        velocity += d.carried;
                    }
                    let velocity = velocity.clamp(-MAX_FLING_OUT * s, MAX_FLING_OUT * s);
                    if !still && velocity.abs() > MIN_FLING * s {
                        self.fling = velocity;
                    }
                    log::info!(
                        "agent view: drag ended at {:.0}, fling {:.0}",
                        self.offset,
                        self.fling
                    );
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
        let was = self.quiet_for >= QUIET_FOR_CONTINUE;
        self.quiet_for += dt.as_secs_f32();
        if !was && self.quiet_for >= QUIET_FOR_CONTINUE {
            // The suggestion row may appear.
            self.settle();
        }
        if self.fling == 0.0 || self.drag.is_some() {
            return false;
        }
        // A slow frame cannot throw the view (the 23:54 EDT amendment,
        // item 6).
        let dt = dt.as_secs_f32().min(FLING_DT_MAX);
        let before = self.offset;
        self.scroll_to(self.offset - self.fling * dt);
        self.fling *= (-DECAY * dt).exp();
        let stopped = self.offset == before
            || self.fling.abs() < MIN_FLING * self.scale()
            || self.offset <= 0.0
            || self.follow;
        if stopped {
            self.fling = 0.0;
            log::info!("agent view: fling stopped at {:.0}", self.offset);
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
        let recording = self.dictation.is_some();
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
        for (i, r) in self.flow.rows.iter().enumerate() {
            let h = if r.mono { term_h(r.px) } else { line_h(r.px) };
            if visible(r.y, h) {
                let spans = link_spans::of_row(&self.flow, i);
                link_spans::draw(list, ctx, term, r, &spans, [r.x, r.y + dy]);
            }
        }
        for b in &self.flow.buttons {
            if visible(b.rect.y, b.rect.h) {
                let rect = Rect::new(b.rect.x, b.rect.y + dy, b.rect.w, b.rect.h);
                SquareButton::draw(
                    list,
                    ctx,
                    term,
                    b,
                    rect,
                    s,
                    self.read_aloud,
                    self.speaking,
                    recording,
                );
            }
        }
        if self.slots.is_empty() {
            let px = TEXT_PT * s;
            ctx.draw_text(
                list,
                if self.loading {
                    "(loading session)"
                } else {
                    "(no session to show)"
                },
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
        if let Some(on) = &self.terminal_on {
            // The view does not follow herdr's focus: say where it is.
            ctx.draw_text(
                list,
                &format!("terminal on: {on}"),
                [name_x, l.top + HEADER_H * s],
                TOOL_PT * s,
                DIM,
            );
        }

        // The pinned card, the dictation card, then the buttons.
        if let Some(t) = &self.frame.typed {
            for f in &t.fills {
                fill(list, f.rect, f.color, f.radius);
            }
            for r in &t.rows {
                draw_row(list, ctx, term, r, [r.x, r.y]);
            }
        }
        if let Some(d) = &self.frame.dictation {
            for f in &d.fills {
                fill(list, f.rect, f.color, f.radius);
            }
            for r in &d.rows {
                ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
            }
            for b in &d.buttons {
                SquareButton::draw(
                    list,
                    ctx,
                    term,
                    b,
                    b.rect,
                    s,
                    self.read_aloud,
                    self.speaking,
                    recording,
                );
            }
        }
        if let Some(p) = &self.frame.pin {
            for f in &p.flow.fills {
                fill(list, f.rect, f.color, f.radius);
            }
            for (i, r) in p.flow.rows.iter().enumerate() {
                let spans = link_spans::of_row(&p.flow, i);
                link_spans::draw(list, ctx, term, r, &spans, [r.x, r.y]);
            }
            for b in &p.flow.buttons {
                SquareButton::draw(
                    list,
                    ctx,
                    term,
                    b,
                    b.rect,
                    s,
                    self.read_aloud,
                    self.speaking,
                    recording,
                );
            }
        }
        if let Some(j) = &self.frame.jump {
            let px = TOOL_PT * s;
            // Centred, at Roboto's half an em a character.
            let w = j.text.chars().count() as f32 * px * 0.5;
            let x = j.rect.x + ((j.rect.w - w) * 0.5).max(PAD * s);
            let y = j.rect.y + (j.rect.h - line_h(px)) * 0.5;
            ctx.draw_text(list, &j.text, [x, y], px, DIM);
        }
        let sg_px = SUGGEST_PT * s;
        for g in &self.frame.suggestions {
            let y = g.rect.y + (g.rect.h - line_h(sg_px)) * 0.5;
            ctx.draw_text(list, g.label, [g.rect.x + 4.0 * s, y], sg_px, g.color);
        }
        for b in &self.frame.buttons {
            SquareButton::draw(
                list,
                ctx,
                term,
                b,
                b.rect,
                s,
                self.read_aloud,
                self.speaking,
                recording,
            );
        }
        self.dirty = false;
        true
    }

    /// Marks the screen to be drawn again.
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
}

pub(crate) fn fill(list: &mut DrawList, rect: Rect, color: [f32; 4], radius: f32) {
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

pub(crate) fn button(
    list: &mut DrawList,
    ctx: &DrawContext,
    label: &str,
    rect: Rect,
    bg: [f32; 4],
    s: f32,
) {
    // The label never outgrows its button: 17 pt, or smaller to fit (a
    // Roboto letter is about 0.55 em wide; 12 pt of padding each side).
    let chars = label.chars().count().max(1) as f32;
    let fits = (rect.w - 24.0 * s) / (0.55 * chars);
    let mut b = Button::new(label, rect)
        .with_bg(bg)
        .with_text_color(FG)
        .with_font_size((17.0 * s).min(fits).max(9.0 * s));
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
            links: Vec::new(),
            origin: None,
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
            about_plan: false,
            links: Vec::new(),
        }
    }

    fn screen(entries: Vec<Entry>, cards: Vec<AgentCard>, pinned: Option<usize>) -> AgentScreen {
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        flat(&mut a, "idle", 0, entries, cards, pinned);
        a
    }

    const FILE: u32 = 0x3fa9_c21e;

    /// The serial of the entry `i` of a test's whole session: on its own
    /// line, the even ones; a card before entry `i` is on the odd line
    /// before it.
    fn entry_serial(i: usize) -> Serial {
        Serial::new(FILE, 2 * i as u32 + 2, 0)
    }

    /// The serials of `sections` that differ from what `a` holds, and those
    /// it holds that `sections` does not have: what the client marks.
    fn changed(a: &AgentScreen, sections: &[ViewSection]) -> Vec<Serial> {
        let mut out: Vec<Serial> = sections
            .iter()
            .filter(|v| a.blocks.get(&v.serial.to_u64()).map(|b| &b.shows) != Some(&v.shows))
            .map(|v| v.serial)
            .collect();
        for slot in &a.slots {
            if !sections.iter().any(|v| v.serial == slot.serial) {
                out.push(slot.serial);
            }
        }
        out.sort_by_key(|s| s.to_u64());
        out
    }

    /// Gives `a` these sections, indexed in order, with what changed.
    fn give(
        a: &mut AgentScreen,
        status: &str,
        mut sections: Vec<ViewSection>,
        pinned: Option<Serial>,
    ) {
        sections.sort_by_key(|v| v.serial.to_u64());
        for (i, v) in sections.iter_mut().enumerate() {
            v.index = i as u64;
        }
        let changed = changed(a, &sections);
        a.set_content("Claude".into(), status.into(), sections, pinned, &changed);
    }

    /// Gives `a` the entries and cards of a flattened view, as the
    /// client's sections: each entry a section of its own, `base` its
    /// place in the whole session, and each card a section before the
    /// entry it is `at`.
    fn flat(
        a: &mut AgentScreen,
        status: &str,
        base: usize,
        entries: Vec<Entry>,
        cards: Vec<AgentCard>,
        pinned: Option<usize>,
    ) {
        let mut sections = Vec::new();
        let mut pin = None;
        for (n, card) in cards.into_iter().enumerate() {
            let serial = Serial::new(FILE, 2 * (base + card.at) as u32 + 1, n as u8);
            if pinned == Some(n) {
                pin = Some(serial);
            }
            sections.push(ViewSection {
                index: 0,
                serial,
                shows: Shows::Card(AgentCard { at: 0, ..card }),
            });
        }
        for (i, e) in entries.into_iter().enumerate() {
            sections.push(ViewSection {
                index: 0,
                serial: entry_serial(base + i),
                shows: Shows::Entries(vec![e]),
            });
        }
        give(a, status, sections, pin);
    }

    const PLAN: &str = "docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md";

    fn plan_link() -> Link {
        Link {
            text: PLAN.to_string(),
            url: "http://w/#/src--handlr/docs/design/20261023_04_OVERVIEW_SCREEN_PLAN".to_string(),
        }
    }

    /// A live question about the plan, at `at`, with its link.
    fn plan_card(at: usize) -> AgentCard {
        AgentCard {
            about_plan: true,
            text: format!("Shall I build {PLAN}?"),
            links: vec![plan_link()],
            ..card(CardKind::Question, at, true)
        }
    }

    #[test]
    fn a_plan_question_offers_approve_discuss_and_implement() {
        let entries = conversation(1);
        let flow = lay_out_flow(&entries, &[plan_card(4)], None, 1080.0, 2.0);
        let buttons: Vec<(AgentHit, &str)> =
            flow.buttons.iter().map(|b| (b.hit, b.label)).collect();
        assert_eq!(
            buttons,
            [
                (AgentHit::Suggest(Suggestion::Approve), "approve"),
                (AgentHit::Suggest(Suggestion::Discuss), "discuss"),
                (AgentHit::Suggest(Suggestion::Implement), "implement"),
                (AgentHit::Reply, ""),
            ]
        );
        // Pinned, the same buttons, and they can be tapped.
        let a = screen(entries, vec![plan_card(4)], Some(0));
        for hit in [
            AgentHit::Suggest(Suggestion::Approve),
            AgentHit::Suggest(Suggestion::Discuss),
            AgentHit::Suggest(Suggestion::Implement),
        ] {
            let r = a.rect_of(hit).expect("on the pinned card");
            assert_eq!(a.hit(r.x + r.w * 0.5, r.y + r.h * 0.5), Some(hit));
        }
    }

    #[test]
    fn a_link_in_the_scrollback_is_marked_and_tapped() {
        let mut entries = conversation(1);
        entries.push(Entry {
            summary: format!("Wrote {PLAN}; see https://example.com/x."),
            links: vec![
                plan_link(),
                Link {
                    text: "https://example.com/x".to_string(),
                    url: "https://example.com/x".to_string(),
                },
            ],
            ..entry(Role::Assistant, EntryKind::Text, "linked")
        });
        let a = screen(entries, vec![], None);
        let flow = a.flow();
        assert_eq!(flow.targets.len(), 2);
        // Each link's text is spanned, the path perhaps over two rows.
        let spanned = |link: usize| -> String {
            flow.links
                .iter()
                .filter(|s| s.link == link)
                .map(|s| {
                    flow.rows[s.row]
                        .text
                        .chars()
                        .skip(s.start)
                        .take(s.end - s.start)
                        .collect::<String>()
                })
                .collect()
        };
        assert_eq!(spanned(0), PLAN);
        assert_eq!(spanned(1), "https://example.com/x");
        // A tap on it is the link.
        let span = flow.links.iter().find(|s| s.link == 1).unwrap();
        let row = &flow.rows[span.row];
        let v = a.frame().viewport;
        let r = link_spans::rect(row, span, v.y - a.offset());
        let hit = a.hit(r.x + r.w * 0.5, r.y + r.h * 0.5);
        assert_eq!(hit, Some(AgentHit::Link(1)));
        assert_eq!(a.link(1).unwrap().url, "https://example.com/x");
        // Text with no link is not one.
        assert!(
            flow.rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.text.starts_with("question"))
                .all(|(i, _)| link_spans::of_row(flow, i).is_empty())
        );
    }

    #[test]
    fn a_link_on_the_pinned_card_is_tapped_before_the_card() {
        let a = screen(conversation(1), vec![plan_card(4)], Some(0));
        let pin = a.frame().pin.as_ref().expect("pinned");
        let span = pin.flow.links.first().expect("the plan path is spanned");
        let r = link_spans::rect(&pin.flow.rows[span.row], span, 0.0);
        assert_eq!(
            a.hit(r.x + r.w * 0.5, r.y + r.h * 0.5),
            Some(AgentHit::Link(span.link))
        );
        assert_eq!(a.link(span.link), Some(&plan_link()));
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
    fn a_context_entry_is_a_dim_line_after_an_amber_dot_and_makes_no_card() {
        let notice = |origin: &str, text: &str| Entry {
            origin: Some(origin.to_string()),
            ..entry(Role::User, EntryKind::Text, text)
        };
        let entries = vec![
            entry(Role::User, EntryKind::Text, "please fix it"),
            entry(Role::Assistant, EntryKind::Text, "Fixed."),
            notice("task", "Build finished"),
            notice("peer:docs-writer", "Rebased on main.\nNothing else."),
            notice("agent:a0123", &"A long report that goes on. ".repeat(40)),
            notice("recap", "You asked for a fix; it is done."),
            notice("compact", "Conversation compacted"),
            notice("compact", "Conversation compacted (ctrl+o for history)"),
        ];
        let flow = lay_out_flow(&entries, &[], None, 1080.0, 2.0);
        assert_eq!(flow.entry_y.len(), entries.len());
        // Only the user's own words are on the band.
        let bands = flow.fills.iter().filter(|f| f.color == BAND).count();
        assert_eq!(bands, 1);
        let row = |prefix: &str| -> Vec<&Row> {
            flow.rows
                .iter()
                .filter(|r| r.text.starts_with(prefix))
                .collect()
        };
        for text in [
            "task finished: Build finished",
            "docs-writer: Rebased on main.",
            "\u{203B} You asked for a fix; it is done.",
        ] {
            let rows = row(text);
            assert_eq!(rows.len(), 1, "{text}: {:?}", flow.rows);
            assert_eq!(rows[0].color, DIM);
            assert_eq!(rows[0].px, TOOL_PT * 2.0);
            // The dot before it, amber, on the same line.
            let dot = flow
                .rows
                .iter()
                .find(|r| r.text == notice_line::DOT && r.y == rows[0].y)
                .expect("a dot");
            assert_eq!(dot.color, input_box::AMBER);
            assert!(dot.x < rows[0].x);
        }
        assert!(row("Nothing else").is_empty(), "first line only");
        // Two lines at most, the second cut.
        let report: Vec<&Row> = flow
            .rows
            .iter()
            .filter(|r| r.text.contains("long report") || r.text.contains("goes on"))
            .collect();
        assert_eq!(report.len(), 2);
        assert!(report[0].text.starts_with("subagent: "));
        assert!(report[1].text.ends_with('\u{2026}'));
        // A compaction is shown once.
        assert_eq!(row("\u{203B} conversation compacted").len(), 1);
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
        let flow = lay_out_flow(&entries, &[], None, 1080.0, 2.0);
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
        let flow = lay_out_flow(&entries, &cards, None, 1080.0, 2.0);
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

        let asking = lay_out_flow(
            &entries,
            &[card(CardKind::Question, 4, true)],
            None,
            1080.0,
            2.0,
        );
        let hits: Vec<AgentHit> = asking.buttons.iter().map(|b| b.hit).collect();
        assert_eq!(
            hits,
            [
                AgentHit::Suggest(Suggestion::Yes),
                AgentHit::Suggest(Suggestion::No),
                AgentHit::Reply
            ]
        );
        let finished = lay_out_flow(
            &entries,
            &[card(CardKind::Finished, 4, true)],
            None,
            1080.0,
            2.0,
        );
        assert!(finished.buttons.is_empty());
    }

    #[test]
    fn it_follows_the_bottom_until_scrolled_back() {
        let mut a = screen(conversation(20), vec![], None);
        assert!(a.following());
        assert!(a.max_offset() > 0.0);
        assert_eq!(a.offset(), a.max_offset());
        // New entries: still at the bottom.
        flat(&mut a, "idle", 0, conversation(21), vec![], None);
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
        flat(&mut a, "idle", 0, conversation(22), vec![], None);
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
        flat(&mut a, "idle", 4, all[4..104].to_vec(), vec![], None);
        let (now_top, now_delta) = a.top_entry().unwrap();
        assert_eq!(now_top, top);
        assert!((now_delta - delta).abs() < 0.5);
    }

    #[test]
    fn moves_arriving_together_do_not_throw_the_view() {
        // The S23's "snap back": moves microseconds apart gave flings of
        // millions of pixels a second, which ran the view to the top.
        let mut a = screen(conversation(40), vec![], None);
        let t = drag(&mut a, 800.0, 1100.0, 5, Duration::ZERO);
        let us = Duration::from_micros(1);
        a.touch(1, TouchPhase::Moved, 540.0, 1140.0, t + us);
        a.touch(1, TouchPhase::Ended, 540.0, 1140.0, t + 2 * us);
        let s = a.scale();
        assert!(a.fling.abs() <= MAX_FLING_OUT * s, "{}", a.fling);
        assert!(a.fling > 0.0, "still a fling, the finger's own speed");
        let lifted = a.offset();
        for _ in 0..200 {
            a.tick(MS);
        }
        let far = MAX_FLING_OUT * s / DECAY;
        assert!(
            lifted - a.offset() <= far + 1.0,
            "{} > {far}",
            lifted - a.offset()
        );
        // A drag too short to measure gives no fling.
        let mut b = screen(conversation(40), vec![], None);
        b.touch(1, TouchPhase::Started, 540.0, 800.0, Duration::ZERO);
        b.touch(1, TouchPhase::Moved, 540.0, 900.0, us);
        b.touch(1, TouchPhase::Ended, 540.0, 900.0, 2 * us);
        assert_eq!(b.fling, 0.0);
    }

    #[test]
    fn a_harder_flick_goes_disproportionately_further() {
        let s = 2.5;
        assert_eq!(
            fling_curve(500.0 * s, s),
            500.0 * s,
            "gentle: as the finger"
        );
        assert_eq!(fling_curve(-500.0 * s, s), -500.0 * s);
        let (two, four) = (fling_curve(2000.0 * s, s), fling_curve(4000.0 * s, s));
        assert!(
            four > 2.5 * two,
            "twice the speed, more than twice: {two} {four}"
        );
        assert!(fling_curve(-4000.0 * s, s) == -four);
    }

    #[test]
    fn the_release_speed_is_the_fingers_at_release() {
        // A flick that speeds up: its speed at release is above its
        // average over the window.
        let mut a = screen(conversation(40), vec![], None);
        a.touch(1, TouchPhase::Started, 540.0, 800.0, Duration::ZERO);
        let mut y = 800.0;
        let mut t = Duration::ZERO;
        for i in 1..=6 {
            t += Duration::from_millis(16);
            y += 10.0 * i as f32;
            a.touch(1, TouchPhase::Moved, 540.0, y, t);
        }
        a.touch(1, TouchPhase::Ended, 540.0, y, t + Duration::from_millis(1));
        let average = (y - 800.0) / t.as_secs_f32();
        assert!(a.fling > average, "{} <= {average}", a.fling);
    }

    #[test]
    fn a_quick_flick_the_same_way_adds_to_a_running_fling() {
        let flick = |a: &mut AgentScreen, from: f32, to: f32, t: Duration| {
            let end = drag(a, from, to, 4, t);
            a.touch(
                1,
                TouchPhase::Ended,
                540.0,
                to,
                end + Duration::from_millis(1),
            );
            end
        };
        let mut a = screen(conversation(80), vec![], None);
        let t = flick(&mut a, 800.0, 1000.0, Duration::ZERO);
        let first = a.fling;
        a.tick(MS);
        let running = a.fling;
        flick(&mut a, 800.0, 1000.0, t + Duration::from_millis(50));
        assert!(a.fling > first, "{} <= {first}", a.fling);
        assert!(a.fling > running);
        // The other way: no boost, just the new flick.
        let mut b = screen(conversation(80), vec![], None);
        let t = flick(&mut b, 800.0, 1000.0, Duration::ZERO);
        b.tick(MS);
        flick(&mut b, 1000.0, 800.0, t + Duration::from_millis(50));
        assert!(
            b.fling < 0.0 && b.fling.abs() <= first * 1.01,
            "{}",
            b.fling
        );
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

        // Tapping the bar goes back to the bottom, where the card is.
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
        assert!(a.following());
        assert!(!a.frame().pin.as_ref().unwrap().compact, "in full");
        // Shown once: the flow has no panel for it.
        assert!(
            !a.flow()
                .fills
                .iter()
                .any(|f| f.color == accent(CardKind::Approval)),
            "not also in the flow"
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
        flat(&mut a, "idle", 0, entries.clone(), cards, Some(0));
        assert_eq!(a.offset(), back);
        assert!(a.frame().pin.as_ref().unwrap().compact);
        // It resolves and leaves the pin: still there.
        let resolved = vec![card(CardKind::Question, n, false)];
        flat(&mut a, "idle", 0, entries, resolved, None);
        assert_eq!(a.offset(), back);
        assert!(a.frame().pin.is_none());
    }

    #[test]
    fn flow_buttons_are_hit_only_in_the_window() {
        let entries = conversation(3);
        let cards = vec![card(CardKind::Question, entries.len(), true)];
        // Not pinned, so its only button is the one in the flow.
        let a = screen(entries, cards, None);
        let take = |hit| a.hits.iter().filter(|(h, _)| *h == hit).count();
        assert_eq!(
            (take(AgentHit::Message), take(AgentHit::Reply)),
            (1, 1),
            "the screen's record and the card's"
        );
        let v = a.frame().viewport;
        for (hit, rect) in &a.hits {
            if *hit == AgentHit::Reply && rect.y < v.y + v.h {
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
            [AgentHit::ReadAloud, AgentHit::Clear, AgentHit::Submit]
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
        flat(&mut a, "idle", 0, conversation(3), vec![], None);
        assert!(!a.draw(&mut list, &ctx, None), "the same content");
        flat(&mut a, "working", 0, conversation(3), vec![], None);
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
        assert_eq!(f.buttons.len(), 2);
        // Play at the bottom left, the message button at the bottom right.
        let play = f.buttons[0].rect;
        assert_eq!(f.buttons[0].hit, AgentHit::ReadAloud);
        assert!((play.x - PAD * PHONE.scale).abs() < 0.5);
        let mic = f.buttons[1].rect;
        assert_eq!(f.buttons[1].hit, AgentHit::Message);
        assert!((mic.y - play.y).abs() < 0.5);
        assert!((mic.x + mic.w - (PHONE.w - PAD * PHONE.scale)).abs() < 0.5);
        assert!(mic.y + mic.h <= PHONE.h - PHONE.bottom);
    }

    #[test]
    fn suggestions_follow_the_pinned_card() {
        // Finished: no cont at first; after 30 s of quiet, cont, in yellow,
        // above the buttons.
        let mut a = screen(
            conversation(3),
            vec![card(CardKind::Finished, 3, true)],
            Some(0),
        );
        assert!(a.frame().suggestions.is_empty(), "not right after a turn");
        a.tick(Duration::from_secs(29));
        assert!(a.frame().suggestions.is_empty());
        a.tick(Duration::from_secs(1));
        let row = a.frame().suggestions.clone();
        assert_eq!(row.len(), 1);
        assert_eq!(row[0].hit, AgentHit::Suggest(Suggestion::Continue));
        assert_eq!((row[0].label, row[0].color), ("cont", YELLOW));
        assert!(row[0].rect.y + row[0].rect.h <= a.frame().buttons[0].rect.y);
        // The play button redraws when it changes, and keeps its place.
        a.draw(&mut DrawList::new(), &DrawContext::default(), None);
        a.set_read_aloud(true);
        assert!(a.draw(&mut DrawList::new(), &DrawContext::default(), None));
        assert!(a.read_aloud());
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
        for (name, glyph) in [
            ("back", BACK_GLYPH),
            ("play", PLAY_GLYPH),
            ("send", SEND_GLYPH),
            ("skip", SKIP_GLYPH),
            ("prompt", "\u{203A}"),
            ("cancel", CANCEL_GLYPH),
            ("notice dot", notice_line::DOT),
            ("notice mark", notice_line::MARK),
        ] {
            let line = shaper.line(glyph, GlyphStyle::Regular);
            let found = line
                .segments
                .iter()
                .flat_map(|g| &g.run.glyphs)
                .all(|g| g.glyph_id != 0);
            assert!(
                found && !line.segments.is_empty(),
                "{name} {glyph} has a glyph"
            );
        }
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
        let second = a.entries()[1].clone();
        // The window moves on: the top entry leaves, and the next one is at
        // the top, not the old offset into the new content.
        flat(&mut a, "idle", 1, all[1..101].to_vec(), vec![], None);
        assert!(!a.following());
        assert_eq!((a.entries()[0].clone(), a.offset()), (second, 0.0));
        // In the middle, entries leaving above keep the view on the same
        // entry, at the same place, even when its summary changes (a
        // result linked to its call).
        a.scroll_to(a.flow.entry_y[20] + 7.0);
        let (top, delta) = a.top_entry().unwrap();
        let mut changed = all[5..105].to_vec();
        for e in changed.iter_mut() {
            e.summary.push_str(" (linked)");
        }
        flat(&mut a, "idle", 5, changed, vec![], None);
        let (now_top, now_delta) = a.top_entry().unwrap();
        assert_eq!(now_top.ts, top.ts);
        assert!((now_delta - delta).abs() < 0.5);
        assert!(!a.following());
    }

    fn said(text: &str) -> Vec<Line> {
        vec![Line {
            text: text.into(),
            size: 20.0,
            color: FG,
            gap: 0.0,
        }]
    }

    #[test]
    fn taking_a_message_turns_play_into_send_with_cancel_between() {
        let mut a = screen(
            conversation(3),
            vec![card(CardKind::Finished, 12, true)],
            Some(0),
        );
        assert!(a.frame().pin.is_some());
        a.set_dictation(Some(said("please go on")));
        let f = a.frame().clone();
        let hits: Vec<AgentHit> = f.buttons.iter().map(|b| b.hit).collect();
        assert_eq!(
            hits,
            [AgentHit::ReadAloud, AgentHit::Clear, AgentHit::Submit]
        );
        // Square: play, cancel in the middle, send in record's place.
        for b in &f.buttons {
            assert!((b.rect.w - b.rect.h).abs() < 0.5);
        }
        let mid = f.buttons[1].rect;
        assert!((mid.x + mid.w * 0.5 - PHONE.w * 0.5).abs() < 0.5);
        // The pinned card makes way; the flow ends above the message.
        assert!(f.pin.is_none());
        let card = f.dictation.expect("the message card");
        assert!(f.viewport.y + f.viewport.h <= card.fills[0].rect.y);
        assert!(card.buttons.is_empty());
    }

    #[test]
    fn replying_to_a_card_puts_the_controls_in_it() {
        let mut a = screen(
            conversation(3),
            vec![card(CardKind::Question, 12, true)],
            Some(0),
        );
        a.begin_reply();
        a.set_dictation(Some(said("yes, the second one")));
        let f = a.frame().clone();
        assert!(f.buttons.is_empty(), "the screen's row is hidden");
        let card = f.dictation.expect("the card replied to");
        assert!(card.rows.iter().any(|r| r.text.ends_with("is asking")));
        assert!(card.rows.iter().any(|r| r.text == "yes, the second one"));
        let hits: Vec<AgentHit> = card.buttons.iter().map(|b| b.hit).collect();
        assert_eq!(
            hits,
            [AgentHit::ReadAloud, AgentHit::Clear, AgentHit::Submit]
        );
        let b = card.buttons[2].rect;
        assert_eq!(a.hit(b.x + 5.0, b.y + 5.0), Some(AgentHit::Submit));
        assert!(a.replying_to().is_some());
        // Sent or cancelled: back to play and record.
        a.set_dictation(None);
        assert!(a.replying_to().is_none());
        let hits: Vec<AgentHit> = a.frame().buttons.iter().map(|b| b.hit).collect();
        assert_eq!(hits, [AgentHit::ReadAloud, AgentHit::Message]);
    }

    #[test]
    fn play_skips_while_a_reply_is_read() {
        let mut a = screen(conversation(3), vec![], None);
        a.set_speaking(true);
        assert!(!a.skips(), "read aloud is off: a tap turns it on");
        a.set_read_aloud(true);
        assert!(a.skips());
        a.draw(&mut DrawList::new(), &DrawContext::default(), None);
        a.set_speaking(false);
        assert!(!a.skips());
        assert!(
            a.draw(&mut DrawList::new(), &DrawContext::default(), None),
            "the glyph changes back"
        );
    }

    #[test]
    fn a_sent_response_shows_as_typed_amber_input_until_the_session_has_it() {
        let mut a = screen(conversation(3), vec![], None);
        assert!(a.frame().typed.is_none());
        a.set_typed(Some("Yes, install it on the S23".into()));
        let f = a.frame().clone();
        let typed = f.typed.expect("the input box");
        let row = &typed.rows[0];
        assert_eq!(row.text, "\u{203A} Yes, install it on the S23");
        assert_eq!(row.color, input_box::AMBER);
        assert!(row.mono, "on the terminal grid");
        // Above the controls; the flow ends above it.
        assert!(typed.fills[0].rect.y + typed.height <= f.buttons[0].rect.y);
        assert!(f.viewport.y + f.viewport.h <= typed.fills[0].rect.y);
        a.set_typed(None);
        assert!(a.frame().typed.is_none());
    }

    #[test]
    fn a_cut_question_expands_when_tapped_and_folds_again() {
        let long = "Should I go on with the next step, which moves the readers onto the record trait, \
                    changes every test fixture's loader, and then rebuilds the daemon and the app \
                    before the check on the phone, or stop here so you can look first?";
        let mut asked = card(CardKind::Question, 12, true);
        asked.text = long.to_string();
        let mut a = screen(conversation(3), vec![asked], Some(0));
        let text_rows = |a: &AgentScreen| {
            a.frame()
                .pin
                .as_ref()
                .unwrap()
                .flow
                .rows
                .iter()
                .filter(|r| (r.px - CARD_PT * PHONE.scale).abs() < 0.01)
                .count()
        };
        assert_eq!(text_rows(&a), PIN_LINES, "cut to its first lines");
        // A tap on the card's text, not its buttons, expands it.
        let pin = a.frame().pin.as_ref().unwrap().rect;
        assert_eq!(a.hit(pin.x + 40.0, pin.y + 40.0), Some(AgentHit::Expand));
        a.toggle_expand();
        assert!(text_rows(&a) > PIN_LINES, "all of it");
        let taller = a.frame().pin.as_ref().unwrap().rect;
        assert!(taller.h > pin.h);
        // The buttons still answer.
        let yes = a.frame().pin.as_ref().unwrap().flow.buttons[0].clone();
        assert_eq!(a.hit(yes.rect.x + 5.0, yes.rect.y + 5.0), Some(yes.hit));
        a.toggle_expand();
        assert_eq!(text_rows(&a), PIN_LINES);
        // A short question has nothing to expand.
        let a = screen(
            conversation(3),
            vec![card(CardKind::Question, 12, true)],
            Some(0),
        );
        let pin = a.frame().pin.as_ref().unwrap().rect;
        assert_ne!(a.hit(pin.x + 40.0, pin.y + 40.0), Some(AgentHit::Expand));
    }

    #[test]
    fn a_label_never_outgrows_its_button() {
        // A long label in a narrow button: drawn smaller, not spilling.
        let mut list = DrawList::new();
        let ctx = DrawContext::default();
        button(
            &mut list,
            &ctx,
            "Implement",
            Rect::new(0.0, 0.0, 80.0, 40.0),
            FG,
            1.0,
        );
        button(
            &mut list,
            &ctx,
            "Yes",
            Rect::new(0.0, 0.0, 200.0, 40.0),
            FG,
            1.0,
        );
        // The size rule itself: 17 pt when it fits, smaller when not.
        let size = |label: &str, w: f32| {
            (17.0f32)
                .min((w - 24.0) / (0.55 * label.len() as f32))
                .max(9.0)
        };
        assert!(size("Implement", 80.0) < 17.0);
        assert_eq!(size("Yes", 200.0), 17.0);
    }

    // Plan 20261023_03, A7 and the 22:55 EDT amendment.

    /// Scrolled back by a drag from 600 to 1800 points, and let go.
    fn scrolled_back(a: &mut AgentScreen) {
        let t = drag(a, 600.0, 1800.0, 10, Duration::ZERO);
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1800.0,
            t + Duration::from_millis(200),
        );
        assert!(!a.following());
    }

    #[test]
    fn a_page_prepended_keeps_the_entry_under_the_thumb_in_place() {
        let all = conversation(40);
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        flat(&mut a, "idle", 80, all[80..].to_vec(), vec![], None);
        scrolled_back(&mut a);
        let (top, delta) = a.top_entry().unwrap();
        let place = |a: &AgentScreen, e: &Entry| {
            let i = a.entries().iter().position(|x| x == e).unwrap();
            a.flow().entry_y[i] - a.offset()
        };
        let before = place(&a, &top);
        // A history page lands above: forty older entries, and the marker
        // row comes and goes.
        a.set_history(true, false);
        assert!((place(&a, &top) - before).abs() < 0.5);
        flat(&mut a, "idle", 0, all.clone(), vec![], None);
        a.set_history(false, false);
        let (now_top, now_delta) = a.top_entry().unwrap();
        assert_eq!(now_top, top);
        assert!((now_delta - delta).abs() < 0.5);
        assert!(
            (place(&a, &top) - before).abs() < 0.5,
            "same place on screen"
        );
    }

    #[test]
    fn nearing_the_top_asks_for_earlier_sections_once_per_approach() {
        // Short content: the top is in the window at once.
        let mut a = screen(conversation(1), vec![], None);
        assert!(a.take_load_earlier());
        assert!(!a.take_load_earlier(), "once");
        // While the page is on its way, and when it lands, the marker row
        // shows and goes.
        a.set_history(true, false);
        assert_eq!(a.flow().rows[0].text, LOADING_EARLIER);
        assert!(!a.take_load_earlier());
        a.set_history(false, false);
        assert_ne!(a.flow().rows[0].text, LOADING_EARLIER);
        // Landed and still near the top: the next approach asks again.
        flat(&mut a, "idle", 0, conversation(2), vec![], None);
        assert!(a.take_load_earlier());
        // At the start, no more.
        a.set_history(false, true);
        assert_eq!(a.flow().rows[0].text, START_OF_SESSION);
        flat(&mut a, "idle", 0, conversation(3), vec![], None);
        assert!(!a.take_load_earlier());

        // Long content at the bottom: nothing until scrolled near the top.
        let mut a = screen(conversation(60), vec![], None);
        assert!(!a.take_load_earlier());
        let mut t = Duration::ZERO;
        while a.offset() > a.frame().viewport.h {
            t = drag(&mut a, 400.0, 2000.0, 10, t) + Duration::from_millis(200);
            a.touch(1, TouchPhase::Ended, 540.0, 2000.0, t);
        }
        assert!(a.take_load_earlier());
    }

    #[test]
    fn far_entries_keep_their_place_but_not_their_rows() {
        let all = conversation(150);
        // Laid out whole, below the ribbon's marker row.
        let full = lay_out_flow_in(&all, &[], None, PHONE.w, PHONE.scale, Some(""), None);
        let a = screen(all.clone(), vec![], None);
        // At the bottom: the same places and height as laid out whole.
        assert_eq!(a.flow().entry_y, full.entry_y);
        assert_eq!(a.flow().height, full.height);
        // Only rows near the window are laid out.
        let (lo, _) = a.flow().kept.expect("a kept span");
        assert!(lo > 0.0);
        assert!(
            a.flow().rows.len() < full.rows.len() / 2,
            "{} of {}",
            a.flow().rows.len(),
            full.rows.len()
        );
        // Past the marker row, always at the top.
        let rows = &a.flow().rows[1..];
        assert!(rows.iter().all(|r| r.y >= lo - 200.0));
        let first = rows.iter().map(|r| r.y).fold(f32::MAX, f32::min);
        assert!(first > full.height / 2.0);
        // Scrolled to the top, the top is laid out again.
        let mut a = a;
        let mut t = Duration::ZERO;
        while a.offset() > 0.0 {
            t = drag(&mut a, 400.0, 2200.0, 10, t) + Duration::from_millis(200);
            a.touch(1, TouchPhase::Ended, 540.0, 2200.0, t);
        }
        assert!(a.flow().rows.iter().any(|r| r.text.contains("question 0")));
        assert!(
            !a.flow()
                .rows
                .iter()
                .any(|r| r.text.contains("question 149"))
        );
        assert_eq!(a.flow().entry_y, full.entry_y);
    }

    #[test]
    fn the_header_says_where_the_terminal_is_when_it_is_elsewhere() {
        let mut a = screen(conversation(3), vec![], None);
        let plain = a.frame().header.h;
        a.set_terminal_on(Some("Codex in keypad".into()));
        assert_eq!(a.terminal_on(), Some("Codex in keypad"));
        assert!(a.frame().header.h > plain, "a line beneath the name");
        assert!(a.frame().viewport.y >= a.frame().header.h);
        a.set_terminal_on(None);
        assert_eq!(a.frame().header.h, plain);
    }

    // Plan 20261022_03, the 23:54 EDT amendment: the ribbon.

    /// A screen on the entries of `all` from `from` on, as sections.
    fn screen_from(all: &[Entry], from: usize) -> AgentScreen {
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        flat(&mut a, "idle", from, all[from..].to_vec(), vec![], None);
        a
    }

    /// Where the section `serial`'s block is on screen.
    fn on_screen(a: &AgentScreen, serial: Serial) -> f32 {
        let i = a.slot_of(serial.to_u64()).expect("held");
        a.tops[i] - a.offset()
    }

    #[test]
    fn a_page_of_sections_landing_above_does_not_move_the_view() {
        let all = conversation(60);
        let mut a = screen_from(&all, 120);
        scrolled_back(&mut a);
        let (serial, delta) = a.place().unwrap();
        let at = on_screen(&a, serial);
        a.set_history(true, false);
        flat(&mut a, "idle", 0, all.clone(), vec![], None);
        a.set_history(false, false);
        assert_eq!(a.slots().len(), all.len());
        let (now, now_delta) = a.place().unwrap();
        assert_eq!(now, serial);
        assert!((now_delta - delta).abs() < 0.01);
        assert!((on_screen(&a, serial) - at).abs() < 0.01);
        assert!(!a.following());
    }

    #[test]
    fn sections_whose_entries_share_a_time_do_not_confuse_the_place() {
        // Parallel tool calls: the same time, role and kind, even the same
        // text.
        let same = |line: u32| ViewSection {
            index: 0,
            serial: Serial::new(FILE, line, 0),
            shows: Shows::Entries(vec![Entry {
                ts: "12:00:00".into(),
                ..entry(
                    Role::Assistant,
                    EntryKind::ToolUse,
                    "Bash {\"command\":\"ls\"}",
                )
            }]),
        };
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        give(&mut a, "idle", (100..300).map(same).collect(), None);
        let target = a.tops[150] + 5.0;
        a.scroll_to(target);
        assert!(!a.following());
        let (serial, delta) = a.place().unwrap();
        assert_eq!(serial, Serial::new(FILE, 250, 0));
        let at = on_screen(&a, serial);
        // A page of more of the same lands above.
        give(&mut a, "idle", (20..300).map(same).collect(), None);
        assert_eq!(a.place(), Some((serial, delta)));
        assert!((on_screen(&a, serial) - at).abs() < 0.01);
    }

    #[test]
    fn replacing_a_section_below_the_window_or_removing_the_top_one_holds_the_view() {
        let all = conversation(60);
        let mut a = screen_from(&all, 0);
        scrolled_back(&mut a);
        let offset = a.offset();
        let (serial, delta) = a.place().unwrap();
        // The newest section grows, below the window.
        let mut grown = all.clone();
        grown
            .last_mut()
            .unwrap()
            .summary
            .push_str("\nAnd a third paragraph.\nAnd a fourth.");
        flat(&mut a, "idle", 0, grown.clone(), vec![], None);
        assert_eq!(a.offset(), offset);
        assert_eq!(a.place(), Some((serial, delta)));
        // The top section goes: the next one stays where it was on screen.
        let i = a.slot_of(serial.to_u64()).unwrap();
        let next = a.slots()[i + 1].serial;
        let at = on_screen(&a, next);
        let mut sections: Vec<ViewSection> = a
            .slots()
            .iter()
            .map(|s| ViewSection {
                index: 0,
                serial: s.serial,
                shows: a.blocks[&s.serial.to_u64()].shows.clone(),
            })
            .collect();
        sections.retain(|v| v.serial != serial);
        give(&mut a, "idle", sections, None);
        assert!(a.slot_of(serial.to_u64()).is_none());
        assert!((on_screen(&a, next) - at).abs() < 0.01);
        assert!(!a.following());
    }

    #[test]
    fn the_marker_row_changing_does_not_move_the_view() {
        let mut a = screen_from(&conversation(40), 0);
        scrolled_back(&mut a);
        let (offset, height, tops) = (a.offset(), a.flow().height, a.flow().entry_y.clone());
        for (loading, start, text) in [
            (true, false, LOADING_EARLIER),
            (false, true, START_OF_SESSION),
            (false, false, ""),
        ] {
            a.set_history(loading, start);
            assert_eq!(a.flow().rows[0].text, text);
            assert_eq!(a.offset(), offset);
            assert_eq!(a.flow().height, height);
            assert_eq!(a.flow().entry_y, tops);
        }
        // At the bottom too.
        let mut a = screen_from(&conversation(40), 0);
        let bottom = a.offset();
        a.set_history(true, false);
        assert_eq!(a.offset(), bottom);
        assert!(a.following());
    }

    #[test]
    fn a_fling_covers_the_same_distance_whether_or_not_a_page_lands() {
        let all = conversation(80);
        let flung = || {
            let mut a = screen_from(&all, 160);
            let t = drag(&mut a, 800.0, 1100.0, 5, Duration::ZERO);
            a.touch(1, TouchPhase::Ended, 540.0, 1100.0, t + MS);
            assert!(a.fling != 0.0);
            a
        };
        let (mut a, mut b) = (flung(), flung());
        let (serial, _) = a.place().unwrap();
        for _ in 0..10 {
            a.tick(MS);
            b.tick(MS);
        }
        // A page lands under b's fling.
        flat(&mut b, "idle", 0, all.clone(), vec![], None);
        for _ in 0..30 {
            a.tick(MS);
            b.tick(MS);
        }
        assert!((on_screen(&a, serial) - on_screen(&b, serial)).abs() < 0.5);
        assert!(on_screen(&a, serial) > 300.0, "it carried on");
        // A slow frame moves it by a thirtieth of a second at most.
        let mut c = flung();
        let (before, speed) = (c.offset(), c.fling.abs());
        c.tick(Duration::from_millis(500));
        assert!((before - c.offset()).abs() <= speed * FLING_DT_MAX + 0.5);
    }

    #[test]
    fn a_delivery_of_one_section_lays_out_one_block() {
        let all = conversation(4);
        let mut a = screen(all[..12].to_vec(), vec![], None);
        a.block_layouts = 0;
        flat(&mut a, "idle", 0, all[..13].to_vec(), vec![], None);
        assert_eq!(a.block_layouts, 1, "the new one");
        // Scrolled back, a section replaced: only it.
        let mut a = screen_from(&conversation(40), 0);
        scrolled_back(&mut a);
        a.block_layouts = 0;
        let mut grown = conversation(40);
        grown[150].summary.push_str(" (linked)");
        flat(&mut a, "idle", 0, grown, vec![], None);
        assert_eq!(a.block_layouts, 1);
        // A new width: every block again.
        let mut a = screen(all.clone(), vec![], None);
        a.block_layouts = 0;
        a.set_layout(Layout { w: 720.0, ..PHONE });
        assert_eq!(a.block_layouts, a.slots().len());
    }

    // Plan 20261022_03, the 00:05 EDT amendment: the jump line.

    #[test]
    fn the_jump_line_shows_scrolled_back_and_says_what_is_new() {
        let all = conversation(30);
        let mut a = screen(all[..80].to_vec(), vec![], None);
        assert!(a.frame().jump.is_none(), "hidden while following");
        scrolled_back(&mut a);
        let jump = |a: &AgentScreen| a.frame().jump.clone().expect("shown").text;
        assert_eq!(jump(&a), JUMP_TO_END);
        // One new section, then several.
        flat(&mut a, "idle", 0, all[..81].to_vec(), vec![], None);
        assert_eq!(jump(&a), "-- 1 new: question 20 --");
        flat(&mut a, "idle", 0, all[..84].to_vec(), vec![], None);
        assert_eq!(jump(&a), "-- 4 new: Answer 20. --");
        // A long message is cut to fit.
        let mut long = all[..84].to_vec();
        long.push(entry(
            Role::Assistant,
            EntryKind::Text,
            &"A very long reply. ".repeat(20),
        ));
        flat(&mut a, "idle", 0, long, vec![], None);
        let text = jump(&a);
        assert!(text.starts_with("-- 5 new: A very long reply."), "{text}");
        assert!(text.ends_with("\u{2026} --"), "{text}");
        assert!(
            text.chars().count()
                <= columns(PHONE.w - 2.0 * PAD * PHONE.scale, TOOL_PT * PHONE.scale)
        );
        // A tap goes back to the bottom, following; the line goes.
        let r = a.rect_of(AgentHit::JumpToEnd).expect("tappable");
        assert_eq!(
            a.hit(r.x + r.w * 0.5, r.y + r.h * 0.5),
            Some(AgentHit::JumpToEnd)
        );
        a.jump_to_end();
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
        assert!(a.frame().jump.is_none());
        // Left again: nothing new since.
        scrolled_back(&mut a);
        assert_eq!(jump(&a), JUMP_TO_END);
    }

    #[test]
    fn the_jump_line_showing_or_going_does_not_move_the_scrollback() {
        let mut a = screen(conversation(30), vec![], None);
        let (view, full) = (a.frame().viewport, a.frame().full_viewport_h);
        scrolled_back(&mut a);
        let line = a.frame().jump.clone().expect("shown");
        assert_eq!(a.frame().viewport, view);
        assert_eq!(a.frame().full_viewport_h, full);
        assert!(
            (line.rect.y - (view.y + view.h)).abs() < 0.01,
            "just below it"
        );
        let offset = a.offset();
        let buttons = a.frame().buttons[0].rect;
        assert!(line.rect.y + line.rect.h <= buttons.y);
        a.jump_to_end();
        assert_eq!(a.frame().viewport, view);
        assert!(a.offset() > offset);
    }
}
