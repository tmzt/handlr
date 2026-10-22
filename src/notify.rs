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

use crate::config::NamingConfig;
use crate::core::naming::{capitalise, Names};

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

/// The sentence announcing `status` for an agent named `name`. The name
/// starts the sentence, so it is capitalised ("The handlr agent finished.").
pub fn announcement(status: &AgentStatus, name: &str) -> Option<String> {
    let name = capitalise(name);
    match status {
        AgentStatus::Blocked => Some(format!("{name} needs your input.")),
        AgentStatus::Done => Some(format!("{name} finished.")),
        _ => None,
    }
}

/// Name the agent in `change` as briefly as the current session allows, in
/// its spoken form (plan 20261022_05, `core::naming`).
///
/// Falls back to the name carried on the event itself when the snapshot cannot
/// be read — a less precise notice beats a silent one.
pub async fn resolve_display_name(
    client: &HerdrClient,
    change: &PaneAgentStatusChanged,
    naming: &NamingConfig,
) -> String {
    let snapshot = match client.list_panes().await {
        Ok(snapshot) => snapshot,
        Err(e) => {
            log::warn!("Could not read snapshot to name the agent: {e}");
            return change.agent_label();
        }
    };
    match Names::from_snapshot(&snapshot, naming).get(&change.pane_id) {
        Some(target) => target.spoken.clone(),
        None => {
            log::debug!(
                "Pane {} not in snapshot; naming from the event",
                change.pane_id
            );
            change.agent_label()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            announcement(&AgentStatus::Done, "the handlr agent").as_deref(),
            Some("The handlr agent finished.")
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
}
