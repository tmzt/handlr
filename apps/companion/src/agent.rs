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

use flankr_companion::view::{SectionKind, Slot as OutlineSlot, ViewWindow};
use flankr_companion::{AgentCard, Bookend, CardKind, Pressure, Shows, Suggestion, ViewSection};
use libcontrols::{Button, DrawContext, DrawControl, Rect};
use libflankr_proto::common::{Entry, EntryKind, Link, Role, Serial};
use libmorph::TouchPhase;
use libmsdf::drawlist::{AtlasRef, DrawList, SdfInstance, SdfKind};
#[cfg(test)]
use libmsdf::font::TerminalSource;
use libmsdf::font::{GlyphStyle, TerminalShaper};

use crate::controls::{ControlRow, SquareButton};
use crate::theme::{theme, with_alpha};
use crate::ui::{Layout, Line};

mod dictation_card;
mod input_box;
pub mod link_spans;
mod notice_line;
use dictation_card::DictationCard;
use input_box::InputBox;
use link_spans::LinkSpan;
use notice_line::NoticeLine;

/// A card kind's accent colour, in the theme in force.
pub fn accent(kind: CardKind) -> [f32; 4] {
    match kind {
        CardKind::Approval => theme().accent,
        CardKind::Question => theme().question,
        CardKind::Broke => theme().red,
        CardKind::Finished => theme().green,
        // A kind from a newer daemon.
        _ => theme().dim,
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
/// The session colour's band under the header, in points.
pub(crate) const SESSION_BAND: f32 = 4.0;
/// The suggestion row's height and type size, and the back glyph's tap
/// width at the header's left.
const SUGGEST_H: f32 = 36.0;
const SUGGEST_PT: f32 = 18.0;
const BACK_W: f32 = 44.0;
/// The record dot: white on blue, red while recording, centred in place of
/// a microphone glyph, which no bundled atlas has yet (a microphone icon
/// is for later).
pub(crate) const DOT: f32 = 18.0;
/// Send and cancel, from the terminal face's symbol page.
pub(crate) const SEND_GLYPH: &str = "\u{27A4}";
pub(crate) const CANCEL_GLYPH: &str = "\u{2715}";
/// The back glyph, from the terminal face (Roboto here is ASCII only).
const BACK_GLYPH: &str = "\u{2039}";
/// The play button's glyph, from the terminal face, and the skip glyph it
/// shows while a reply is being read (no ⏭ in the face: play and a bar).
pub(crate) const PLAY_GLYPH: &str = "\u{25B6}";
pub(crate) const SKIP_GLYPH: &str = "\u{25B6}\u{2502}";
/// Lines of a folded tool result shown: one, or a diff's or an error's
/// first few.
const DIFF_LINES: usize = 4;
const ERROR_LINES: usize = 4;
/// Lines of a summary card's short form, at most (the 12:08 EDT
/// amendment).
const SHORT_LINES: usize = 3;
/// The dim word at a cut short form's bottom right.
pub const MORE: &str = "more";
/// The dim label over a summary card's spoken text.
pub const SPOKEN: &str = "spoken";
/// A point's mark.
const BULLET: &str = "\u{2022} ";
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
    /// The keyboard glyph left of record: type a message (plan
    /// 20261022_03, the L3 amendment on the keyboard glyph).
    Keyboard,
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
    /// A tool call, a diff or a summary card in the scrollback: expand it,
    /// or fold it again (plan 20261022_03, the L3 amendments of 2026-10-24
    /// 11:54 to 12:03 EDT). Its section's packed serial.
    Fold(u64),
    /// A summary card's play button: say its spoken text again. The packed
    /// serial of its card.
    Replay(u64),
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
    /// Areas a tap folds or expands: a tool section, a diff or a summary
    /// card, the whole block ([`AgentHit::Fold`]).
    pub taps: Vec<(AgentHit, Rect)>,
}

/// What a block is laid out with besides its section: its key (the packed
/// serial, for its taps), whether the user expanded it, whether sound is
/// on (a summary card's replay button), and, for a summary card, the
/// whole reply it is about when held.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockState {
    pub key: u64,
    pub expanded: bool,
    pub sound: bool,
    pub context: Option<String>,
}

impl Default for BlockState {
    fn default() -> Self {
        BlockState {
            key: 0,
            expanded: false,
            sound: true,
            context: None,
        }
    }
}

impl BlockState {
    /// A summary card's form in the scrollback: short, or expanded.
    fn form(&self, card: &AgentCard) -> CardForm<'_> {
        if !card.is_summary() {
            CardForm::Text {
                max_lines: usize::MAX,
            }
        } else if self.expanded {
            CardForm::Expanded {
                context: self.context.as_deref(),
                max_lines: usize::MAX,
                key: self.key,
                sound: self.sound,
            }
        } else {
            CardForm::Short {
                context: self.context.as_deref(),
            }
        }
    }
}

/// An entry's whole text: `Entry::text` when the daemon sent it (plan
/// 20261022_03, the L3 amendment of 2026-10-24 11:54 EDT), else `summary`
/// (an older daemon, or a text that was not cut).
pub fn full_text(e: &Entry) -> &str {
    e.text.as_deref().unwrap_or(&e.summary)
}

/// `raw` after `lead`, wrapped at `cols`, its own indentation kept and
/// the lines after the first indented under it.
fn wrap_indented(lead: &str, raw: &str, cols: usize) -> Vec<String> {
    let body = raw.trim_start();
    let indent = format!("{lead}{}", &raw[..raw.len() - body.len()]);
    let width = indent.chars().count();
    let hang = " ".repeat(width);
    wrap_hard(body, cols.saturating_sub(width).max(1))
        .into_iter()
        .enumerate()
        .map(|(i, l)| format!("{}{l}", if i == 0 { &indent } else { &hang }))
        .collect()
}

/// A laid-out line of an entry: its text, type size and colour.
type EntryLine = (String, f32, [f32; 4]);

/// Marks the last of `lines` as cut: "…" at its end, unless it has one.
fn mark_cut(lines: &mut [EntryLine]) {
    if let Some((last, _, _)) = lines.last_mut()
        && !last.ends_with('\u{2026}')
    {
        last.push('\u{2026}');
    }
}

/// How an entry is drawn: its lines, their size and colours, whether it
/// sits on the grey band, and whether it is folded with more to show.
/// Text shows whole; a tool call and its result, and a diff, fold to
/// their first lines unless `expanded` (the 11:54 and 11:59 EDT
/// amendments).
fn entry_lines(
    e: &Entry,
    expanded: bool,
    cols: impl Fn(f32) -> usize,
) -> (Vec<EntryLine>, bool, bool) {
    let mut lines = Vec::new();
    let mut more = false;
    let full = full_text(e);
    match (e.role, e.kind) {
        (Role::User, EntryKind::Text) => {
            for l in wrap_hard(full.trim(), cols(TEXT_PT)) {
                lines.push((l, TEXT_PT, theme().fg));
            }
            return (lines, true, false);
        }
        (_, EntryKind::Text) => {
            // Replies are plain; inside a fenced block, diff lines are green
            // and red and the fences dim.
            let mut fenced = false;
            for raw in full.trim().split('\n') {
                let fence = raw.trim_start().starts_with("```");
                let color = if fence {
                    theme().dim
                } else if fenced {
                    diff_colour(raw).unwrap_or(theme().fg)
                } else {
                    theme().fg
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
            let text = full.trim_end();
            let first = text.lines().next().unwrap_or("");
            if expanded {
                for (i, raw) in text.lines().enumerate() {
                    let lead = if i == 0 { "> " } else { "  " };
                    for l in wrap_indented(lead, raw, cols(TOOL_PT)) {
                        lines.push((l, TOOL_PT, theme().dim));
                    }
                }
            } else {
                let line = format!("> {first}");
                more = line.chars().count() > cols(TOOL_PT) || text.lines().nth(1).is_some();
                lines.push((one_line(&line, cols(TOOL_PT)), TOOL_PT, theme().dim));
            }
        }
        (_, EntryKind::ToolResult) => {
            if let Some(error) = full.strip_prefix("error: ") {
                let wrapped = wrap_hard(&format!("! {}", error.trim()), cols(TOOL_PT));
                more = !expanded && wrapped.len() > ERROR_LINES;
                let shown = if expanded { usize::MAX } else { ERROR_LINES };
                for l in wrapped.into_iter().take(shown) {
                    lines.push((l, TOOL_PT, theme().red));
                }
            } else if is_diff(full) {
                let raws: Vec<&str> = full.trim_end().lines().collect();
                if expanded {
                    for raw in raws {
                        let color = diff_colour(raw).unwrap_or(theme().dim);
                        for l in wrap_indented("  ", raw, cols(TOOL_PT)) {
                            lines.push((l, TOOL_PT, color));
                        }
                    }
                } else {
                    more = raws.len() > DIFF_LINES;
                    for raw in raws.iter().take(DIFF_LINES) {
                        let color = diff_colour(raw).unwrap_or(theme().dim);
                        let line = format!("  {raw}");
                        more |= line.chars().count() > cols(TOOL_PT);
                        lines.push((one_line(&line, cols(TOOL_PT)), TOOL_PT, color));
                    }
                }
            } else if expanded {
                for raw in full.trim_end().lines() {
                    for l in wrap_indented("  ", raw, cols(TOOL_PT)) {
                        lines.push((l, TOOL_PT, theme().dim));
                    }
                }
            } else {
                let mut shown = full.lines().filter(|l| !l.trim().is_empty());
                let first = shown.next().unwrap_or("");
                let line = format!("  {first}");
                more = line.chars().count() > cols(TOOL_PT) || shown.next().is_some();
                lines.push((one_line(&line, cols(TOOL_PT)), TOOL_PT, theme().dim));
            }
        }
    }
    if more {
        mark_cut(&mut lines);
    }
    (lines, false, more)
}

/// A diff line's colour: `+` green and `-` red, but not `+++` or `---`.
pub fn diff_colour(line: &str) -> Option<[f32; 4]> {
    if line.starts_with("+++") || line.starts_with("---") {
        None
    } else if line.starts_with('+') {
        Some(theme().green)
    } else if line.starts_with('-') {
        Some(theme().red)
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
            (AgentHit::Approve(id), "Approve", theme().approve),
            (AgentHit::Deny(id), "Deny", theme().deny),
        ],
        // A question about a plan (plan 20261022_03, the 21:57 EDT
        // amendment): just the three words, as plain coloured text, not
        // buttons (owner, 2026-10-23: "Yes/No are buttons but suggestions
        // (approve, impl) can be plain text"). The colour stands in the
        // background slot; `SquareButton::draw` draws these as text.
        (CardKind::Question, _) if card.about_plan => vec![
            (
                AgentHit::Suggest(Suggestion::Approve),
                "approve",
                theme().green,
            ),
            (
                AgentHit::Suggest(Suggestion::Discuss),
                "discuss",
                theme().link,
            ),
            (
                AgentHit::Suggest(Suggestion::Implement),
                "implement",
                theme().yellow,
            ),
            (AgentHit::Reply, "", theme().message),
        ],
        (CardKind::Question, _) => vec![
            (AgentHit::Suggest(Suggestion::Yes), "Yes", theme().approve),
            (AgentHit::Suggest(Suggestion::No), "No", theme().deny),
            (AgentHit::Reply, "", theme().message),
        ],
        _ => vec![],
    }
}

/// How a card's text is laid out (plan 20261022_03, the L3 amendments of
/// 2026-10-24 12:03 to 12:08 EDT).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CardForm<'a> {
    /// Its text, at most `max_lines` lines, the last cut with "…".
    Text { max_lines: usize },
    /// A summary card folded: its title and short form, with "more" when
    /// the short form is cut. `context` is the reply it is about, when
    /// held.
    Short { context: Option<&'a str> },
    /// A summary card expanded: the whole reply (at most `max_lines`
    /// lines), its points, and its spoken text, with a replay button for
    /// the block `key` when `sound` is on.
    Expanded {
        context: Option<&'a str>,
        max_lines: usize,
        key: u64,
        sound: bool,
    },
}

/// One line of a summary card's form.
#[derive(Clone, Debug, PartialEq)]
enum Part {
    /// The short form, or the full context.
    Body(String),
    /// A line of a point, the first with its bullet.
    Point(String),
    /// The dim "spoken" label.
    Label,
    /// A line of the spoken text.
    Spoken(String),
}

/// A summary card's short form at `cols` (the 12:08 EDT amendment): the
/// daemon's summary (`spoken`) whole when it has one; else the reply's
/// opening (`context`, else the card's text), its first paragraph, or,
/// when its first line ends in ":", the lines after it too, up to
/// `SHORT_LINES` lines. Whether it is cut: the last line then ends with
/// "…".
pub fn short_form(card: &AgentCard, context: Option<&str>, cols: usize) -> (Vec<String>, bool) {
    if let Some(spoken) = card
        .spoken
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        return (wrap_hard(spoken, cols), false);
    }
    let source = context.unwrap_or(&card.text).trim();
    let raws: Vec<&str> = source.lines().map(str::trim_end).collect();
    let lead_in = raws.first().is_some_and(|l| l.ends_with(':'));
    let opening: Vec<&str> = if lead_in {
        raws.iter()
            .copied()
            .filter(|l| !l.trim().is_empty())
            .collect()
    } else {
        raws.iter()
            .copied()
            .take_while(|l| !l.trim().is_empty())
            .collect()
    };
    let rest = raws.iter().filter(|l| !l.trim().is_empty()).count() > opening.len();
    let mut lines: Vec<String> = opening.iter().flat_map(|l| wrap_hard(l, cols)).collect();
    let mut cut = rest;
    if lines.len() > SHORT_LINES {
        lines.truncate(SHORT_LINES);
        cut = true;
    }
    if cut && let Some(last) = lines.last_mut() {
        if last.chars().count() >= cols {
            let kept: String = last.chars().take(cols.saturating_sub(1)).collect();
            *last = kept;
        }
        if !last.ends_with('\u{2026}') {
            last.push('\u{2026}');
        }
    }
    (lines, cut)
}

/// Whether a summary card's expanded form shows more than its short one.
pub fn summary_folds(card: &AgentCard, context: Option<&str>, cols: usize) -> bool {
    card.is_summary()
        && (short_form(card, context, cols).1 || !card.points.is_empty() || card.spoken.is_some())
}

/// A summary card's lines in `form`, at `cols`, and whether its short form
/// is cut. Without the daemon's spoken text there is no spoken part: what
/// reading aloud would say is worked out when it is said.
fn summary_parts(card: &AgentCard, form: CardForm, cols: usize) -> (Vec<Part>, bool) {
    match form {
        CardForm::Text { .. } => (Vec::new(), false),
        CardForm::Short { context } => {
            let (lines, cut) = short_form(card, context, cols);
            (lines.into_iter().map(Part::Body).collect(), cut)
        }
        CardForm::Expanded {
            context, max_lines, ..
        } => {
            let source = context.unwrap_or(&card.text).trim();
            let mut body = wrap_hard(source, cols);
            if body.len() > max_lines {
                body.truncate(max_lines.max(1));
                if let Some(last) = body.last_mut() {
                    last.push('\u{2026}');
                }
            }
            let mut parts: Vec<Part> = body.into_iter().map(Part::Body).collect();
            for p in &card.points {
                let wrapped = wrap_hard(p.trim(), cols.saturating_sub(2).max(1));
                for (i, l) in wrapped.into_iter().enumerate() {
                    let lead = if i == 0 { BULLET } else { "  " };
                    parts.push(Part::Point(format!("{lead}{l}")));
                }
            }
            if let Some(spoken) = card
                .spoken
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                parts.push(Part::Label);
                parts.extend(wrap_hard(spoken, cols).into_iter().map(Part::Spoken));
            }
            (parts, false)
        }
    }
}

/// The replay button beside the "spoken" label at `(x, y)`, for the block
/// `key`: the play glyph alone, dim.
fn replay_button(key: u64, x: f32, y: f32, s: f32) -> FlowButton {
    let h = line_h(TOOL_PT * s).max(REPLAY_W * s);
    FlowButton {
        hit: AgentHit::Replay(key),
        rect: Rect::new(x, y - (h - line_h(TOOL_PT * s)) * 0.5, REPLAY_W * s, h),
        label: "",
        bg: theme().dim,
    }
}

/// The replay button's width and height, in points.
const REPLAY_W: f32 = 28.0;

/// A live card's block at `y`, `w` wide from `x`: its fills, rows and
/// buttons, and its height. Its text as `form` says. Its links are marked
/// by their index in `targets`.
#[allow(clippy::too_many_arguments)]
fn card_block(
    card: &AgentCard,
    x: f32,
    y: f32,
    w: f32,
    s: f32,
    form: CardForm,
    targets: &[Link],
    flow: &mut Flow,
) -> f32 {
    let pad = CARD_PAD * s;
    let inner_x = x + EDGE * s + pad;
    let inner_w = w - EDGE * s - 2.0 * pad;
    let (title_px, text_px) = (TITLE_PT * s, CARD_PT * s);
    let label_px = TOOL_PT * s;
    let cols = columns(inner_w, text_px);
    let (parts, cut) = match form {
        CardForm::Text { max_lines } => {
            let mut lines = wrap_hard(card.text.trim(), cols);
            if lines.len() > max_lines {
                lines.truncate(max_lines);
                if let Some(last) = lines.last_mut() {
                    last.push('\u{2026}');
                }
            }
            (lines.into_iter().map(Part::Body).collect(), false)
        }
        _ => summary_parts(card, form, cols),
    };
    let part_h = |p: &Part| match p {
        Part::Label => line_h(label_px) + 4.0 * s,
        _ => line_h(text_px),
    };
    let lines_h: f32 = parts.iter().map(part_h).sum();
    let acts = actions(card);
    let mut h = pad + line_h(title_px) + 4.0 * s + lines_h + pad;
    if cut {
        h += line_h(label_px);
    }
    if !acts.is_empty() {
        h += ACTION_H * s + pad * 0.5;
    }
    let panel = Rect::new(x, y, w, h);
    flow.fills.push(Fill {
        rect: Rect::new(x + 3.0 * s, y + 4.0 * s, w, h),
        color: theme().shadow,
        radius: 10.0 * s,
    });
    flow.fills.push(Fill {
        rect: panel,
        color: theme().card,
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
    for p in parts {
        let (text, px, color) = match p {
            Part::Body(t) | Part::Point(t) | Part::Spoken(t) => (t, text_px, theme().fg),
            Part::Label => {
                flow.rows.push(Row {
                    text: SPOKEN.to_string(),
                    x: inner_x,
                    y: ty + 4.0 * s,
                    px: label_px,
                    color: theme().dim,
                    mono: false,
                });
                if let CardForm::Expanded {
                    key, sound: true, ..
                } = form
                {
                    let lx = inner_x + (SPOKEN.len() as f32 + 1.0) * label_px * 0.5;
                    flow.buttons.push(replay_button(key, lx, ty + 4.0 * s, s));
                }
                ty += line_h(label_px) + 4.0 * s;
                continue;
            }
        };
        flow.rows.push(Row {
            text,
            x: inner_x,
            y: ty,
            px,
            color,
            mono: false,
        });
        ty += line_h(text_px);
    }
    if cut {
        // "more", small and dim, at the card's bottom right.
        let w_more = MORE.len() as f32 * label_px * 0.5;
        flow.rows.push(Row {
            text: MORE.to_string(),
            x: inner_x + inner_w - w_more,
            y: ty,
            px: label_px,
            color: theme().dim,
            mono: false,
        });
        ty += line_h(label_px);
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
        color: theme().dim,
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
/// is `pinned` shows at the bottom instead, and keeps only its place. A
/// summary card shows its short or expanded form as `state` says.
fn lay_out_card(
    card: &AgentCard,
    pinned: bool,
    y: f32,
    pen: Pen,
    targets: &[Link],
    flow: &mut Flow,
    state: &BlockState,
) -> f32 {
    let s = pen.s;
    flow.card_y.push(y);
    if card.live && pinned {
        // Shown at the bottom instead.
        y
    } else if card.live {
        let y = y + 8.0 * s;
        *flow.card_y.last_mut().expect("pushed") = y;
        let form = state.form(card);
        y + card_block(card, pen.pad, y, pen.text_w, s, form, targets, flow) + 12.0 * s
    } else if card.is_summary() {
        lay_out_resolved_summary(card, y, pen, state, flow)
    } else {
        // Resolved: ordinary scrollback, dim, shown whole (the 11:59 EDT
        // amendment).
        let text = format!("-- {}: {}", card.title, card.text.trim());
        let px = TOOL_PT * s;
        let mut ly = y;
        for line in wrap_hard(&text, pen.cols(TOOL_PT)) {
            flow.rows.push(Row {
                text: line,
                x: pen.pad,
                y: ly,
                px,
                color: theme().dim,
                mono: true,
            });
            ly += term_h(px);
        }
        ly.max(y + term_h(px)) + 4.0 * s
    }
}

/// A resolved summary card in the scrollback: `-- <title>: ` and its
/// short form, dim, with "more" at the bottom right when cut; or,
/// expanded, its title, the whole reply, its points and its spoken text
/// with the replay button.
fn lay_out_resolved_summary(
    card: &AgentCard,
    y: f32,
    pen: Pen,
    state: &BlockState,
    flow: &mut Flow,
) -> f32 {
    let s = pen.s;
    let px = TOOL_PT * s;
    let lh = term_h(px);
    let lead = format!("-- {}: ", card.title);
    let indent = " ".repeat(3);
    let cols = pen
        .cols(TOOL_PT)
        .saturating_sub(lead.chars().count())
        .max(8);
    let form = state.form(card);
    let (parts, cut) = summary_parts(card, form, cols);
    let mut ly = y;
    let push = |flow: &mut Flow, text: String, color: [f32; 4], ly: &mut f32| {
        flow.rows.push(Row {
            text,
            x: pen.pad,
            y: *ly,
            px,
            color,
            mono: true,
        });
        *ly += lh;
    };
    let expanded = matches!(form, CardForm::Expanded { .. });
    if expanded {
        push(flow, format!("-- {}", card.title), theme().dim, &mut ly);
    }
    for (i, p) in parts.into_iter().enumerate() {
        match p {
            Part::Body(t) if !expanded && i == 0 => {
                push(flow, format!("{lead}{t}"), theme().dim, &mut ly)
            }
            Part::Body(t) if !expanded => push(flow, format!("{indent}{t}"), theme().dim, &mut ly),
            Part::Body(t) | Part::Point(t) => {
                push(flow, format!("{indent}{t}"), theme().fg, &mut ly)
            }
            Part::Spoken(t) => push(flow, format!("{indent}{t}"), theme().fg, &mut ly),
            Part::Label => {
                let label = format!("{indent}{SPOKEN}");
                if state.sound {
                    let lx = pen.pad + (label.chars().count() as f32 + 1.0) * px * CELL_EM;
                    flow.buttons.push(replay_button(state.key, lx, ly, s));
                }
                push(flow, label, theme().dim, &mut ly);
            }
        }
    }
    if cut {
        // "more", at the bottom right.
        let w_more = MORE.len() as f32 * px * CELL_EM;
        flow.rows.push(Row {
            text: MORE.to_string(),
            x: pen.pad + pen.text_w - w_more,
            y: ly,
            px,
            color: theme().dim,
            mono: true,
        });
        ly += lh;
    }
    ly.max(y + lh) + 4.0 * s
}

/// Lays out entry `e` at `y` into `flow`, after an entry that
/// `previous_compacts`, a tool call or diff folded unless `expanded`; the
/// y after it.
fn lay_out_entry(
    e: &Entry,
    previous_compacts: bool,
    expanded: bool,
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
    let (lines, band, _) = entry_lines(e, expanded, |pt| pen.cols(pt));
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
            color: theme().band,
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
            y = lay_out_card(
                card,
                pinned == Some(next_card),
                y,
                pen,
                &targets,
                &mut flow,
                &BlockState::default(),
            );
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
        y = lay_out_entry(e, previous_compacts, false, y, pen, &targets, &mut flow);
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
    lay_out_block_as(
        shows,
        pinned,
        after_compact,
        &BlockState::default(),
        width,
        s,
    )
}

/// As [`lay_out_block`], folded or expanded as `state` says. A block that
/// folds (a tool call or diff with more than its first lines, or a summary
/// card) is a tap area, [`AgentHit::Fold`] of its key.
pub fn lay_out_block_as(
    shows: &Shows,
    pinned: bool,
    after_compact: bool,
    state: &BlockState,
    width: f32,
    s: f32,
) -> Flow {
    let mut flow = Flow::default();
    let pen = Pen::new(width, s);
    let mut y = 0.0;
    let mut folds = false;
    match shows {
        Shows::Entries(entries) => {
            flow.targets = link_spans::targets(entries.iter().map(|e| &e.links[..]));
            let targets = std::mem::take(&mut flow.targets);
            let mut previous_compacts = after_compact;
            for e in entries {
                y = lay_out_entry(
                    e,
                    previous_compacts,
                    state.expanded,
                    y,
                    pen,
                    &targets,
                    &mut flow,
                );
                previous_compacts = NoticeLine::compacts(e);
                folds |= e.origin.is_none() && entry_lines(e, false, |pt| pen.cols(pt)).2;
            }
            flow.targets = targets;
        }
        Shows::Card(card) => {
            let targets = link_spans::targets([&card.links[..]]);
            y = lay_out_card(card, pinned, y, pen, &targets, &mut flow, state);
            flow.targets = targets;
            let cols = if card.live {
                columns(pen.text_w - EDGE * s - 2.0 * CARD_PAD * s, CARD_PT * s)
            } else {
                pen.cols(TOOL_PT)
                    .saturating_sub(card.title.chars().count() + 5)
                    .max(8)
            };
            folds = !(card.live && pinned) && summary_folds(card, state.context.as_deref(), cols);
        }
        Shows::Pending { kind, lines, chars } => {
            y = lay_out_pending(*kind, *lines, *chars, pen, &mut flow);
        }
    }
    flow.height = y;
    if folds {
        flow.taps
            .push((AgentHit::Fold(state.key), Rect::new(0.0, 0.0, width, y)));
    }
    flow
}

/// One sweep of the shimmer, in seconds.
const SHIMMER_PERIOD: f32 = 1.6;

/// The estimated height of a section not yet held (plan 20261024_01,
/// decision 7), from its slot's `lines` and `chars`: its text wrapped at
/// the view's font as the layout would wrap plain text, and a block's
/// fixed parts. A Text or Turn block is its lines, or its characters'
/// wrapped rows when more, at the text size after its gap (a turn on its
/// band); a tool call is its call and result lines, at most two; a context
/// entry and a resolved card are one dim line.
pub fn estimate_height(kind: SectionKind, lines: u32, chars: u32, width: f32, s: f32) -> f32 {
    let pen = Pen::new(width, s);
    let tool_line = term_h(TOOL_PT * s);
    match kind {
        SectionKind::Tool => lines.clamp(1, 2) as f32 * (2.0 * s + tool_line),
        SectionKind::Context => 4.0 * s + tool_line,
        // In the scrollback a card is mostly resolved: one dim line. A
        // live one comes with its contents (the newest slots).
        SectionKind::Card => tool_line + 4.0 * s,
        _ => {
            let cols = pen.cols(TEXT_PT).max(1) as u32;
            let rows = lines.max(chars.div_ceil(cols)).max(1);
            let band = if kind == SectionKind::Turn {
                8.0 * s
            } else {
                0.0
            };
            10.0 * s + rows as f32 * term_h(TEXT_PT * s) + band
        }
    }
}

/// A slot without contents: faint rounded bars, one per estimated line,
/// the last one shorter, or one faint panel for a card; the y after it.
fn lay_out_pending(kind: SectionKind, lines: u32, chars: u32, pen: Pen, flow: &mut Flow) -> f32 {
    let s = pen.s;
    let h = estimate_height(kind, lines, chars, pen.width, s);
    let bar = |flow: &mut Flow, y: f32, line: f32, w: f32| {
        let bh = (line * 0.55).max(2.0 * s);
        flow.fills.push(Fill {
            rect: Rect::new(pen.pad, y + (line - bh) / 2.0, w, bh),
            color: theme().placeholder,
            radius: bh / 2.0,
        });
    };
    match kind {
        SectionKind::Card => flow.fills.push(Fill {
            rect: Rect::new(pen.pad, 2.0 * s, pen.text_w, (h - 4.0 * s).max(1.0)),
            color: theme().placeholder,
            radius: 6.0 * s,
        }),
        SectionKind::Tool | SectionKind::Context => {
            let line = term_h(TOOL_PT * s);
            let tool = kind == SectionKind::Tool;
            let n = if tool { lines.clamp(1, 2) } else { 1 };
            let mut y = if tool { 2.0 * s } else { 4.0 * s };
            for i in 0..n {
                let w = if i + 1 == n { 0.45 } else { 0.7 };
                bar(flow, y, line, pen.text_w * w);
                y += line + 2.0 * s;
            }
        }
        _ => {
            let line = term_h(TEXT_PT * s);
            let band = if kind == SectionKind::Turn {
                8.0 * s
            } else {
                0.0
            };
            let rows = ((h - 10.0 * s - band) / line).round().max(1.0) as u32;
            let mut y = 10.0 * s + band / 2.0;
            for i in 0..rows {
                let w = if i + 1 == rows && rows > 1 { 0.6 } else { 1.0 };
                bar(flow, y, line, pen.text_w * w);
                y += line;
            }
        }
    }
    h
}

/// The shimmer's band at `t` seconds into a sweep, over a bar `r` in a
/// view `w` wide: the parts of the bar under the band, three nested
/// widths, so that drawn over each other it is softest at its edges.
fn shimmer_over(r: Rect, t: f32, w: f32) -> Vec<Rect> {
    let band = w * 0.35;
    let cx = -band + (w + 2.0 * band) * (t / SHIMMER_PERIOD).fract();
    let mut out = Vec::new();
    for k in [1.0, 0.66, 0.33] {
        let half = band * k / 2.0;
        let (x0, x1) = ((cx - half).max(r.x), (cx + half).min(r.x + r.w));
        if x1 > x0 {
            out.push(Rect::new(x0, r.y, x1 - x0, r.h));
        }
    }
    out
}

/// One finger of a drag: its id, where it came down, and its last y.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Finger {
    id: u64,
    start: [f32; 2],
    y: f32,
}

/// The fingers on the flow (plan 20261022_03, the 11:09 EDT amendment,
/// item 1): one drag, which a second finger joins without a jump.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    /// The fingers down, the first one first.
    fingers: Vec<Finger>,
    /// The fingers' mean movement, summed: what the samples follow, so a
    /// finger joining or leaving does not move it.
    pos: f32,
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
    /// More than one finger came down: never a tap.
    multi: bool,
    /// When the fingers went from two to one, if they did: a release soon
    /// after flings at `TWO_FINGER`, as the fingers lift together.
    pair_lifted: Option<Duration>,
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
/// With two fingers down, a drag moves this many times the fingers' mean
/// movement, and its fling starts at this many times the one-finger
/// fling (plan 20261022_03, the 11:09 EDT amendment, item 1).
pub(crate) const TWO_FINGER: f32 = 5.0;
/// The scrollbar's thumb (the 11:09 EDT amendment, items 2 and 3), in
/// points: its width, its least length, and the slop either side of it
/// that is its hit strip. It sits `BAR_SLOP` in from the right edge, so
/// the strip ends at the edge.
const BAR_W: f32 = 4.0;
const BAR_MIN: f32 = 24.0;
const BAR_SLOP: f32 = 4.0;
/// The thumb's opacity, in the text's colour.
const BAR_ALPHA: f32 = 0.35;
/// How long the scrollbar stays fully visible once the scrolling stops,
/// then how long it fades out over.
pub const BAR_HOLD: Duration = Duration::from_secs(2);
pub const BAR_FADE: Duration = Duration::from_millis(300);
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
    /// Where other sessions' cards end, above the pinned card (plan
    /// 20261022_03, the L3 amendment of 2026-10-24 15:56 EDT, item 2).
    pub cross_bottom: f32,
}

/// The one-line "jump to end" message below the scrollback, while
/// scrolled back (plan 20261022_03, the 00:05 EDT amendment): its tap area,
/// the screen's width, and its text, drawn centred in its colour: the
/// state at the session's end (the 01:07 EDT amendment).
#[derive(Clone, Debug, PartialEq)]
pub struct JumpLine {
    pub rect: Rect,
    pub text: String,
    pub color: [f32; 4],
}

/// The jump line's height, in points, kept whether or not it shows.
const JUMP_H: f32 = 28.0;
/// The jump line with no state to tell.
pub const JUMP_TO_END: &str = "-- jump to end --";
/// How long the jump line's scroll to the end takes, whatever the
/// distance (plan 20261022_03, the 01:06 EDT amendment, item 1).
pub const JUMP_SCROLL: Duration = Duration::from_secs(2);

/// `-- <what>: <line> --` in at most `cols` characters, the line cut to
/// fit; `-- <what> --` without one.
fn state_line(what: &str, line: &str, cols: usize) -> String {
    if line.is_empty() {
        return format!("-- {what} --");
    }
    let lead = format!("-- {what}: ");
    let room = cols.saturating_sub(lead.chars().count() + 3).max(1);
    format!("{lead}{} --", one_line(line, room))
}

/// Pull to refresh (plan 20261022_03, the L3 amendment of 2026-10-24
/// 12:22 EDT): the finger's movement past the end counts at this share,
/// up to `PULL_MAX` points; let go past `PULL_ARM`, it refreshes.
const PULL_RESIST: f32 = 0.5;
pub const PULL_MAX: f32 = 96.0;
pub const PULL_ARM: f32 = 64.0;
/// How fast a pull springs back, in points a second.
const PULL_SPRING: f32 = 480.0;
/// How long the refresh's result line shows.
pub const PULL_RESULT: Duration = Duration::from_secs(1);
pub const PULL_TO_REFRESH: &str = "-- pull to refresh --";
pub const RELEASE_TO_REFRESH: &str = "-- release to refresh --";
pub const REFRESHING: &str = "-- refreshing\u{2026} --";
pub const UP_TO_DATE: &str = "-- up to date --";
/// A section label's text before its card's summary arrives.
pub const DONE: &str = "Done";

/// Where a pull to refresh is.
#[derive(Clone, Debug, Default, PartialEq)]
enum PullState {
    #[default]
    Idle,
    /// Let go past `PULL_ARM`: the client refreshes.
    Refreshing,
    /// The result line, `t` seconds in.
    Result {
        text: String,
        color: [f32; 4],
        t: f32,
    },
}

/// A section label on the scrollbar's scale (the 12:27 EDT amendment):
/// its text and pill, and whether it is the one nearest the thumb.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionLabel {
    pub text: String,
    pub rect: Rect,
    pub nearest: bool,
}

/// A label's pill and text, in the text colour at these opacities; the
/// one nearest the thumb brighter.
const LABEL_BG: f32 = 0.10;
const LABEL_TEXT: f32 = 0.60;
const LABEL_NEAR_BG: f32 = 0.20;
const LABEL_NEAR_TEXT: f32 = 0.90;

/// The cubic ease-in-out of the jump scroll: from 0 at `u = 0` to 1 at
/// `u = 1`, with zero speed at both ends and the highest midway.
pub fn ease_in_out(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    if u < 0.5 {
        4.0 * u * u * u
    } else {
        1.0 - (-2.0 * u + 2.0).powi(3) / 2.0
    }
}

/// When the scrollbar is drawn, and whether its band takes touches (L3,
/// the band amendment).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollbarVisibility {
    /// No thumb and no band: the strip is plain scrollback.
    Disabled,
    /// The thumb while scrolling and `BAR_HOLD` after; the band always.
    #[default]
    Hidden,
    /// The thumb always; the band always.
    Always,
}

/// The scrollbar's thumb held by a finger: its id, the finger's y less the
/// thumb's top when it came down, and the offset then.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ThumbDrag {
    id: u64,
    grab: f32,
    from: f32,
}

/// The jump line's scroll to the end, running: where it started, in
/// pixels from the top of the flow, and how long it has run.
#[derive(Clone, Copy, Debug, PartialEq)]
struct JumpScroll {
    start: f32,
    elapsed: Duration,
}

/// The first non-empty line of `text`, trimmed.
fn text_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|l| !l.is_empty())
}
/// How far from the window contents are kept, in screens; beyond, the
/// client forgets them (plan 20261024_01, decision 6).
pub const FORGET_SCREENS: f32 = 20.0;

/// The first line of what a section shows: its first entry's text (a
/// turn's prompt, a reply, a tool call's summary), or its card's title.
fn first_line(shows: &Shows) -> Option<String> {
    let line = match shows {
        Shows::Entries(entries) => entries
            .iter()
            .find_map(|e| e.summary.lines().map(str::trim).find(|l| !l.is_empty()))?,
        Shows::Card(card) => card.title.trim(),
        Shows::Pending { .. } => return None,
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
        Suggestion::Yes => theme().green,
        Suggestion::No => theme().red,
        Suggestion::Continue | Suggestion::Compact => theme().yellow,
        Suggestion::Approve | Suggestion::Implement => theme().green,
        Suggestion::Discuss => theme().link,
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
    /// Folded or expanded, sound, and a summary card's reply.
    state: BlockState,
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
            state: BlockState::default(),
            flow: Flow::default(),
            rows: false,
            stale: true,
        }
    }

    fn lay_out(&mut self, width: f32, s: f32) {
        self.flow = lay_out_block_as(
            &self.shows,
            self.pinned,
            self.after_compact,
            &self.state,
            width,
            s,
        );
        self.rows = true;
        self.stale = false;
    }

    /// The memory guard: only its height, tops and links stay.
    fn drop_rows(&mut self) {
        self.flow.rows = Vec::new();
        self.flow.fills = Vec::new();
        self.flow.buttons = Vec::new();
        self.flow.links = Vec::new();
        self.flow.taps = Vec::new();
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
    /// The sections the user expanded (tool calls, diffs, summary cards),
    /// by packed serial, while the view is open.
    expanded: HashSet<u64>,
    /// *All sound* is on: summary cards draw their replay button.
    sound: bool,
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
    /// The jump line's scroll to the end, while it runs.
    jump: Option<JumpScroll>,
    /// The end of a new outline the client holds aside (the event's
    /// `held_end`): its live cards and newest section, for the jump line's
    /// state.
    held_end: Vec<ViewSection>,
    /// Packed serial to its slot's place in `slots`.
    slot_index: HashMap<u64, usize>,
    /// A new outline is held aside by the client: the jump line's tap
    /// asks for its newer sections below the end.
    new_content: bool,
    /// The window on the ribbon, by index, as last worked out, and as last
    /// taken for the client.
    window: Option<ViewWindow>,
    window_taken: Option<ViewWindow>,
    dirty: bool,
    /// Seconds into the placeholders' shimmer sweep.
    shimmer: f32,
    /// The scrollbar, while it shows: seconds since the scrolling
    /// stopped (0 while it goes on). `None` once it has faded.
    bar: Option<f32>,
    /// The scrollbar's thumb, while a finger drags it.
    thumb: Option<ThumbDrag>,
    /// The scrollbar's visibility setting.
    bar_mode: ScrollbarVisibility,
    /// The slots of a new outline held aside that come after the ribbon's
    /// newest (the event's `held_newer`): counted in the scrollbar's
    /// length at their estimated heights.
    held_newer: Vec<OutlineSlot>,
    /// A placeholder was on screen at the last draw: the shimmer runs.
    placeholders_shown: bool,
    /// How far the view is pulled past its end, in pixels, and the pull's
    /// state; it springs back while `springing`.
    pull: f32,
    pull_state: PullState,
    springing: bool,
    /// A pull was let go past `PULL_ARM`: the client is to refresh.
    wants_refresh: bool,
    /// The Finished cards' slots, for the section labels.
    bookends: Vec<Bookend>,
    /// The section labels, while they show: seconds since the thumb was
    /// let go (0 while it is held).
    labels: Option<f32>,
    /// Memory pressure, until the app is back in front.
    pressure: Option<Pressure>,
    /// The session's colour, 0 to 15, for the header's band (plan
    /// 20261022_03, the L3 amendment of 2026-10-24 15:56 EDT, item 1).
    session_colour: Option<u8>,
    /// The height other sessions' cards take above the pinned card.
    cross_h: f32,
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
            sound: true,
            ..Default::default()
        }
    }

    /// The session's colour (0 to 15), drawn as a band under the header.
    pub fn set_session_colour(&mut self, colour: Option<u8>) {
        if colour != self.session_colour {
            self.session_colour = colour;
            self.dirty = true;
        }
    }

    /// The band under the header in the session's colour, while it has
    /// one, and its colour.
    pub fn session_band(&self) -> Option<(Rect, [f32; 4])> {
        let colour = self.session_colour?;
        let h = self.frame.header;
        let band = SESSION_BAND * self.scale();
        Some((
            Rect::new(h.x, h.y + h.h - band, h.w, band),
            theme().session(colour),
        ))
    }

    /// The height other sessions' cards need above the pinned card: the
    /// flow makes room for them.
    pub fn set_cross_height(&mut self, h: f32) {
        if (h - self.cross_h).abs() > 0.01 {
            self.cross_h = h;
            self.settle();
        }
    }

    /// Where other sessions' cards end: above the pinned card, or above
    /// what is under the flow.
    pub fn cross_bottom(&self) -> f32 {
        self.frame.cross_bottom
    }

    /// *All sound* on or off: summary cards' replay buttons show or go.
    pub fn set_sound(&mut self, on: bool) {
        if on == self.sound {
            return;
        }
        self.sound = on;
        for b in self.blocks.values_mut() {
            if b.card().is_some_and(AgentCard::is_summary) {
                b.stale = true;
            }
        }
        let keep = self.keep_anchors();
        self.refresh_blocks();
        self.hold(keep);
        self.flow_stale = true;
        self.settle();
    }

    /// The theme changed: every block is laid out again in its colours,
    /// holding the view where it is, and the screen is drawn again.
    pub fn restyle(&mut self) {
        for b in self.blocks.values_mut() {
            b.stale = true;
        }
        let keep = self.keep_anchors();
        self.refresh_blocks();
        self.hold(keep);
        self.flow_stale = true;
        self.settle();
        self.dirty = true;
    }

    /// Expands the section `key`, or folds it again (the 11:54 EDT
    /// amendment, item 2): its block is laid out again, and the view holds
    /// by the section at its top, so it does not move.
    pub fn toggle_fold(&mut self, key: u64) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
        log::info!(
            "agent view: section {} {}",
            Serial::from_u64(key),
            if self.expanded.contains(&key) {
                "expanded"
            } else {
                "folded"
            }
        );
        let keep = self.anchors();
        self.refresh_blocks();
        self.hold(keep);
        self.follow = self.offset >= self.max_offset() - 1.0 && self.follow;
        self.flow_stale = true;
        self.settle();
    }

    /// Whether the section `key` is expanded.
    pub fn is_expanded(&self, key: u64) -> bool {
        self.expanded.contains(&key)
    }

    /// The spoken text of the summary card `key`, for its replay button.
    pub fn spoken_of(&self, key: u64) -> Option<String> {
        self.blocks
            .get(&key)
            .and_then(Block::card)
            .and_then(|c| c.spoken.clone())
    }

    /// The whole reply a card on the line of `serial` is about: the last
    /// reply in the sections on that line, as held.
    fn context_of(&self, serial: Serial) -> Option<String> {
        self.slots
            .iter()
            .filter(|s| s.serial.file == serial.file && s.serial.line == serial.line)
            .filter_map(|s| match &self.blocks.get(&s.serial.to_u64())?.shows {
                Shows::Entries(entries) => entries
                    .iter()
                    .rev()
                    .find(|e| e.role == Role::Assistant && e.kind == EntryKind::Text)
                    .map(|e| full_text(e).to_string()),
                _ => None,
            })
            .next_back()
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
        let is_pending =
            |b: Option<&Block>| matches!(b.map(|b| &b.shows), Some(Shows::Pending { .. }));
        let now_pending: HashSet<u64> = sections
            .iter()
            .filter(|v| matches!(v.shows, Shows::Pending { .. }))
            .map(|v| v.serial.to_u64())
            .collect();
        // Contents fetched for a placeholder, or forgotten, are not the
        // agent's activity.
        let mut entries_changed = changed
            .iter()
            .any(|k| is_entries(self.blocks.get(k)) && !now_pending.contains(k));
        if self.status != status {
            self.quiet_for = 0.0;
        }
        let keep = self.keep_anchors();
        let pin_was = self.pinned_card().map(|c| (c.kind, c.text.clone()));
        let mut slots = Vec::with_capacity(sections.len());
        for v in sections {
            let key = v.serial.to_u64();
            if changed.contains(&key) || !self.blocks.contains_key(&key) {
                entries_changed |=
                    matches!(v.shows, Shows::Entries(_)) && !is_pending(self.blocks.get(&key));
                self.blocks.insert(key, Block::new(v.shows));
            }
            slots.push(Slot {
                index: v.index,
                serial: v.serial,
                height: self.blocks[&key].flow.height,
            });
        }
        self.slots = slots;
        self.slot_index = self
            .slots
            .iter()
            .enumerate()
            .map(|(i, s)| (s.serial.to_u64(), i))
            .collect();
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
            // A jump scroll running keeps its curve on the same content.
            if let Some(j) = self.jump.as_mut() {
                j.start += to - self.offset;
            }
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
        // By map: in outline mode the slots are in index order, which a
        // late card can take out of serial order.
        self.slot_index.get(&serial).copied()
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
        // Each summary card's reply, by its line (the 12:03 EDT
        // amendment's full context).
        let contexts: HashMap<u64, Option<String>> = self
            .slots
            .iter()
            .filter(|slot| {
                self.blocks
                    .get(&slot.serial.to_u64())
                    .and_then(Block::card)
                    .is_some_and(AgentCard::is_summary)
            })
            .map(|slot| (slot.serial.to_u64(), self.context_of(slot.serial)))
            .collect();
        #[cfg(test)]
        let mut laid = 0;
        for slot in &mut self.slots {
            let key = slot.serial.to_u64();
            let b = self.blocks.get_mut(&key).expect("a block for each slot");
            let state = BlockState {
                key,
                expanded: self.expanded.contains(&key),
                sound: self.sound,
                context: contexts.get(&key).cloned().flatten(),
            };
            if b.state != state {
                b.state = state;
                b.stale = true;
            }
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
            flow.taps.extend(
                f.taps
                    .iter()
                    .map(|&(hit, r)| (hit, Rect::new(r.x, r.y + top, r.w, r.h))),
            );
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
    /// `KEEP_SCREENS` screens on each side, or fewer under memory
    /// pressure.
    fn keep_span(&self) -> (f32, f32) {
        let k = self.keep_screens();
        if k == 0.0 {
            let v = self.frame.viewport.h.max(1.0);
            return (self.offset, self.offset + v);
        }
        let screen = self.frame.viewport.h.max(self.layout.h).max(1.0);
        (self.offset - k * screen, self.offset + screen + k * screen)
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
        // Within a screen of the edge; at once at `Low`, which keeps no
        // margin.
        let margin = screen * self.keep_screens().min(1.0);
        let short_above = lo > 0.0 && self.offset - margin < lo;
        let window = if margin > 0.0 {
            screen
        } else {
            self.frame.viewport.h
        };
        let short_below = hi < self.ribbon_height() && self.offset + window + margin > hi;
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
                Shows::Card(_) | Shows::Pending { .. } => None,
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
        let cols = columns(inner_w, CARD_PT * s);
        if card.is_summary() {
            let context = self.pinned.and_then(|p| self.context_of(p));
            return summary_folds(card, context.as_deref(), cols);
        }
        wrap_hard(card.text.trim(), cols).len() > PIN_LINES
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

    /// Whether the pinned card is the bar: while the view is scrolled back
    /// or the scrollbar is held. An expanded card stays expanded while the
    /// view scrolls (owner, 2026-10-24: "scrolling with finish card
    /// expanded shouldn't collapse it").
    pub fn compact(&self) -> bool {
        // A finger moving at the bottom (a pull to refresh) leaves it whole
        // (owner, 2026-10-24): it folds once the view leaves the bottom.
        !self.pin_expanded && (!self.follow || self.thumb.is_some())
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
        self.note_window();
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
            // A summary card folds to its short form, and expands to its
            // reply, points and spoken text (the 12:03 EDT amendment).
            let context = self.pinned.and_then(|p| self.context_of(p));
            let form = match (card.is_summary(), self.pin_expanded) {
                (false, _) => CardForm::Text { max_lines },
                (true, false) => CardForm::Short {
                    context: context.as_deref(),
                },
                (true, true) => CardForm::Expanded {
                    context: context.as_deref(),
                    max_lines,
                    key: self.pinned.map_or(0, Serial::to_u64),
                    sound: self.sound,
                },
            };
            // The full card, laid out once to know its height.
            let mut full = Flow::default();
            let targets = &self.flow.targets;
            let h = card_block(card, pad, 0.0, l.w - 2.0 * pad, s, form, targets, &mut full);
            full_top = above - h;
            if compact {
                let bar = Rect::new(pad, above - BAR_H * s, l.w - 2.0 * pad, BAR_H * s);
                let mut flow = Flow::default();
                flow.fills.push(Fill {
                    rect: bar,
                    color: theme().card,
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
                    color: theme().fg,
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
                    form,
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
        // Other sessions' cards, above the pinned card: the flow ends
        // above them.
        let cross_bottom = view_bottom;
        let cross = if self.cross_h > 0.0 {
            self.cross_h + 8.0 * s
        } else {
            0.0
        };
        let (view_bottom, full_bottom) = (view_bottom - cross, full_bottom - cross);
        // The jump line's height is kept below the scrollback whether or
        // not it shows, so it never moves the scrollback (the 00:05 EDT
        // amendment, item 4).
        let jump_h = JUMP_H * s;
        let (view_bottom, full_bottom) = (view_bottom - jump_h, full_bottom - jump_h);
        let jump = (!self.follow).then(|| {
            let px = TOOL_PT * s;
            let (text, color) = self.jump_line(columns(l.w - 2.0 * pad, px));
            JumpLine {
                rect: Rect::new(0.0, view_bottom, l.w, jump_h),
                text,
                color,
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
            cross_bottom,
        }
    }

    /// The jump line's text and colour, at most `cols` characters: the
    /// state at the session's end (plan 20261022_03, the 01:07 EDT
    /// amendment), the first that holds of a question waiting, an approval
    /// needed, something broken, a finished turn and the agent working;
    /// else `-- jump to end --`. The states come from the live cards, of
    /// the ribbon and of an outline held aside, and herdr's status.
    fn jump_line(&self, cols: usize) -> (String, [f32; 4]) {
        match self.end_state(true) {
            Some((what, line, color)) => (state_line(what, &line, cols), color),
            None => (JUMP_TO_END.to_string(), theme().dim),
        }
    }

    /// The state at the session's end, the first that holds of a question
    /// waiting, an approval needed, something broken, a finished turn (its
    /// card's short form: the daemon's summary when it has one) and, with
    /// `working`, the agent working: what it is, its line, and its colour.
    fn end_state(&self, working: bool) -> Option<(&'static str, String, [f32; 4])> {
        // The ribbon's sections, then those held aside (newer), oldest
        // first.
        let shows: Vec<&Shows> = self
            .slots
            .iter()
            .filter_map(|s| self.blocks.get(&s.serial.to_u64()).map(|b| &b.shows))
            .chain(self.held_end.iter().map(|v| &v.shows))
            .collect();
        let live = |kind: CardKind| {
            shows.iter().rev().find_map(|sh| match sh {
                Shows::Card(c) if c.live && c.kind == kind => {
                    let short = c.spoken.as_deref().and_then(text_line);
                    Some(
                        short
                            .or_else(|| text_line(&c.text))
                            .unwrap_or(c.title.trim())
                            .to_string(),
                    )
                }
                _ => None,
            })
        };
        let is_working = self.status == "working";
        if let Some(q) = live(CardKind::Question) {
            Some(("question", q, accent(CardKind::Question)))
        } else if let Some(a) = live(CardKind::Approval) {
            Some(("approval", a, accent(CardKind::Approval)))
        } else if let Some(b) = live(CardKind::Broke) {
            Some(("broke", b, theme().red))
        } else if let Some(d) = live(CardKind::Finished).filter(|_| !is_working) {
            Some(("done", d, theme().green))
        } else if is_working && working {
            let newest = shows.iter().rev().find_map(|sh| first_line(sh));
            Some(("working", newest.unwrap_or_default(), theme().dim))
        } else {
            None
        }
    }

    /// Whether a refresh is asked for: the pull past the end was let go
    /// past `PULL_ARM` (plan
    /// 20261022_03, the L3 amendment of 2026-10-24 12:22 EDT). Once per
    /// pull.
    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.wants_refresh)
    }

    /// How far the view is pulled past its end, in pixels.
    pub fn pull(&self) -> f32 {
        self.pull
    }

    /// The dim line below the last section while pulled, refreshing, or
    /// giving the refresh's result.
    pub fn pull_line(&self) -> Option<(String, [f32; 4])> {
        match &self.pull_state {
            PullState::Refreshing => Some((REFRESHING.to_string(), theme().dim)),
            PullState::Result { text, color, .. } => Some((text.clone(), *color)),
            PullState::Idle if self.pull > 0.0 => {
                let text = if self.pull >= PULL_ARM * self.scale() {
                    RELEASE_TO_REFRESH
                } else {
                    PULL_TO_REFRESH
                };
                Some((text.to_string(), theme().dim))
            }
            PullState::Idle => None,
        }
    }

    /// The refresh is done ([`flankr_companion::Event::Refreshed`]): `new`
    /// sections came. Its result line shows for `PULL_RESULT`: the end's
    /// state as the jump line says it, else `-- N new --`, or `-- up to
    /// date --` with nothing new (the 12:23 EDT amendment); then the view
    /// springs back to the bottom, following.
    pub fn refreshed(&mut self, new: usize) {
        if self.pull_state != PullState::Refreshing {
            return;
        }
        let s = self.scale();
        let cols = columns(self.layout.w - 2.0 * PAD * s, TOOL_PT * s);
        let (text, color) = match (new, self.end_state(false)) {
            (0, _) => (UP_TO_DATE.to_string(), theme().dim),
            (_, Some((what, line, color))) => (state_line(what, &line, cols), color),
            (n, None) => (format!("-- {n} new --"), theme().dim),
        };
        log::info!("agent view: refreshed, {new} new");
        self.pull_state = PullState::Result {
            text,
            color,
            t: 0.0,
        };
        self.dirty = true;
    }

    /// A finger's move of `delta` pixels (down positive) at the end: the
    /// part past the end pulls, at half the finger's movement, up to
    /// `PULL_MAX`; moving back, the pull goes first. Whether it took the
    /// move.
    fn pull_by(&mut self, delta: f32) -> bool {
        if self.pull_state != PullState::Idle {
            return false;
        }
        let s = self.scale();
        let max = self.max_offset();
        if self.pull > 0.0 {
            let p = self.pull - delta * PULL_RESIST;
            self.springing = false;
            self.dirty = true;
            if p >= 0.0 {
                self.pull = p.min(PULL_MAX * s);
                return true;
            }
            // Back past the end: the rest scrolls.
            self.pull = 0.0;
            self.scroll_to(max + p / PULL_RESIST);
            return true;
        }
        let target = self.offset - delta;
        if target <= max || delta >= 0.0 {
            return false;
        }
        self.scroll_to(max);
        self.pull = ((target - max) * PULL_RESIST).min(PULL_MAX * s);
        self.springing = false;
        self.dirty = true;
        true
    }

    /// The finger let go of a pull: past `PULL_ARM` it refreshes, holding
    /// there until the result; before, it springs back.
    fn release_pull(&mut self) {
        let s = self.scale();
        if self.pull >= PULL_ARM * s && self.pull_state == PullState::Idle {
            log::info!("agent view: pulled to refresh");
            self.pull = PULL_ARM * s;
            self.pull_state = PullState::Refreshing;
            self.wants_refresh = true;
        } else {
            self.springing = true;
        }
        self.dirty = true;
    }

    /// The pull's result line's time, and its spring back.
    fn step_pull(&mut self, dt: Duration) {
        if let PullState::Result { t, .. } = &mut self.pull_state {
            *t += dt.as_secs_f32();
            if *t >= PULL_RESULT.as_secs_f32() {
                self.pull_state = PullState::Idle;
                self.springing = true;
            }
            self.dirty = true;
        }
        if self.springing && self.drag.is_none() {
            let s = self.scale();
            self.pull = (self.pull - PULL_SPRING * s * dt.as_secs_f32()).max(0.0);
            self.dirty = true;
            if self.pull <= 0.0 {
                self.springing = false;
                let end = self.max_offset();
                self.scroll_to(end);
            }
        }
    }

    /// The scrollbar's section labels (the 12:27 EDT amendment), while the
    /// thumb is dragged and as it fades: each bookend's at its share of the
    /// calculated length down the track, centred on the screen; those that
    /// would overlap give way to the one nearest the thumb.
    pub fn section_labels(&self) -> Vec<SectionLabel> {
        if self.labels.is_none() || self.bookends.is_empty() {
            return Vec::new();
        }
        let Some((thumb, _)) = self.thumb_track() else {
            return Vec::new();
        };
        let l = &self.layout;
        let s = self.scale();
        let v = self.frame.viewport;
        let total = self.calculated_length().max(1.0);
        let px = TOOL_PT * s;
        let cols = columns(l.w - 6.0 * PAD * s, px);
        let h = line_h(px) + 8.0 * s;
        let thumb_y = thumb.y + thumb.h * 0.5;
        let mut placed: Vec<(f32, SectionLabel)> = self
            .bookends
            .iter()
            .filter_map(|b| {
                let i = self.slot_of(b.serial.to_u64())?;
                let share = (self.tops[i] / total).clamp(0.0, 1.0);
                let y = v.y + share * v.h;
                let summary = b.summary.as_deref().unwrap_or(DONE);
                let text = match &b.time {
                    Some(t) => format!("{summary} \u{b7} {t}"),
                    None => summary.to_string(),
                };
                let text = one_line(&text, cols);
                let w = text.chars().count() as f32 * px * 0.5 + 2.0 * PAD * s;
                let rect = Rect::new((l.w - w) * 0.5, y - h * 0.5, w, h);
                Some((
                    (y - thumb_y).abs(),
                    SectionLabel {
                        text,
                        rect,
                        nearest: false,
                    },
                ))
            })
            .collect();
        placed.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut shown: Vec<SectionLabel> = Vec::new();
        for (i, (_, mut label)) in placed.into_iter().enumerate() {
            let r = label.rect;
            let clear = shown
                .iter()
                .all(|o| r.y + r.h + 2.0 * s <= o.rect.y || r.y >= o.rect.y + o.rect.h + 2.0 * s);
            if clear {
                label.nearest = i == 0;
                shown.push(label);
            }
        }
        shown.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y));
        shown
    }

    /// The section labels' opacity: whole while the thumb is dragged and
    /// `BAR_HOLD` after, then fading over `BAR_FADE` with the thumb.
    pub fn labels_alpha(&self) -> f32 {
        let Some(t) = self.labels else {
            return 0.0;
        };
        1.0 - ((t - BAR_HOLD.as_secs_f32()) / BAR_FADE.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// The Finished cards' slots ([`flankr_companion::Event::AgentView`]'s
    /// `bookends`), for the section labels.
    pub fn set_bookends(&mut self, bookends: Vec<Bookend>) {
        if bookends != self.bookends {
            self.bookends = bookends;
            self.dirty = true;
        }
    }

    /// Memory pressure (the 12:21 EDT amendment): laid-out rows are kept
    /// only this close to the window, and, at `Low`, only on screen, the
    /// shimmer with them. `None` once the app is back in front.
    pub fn set_pressure(&mut self, pressure: Option<Pressure>) {
        if pressure == self.pressure {
            return;
        }
        self.pressure = pressure;
        if pressure == Some(Pressure::Low) {
            self.shimmer = 0.0;
            self.placeholders_shown = false;
        }
        self.flow_stale = true;
        self.settle();
    }

    /// How many screens either side of the window keep their rows.
    fn keep_screens(&self) -> f32 {
        match self.pressure {
            None | Some(Pressure::UiHidden) => KEEP_SCREENS,
            Some(Pressure::Background) => 1.0,
            Some(Pressure::Low) => 0.0,
        }
    }

    /// The blocks with laid-out rows, by packed serial.
    pub fn laid_out(&self) -> Vec<u64> {
        let mut keys: Vec<u64> = self
            .blocks
            .iter()
            .filter(|(_, b)| b.rows)
            .map(|(k, _)| *k)
            .collect();
        keys.sort_unstable();
        keys
    }

    /// The jump line's tap: scrolls to the end in `JUMP_SCROLL` on an
    /// ease-in-out curve, then follows again (plan 20261022_03, the 01:06
    /// EDT amendment). Returns whether a new outline was waiting, for the
    /// client to add its newer sections below the end.
    pub fn jump_to_end(&mut self) -> bool {
        self.fling = 0.0;
        let end = self.max_offset();
        log::info!(
            "agent view: jump scroll from {:.0} to {:.0}",
            self.offset,
            end
        );
        self.jump = Some(JumpScroll {
            start: self.offset,
            elapsed: Duration::ZERO,
        });
        self.bar = Some(0.0);
        self.dirty = true;
        std::mem::take(&mut self.new_content)
    }

    /// Whether the jump line's scroll to the end is running.
    pub fn jumping(&self) -> bool {
        self.jump.is_some()
    }

    /// Carries the jump scroll on by `dt`: the end is read again each
    /// frame, so an end that grows is still reached at `JUMP_SCROLL`.
    fn step_jump(&mut self, dt: Duration) -> bool {
        let Some(mut j) = self.jump else {
            return false;
        };
        j.elapsed += dt;
        let before = self.offset;
        let u = j.elapsed.as_secs_f32() / JUMP_SCROLL.as_secs_f32();
        if u >= 1.0 {
            self.jump = None;
            let end = self.max_offset();
            self.scroll_to(end);
            log::info!("agent view: jump scroll landed at {:.0}", self.offset);
            return self.offset != before;
        }
        self.jump = Some(j);
        let end = self.max_offset();
        // Not following until it lands, even within a pixel of the end.
        self.offset = (j.start + (end - j.start) * ease_in_out(u)).clamp(0.0, end);
        self.placed();
        self.offset != before
    }

    /// Whether the client holds a new outline aside (the event's
    /// `new_content`).
    pub fn set_new_content(&mut self, on: bool) {
        if on != self.new_content {
            self.new_content = on;
            self.settle();
        }
    }

    /// The end of the outline the client holds aside (the event's
    /// `held_end`), for the jump line's state.
    pub fn set_held_end(&mut self, held_end: Vec<ViewSection>) {
        if held_end != self.held_end {
            self.held_end = held_end;
            self.settle();
        }
    }

    /// The slots of the outline held aside after the ribbon's newest (the
    /// event's `held_newer`), for the scrollbar's length.
    pub fn set_held_newer(&mut self, held_newer: Vec<OutlineSlot>) {
        if held_newer != self.held_newer {
            self.held_newer = held_newer;
            self.dirty = true;
        }
    }

    /// The scrollbar's calculated length, in pixels: the ribbon, which
    /// counts every slot at its laid-out or estimated height, and the
    /// newer sections of an outline held aside at their estimated heights
    /// (the 11:09 EDT amendment, item 2).
    fn calculated_length(&self) -> f32 {
        let (w, s) = (self.layout.w, self.scale());
        let held: f32 = self
            .held_newer
            .iter()
            .map(|slot| estimate_height(slot.kind, slot.lines, slot.chars, w, s))
            .sum();
        self.ribbon_height() + held
    }

    /// The scrollbar's thumb on screen, for the offset as it is, and the
    /// offset at the calculated length's end; none when the session fits
    /// its window. Its length is the window's share of the calculated
    /// length, at least `BAR_MIN`; it runs from the session's top to that
    /// end, so with newer sections held aside it stops short at the old
    /// end.
    fn thumb_track(&self) -> Option<(Rect, f32)> {
        let v = self.frame.viewport;
        let s = self.scale();
        let total = self.calculated_length();
        let range = total - self.frame.full_viewport_h;
        if range <= 0.5 || v.h <= 0.0 {
            return None;
        }
        let len = (v.h / total * v.h).max(BAR_MIN * s).min(v.h);
        let u = (self.offset / range).clamp(0.0, 1.0);
        let y = v.y + u * (v.h - len);
        let x = v.x + v.w - (BAR_SLOP + BAR_W) * s;
        Some((Rect::new(x, y, BAR_W * s, len), range))
    }

    /// The scrollbar's visibility (a setting; `Hidden` until the settings
    /// screen gives one).
    pub fn set_scrollbar(&mut self, mode: ScrollbarVisibility) {
        if mode != self.bar_mode {
            self.bar_mode = mode;
            self.dirty = true;
        }
    }

    /// The band: the right edge's strip, `BAR_W` and `BAR_SLOP` either
    /// side, the window's height. A scrollbar unless `Disabled`, drawn or
    /// not.
    fn band(&self) -> Option<Rect> {
        if self.bar_mode == ScrollbarVisibility::Disabled {
            return None;
        }
        let (r, _) = self.thumb_track()?;
        let s = self.scale();
        let v = self.frame.viewport;
        Some(Rect::new(
            r.x - BAR_SLOP * s,
            v.y,
            r.w + 2.0 * BAR_SLOP * s,
            v.h,
        ))
    }

    /// The scrollbar's thumb on screen, while it shows.
    pub fn thumb_rect(&self) -> Option<Rect> {
        (self.bar_alpha() > 0.0)
            .then(|| self.thumb_track().map(|(r, _)| r))
            .flatten()
    }

    /// The thumb's opacity: `BAR_ALPHA` until `BAR_HOLD` after the
    /// scrolling stopped, then down to 0 over `BAR_FADE`.
    pub fn bar_alpha(&self) -> f32 {
        match self.bar_mode {
            ScrollbarVisibility::Disabled => return 0.0,
            ScrollbarVisibility::Always => return BAR_ALPHA,
            ScrollbarVisibility::Hidden => {}
        }
        let Some(t) = self.bar else {
            return 0.0;
        };
        let fade = ((t - BAR_HOLD.as_secs_f32()) / BAR_FADE.as_secs_f32()).clamp(0.0, 1.0);
        BAR_ALPHA * (1.0 - fade)
    }

    /// Something is scrolling the view: a drag past the slop, a fling,
    /// the jump scroll or the thumb.
    fn scrolling(&self) -> bool {
        self.thumb.is_some()
            || self.fling != 0.0
            || self.jump.is_some()
            || self.drag.as_ref().is_some_and(|d| d.moved)
    }

    /// Moves the view for the thumb's finger at `y`: in proportion to the
    /// calculated length, clamped to the ribbon.
    fn drag_thumb(&mut self, y: f32) {
        let (Some(t), Some((r, range))) = (self.thumb, self.thumb_track()) else {
            return;
        };
        let v = self.frame.viewport;
        let travel = v.h - r.h;
        if travel <= 0.0 {
            return;
        }
        let u = ((y - t.grab - v.y) / travel).clamp(0.0, 1.0);
        let max = self.max_offset();
        self.fling = 0.0;
        self.scroll_to((u * range).min(max));
    }

    /// The window on the ribbon, by index, when it changed since last
    /// taken: for [`flankr_companion::Input::Window`].
    pub fn take_window(&mut self) -> Option<ViewWindow> {
        // Not while the jump scroll runs: nothing is fetched on the way,
        // and landing reports the bottom, following. Nor while the thumb
        // is dragged: contents load where it stops.
        if self.jump.is_some() || self.thumb.is_some() || self.window == self.window_taken {
            return None;
        }
        self.window_taken = self.window;
        self.window
    }

    /// Works out the window on the ribbon: the indexes it shows, those
    /// within `KEEP_SCREENS` screens of it, and those within
    /// `FORGET_SCREENS`.
    fn note_window(&mut self) {
        if self.slots.is_empty() {
            self.window = None;
            return;
        }
        let screen = self.frame.viewport.h.max(1.0);
        let span = |lo: f32, hi: f32| {
            let a = self.slot_at(lo).unwrap_or(0);
            let b = self.slot_at(hi).unwrap_or(0).max(a);
            (self.slots[a].index, self.slots[b].index)
        };
        let top = self.offset;
        let bottom = top + screen;
        self.window = Some(ViewWindow {
            shown: span(top, bottom),
            close: span(top - screen, bottom + screen),
            near: span(top - KEEP_SCREENS * screen, bottom + KEEP_SCREENS * screen),
            keep: span(
                top - FORGET_SCREENS * screen,
                bottom + FORGET_SCREENS * screen,
            ),
            scrolled_back: !self.follow,
        });
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
        // Last: a block that folds, where nothing else in it was hit, as
        // much of it as is in the window.
        for &(hit, r) in &self.flow.taps {
            let top = (v.y + r.y - self.offset).max(v.y);
            let bottom = (v.y + r.y + r.h - self.offset).min(v.y + v.h);
            if bottom > top {
                self.hits
                    .push((hit, Rect::new(r.x, top, r.w, bottom - top)));
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
        self.follow = self.offset >= max - 1.0;
        self.placed();
    }

    /// Places everything for the offset as it is.
    fn placed(&mut self) {
        if self.rows_short() {
            self.assemble();
        }
        self.frame = self.arrange(self.compact());
        self.place_hits();
        self.check_earlier();
        self.note_window();
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
        self.jump = None;
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
                if self.thumb.is_some() {
                    // Another finger while the thumb is held: ignored.
                    return None;
                }
                if let Some(d) = self.drag.as_mut() {
                    // A second finger joins the drag (the 11:09 EDT
                    // amendment, item 1): nothing resets or jumps. The same
                    // id again is a finger whose lift was lost: start over.
                    if !d.fingers.iter().any(|f| f.id == id) {
                        d.fingers.push(Finger {
                            id,
                            start: [x, y],
                            y,
                        });
                        d.multi = true;
                        log::info!("agent view: {} fingers down", d.fingers.len());
                        return None;
                    }
                }
                // A touch stops the jump scroll, as it catches a fling.
                if self.jump.take().is_some() {
                    log::info!("agent view: jump scroll stopped at {:.0}", self.offset);
                }
                // The band is a scrollbar, drawn or not (the band
                // amendment): on the thumb it is grabbed where touched;
                // elsewhere the thumb comes to the finger, centred.
                if self.band().is_some_and(|r| r.contains([x, y])) {
                    let (r, _) = self.thumb_track()?;
                    let on_thumb = y >= r.y && y <= r.y + r.h;
                    self.fling = 0.0;
                    self.drag = None;
                    self.thumb = Some(ThumbDrag {
                        id,
                        grab: if on_thumb { y - r.y } else { r.h / 2.0 },
                        from: self.offset,
                    });
                    self.bar = Some(0.0);
                    self.labels = Some(0.0);
                    if !on_thumb {
                        self.drag_thumb(y);
                    }
                    self.placed();
                    return None;
                }
                let carried = std::mem::take(&mut self.fling);
                self.drag = Some(Drag {
                    fingers: vec![Finger {
                        id,
                        start: [x, y],
                        y,
                    }],
                    pos: 0.0,
                    carried,
                    down: now,
                    last_t: now,
                    moved: false,
                    samples: [(now, 0.0); SAMPLES],
                    sampled: 1,
                    multi: false,
                    pair_lifted: None,
                });
                None
            }
            TouchPhase::Moved => {
                if let Some(t) = self.thumb {
                    if t.id == id {
                        self.drag_thumb(y);
                    }
                    return None;
                }
                let d = self.drag.as_mut()?;
                let n = d.fingers.len();
                let f = d.fingers.iter_mut().find(|f| f.id == id)?;
                let dy = y - f.y;
                f.y = y;
                if !d.moved
                    && ((y - f.start[1]).abs() > SLOP * s || (x - f.start[0]).abs() > SLOP * s)
                {
                    d.moved = true;
                }
                // Each finger's move counts its share of the mean: the
                // fingers moving together move it by their mean movement.
                let mean = dy / n as f32;
                d.pos += mean;
                let pos = d.pos;
                d.sample(now, pos);
                d.last_t = now;
                let moved = d.moved;
                if moved {
                    let factor = if n >= 2 { TWO_FINGER } else { 1.0 };
                    self.bar = Some(0.0);
                    // One finger past the end pulls to refresh.
                    if n >= 2 || !self.pull_by(factor * mean) {
                        self.scroll_to(self.offset - factor * mean);
                    }
                }
                None
            }
            TouchPhase::Ended => {
                if let Some(t) = self.thumb {
                    if t.id == id {
                        // Letting go leaves the view there: no fling. The
                        // scrollbar's 2 s start now.
                        self.thumb = None;
                        self.bar = Some(0.0);
                        log::info!(
                            "agent view: scrollbar drag from {:.0} to {:.0}",
                            t.from,
                            self.offset
                        );
                        self.scroll_to(self.offset);
                    }
                    return None;
                }
                let d = self.drag.as_mut()?;
                let i = d.fingers.iter().position(|f| f.id == id)?;
                if d.fingers.len() > 1 {
                    // One finger of several lifts: the others carry on, at
                    // 1x once only one is left, from where the view is.
                    d.fingers.remove(i);
                    if d.fingers.len() == 1 {
                        d.pair_lifted = Some(now);
                    }
                    return None;
                }
                let d = self.drag.take()?;
                if d.moved && self.pull > 0.0 && self.pull_state == PullState::Idle {
                    // A pull let go: no fling.
                    self.release_pull();
                    return None;
                }
                if d.moved {
                    // A finger that stopped before lifting does not fling.
                    let still = now.saturating_sub(d.last_t) > Duration::from_millis(100);
                    let mut velocity = fling_curve(release_speed(&d, s), s);
                    // Two fingers lifted together fling at 5x.
                    let pair = d
                        .pair_lifted
                        .is_some_and(|t| now.saturating_sub(t) <= SPEED_WINDOW);
                    if pair {
                        velocity *= TWO_FINGER;
                    }
                    let quick = now.saturating_sub(d.down) <= FLING_BOOST_TOUCH;
                    if quick && velocity * d.carried > 0.0 {
                        velocity += d.carried;
                    }
                    let velocity = velocity.clamp(-MAX_FLING_OUT * s, MAX_FLING_OUT * s);
                    if !still && velocity.abs() > MIN_FLING * s {
                        self.fling = velocity;
                    }
                    log::info!(
                        "agent view: drag ended at {:.0}, fling {:.0}{}",
                        self.offset,
                        self.fling,
                        if pair { " (two fingers)" } else { "" }
                    );
                    self.scroll_to(self.offset);
                    return None;
                }
                self.scroll_to(self.offset);
                if d.multi {
                    // Two fingers down is never a tap.
                    return None;
                }
                // A control the settings turn off takes no tap (plan
                // 20261024_02, decision 4).
                self.hit(x, y).filter(|&hit| crate::settings::allows(hit))
            }
            TouchPhase::Cancelled => {
                if self.pull > 0.0 && self.pull_state == PullState::Idle {
                    self.springing = true;
                }
                let thumb = self.thumb.take().is_some();
                if self.drag.take().is_some() || thumb {
                    if thumb {
                        self.bar = Some(0.0);
                    }
                    self.scroll_to(self.offset);
                }
                None
            }
        }
    }

    /// Carries a fling, or the jump scroll, on by `dt`, and the
    /// scrollbar's showing and fading. Returns whether the view moved.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let moved = self.step(dt);
        self.step_bar(dt);
        self.step_pull(dt);
        moved
    }

    /// The scrollbar: fully visible while scrolling and `BAR_HOLD` after,
    /// then fading over `BAR_FADE`; frames are asked for while it shows or
    /// fades, and once more as it goes, but not after.
    fn step_bar(&mut self, dt: Duration) {
        // The section labels show while the thumb is held, and fade with
        // it.
        if self.thumb.is_some() {
            self.labels = Some(0.0);
        } else if let Some(t) = self.labels {
            let t = t + dt.as_secs_f32();
            self.labels = (t < (BAR_HOLD + BAR_FADE).as_secs_f32()).then_some(t);
            self.dirty = true;
        }
        if self.scrolling() {
            self.bar = Some(0.0);
            self.dirty = true;
            return;
        }
        let Some(t) = self.bar else {
            return;
        };
        let t = t + dt.as_secs_f32();
        self.bar = (t < (BAR_HOLD + BAR_FADE).as_secs_f32()).then_some(t);
        self.dirty = true;
    }

    fn step(&mut self, dt: Duration) -> bool {
        if self.placeholders_shown {
            self.shimmer = (self.shimmer + dt.as_secs_f32()) % SHIMMER_PERIOD;
            self.dirty = true;
        }
        let was = self.quiet_for >= QUIET_FOR_CONTINUE;
        self.quiet_for += dt.as_secs_f32();
        if !was && self.quiet_for >= QUIET_FOR_CONTINUE {
            // The suggestion row may appear.
            self.settle();
        }
        if self.jump.is_some() {
            return self.step_jump(dt);
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
        fill(list, Rect::new(0.0, 0.0, l.w, l.h), theme().bg, 0.0);

        // The flow, in its window, pulled up by a pull to refresh.
        let v = self.frame.viewport;
        list.push_clip(v.pos(), v.size());
        let dy = v.y - self.offset - self.pull;
        let visible = |y: f32, h: f32| y + h + dy >= v.y && y + dy <= v.y + v.h;
        let mut placeholders = false;
        for f in &self.flow.fills {
            if visible(f.rect.y, f.rect.h) {
                let r = Rect::new(f.rect.x, f.rect.y + dy, f.rect.w, f.rect.h);
                fill(list, r, f.color, f.radius);
                if f.color == theme().placeholder {
                    placeholders = true;
                    for band in shimmer_over(r, self.shimmer, l.w) {
                        fill(list, band, theme().shimmer, f.radius.min(band.w / 2.0));
                    }
                }
            }
        }
        self.placeholders_shown = placeholders;
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
                theme().dim,
            );
        }
        // The pull's line, dim, centred in the space below the last
        // section.
        if let Some((text, color)) = self.pull_line() {
            let px = TOOL_PT * s;
            let w = text.chars().count() as f32 * px * 0.5;
            let bottom = v.y + self.ribbon_height() - self.offset - self.pull;
            let y = bottom + (self.pull - line_h(px)) * 0.5;
            let x = ((l.w - w) * 0.5).max(PAD * s);
            ctx.draw_text(list, &text, [x, y], px, color);
        }
        // The scrollbar's thumb, while it shows.
        if let Some(r) = self.thumb_rect() {
            let color = with_alpha(theme().fg, self.bar_alpha());
            fill(list, r, color, r.w / 2.0);
        }
        // The section labels over the scrollback, in the thumb's grays.
        let alpha = self.labels_alpha();
        if alpha > 0.0 {
            let px = TOOL_PT * s;
            for label in self.section_labels() {
                let (bg, fg) = if label.nearest {
                    (LABEL_NEAR_BG, LABEL_NEAR_TEXT)
                } else {
                    (LABEL_BG, LABEL_TEXT)
                };
                let r = label.rect;
                fill(list, r, with_alpha(theme().fg, bg * alpha), r.h / 2.0);
                let w = label.text.chars().count() as f32 * px * 0.5;
                let pos = [r.x + (r.w - w) * 0.5, r.y + (r.h - line_h(px)) * 0.5];
                ctx.draw_text(
                    list,
                    &label.text,
                    pos,
                    px,
                    with_alpha(theme().fg, fg * alpha),
                );
            }
        }
        list.push_clip_end();

        // The header: the agent and its status, over a band in its
        // session's colour.
        fill(list, self.frame.header, theme().header, 0.0);
        if let Some((band, colour)) = self.session_band() {
            fill(list, band, colour, 0.0);
        }
        let name_px = NAME_PT * s;
        let top = l.top + 8.0 * s;
        let back = Row {
            text: if term.is_some() { BACK_GLYPH } else { "<" }.to_string(),
            x: 0.0,
            y: 0.0,
            px: 1.6 * name_px,
            color: theme().fg,
            mono: true,
        };
        let back_y = l.top + (HEADER_H * s - term_h(back.px)) * 0.5;
        draw_row(list, ctx, term, &back, [PAD * s, back_y]);
        let name_x = BACK_W * s;
        ctx.draw_text(list, &self.name, [name_x, top], name_px, theme().fg);
        ctx.draw_text(
            list,
            &self.status,
            [name_x, top + line_h(name_px)],
            TOOL_PT * s,
            theme().dim,
        );
        if let Some(on) = &self.terminal_on {
            // The view does not follow herdr's focus: say where it is.
            ctx.draw_text(
                list,
                &format!("terminal on: {on}"),
                [name_x, l.top + HEADER_H * s],
                TOOL_PT * s,
                theme().dim,
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
            let start = list.instances.len();
            for f in &d.fills {
                fill(list, f.rect, f.color, f.radius);
            }
            for r in &d.rows {
                ctx.draw_text(list, &r.text, [r.x, r.y], r.px, r.color);
            }
            // Without the microphone, the dictation card is dim.
            if !crate::settings::current().microphone {
                crate::settings::dim_since(list, start);
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
            ctx.draw_text(list, &j.text, [x, y], px, j.color);
        }
        let sg_px = SUGGEST_PT * s;
        for g in &self.frame.suggestions {
            let start = list.instances.len();
            let y = g.rect.y + (g.rect.h - line_h(sg_px)) * 0.5;
            ctx.draw_text(list, g.label, [g.rect.x + 4.0 * s, y], sg_px, g.color);
            if !crate::settings::allows(g.hit) {
                crate::settings::dim_since(list, start);
            }
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
        .with_text_color(theme().fg)
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
            text: None,
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
            points: Vec::new(),
            spoken: None,
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
            url: "http://w/#/src--flankr/docs/design/20261023_04_OVERVIEW_SCREEN_PLAN".to_string(),
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
        let bands = flow
            .fills
            .iter()
            .filter(|f| f.color == theme().band)
            .count();
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
            assert_eq!(rows[0].color, theme().dim);
            assert_eq!(rows[0].px, TOOL_PT * 2.0);
            // The dot before it, amber, on the same line.
            let dot = flow
                .rows
                .iter()
                .find(|r| r.text == notice_line::DOT && r.y == rows[0].y)
                .expect("a dot");
            assert_eq!(dot.color, theme().accent);
            assert!(dot.x < rows[0].x);
        }
        // Whole, never cut (plan 20261022_03, the L3 amendment of
        // 2026-10-24 11:54 EDT).
        assert_eq!(
            row("docs-writer: ")[0].text,
            "docs-writer: Rebased on main. Nothing else."
        );
        let report: Vec<&Row> = flow
            .rows
            .iter()
            .filter(|r| r.text.contains("long report") || r.text.contains("goes on"))
            .collect();
        assert!(report.len() > 2);
        assert!(report[0].text.starts_with("subagent: "));
        assert!(!report.iter().any(|r| r.text.ends_with('\u{2026}')));
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
        let band = flow
            .fills
            .iter()
            .find(|f| f.color == theme().band)
            .expect("a band");
        assert_eq!((band.rect.x, band.rect.w), (0.0, 1080.0));
        assert!(band.rect.y <= flow.entry_y[0] && band.rect.y + band.rect.h > flow.entry_y[0]);
        assert_eq!(colour("please fix it"), theme().fg);
        // A tool call is one dim line.
        let call: Vec<&Row> = flow
            .rows
            .iter()
            .filter(|r| r.text.starts_with("> Edit"))
            .collect();
        assert_eq!(call.len(), 1);
        assert_eq!(call[0].color, theme().dim);
        assert!(!flow.rows.iter().any(|r| r.text.trim() == "more"));
        // A diff result: green and red.
        assert_eq!(colour("-old"), theme().red);
        assert_eq!(colour("+new"), theme().green);
        // An error, red.
        assert_eq!(colour("! exit status 1"), theme().red);
        // In a reply, diff lines only inside a fence; a bullet stays plain.
        assert_eq!(colour("-a"), theme().red);
        assert_eq!(colour("+b"), theme().green);
        assert_eq!(colour("- a bullet"), theme().fg);
        assert_eq!(colour("```diff"), theme().dim);
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
        assert_eq!(faded.color, theme().dim);
        // The live one: a shadow, a panel and its accent edge, larger text.
        assert!(flow.fills.iter().any(|f| f.color == theme().card));
        assert!(flow.fills.iter().any(|f| f.color == theme().shadow));
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
                color: theme().fg,
                gap: 4.0,
            },
            Line {
                text: "please go on".into(),
                size: 20.0,
                color: theme().fg,
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
            color: theme().fg,
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
    fn switching_to_light_leaves_no_dark_gray_colour_drawn() {
        use crate::theme::testing::{dark_gray_only, draw_context, use_theme};
        use_theme("dark_gray");
        let ctx = draw_context();
        let cards = vec![
            card(CardKind::Question, 2, false),
            card(CardKind::Approval, 4, true),
        ];
        let mut a = screen(conversation(4), cards, Some(1));
        a.set_scrollbar(ScrollbarVisibility::Always);
        let mut list = DrawList::new();
        assert!(a.draw(&mut list, &ctx, None));
        assert!(!dark_gray_only(&list).is_empty(), "drawn in Dark (gray)");

        use_theme("light_black");
        a.restyle();
        assert!(a.draw(&mut list, &ctx, None), "drawn again");
        assert_eq!(dark_gray_only(&list), Vec::<[f32; 4]>::new());
        use_theme("dark_gray");
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
        assert_eq!(f.buttons.len(), 3);
        // Play at the bottom left, the message button at the bottom right,
        // the keyboard glyph just left of it.
        let play = f.buttons[0].rect;
        assert_eq!(f.buttons[0].hit, AgentHit::ReadAloud);
        assert!((play.x - PAD * PHONE.scale).abs() < 0.5);
        let keys = &f.buttons[1];
        let mic = f.buttons[2].rect;
        assert_eq!(f.buttons[2].hit, AgentHit::Message);
        assert!((mic.y - play.y).abs() < 0.5);
        assert!((mic.x + mic.w - (PHONE.w - PAD * PHONE.scale)).abs() < 0.5);
        assert!(mic.y + mic.h <= PHONE.h - PHONE.bottom);
        assert_eq!(keys.hit, AgentHit::Keyboard);
        assert_eq!(
            (keys.rect.w, keys.rect.h),
            (mic.w, mic.h),
            "record's hit box"
        );
        assert!(keys.rect.x + keys.rect.w < mic.x, "left of record");
        assert_eq!(keys.bg[3], 0.0, "no background");
        // Live with text input.
        assert!(crate::settings::allows(AgentHit::Keyboard));
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
        assert_eq!((row[0].label, row[0].color), ("cont", theme().yellow));
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
        assert_eq!(suggestion_colour(Suggestion::Yes), theme().green);
        assert_eq!(suggestion_colour(Suggestion::No), theme().red);
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
            color: theme().fg,
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
        assert_eq!(
            hits,
            [AgentHit::ReadAloud, AgentHit::Keyboard, AgentHit::Message]
        );
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
        assert_eq!(row.color, theme().accent);
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
            theme().fg,
            1.0,
        );
        button(
            &mut list,
            &ctx,
            "Yes",
            Rect::new(0.0, 0.0, 200.0, 40.0),
            theme().fg,
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
    fn the_header_has_a_band_in_the_sessions_colour() {
        crate::theme::testing::use_theme("dark_gray");
        let mut a = screen(conversation(3), vec![], None);
        assert!(a.session_band().is_none(), "none until its colour is known");
        a.set_session_colour(Some(9));
        let (band, colour) = a.session_band().expect("a band");
        assert_eq!(colour, theme().session(9));
        let h = a.frame().header;
        assert_eq!((band.x, band.w), (h.x, h.w));
        assert_eq!(band.y + band.h, h.y + h.h, "at the header's foot");
        // Drawn.
        let mut list = DrawList::new();
        a.draw(&mut list, &crate::theme::testing::draw_context(), None);
        assert!(list.instances.iter().any(|i| i.color == colour));
    }

    #[test]
    fn other_sessions_cards_sit_above_the_pinned_card_and_the_flow_makes_room() {
        let entries = conversation(20);
        let n = entries.len();
        let mut a = screen(entries, vec![card(CardKind::Approval, n, true)], Some(0));
        let before = a.frame().clone();
        let pin = before.pin.clone().expect("pinned");
        assert_eq!(a.cross_bottom(), pin.rect.y - 8.0 * 2.0);
        a.set_cross_height(120.0);
        let after = a.frame().clone();
        assert_eq!(after.pin.as_ref().unwrap().rect, pin.rect, "the pin stays");
        assert_eq!(a.cross_bottom(), pin.rect.y - 8.0 * 2.0);
        assert!(
            after.viewport.y + after.viewport.h <= a.cross_bottom() - 120.0,
            "the flow ends above the card"
        );
        assert!(
            (before.viewport.h - after.viewport.h - (120.0 + 16.0)).abs() < 0.5,
            "by its height and a gap"
        );
        // Without a pinned card, above what is under the flow.
        let mut b = screen(conversation(3), vec![], None);
        b.set_cross_height(80.0);
        let f = b.frame();
        assert!(b.cross_bottom() <= f.buttons[0].rect.y);
        assert!(f.viewport.y + f.viewport.h <= b.cross_bottom() - 80.0);
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

    // Plan 20261022_03, the 00:05 EDT amendment: the jump line, and the
    // 01:07 EDT amendment: it says what is at the end.

    fn jump(a: &AgentScreen) -> (String, [f32; 4]) {
        let j = a.frame().jump.clone().expect("shown");
        (j.text, j.color)
    }

    fn live_card(kind: CardKind, at: usize, text: &str) -> AgentCard {
        AgentCard {
            text: text.to_string(),
            ..card(kind, at, true)
        }
    }

    #[test]
    fn the_jump_line_shows_scrolled_back_and_says_jump_to_end_when_nothing_is_new() {
        let all = conversation(30);
        let mut a = screen(all[..80].to_vec(), vec![], None);
        assert!(a.frame().jump.is_none(), "hidden while following");
        scrolled_back(&mut a);
        assert_eq!(jump(&a), (JUMP_TO_END.to_string(), theme().dim));
        // New sections with no state to tell: still jump to end.
        flat(&mut a, "idle", 0, all[..84].to_vec(), vec![], None);
        assert_eq!(jump(&a), (JUMP_TO_END.to_string(), theme().dim));
        // Tappable.
        let r = a.rect_of(AgentHit::JumpToEnd).expect("tappable");
        assert_eq!(
            a.hit(r.x + r.w * 0.5, r.y + r.h * 0.5),
            Some(AgentHit::JumpToEnd)
        );
        a.jump_to_end();
        run_jump(&mut a);
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
        assert!(a.frame().jump.is_none());
    }

    #[test]
    fn the_jump_line_says_each_state_in_its_colour() {
        let all = conversation(30);
        let entries = all[..80].to_vec();
        let mut a = screen(entries.clone(), vec![], None);
        scrolled_back(&mut a);
        let mut with = |status: &str, cards: Vec<AgentCard>| {
            flat(&mut a, status, 0, entries.clone(), cards, None);
            jump(&a)
        };
        assert_eq!(
            with(
                "idle",
                vec![live_card(CardKind::Question, 79, "Which one?\nA or B.")]
            ),
            (
                "-- question: Which one? --".to_string(),
                accent(CardKind::Question)
            )
        );
        assert_eq!(
            with(
                "idle",
                vec![live_card(CardKind::Approval, 79, "cargo test")]
            ),
            (
                "-- approval: cargo test --".to_string(),
                accent(CardKind::Approval)
            )
        );
        assert_eq!(
            with(
                "idle",
                vec![live_card(
                    CardKind::Broke,
                    79,
                    "error[E0308]: mismatched types\nmore"
                )]
            ),
            (
                "-- broke: error[E0308]: mismatched types --".to_string(),
                theme().red
            )
        );
        assert_eq!(
            with(
                "idle",
                vec![live_card(CardKind::Finished, 79, "All tests pass.")]
            ),
            ("-- done: All tests pass. --".to_string(), theme().green)
        );
        // Working: the newest section's first line.
        assert_eq!(
            with("working", vec![]),
            ("-- working: Answer 19. --".to_string(), theme().dim)
        );
        // A resolved card is no state.
        assert_eq!(
            with("idle", vec![card(CardKind::Question, 79, false)]),
            (JUMP_TO_END.to_string(), theme().dim)
        );
    }

    #[test]
    fn the_first_state_that_holds_wins_and_a_long_line_is_cut() {
        let all = conversation(30);
        let entries = all[..80].to_vec();
        let mut a = screen(entries.clone(), vec![], None);
        scrolled_back(&mut a);
        let mut with = |status: &str, kinds: &[CardKind]| {
            let cards = kinds
                .iter()
                .map(|&k| live_card(k, 79, &format!("{k:?} text")))
                .collect();
            flat(&mut a, status, 0, entries.clone(), cards, None);
            jump(&a).0
        };
        use CardKind::*;
        assert_eq!(
            with("working", &[Finished, Broke, Approval, Question]),
            "-- question: Question text --"
        );
        assert_eq!(
            with("working", &[Finished, Broke, Approval]),
            "-- approval: Approval text --"
        );
        assert_eq!(
            with("working", &[Finished, Broke]),
            "-- broke: Broke text --"
        );
        assert_eq!(with("idle", &[Finished]), "-- done: Finished text --");
        // The turn is not finished while the agent works again.
        assert_eq!(with("working", &[Finished]), "-- working: Answer 19. --");
        // A long line is cut to fit with an ellipsis.
        let long = "A very long question. ".repeat(20);
        flat(
            &mut a,
            "idle",
            0,
            entries.clone(),
            vec![live_card(Question, 79, &long)],
            None,
        );
        let (text, _) = jump(&a);
        assert!(
            text.starts_with("-- question: A very long question."),
            "{text}"
        );
        assert!(text.ends_with("\u{2026} --"), "{text}");
        assert!(
            text.chars().count()
                <= columns(PHONE.w - 2.0 * PAD * PHONE.scale, TOOL_PT * PHONE.scale)
        );
    }

    #[test]
    fn the_end_of_an_outline_held_aside_gives_the_state() {
        let mut a = outline_screen(300, 290..300);
        scrolled_back(&mut a);
        a.set_new_content(true);
        assert_eq!(jump(&a), (JUMP_TO_END.to_string(), theme().dim));
        a.set_held_end(vec![ViewSection {
            index: 300,
            serial: Serial::new(FILE, 301, 0),
            shows: Shows::Card(live_card(CardKind::Question, 0, "Go on?")),
        }]);
        assert_eq!(
            jump(&a),
            (
                "-- question: Go on? --".to_string(),
                accent(CardKind::Question)
            )
        );
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
        run_jump(&mut a);
        assert_eq!(a.frame().viewport, view);
        assert!(a.offset() > offset);
    }

    // Plan 20261022_03, the 01:06 EDT amendment: the jump line scrolls to
    // the end in 2 s on an ease-in-out curve.

    const FRAME: Duration = Duration::from_millis(20);

    /// Ticks 20 ms frames until the jump scroll lands; returns the offset
    /// after each frame.
    fn run_jump(a: &mut AgentScreen) -> Vec<f32> {
        let mut offsets = Vec::new();
        while a.jumping() {
            a.tick(FRAME);
            offsets.push(a.offset());
            assert!(offsets.len() <= 200, "runs away");
        }
        offsets
    }

    /// An outline screen scrolled back `screens` screens from the end.
    fn back_by(screens: f32) -> AgentScreen {
        let mut a = outline_screen(8000, 7990..8000);
        let screen = a.frame().viewport.h;
        scrolled_back(&mut a);
        a.scroll_to(a.max_offset() - screens * screen);
        assert!(!a.following());
        assert!(
            a.max_offset() - a.offset() >= screens * screen - 1.0,
            "the ribbon is long enough"
        );
        a
    }

    #[test]
    fn the_ease_is_cubic_in_out() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(0.5), 0.5);
        assert_eq!(ease_in_out(1.0), 1.0);
        assert!((ease_in_out(0.25) - 4.0 * 0.25f32.powi(3)).abs() < 1e-6);
        assert!((ease_in_out(0.75) - (1.0 - 0.5f32.powi(3) / 2.0)).abs() < 1e-6);
    }

    #[test]
    fn the_jump_scroll_takes_2_s_from_one_screen_back_and_from_300() {
        for screens in [1.0, 300.0] {
            let mut a = back_by(screens);
            a.jump_to_end();
            assert!(a.jumping());
            let offsets = run_jump(&mut a);
            // 100 frames of 20 ms: 2 s, whatever the distance.
            assert_eq!(offsets.len(), 100, "{screens} screen(s)");
            assert!(a.following());
            assert_eq!(a.offset(), a.max_offset());
            // Still on its way a frame before, not following; at 1.8 s
            // visibly short of the end.
            let mut b = back_by(screens);
            b.jump_to_end();
            for _ in 0..99 {
                b.tick(FRAME);
            }
            assert!(b.jumping() && !b.following());
            let mut c = back_by(screens);
            c.jump_to_end();
            for _ in 0..90 {
                c.tick(FRAME);
            }
            assert!(c.max_offset() - c.offset() > 1.0);
        }
    }

    #[test]
    fn the_jump_scroll_is_still_at_both_ends_and_fastest_midway() {
        let mut a = back_by(300.0);
        let start = a.offset();
        a.jump_to_end();
        let mut offsets = vec![start];
        offsets.extend(run_jump(&mut a));
        let speeds: Vec<f32> = offsets.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(speeds.iter().all(|&v| v >= 0.0), "always toward the end");
        let top = speeds.iter().cloned().fold(0.0, f32::max);
        assert!(speeds[0] < top * 0.01, "{} of {top}", speeds[0]);
        assert!(speeds[99] < top * 0.01, "{} of {top}", speeds[99]);
        let fastest = speeds.iter().position(|&v| v == top).unwrap();
        assert!((48..=51).contains(&fastest), "{fastest}");
        // Speeding up through the first second, slowing through the second.
        assert!(speeds[..49].windows(2).all(|w| w[1] >= w[0]));
        assert!(speeds[51..].windows(2).all(|w| w[1] <= w[0]));
    }

    #[test]
    fn an_end_that_grows_during_the_jump_scroll_is_reached_at_2_s() {
        let mut a = outline_screen(300, 290..300);
        scrolled_back(&mut a);
        a.jump_to_end();
        for _ in 0..50 {
            a.tick(FRAME);
        }
        let end_before = a.max_offset();
        // Twenty more sections arrive at the end.
        give(&mut a, "idle", outline_sections(320, 290..320), None);
        assert!(a.max_offset() > end_before + 100.0);
        for _ in 0..49 {
            a.tick(FRAME);
        }
        assert!(a.jumping() && a.offset() < a.max_offset());
        a.tick(FRAME);
        assert!(!a.jumping());
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
        assert_eq!(a.slots().len(), 320);
    }

    #[test]
    fn a_touch_stops_the_jump_scroll() {
        let mut a = back_by(10.0);
        a.jump_to_end();
        for _ in 0..30 {
            a.tick(FRAME);
        }
        a.touch(
            1,
            TouchPhase::Started,
            540.0,
            1000.0,
            Duration::from_secs(10),
        );
        assert!(!a.jumping());
        let at = a.offset();
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1000.0,
            Duration::from_millis(10_050),
        );
        for _ in 0..100 {
            a.tick(FRAME);
        }
        assert_eq!(a.offset(), at, "it stays where the finger caught it");
        assert!(!a.following());
    }

    #[test]
    fn the_jump_scroll_lands_following_and_reports_no_window_on_the_way() {
        let mut a = back_by(10.0);
        a.take_window();
        a.jump_to_end();
        for _ in 0..99 {
            a.tick(FRAME);
            assert_eq!(a.take_window(), None, "no contents asked for on the way");
        }
        a.tick(FRAME);
        let w = a.take_window().expect("landing reports the bottom");
        assert!(!w.scrolled_back);
        assert!(a.following());
        // Following: new sections keep it at the bottom.
        give(&mut a, "idle", outline_sections(8010, 7990..8010), None);
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
    }

    // Plan 20261024_01, step 5, and CHANGES.md 2026-10-24 00:36 EDT.

    /// The slot of index `i`, on line `i + 1`: its contents if `held`,
    /// else pending.
    fn outline_section(i: u64, held: bool) -> ViewSection {
        let serial = Serial::new(FILE, i as u32 + 1, 0);
        let shows = if held {
            Shows::Entries(vec![entry(
                Role::Assistant,
                EntryKind::Text,
                &format!(
                    "Reply {i}.\n\nWith a second paragraph long enough to wrap at the phone's width, and then some more words to wrap again."
                ),
            )])
        } else {
            Shows::Pending {
                kind: SectionKind::Text,
                lines: 2,
                chars: 40,
            }
        };
        ViewSection {
            index: i,
            serial,
            shows,
        }
    }

    /// `n` slots, those in `held` with contents.
    fn outline_sections(n: u64, held: std::ops::Range<u64>) -> Vec<ViewSection> {
        (0..n)
            .map(|i| outline_section(i, held.contains(&i)))
            .collect()
    }

    fn outline_screen(n: u64, held: std::ops::Range<u64>) -> AgentScreen {
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        give(&mut a, "idle", outline_sections(n, held), None);
        a
    }

    #[test]
    fn the_ribbon_is_as_long_as_the_outline_before_any_contents() {
        let a = outline_screen(300, 0..0);
        let s = PHONE.scale;
        let each = estimate_height(SectionKind::Text, 2, 40, PHONE.w, s);
        assert!(each > 0.0);
        assert_eq!(a.slots().len(), 300);
        assert!(
            a.slots()
                .iter()
                .all(|slot| (slot.height - each).abs() < 0.01)
        );
        let expected = ribbon_top(s) + 300.0 * each + 16.0 * s;
        assert!((a.flow().height - expected).abs() < 0.5);
        // Following, at the bottom of the whole session.
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
        // Longer text is estimated taller: wrapped at the text's columns.
        let wide = estimate_height(SectionKind::Text, 1, 2000, PHONE.w, s);
        let cols = Pen::new(PHONE.w, s).cols(TEXT_PT) as f32;
        let rows = (2000.0 / cols).ceil();
        assert!((wide - (10.0 * s + rows * term_h(TEXT_PT * s))).abs() < 0.01);
        // A tool call is at most its two lines; a turn has its band.
        assert!(
            estimate_height(SectionKind::Tool, 40, 4000, PHONE.w, s)
                < estimate_height(SectionKind::Text, 40, 4000, PHONE.w, s)
        );
        assert!(
            estimate_height(SectionKind::Turn, 2, 40, PHONE.w, s)
                > estimate_height(SectionKind::Text, 2, 40, PHONE.w, s)
        );
    }

    #[test]
    fn contents_replacing_estimates_do_not_move_the_view() {
        let mut a = outline_screen(300, 290..300);
        scrolled_back(&mut a);
        let (serial, delta) = a.place().unwrap();
        let at = on_screen(&a, serial);
        let top = a.slot_of(serial.to_u64()).unwrap() as u64;
        let height_before = a.flow().height;
        // The contents around the window arrive, the top one's among them:
        // the real heights differ from the estimates.
        let near = top.saturating_sub(40)..(top + 40).min(300);
        give(&mut a, "idle", outline_sections(300, near.start..300), None);
        assert!(
            (a.flow().height - height_before).abs() > 1.0,
            "heights changed"
        );
        let (now, now_delta) = a.place().unwrap();
        assert_eq!(now, serial);
        assert!((now_delta - delta).abs() < 0.01);
        assert!((on_screen(&a, serial) - at).abs() < 0.01);
        assert!(!a.following());
        // Forgotten again (far away now): back to estimates, still held.
        give(&mut a, "idle", outline_sections(300, 290..300), None);
        assert_eq!(a.place().unwrap().0, serial);
        assert!((on_screen(&a, serial) - at).abs() < 0.01);
    }

    #[test]
    fn a_slot_without_contents_is_faint_bars_not_text() {
        let a = outline_screen(40, 30..40);
        let flow = a.flow();
        let bars = flow
            .fills
            .iter()
            .filter(|f| f.color == theme().placeholder)
            .count();
        assert!(bars > 0, "placeholders draw bars");
        assert!(
            !flow.rows.iter().any(|r| r.text == "\u{2026}"),
            "and no text"
        );
        assert!(flow.rows.iter().any(|r| r.text.starts_with("Reply 35.")));
        // Each placeholder keeps its slot's estimated height.
        let i = a.slot_of(Serial::new(FILE, 1, 0).to_u64()).unwrap();
        let est = estimate_height(SectionKind::Text, 2, 40, PHONE.w, PHONE.scale);
        assert!((a.slots()[i].height - est).abs() < 0.01);
        // A text section's bars: one per line, the last one shorter.
        let mut f = Flow::default();
        let pen = Pen::new(PHONE.w, PHONE.scale);
        lay_out_pending(SectionKind::Text, 3, 0, pen, &mut f);
        assert_eq!(f.fills.len(), 3);
        assert!(f.fills[2].rect.w < f.fills[0].rect.w);
        assert!(f.fills.iter().all(|b| b.color[3] < 0.1), "almost clear");
        // A card's: one panel.
        let mut c = Flow::default();
        lay_out_pending(SectionKind::Card, 1, 10, pen, &mut c);
        assert_eq!(c.fills.len(), 1);
        // A placeholder is not news for the jump line.
        assert_eq!(first_line(&outline_section(0, false).shows), None);
    }

    #[test]
    fn the_shimmer_sweeps_every_1_6_s_and_runs_only_over_placeholders() {
        let bar = Rect::new(0.0, 0.0, 1000.0, 10.0);
        let at = |t: f32| shimmer_over(bar, t, 1000.0).first().map(|r| r.x);
        let (a, b) = (at(0.6), at(0.8));
        assert!(a.unwrap() < b.unwrap(), "it moves left to right");
        assert_eq!(at(0.6), at(0.6 + SHIMMER_PERIOD), "and wraps");
        assert!(at(0.0).is_none(), "it starts off the left edge");
        // With nothing pending on screen, no frames are asked for.
        let mut full = screen(conversation(10), vec![], None);
        full.dirty = false;
        assert!(!full.placeholders_shown);
        assert!(!full.tick(Duration::from_millis(16)) && !full.dirty);
    }

    #[test]
    fn the_marker_row_says_start_of_session_at_index_0() {
        let mut a = outline_screen(300, 290..300);
        // The client says the outline is held from index 0.
        a.set_history(false, true);
        assert_eq!(a.flow().rows[0].text, START_OF_SESSION);
        // Scrolled to the very top, it shows above index 0's block.
        a.scroll_to(0.0);
        assert_eq!(a.slots()[0].index, 0);
        assert!(
            a.tops[0] - a.offset() < a.frame().viewport.h,
            "index 0 in the window"
        );
        let marker = &a.flow().rows[0];
        assert!(marker.y < a.tops[0]);
    }

    #[test]
    fn the_window_is_reported_by_index_and_scrolled_back() {
        let mut a = outline_screen(300, 290..300);
        let w = a.take_window().expect("a window");
        assert!(!w.scrolled_back);
        assert_eq!(w.shown.1, 299);
        assert!(w.near.0 < w.shown.0 && w.keep.0 <= w.near.0);
        assert_eq!(a.take_window(), None, "unchanged: not again");
        scrolled_back(&mut a);
        let w = a.take_window().expect("moved");
        assert!(w.scrolled_back);
        let (serial, _) = a.place().unwrap();
        let top = a.slots()[a.slot_of(serial.to_u64()).unwrap()].index;
        assert_eq!(w.shown.0, top);
        assert!(w.shown.1 < 299);
    }

    #[test]
    fn with_a_new_outline_held_aside_the_jump_scroll_carries_on_into_its_newer_sections() {
        let mut a = outline_screen(300, 290..300);
        scrolled_back(&mut a);
        let (serial, _) = a.place().unwrap();
        let at = on_screen(&a, serial);
        // A new outline held aside: the old ribbon stays as it was.
        a.set_new_content(true);
        assert!((on_screen(&a, serial) - at).abs() < 0.01);
        // The tap: the client is to add the newer sections below.
        assert!(a.jump_to_end());
        assert!(!a.following());
        for _ in 0..10 {
            a.tick(FRAME);
        }
        // They come below the old end, at indexes after its last.
        let mut sections = outline_sections(300, 290..300);
        sections.extend((300..320).map(|i| outline_section(i, i >= 310)));
        let diff = changed(&a, &sections);
        a.set_content("Claude".into(), "idle".into(), sections, None, &diff);
        assert_eq!(a.slots().len(), 320);
        let newest = a.slots()[319].serial;
        run_jump(&mut a);
        // The scroll reached them: at the very end, following.
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
        let bottom = on_screen(&a, newest);
        let v = a.frame().viewport;
        assert!(bottom >= v.y && bottom < v.y + v.h, "the newest in view");
        // Back at the bottom, the client merges the full new outline: here
        // two old sections gone and the indexes renumbered. The view stays.
        a.set_new_content(false);
        let mut merged = outline_sections(320, 290..320);
        merged.remove(20);
        merged.remove(20);
        for (i, v) in merged.iter_mut().enumerate() {
            v.index = i as u64;
        }
        let diff = changed(&a, &merged);
        a.set_content("Claude".into(), "idle".into(), merged, None, &diff);
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
        assert!((on_screen(&a, newest) - bottom).abs() < 0.01);
        assert_eq!(a.slots().len(), 318);
    }

    #[test]
    fn a_late_cards_index_out_of_serial_order_keeps_the_place() {
        // Index 150 is a card on a line before index 140's (a Finished card
        // after context lines takes the next index).
        let mut sections = outline_sections(200, 0..200);
        sections[150].serial = Serial::new(FILE, 140, 200);
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        a.set_content("Claude".into(), "idle".into(), sections.clone(), None, &[]);
        scrolled_back(&mut a);
        let (serial, delta) = a.place().unwrap();
        let at = on_screen(&a, serial);
        // A section below the window replaced: the place holds.
        let changed = [sections[199].serial];
        sections[199] = ViewSection {
            shows: Shows::Entries(vec![entry(Role::Assistant, EntryKind::Text, "grown")]),
            ..sections[199].clone()
        };
        a.set_content("Claude".into(), "idle".into(), sections, None, &changed);
        assert_eq!(a.place().unwrap(), (serial, delta));
        assert!((on_screen(&a, serial) - at).abs() < 0.01);
    }

    /// Fingers `ids` down at `(x, from)` at `t`, then moved together by
    /// `step` pixels each, `steps` times, 16 ms apart. Returns the time.
    fn fingers_drag(
        a: &mut AgentScreen,
        ids: &[u64],
        from: f32,
        step: f32,
        steps: u32,
        t: Duration,
    ) -> Duration {
        for (k, &id) in ids.iter().enumerate() {
            a.touch(id, TouchPhase::Started, 300.0 + 300.0 * k as f32, from, t);
        }
        fingers_move(a, ids, from, step, steps, t)
    }

    /// Moves fingers `ids`, now at `from`, by `step` each, `steps` times.
    fn fingers_move(
        a: &mut AgentScreen,
        ids: &[u64],
        from: f32,
        step: f32,
        steps: u32,
        t: Duration,
    ) -> Duration {
        let mut now = t;
        for i in 1..=steps {
            now += MS;
            for (k, &id) in ids.iter().enumerate() {
                let y = from + step * i as f32;
                a.touch(id, TouchPhase::Moved, 300.0 + 300.0 * k as f32, y, now);
            }
        }
        now
    }

    /// An outline screen in the middle of its ribbon.
    fn midway() -> AgentScreen {
        let mut a = outline_screen(300, 290..300);
        scrolled_back(&mut a);
        a.scroll_to(a.max_offset() / 2.0);
        a
    }

    #[test]
    fn a_two_finger_drag_moves_5x_and_flings_at_5x_and_one_finger_goes_back_to_1x() {
        // One finger: past the slop, the view follows it.
        let mut one = midway();
        let t = fingers_drag(&mut one, &[1], 1000.0, 10.0, 5, Duration::ZERO);
        let at = one.offset();
        let t = fingers_move(&mut one, &[1], 1050.0, 10.0, 15, t);
        assert!((at - one.offset() - 150.0).abs() < 0.01);
        one.touch(1, TouchPhase::Ended, 300.0, 1200.0, t + MS);
        let one_fling = one.fling;
        assert!(one_fling > 0.0);
        // Two fingers moving together: 5x their mean movement.
        let mut two = midway();
        let t = fingers_drag(&mut two, &[1, 2], 1000.0, 10.0, 5, Duration::ZERO);
        let at = two.offset();
        let t = fingers_move(&mut two, &[1, 2], 1050.0, 10.0, 15, t);
        assert!((at - two.offset() - 750.0).abs() < 0.01);
        // Lifted together: the fling starts at 5x.
        two.touch(2, TouchPhase::Ended, 600.0, 1200.0, t + MS);
        two.touch(1, TouchPhase::Ended, 300.0, 1200.0, t + MS);
        assert!(
            (two.fling - TWO_FINGER * one_fling).abs() < 1.0,
            "{} vs {}",
            two.fling,
            one_fling
        );
        assert!(two.fling <= MAX_FLING_OUT * PHONE.scale);
        // Only one finger of two moving: 5x the mean, half its movement.
        let mut half = midway();
        let t = fingers_drag(&mut half, &[1, 2], 1000.0, 10.0, 5, Duration::ZERO);
        let at = half.offset();
        fingers_move(&mut half, &[1], 1050.0, 10.0, 10, t);
        assert!((at - half.offset() - 250.0).abs() < 0.01);
        // Lifting one finger mid-drag: no jump, then 1x from there.
        let mut lift = midway();
        let t = fingers_drag(&mut lift, &[1, 2], 1000.0, 10.0, 10, Duration::ZERO);
        let at = lift.offset();
        lift.touch(2, TouchPhase::Ended, 600.0, 1100.0, t + MS);
        assert_eq!(lift.offset(), at, "no jump on the lift");
        assert_eq!(lift.fling, 0.0, "and no fling yet");
        let t = fingers_move(&mut lift, &[1], 1100.0, 10.0, 20, t + MS);
        assert!((at - lift.offset() - 200.0).abs() < 0.01);
        // Released well after the other finger lifted: a 1x fling.
        lift.touch(1, TouchPhase::Ended, 300.0, 1300.0, t + MS);
        assert!(
            (lift.fling - one_fling).abs() < 1.0,
            "{} vs {}",
            lift.fling,
            one_fling
        );
        // A second finger arriving mid-drag neither resets nor jumps it.
        let mut join = midway();
        let t = fingers_drag(&mut join, &[1], 1000.0, 10.0, 10, Duration::ZERO);
        let at = join.offset();
        join.touch(2, TouchPhase::Started, 700.0, 400.0, t + MS);
        assert_eq!(join.offset(), at);
        assert!(
            join.drag
                .as_ref()
                .is_some_and(|d| d.moved && d.fingers.len() == 2)
        );
        // Finger 1 at 1100, finger 2 at 400: both move by 10 a step.
        let mut now = t + MS;
        for i in 1..=10 {
            now += MS;
            join.touch(1, TouchPhase::Moved, 300.0, 1100.0 + 10.0 * i as f32, now);
            join.touch(2, TouchPhase::Moved, 700.0, 400.0 + 10.0 * i as f32, now);
        }
        assert!((at - join.offset() - 500.0).abs() < 0.01);
        // Two fingers down is never a tap.
        let mut tap = outline_screen(300, 290..300);
        tap.touch(1, TouchPhase::Started, 300.0, 1000.0, Duration::ZERO);
        tap.touch(2, TouchPhase::Started, 600.0, 1000.0, MS);
        tap.touch(2, TouchPhase::Ended, 600.0, 1000.0, MS * 2);
        assert_eq!(tap.touch(1, TouchPhase::Ended, 300.0, 1000.0, MS * 3), None);
    }

    #[test]
    fn the_thumb_is_the_windows_share_and_runs_from_the_top_to_the_end() {
        let s = PHONE.scale;
        let mut a = midway();
        let total = a.calculated_length();
        assert!(
            (total - a.ribbon_height()).abs() < 0.01,
            "nothing held aside"
        );
        let range = total - a.frame().full_viewport_h;
        let len = |a: &AgentScreen| {
            let v = a.frame().viewport;
            (v.h / total * v.h).max(BAR_MIN * s)
        };
        // At the top.
        a.scroll_to(0.0);
        let (v, r) = (a.frame().viewport, a.thumb_rect().unwrap());
        assert!((r.y - v.y).abs() < 0.01);
        assert!((r.h - len(&a)).abs() < 0.01);
        assert!(r.h > BAR_MIN * s, "longer than the least here");
        assert!((r.w - BAR_W * s).abs() < 0.01);
        assert!(
            (r.x + r.w - (v.x + v.w - BAR_SLOP * s)).abs() < 0.01,
            "at the right"
        );
        // Midway.
        a.scroll_to(range / 2.0);
        let (v, r) = (a.frame().viewport, a.thumb_rect().unwrap());
        assert!((r.y - (v.y + (v.h - r.h) / 2.0)).abs() < 0.01);
        // At the end.
        a.scroll_to(a.max_offset());
        let (v, r) = (a.frame().viewport, a.thumb_rect().unwrap());
        assert!((r.y + r.h - (v.y + v.h)).abs() < 0.01);
        // A long session: the least length.
        let mut long = outline_screen(8000, 7990..8000);
        scrolled_back(&mut long);
        let r = long.thumb_rect().unwrap();
        assert!((r.h - BAR_MIN * s).abs() < 0.01);
        // A session that fits its window has none.
        let mut short = outline_screen(3, 0..3);
        short.bar = Some(0.0);
        assert_eq!(short.thumb_rect(), None);
    }

    #[test]
    fn the_scrollbars_length_counts_a_held_aside_outlines_newer_sections() {
        let s = PHONE.scale;
        let mut a = midway();
        let before = a.calculated_length();
        let short_len = a.thumb_rect().unwrap().h;
        a.set_new_content(true);
        let newer: Vec<OutlineSlot> = (300..340u64)
            .map(|i| OutlineSlot {
                index: i,
                serial: Serial::new(FILE, i as u32 + 1, 0),
                kind: SectionKind::Text,
                lines: 2,
                chars: 40,
                rev: 0,
                bookend: false,
            })
            .collect();
        a.set_held_newer(newer);
        let each = estimate_height(SectionKind::Text, 2, 40, PHONE.w, s);
        assert!((a.calculated_length() - (before + 40.0 * each)).abs() < 0.5);
        assert!(a.thumb_rect().unwrap().h < short_len, "a smaller share");
        // At the old end the thumb stops short of the bottom.
        a.scroll_to(a.max_offset() - 2.0);
        let (v, r) = (a.frame().viewport, a.thumb_rect().unwrap());
        assert!(r.y + r.h < v.y + v.h - 10.0, "short of the bottom");
        // Once below the end, the client gives none: back to the ribbon.
        a.set_held_newer(Vec::new());
        assert!((a.calculated_length() - a.ribbon_height()).abs() < 0.01);
    }

    #[test]
    fn the_scrollbar_shows_on_scrolling_stays_2_s_after_lifting_then_fades() {
        let mut a = outline_screen(300, 290..300);
        assert_eq!(a.bar_alpha(), 0.0);
        assert_eq!(a.thumb_rect(), None, "not before scrolling");
        // A drag shows it, and held still it stays.
        let t = drag(&mut a, 600.0, 1800.0, 10, Duration::ZERO);
        assert_eq!(a.bar_alpha(), BAR_ALPHA);
        assert!(a.thumb_rect().is_some());
        for _ in 0..150 {
            a.tick(FRAME);
        }
        assert_eq!(a.bar_alpha(), BAR_ALPHA, "the finger is still down");
        // Lifted (stopped first: no fling): 2 s fully visible.
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1800.0,
            t + Duration::from_secs(3),
        );
        let frames = |ms: u64| ms / FRAME.as_millis() as u64;
        for _ in 0..frames(1980) {
            a.dirty = false;
            a.tick(FRAME);
            assert!(a.dirty, "frames while it shows");
            assert_eq!(a.bar_alpha(), BAR_ALPHA);
        }
        // Then it fades over 0.3 s.
        for _ in 0..frames(160) {
            a.tick(FRAME);
        }
        let mid = a.bar_alpha();
        assert!(mid > 0.0 && mid < BAR_ALPHA, "{mid}");
        let mut fading = 0;
        while a.bar.is_some() {
            a.dirty = false;
            a.tick(FRAME);
            assert!(a.dirty, "frames while it fades, and as it goes");
            fading += 1;
            assert!(fading <= frames(200), "gone by 2.3 s");
        }
        assert_eq!(a.thumb_rect(), None);
        // Gone: no frames asked for.
        a.dirty = false;
        a.tick(FRAME);
        assert!(!a.dirty);
        // A fling keeps it fully visible while it runs.
        let t = drag(&mut a, 800.0, 1100.0, 5, Duration::from_secs(10));
        a.touch(1, TouchPhase::Ended, 540.0, 1100.0, t + MS);
        assert!(a.fling != 0.0);
        let mut ran = 0;
        while a.fling != 0.0 && ran < 1000 {
            a.tick(FRAME);
            assert_eq!(a.bar_alpha(), BAR_ALPHA);
            ran += 1;
        }
        assert!(ran > frames(2000), "a fling that runs past 2 s: {ran}");
        // And the jump scroll shows it.
        let mut b = back_by(3.0);
        for _ in 0..frames(2500) {
            b.tick(FRAME);
        }
        assert_eq!(b.bar, None);
        b.jump_to_end();
        assert_eq!(b.bar_alpha(), BAR_ALPHA);
        run_jump(&mut b);
        assert_eq!(b.bar_alpha(), BAR_ALPHA, "2 s from the landing");
    }

    #[test]
    fn a_touch_within_4_pt_of_the_thumb_drags_it_proportionally_with_no_fling() {
        let s = PHONE.scale;
        let mut a = midway();
        a.take_window();
        let r = a.thumb_rect().unwrap();
        let v = a.frame().viewport;
        let range = a.calculated_length() - a.frame().full_viewport_h;
        let from = a.offset();
        // 3.5 pt left of the thumb, in its strip.
        let (x, y) = (r.x - 3.5 * s, r.y + r.h / 2.0);
        assert_eq!(a.touch(1, TouchPhase::Started, x, y, Duration::ZERO), None);
        assert!(a.thumb.is_some() && a.drag.is_none());
        assert_eq!(a.offset(), from, "grabbing it does not move the view");
        // A quick move down a quarter of the track.
        let travel = v.h - r.h;
        let mut now = Duration::ZERO;
        for i in 1..=5 {
            now += MS;
            a.touch(
                1,
                TouchPhase::Moved,
                x,
                y + travel / 4.0 * i as f32 / 5.0,
                now,
            );
        }
        let want = ((r.y - v.y) / travel + 0.25) * range;
        assert!((a.offset() - want).abs() < 0.5, "{} vs {want}", a.offset());
        assert_eq!(a.take_window(), None, "contents load where it stops");
        // Held: fully visible however long.
        for _ in 0..200 {
            a.tick(FRAME);
        }
        assert_eq!(a.bar_alpha(), BAR_ALPHA);
        // Let go: no fling, the view stays, the 2 s start.
        let at = a.offset();
        a.touch(1, TouchPhase::Ended, x, y + travel / 4.0, now + MS);
        assert!(a.thumb.is_none());
        assert_eq!(a.fling, 0.0);
        assert!(a.take_window().is_some(), "reported where it stopped");
        a.tick(FRAME);
        assert_eq!(a.offset(), at);
        assert_eq!(a.bar_alpha(), BAR_ALPHA);
        // Past the bottom of the track: clamped to the end, following.
        let r = a.thumb_rect().unwrap();
        a.touch(1, TouchPhase::Started, r.x + r.w / 2.0, r.y + 1.0, now);
        a.touch(1, TouchPhase::Moved, r.x, r.y + 2.0 * v.h, now + MS);
        assert_eq!(a.offset(), a.max_offset());
        // And past the top: the session's top.
        a.touch(1, TouchPhase::Moved, r.x, -v.h, now + MS * 2);
        assert_eq!(a.offset(), 0.0);
        a.touch(1, TouchPhase::Ended, r.x, -v.h, now + MS * 3);
        assert_eq!(a.fling, 0.0);
    }

    #[test]
    fn a_touch_outside_the_thumbs_strip_scrolls_as_before() {
        let s = PHONE.scale;
        let mut a = midway();
        let r = a.thumb_rect().unwrap();
        // 5 pt left of the thumb: outside the strip.
        let x = r.x - 5.0 * s;
        let y = r.y + r.h / 2.0;
        a.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        assert!(a.thumb.is_none() && a.drag.is_some());
        a.touch(1, TouchPhase::Moved, x, y + 30.0, MS);
        let at = a.offset();
        a.touch(1, TouchPhase::Moved, x, y + 130.0, MS * 2);
        assert!((at - a.offset() - 100.0).abs() < 0.01, "a 1x drag");
        a.touch(1, TouchPhase::Ended, x, y + 130.0, MS * 30);
    }

    #[test]
    fn a_touch_in_the_band_brings_the_thumb_to_the_finger_centred() {
        let mut a = midway();
        let r = a.thumb_rect().unwrap();
        let v = a.frame().viewport;
        let y = (r.y - 300.0).max(v.y + r.h / 2.0 + 1.0);
        a.touch(2, TouchPhase::Started, r.x, y, Duration::ZERO);
        assert!(a.thumb.is_some() && a.drag.is_none());
        let t = a.thumb_rect().unwrap();
        assert!(
            (t.y + t.h / 2.0 - y).abs() < 0.5,
            "centred: {} vs {y}",
            t.y + t.h / 2.0
        );
    }

    #[test]
    fn the_band_works_when_the_thumb_has_faded() {
        let mut a = midway();
        let r = a.thumb_rect().unwrap();
        for _ in 0..((BAR_HOLD + BAR_FADE).as_millis() / FRAME.as_millis() + 1) {
            a.tick(FRAME);
        }
        assert_eq!(a.bar_alpha(), 0.0);
        let from = a.offset();
        let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0 + 200.0);
        a.touch(1, TouchPhase::Started, x, y, Duration::from_secs(10));
        assert!(a.thumb.is_some() && a.drag.is_none());
        assert!(a.offset() > from, "moved to the finger");
        assert_eq!(a.bar_alpha(), BAR_ALPHA, "and shown");
        a.touch(
            1,
            TouchPhase::Moved,
            x,
            y + 50.0,
            Duration::from_secs(10) + MS,
        );
        a.touch(
            1,
            TouchPhase::Ended,
            x,
            y + 50.0,
            Duration::from_secs(10) + MS * 2,
        );
        assert_eq!(a.fling, 0.0);
    }

    #[test]
    fn disabled_has_no_band_and_always_draws_the_thumb_at_rest() {
        let mut a = midway();
        let r = a.thumb_track().unwrap().0;
        a.set_scrollbar(ScrollbarVisibility::Disabled);
        assert_eq!(a.thumb_rect(), None);
        a.touch(1, TouchPhase::Started, r.x, r.y + r.h / 2.0, Duration::ZERO);
        assert!(a.thumb.is_none() && a.drag.is_some(), "plain scrollback");
        a.touch(1, TouchPhase::Ended, r.x, r.y + r.h / 2.0, MS * 30);
        let mut b = midway();
        b.set_scrollbar(ScrollbarVisibility::Always);
        for _ in 0..((BAR_HOLD + BAR_FADE).as_millis() / FRAME.as_millis() + 1) {
            b.tick(FRAME);
        }
        assert_eq!(b.bar_alpha(), BAR_ALPHA);
        assert!(b.thumb_rect().is_some());
    }

    // The settings' toggles (plan 20261024_02, decision 4).

    fn alpha_in(list: &DrawList, r: Rect) -> f32 {
        list.instances
            .iter()
            .filter(|i| r.contains(i.position))
            .map(|i| i.color[3])
            .fold(0.0, f32::max)
    }

    #[test]
    fn controls_the_settings_turn_off_are_dim_and_take_no_taps() {
        use crate::settings::{self, DISABLED_ALPHA, Settings};
        let cards = vec![card(CardKind::Question, 3, true)];
        let mut a = screen(conversation(3), cards, Some(0));
        a.set_read_aloud(true);
        let tap = |a: &mut AgentScreen, r: Rect| {
            a.touch(1, TouchPhase::Started, r.x + 5.0, r.y + 5.0, Duration::ZERO);
            a.touch(1, TouchPhase::Ended, r.x + 5.0, r.y + 5.0, MS)
        };
        for (off, hits) in [
            (
                Settings {
                    sound: false,
                    ..Settings::default()
                },
                vec![AgentHit::ReadAloud],
            ),
            (
                Settings {
                    microphone: false,
                    ..Settings::default()
                },
                vec![AgentHit::Message, AgentHit::Reply],
            ),
        ] {
            settings::set_current(Settings::default());
            for &hit in &hits {
                let r = a.rect_of(hit).unwrap_or_else(|| panic!("{hit:?} shown"));
                assert_eq!(tap(&mut a, r), Some(hit), "{hit:?} live");
            }
            settings::set_current(off.clone());
            a.invalidate();
            let mut list = DrawList::new();
            a.draw(&mut list, &DrawContext::default(), None);
            for &hit in &hits {
                let r = a.rect_of(hit).unwrap();
                assert!(
                    (alpha_in(&list, r) - DISABLED_ALPHA).abs() < 1e-6,
                    "{hit:?} dim"
                );
                assert_eq!(tap(&mut a, r), None, "{hit:?} takes no tap");
            }
            // A card's Yes and No still work.
            if !off.microphone {
                let yes = a.rect_of(AgentHit::Suggest(Suggestion::Yes)).unwrap();
                assert_eq!(tap(&mut a, yes), Some(AgentHit::Suggest(Suggestion::Yes)));
            }
        }
        // The keyboard glyph is live with text input.
        let keys = a.rect_of(AgentHit::Keyboard).unwrap();
        let mut list = DrawList::new();
        a.draw(&mut list, &DrawContext::default(), None);
        assert_eq!(alpha_in(&list, keys), 1.0, "keyboard live with text input");
        assert_eq!(
            tap(&mut a, keys),
            Some(AgentHit::Keyboard),
            "keyboard live with text input"
        );
        settings::set_current(Settings::default());
    }

    // Plan 20261022_03, the L3 amendments of 2026-10-24 11:54 to 12:27 EDT.

    fn texts(flow: &Flow) -> Vec<String> {
        flow.rows.iter().map(|r| r.text.clone()).collect()
    }

    fn with_text(mut e: Entry, text: &str) -> Entry {
        e.text = Some(text.to_string());
        e
    }

    #[test]
    fn a_long_reply_shows_whole_and_the_summary_stands_in_without_text() {
        let long: String = (0..400).map(|i| format!("w{i:03} ")).collect();
        assert!(long.len() >= 2000);
        let e = with_text(
            entry(Role::Assistant, EntryKind::Text, "w000 w001\u{2026}"),
            &long,
        );
        let f = lay_out_block(&Shows::Entries(vec![e]), false, false, PHONE.w, PHONE.scale);
        let all = texts(&f).join(" ");
        assert!((0..400).all(|i| all.contains(&format!("w{i:03}"))));
        assert!(!all.contains('\u{2026}'), "never cut");
        assert!(f.taps.is_empty(), "text does not fold");
        // An older daemon: the summary.
        let e = entry(Role::Assistant, EntryKind::Text, "Only the summary.");
        let f = lay_out_block(&Shows::Entries(vec![e]), false, false, PHONE.w, PHONE.scale);
        assert_eq!(texts(&f), ["Only the summary."]);
    }

    #[test]
    fn a_context_line_shows_whole() {
        let mut e = entry(Role::User, EntryKind::Text, "short");
        e.origin = Some("peer:docs".into());
        let long: String = (0..60).map(|i| format!("p{i:02} ")).collect();
        let e = with_text(e, &long);
        let f = lay_out_block(&Shows::Entries(vec![e]), false, false, PHONE.w, PHONE.scale);
        let all = texts(&f).join(" ");
        assert!(all.contains("p00") && all.contains("p59"), "{all}");
        assert!(!all.contains('\u{2026}'));
    }

    /// 40 replies, the section at index 20 replaced by `shows`, scrolled
    /// back so that it is in the window's lower half.
    fn with_section(shows: Shows) -> (AgentScreen, u64) {
        let mut sections = outline_sections(40, 0..40);
        sections[20].shows = shows;
        let key = sections[20].serial.to_u64();
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        give(&mut a, "idle", sections, None);
        scrolled_back(&mut a);
        let i = a.slot_of(key).unwrap();
        let top = a.tops[i] - a.frame().viewport.h * 0.5;
        a.scroll_to(top);
        assert!(!a.following());
        (a, key)
    }

    /// Where the block `key` is on screen.
    fn block_point(a: &AgentScreen, key: u64) -> [f32; 2] {
        let i = a.slot_of(key).unwrap();
        let v = a.frame().viewport;
        [PHONE.w * 0.5, v.y + a.tops[i] - a.offset() + 6.0]
    }

    fn block_texts(a: &AgentScreen, key: u64) -> Vec<String> {
        texts(&a.blocks[&key].flow)
    }

    fn tool_section() -> Shows {
        let call = with_text(
            entry(Role::Assistant, EntryKind::ToolUse, "Bash cargo test"),
            "Bash cargo test\n  --workspace\n  --all-targets",
        );
        let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        let result = with_text(
            entry(Role::Tool, EntryKind::ToolResult, "line 1"),
            &lines.join("\n"),
        );
        Shows::Entries(vec![call, result])
    }

    #[test]
    fn a_tool_call_folds_and_a_tap_expands_it_without_moving_the_view() {
        let (mut a, key) = with_section(tool_section());
        let folded = block_texts(&a, key);
        assert_eq!(
            folded,
            ["> Bash cargo test\u{2026}", "  line 1\u{2026}"],
            "its first lines, marked"
        );
        let [x, y] = block_point(&a, key);
        assert_eq!(a.hit(x, y), Some(AgentHit::Fold(key)));
        let place = a.place();
        a.toggle_fold(key);
        assert!(a.is_expanded(key));
        let open = block_texts(&a, key);
        assert!(open.contains(&"    --all-targets".to_string()), "{open:?}");
        assert!(open.contains(&"  line 30".to_string()), "{open:?}");
        assert!(!open.iter().any(|l| l.ends_with('\u{2026}')));
        assert_eq!(a.place(), place, "the view does not move");
        // Tapped again: folded.
        a.toggle_fold(key);
        assert_eq!(block_texts(&a, key), folded);
        assert_eq!(a.place(), place);
    }

    #[test]
    fn a_diff_folds_and_expands_as_a_tool_call_does() {
        let patch = "@@ -1,3 +1,3 @@\n-old\n+new\n context\n-a\n+b\n c";
        let result = with_text(entry(Role::Tool, EntryKind::ToolResult, "@@"), patch);
        let (mut a, key) = with_section(Shows::Entries(vec![result]));
        let folded = block_texts(&a, key);
        assert_eq!(folded.len(), DIFF_LINES);
        assert!(folded.last().unwrap().ends_with('\u{2026}'));
        a.toggle_fold(key);
        let open = block_texts(&a, key);
        assert_eq!(open.len(), 7);
        assert_eq!(open.last().unwrap(), "   c");
        // Its colours stay a diff's.
        let rows = &a.blocks[&key].flow.rows;
        assert_eq!(rows[2].color, theme().green);
    }

    #[test]
    fn a_resolved_card_shows_whole() {
        let mut done = card(CardKind::Approval, 0, false);
        done.title = "Approved".into();
        done.text = (0..40).map(|i| format!("word{i} ")).collect();
        let f = lay_out_block(&Shows::Card(done), false, false, PHONE.w, PHONE.scale);
        assert!(f.rows.len() > 1);
        let all = texts(&f).join(" ");
        assert!(all.contains("word0") && all.contains("word39"));
        assert!(!all.contains('\u{2026}'));
        assert!(f.taps.is_empty(), "it does not fold");
    }

    const REPLY: &str =
        "Landed it.\n\nThe settings screen has its rows, the scrollbar setting, and Clear cache.";

    fn finished(points: &[&str], spoken: Option<&str>) -> AgentCard {
        AgentCard {
            title: "Finished".into(),
            text: "Landed it.".into(),
            points: points.iter().map(|p| p.to_string()).collect(),
            spoken: spoken.map(str::to_string),
            ..card(CardKind::Finished, 0, false)
        }
    }

    /// A screen with a reply on line 21 and its Finished card after it.
    fn with_finished(card: AgentCard) -> (AgentScreen, u64) {
        let mut sections = outline_sections(40, 0..40);
        let reply = Serial::new(FILE, 21, 0);
        sections[20] = ViewSection {
            index: 20,
            serial: reply,
            shows: Shows::Entries(vec![with_text(
                entry(Role::Assistant, EntryKind::Text, "Landed it."),
                REPLY,
            )]),
        };
        let serial = Serial::new(FILE, 21, 194);
        sections.insert(
            21,
            ViewSection {
                index: 0,
                serial,
                shows: Shows::Card(card),
            },
        );
        let mut a = AgentScreen::new();
        a.set_layout(PHONE);
        give(&mut a, "idle", sections, None);
        scrolled_back(&mut a);
        let key = serial.to_u64();
        let i = a.slot_of(key).unwrap();
        a.scroll_to(a.tops[i] - a.frame().viewport.h * 0.5);
        (a, key)
    }

    #[test]
    fn a_summary_card_folds_to_its_summary_and_expands_to_the_reply_points_and_spoken_text() {
        let (mut a, key) = with_finished(finished(
            &["Rows for each setting.", "Clear cache."],
            Some("Landed the settings screen."),
        ));
        assert_eq!(
            block_texts(&a, key),
            ["-- Finished: Landed the settings screen."],
            "folded: the title and the daemon's summary"
        );
        let [x, y] = block_point(&a, key);
        assert_eq!(a.hit(x, y), Some(AgentHit::Fold(key)));
        let place = a.place();
        a.toggle_fold(key);
        let open = block_texts(&a, key);
        assert_eq!(open[0], "-- Finished");
        let all = open.join("\n");
        assert!(all.contains("Landed it."), "{all}");
        assert!(all.contains("Clear cache."), "the whole reply: {all}");
        assert!(
            open.contains(&format!("   {BULLET}Rows for each setting.")),
            "{open:?}"
        );
        assert!(open.contains(&format!("   {SPOKEN}")), "{open:?}");
        assert_eq!(open.last().unwrap(), "   Landed the settings screen.");
        let buttons = &a.blocks[&key].flow.buttons;
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].hit, AgentHit::Replay(key));
        assert_eq!(
            a.spoken_of(key).as_deref(),
            Some("Landed the settings screen.")
        );
        assert_eq!(a.place(), place, "the view does not move");
        // Sound off: no replay button.
        a.set_sound(false);
        assert!(a.blocks[&key].flow.buttons.is_empty());
        a.set_sound(true);
        // Tapped again: folded.
        a.toggle_fold(key);
        assert_eq!(block_texts(&a, key).len(), 1);
        // With no points, there is no list.
        let (mut a, key) = with_finished(finished(&[], Some("Landed it.")));
        a.toggle_fold(key);
        assert!(!block_texts(&a, key).iter().any(|l| l.contains(BULLET)));
    }

    #[test]
    fn the_pinned_summary_card_expands_to_its_points_and_spoken_text() {
        let mut live = finished(&["One.", "Two."], Some("Landed the settings screen."));
        live.live = true;
        let mut a = screen(conversation(40), vec![live], Some(0));
        let pin_texts = |a: &AgentScreen| texts(&a.frame().pin.as_ref().unwrap().flow);
        assert_eq!(pin_texts(&a), ["Finished", "Landed the settings screen."]);
        let pin = a.frame().pin.as_ref().unwrap().rect;
        assert_eq!(a.hit(pin.x + 40.0, pin.y + 40.0), Some(AgentHit::Expand));
        a.toggle_expand();
        let open = pin_texts(&a);
        assert!(open.contains(&format!("{BULLET}One.")), "{open:?}");
        assert!(open.contains(&SPOKEN.to_string()), "{open:?}");
        let buttons = &a.frame().pin.as_ref().unwrap().flow.buttons;
        assert!(buttons.iter().any(|b| matches!(b.hit, AgentHit::Replay(_))));
        // Scrolling back keeps it expanded: not folded to the bar.
        let t = drag(&mut a, 600.0, 1400.0, 10, Duration::ZERO);
        assert!(!a.compact());
        assert_eq!(pin_texts(&a), open, "still expanded while dragging");
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1400.0,
            t + Duration::from_millis(300),
        );
        assert!(!a.following());
        assert_eq!(pin_texts(&a), open, "and once scrolled back");
    }

    #[test]
    fn a_short_form_never_reads_as_cut_off() {
        let cols = 40;
        // With the daemon's summary: the summary.
        let c = finished(&[], Some("Pushed the fix."));
        assert_eq!(
            short_form(&c, Some(REPLY), cols),
            (vec!["Pushed the fix.".to_string()], false)
        );
        // A lead-in takes the list after it, cut with "…" and "more".
        let list = "Written and pushed:\n- src/a.rs\n- src/b.rs\n- src/c.rs\n- src/d.rs";
        let c = finished(&[], None);
        let (lines, cut) = short_form(&c, Some(list), cols);
        assert_eq!(
            lines,
            ["Written and pushed:", "- src/a.rs", "- src/b.rs\u{2026}"]
        );
        assert!(cut);
        let f = lay_out_block(
            &Shows::Card(AgentCard {
                text: list.into(),
                ..finished(&[], None)
            }),
            false,
            false,
            PHONE.w,
            PHONE.scale,
        );
        let t = texts(&f);
        assert_eq!(t[0], "-- Finished: Written and pushed:");
        assert!(t.iter().any(|l| l.ends_with('\u{2026}')), "{t:?}");
        assert_eq!(t.last().unwrap(), MORE);
        assert_eq!(f.taps.len(), 1, "it expands");
        // A short reply shows whole, with no "more".
        let c = AgentCard {
            text: "All done.".into(),
            ..finished(&[], None)
        };
        assert_eq!(
            short_form(&c, None, cols),
            (vec!["All done.".to_string()], false)
        );
        let f = lay_out_block(&Shows::Card(c), false, false, PHONE.w, PHONE.scale);
        assert_eq!(texts(&f), ["-- Finished: All done."]);
        assert!(f.taps.is_empty(), "nothing more to show");
    }

    #[test]
    fn low_memory_leaves_only_the_screens_blocks_laid_out() {
        let mut a = outline_screen(300, 0..300);
        scrolled_back(&mut a);
        a.scroll_to(a.max_offset() / 2.0);
        let before = a.laid_out().len();
        a.set_pressure(Some(Pressure::Low));
        let v = a.frame().viewport;
        let mut on_screen: Vec<u64> = a
            .slots
            .iter()
            .zip(&a.tops)
            .filter(|(slot, top)| **top + slot.height >= a.offset() && **top <= a.offset() + v.h)
            .map(|(slot, _)| slot.serial.to_u64())
            .collect();
        on_screen.sort_unstable();
        assert_eq!(a.laid_out(), on_screen);
        assert!(before > on_screen.len());
        // Background keeps a screen either side; back in front, as before.
        a.set_pressure(Some(Pressure::Background));
        let one = a.laid_out().len();
        assert!(one > on_screen.len() && one < before, "{one}");
        a.set_pressure(None);
        assert_eq!(a.laid_out().len(), before);
    }

    /// One finger dragging up by `dy` pixels from `y`, in steps of 10.
    fn pull_up(a: &mut AgentScreen, y: f32, dy: f32, t: Duration) -> Duration {
        let steps = (dy / 10.0) as u32;
        drag(a, y, y - dy, steps, t)
    }

    #[test]
    fn a_pull_to_refresh_leaves_the_pinned_card_whole() {
        let mut live = finished(&["One."], Some("Landed it."));
        live.live = true;
        let mut a = screen(conversation(40), vec![live], Some(0));
        assert!(a.following() && !a.compact());
        let t = pull_up(&mut a, 1500.0, 200.0, Duration::ZERO);
        assert!(a.pull() > 0.0);
        assert!(!a.compact(), "not folded to the bar while pulling");
        a.touch(
            1,
            TouchPhase::Ended,
            540.0,
            1300.0,
            t + Duration::from_millis(16),
        );
        assert!(!a.compact());
    }

    #[test]
    fn pulling_past_the_end_resists_and_says_pull_then_release() {
        let s = PHONE.scale;
        let mut a = outline_screen(100, 0..100);
        assert!(a.following());
        let t = pull_up(&mut a, 1500.0, 200.0, Duration::ZERO);
        // Past the 10 pt slop, the rest at half: 180 px, 90.
        assert!((a.pull() - 90.0).abs() < 0.5, "{}", a.pull());
        assert_eq!(a.offset(), a.max_offset());
        assert_eq!(a.pull_line().unwrap().0, PULL_TO_REFRESH);
        for i in 1..=10 {
            a.touch(1, TouchPhase::Moved, 540.0, 1300.0 - 10.0 * i as f32, t);
        }
        assert!(a.pull() >= PULL_ARM * s);
        assert_eq!(a.pull_line().unwrap().0, RELEASE_TO_REFRESH);
        // At most 96 pt.
        for i in 1..=60 {
            a.touch(1, TouchPhase::Moved, 540.0, 1200.0 - 10.0 * i as f32, t);
        }
        assert_eq!(a.pull(), PULL_MAX * s);
        // Back down: the pull goes first.
        a.touch(1, TouchPhase::Moved, 540.0, 700.0, t);
        assert!((a.pull() - (PULL_MAX * s - 50.0)).abs() < 0.5);
        assert_eq!(a.offset(), a.max_offset());
    }

    #[test]
    fn letting_go_past_64_pt_refreshes_and_shows_the_result_then_springs_back() {
        let s = PHONE.scale;
        let mut a = outline_screen(100, 0..100);
        let t = pull_up(&mut a, 1500.0, 400.0, Duration::ZERO);
        a.touch(1, TouchPhase::Ended, 540.0, 1100.0, t + MS);
        assert!(a.take_refresh());
        assert!(!a.take_refresh(), "once");
        assert_eq!(a.fling, 0.0);
        assert_eq!(a.pull(), PULL_ARM * s, "held overscrolled");
        assert_eq!(a.pull_line().unwrap().0, REFRESHING);
        for _ in 0..50 {
            a.tick(FRAME);
        }
        assert_eq!(a.pull(), PULL_ARM * s, "while it runs");
        a.refreshed(0);
        assert_eq!(a.pull_line().unwrap().0, UP_TO_DATE);
        for _ in 0..40 {
            a.tick(FRAME);
        }
        assert_eq!(a.pull_line().unwrap().0, UP_TO_DATE, "for 1 s");
        for _ in 0..40 {
            a.tick(FRAME);
        }
        assert_eq!(a.pull(), 0.0, "sprung back");
        assert!(a.pull_line().is_none());
        assert!(a.following());
        assert_eq!(a.offset(), a.max_offset());
    }

    #[test]
    fn the_refresh_result_is_the_ends_state_or_n_new() {
        let refreshed_with = |cards: Vec<AgentCard>, pinned, new| {
            let mut a = screen(conversation(10), cards, pinned);
            let t = pull_up(&mut a, 1500.0, 400.0, Duration::ZERO);
            a.touch(1, TouchPhase::Ended, 540.0, 1100.0, t + MS);
            assert!(a.take_refresh());
            a.refreshed(new);
            a.pull_line().unwrap()
        };
        let (text, color) = refreshed_with(vec![card(CardKind::Question, 40, true)], Some(0), 2);
        assert_eq!(text, "-- question: Shall I go on? --");
        assert_eq!(color, accent(CardKind::Question));
        let done = AgentCard {
            live: true,
            ..finished(&[], Some("Landed the settings screen."))
        };
        let (text, _) = refreshed_with(vec![done], Some(0), 1);
        assert_eq!(text, "-- done: Landed the settings screen. --");
        let (text, _) = refreshed_with(vec![], None, 3);
        assert_eq!(text, "-- 3 new --");
        let (text, _) = refreshed_with(vec![card(CardKind::Question, 40, true)], Some(0), 0);
        assert_eq!(text, UP_TO_DATE);
    }

    #[test]
    fn letting_go_early_springs_back_and_a_fling_to_the_end_refreshes_nothing() {
        let mut a = outline_screen(100, 0..100);
        let t = pull_up(&mut a, 1500.0, 100.0, Duration::ZERO);
        assert!(a.pull() > 0.0);
        a.touch(1, TouchPhase::Ended, 540.0, 1400.0, t + MS);
        assert!(!a.take_refresh());
        for _ in 0..30 {
            a.tick(FRAME);
        }
        assert_eq!(a.pull(), 0.0);
        assert!(a.following());
        // A hard fling from scrolled back to the end.
        scrolled_back(&mut a);
        let t = drag(&mut a, 1500.0, 1200.0, 5, Duration::from_secs(10));
        assert!(!a.following() && a.pull() == 0.0);
        a.touch(1, TouchPhase::Ended, 540.0, 1200.0, t);
        assert!(a.fling != 0.0, "a fling");
        for _ in 0..200 {
            a.tick(FRAME);
        }
        assert!(a.following());
        assert_eq!(a.pull(), 0.0);
        assert!(!a.take_refresh());
    }

    fn bookend(i: u64, summary: Option<&str>, time: Option<&str>) -> Bookend {
        Bookend {
            serial: Serial::new(FILE, i as u32 + 1, 0),
            summary: summary.map(str::to_string),
            time: time.map(str::to_string),
        }
    }

    #[test]
    fn section_labels_show_on_the_scrollbars_scale_while_its_thumb_is_dragged() {
        let mut a = midway();
        a.set_bookends(vec![
            bookend(60, Some("Landed the settings screen"), Some("12:41")),
            bookend(220, None, Some("13:05")),
            bookend(221, None, None),
        ]);
        assert!(a.section_labels().is_empty(), "not while just scrolled");
        // The thumb grabbed.
        let r = a.thumb_rect().unwrap();
        let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        a.touch(1, TouchPhase::Started, x, y, Duration::ZERO);
        let v = a.frame().viewport;
        let total = a.calculated_length();
        let labels = a.section_labels();
        let first = &labels[0];
        assert_eq!(first.text, "Landed the settings screen \u{b7} 12:41");
        // At its share of the track, centred on the screen.
        let i = a.slot_of(Serial::new(FILE, 61, 0).to_u64()).unwrap();
        let want = v.y + a.tops[i] / total * v.h;
        assert!((first.rect.y + first.rect.h / 2.0 - want).abs() < 0.5);
        assert!((first.rect.x + first.rect.w / 2.0 - PHONE.w / 2.0).abs() < 0.5);
        // Two that would overlap: the one nearer the thumb shows.
        assert_eq!(labels.len(), 2, "{labels:?}");
        assert_eq!(labels[1].text, "Done \u{b7} 13:05");
        assert_eq!(labels.iter().filter(|l| l.nearest).count(), 1);
        // The thumb taken near the first: it is the nearest, and brighter.
        a.touch(1, TouchPhase::Moved, x, want, MS);
        let labels = a.section_labels();
        assert!(labels[0].nearest && !labels[1].nearest);
        a.invalidate();
        let mut list = DrawList::new();
        a.draw(&mut list, &DrawContext::default(), None);
        // (No font here: the pills are what is drawn.)
        let near = alpha_in(&list, labels[0].rect);
        let far = alpha_in(&list, labels[1].rect);
        assert!((near - LABEL_NEAR_BG).abs() < 1e-4, "{near}");
        assert!((far - LABEL_BG).abs() < 1e-4, "{far}");
        // Near the last: it gives way the other way, and reads "Done".
        let j = a.slot_of(Serial::new(FILE, 222, 0).to_u64()).unwrap();
        let last = v.y + a.tops[j] / total * v.h;
        a.touch(1, TouchPhase::Moved, x, last + 20.0, MS * 2);
        let labels = a.section_labels();
        assert!(
            labels.iter().any(|l| l.text == DONE && l.nearest),
            "{labels:?}"
        );
        // Let go: they stay 2 s, then fade with the thumb.
        a.touch(1, TouchPhase::Ended, x, last + 20.0, MS * 3);
        for _ in 0..90 {
            a.tick(FRAME);
        }
        assert_eq!(a.labels_alpha(), 1.0);
        for _ in 0..30 {
            a.tick(FRAME);
        }
        assert_eq!(a.labels_alpha(), 0.0);
        assert!(a.section_labels().is_empty());
    }
}
