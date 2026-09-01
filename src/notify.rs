//! Spoken notifications for watched panes.
//!
//! Two rules govern when an announcement is allowed to reach the speaker, both
//! of them about not talking over the user:
//!
//! * **Never barge.** Announcements are submitted with `barge = false` and on
//!   the same TTS session as the assistant's replies, so they queue behind
//!   whatever is playing instead of preempting it.
//! * **Never interrupt a confirmation.** While the mechanical confirmation gate
//!   is armed or confirmed, announcements are held back entirely. Speaking
//!   between "Should I send X?" and the user's "yes" would talk over the
//!   exchange the gate is waiting on, and the reply that follows could be
//!   mistaken for an answer to the announcement.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use libconversate::tts::TtsSpeaker;
use libconversate::VoiceState;
use libherdr::events::{HerdrEvent, PaneAgentStatusChanged, Subscription};
use libherdr::protocol::AgentStatus;
use libherdr::transport::HerdrTransport;

use crate::watch::WatchRegistry;

/// TTS session id the s2s pipeline speaks on. Announcements must use the same
/// one: herdr's TTS manager treats a *different* session from the same client
/// as a topic switch and preempts what is playing, which is exactly the barge
/// we are trying to avoid.
const TTS_SESSION: &str = "s2s";

/// How often to re-check whether held-back announcements can be released.
const FLUSH_INTERVAL: Duration = Duration::from_millis(500);

/// How long to wait before reconnecting after herdr closes the event stream.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

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

    /// Record a status change and queue an announcement if it is worth saying.
    ///
    /// Only transitions *into* `blocked` or `done` are announced, and only when
    /// the status actually changed — herdr re-reports a status on reconnect and
    /// on unrelated pane updates, which would otherwise repeat the notice.
    pub fn observe(&mut self, event: &PaneAgentStatusChanged) {
        let previous = self.last_status.insert(event.pane_id.clone(), event.agent_status.clone());
        if previous.as_ref() == Some(&event.agent_status) {
            return;
        }
        let label = event.agent_label();
        let sentence = match event.agent_status {
            AgentStatus::Blocked => format!("{label} needs your input."),
            AgentStatus::Done => format!("{label} finished."),
            // Working / Idle / Unknown are normal churn, not notifications.
            _ => return,
        };
        self.queue(sentence);
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

/// Speak everything the announcer is willing to release right now.
fn drain(announcer: &mut Announcer, voice: &VoiceState, tts: &TtsSpeaker) {
    while let Some(sentence) = announcer.take_speakable(voice.is_confirmation_active()) {
        log::info!("Announcing: {sentence}");
        // barge = false: queue behind the current utterance, never preempt it.
        tts.speak(TTS_SESSION, &sentence, false);
    }
}

/// Supervise the herdr event subscription and speak notifications for watched panes.
///
/// Runs until the process exits. Reconnects when the watch set changes (herdr
/// closes any connection that receives a second `events.subscribe`, so the set
/// can only be changed by opening a new one) and when herdr hangs up.
pub async fn run_agent_watch(
    transport: HerdrTransport,
    registry: Arc<WatchRegistry>,
    voice: Arc<VoiceState>,
    tts: Arc<TtsSpeaker>,
) {
    let mut changes = registry.subscribe_changes();
    let mut announcer = Announcer::new();

    loop {
        let panes = registry.watched();
        if panes.is_empty() {
            log::debug!("No panes watched; event subscription idle");
            // Still release anything queued from before the last unwatch.
            drain(&mut announcer, &voice, &tts);
            if changes.changed().await.is_err() {
                return;
            }
            continue;
        }

        let subscriptions: Vec<Subscription> =
            panes.iter().map(Subscription::agent_status).collect();
        log::info!("Subscribing to agent status for {} pane(s): {:?}", panes.len(), panes);

        let mut stream = match transport.open_event_stream(&subscriptions).await {
            Ok(stream) => stream,
            Err(e) => {
                log::warn!("Failed to open herdr event stream: {e}; retrying");
                tokio::select! {
                    _ = tokio::time::sleep(RECONNECT_DELAY) => {}
                    changed = changes.changed() => {
                        if changed.is_err() { return; }
                    }
                }
                continue;
            }
        };

        let mut flush = tokio::time::interval(FLUSH_INTERVAL);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                event = stream.next_event() => match event {
                    Some(HerdrEvent::AgentStatusChanged(change)) => {
                        if registry.is_watched(&change.pane_id) {
                            log::info!(
                                "Agent status change: pane={} agent={:?} status={:?}",
                                change.pane_id, change.agent, change.agent_status
                            );
                            announcer.observe(&change);
                            drain(&mut announcer, &voice, &tts);
                        }
                    }
                    Some(HerdrEvent::SubscriptionStarted) => {
                        log::info!("herdr event subscription is live");
                    }
                    Some(HerdrEvent::Error(message)) => {
                        log::warn!("herdr rejected the event subscription: {message}");
                    }
                    Some(HerdrEvent::Other { event, .. }) => {
                        log::debug!("Ignoring unsubscribed herdr event '{event}'");
                    }
                    None => {
                        log::info!("herdr event stream closed; reconnecting");
                        tokio::time::sleep(RECONNECT_DELAY).await;
                        break;
                    }
                },
                changed = changes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    log::info!("Watch set changed; reopening the event subscription");
                    break;
                }
                _ = flush.tick() => {
                    // Release anything held back while a confirmation was active.
                    drain(&mut announcer, &voice, &tts);
                }
            }
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

    #[test]
    fn test_announces_blocked_and_done_only() {
        let mut a = Announcer::new();
        a.observe(&change("w1:p6", "claude", AgentStatus::Working));
        a.observe(&change("w1:p6", "claude", AgentStatus::Idle));
        assert_eq!(a.pending_count(), 0, "routine churn is not announced");

        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));
        assert_eq!(
            a.take_speakable(false).as_deref(),
            Some("claude needs your input.")
        );

        a.observe(&change("w1:p6", "claude", AgentStatus::Done));
        assert_eq!(a.take_speakable(false).as_deref(), Some("claude finished."));
    }

    #[test]
    fn test_repeated_status_is_announced_once() {
        let mut a = Announcer::new();
        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));
        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));
        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));
        assert_eq!(a.pending_count(), 1);

        // Leaving and re-entering the state does announce again.
        a.observe(&change("w1:p6", "claude", AgentStatus::Working));
        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));
        assert_eq!(a.pending_count(), 2);
    }

    #[test]
    fn test_panes_are_tracked_independently() {
        let mut a = Announcer::new();
        a.observe(&change("w1:p6", "claude", AgentStatus::Done));
        a.observe(&change("w1:p1", "agy", AgentStatus::Done));
        assert_eq!(a.take_speakable(false).as_deref(), Some("claude finished."));
        assert_eq!(a.take_speakable(false).as_deref(), Some("agy finished."));
    }

    #[test]
    fn test_held_back_while_confirmation_active() {
        let mut a = Announcer::new();
        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));

        // Mid-confirmation: nothing is released, and nothing is lost.
        assert_eq!(a.take_speakable(true), None);
        assert_eq!(a.pending_count(), 1);

        // Gate clears: the announcement is spoken.
        assert_eq!(
            a.take_speakable(false).as_deref(),
            Some("claude needs your input.")
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
        // The oldest were dropped, so the newest notice survives.
        assert_eq!(a.take_speakable(false).as_deref(), Some("notice 2"));
    }

    #[test]
    fn test_display_name_preferred_for_speech() {
        let mut ev = change("w1:p6", "claude", AgentStatus::Blocked);
        ev.display_agent = Some("Claude".to_string());
        let mut a = Announcer::new();
        a.observe(&ev);
        assert_eq!(
            a.take_speakable(false).as_deref(),
            Some("Claude needs your input.")
        );
    }

    #[test]
    fn test_forget_allows_reannouncing() {
        let mut a = Announcer::new();
        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));
        let _ = a.take_speakable(false);
        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));
        assert_eq!(a.pending_count(), 0, "same status is still deduped");

        a.forget("w1:p6");
        a.observe(&change("w1:p6", "claude", AgentStatus::Blocked));
        assert_eq!(a.pending_count(), 1, "after forget, it announces again");
    }
}
