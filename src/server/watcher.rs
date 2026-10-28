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
use super::terminal::TerminalPanes;
use crate::core::agents::extract_agent_name_for_pane;
use crate::watch::WatchRegistry;

/// How long to wait before reconnecting after herdr closes the event stream.
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// What the daemon subscribes to while `panes` are watched and
/// `terminals` have open terminals (plan 20261027_07, decision 8).
pub fn subscriptions(panes: &[String], terminals: &[String]) -> Vec<Subscription> {
    let mut subs = Subscription::focus_and_panes().to_vec();
    subs.extend(panes.iter().map(Subscription::agent_status));
    subs.extend(
        terminals
            .iter()
            .map(|pane_id| Subscription::PaneScrollChanged {
                pane_id: pane_id.clone(),
            }),
    );
    subs
}

/// The pane of a `pane.scroll_changed` event, which libherdr does not
/// model: it arrives as `HerdrEvent::Other`, in either spelling.
pub fn scrolled_pane(event: &str, data: &serde_json::Value) -> Option<String> {
    if event.replace('.', "_") != "pane_scroll_changed" {
        return None;
    }
    data.get("pane_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

pub async fn run(
    transport: HerdrTransport,
    client: HerdrClient,
    registry: Arc<WatchRegistry>,
    terminals: Arc<TerminalPanes>,
    events: broadcast::Sender<CoreEvent>,
) {
    let mut changes = registry.subscribe_changes();
    let mut terminal_changes = terminals.subscribe_changes();
    loop {
        let panes = registry.watched();
        let terminal_panes = terminals.panes();
        log::info!(
            "Subscribing to herdr's focus and panes, agent status for {} pane(s): {:?}, and scrolling for {} terminal pane(s)",
            panes.len(),
            panes,
            terminal_panes.len()
        );
        let subs = subscriptions(&panes, &terminal_panes);
        let mut stream = match transport.open_event_stream(&subs).await {
            Ok(stream) => stream,
            Err(e) => {
                log::warn!("Failed to open herdr event stream: {e}; retrying");
                tokio::select! {
                    _ = tokio::time::sleep(RECONNECT_DELAY) => {}
                    changed = changes.changed() => {
                        if changed.is_err() { return; }
                    }
                    changed = terminal_changes.changed() => {
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
                    Some(HerdrEvent::Other { event, data }) => match scrolled_pane(&event, &data) {
                        Some(pane_id) => {
                            let _ = events.send(CoreEvent::Output { pane_id });
                        }
                        None => log::debug!("Ignoring unsubscribed herdr event '{event}'"),
                    },
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
                changed = terminal_changes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    log::info!("Terminal panes changed; reopening the event subscription");
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
        let none = serde_json::to_value(subscriptions(&[], &[])).unwrap();
        assert_eq!(
            none,
            serde_json::json!([
                {"type": "pane.focused"}, {"type": "tab.focused"},
                {"type": "pane.updated"}, {"type": "pane.closed"}
            ])
        );
        let one = subscriptions(&["w1:p6".to_string()], &[]);
        assert_eq!(one.len(), 5);
        assert_eq!(one[4], Subscription::agent_status("w1:p6"));
    }

    #[test]
    fn terminal_panes_add_scroll_subscriptions() {
        // Plan 20261027_07, decision 8.
        let subs = subscriptions(&["w1:p6".to_string()], &["w1:p2".to_string()]);
        assert_eq!(subs.len(), 6);
        assert_eq!(
            serde_json::to_value(&subs[5]).unwrap(),
            serde_json::json!({"type": "pane.scroll_changed", "pane_id": "w1:p2"})
        );
        let data = serde_json::json!({"pane_id": "w1:p2", "workspace_id": "w1"});
        assert_eq!(
            scrolled_pane("pane.scroll_changed", &data).as_deref(),
            Some("w1:p2")
        );
        assert_eq!(
            scrolled_pane("pane_scroll_changed", &data).as_deref(),
            Some("w1:p2")
        );
        assert_eq!(scrolled_pane("pane.output_matched", &data), None);
        // From herdr's line, as libherdr decodes it.
        let line = r#"{"event":"pane.scroll_changed","data":{"pane_id":"w1:p2","workspace_id":"w1","scroll":{"offset_from_bottom":0,"max_offset_from_bottom":10,"viewport_rows":30}}}"#;
        match HerdrEvent::from_line(line) {
            Some(HerdrEvent::Other { event, data }) => {
                assert_eq!(scrolled_pane(&event, &data).as_deref(), Some("w1:p2"))
            }
            other => panic!("{other:?}"),
        }
    }
}
