//! Notifications for watched panes.
//!
//! An announcement is never spoken while a confirmation is pending. Speaking
//! between "Approve sending X?" and the user's answer would talk over the
//! exchange the confirmation is waiting on, and the reply that follows could
//! be mistaken for an answer to the announcement. The `Announcer` holds them
//! back until it is free; queueing behind playback is the speaking client's
//! job.

use std::collections::{HashMap, VecDeque};

use libherdr::client::HerdrClient;
use libherdr::events::PaneAgentStatusChanged;
use libherdr::protocol::AgentStatus;

/// Most announcements to hold while a confirmation is in flight. A confirmation
/// exchange is short; if it somehow is not, stale "needs input" notices are
/// worse than silence.
const MAX_PENDING: usize = 4;

/// Decides what to say about agent status changes, and when.
///
/// Kept free of TTS and socket types so the announcement rules can be tested
/// directly.
#[derive(Debug, Default)]
pub struct Announcer {
    last_status: HashMap<String, AgentStatus>,
    pending: VecDeque<String>,
}

impl Announcer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a status change and report whether it is worth announcing.
    ///
    /// Only transitions *into* `blocked` or `done` qualify, and only when the
    /// status actually changed — herdr re-reports a status on reconnect and on
    /// unrelated pane updates, which would otherwise repeat the notice.
    ///
    /// Naming the agent is deliberately not done here: the caller resolves it
    /// against the live session so a notice only says as much as it must.
    pub fn note(&mut self, event: &PaneAgentStatusChanged) -> Option<AgentStatus> {
        let previous = self
            .last_status
            .insert(event.pane_id.clone(), event.agent_status.clone());
        if previous.as_ref() == Some(&event.agent_status) {
            return None;
        }
        match event.agent_status {
            AgentStatus::Blocked | AgentStatus::Done => Some(event.agent_status.clone()),
            // Working / Idle / Unknown are normal churn, not notifications.
            _ => None,
        }
    }

    /// Queue a sentence to be spoken when the voice channel is free.
    pub fn queue(&mut self, sentence: String) {
        if self.pending.len() >= MAX_PENDING {
            let dropped = self.pending.pop_front();
            log::warn!("Announcement backlog full; dropping oldest: {dropped:?}");
        }
        self.pending.push_back(sentence);
    }

    /// Take the next announcement that may be spoken right now.
    ///
    /// Returns `None` while a confirmation exchange is in flight, leaving the
    /// backlog intact for the next call.
    pub fn take_speakable(&mut self, confirmation_active: bool) -> Option<String> {
        if confirmation_active {
            return None;
        }
        self.pending.pop_front()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Forget a pane's remembered status, so the next event about it announces
    /// again. Used when a pane stops being watched.
    pub fn forget(&mut self, pane_id: &str) {
        self.last_status.remove(pane_id);
    }
}

/// The sentence announcing `status` for an agent named `name`.
pub fn announcement(status: &AgentStatus, name: &str) -> Option<String> {
    match status {
        AgentStatus::Blocked => Some(format!("{name} needs your input.")),
        AgentStatus::Done => Some(format!("{name} finished.")),
        _ => None,
    }
}

/// Name the agent in `change` as briefly as the current session allows.
///
/// Falls back to the name carried on the event itself when the snapshot cannot
/// be read — a less precise notice beats a silent one.
pub async fn resolve_display_name(client: &HerdrClient, change: &PaneAgentStatusChanged) -> String {
    let snapshot = match client.list_panes().await {
        Ok(snapshot) => snapshot,
        Err(e) => {
            log::warn!("Could not read snapshot to name the agent: {e}");
            return change.agent_label();
        }
    };
    let peers = agent_identities(&snapshot);
    match peers.iter().find(|p| p.pane_id == change.pane_id) {
        Some(target) => speakable_name(target, &peers),
        None => {
            log::debug!(
                "Pane {} not in snapshot; naming from the event",
                change.pane_id
            );
            change.agent_label()
        }
    }
}

// ── Naming an agent out loud ────────────────────────────────────────────

/// One agent that could be named in an announcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentity {
    pub pane_id: String,
    /// Agent name as spoken (e.g. "Claude").
    pub agent: String,
    /// Project the pane sits in — the workspace label, or the working
    /// directory's basename when the workspace is unlabelled.
    pub project: Option<String>,
}

/// Capitalize a name for speech; agent and project names are proper nouns here.
fn speech_case(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) if first.is_lowercase() => {
            first.to_uppercase().collect::<String>() + chars.as_str()
        }
        _ => trimmed.to_string(),
    }
}

/// Read every agent-bearing pane out of a session snapshot, resolving each
/// pane's project from its workspace label (falling back to the cwd basename).
pub fn agent_identities(snapshot: &serde_json::Value) -> Vec<AgentIdentity> {
    let root = crate::transport::snapshot_root(snapshot);

    let mut labels: HashMap<String, String> = HashMap::new();
    if let Some(workspaces) = root.get("workspaces").and_then(|w| w.as_array()) {
        for ws in workspaces {
            let id = ws.get("workspace_id").and_then(|v| v.as_str());
            let label = ws
                .get("label")
                .or_else(|| ws.get("name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let (Some(id), Some(label)) = (id, label) {
                labels.insert(id.to_string(), label.to_string());
            }
        }
    }

    let mut identities = Vec::new();
    for pane in crate::transport::collect_panes(snapshot) {
        let Some(pane_id) = pane.get("pane_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let agent = pane
            .get("agent")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(agent) = agent else { continue };

        let project = pane
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .and_then(|ws| labels.get(ws).cloned())
            .or_else(|| {
                // Unlabelled workspace: the repo directory names it well enough.
                pane.get("cwd")
                    .and_then(|v| v.as_str())
                    .and_then(|cwd| cwd.trim_end_matches('/').rsplit('/').next())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            });

        // herdr reports panes both flat and nested under workspaces -> tabs.
        // Keyed on pane_id, not the whole record: the two copies can carry
        // different detail, and a pane counted twice would look like a peer of
        // itself and push every name to its most verbose form.
        if identities
            .iter()
            .any(|i: &AgentIdentity| i.pane_id == pane_id)
        {
            continue;
        }
        identities.push(AgentIdentity {
            pane_id: pane_id.to_string(),
            agent: speech_case(agent),
            project: project.as_deref().map(speech_case),
        });
    }
    identities
}

/// Shortest phrase that unambiguously names `target` among `peers`.
///
/// Escalates only as far as it has to:
///   1. `"Claude"` — no other agent shares the name.
///   2. `"Highbay agent"` — the name is shared, but the project is not, so the
///      project identifies the pane and repeating the agent adds nothing.
///   3. `"Claude in Highbay"` — neither alone is unique.
///   4. `"Claude in Highbay, pane 6"` — same agent and project on two panes.
pub fn speakable_name(target: &AgentIdentity, peers: &[AgentIdentity]) -> String {
    let others: Vec<&AgentIdentity> = peers
        .iter()
        .filter(|p| p.pane_id != target.pane_id)
        .collect();

    let agent_is_unique = !others
        .iter()
        .any(|p| p.agent.eq_ignore_ascii_case(&target.agent));
    if agent_is_unique {
        return target.agent.clone();
    }

    let Some(project) = target.project.as_deref() else {
        // Nothing to disambiguate with; fall back to the pane.
        return format!("{} in {}", target.agent, spoken_pane(&target.pane_id));
    };

    let project_is_unique = !others.iter().any(|p| {
        p.project
            .as_deref()
            .is_some_and(|other| other.eq_ignore_ascii_case(project))
    });
    if project_is_unique {
        return format!("{project} agent");
    }

    let combination_is_unique = !others.iter().any(|p| {
        p.agent.eq_ignore_ascii_case(&target.agent)
            && p.project
                .as_deref()
                .is_some_and(|other| other.eq_ignore_ascii_case(project))
    });
    if combination_is_unique {
        return format!("{} in {}", target.agent, project);
    }

    format!(
        "{} in {}, {}",
        target.agent,
        project,
        spoken_pane(&target.pane_id)
    )
}

/// Render a pane id for speech: `w1:p6` -> `pane 6`.
fn spoken_pane(pane_id: &str) -> String {
    let short = pane_id.rsplit(':').next().unwrap_or(pane_id);
    let digits = short.trim_start_matches(|c: char| !c.is_ascii_digit());
    if digits.is_empty() {
        format!("pane {short}")
    } else {
        format!("pane {digits}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change(pane: &str, agent: &str, status: AgentStatus) -> PaneAgentStatusChanged {
        PaneAgentStatusChanged {
            pane_id: pane.to_string(),
            workspace_id: "w1".to_string(),
            agent_status: status,
            agent: Some(agent.to_string()),
            display_agent: None,
            title: None,
        }
    }

    fn identity(pane: &str, agent: &str, project: Option<&str>) -> AgentIdentity {
        AgentIdentity {
            pane_id: pane.to_string(),
            agent: agent.to_string(),
            project: project.map(str::to_string),
        }
    }

    // ── what is worth announcing ──

    #[test]
    fn test_notes_blocked_and_done_only() {
        let mut a = Announcer::new();
        assert_eq!(
            a.note(&change("w1:p6", "claude", AgentStatus::Working)),
            None
        );
        assert_eq!(a.note(&change("w1:p6", "claude", AgentStatus::Idle)), None);
        assert_eq!(
            a.note(&change("w1:p6", "claude", AgentStatus::Blocked)),
            Some(AgentStatus::Blocked)
        );
        assert_eq!(
            a.note(&change("w1:p6", "claude", AgentStatus::Done)),
            Some(AgentStatus::Done)
        );
    }

    #[test]
    fn test_repeated_status_is_noted_once() {
        let mut a = Announcer::new();
        assert!(a
            .note(&change("w1:p6", "claude", AgentStatus::Blocked))
            .is_some());
        assert!(a
            .note(&change("w1:p6", "claude", AgentStatus::Blocked))
            .is_none());

        // Leaving and re-entering the state does qualify again.
        assert!(a
            .note(&change("w1:p6", "claude", AgentStatus::Working))
            .is_none());
        assert!(a
            .note(&change("w1:p6", "claude", AgentStatus::Blocked))
            .is_some());
    }

    #[test]
    fn test_panes_are_tracked_independently() {
        let mut a = Announcer::new();
        assert!(a
            .note(&change("w1:p6", "claude", AgentStatus::Done))
            .is_some());
        assert!(a.note(&change("w1:p1", "agy", AgentStatus::Done)).is_some());
    }

    #[test]
    fn test_announcement_wording() {
        assert_eq!(
            announcement(&AgentStatus::Blocked, "Claude").as_deref(),
            Some("Claude needs your input.")
        );
        assert_eq!(
            announcement(&AgentStatus::Done, "Highbay agent").as_deref(),
            Some("Highbay agent finished.")
        );
        assert_eq!(announcement(&AgentStatus::Working, "Claude"), None);
    }

    // ── holding back while a confirmation is in flight ──

    #[test]
    fn test_held_back_while_confirmation_active() {
        let mut a = Announcer::new();
        a.queue("Claude needs your input.".to_string());

        assert_eq!(a.take_speakable(true), None);
        assert_eq!(a.pending_count(), 1, "nothing is lost while held");

        assert_eq!(
            a.take_speakable(false).as_deref(),
            Some("Claude needs your input.")
        );
        assert_eq!(a.pending_count(), 0);
    }

    #[test]
    fn test_backlog_is_capped_dropping_oldest() {
        let mut a = Announcer::new();
        for i in 0..MAX_PENDING + 2 {
            a.queue(format!("notice {i}"));
        }
        assert_eq!(a.pending_count(), MAX_PENDING);
        assert_eq!(a.take_speakable(false).as_deref(), Some("notice 2"));
    }

    #[test]
    fn test_forget_allows_reannouncing() {
        let mut a = Announcer::new();
        assert!(a
            .note(&change("w1:p6", "claude", AgentStatus::Blocked))
            .is_some());
        assert!(a
            .note(&change("w1:p6", "claude", AgentStatus::Blocked))
            .is_none());

        a.forget("w1:p6");
        assert!(a
            .note(&change("w1:p6", "claude", AgentStatus::Blocked))
            .is_some());
    }

    // ── naming an agent out loud ──

    #[test]
    fn test_lone_agent_is_named_alone() {
        let peers = vec![
            identity("w1:p6", "Claude", Some("Handlr")),
            identity("w1:p1", "Agy", Some("Handlr")),
        ];
        assert_eq!(speakable_name(&peers[0], &peers), "Claude");
        assert_eq!(speakable_name(&peers[1], &peers), "Agy");
    }

    #[test]
    fn test_two_claudes_in_different_projects_use_the_project() {
        let peers = vec![
            identity("w1:p6", "Claude", Some("Handlr")),
            identity("w2:p1", "Claude", Some("Highbay")),
        ];
        assert_eq!(speakable_name(&peers[0], &peers), "Handlr agent");
        assert_eq!(speakable_name(&peers[1], &peers), "Highbay agent");
    }

    #[test]
    fn test_multiple_combinations_use_agent_in_project() {
        // Claude appears twice, and Handlr holds two different agents, so
        // neither half identifies the pane on its own.
        let peers = vec![
            identity("w1:p6", "Claude", Some("Handlr")),
            identity("w1:p1", "Agy", Some("Handlr")),
            identity("w2:p1", "Claude", Some("Highbay")),
        ];
        assert_eq!(speakable_name(&peers[0], &peers), "Claude in Handlr");
        // Highbay holds only Claude, so the project alone still identifies it.
        assert_eq!(speakable_name(&peers[2], &peers), "Highbay agent");
        // Agy is still unique by name.
        assert_eq!(speakable_name(&peers[1], &peers), "Agy");
    }

    #[test]
    fn test_identical_agent_and_project_falls_back_to_pane() {
        let peers = vec![
            identity("w1:p6", "Claude", Some("Handlr")),
            identity("w1:p9", "Claude", Some("Handlr")),
        ];
        assert_eq!(
            speakable_name(&peers[0], &peers),
            "Claude in Handlr, pane 6"
        );
        assert_eq!(
            speakable_name(&peers[1], &peers),
            "Claude in Handlr, pane 9"
        );
    }

    #[test]
    fn test_projectless_duplicate_falls_back_to_pane() {
        let peers = vec![
            identity("w1:p6", "Claude", None),
            identity("w1:p9", "Claude", None),
        ];
        assert_eq!(speakable_name(&peers[0], &peers), "Claude in pane 6");
    }

    // ── reading identities out of a snapshot ──

    #[test]
    fn test_agent_identities_use_workspace_label() {
        let snap = json!({"snapshot": {
            "workspaces": [
                {"workspace_id": "w1", "label": "handlr"},
                {"workspace_id": "w2", "label": "highbay"}
            ],
            "panes": [
                {"pane_id": "w1:p6", "agent": "claude", "workspace_id": "w1"},
                {"pane_id": "w2:p1", "agent": "claude", "workspace_id": "w2"},
                {"pane_id": "w1:p3", "workspace_id": "w1"}
            ]
        }});
        let ids = agent_identities(&snap);
        assert_eq!(ids.len(), 2, "panes with no agent are skipped");
        assert_eq!(ids[0], identity("w1:p6", "Claude", Some("Handlr")));
        assert_eq!(ids[1], identity("w2:p1", "Claude", Some("Highbay")));
        assert_eq!(speakable_name(&ids[0], &ids), "Handlr agent");
    }

    #[test]
    fn test_agent_identities_fall_back_to_cwd_basename() {
        let snap = json!({"snapshot": {
            "workspaces": [{"workspace_id": "w1"}],
            "panes": [{
                "pane_id": "w1:p6", "agent": "agy", "workspace_id": "w1",
                "cwd": "/Users/tmeade/src/handlr"
            }]
        }});
        let ids = agent_identities(&snap);
        assert_eq!(ids[0].project.as_deref(), Some("Handlr"));
    }

    #[test]
    fn test_agent_identities_deduplicate_across_flat_and_nested_lists() {
        // herdr reports panes both flat and under workspaces -> tabs.
        let snap = json!({"snapshot": {
            "workspaces": [{
                "workspace_id": "w1", "label": "handlr",
                "tabs": [{"tab_id": "w1:t6", "panes": [
                    {"pane_id": "w1:p6", "agent": "claude", "workspace_id": "w1"}
                ]}]
            }],
            "panes": [{"pane_id": "w1:p6", "agent": "claude", "workspace_id": "w1"}]
        }});
        let ids = agent_identities(&snap);
        assert_eq!(ids.len(), 1, "the same pane must not become its own peer");
        assert_eq!(speakable_name(&ids[0], &ids), "Claude");
    }
}
