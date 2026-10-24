//! Live subscription events pushed by `herdr` over a held-open connection.
//!
//! The request/response transport in [`crate::transport`] opens a connection
//! per call and reads exactly one reply. Subscriptions are the opposite shape:
//! one `events.subscribe` request, then an open-ended stream of event lines
//! until either side closes the connection.
//!
//! Wire format, as spoken by herdr protocol 19:
//!
//! ```text
//! -> {"id":"sub-1","method":"events.subscribe","params":{"subscriptions":[
//!      {"type":"pane.agent_status_changed","pane_id":"w1:p6"}]}}
//! <- {"id":"sub-1","result":{"type":"subscription_started"}}
//! <- {"event":"pane_agent_status_changed","data":{...}}
//! <- {"event":"pane_agent_status_changed","data":{...}}
//! ```
//!
//! Note the two spellings: subscriptions are requested with dotted names
//! (`pane.agent_status_changed`) and delivered with underscored ones
//! (`pane_agent_status_changed`).
//!
//! **A connection accepts exactly one `events.subscribe`.** Sending a second
//! one makes herdr close the socket, so changing the subscription set means
//! reconnecting — see [`crate::transport::HerdrTransport::open_event_stream`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::protocol::{AgentStatus, PaneInfo};

/// A subscription requested from herdr.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Subscription {
    /// Fires when the agent in `pane_id` changes status. `agent_status`
    /// optionally narrows delivery to a single target status.
    #[serde(rename = "pane.agent_status_changed")]
    PaneAgentStatusChanged {
        pane_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_status: Option<AgentStatus>,
    },
    /// Fires when `pane_id` produces output matching a pattern.
    #[serde(rename = "pane.scroll_changed")]
    PaneScrollChanged { pane_id: String },
    /// Fires when any pane takes the focus (`pane_focused`). Not scoped to
    /// a pane.
    #[serde(rename = "pane.focused")]
    PaneFocused,
    /// Fires when any tab takes the focus (`tab_focused`). Not scoped.
    #[serde(rename = "tab.focused")]
    TabFocused,
    /// Fires when any pane changes (`pane_updated`), such as its agent's
    /// session. Not scoped.
    #[serde(rename = "pane.updated")]
    PaneUpdated,
    /// Fires when any pane closes (`pane_closed`). Not scoped.
    #[serde(rename = "pane.closed")]
    PaneClosed,
}

impl Subscription {
    /// Subscribe to every status change for a pane.
    pub fn agent_status(pane_id: impl Into<String>) -> Self {
        Self::PaneAgentStatusChanged {
            pane_id: pane_id.into(),
            agent_status: None,
        }
    }

    /// The pane this subscription is scoped to; `None` for the focus and
    /// pane lifecycle subscriptions, which herdr does not scope.
    pub fn pane_id(&self) -> Option<&str> {
        match self {
            Self::PaneAgentStatusChanged { pane_id, .. } => Some(pane_id),
            Self::PaneScrollChanged { pane_id } => Some(pane_id),
            Self::PaneFocused | Self::TabFocused | Self::PaneUpdated | Self::PaneClosed => None,
        }
    }

    /// The four unscoped subscriptions for herdr's focus and its panes'
    /// lifecycle (plan 20261023_03, decision 12).
    pub fn focus_and_panes() -> [Subscription; 4] {
        [
            Self::PaneFocused,
            Self::TabFocused,
            Self::PaneUpdated,
            Self::PaneClosed,
        ]
    }
}

/// Payload of a `pane_agent_status_changed` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneAgentStatusChanged {
    pub pane_id: String,
    #[serde(default)]
    pub workspace_id: String,
    pub agent_status: AgentStatus,
    /// Agent identifier as reported by the runner (e.g. `claude`, `agy`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Display name herdr shows for the agent, when it differs from `agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_agent: Option<String>,
    /// Terminal title at the time of the change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl PaneAgentStatusChanged {
    /// Best human-readable name for the agent, for speaking aloud.
    pub fn agent_label(&self) -> String {
        self.display_agent
            .as_deref()
            .or(self.agent.as_deref())
            .or(self.title.as_deref())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("the agent")
            .to_string()
    }
}

/// One decoded line from a subscription stream.
#[derive(Debug, Clone, PartialEq)]
pub enum HerdrEvent {
    /// The server acknowledged the subscription; the stream is live.
    SubscriptionStarted,
    /// An agent changed status in a watched pane.
    AgentStatusChanged(PaneAgentStatusChanged),
    /// A pane took the focus (`pane_focused`).
    PaneFocused {
        pane_id: String,
        workspace_id: String,
    },
    /// A tab took the focus (`tab_focused`); its focused pane is read from
    /// a snapshot.
    TabFocused {
        tab_id: String,
        workspace_id: String,
    },
    /// A pane changed (`pane_updated`), with the pane as herdr now has it.
    PaneUpdated(Box<PaneInfo>),
    /// A pane closed (`pane_closed`).
    PaneClosed {
        pane_id: String,
        workspace_id: String,
    },
    /// A subscribed event this crate does not model yet.
    Other { event: String, data: Value },
    /// The server rejected the subscription request.
    Error(String),
}

impl HerdrEvent {
    /// Decode a single newline-delimited line from the event connection.
    ///
    /// Returns `None` for blank lines and for anything that does not parse as
    /// JSON — a malformed line should not tear down a long-lived stream.
    pub fn from_line(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let value: Value = serde_json::from_str(trimmed).ok()?;

        if let Some(err) = value.get("error") {
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Some(Self::Error(message.to_string()));
        }

        // Subscription acknowledgement: {"id":..,"result":{"type":"subscription_started"}}
        if let Some(result) = value.get("result") {
            let kind = result.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if kind == "subscription_started" {
                return Some(Self::SubscriptionStarted);
            }
            return Some(Self::Other {
                event: kind.to_string(),
                data: result.clone(),
            });
        }

        let event = value.get("event").and_then(|e| e.as_str())?.to_string();
        let data = value.get("data").cloned().unwrap_or(Value::Null);

        // herdr delivers `pane.agent_status_changed` as `pane_agent_status_changed`;
        // accept either spelling so a protocol tweak does not silence watches.
        if event == "pane_agent_status_changed" || event == "pane.agent_status_changed" {
            match serde_json::from_value::<PaneAgentStatusChanged>(data.clone()) {
                Ok(changed) => return Some(Self::AgentStatusChanged(changed)),
                Err(e) => {
                    log::warn!(
                        "libherdr: agent status event did not deserialize: {e}; data={data}"
                    );
                }
            }
        }
        if let Some(decoded) = Self::lifecycle(&event, &data) {
            return Some(decoded);
        }
        Some(Self::Other { event, data })
    }

    /// The focus and pane lifecycle events, in either spelling; `None`
    /// for other events and for a payload that does not decode.
    fn lifecycle(event: &str, data: &Value) -> Option<Self> {
        let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_string);
        let decoded = match event.replace('.', "_").as_str() {
            "pane_focused" => Self::PaneFocused {
                pane_id: text("pane_id")?,
                workspace_id: text("workspace_id").unwrap_or_default(),
            },
            "tab_focused" => Self::TabFocused {
                tab_id: text("tab_id")?,
                workspace_id: text("workspace_id").unwrap_or_default(),
            },
            "pane_closed" => Self::PaneClosed {
                pane_id: text("pane_id")?,
                workspace_id: text("workspace_id").unwrap_or_default(),
            },
            "pane_updated" => match serde_json::from_value::<PaneInfo>(data.get("pane")?.clone()) {
                Ok(pane) => Self::PaneUpdated(Box::new(pane)),
                Err(e) => {
                    log::warn!("libherdr: pane_updated did not deserialize: {e}");
                    return None;
                }
            },
            _ => return None,
        };
        Some(decoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_serializes_with_dotted_type() {
        let sub = Subscription::agent_status("w1:p6");
        let json = serde_json::to_value(&sub).unwrap();
        assert_eq!(json["type"], "pane.agent_status_changed");
        assert_eq!(json["pane_id"], "w1:p6");
        // The optional filter is omitted rather than sent as null.
        assert!(json.get("agent_status").is_none());
    }

    #[test]
    fn test_focus_and_pane_subscriptions_carry_no_pane() {
        let subs = Subscription::focus_and_panes();
        let json = serde_json::to_value(&subs).unwrap();
        assert_eq!(
            json,
            serde_json::json!([
                {"type": "pane.focused"},
                {"type": "tab.focused"},
                {"type": "pane.updated"},
                {"type": "pane.closed"}
            ])
        );
        for sub in &subs {
            assert_eq!(sub.pane_id(), None);
            let back: Subscription =
                serde_json::from_value(serde_json::to_value(sub).unwrap()).unwrap();
            assert_eq!(&back, sub);
        }
        assert_eq!(Subscription::agent_status("w1:p6").pane_id(), Some("w1:p6"));
    }

    /// Synthetic events, written from herdr's API schema (`herdr api schema
    /// --json`: protocol 19, schema_version 1, read on 2026-10-23). No live
    /// event was recorded (CHANGES.md, plan 20261023_03, step 1).
    const LIFECYCLE_FIXTURE: &str =
        include_str!("../tests/fixtures/lifecycle_events_synthetic.jsonl");

    #[test]
    fn test_parse_focus_and_pane_events_from_the_fixture() {
        let events: Vec<HerdrEvent> = LIFECYCLE_FIXTURE
            .lines()
            .filter_map(HerdrEvent::from_line)
            .collect();
        assert_eq!(events.len(), 5, "{events:?}");
        assert_eq!(
            events[0],
            HerdrEvent::PaneFocused {
                pane_id: "w2:pZ".to_string(),
                workspace_id: "w2".to_string()
            }
        );
        assert_eq!(
            events[1],
            HerdrEvent::TabFocused {
                tab_id: "w2:t3".to_string(),
                workspace_id: "w2".to_string()
            }
        );
        match &events[2] {
            HerdrEvent::PaneUpdated(pane) => {
                assert_eq!(pane.pane_id, "w2:pZ");
                assert_eq!(pane.cwd.as_deref(), Some("/src/handlr"));
                assert!(pane.focused);
                let session = pane.agent_session.as_ref().expect("a session");
                assert_eq!(session.id(), Some("00000000-0000-4000-8000-0000000000a1"));
            }
            other => panic!("expected pane_updated, got {other:?}"),
        }
        assert_eq!(
            events[3],
            HerdrEvent::PaneClosed {
                pane_id: "w2:pV".to_string(),
                workspace_id: "w2".to_string()
            }
        );
        // The dotted spelling decodes the same.
        assert_eq!(
            events[4],
            HerdrEvent::PaneFocused {
                pane_id: "w1:p1".to_string(),
                workspace_id: "w1".to_string()
            }
        );
    }

    #[test]
    fn test_parse_subscription_ack() {
        let line = r#"{"id":"sub-1","result":{"type":"subscription_started"}}"#;
        assert_eq!(
            HerdrEvent::from_line(line),
            Some(HerdrEvent::SubscriptionStarted)
        );
    }

    #[test]
    fn test_parse_agent_status_changed() {
        let line = r#"{"event":"pane_agent_status_changed","data":{
            "type":"pane_agent_status_changed","pane_id":"w1:p6","workspace_id":"w1",
            "agent_status":"blocked","agent":"claude","display_agent":"Claude",
            "title":"claude"}}"#;
        match HerdrEvent::from_line(line).expect("should decode") {
            HerdrEvent::AgentStatusChanged(c) => {
                assert_eq!(c.pane_id, "w1:p6");
                assert_eq!(c.workspace_id, "w1");
                assert_eq!(c.agent_status, AgentStatus::Blocked);
                assert_eq!(c.agent_label(), "Claude");
            }
            other => panic!("expected agent status change, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_done_status_without_display_name() {
        let line = r#"{"event":"pane_agent_status_changed","data":{
            "pane_id":"w1:p1","workspace_id":"w1","agent_status":"done","agent":"agy"}}"#;
        match HerdrEvent::from_line(line).expect("should decode") {
            HerdrEvent::AgentStatusChanged(c) => {
                assert_eq!(c.agent_status, AgentStatus::Done);
                assert_eq!(c.agent_label(), "agy");
            }
            other => panic!("expected agent status change, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_unmodelled_event_is_preserved() {
        let line = r#"{"event":"layout_updated","data":{"type":"layout_updated"}}"#;
        match HerdrEvent::from_line(line).expect("should decode") {
            HerdrEvent::Other { event, .. } => assert_eq!(event, "layout_updated"),
            other => panic!("expected passthrough, got {other:?}"),
        }
        // A lifecycle event whose payload lacks its pane stays `Other`.
        let line = r#"{"event":"pane_focused","data":{"type":"pane_focused"}}"#;
        match HerdrEvent::from_line(line).expect("should decode") {
            HerdrEvent::Other { event, .. } => assert_eq!(event, "pane_focused"),
            other => panic!("expected passthrough, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_error_and_junk_lines() {
        let line =
            r#"{"id":"","error":{"code":"invalid_request","message":"missing field `pane_id`"}}"#;
        match HerdrEvent::from_line(line).expect("should decode") {
            HerdrEvent::Error(msg) => assert!(msg.contains("pane_id")),
            other => panic!("expected error, got {other:?}"),
        }
        // Blank and malformed lines must not kill the stream.
        assert_eq!(HerdrEvent::from_line("   "), None);
        assert_eq!(HerdrEvent::from_line("not json"), None);
    }
}
