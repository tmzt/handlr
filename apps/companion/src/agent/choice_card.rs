//! The choice card: a live Question card whose pending question is a
//! choice question (docs/design/20261029_01_CHOICE_QUESTIONS_PLAN.md,
//! decisions 7 to 9, step 6), answered as Claude's own question screen is.
//!
//! - A **header chip** (the question's `header`, 12 pt, in the session's
//!   colour) over the question (15 pt); with two or more questions, "1 of
//!   N" at the chip's right, and a back arrow at its left from the second
//!   on, which returns to the previous question with its answer kept.
//! - Each **option** a row of 44 pt at least, 8 pt apart: its label
//!   (15 pt) and its `detail` beneath, dim (13 pt), wrapped.
//! - **Single-select:** a tap chooses the row; one question sends at once,
//!   several move on to the next.
//! - **Multi-select:** each row is a libcontrols toggle item
//!   (`PopupItem::toggle`, with `PopupRow` cells for the label and the dim
//!   detail), flipped in place by a tap as a popup menu's `Toggled` flips
//!   it, its check mark drawn from the terminal face; **Done** under the
//!   rows is enabled once a row is checked or Other has text.
//! - **Other…** (where the question takes one) opens the typed card; the
//!   text it gives back is checked here with the daemon's message rules and
//!   shows in the row in quotes, as the answer (single) or one more checked
//!   row (multi).
//! - **Previews:** under their option, folded to 3 lines in the monospace
//!   face with "…"; a tap expands one to its whole text (as many lines as
//!   the card has room for) and a tap again folds it.
//! - **Several questions:** after the last, a short review, each header and
//!   its answer on one line, with **Submit** and **Back**. A single
//!   question skips the review.
//! - **Sending:** the options grey and "Sending…" shows until the result;
//!   then the result's or the refusal's line.
//!
//! The card's state ([`ChoiceCard`]) belongs to the agent screen, kept per
//! choice set: a new set (a new question) starts it again.

use std::collections::BTreeSet;

use libcontrols::{PopupCell, PopupItem, PopupRow, Rect};
use libflankr_proto::common::{ChoiceAnswer, ChoiceQuestion, ChoiceSet};
use libflankr_proto::envelope::codes;

use super::{
    ACTION_H, BACK_GLYPH, CARD_PAD, EDGE, Fill, Flow, FlowButton, Row, columns, line_h,
    term_columns, term_h, wrap_hard,
};
use crate::settings::copy;
use crate::theme::{theme, wants_dark_icons, with_alpha};

/// Type sizes, in points (decision 7).
const CHIP_PT: f32 = 12.0;
const QUESTION_PT: f32 = 15.0;
const LABEL_PT: f32 = 15.0;
const DETAIL_PT: f32 = 13.0;
const PREVIEW_PT: f32 = 13.0;
/// An option row's least height and the gap between rows, in points.
const ROW_MIN: f32 = 44.0;
const ROW_GAP: f32 = 8.0;
/// A row's inner padding, and the check column of a multi-select row.
const ROW_PAD: f32 = 10.0;
const CHECK_W: f32 = 26.0;
/// The chip's height and its side padding.
const CHIP_H: f32 = 22.0;
const CHIP_PAD: f32 = 8.0;
/// The back arrow's tap width, left of the chip.
const BACK_W: f32 = 36.0;
/// A folded preview's lines (decision 7).
pub const PREVIEW_FOLDED: usize = 3;
/// The most lines an expanded preview shows (`CHOICE_PREVIEW_LINES`).
pub const PREVIEW_LINES: usize = libflankr_proto::common::CHOICE_PREVIEW_LINES;
/// The check marks, from the terminal face's symbol page.
pub const CHECKED: &str = "\u{2611}";
pub const UNCHECKED: &str = "\u{2610}";
/// The most characters of an Other text (`MAX_MESSAGE_LEN`).
pub const OTHER_MAX: usize = 4000;

/// A tap on the choice card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceHit {
    /// The question's option at this index: choose it, or toggle it.
    Option(usize),
    /// The preview under this option: expand it, or fold it again.
    Preview(usize),
    /// The Other row: the typed card.
    Other,
    /// A multi-select question's Done.
    Done,
    /// The back arrow at the chip's left, or the review's Back: the
    /// previous question, its answer kept.
    Back,
    /// The review's Submit.
    Submit,
}

/// What a tap asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChoiceAction {
    /// `choice.answer` (`Input::Choose`) with these answers.
    Send {
        id: String,
        answers: Vec<ChoiceAnswer>,
    },
    /// The typed card, headed with `label`, holding `text` (the Other text
    /// given before, if any).
    Other { label: String, text: String },
}

/// One question's answer as it is being given.
#[derive(Clone, Debug, PartialEq)]
struct Draft {
    /// Single-select: the chosen option.
    chosen: Option<usize>,
    /// Multi-select: each option a libcontrols toggle item with its cells
    /// (the label, and the detail dim).
    rows: Vec<PopupRow>,
    /// Multi-select: the checked options in the order they were checked
    /// (Claude joins them so; the daemon does not compare the order).
    order: Vec<usize>,
    /// The Other text, once given (cleaned as the daemon cleans it).
    other: Option<String>,
}

impl Draft {
    fn new(q: &ChoiceQuestion) -> Self {
        let rows = if q.multi {
            q.options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    let mut cells = vec![PopupCell::new(o.label.clone()).flex()];
                    if !o.detail.is_empty() {
                        cells.push(PopupCell::new(o.detail.clone()).dim(true));
                    }
                    PopupItem::toggle(format!("option/{i}"), o.label.clone(), false)
                        .with_cells(cells)
                })
                .collect()
        } else {
            Vec::new()
        };
        Draft {
            chosen: None,
            rows,
            order: Vec::new(),
            other: None,
        }
    }

    fn checked(&self, i: usize) -> bool {
        self.rows
            .get(i)
            .and_then(|r| r.item.checked)
            .unwrap_or(false)
    }

    /// Flips option `i`'s toggle, as a popup menu's `Toggled` does.
    fn toggle(&mut self, i: usize) {
        let Some(row) = self.rows.get_mut(i) else {
            return;
        };
        let checked = !row.item.checked.unwrap_or(false);
        row.item.checked = Some(checked);
        self.order.retain(|&k| k != i);
        if checked {
            self.order.push(i);
        }
    }

    /// Whether it is an answer: something chosen, checked or typed.
    fn answered(&self) -> bool {
        self.chosen.is_some() || !self.order.is_empty() || self.other.is_some()
    }

    fn answer(&self) -> ChoiceAnswer {
        let picked = match self.chosen {
            Some(i) => vec![i],
            None => self.order.clone(),
        };
        ChoiceAnswer {
            picked,
            other: self.other.clone(),
        }
    }

    /// The review's answer: the labels, then the Other text in quotes.
    fn summary(&self, q: &ChoiceQuestion) -> String {
        let a = self.answer();
        let mut parts: Vec<String> = a
            .picked
            .iter()
            .filter_map(|&i| q.options.get(i).map(|o| o.label.clone()))
            .collect();
        if let Some(t) = &a.other {
            parts.push(format!("\u{201c}{t}\u{201d}"));
        }
        if parts.is_empty() {
            copy::CHOICE_NO_ANSWER.text.to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// A question's chip: its header, else its text.
pub fn chip_of(q: &ChoiceQuestion) -> &str {
    if q.header.trim().is_empty() {
        q.text.trim()
    } else {
        q.header.trim()
    }
}

/// The card's line under the question: a result, a refusal, or the card's
/// own check of an Other text, and its colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub color: [f32; 4],
}

/// The choice card's state, for one choice set.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ChoiceCard {
    /// The set's id: another set starts the card again.
    id: String,
    /// The question shown.
    question: usize,
    /// The review is shown, after the last of several questions.
    review: bool,
    drafts: Vec<Draft>,
    /// The previews shown whole, by question and option.
    expanded: BTreeSet<(usize, usize)>,
    /// `choice.answer` is on its way.
    sending: bool,
    line: Option<Line>,
    /// The session's colour, for the chip.
    pub colour: Option<[f32; 4]>,
}

impl ChoiceCard {
    /// A card for `set`, at its first question with nothing chosen.
    pub fn for_set(set: &ChoiceSet) -> Self {
        ChoiceCard {
            id: set.id.clone(),
            drafts: set.questions.iter().map(Draft::new).collect(),
            ..Default::default()
        }
    }

    /// Follows the view's choice question: a different set (or none)
    /// starts again. Returns whether anything changed.
    pub fn sync(&mut self, set: Option<&ChoiceSet>) -> bool {
        match set {
            Some(set) if set.id == self.id && self.drafts.len() == set.questions.len() => false,
            Some(set) => {
                let colour = self.colour;
                *self = ChoiceCard::for_set(set);
                self.colour = colour;
                true
            }
            None if self.id.is_empty() => false,
            None => {
                let colour = self.colour;
                *self = ChoiceCard {
                    colour,
                    ..Default::default()
                };
                true
            }
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn question(&self) -> usize {
        self.question
    }

    pub fn in_review(&self) -> bool {
        self.review
    }

    pub fn sending(&self) -> bool {
        self.sending
    }

    pub fn line(&self) -> Option<&Line> {
        self.line.as_ref()
    }

    /// Whether the shown multi-select question's Done is enabled: a row
    /// checked, or Other given.
    pub fn done_enabled(&self) -> bool {
        self.drafts
            .get(self.question)
            .is_some_and(|d| !d.order.is_empty() || d.other.is_some())
    }

    /// Whether option `i` of question `q` is checked (multi) or chosen.
    pub fn is_picked(&self, q: usize, i: usize) -> bool {
        self.drafts
            .get(q)
            .is_some_and(|d| d.chosen == Some(i) || d.checked(i))
    }

    /// Question `q`'s Other text, once given.
    pub fn other(&self, q: usize) -> Option<&str> {
        self.drafts.get(q).and_then(|d| d.other.as_deref())
    }

    /// The libcontrols rows of question `q` (multi-select).
    pub fn rows(&self, q: usize) -> &[PopupRow] {
        self.drafts.get(q).map_or(&[], |d| &d.rows)
    }

    /// The answers as they stand, one per question.
    pub fn answers(&self) -> Vec<ChoiceAnswer> {
        self.drafts.iter().map(Draft::answer).collect()
    }

    /// The review's lines: each header and its answer.
    pub fn review_lines(&self, set: &ChoiceSet) -> Vec<String> {
        set.questions
            .iter()
            .zip(&self.drafts)
            .map(|(q, d)| {
                copy::CHOICE_REVIEW_LINE
                    .fill(None, &[("header", chip_of(q)), ("answer", &d.summary(q))])
                    .trim()
                    .to_string()
            })
            .collect()
    }

    /// A tap on the card, for `set`. Taps while sending do nothing.
    pub fn tap(&mut self, hit: ChoiceHit, set: &ChoiceSet) -> Option<ChoiceAction> {
        if set.id != self.id {
            self.sync(Some(set));
        }
        if self.sending {
            return None;
        }
        let q = set.questions.get(self.question)?;
        match hit {
            ChoiceHit::Preview(i) => {
                let key = (self.question, i);
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
                None
            }
            ChoiceHit::Option(i) if self.review || i >= q.options.len() => None,
            ChoiceHit::Option(i) if q.multi => {
                self.drafts[self.question].toggle(i);
                self.line = None;
                None
            }
            ChoiceHit::Option(i) => {
                let d = &mut self.drafts[self.question];
                d.chosen = Some(i);
                d.other = None;
                self.line = None;
                self.next(set)
            }
            ChoiceHit::Other if self.review || !q.other => None,
            ChoiceHit::Other => Some(ChoiceAction::Other {
                label: copy::CHOICE_OTHER_HEADER
                    .fill(None, &[("header", chip_of(q))])
                    .trim()
                    .to_string(),
                text: self.drafts[self.question].other.clone().unwrap_or_default(),
            }),
            ChoiceHit::Done if self.review || !q.multi || !self.done_enabled() => None,
            ChoiceHit::Done => self.next(set),
            ChoiceHit::Back => {
                if self.review {
                    self.review = false;
                } else {
                    self.question = self.question.saturating_sub(1);
                }
                self.line = None;
                None
            }
            ChoiceHit::Submit if self.review => self.send(),
            ChoiceHit::Submit => None,
        }
    }

    /// After a question is answered: send (a single question), the next
    /// question, or the review after the last of several.
    fn next(&mut self, set: &ChoiceSet) -> Option<ChoiceAction> {
        if set.questions.len() == 1 {
            return self.send();
        }
        if self.question + 1 < set.questions.len() {
            self.question += 1;
        } else {
            self.review = true;
        }
        None
    }

    fn send(&mut self) -> Option<ChoiceAction> {
        if !self.drafts.iter().all(Draft::answered) {
            return None;
        }
        self.sending = true;
        self.line = None;
        Some(ChoiceAction::Send {
            id: self.id.clone(),
            answers: self.answers(),
        })
    }

    /// The typed card gave back `text` for the shown question's Other: it
    /// is checked with the daemon's message rules; refused, the card says
    /// why and nothing changes. Given, it is the answer (single, which then
    /// moves on as a tap does) or one more checked row (multi); empty text
    /// on a multi-select question takes Other back.
    pub fn other_given(&mut self, text: &str, set: &ChoiceSet) -> Option<ChoiceAction> {
        if set.id != self.id || self.sending || self.review {
            return None;
        }
        let q = set.questions.get(self.question)?;
        if !q.other {
            return None;
        }
        if q.multi && text.trim().is_empty() {
            self.drafts[self.question].other = None;
            self.line = None;
            return None;
        }
        match check_other(text) {
            Err(why) => {
                self.line = Some(Line {
                    text: why,
                    color: theme().red,
                });
                None
            }
            Ok(cleaned) => {
                self.line = None;
                let d = &mut self.drafts[self.question];
                d.other = Some(cleaned);
                if q.multi {
                    None
                } else {
                    d.chosen = None;
                    self.next(set)
                }
            }
        }
    }

    /// `choice.answer`'s result, or its refusal (`Event::ChoiceAnswered`),
    /// for the set `id`: the line it leaves. `agent` is the agent's shown
    /// name, for a mismatch. Answered, the card goes (the client hides it)
    /// and the state starts again.
    pub fn answered(
        &mut self,
        id: &str,
        outcome: &str,
        got: &[String],
        code: Option<i64>,
        agent: &str,
    ) {
        if id != self.id {
            return;
        }
        self.sending = false;
        self.line =
            result_line(outcome, got, code, agent).map(|(text, color)| Line { text, color });
        if matches!(outcome, "answered" | "cancelled") {
            self.question = 0;
            self.review = false;
        }
    }
}

/// The line a result or refusal leaves on the card, and its colour; none
/// for `answered`.
pub fn result_line(
    outcome: &str,
    got: &[String],
    code: Option<i64>,
    agent: &str,
) -> Option<(String, [f32; 4])> {
    let reason = got.first().map(String::as_str).unwrap_or("");
    let text = match (outcome, code) {
        ("answered" | "cancelled", _) => return None,
        // The drag's cancel (plan 20261029_01, the 2026-10-30 16:42 EDT
        // amendment, item 4): the card springs back with this line.
        ("not_cancelled", _) => copy::CHOICE_NOT_DISMISSED.text.to_string(),
        ("mismatch", _) => {
            return Some((
                copy::CHOICE_RECORDED
                    .fill(None, &[("agent", agent), ("answers", &got.join("; "))])
                    .trim()
                    .to_string(),
                theme().yellow,
            ));
        }
        ("no_result", _) => copy::CHOICE_NOT_ANSWERED.text.to_string(),
        (_, Some(codes::SCREEN_MISMATCH)) => copy::CHOICE_NOT_ANSWERED.text.to_string(),
        (_, Some(codes::CHOICE_STALE)) => copy::CHOICE_STALE.text.to_string(),
        (_, Some(codes::CHOICE_UNSUPPORTED)) => copy::CHOICE_UNSUPPORTED.text.to_string(),
        (_, Some(codes::NOT_PERMITTED)) => copy::CHOICE_NOT_PERMITTED
            .fill(None, &[("reason", reason)])
            .trim()
            .to_string(),
        _ => copy::CHOICE_FAILED
            .fill(None, &[("reason", reason)])
            .trim()
            .to_string(),
    };
    Some((text, theme().red))
}

/// An Other text checked as the daemon's `validate_message` checks a
/// message: one line, folded whitespace with trailing `.` and `,` taken
/// off, something in it, not starting with `/`, `!` or `#`, and at most
/// 4000 characters. The cleaned text, as the daemon will type and compare
/// it, or why not. Paths are the daemon's to check, on Submit.
pub fn check_other(text: &str) -> Result<String, String> {
    if text.trim().chars().any(char::is_control) {
        return Err(copy::CHOICE_OTHER_ONE_LINE.text.to_string());
    }
    let folded = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = folded.trim_end_matches(['.', ',']).trim_end();
    if !cleaned.chars().any(char::is_alphanumeric) {
        return Err(copy::CHOICE_OTHER_EMPTY.text.to_string());
    }
    if let Some(c) = cleaned
        .chars()
        .next()
        .filter(|c| matches!(c, '/' | '!' | '#'))
    {
        return Err(copy::CHOICE_OTHER_COMMAND
            .fill(None, &[("mark", &c.to_string())])
            .trim()
            .to_string());
    }
    if cleaned.chars().count() > OTHER_MAX {
        return Err(copy::CHOICE_OTHER_TOO_LONG.text.to_string());
    }
    Ok(cleaned.to_string())
}

/// Black or white, whichever reads on `c`.
fn on(c: [f32; 4]) -> [f32; 4] {
    if wants_dark_icons(c) {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [1.0, 1.0, 1.0, 1.0]
    }
}

/// A preview's lines at `cols`: its whole text up to `max` lines, or
/// folded to [`PREVIEW_FOLDED`], the last cut with "…" when there is more.
pub fn preview_lines(text: &str, cols: usize, max: usize) -> Vec<String> {
    let all: Vec<String> = text
        .trim_end()
        .lines()
        .flat_map(|l| {
            let l = l.replace('\t', "    ");
            if l.chars().count() <= cols {
                vec![l]
            } else {
                // Monospace: cut at the column, no word wrap.
                l.chars()
                    .collect::<Vec<_>>()
                    .chunks(cols.max(1))
                    .map(|c| c.iter().collect())
                    .collect()
            }
        })
        .collect();
    let max = max.max(1);
    if all.len() <= max {
        return all;
    }
    let mut kept: Vec<String> = all.into_iter().take(max).collect();
    if let Some(last) = kept.last_mut() {
        if last.chars().count() >= cols {
            *last = last.chars().take(cols.saturating_sub(1)).collect();
        }
        last.push('\u{2026}');
    }
    kept
}

/// Lays out the choice card for a live Question card's `set`, as `state` stands
/// (a fresh state when it is another set's), at `(x, y)`, `w` wide, scale
/// `s`, into `flow`; an expanded preview shows at most `preview_max`
/// lines. Options, previews and Other are tap areas (`flow.taps`); Done,
/// Back and Submit are buttons. Returns the card's height.
#[allow(clippy::too_many_arguments)]
pub fn lay_out(
    set: &ChoiceSet,
    state: &ChoiceCard,
    x: f32,
    y: f32,
    w: f32,
    s: f32,
    preview_max: usize,
    flow: &mut Flow,
) -> f32 {
    let fresh;
    let state = if state.id == set.id && state.drafts.len() == set.questions.len() {
        state
    } else {
        fresh = ChoiceCard {
            colour: state.colour,
            ..ChoiceCard::for_set(set)
        };
        &fresh
    };
    let accent = theme().question;
    let colour = state.colour.unwrap_or(accent);
    let pad = CARD_PAD * s;
    let inner_x = x + EDGE * s + pad;
    let inner_w = w - EDGE * s - 2.0 * pad;
    // Greyed while sending.
    let (fg, dim) = if state.sending {
        (theme().dim, with_alpha(theme().dim, 0.6))
    } else {
        (theme().fg, theme().dim)
    };
    let hit = super::AgentHit::Choice;
    // The panel goes in first, under everything; its height is set last.
    let shadow = flow.fills.len();
    flow.fills.push(Fill {
        rect: Rect::new(x + 3.0 * s, y + 4.0 * s, w, 0.0),
        color: theme().shadow,
        radius: 10.0 * s,
    });
    flow.fills.push(Fill {
        rect: Rect::new(x, y, w, 0.0),
        color: theme().card,
        radius: 10.0 * s,
    });
    flow.fills.push(Fill {
        rect: Rect::new(x, y, EDGE * s, 0.0),
        color: accent,
        radius: 3.0 * s,
    });
    let mut ty = y + pad;
    let q_index = state.question.min(set.questions.len().saturating_sub(1));
    let q = &set.questions[q_index];
    let several = set.questions.len() > 1;

    // The chip row: back, the chip, "1 of N".
    let chip_px = CHIP_PT * s;
    let mut cx = inner_x;
    if several && (state.question > 0 || state.review) {
        let r = Rect::new(
            inner_x - 6.0 * s,
            ty - 6.0 * s,
            BACK_W * s,
            CHIP_H * s + 12.0 * s,
        );
        flow.rows.push(Row {
            text: BACK_GLYPH.to_string(),
            x: inner_x,
            y: ty + (CHIP_H * s - term_h(18.0 * s)) * 0.5,
            px: 18.0 * s,
            color: fg,
            mono: true,
        });
        flow.taps.push((hit(ChoiceHit::Back), r));
        cx += BACK_W * s - 6.0 * s;
    }
    let chip_text = if state.review {
        copy::CHOICE_REVIEW.text.to_string()
    } else {
        chip_of(q).to_uppercase()
    };
    let count = if several && !state.review {
        Some(
            copy::CHOICE_COUNT
                .fill(
                    None,
                    &[
                        ("n", &(q_index + 1).to_string()),
                        ("count", &set.questions.len().to_string()),
                    ],
                )
                .trim()
                .to_string(),
        )
    } else {
        None
    };
    let count_w = count
        .as_ref()
        .map_or(0.0, |c| c.chars().count() as f32 * chip_px * 0.55 + 8.0 * s);
    let room = (inner_x + inner_w - count_w - cx - 2.0 * CHIP_PAD * s).max(chip_px * 4.0);
    let chip_text = super::one_line(&chip_text, (room / (chip_px * 0.55)) as usize);
    let chip_w = chip_text.chars().count() as f32 * chip_px * 0.55 + 2.0 * CHIP_PAD * s;
    flow.fills.push(Fill {
        rect: Rect::new(cx, ty, chip_w, CHIP_H * s),
        color: colour,
        radius: CHIP_H * s * 0.5,
    });
    flow.rows.push(Row {
        text: chip_text,
        x: cx + CHIP_PAD * s,
        y: ty + (CHIP_H * s - line_h(chip_px)) * 0.5,
        px: chip_px,
        color: on(colour),
        mono: false,
    });
    if let Some(c) = count {
        flow.rows.push(Row {
            x: inner_x + inner_w - count_w + 8.0 * s,
            y: ty + (CHIP_H * s - line_h(chip_px)) * 0.5,
            text: c,
            px: chip_px,
            color: dim,
            mono: false,
        });
    }
    ty += CHIP_H * s + 8.0 * s;

    let text_rows =
        |text: &str, px: f32, color: [f32; 4], x: f32, w: f32, ty: &mut f32, flow: &mut Flow| {
            for l in wrap_hard(text.trim(), columns(w, px)) {
                flow.rows.push(Row {
                    text: l,
                    x,
                    y: *ty,
                    px,
                    color,
                    mono: false,
                });
                *ty += line_h(px);
            }
        };

    if state.review {
        for l in state.review_lines(set) {
            text_rows(&l, QUESTION_PT * s, fg, inner_x, inner_w, &mut ty, flow);
            ty += 4.0 * s;
        }
    } else {
        // The question.
        text_rows(
            &q.text,
            QUESTION_PT * s,
            fg,
            inner_x,
            inner_w,
            &mut ty,
            flow,
        );
        ty += 8.0 * s;
        let draft = &state.drafts[q_index];
        let label_x = if q.multi {
            inner_x + ROW_PAD * s + CHECK_W * s
        } else {
            inner_x + ROW_PAD * s
        };
        let label_w = inner_x + inner_w - ROW_PAD * s - label_x;
        // One row: a fill, its check, its label and detail lines; its tap.
        let row = |checked: Option<bool>,
                   picked: bool,
                   label: &str,
                   label_color: [f32; 4],
                   detail: &str,
                   tap: ChoiceHit,
                   ty: &mut f32,
                   flow: &mut Flow| {
            let label_lines = wrap_hard(label.trim(), columns(label_w, LABEL_PT * s));
            let detail_lines = if detail.trim().is_empty() {
                Vec::new()
            } else {
                wrap_hard(detail.trim(), columns(label_w, DETAIL_PT * s))
            };
            let content = label_lines.len() as f32 * line_h(LABEL_PT * s)
                + detail_lines.len() as f32 * line_h(DETAIL_PT * s);
            let h = (content + 2.0 * ROW_PAD * s).max(ROW_MIN * s);
            let r = Rect::new(inner_x, *ty, inner_w, h);
            flow.fills.push(Fill {
                rect: r,
                color: if picked {
                    with_alpha(accent, 0.25)
                } else {
                    theme().input
                },
                radius: 8.0 * s,
            });
            let mut ly = *ty + (h - content) * 0.5;
            if let Some(on) = checked {
                let px = LABEL_PT * s * 1.2;
                flow.rows.push(Row {
                    text: if on { CHECKED } else { UNCHECKED }.to_string(),
                    x: inner_x + ROW_PAD * s,
                    y: ly + (line_h(LABEL_PT * s) - term_h(px)) * 0.5,
                    px,
                    color: if on { accent } else { dim },
                    mono: true,
                });
            }
            for l in label_lines {
                flow.rows.push(Row {
                    text: l,
                    x: label_x,
                    y: ly,
                    px: LABEL_PT * s,
                    color: label_color,
                    mono: false,
                });
                ly += line_h(LABEL_PT * s);
            }
            for l in detail_lines {
                flow.rows.push(Row {
                    text: l,
                    x: label_x,
                    y: ly,
                    px: DETAIL_PT * s,
                    color: dim,
                    mono: false,
                });
                ly += line_h(DETAIL_PT * s);
            }
            flow.taps.push((hit(tap), r));
            *ty += h + ROW_GAP * s;
        };
        for (i, o) in q.options.iter().enumerate() {
            let checked = q.multi.then(|| draft.checked(i));
            // The label and detail from the libcontrols row's cells.
            let (label, detail) = match draft.rows.get(i) {
                Some(r) => (
                    r.cells
                        .first()
                        .map_or(r.item.label.as_str(), |c| c.text.as_str()),
                    r.cells
                        .iter()
                        .find(|c| c.dim)
                        .map_or("", |c| c.text.as_str()),
                ),
                None => (o.label.as_str(), o.detail.as_str()),
            };
            let picked = draft.chosen == Some(i) || checked == Some(true);
            row(
                checked,
                picked,
                label,
                fg,
                detail,
                ChoiceHit::Option(i),
                &mut ty,
                flow,
            );
            // Its preview, folded or whole, under it.
            if let Some(p) = o.preview.as_deref().filter(|p| !p.trim().is_empty()) {
                let px = PREVIEW_PT * s;
                let cols = term_columns(inner_w - 2.0 * ROW_PAD * s, px);
                let max = if state.expanded.contains(&(q_index, i)) {
                    preview_max.clamp(PREVIEW_FOLDED, PREVIEW_LINES)
                } else {
                    PREVIEW_FOLDED
                };
                let lines = preview_lines(p, cols, max);
                let h = lines.len() as f32 * term_h(px) + 2.0 * 6.0 * s;
                let r = Rect::new(inner_x, ty - ROW_GAP * s * 0.5, inner_w, h);
                flow.fills.push(Fill {
                    rect: r,
                    color: with_alpha(theme().band, 0.9),
                    radius: 6.0 * s,
                });
                let mut ly = r.y + 6.0 * s;
                for l in lines {
                    flow.rows.push(Row {
                        text: l,
                        x: inner_x + ROW_PAD * s,
                        y: ly,
                        px,
                        color: if state.sending { dim } else { theme().code },
                        mono: true,
                    });
                    ly += term_h(px);
                }
                flow.taps.push((hit(ChoiceHit::Preview(i)), r));
                ty = r.y + r.h + ROW_GAP * s;
            }
        }
        if q.other {
            let (label, color) = match &draft.other {
                Some(t) => (format!("\u{201c}{t}\u{201d}"), fg),
                None => (copy::CHOICE_OTHER.text.to_string(), dim),
            };
            let checked = q.multi.then_some(draft.other.is_some());
            let picked = draft.other.is_some();
            row(
                checked,
                picked,
                &label,
                color,
                "",
                ChoiceHit::Other,
                &mut ty,
                flow,
            );
        }
    }

    // The line: sending, or a result's, a refusal's or the card's own.
    let line = if state.sending {
        Some((copy::CHOICE_SENDING.text.to_string(), theme().dim))
    } else {
        state.line.as_ref().map(|l| (l.text.clone(), l.color))
    };
    if let Some((text, color)) = line {
        text_rows(&text, DETAIL_PT * s, color, inner_x, inner_w, &mut ty, flow);
        ty += 4.0 * s;
    }

    // The buttons: Done; or the review's Back and Submit.
    let gap = 12.0 * s;
    let bw = (inner_w - gap) / 2.0;
    let disabled = with_alpha(theme().dim, 0.35);
    if state.review {
        let back = FlowButton {
            hit: hit(ChoiceHit::Back),
            rect: Rect::new(inner_x, ty + pad * 0.25, bw, ACTION_H * s),
            label: copy::CHOICE_BACK.text.into(),
            bg: if state.sending {
                disabled
            } else {
                theme().back
            },
        };
        let submit = FlowButton {
            hit: hit(ChoiceHit::Submit),
            rect: Rect::new(inner_x + bw + gap, ty + pad * 0.25, bw, ACTION_H * s),
            label: copy::CHOICE_SUBMIT.text.into(),
            bg: if state.sending {
                disabled
            } else {
                theme().approve
            },
        };
        flow.buttons.push(back);
        flow.buttons.push(submit);
        ty += pad * 0.25 + ACTION_H * s;
    } else if q.multi {
        let enabled = state.done_enabled() && !state.sending;
        flow.buttons.push(FlowButton {
            hit: hit(ChoiceHit::Done),
            rect: Rect::new(inner_x + inner_w - bw, ty + pad * 0.25, bw, ACTION_H * s),
            label: copy::CHOICE_DONE.text.into(),
            bg: if enabled { theme().approve } else { disabled },
        });
        ty += pad * 0.25 + ACTION_H * s;
    } else {
        ty -= ROW_GAP * s;
    }
    let h = ty + pad - y;
    for f in &mut flow.fills[shadow..shadow + 3] {
        f.rect.h = h;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_proto::common::ChoiceOption;

    fn option(label: &str, detail: &str) -> ChoiceOption {
        ChoiceOption::new(label, detail, None)
    }

    /// The test's two questions: single then multi, both with Other.
    pub(crate) fn two() -> ChoiceSet {
        ChoiceSet::new(
            "toolu_1",
            vec![
                ChoiceQuestion::new(
                    "Colour",
                    "Which colour should the button be?",
                    false,
                    true,
                    vec![option("Red", "warm, loud"), option("Blue", "calm")],
                ),
                ChoiceQuestion::new(
                    "Extras",
                    "Which extras?",
                    true,
                    true,
                    vec![
                        option("Nuts", ""),
                        option("Cream", "whipped"),
                        option("Fudge", ""),
                    ],
                ),
            ],
            "claude_code/AskUserQuestion",
        )
    }

    fn single() -> ChoiceSet {
        ChoiceSet::new(
            "toolu_2",
            vec![ChoiceQuestion::new(
                "Go",
                "Go ahead?",
                false,
                true,
                vec![option("Yes", ""), option("No", "")],
            )],
            "claude_code/AskUserQuestion",
        )
    }

    #[test]
    fn single_select_moves_on_and_the_review_submits_the_indexes() {
        let set = two();
        let mut c = ChoiceCard::for_set(&set);
        assert_eq!(c.tap(ChoiceHit::Option(1), &set), None);
        assert_eq!(c.question(), 1, "on to the next question");
        // Done is disabled with nothing checked.
        assert!(!c.done_enabled());
        assert_eq!(c.tap(ChoiceHit::Done, &set), None);
        assert_eq!(c.question(), 1);
        // Toggles, in the order checked; unchecking takes one back.
        c.tap(ChoiceHit::Option(2), &set);
        c.tap(ChoiceHit::Option(0), &set);
        c.tap(ChoiceHit::Option(1), &set);
        c.tap(ChoiceHit::Option(1), &set);
        assert!(c.is_picked(1, 2) && c.is_picked(1, 0) && !c.is_picked(1, 1));
        assert_eq!(c.rows(1)[2].item.checked, Some(true));
        assert_eq!(c.rows(1)[1].item.checked, Some(false));
        assert!(c.done_enabled());
        assert_eq!(c.tap(ChoiceHit::Done, &set), None);
        assert!(c.in_review());
        assert_eq!(
            c.review_lines(&set),
            ["Colour: Blue", "Extras: Fudge, Nuts"]
        );
        // Back keeps the answers.
        c.tap(ChoiceHit::Back, &set);
        assert!(!c.in_review());
        c.tap(ChoiceHit::Back, &set);
        assert_eq!(c.question(), 0);
        assert!(c.is_picked(0, 1));
        c.tap(ChoiceHit::Option(1), &set);
        c.tap(ChoiceHit::Done, &set);
        assert!(c.in_review());
        let sent = c.tap(ChoiceHit::Submit, &set);
        assert_eq!(
            sent,
            Some(ChoiceAction::Send {
                id: "toolu_1".into(),
                answers: vec![
                    ChoiceAnswer {
                        picked: vec![1],
                        other: None
                    },
                    ChoiceAnswer {
                        picked: vec![2, 0],
                        other: None
                    },
                ],
            })
        );
        assert!(c.sending());
        // Taps while sending do nothing.
        assert_eq!(c.tap(ChoiceHit::Back, &set), None);
        assert!(c.in_review());
    }

    #[test]
    fn a_single_question_sends_on_its_tap_with_no_review() {
        let set = single();
        let mut c = ChoiceCard::for_set(&set);
        let sent = c.tap(ChoiceHit::Option(0), &set);
        assert_eq!(
            sent,
            Some(ChoiceAction::Send {
                id: "toolu_2".into(),
                answers: vec![ChoiceAnswer {
                    picked: vec![0],
                    other: None
                }],
            })
        );
        assert!(!c.in_review());
    }

    #[test]
    fn other_opens_the_typed_card_and_its_text_returns_to_the_row() {
        let set = two();
        let mut c = ChoiceCard::for_set(&set);
        assert_eq!(
            c.tap(ChoiceHit::Other, &set),
            Some(ChoiceAction::Other {
                label: "Other: Colour".into(),
                text: String::new()
            })
        );
        // A `/` first is refused on the card, and nothing moves.
        assert_eq!(c.other_given("/compact", &set), None);
        assert_eq!(c.line().unwrap().text, "An answer can't start with /");
        assert_eq!(c.question(), 0);
        assert_eq!(c.other(0), None);
        // Given: the answer, cleaned, and on to the next.
        assert_eq!(c.other_given("  teal,  please. ", &set), None);
        assert_eq!(c.other(0), Some("teal, please"));
        assert_eq!(c.question(), 1);
        assert!(c.line().is_none());
        // Multi: one more checked row; Done enabled by it alone.
        assert!(!c.done_enabled());
        c.other_given("sprinkles", &set);
        assert_eq!(c.question(), 1);
        assert!(c.done_enabled());
        // Asked again, the typed card holds it.
        assert_eq!(
            c.tap(ChoiceHit::Other, &set),
            Some(ChoiceAction::Other {
                label: "Other: Extras".into(),
                text: "sprinkles".into()
            })
        );
        c.tap(ChoiceHit::Done, &set);
        assert_eq!(
            c.review_lines(&set),
            [
                "Colour: \u{201c}teal, please\u{201d}",
                "Extras: \u{201c}sprinkles\u{201d}"
            ]
        );
        let Some(ChoiceAction::Send { answers, .. }) = c.tap(ChoiceHit::Submit, &set) else {
            panic!("sent");
        };
        assert_eq!(answers[0].picked, Vec::<usize>::new());
        assert_eq!(answers[0].other.as_deref(), Some("teal, please"));
        assert_eq!(answers[1].other.as_deref(), Some("sprinkles"));
    }

    #[test]
    fn other_texts_are_checked_as_the_daemon_checks_messages() {
        assert_eq!(check_other("ok then.").unwrap(), "ok then");
        assert!(check_other("two\nlines").is_err());
        assert!(check_other("   ").is_err());
        assert!(check_other("...").is_err());
        for mark in ["/", "!", "#"] {
            assert!(check_other(&format!("{mark}x")).is_err(), "{mark}");
        }
        assert!(check_other(&"a".repeat(OTHER_MAX)).is_ok());
        assert!(check_other(&"a".repeat(OTHER_MAX + 1)).is_err());
    }

    #[test]
    fn a_new_set_starts_again_and_the_same_keeps_its_answers() {
        let set = two();
        let mut c = ChoiceCard::for_set(&set);
        c.tap(ChoiceHit::Option(0), &set);
        assert!(!c.sync(Some(&set)));
        assert_eq!(c.question(), 1);
        assert!(c.sync(Some(&single())));
        assert_eq!((c.id(), c.question()), ("toolu_2", 0));
        assert!(c.sync(None));
        assert_eq!(c.id(), "");
    }

    #[test]
    fn each_result_leaves_its_line() {
        let set = single();
        let line = |outcome: &str, got: &[&str], code: Option<i64>| {
            let mut c = ChoiceCard::for_set(&set);
            c.tap(ChoiceHit::Option(0), &set);
            let got: Vec<String> = got.iter().map(|g| g.to_string()).collect();
            c.answered("toolu_2", outcome, &got, code, "Claude");
            assert!(!c.sending());
            c.line().map(|l| l.text.clone())
        };
        assert_eq!(line("answered", &[], None), None);
        assert_eq!(
            line("mismatch", &["Go: No", "Size: L"], None).unwrap(),
            "Claude recorded: Go: No; Size: L"
        );
        let finish = "Not answered: finish on the Mac";
        assert_eq!(line("no_result", &[], None).unwrap(), finish);
        assert_eq!(
            line(
                "failed",
                &["question 2 did not appear"],
                Some(codes::SCREEN_MISMATCH)
            )
            .unwrap(),
            finish
        );
        assert_eq!(
            line("failed", &["stale"], Some(codes::CHOICE_STALE)).unwrap(),
            "Already answered or closed on the Mac"
        );
        assert!(
            line("failed", &["x"], Some(codes::CHOICE_UNSUPPORTED))
                .unwrap()
                .starts_with("This agent's questions")
        );
        assert_eq!(
            line("failed", &["messages are off"], Some(codes::NOT_PERMITTED)).unwrap(),
            "Not allowed: messages are off"
        );
        assert_eq!(
            line("failed", &["the connection closed"], None).unwrap(),
            "Not sent: the connection closed"
        );
        // Another set's result is not this card's.
        let mut c = ChoiceCard::for_set(&set);
        c.tap(ChoiceHit::Option(0), &set);
        c.answered("toolu_9", "no_result", &[], None, "Claude");
        assert!(c.sending());
    }

    #[test]
    fn a_preview_folds_to_three_lines_and_expands() {
        let text = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let folded = preview_lines(&text, 40, PREVIEW_FOLDED);
        assert_eq!(folded, ["line 1", "line 2", "line 3\u{2026}"]);
        assert_eq!(preview_lines(&text, 40, PREVIEW_LINES).len(), 10);
        assert_eq!(preview_lines("a\nb", 40, PREVIEW_FOLDED), ["a", "b"]);
        // Monospace lines are cut at the column, not wrapped at words.
        assert_eq!(preview_lines("abcdefghij", 4, 9), ["abcd", "efgh", "ij"]);
        // A tap expands it and a tap folds it; the row still chooses.
        let mut set = single();
        set.questions[0].options[0].preview = Some(text.clone());
        set.questions[0].other = false;
        let mut c = ChoiceCard::for_set(&set);
        assert_eq!(c.tap(ChoiceHit::Preview(0), &set), None);
        assert!(c.expanded.contains(&(0, 0)));
        c.tap(ChoiceHit::Preview(0), &set);
        assert!(!c.expanded.contains(&(0, 0)));
        assert!(matches!(
            c.tap(ChoiceHit::Option(0), &set),
            Some(ChoiceAction::Send { .. })
        ));
    }
}
