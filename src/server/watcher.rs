//! The herdr event subscription for watched panes, feeding the daemon's
//! event bus (plan 20261022_01, decision 11).
//!
//! herdr closes any connection that receives a second `events.subscribe`,
//! so the watch set can only change by reconnecting with a new one.

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

pub async fn run(
    transport: HerdrTransport,
    client: HerdrClient,
    registry: Arc<WatchRegistry>,
    events: broadcast::Sender<CoreEvent>,
) {
    let mut changes = registry.subscribe_changes();
    loop {
        let panes = registry.watched();
        if panes.is_empty() {
            if changes.changed().await.is_err() {
                return;
            }
            continue;
        }

        let subscriptions: Vec<Subscription> =
            panes.iter().map(Subscription::agent_status).collect();
        log::info!(
            "Subscribing to agent status for {} pane(s): {:?}",
            panes.len(),
            panes
        );
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
