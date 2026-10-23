//! An agent's pending question (plan 20261022_03, decision 8).
//!
//! The final paragraph of the agent's last assistant `Text` entry is its
//! pending question when:
//! - the paragraph ends with `?`, or has a line that does;
//! - no user entry follows that entry; and
//! - the pane's `agent_status` is `idle` or `blocked`.
//!
//! The question is redacted, then cut to `PENDING_QUESTION_MAX`
//! characters.

use libherdr::protocol::AgentStatus;

use super::redact::redact;
use super::{EntryKind, Item, PENDING_QUESTION_MAX, PendingQuestion, Role, cut};

/// The pending question in `items` (oldest first) for an agent whose
/// status is `status`, if it has one.
pub fn pending_question(items: &[Item], status: &AgentStatus) -> Option<PendingQuestion> {
    if !matches!(status, AgentStatus::Idle | AgentStatus::Blocked) {
        return None;
    }
    let last = items
        .iter()
        .rposition(|i| i.role == Role::Assistant && i.kind == EntryKind::Text)?;
    if items[last + 1..].iter().any(|i| i.role == Role::User) {
        return None;
    }
    let paragraph = final_paragraph(&items[last].text)?;
    if !asks(paragraph) {
        return None;
    }
    Some(PendingQuestion {
        text: cut(&redact(paragraph), PENDING_QUESTION_MAX),
        ts: items[last].ts.clone(),
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
            ts: ts.to_string(),
            role,
            kind: EntryKind::Text,
            tool: None,
            text: text.to_string(),
            call_id: None,
        }
    }

    fn tool(role: Role, kind: EntryKind) -> Item {
        Item {
            ts: "t".to_string(),
            role,
            kind,
            tool: Some("Bash".to_string()),
            text: "Bash {}".to_string(),
            call_id: None,
        }
    }

    #[test]
    fn the_last_reply_ending_in_a_question_is_pending() {
        // The shape of the 2026-10-23 transcript's end: a long reply whose
        // last paragraph asks.
        let path = fixture("claude_question.jsonl");
        let items = ClaudeCode::new(fixture("x")).tail(&path, 50).unwrap();
        let q = pending_question(&items, &AgentStatus::Idle).unwrap();
        assert_eq!(
            q.text,
            "Should the notices wait for the phone to finish speaking, or interrupt it?"
        );
        assert_eq!(q.ts, "2026-01-04T07:15:00.000Z");
        // Blocked counts too; working and done do not.
        assert!(pending_question(&items, &AgentStatus::Blocked).is_some());
        assert!(pending_question(&items, &AgentStatus::Working).is_none());
        assert!(pending_question(&items, &AgentStatus::Done).is_none());
    }

    #[test]
    fn a_following_user_entry_clears_it() {
        let mut items = vec![
            text(Role::User, "t1", "do it"),
            text(Role::Assistant, "t2", "Done.\n\nShall I push?"),
        ];
        assert!(pending_question(&items, &AgentStatus::Idle).is_some());
        // Tool entries after it do not clear it.
        items.push(tool(Role::Assistant, EntryKind::ToolUse));
        items.push(tool(Role::Tool, EntryKind::ToolResult));
        assert!(pending_question(&items, &AgentStatus::Idle).is_some());
        items.push(text(Role::User, "t3", "yes"));
        assert_eq!(pending_question(&items, &AgentStatus::Idle), None);
    }

    #[test]
    fn only_the_final_paragraph_counts() {
        let asked_earlier = vec![text(
            Role::Assistant,
            "t",
            "Did that work?\n\nIt did. Everything is committed.",
        )];
        assert_eq!(pending_question(&asked_earlier, &AgentStatus::Idle), None);
        // A line of the final paragraph that asks, with markdown after it.
        let list = vec![text(
            Role::Assistant,
            "t",
            "Summary.\n\n1. **Keep the old name?**\n2. Rename it everywhere.",
        )];
        assert_eq!(
            pending_question(&list, &AgentStatus::Blocked).unwrap().text,
            "1. **Keep the old name?**\n2. Rename it everywhere."
        );
        assert_eq!(
            pending_question(
                &[text(Role::Assistant, "t", "\n\n  \n")],
                &AgentStatus::Idle
            ),
            None
        );
        assert_eq!(pending_question(&[], &AgentStatus::Idle), None);
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
        assert_eq!(pending_question(&items, &AgentStatus::Idle), None);
        let items = vec![text(
            Role::Assistant,
            "t",
            &format!("Is {key} the key? {} Or another?", "x".repeat(600)),
        )];
        let q = pending_question(&items, &AgentStatus::Idle).unwrap();
        assert!(!q.text.contains(key));
        assert_eq!(q.text.chars().count(), PENDING_QUESTION_MAX);
        assert!(q.text.ends_with('…'));
    }
}
