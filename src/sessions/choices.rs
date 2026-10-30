//! Answering a choice question with the agent's own keys
//! (docs/design/20261029_01_CHOICE_QUESTIONS_PLAN.md, decision 5).
//!
//! For Claude Code's `AskUserQuestion` (`claude_code/AskUserQuestion`), an
//! answer becomes a list of [`Step`]s: screen checks, keys and literal
//! text, one herdr call each, [`STEP_GAP`] apart. With N a question's
//! option count and k a 1-based option number:
//!
//! | Case | Keys, per question |
//! |---|---|
//! | A. Single-select, option k | `k` |
//! | B. Single-select, Other text T | `N+1`; T as text; `enter` |
//! | C. Multi-select, options k1…km | each `ki`; `down` N+1 times; `enter` |
//! | D. Multi-select with Other T | each `ki`; `down` N times; T as text; `down`; `enter` |
//! | E. Preview question, option k | `k`; `enter` |
//! | F. After the last question, with two or more questions or one multi-select | `1` on the review |
//!
//! `escape`, Tab and ← → are never sent in an answer. A cancel
//! ([`cancel_plan`], `choice.cancel`, the plan's 2026-10-30 16:42 EDT
//! amendment) is question 1's screen check and one `escape`, which rejects
//! the whole call. Before each question's keys and
//! before the review's `1`, the visible frame is read until it shows what
//! the step expects ([`Step::Question`], [`Step::Review`]); a check that
//! fails stops the answer there, the screen left for the Mac. After the
//! last key the session's new result is compared with what was sent
//! ([`compare`]).

use std::time::Duration;

use async_trait::async_trait;
use libflankr_proto::common::{
    CHOICE_RESULT_WAIT_S, ChoiceAnswer, ChoiceAnswerResult, ChoiceQuestion, ChoiceSet,
};

use super::ChoiceOutcome;
use super::claude::ASK_SOURCE;

/// The pause between two herdr calls of an answer.
pub const STEP_GAP: Duration = Duration::from_millis(150);
/// How often a screen check reads the frame.
pub const CHECK_POLL: Duration = Duration::from_millis(100);
/// How long a screen check waits for its frame.
pub const CHECK_FOR: Duration = Duration::from_secs(1);
/// How often the session file is read for the result.
pub const RESULT_POLL: Duration = Duration::from_millis(100);
/// How long the result is waited for after the last key.
pub const RESULT_WAIT: Duration = Duration::from_secs(CHOICE_RESULT_WAIT_S);
/// How much of a question's text a screen check looks for, in characters.
pub const QUESTION_NEEDLE: usize = 40;
/// The Claude Code versions the key table was written against: the session
/// history from the first, the shipped client at the last (findings 1 and
/// 2). Others are driven and logged as untested.
pub const TESTED_FIRST: (u32, u32, u32) = (2, 1, 227);
pub const TESTED_LAST: (u32, u32, u32) = (2, 1, 312);

/// The hint every question screen shows (`Enter to select · …`).
const HINT: &str = "to select";
/// The empty Other row's placeholder (`Type something.`, or `Type
/// something` for multi-select).
const OTHER_EMPTY: &str = "Type something";
/// The focus pointer on the first row.
const FIRST_FOCUSED: &str = "❯ 1.";
const REVIEW: [&str; 2] = ["Review your answers", "Ready to submit your answers?"];

/// One step of an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Wait for question `index` (0-based): `needle` (its text's first
    /// [`QUESTION_NEEDLE`] characters, folded) and the hint; with
    /// `other_empty`, the empty Other row's placeholder; with
    /// `first_focused`, the focus pointer on row 1.
    Question {
        index: usize,
        needle: String,
        other_empty: bool,
        first_focused: bool,
    },
    /// Wait for the review screen.
    Review,
    /// One key, by herdr's name (`1`, `down`, `enter`).
    Key(String),
    /// Literal text, with no Enter (herdr's `pane.send_text`).
    Text(String),
}

/// Why an answer cannot be planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The answers do not fit the questions (`INVALID_PARAMS`).
    Invalid(String),
    /// The question's `source` has no driver (`CHOICE_UNSUPPORTED`).
    Unsupported(String),
}

/// Whether `source` has a driver.
pub fn supported(source: &str) -> bool {
    source == ASK_SOURCE
}

/// Whether question `q` is answered on Claude Code's side-by-side preview
/// screen: single-select with an option that has a preview.
fn is_preview(q: &ChoiceQuestion) -> bool {
    !q.multi && q.options.iter().any(|o| o.preview.is_some())
}

/// Check `answers` against `set`'s questions: one answer per question; a
/// single-choice answer exactly one `picked` index or an `other`; a multi
/// answer a non-empty mix of distinct indexes and an `other`; `other` only
/// where the question has it, and not blank.
pub fn check(set: &ChoiceSet, answers: &[ChoiceAnswer]) -> Result<(), String> {
    if answers.len() != set.questions.len() {
        return Err(format!(
            "{} answers for {} questions",
            answers.len(),
            set.questions.len()
        ));
    }
    for (i, (q, a)) in set.questions.iter().zip(answers).enumerate() {
        let n = i + 1;
        if let Some(&bad) = a.picked.iter().find(|&&k| k >= q.options.len()) {
            return Err(format!("question {n} has no option {bad}"));
        }
        let mut seen = a.picked.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() != a.picked.len() {
            return Err(format!("question {n} picks an option twice"));
        }
        if let Some(other) = &a.other {
            if !q.other {
                return Err(format!("question {n} takes no Other answer"));
            }
            if other.trim().is_empty() {
                return Err(format!("question {n}'s Other answer is empty"));
            }
        }
        let given = a.picked.len() + usize::from(a.other.is_some());
        if q.multi {
            if given == 0 {
                return Err(format!("question {n} has no answer"));
            }
        } else if given != 1 {
            return Err(format!("question {n} takes one option or an Other answer"));
        }
    }
    Ok(())
}

/// The steps that give `answers` to `set` (the case table above). The
/// Other texts must already be checked and cleaned (decision 6).
pub fn plan(set: &ChoiceSet, answers: &[ChoiceAnswer]) -> Result<Vec<Step>, PlanError> {
    if !supported(&set.source) {
        return Err(PlanError::Unsupported(format!(
            "no driver for {}",
            set.source
        )));
    }
    check(set, answers).map_err(PlanError::Invalid)?;
    let mut steps = Vec::new();
    let key = |k: usize| Step::Key(k.to_string());
    for (index, (q, a)) in set.questions.iter().zip(answers).enumerate() {
        let n = q.options.len();
        let other = a.other.as_ref().map(|t| t.trim().to_string());
        steps.push(Step::Question {
            index,
            needle: needle(&q.text),
            other_empty: other.is_some(),
            first_focused: q.multi,
        });
        if q.multi {
            // C and D: digits toggle in place, focus stays on row 1.
            for &k in &a.picked {
                steps.push(key(k + 1));
            }
            match other {
                None => {
                    steps.extend((0..=n).map(|_| Step::Key("down".into())));
                }
                Some(text) => {
                    steps.extend((0..n).map(|_| Step::Key("down".into())));
                    steps.push(Step::Text(text));
                    steps.push(Step::Key("down".into()));
                }
            }
            steps.push(Step::Key("enter".into()));
        } else if let Some(text) = other {
            // B: the digit focuses the empty input; Enter submits it.
            steps.push(key(n + 1));
            steps.push(Step::Text(text));
            steps.push(Step::Key("enter".into()));
        } else if is_preview(q) {
            // E: the digit only moves focus; Enter picks.
            steps.push(key(a.picked[0] + 1));
            steps.push(Step::Key("enter".into()));
        } else {
            // A.
            steps.push(key(a.picked[0] + 1));
        }
    }
    // F: one single-select question submits by its answer.
    if set.questions.len() > 1 || set.questions[0].multi {
        steps.push(Step::Review);
        steps.push(Step::Key("1".into()));
    }
    Ok(steps)
}

/// The steps that cancel `set` (`choice.cancel`, the plan's 2026-10-30
/// 16:42 EDT amendment, item 2): the screen check before question 1 (its
/// text and the hint), then one `escape`, which rejects the whole call as
/// the agent's own dismiss does. Tab and ← → are still never sent.
pub fn cancel_plan(set: &ChoiceSet) -> Result<Vec<Step>, PlanError> {
    if !supported(&set.source) {
        return Err(PlanError::Unsupported(format!(
            "no driver for {}",
            set.source
        )));
    }
    let Some(first) = set.questions.first() else {
        return Err(PlanError::Invalid("the question has no questions".into()));
    };
    Ok(vec![
        Step::Question {
            index: 0,
            needle: needle(&first.text),
            other_empty: false,
            first_focused: false,
        },
        Step::Key("escape".into()),
    ])
}

/// The first [`QUESTION_NEEDLE`] characters of a question's text, folded.
fn needle(text: &str) -> String {
    let folded = fold(text);
    folded.chars().take(QUESTION_NEEDLE).collect::<String>()
}

/// A frame or text as the checks compare it: box-drawing characters (the
/// gutter) as spaces, and runs of whitespace as one space.
pub fn fold(text: &str) -> String {
    text.chars()
        .map(|c| {
            if ('\u{2500}'..='\u{257f}').contains(&c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a check found wanting on `screen`, or `None` when it passes.
pub fn unmet(step: &Step, screen: &str) -> Option<String> {
    let screen = fold(screen);
    match step {
        Step::Question {
            index,
            needle,
            other_empty,
            first_focused,
        } => {
            let n = index + 1;
            if !(screen.contains(needle.as_str()) && screen.contains(HINT)) {
                return Some(format!("question {n} did not appear"));
            }
            if *other_empty && !screen.contains(OTHER_EMPTY) {
                return Some(format!("the Other row of question {n} already has text"));
            }
            if *first_focused && !screen.contains(FIRST_FOCUSED) {
                return Some(format!("the first row of question {n} is not focused"));
            }
            None
        }
        Step::Review => (!REVIEW.iter().all(|r| screen.contains(r)))
            .then(|| "the review did not appear".to_string()),
        Step::Key(_) | Step::Text(_) => None,
    }
}

/// The pane an answer is driven into.
#[async_trait]
pub trait Pane: Send + Sync {
    /// The visible frame as text.
    async fn screen(&self) -> Result<String, String>;
    /// One key, by herdr's name.
    async fn key(&self, key: &str) -> Result<(), String>;
    /// Literal text, with no Enter.
    async fn text(&self, text: &str) -> Result<(), String>;
}

/// The driver's pauses; [`Timing::default`] is decision 5's.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub gap: Duration,
    pub poll: Duration,
    pub check_for: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            gap: STEP_GAP,
            poll: CHECK_POLL,
            check_for: CHECK_FOR,
        }
    }
}

/// Why driving stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveError {
    /// A screen check failed (`SCREEN_MISMATCH`), naming the step; `sent`
    /// is how many keys and texts went before it.
    Screen { step: String, sent: usize },
    /// herdr failed (`HERDR_UNAVAILABLE`).
    Herdr(String),
}

/// Run `steps` against `pane`. A check reads the frame every
/// `timing.poll` for at most `timing.check_for`; keys and texts go one per
/// call, `timing.gap` apart.
pub async fn drive(pane: &dyn Pane, steps: &[Step], timing: &Timing) -> Result<(), DriveError> {
    let mut sent = 0;
    for step in steps {
        match step {
            Step::Question { .. } | Step::Review => {
                let deadline = tokio::time::Instant::now() + timing.check_for;
                loop {
                    let screen = pane.screen().await.map_err(DriveError::Herdr)?;
                    let Some(why) = unmet(step, &screen) else {
                        break;
                    };
                    // Text already in the Other row will not go away.
                    let settled = why.contains("already has text");
                    if settled || tokio::time::Instant::now() >= deadline {
                        return Err(DriveError::Screen { step: why, sent });
                    }
                    tokio::time::sleep(timing.poll).await;
                }
            }
            Step::Key(k) => {
                if sent > 0 {
                    tokio::time::sleep(timing.gap).await;
                }
                pane.key(k).await.map_err(DriveError::Herdr)?;
                sent += 1;
            }
            Step::Text(t) => {
                if sent > 0 {
                    tokio::time::sleep(timing.gap).await;
                }
                pane.text(t).await.map_err(DriveError::Herdr)?;
                sent += 1;
            }
        }
    }
    Ok(())
}

/// What a question's recorded answer must be.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expected {
    /// A single-choice answer: the label, or the Other text.
    One(String),
    /// A multi answer: the labels and the Other text, in any order.
    Many(Vec<String>),
}

fn expected(set: &ChoiceSet, answers: &[ChoiceAnswer]) -> Vec<Expected> {
    set.questions
        .iter()
        .zip(answers)
        .map(|(q, a)| {
            let mut all: Vec<String> = a
                .picked
                .iter()
                .filter_map(|&k| q.options.get(k).map(|o| o.label.clone()))
                .collect();
            all.extend(a.other.as_ref().map(|t| t.trim().to_string()));
            if q.multi {
                Expected::Many(all)
            } else {
                Expected::One(all.into_iter().next().unwrap_or_default())
            }
        })
        .collect()
}

/// A multi-select answer as Claude Code writes it: the labels joined with
/// `", "`, a label containing `", "` or `"` written as a JSON string, the
/// Other text last (finding 2). The JSON-quoted labels are read whole;
/// whatever is left after the known `labels` is the Other text.
fn split_multi(recorded: &str, labels: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = recorded.trim();
    while !rest.is_empty() {
        let token_end = if rest.starts_with('"') {
            quoted_end(rest)
        } else {
            labels
                .iter()
                .filter(|l| {
                    rest.starts_with(**l)
                        && (rest.len() == l.len() || rest[l.len()..].starts_with(", "))
                })
                .map(|l| l.len())
                .max()
        };
        match token_end {
            Some(end) => {
                let token = &rest[..end];
                let token = if token.starts_with('"') {
                    serde_json::from_str::<String>(token).unwrap_or_else(|_| token.to_string())
                } else {
                    token.to_string()
                };
                out.push(token);
                rest = rest[end..].strip_prefix(", ").unwrap_or(&rest[end..]);
            }
            None => {
                // Not a label: the Other text, to the end.
                out.push(rest.to_string());
                break;
            }
        }
    }
    out
}

/// The end of a JSON string at the start of `s`, if it is one followed by
/// `", "` or the end.
fn quoted_end(s: &str) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => {
                let end = i + 1;
                let ok = end == s.len() || s[end..].starts_with(", ");
                return ok.then_some(end);
            }
            _ => {}
        }
    }
    None
}

/// The result of an answer whose session recorded `outcome` (decision 5):
/// `Answered` when every question's answer is as sent (a pick its label,
/// Other its text as sent, multi the set of both, order not compared);
/// else `Mismatch` with one `<header>: <answer>` line per question, or
/// `Closed on the Mac` when the call was rejected.
pub fn compare(
    set: &ChoiceSet,
    answers: &[ChoiceAnswer],
    outcome: &ChoiceOutcome,
) -> ChoiceAnswerResult {
    if outcome.rejected {
        return ChoiceAnswerResult::Mismatch {
            got: vec!["Closed on the Mac".to_string()],
        };
    }
    let recorded = outcome.answers_for(set);
    let all_equal = expected(set, answers)
        .iter()
        .zip(&recorded)
        .zip(&set.questions)
        .all(|((want, got), q)| {
            let Some(got) = got else { return false };
            match want {
                Expected::One(want) => got.trim() == want,
                Expected::Many(want) => {
                    let labels: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
                    let mut got = split_multi(got, &labels);
                    let mut want = want.clone();
                    got.sort();
                    want.sort();
                    got == want
                }
            }
        });
    if all_equal {
        return ChoiceAnswerResult::Answered;
    }
    let got = set
        .questions
        .iter()
        .zip(recorded)
        .map(|(q, a)| {
            let head = if q.header.is_empty() {
                &q.text
            } else {
                &q.header
            };
            format!("{head}: {}", a.as_deref().unwrap_or("(no answer)"))
        })
        .collect();
    ChoiceAnswerResult::Mismatch { got }
}

/// Wait for the result: `read` every `poll` until it gives one, for at
/// most `wait`. `None` when none came.
pub async fn await_result(
    read: impl Fn() -> Option<ChoiceOutcome>,
    wait: Duration,
    poll: Duration,
) -> Option<ChoiceOutcome> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if let Some(outcome) = read() {
            return Some(outcome);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(poll).await;
    }
}

/// A `major.minor.patch` version.
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Whether Claude Code `version` is one the key table was written against
/// (`TESTED_FIRST..=TESTED_LAST`). An unknown version is untested.
pub fn tested(version: Option<&str>) -> bool {
    version
        .and_then(parse_version)
        .is_some_and(|v| (TESTED_FIRST..=TESTED_LAST).contains(&v))
}

#[cfg(test)]
mod tests;
