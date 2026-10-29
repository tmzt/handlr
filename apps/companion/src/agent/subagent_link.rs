//! The link in the parent's flow to a subagent it started
//! (docs/design/20261023_07_SUBAGENTS_PLAN.md, decision 4): the `Agent`
//! tool call is drawn as one line, "⎿ <description> (running|finished)",
//! and a tap on it opens that subagent in the same view.
//!
//! An entry carries no tool call id, so the call is matched to its
//! subagent by the `description` in its input (`Entry::text` is the tool's
//! name and its input's JSON) against `ChildInfo::description`, which the
//! daemon redacts and may cut with "…".

use libflankr_proto::common::{ChildInfo, Entry, EntryKind};

use super::full_text;

/// The tools that start a subagent: Claude Code's `Agent`, once `Task`.
pub const TOOLS: [&str; 2] = ["Agent", "Task"];
/// The link's lead, as Claude Code's interface draws a call's result.
pub const LEAD: char = '\u{23bf}';

/// A link laid out in a block: the entry's index in its section, the
/// subagent's index in the view's children, and what the line says.
#[derive(Clone, Debug, PartialEq)]
pub struct SubagentLink {
    pub entry: usize,
    pub child: usize,
    pub description: String,
    pub finished: bool,
}

impl SubagentLink {
    /// "⎿ overview screen plan (running)".
    pub fn line(&self) -> String {
        format!(
            "{LEAD} {} ({})",
            self.description.trim(),
            if self.finished { "finished" } else { "running" }
        )
    }
}

/// The `description` of a subagent-starting call's input, if `e` is one.
pub fn description_of(e: &Entry) -> Option<String> {
    if e.kind != EntryKind::ToolUse {
        return None;
    }
    let tool = e.tool.as_deref().filter(|t| TOOLS.contains(t))?;
    let text = full_text(e);
    let input = text.strip_prefix(tool).unwrap_or(text).trim_start();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(input) {
        return v
            .get("description")
            .and_then(|d| d.as_str())
            .map(str::to_string);
    }
    // The input was cut: read the field's string where it starts.
    let at = input.find("\"description\"")? + "\"description\"".len();
    let rest = input[at..].trim_start().strip_prefix(':')?.trim_start();
    let mut de = serde_json::Deserializer::from_str(rest).into_iter::<String>();
    de.next()?.ok()
}

/// Whether a call's `description` is the subagent's: the same, or, where
/// the daemon cut the subagent's with "…", its start.
fn same(call: &str, child: &str) -> bool {
    let (call, child) = (call.trim(), child.trim());
    if child.is_empty() {
        return false;
    }
    call == child
        || child
            .strip_suffix('\u{2026}')
            .is_some_and(|start| !start.is_empty() && call.starts_with(start.trim_end()))
}

/// The links in `entries` to `children`: each subagent-starting call whose
/// description is a subagent's. A subagent is linked from one call only,
/// the first that names it.
pub fn links(entries: &[Entry], children: &[ChildInfo]) -> Vec<SubagentLink> {
    if children.is_empty() {
        return Vec::new();
    }
    let mut used = Vec::new();
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let Some(call) = description_of(e) else {
            continue;
        };
        let Some(c) = (0..children.len())
            .find(|j| !used.contains(j) && same(&call, &children[*j].description))
        else {
            continue;
        };
        used.push(c);
        out.push(SubagentLink {
            entry: i,
            child: c,
            description: children[c].description.clone(),
            finished: children[c].finished,
        });
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::agent::subagent_menu::tests::children;
    use libflankr_proto::common::Role;

    /// Claude Code's `Agent` call, as the reader gives it: the name and
    /// the input's JSON, its keys sorted.
    pub(crate) fn agent_call(description: &str) -> Entry {
        let input = serde_json::json!({
            "description": description,
            "prompt": "Read the plan and build it.",
            "subagent_type": "general-purpose",
        });
        Entry {
            ts: "2026-10-29T06:00:00Z".into(),
            role: Role::Assistant,
            kind: EntryKind::ToolUse,
            tool: Some("Agent".into()),
            summary: format!("Agent {input}").chars().take(60).collect(),
            links: Vec::new(),
            origin: None,
            text: Some(format!("Agent {input}")),
        }
    }

    #[test]
    fn a_call_is_matched_by_its_description() {
        let kids = children();
        let entries = vec![
            agent_call("Build the libtypedmd licences plan"),
            agent_call("Something else"),
            agent_call("Build the overview screen plan"),
        ];
        let l = links(&entries, &kids);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].entry, l[0].child), (0, 2));
        assert_eq!(
            l[0].line(),
            "\u{23bf} Build the libtypedmd licences plan (finished)"
        );
        assert_eq!((l[1].entry, l[1].child), (2, 0));
        assert_eq!(
            l[1].line(),
            "\u{23bf} Build the overview screen plan (running)"
        );
        assert!(links(&entries, &[]).is_empty());
    }

    #[test]
    fn a_cut_input_and_a_cut_description_still_match() {
        let mut e = agent_call("Build the overview screen plan");
        // The input cut after the description.
        let text = e.text.take().unwrap();
        let cut = &text[..text.find("\"prompt\"").unwrap() + 12];
        e.text = Some(format!("{cut}\u{2026}"));
        assert_eq!(
            description_of(&e).as_deref(),
            Some("Build the overview screen plan")
        );
        // The subagent's description cut by the daemon.
        let mut kids = children();
        kids[0].description = "Build the overview\u{2026}".into();
        assert_eq!(links(&[e], &kids)[0].child, 0);
        // Another tool, or a text, is no call.
        let mut bash = agent_call("Build the overview screen plan");
        bash.tool = Some("Bash".into());
        assert_eq!(description_of(&bash), None);
        // `Task`, the tool's older name.
        let mut task = agent_call("x");
        task.tool = Some("Task".into());
        task.text = Some(task.text.unwrap().replacen("Agent", "Task", 1));
        assert_eq!(description_of(&task).as_deref(), Some("x"));
    }
}
