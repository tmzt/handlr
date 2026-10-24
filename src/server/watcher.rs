//! The herdr event subscription, feeding the daemon's event bus (plan
//! 20261022_01, decision 11): the status of watched panes, and always
//! herdr's focus and its panes' lifecycle (plan 20261023_03, decision
//! 12).
//!
//! herdr closes any connection that receives a second `events.subscribe`,
//! so the subscription set can only change by reconnecting with a new one;
//! the focus and pane subscriptions join the watch set's, and the
//! connection stays open with no pane watched.

use std::sync::Arc;
use std::time::Duration;

use libherdr::client::HerdrClient;
use libherdr::events::{HerdrEvent, Subscription};
use libherdr::transport::HerdrTransport;
use tokio::sync::broadcast;

use super::CoreEvent;
use crate::core::agents::extract_agent_name_for_pane;
use crate::watch::WatchRegistry;

/// How long to wait before reconnecting after herdr closes the event stream.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// What the daemon subscribes to while `panes` are watched.
pub fn subscriptions(panes: &[String]) -> Vec<Subscription> {
    let mut subs = Subscription::focus_and_panes().to_vec();
    subs.extend(panes.iter().map(Subscription::agent_status));
    subs
}

pub async fn run(
    transport: HerdrTransport,
    client: HerdrClient,
    registry: Arc<WatchRegistry>,
    events: broadcast::Sender<CoreEvent>,
) {
    let mut changes = registry.subscribe_changes();
    loop {
        let panes = registry.watched();
        log::info!(
            "Subscribing to herdr's focus and panes, and agent status for {} pane(s): {:?}",
            panes.len(),
            panes
        );
        let mut stream = match transport.open_event_stream(&subscriptions(&panes)).await {
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

        loop {
            tokio::select! {
                event = stream.next_event() => match event {
                    Some(HerdrEvent::AgentStatusChanged(change)) => {
                        if registry.is_watched(&change.pane_id) {
                            let agent = match client.list_panes().await {
                                Ok(snapshot) => extract_agent_name_for_pane(&snapshot, &change.pane_id),
                                Err(_) => change.agent_label().to_lowercase(),
                            };
                            let _ = events.send(CoreEvent::AgentStatus {
                                pane_id: change.pane_id.clone(),
                                agent,
                                status: change.agent_status.clone(),
                            });
                        }
                    }
                    Some(HerdrEvent::PaneFocused { pane_id, .. }) => {
                        let _ = events.send(CoreEvent::Focus { pane_id: Some(pane_id) });
                    }
                    Some(HerdrEvent::TabFocused { .. }) => {
                        let _ = events.send(CoreEvent::Focus { pane_id: None });
                    }
                    Some(HerdrEvent::PaneUpdated(pane)) => {
                        let _ = events.send(CoreEvent::Pane {
                            pane_id: pane.pane_id.clone(),
                            closed: false,
                        });
                    }
                    Some(HerdrEvent::PaneClosed { pane_id, .. }) => {
                        let _ = events.send(CoreEvent::Pane { pane_id, closed: true });
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_and_panes_are_always_subscribed() {
        let none = serde_json::to_value(subscriptions(&[])).unwrap();
        assert_eq!(
            none,
            serde_json::json!([
                {"type": "pane.focused"}, {"type": "tab.focused"},
                {"type": "pane.updated"}, {"type": "pane.closed"}
            ])
        );
        let one = subscriptions(&["w1:p6".to_string()]);
        assert_eq!(one.len(), 5);
        assert_eq!(one[4], Subscription::agent_status("w1:p6"));
    }
}
