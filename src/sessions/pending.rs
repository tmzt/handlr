//! An agent's pending question (plan 20261022_03, decision 8).
//!
//! The final paragraph of the agent's last assistant `Text` entry is its
//! pending question when:
//! - the paragraph ends with `?`, or has a line that does;
//! - no user entry follows that entry (an entry with an `origin`, such as
//!   a task notification or a recap, is context, not the user answering);
//!   and
//! - the pane's `agent_status` is `idle`, `blocked` or `done` (herdr marks a
//!   pane `done` when its agent has finished a turn the user has not yet
//!   looked at, which is when a question asked at the end of a task waits);
//!   or, whatever herdr says, the turn has visibly ended: that reply is the
//!   session's last entry but for context and the file has been quiet for
//!   [`TURN_QUIET`] (herdr reports a Claude Code pane `working` while a background agent
//!   of its runs, after its turn has ended).
//!
//! The question is redacted, then cut to `PENDING_QUESTION_MAX`
//! characters. Asked for its answers (an [`Asker`]), it gets them from
//! `SUGGESTIONS.md`'s rules (plan 20261027_06, step 3), and is marked
//! `about_plan`, with `plan` its `detected_plan_id`, when the `plan` rule
//! took it. Otherwise it has no answers and is marked by
//! [`super::links::plan_question`] (the 21:57 EDT amendment).

use libherdr::protocol::AgentStatus;

use super::redact::redact;
use libflankr_proto::common::{
    CHOICE_DETAIL_MAX, CHOICE_HEADER_MAX, CHOICE_LABEL_MAX, CHOICE_PREVIEW_MAX, ChoiceOption,
    ChoiceQuestion, ChoiceSet,
};

use super::{EntryKind, Item, PENDING_QUESTION_MAX, PendingQuestion, Role, cut};
use crate::suggestions::{Suggestions, Who};

/// Who a question's answers are for, and the rules that pick them.
#[derive(Debug, Clone, Copy)]
pub struct Asker<'a> {
    pub suggestions: &'a Suggestions,
    pub who: Who<'a>,
}

/// How long the session file must be unwritten before a final reply counts
/// as the end of the turn whatever herdr's status.
pub const TURN_QUIET: std::time::Duration = std::time::Duration::from_secs(5);

/// The pending question in `items` (oldest first) for an agent whose
/// status is `status`, if it has one. `quiet` is whether the session file
/// has been unwritten for [`TURN_QUIET`]. With an `asker`, the question
/// carries its answers.
pub fn pending_question(
    items: &[Item],
    status: &AgentStatus,
    quiet: bool,
    asker: Option<&Asker>,
) -> Option<PendingQuestion> {
    if let Some(at) = open_choice(items, status, quiet) {
        return choice_question(&items[at]);
    }
    let last = items
        .iter()
        .rposition(|i| i.role == Role::Assistant && i.kind == EntryKind::Text)?;
    let waiting = matches!(
        status,
        AgentStatus::Idle | AgentStatus::Blocked | AgentStatus::Done
    );
    // Context (a notification, a recap) after the reply neither answers
    // it nor means the agent went on.
    let after = &items[last + 1..];
    let ended = quiet && after.iter().all(|i| i.origin.is_some());
    if !waiting && !ended {
        return None;
    }
    if after.iter().any(Item::is_user_said) {
        return None;
    }
    question_for(&items[last], asker)
}

/// The index in `items` of the open choice question (plan 20261029_01,
/// decision 4): the newest tool call with `choices` whose `call_id` has no
/// result after it, when only context follows it and the pane waits or
/// the file is quiet.
pub(crate) fn open_choice(items: &[Item], status: &AgentStatus, quiet: bool) -> Option<usize> {
    let at = items
        .iter()
        .rposition(|i| i.kind == EntryKind::ToolUse && i.choices.is_some())?;
    let id = items[at].call_id.as_deref()?;
    let after = &items[at + 1..];
    if after
        .iter()
        .any(|i| i.kind == EntryKind::ToolResult && i.call_id.as_deref() == Some(id))
    {
        return None;
    }
    if !after.iter().all(|i| i.origin.is_some()) {
        return None;
    }
    let waiting = matches!(
        status,
        AgentStatus::Idle | AgentStatus::Blocked | AgentStatus::Done
    );
    (waiting || quiet).then_some(at)
}

/// The pending question of a call with `choices`: every text redacted and
/// cut to its bound, `text` the first question's. `SUGGESTIONS.md`'s rules
/// are not applied, and it has no `answers`.
pub(crate) fn choice_question(call: &Item) -> Option<PendingQuestion> {
    let set = call.choices.as_ref()?;
    let shown = |s: &str, max: usize| cut(&redact(s), max);
    let questions = set
        .questions
        .iter()
        .map(|q| {
            let options = q
                .options
                .iter()
                .map(|o| {
                    ChoiceOption::new(
                        shown(&o.label, CHOICE_LABEL_MAX),
                        shown(&o.detail, CHOICE_DETAIL_MAX),
                        o.preview.as_deref().map(|p| shown(p, CHOICE_PREVIEW_MAX)),
                    )
                })
                .collect();
            ChoiceQuestion::new(
                shown(&q.header, CHOICE_HEADER_MAX),
                shown(&q.text, PENDING_QUESTION_MAX),
                q.multi,
                q.other,
                options,
            )
        })
        .collect::<Vec<_>>();
    let text = questions.first()?.text.clone();
    Some(PendingQuestion {
        text,
        ts: call.ts.clone(),
        about_plan: false,
        plan: None,
        links: Vec::new(),
        answers: Vec::new(),
        choices: Some(ChoiceSet::new(
            set.id.clone(),
            questions,
            set.source.clone(),
        )),
    })
}

/// The question a reply asks, whether or not it waits: its final
/// paragraph when that asks, redacted, cut and marked about a plan. The
/// sections' resolved Question cards come from it (plan 20261023_03, A3).
pub fn question_in(reply: &Item) -> Option<PendingQuestion> {
    question_for(reply, None)
}

/// [`question_in`], with its answers when there is an `asker`.
pub fn question_for(reply: &Item, asker: Option<&Asker>) -> Option<PendingQuestion> {
    let paragraph = final_paragraph(&reply.text)?;
    if !asks(paragraph) {
        return None;
    }
    let redacted = redact(paragraph);
    // An id the question lacks may be earlier in the reply.
    let context = redact(&reply.text);
    let (about_plan, plan, answers) = match asker {
        Some(a) => match a.suggestions.offer(&redacted, &context, a.who) {
            Some(offered) => {
                let about_plan = offered.about_plan();
                let answers = offered.answers.into_iter().map(Into::into).collect();
                (about_plan, about_plan.then_some(offered.plan), answers)
            }
            None => (false, None, Vec::new()),
        },
        None => {
            let (about_plan, plan) = super::links::plan_question(&redacted, &context);
            (about_plan, plan, Vec::new())
        }
    };
    Some(PendingQuestion {
        text: cut(&redacted, PENDING_QUESTION_MAX),
        ts: reply.ts.clone(),
        about_plan,
        plan,
        links: Vec::new(),
        answers,
        choices: None,
    })
}

/// The last paragraph of `text`: what follows its last blank line.
fn final_paragraph(text: &str) -> Option<&str> {
    let text = text.trim_end();
    let mut start = 0;
    let mut offset = 0;
    let mut blank_seen = false;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            blank_seen = true;
        } else if blank_seen {
            start = offset;
            blank_seen = false;
        }
        offset += line.len();
    }
    let paragraph = text[start..].trim();
    (!paragraph.is_empty()).then_some(paragraph)
}

/// Whether the paragraph ends with `?` or has a line that does. Markdown
/// emphasis and closing quotes or brackets after the `?` do not count.
fn asks(paragraph: &str) -> bool {
    paragraph.lines().any(|line| {
        line.trim_end()
            .trim_end_matches(['*', '_', '`', '"', '\'', ')', ']', '”', '’'])
            .ends_with('?')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionReader;
    use crate::sessions::claude::ClaudeCode;
    use crate::sessions::test_support::fixture;

    fn text(role: Role, ts: &str, text: &str) -> Item {
        Item {
            pos: 0,
            ts: ts.to_string(),
            role,
            kind: EntryKind::Text,
            tool: None,
            text: text.to_string(),
            call_id: None,
            origin: None,
            choices: None,
            choice_result: None,
        }
    }

    fn tool(role: Role, kind: EntryKind) -> Item {
        Item {
            pos: 0,
            ts: "t".to_string(),
            role,
            kind,
            tool: Some("Bash".to_string()),
            text: "Bash {}".to_string(),
            call_id: None,
            origin: None,
            choices: None,
            choice_result: None,
        }
    }

    #[test]
    fn the_last_reply_ending_in_a_question_is_pending() {
        // The shape of the 2026-10-23 transcript's end: a long reply whose
        // last paragraph asks.
        let path = fixture("claude_question.jsonl");
        let items = ClaudeCode::new(fixture("x")).tail(&path, 50).unwrap();
        let q = pending_question(&items, &AgentStatus::Idle, false, None).unwrap();
        assert_eq!(
            q.text,
            "Should the notices wait for the phone to finish speaking, or interrupt it?"
        );
        assert_eq!(q.ts, "2026-01-04T07:15:00.000Z");
        // Blocked counts too; working does not.
        assert!(pending_question(&items, &AgentStatus::Blocked, false, None).is_some());
        assert!(pending_question(&items, &AgentStatus::Working, false, None).is_none());
        // A task that ends with a question: herdr says done until it is seen.
        assert!(pending_question(&items, &AgentStatus::Done, false, None).is_some());
    }

    #[test]
    fn a_following_user_entry_clears_it() {
        let mut items = vec![
            text(Role::User, "t1", "do it"),
            text(Role::Assistant, "t2", "Done.\n\nShall I push?"),
        ];
        assert!(pending_question(&items, &AgentStatus::Idle, false, None).is_some());
        // Tool entries after it do not clear it.
        items.push(tool(Role::Assistant, EntryKind::ToolUse));
        items.push(tool(Role::Tool, EntryKind::ToolResult));
        assert!(pending_question(&items, &AgentStatus::Idle, false, None).is_some());
        items.push(text(Role::User, "t3", "yes"));
        assert_eq!(
            pending_question(&items, &AgentStatus::Idle, false, None),
            None
        );
    }

    #[test]
    fn context_after_the_question_does_not_answer_it() {
        let mut items = vec![
            text(Role::User, "t1", "do it"),
            text(Role::Assistant, "t2", "Done.\n\nShall I push?"),
        ];
        for origin in ["task", "peer:docs-writer", "agent:a1", "recap", "compact"] {
            let mut notice = text(Role::User, "t3", "Build finished?");
            notice.origin = Some(origin.to_string());
            items.push(notice);
        }
        assert_eq!(
            pending_question(&items, &AgentStatus::Idle, false, None)
                .unwrap()
                .text,
            "Shall I push?"
        );
        // Quiet with only context after the reply: the turn has ended.
        assert!(pending_question(&items, &AgentStatus::Working, true, None).is_some());
        // The user's own words answer it.
        items.push(text(Role::User, "t4", "yes"));
        assert_eq!(
            pending_question(&items, &AgentStatus::Idle, false, None),
            None
        );
    }

    #[test]
    fn a_task_notification_read_from_the_file_leaves_the_question_pending() {
        // The context fixture ends on a question; put a queued notification
        // after it.
        let mut body = std::fs::read_to_string(fixture("claude_context.jsonl")).unwrap();
        body.push_str(concat!(
            r#"{"parentUuid":"u","isSidechain":false,"attachment":{"type":"queued_command","#,
            r#""prompt":"<task-notification>\n<summary>Tests passed</summary>\n</task-notification>","#,
            r#""commandMode":"task-notification","origin":{"kind":"task-notification"},"#,
            r#""timestamp":"2026-01-05T09:00:20.000Z"},"type":"attachment"}"#,
            "\n"
        ));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        std::fs::write(&path, body).unwrap();
        let items = ClaudeCode::new(fixture("x")).tail(&path, 50).unwrap();
        assert_eq!(items.last().unwrap().origin.as_deref(), Some("task"));
        let q = pending_question(&items, &AgentStatus::Idle, false, None).unwrap();
        assert_eq!(q.text, "Done. Shall I push?");
    }

    #[test]
    fn only_the_final_paragraph_counts() {
        let asked_earlier = vec![text(
            Role::Assistant,
            "t",
            "Did that work?\n\nIt did. Everything is committed.",
        )];
        assert_eq!(
            pending_question(&asked_earlier, &AgentStatus::Idle, false, None),
            None
        );
        // A line of the final paragraph that asks, with markdown after it.
        let list = vec![text(
            Role::Assistant,
            "t",
            "Summary.\n\n1. **Keep the old name?**\n2. Rename it everywhere.",
        )];
        assert_eq!(
            pending_question(&list, &AgentStatus::Blocked, false, None)
                .unwrap()
                .text,
            "1. **Keep the old name?**\n2. Rename it everywhere."
        );
        assert_eq!(
            pending_question(
                &[text(Role::Assistant, "t", "\n\n  \n")],
                &AgentStatus::Idle,
                false,
                None
            ),
            None
        );
        assert_eq!(pending_question(&[], &AgentStatus::Idle, false, None), None);
    }

    #[test]
    fn the_question_is_redacted_and_cut() {
        let key = "AIzaSyD0nTuSeTh1sFakeKeyAbCdEfGhIjKlMn";
        let items = vec![text(
            Role::Assistant,
            "t",
            &format!("Is {key} the key you meant? {}", "x".repeat(600)),
        )];
        // The `?` is mid-line: this line does not end with one.
        assert_eq!(
            pending_question(&items, &AgentStatus::Idle, false, None),
            None
        );
        let items = vec![text(
            Role::Assistant,
            "t",
            &format!("Is {key} the key? {} Or another?", "x".repeat(600)),
        )];
        let q = pending_question(&items, &AgentStatus::Idle, false, None).unwrap();
        assert!(!q.text.contains(key));
        assert_eq!(q.text.chars().count(), PENDING_QUESTION_MAX);
        assert!(q.text.ends_with('…'));
    }

    #[test]
    fn a_question_ending_a_quiet_turn_is_pending_while_herdr_says_working() {
        // herdr says working while a background agent runs; the session
        // shows the turn ended on a question.
        let asked = [
            text(Role::User, "1", "go"),
            text(Role::Assistant, "2", "Built it.\n\nInstall it on the S23?"),
        ];
        assert_eq!(
            pending_question(&asked, &AgentStatus::Working, true, None)
                .unwrap()
                .text,
            "Install it on the S23?"
        );
        // Not quiet yet: still working, maybe.
        assert!(pending_question(&asked, &AgentStatus::Working, false, None).is_none());
        // Quiet, but the reply is not the last entry: the agent went on.
        let went_on = [
            asked[0].clone(),
            asked[1].clone(),
            tool(Role::Assistant, EntryKind::ToolUse),
        ];
        assert!(pending_question(&went_on, &AgentStatus::Working, true, None).is_none());
    }

    #[test]
    fn a_question_about_a_plan_says_so_and_names_it() {
        let asked = [text(
            Role::Assistant,
            "2",
            "Wrote the amendment.\n\nApprove plan 04 and build it now?",
        )];
        let q = pending_question(&asked, &AgentStatus::Idle, false, None).unwrap();
        assert!(q.about_plan);
        assert_eq!(q.plan.as_deref(), Some("plan 04"));
        assert!(q.links.is_empty(), "links are the server's to add");
        let plain = [text(Role::Assistant, "2", "Install it on the S23?")];
        let q = pending_question(&plain, &AgentStatus::Idle, false, None).unwrap();
        assert!(!q.about_plan);
        assert_eq!(q.plan, None);
    }
}
