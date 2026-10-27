//! Where an agent's turn stands (plan 20261023_04, the 17:04 EDT
//! amendment, A4).
//!
//! The turn is found from the reader-neutral [`Item`]s and the session
//! file's quiet, never from one agent's own format, so every
//! [`SessionReader`](super::SessionReader) gives turns:
//! - **asking** when the session has a pending question
//!   ([`pending::pending_question`]);
//! - **stopped** when the turn has visibly ended (the last entry is the
//!   agent's reply, but for context such as a notification or a recap, and
//!   the file has been quiet for [`pending::TURN_QUIET`])
//!   or herdr says `idle`, `blocked` or `done`;
//! - **working** otherwise, on the newest entry that is not context.
//!
//! Each text is redacted, then cut to `TURN_TEXT_MAX` characters.

use libherdr::protocol::AgentStatus;

pub use libflankr_proto::common::{TURN_TEXT_MAX, Turn};

use super::redact::redact;
use super::{EntryKind, Item, Role, cut, pending};

/// The turn of an agent whose session read gave `items` (oldest first)
/// and whose status is `status`. `quiet` is whether the session file has
/// been unwritten for [`pending::TURN_QUIET`].
pub fn turn(items: &[Item], status: &AgentStatus, quiet: bool) -> Turn {
    if let Some(q) = pending::pending_question(items, status, quiet, None) {
        return Turn::Asking {
            question: cut(&q.text, TURN_TEXT_MAX),
        };
    }
    let reply = items
        .iter()
        .rposition(|i| i.role == Role::Assistant && i.kind == EntryKind::Text);
    // Context after the reply (a notification, a recap) is not the agent
    // going on.
    let ended = quiet && reply.is_some_and(|r| items[r + 1..].iter().all(|i| i.origin.is_some()));
    let waiting = matches!(
        status,
        AgentStatus::Idle | AgentStatus::Blocked | AgentStatus::Done
    );
    if ended || waiting {
        let line = reply.map_or("", |r| first_line(&items[r].text));
        return Turn::Stopped { line: shown(line) };
    }
    let line = items
        .iter()
        .rfind(|i| i.origin.is_none())
        .map_or("", |i| last_line(&i.text));
    Turn::Working { line: shown(line) }
}

/// `line` as a client sees it: redacted and cut.
fn shown(line: &str) -> String {
    cut(&redact(line), TURN_TEXT_MAX)
}

/// The first line of `text` that is not blank, trimmed.
fn first_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// The last line of `text` that is not blank, trimmed.
fn last_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(role: Role, text: &str) -> Item {
        Item {
            pos: 0,
            ts: "t".to_string(),
            role,
            kind: EntryKind::Text,
            tool: None,
            text: text.to_string(),
            call_id: None,
            origin: None,
        }
    }

    fn tool_use(text: &str) -> Item {
        Item {
            pos: 0,
            ts: "t".to_string(),
            role: Role::Assistant,
            kind: EntryKind::ToolUse,
            tool: Some("Bash".to_string()),
            text: text.to_string(),
            call_id: None,
            origin: None,
        }
    }

    #[test]
    fn a_pending_question_is_asking() {
        let items = [
            text(Role::User, "go"),
            text(Role::Assistant, "Built it.\n\nInstall it on the S23?"),
        ];
        assert_eq!(
            turn(&items, &AgentStatus::Idle, false),
            Turn::Asking {
                question: "Install it on the S23?".to_string()
            }
        );
        // Quiet, whatever herdr says.
        assert!(matches!(
            turn(&items, &AgentStatus::Working, true),
            Turn::Asking { .. }
        ));
    }

    #[test]
    fn a_quiet_reply_is_stopped_while_herdr_says_working() {
        let items = [
            text(Role::User, "go"),
            text(
                Role::Assistant,
                "\nAll 12 tests pass.\n\nCommitted as `abc`.",
            ),
        ];
        let stopped = Turn::Stopped {
            line: "All 12 tests pass.".to_string(),
        };
        assert_eq!(turn(&items, &AgentStatus::Working, true), stopped);
        // herdr says the agent is waiting: stopped even while written.
        for status in [AgentStatus::Idle, AgentStatus::Blocked, AgentStatus::Done] {
            assert_eq!(turn(&items, &status, false), stopped);
        }
        // Not quiet yet, and herdr says working: still working, on the
        // newest line.
        assert_eq!(
            turn(&items, &AgentStatus::Working, false),
            Turn::Working {
                line: "Committed as `abc`.".to_string()
            }
        );
    }

    fn context(origin: &str, body: &str) -> Item {
        Item {
            origin: Some(origin.to_string()),
            ..text(Role::User, body)
        }
    }

    #[test]
    fn context_after_the_reply_is_not_the_agent_going_on() {
        let items = [
            text(Role::User, "go"),
            text(Role::Assistant, "All 12 tests pass."),
            context("recap", "You asked for the tests; they pass."),
            context("task", "Background command finished"),
        ];
        assert_eq!(
            turn(&items, &AgentStatus::Working, true),
            Turn::Stopped {
                line: "All 12 tests pass.".to_string()
            }
        );
        // Still working (not quiet): the line is the agent's, not the
        // notification's.
        assert_eq!(
            turn(&items, &AgentStatus::Working, false),
            Turn::Working {
                line: "All 12 tests pass.".to_string()
            }
        );
        // A question stays asked past a peer's message.
        let asked = [
            text(Role::Assistant, "Built it.\n\nInstall it on the S23?"),
            context("peer:docs-writer", "Rebased."),
        ];
        assert_eq!(
            turn(&asked, &AgentStatus::Idle, false),
            Turn::Asking {
                question: "Install it on the S23?".to_string()
            }
        );
    }

    #[test]
    fn a_tool_call_after_the_reply_is_working() {
        let items = [
            text(Role::Assistant, "Running the tests."),
            tool_use("Bash {\"command\":\"cargo test\"}"),
        ];
        // Quiet, but the reply is not the last entry: the agent went on.
        assert_eq!(
            turn(&items, &AgentStatus::Working, true),
            Turn::Working {
                line: "Bash {\"command\":\"cargo test\"}".to_string()
            }
        );
        assert_eq!(
            turn(&[], &AgentStatus::Working, false),
            Turn::Working {
                line: String::new()
            }
        );
        assert_eq!(
            turn(&[], &AgentStatus::Idle, false),
            Turn::Stopped {
                line: String::new()
            }
        );
    }

    #[test]
    fn lines_are_redacted_and_cut() {
        let key = "AIzaSyD0nTuSeTh1sFakeKeyAbCdEfGhIjKlMn";
        let items = [text(
            Role::Assistant,
            &format!("The key is {key} {}", "x".repeat(400)),
        )];
        let Turn::Stopped { line } = turn(&items, &AgentStatus::Idle, false) else {
            panic!("stopped");
        };
        assert!(!line.contains(key), "{line}");
        assert_eq!(line.chars().count(), TURN_TEXT_MAX);
        assert!(line.ends_with('…'));
        let items = [text(
            Role::Assistant,
            &format!("{} Or {key}?", "Is it this one ".repeat(30)),
        )];
        let Turn::Asking { question } = turn(&items, &AgentStatus::Idle, false) else {
            panic!("asking");
        };
        assert_eq!(question.chars().count(), TURN_TEXT_MAX);
    }
}
