//! The `### Agent sessions` section of Gemini's turn context (plan
//! 20261022_03, decision 7; living section L2).
//!
//! Each agent gets its spoken name, its last `ENTRIES_PER_AGENT` text
//! entries (user and assistant, oldest first, each redacted and cut to
//! `ENTRY_MAX` characters), one line counting the tool calls among them,
//! and its pending question. A block is at most `AGENT_MAX` bytes and the
//! section at most `TOTAL_MAX`: the oldest entries are dropped first, and
//! the pending question is always kept.

use super::redact::redact;
use super::{EntryKind, Item, PendingQuestion, Role, cut};

/// Text entries per agent.
pub const ENTRIES_PER_AGENT: usize = 6;
/// The longest entry, in characters.
pub const ENTRY_MAX: usize = 800;
/// The largest agent block, in bytes.
pub const AGENT_MAX: usize = 6 * 1024;
/// The largest section, in bytes.
pub const TOTAL_MAX: usize = 12 * 1024;

/// The section's heading and what it is.
pub const HEADING: &str = "### Agent sessions\n\
From each agent's own transcript, oldest first; secrets are redacted. \
A pending question is what the agent is waiting for the user to answer. \
Lines marked task, peer:<name>, agent:<id>, recap or compact are context \
the agent was given (a finished task, another session's or a subagent's \
message, a recap, a compaction), not the user speaking.\n";

/// One text entry, redacted and cut.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    ts: String,
    role: Role,
    /// The entry's origin, for context (`Item::origin`).
    origin: Option<String>,
    text: String,
}

/// One agent's block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentContext {
    pub spoken: String,
    pub pane_id: String,
    pub status: String,
    lines: Vec<Line>,
    /// Tool calls from the first entry shown to the end.
    pub tool_calls: usize,
    pub pending: Option<PendingQuestion>,
}

impl AgentContext {
    /// The block for an agent whose session read `items` (oldest first).
    /// `pending` is already redacted.
    pub fn new(
        spoken: &str,
        pane_id: &str,
        status: &str,
        items: &[Item],
        pending: Option<PendingQuestion>,
    ) -> Self {
        let texts: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.kind == EntryKind::Text)
            .map(|(n, _)| n)
            .collect();
        let shown = &texts[texts.len().saturating_sub(ENTRIES_PER_AGENT)..];
        let tool_calls = shown.first().map_or(0, |&first| {
            items[first..]
                .iter()
                .filter(|i| i.kind == EntryKind::ToolUse)
                .count()
        });
        let lines = shown
            .iter()
            .map(|&n| Line {
                ts: items[n].ts.clone(),
                role: items[n].role,
                origin: items[n].origin.clone(),
                text: cut(&redact(&items[n].text), ENTRY_MAX),
            })
            .collect();
        let mut block = AgentContext {
            spoken: spoken.to_string(),
            pane_id: pane_id.to_string(),
            status: status.to_string(),
            lines,
            tool_calls,
            pending,
        };
        while block.render().len() > AGENT_MAX && !block.lines.is_empty() {
            block.lines.remove(0);
        }
        block
    }

    /// How many entries the block shows.
    pub fn entries(&self) -> usize {
        self.lines.len()
    }

    /// The block as it goes into the context.
    pub fn render(&self) -> String {
        let mut out = format!(
            "#### {} (pane {}, {})\n",
            self.spoken, self.pane_id, self.status
        );
        if self.lines.is_empty() {
            out.push_str("(no recent messages)\n");
        }
        for line in &self.lines {
            let who = match (&line.origin, line.role) {
                (Some(origin), _) => origin.as_str(),
                (None, Role::User) => "user",
                (None, Role::Assistant) => "agent",
                (None, Role::Tool) => "tool",
            };
            let text = line.text.replace('\n', "\n  ");
            out.push_str(&format!("- {who} [{}]: {text}\n", line.ts));
        }
        if self.tool_calls > 0 {
            let s = if self.tool_calls == 1 { "" } else { "s" };
            out.push_str(&format!("({} tool call{s} in between)\n", self.tool_calls));
        }
        if let Some(q) = &self.pending {
            out.push_str(&format!(
                "Pending question [{}]: {}\n",
                q.ts,
                q.text.replace('\n', "\n  ")
            ));
        }
        out
    }
}

/// The section for `agents`, in the order given, within `TOTAL_MAX`:
/// while it is over, the oldest entry of any agent is dropped. Empty when
/// there are no agents.
pub fn section(mut agents: Vec<AgentContext>) -> String {
    if agents.is_empty() {
        return String::new();
    }
    let size = |agents: &[AgentContext]| {
        HEADING.len() + agents.iter().map(|a| a.render().len() + 1).sum::<usize>()
    };
    while size(&agents) > TOTAL_MAX {
        let oldest = agents
            .iter()
            .enumerate()
            .filter_map(|(n, a)| a.lines.first().map(|l| (l.ts.clone(), n)))
            .min();
        match oldest {
            Some((_, n)) => {
                agents[n].lines.remove(0);
            }
            None => break,
        }
    }
    let mut out = HEADING.to_string();
    for agent in &agents {
        out.push('\n');
        out.push_str(&agent.render());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(role: Role, kind: EntryKind, ts: &str, text: &str) -> Item {
        Item {
            pos: 0,
            ts: ts.to_string(),
            role,
            kind,
            tool: (kind != EntryKind::Text).then(|| "Bash".to_string()),
            text: text.to_string(),
            call_id: None,
            origin: None,
        }
    }

    fn transcript(prefix: &str, replies: usize, len: usize) -> Vec<Item> {
        let mut items = Vec::new();
        for i in 0..replies {
            let ts = format!("2026-01-01T00:{prefix}{i:02}:00Z");
            items.push(item(Role::User, EntryKind::Text, &ts, &format!("ask {i}")));
            items.push(item(Role::Assistant, EntryKind::ToolUse, &ts, "Bash {}"));
            items.push(item(Role::Tool, EntryKind::ToolResult, &ts, "ok"));
            items.push(item(
                Role::Assistant,
                EntryKind::Text,
                &ts,
                &format!("reply {i} {}", "y".repeat(len)),
            ));
        }
        items
    }

    #[test]
    fn the_last_six_texts_and_the_tool_calls_among_them() {
        let items = transcript("0", 5, 10);
        let block = AgentContext::new("The flankr agent", "w2:pZ", "idle", &items, None);
        assert_eq!(block.entries(), 6);
        // From "ask 2": calls after asks 2, 3 and 4.
        assert_eq!(block.tool_calls, 3);
        let text = block.render();
        assert!(text.starts_with("#### The flankr agent (pane w2:pZ, idle)\n"));
        assert!(!text.contains("ask 1"));
        assert!(text.contains("- user [2026-01-01T00:002:00Z]: ask 2"));
        assert!(text.contains("(3 tool calls in between)"));
        assert!(text.contains("- agent [2026-01-01T00:004:00Z]: reply 4 yyyyyyyyyy\n"));
    }

    #[test]
    fn context_entries_are_marked_by_their_origin_not_as_the_user() {
        let mut task = item(Role::User, EntryKind::Text, "t1", "Build finished");
        task.origin = Some("task".to_string());
        let mut peer = item(Role::User, EntryKind::Text, "t2", "Rebased on main");
        peer.origin = Some("peer:alpha".to_string());
        let items = vec![item(Role::User, EntryKind::Text, "t0", "go"), task, peer];
        let text = AgentContext::new("Claude", "p1", "idle", &items, None).render();
        assert!(text.contains("- user [t0]: go\n"), "{text}");
        assert!(text.contains("- task [t1]: Build finished\n"), "{text}");
        assert!(
            text.contains("- peer:alpha [t2]: Rebased on main\n"),
            "{text}"
        );
        assert!(HEADING.contains("not the user speaking"));
    }

    #[test]
    fn entries_are_cut_and_redacted() {
        let key = "AIzaSyD0nTuSeTh1sFakeKeyAbCdEfGhIjKlMn";
        let items = vec![
            item(Role::User, EntryKind::Text, "t1", &format!("use {key}")),
            item(Role::Assistant, EntryKind::Text, "t2", &"z".repeat(2000)),
        ];
        let text = AgentContext::new("Claude", "p1", "idle", &items, None).render();
        assert!(!text.contains(key));
        assert!(text.contains("[redacted]"));
        assert!(text.contains(&format!("{}…", "z".repeat(ENTRY_MAX - 1))));
        assert!(!text.contains(&"z".repeat(ENTRY_MAX)));
    }

    #[test]
    fn a_block_stays_within_six_kilobytes_keeping_the_question() {
        // Six replies of 800 multi-byte characters: well over 6 KB.
        let mut items = Vec::new();
        for i in 0..6 {
            items.push(item(
                Role::Assistant,
                EntryKind::Text,
                &format!("t{i}"),
                &"é".repeat(800),
            ));
        }
        let q = PendingQuestion {
            text: "Shall I go on?".to_string(),
            ts: "t5".to_string(),
            about_plan: false,
            plan: None,
            links: vec![],
        };
        let block = AgentContext::new("Claude", "p1", "blocked", &items, Some(q));
        let text = block.render();
        assert!(text.len() <= AGENT_MAX, "{}", text.len());
        assert!(block.entries() < 6);
        assert!(text.contains("Pending question [t5]: Shall I go on?"));
        // The newest entry is kept.
        assert!(text.contains("[t5]"));
        assert!(!text.contains("[t0]"));
    }

    #[test]
    fn the_section_stays_within_twelve_kilobytes_dropping_the_oldest() {
        let agents: Vec<AgentContext> = (0..4)
            .map(|a| {
                let items = transcript(&a.to_string(), 6, 700);
                let q = PendingQuestion {
                    text: format!("Question {a}?"),
                    ts: "t".to_string(),
                    about_plan: false,
                    plan: None,
                    links: vec![],
                };
                AgentContext::new(
                    &format!("Agent {a}"),
                    &format!("p{a}"),
                    "idle",
                    &items,
                    Some(q),
                )
            })
            .collect();
        let text = section(agents);
        assert!(text.len() <= TOTAL_MAX, "{}", text.len());
        for a in 0..4 {
            assert!(text.contains(&format!("#### Agent {a} ")));
            assert!(text.contains(&format!("Question {a}?")));
        }
        // Agent 0's entries are the oldest: they went first.
        assert!(!text.contains("[2026-01-01T00:000"));
        assert!(text.contains("reply 5"));
        assert!(text.starts_with(HEADING));
        assert_eq!(section(Vec::new()), "");
    }
}
