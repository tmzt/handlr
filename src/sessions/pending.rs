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
//! characters. It is marked `about_plan`, with its plan's spoken name,
//! by [`super::links::plan_question`] (the 21:57 EDT amendment).

use libherdr::protocol::AgentStatus;

use super::redact::redact;
use super::{EntryKind, Item, PENDING_QUESTION_MAX, PendingQuestion, Role, cut};

/// How long the session file must be unwritten before a final reply counts
/// as the end of the turn whatever herdr's status.
pub const TURN_QUIET: std::time::Duration = std::time::Duration::from_secs(5);

/// The pending question in `items` (oldest first) for an agent whose
/// status is `status`, if it has one. `quiet` is whether the session file
/// has been unwritten for [`TURN_QUIET`].
pub fn pending_question(
    items: &[Item],
    status: &AgentStatus,
    quiet: bool,
) -> Option<PendingQuestion> {
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
    question_in(&items[last])
}

/// The question a reply asks, whether or not it waits: its final
/// paragraph when that asks, redacted, cut and marked about a plan. The
/// sections' resolved Question cards come from it (plan 20261023_03, A3).
pub fn question_in(reply: &Item) -> Option<PendingQuestion> {
    let paragraph = final_paragraph(&reply.text)?;
    if !asks(paragraph) {
        return None;
    }
    let redacted = redact(paragraph);
    // An id the question lacks may be earlier in the reply.
    let (about_plan, plan) = super::links::plan_question(&redacted, &redact(&reply.text));
    Some(PendingQuestion {
        text: cut(&redacted, PENDING_QUESTION_MAX),
        ts: reply.ts.clone(),
        about_plan,
        plan,
        links: Vec::new(),
        answers: Vec::new(),
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
        }
    }

    #[test]
    fn the_last_reply_ending_in_a_question_is_pending() {
        // The shape of the 2026-10-23 transcript's end: a long reply whose
        // last paragraph asks.
        let path = fixture("claude_question.jsonl");
        let items = ClaudeCode::new(fixture("x")).tail(&path, 50).unwrap();
        let q = pending_question(&items, &AgentStatus::Idle, false).unwrap();
        assert_eq!(
            q.text,
            "Should the notices wait for the phone to finish speaking, or interrupt it?"
        );
        assert_eq!(q.ts, "2026-01-04T07:15:00.000Z");
        // Blocked counts too; working does not.
        assert!(pending_question(&items, &AgentStatus::Blocked, false).is_some());
        assert!(pending_question(&items, &AgentStatus::Working, false).is_none());
        // A task that ends with a question: herdr says done until it is seen.
        assert!(pending_question(&items, &AgentStatus::Done, false).is_some());
    }

    #[test]
    fn a_following_user_entry_clears_it() {
        let mut items = vec![
            text(Role::User, "t1", "do it"),
            text(Role::Assistant, "t2", "Done.\n\nShall I push?"),
        ];
        assert!(pending_question(&items, &AgentStatus::Idle, false).is_some());
        // Tool entries after it do not clear it.
        items.push(tool(Role::Assistant, EntryKind::ToolUse));
        items.push(tool(Role::Tool, EntryKind::ToolResult));
        assert!(pending_question(&items, &AgentStatus::Idle, false).is_some());
        items.push(text(Role::User, "t3", "yes"));
        assert_eq!(pending_question(&items, &AgentStatus::Idle, false), None);
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
            pending_question(&items, &AgentStatus::Idle, false)
                .unwrap()
                .text,
            "Shall I push?"
        );
        // Quiet with only context after the reply: the turn has ended.
        assert!(pending_question(&items, &AgentStatus::Working, true).is_some());
        // The user's own words answer it.
        items.push(text(Role::User, "t4", "yes"));
        assert_eq!(pending_question(&items, &AgentStatus::Idle, false), None);
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
        let q = pending_question(&items, &AgentStatus::Idle, false).unwrap();
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
            pending_question(&asked_earlier, &AgentStatus::Idle, false),
            None
        );
        // A line of the final paragraph that asks, with markdown after it.
        let list = vec![text(
            Role::Assistant,
            "t",
            "Summary.\n\n1. **Keep the old name?**\n2. Rename it everywhere.",
        )];
        assert_eq!(
            pending_question(&list, &AgentStatus::Blocked, false)
                .unwrap()
                .text,
            "1. **Keep the old name?**\n2. Rename it everywhere."
        );
        assert_eq!(
            pending_question(
                &[text(Role::Assistant, "t", "\n\n  \n")],
                &AgentStatus::Idle,
                false
            ),
            None
        );
        assert_eq!(pending_question(&[], &AgentStatus::Idle, false), None);
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
        assert_eq!(pending_question(&items, &AgentStatus::Idle, false), None);
        let items = vec![text(
            Role::Assistant,
            "t",
            &format!("Is {key} the key? {} Or another?", "x".repeat(600)),
        )];
        let q = pending_question(&items, &AgentStatus::Idle, false).unwrap();
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
            pending_question(&asked, &AgentStatus::Working, true)
                .unwrap()
                .text,
            "Install it on the S23?"
        );
        // Not quiet yet: still working, maybe.
        assert!(pending_question(&asked, &AgentStatus::Working, false).is_none());
        // Quiet, but the reply is not the last entry: the agent went on.
        let went_on = [
            asked[0].clone(),
            asked[1].clone(),
            tool(Role::Assistant, EntryKind::ToolUse),
        ];
        assert!(pending_question(&went_on, &AgentStatus::Working, true).is_none());
    }

    #[test]
    fn a_question_about_a_plan_says_so_and_names_it() {
        let asked = [text(
            Role::Assistant,
            "2",
            "Wrote the amendment.\n\nApprove plan 04 and build it now?",
        )];
        let q = pending_question(&asked, &AgentStatus::Idle, false).unwrap();
        assert!(q.about_plan);
        assert_eq!(q.plan.as_deref(), Some("plan 04"));
        assert!(q.links.is_empty(), "links are the server's to add");
        let plain = [text(Role::Assistant, "2", "Install it on the S23?")];
        let q = pending_question(&plain, &AgentStatus::Idle, false).unwrap();
        assert!(!q.about_plan);
        assert_eq!(q.plan, None);
    }
}
