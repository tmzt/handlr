//! The methods every handlr protocol shares (plan 20261022_01, decision 7,
//! as amended; the confirmation fields are named in CHANGES.md,
//! 2026-10-22, "Confirmations: handle, token and nonce").

use serde::{Deserialize, Serialize};

/// Protocol names, as `hello` declares them.
pub mod protocols {
    pub const HARNESS: &str = "handlr.harness";
    pub const COMPANION: &str = "handlr.companion";
}

/// Method and notification names.
pub mod methods {
    pub const HELLO: &str = "hello";
    pub const PING: &str = "ping";
    pub const CANCEL: &str = "cancel";
    pub const AGENTS_LIST: &str = "agents.list";
    pub const AGENTS_READ: &str = "agents.read";
    pub const AGENTS_SEND: &str = "agents.send";
    pub const AGENTS_FOCUS: &str = "agents.focus";
    pub const WATCH_ADD: &str = "watch.add";
    pub const WATCH_REMOVE: &str = "watch.remove";
    pub const CONFIRM_REPLY: &str = "confirm.reply";
    pub const CONFIRM_CANCEL: &str = "confirm.cancel";
    pub const CONFIRM_STATUS: &str = "confirm.status";
    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";

    /// Notifications from the daemon.
    pub const AGENT_STATUS_CHANGED: &str = "agent.status_changed";
    pub const CONFIRM_REQUESTED: &str = "confirm.requested";
    pub const CONFIRM_RESOLVED: &str = "confirm.resolved";
}

/// Topics for `events.subscribe`.
pub mod topics {
    pub const AGENT_STATUS: &str = "agent.status";
    pub const CONFIRM: &str = "confirm";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloParams {
    pub protocol: String,
    pub version: u32,
    /// A label for the client, such as `ctl` or `mcp`. Not a credential.
    pub client: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloResult {
    /// Always `handlr`.
    pub server: String,
    pub server_version: String,
    pub protocol_version: u32,
}

/// An agent, as `agents.list` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub pane_id: String,
    /// Lowercased; `shell` when the pane runs no known agent.
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    /// herdr's `agent_status`: `working`, `idle`, `blocked`, `done`,
    /// `unknown`, or a runner's own word.
    pub status: String,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsListResult {
    pub agents: Vec<AgentInfo>,
}

/// Lines read by default, and the most that may be asked for.
pub const READ_LINES_DEFAULT: usize = 40;
pub const READ_LINES_MAX: usize = 200;

fn read_lines_default() -> usize {
    READ_LINES_DEFAULT
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsReadParams {
    pub pane_id: String,
    /// 1 to `READ_LINES_MAX`.
    #[serde(default = "read_lines_default")]
    pub lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsReadResult {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsSendParams {
    pub pane_id: String,
    pub command: String,
    /// The confirmation handle: describes the action, 3-64 of `a-z`, `0-9`
    /// and `-`. Required by `handlr.harness`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AgentsSendResult {
    Sent,
    Pending {
        confirm_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handle: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsFocusParams {
    pub target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchParams {
    pub pane_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchResult {
    pub watched: Vec<String>,
}

/// Where a confirmation stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pending,
    Approved,
    Sent,
    Failed,
    Denied,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmReplyParams {
    pub confirm_id: u64,
    pub approve: bool,
    /// Hex BLAKE2s-256-MAC of the token over the id and handle. Required
    /// unless the principal is paired with `approve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmCancelParams {
    pub confirm_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeResult {
    pub outcome: Outcome,
}

/// `confirm.status`: by id or by handle, one of the two.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmStatusParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmStatusResult {
    pub confirm_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsSubscribeParams {
    pub topics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelParams {
    /// A request id on this connection.
    pub id: crate::envelope::Id,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelResult {
    pub cancelled: bool,
}

/// The empty object, for methods with no parameters or no result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Empty {}

/// `agent.status_changed`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatusChanged {
    pub pane_id: String,
    pub agent: String,
    pub status: String,
}

/// `confirm.requested`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmRequested {
    pub confirm_id: u64,
    pub pane_id: String,
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub command: String,
    pub requested_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// The confirmation token, sent only to principals that can approve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// A second announcement of a confirmation still waiting.
    #[serde(default)]
    pub reminder: bool,
}

/// `confirm.resolved`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmResolved {
    pub confirm_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: T) {
        let text = serde_json::to_string(&value).unwrap();
        let back: T = serde_json::from_str(&text).unwrap();
        assert_eq!(back, value, "{text}");
    }

    #[test]
    fn every_type_round_trips() {
        round_trip(HelloParams {
            protocol: protocols::HARNESS.to_string(),
            version: 1,
            client: "ctl".to_string(),
        });
        round_trip(HelloResult {
            server: "handlr".to_string(),
            server_version: "0.1.0".to_string(),
            protocol_version: 1,
        });
        round_trip(AgentsListResult {
            agents: vec![AgentInfo {
                pane_id: "w2:pZ".to_string(),
                agent: "claude".to_string(),
                project: Some("handlr".to_string()),
                cwd: Some("/src/handlr".to_string()),
                workspace_id: Some("w2".to_string()),
                tab_id: None,
                status: "working".to_string(),
                focused: true,
            }],
        });
        round_trip(AgentsReadParams {
            pane_id: "p1".to_string(),
            lines: 12,
        });
        round_trip(AgentsReadResult {
            text: "ok".to_string(),
        });
        round_trip(AgentsSendParams {
            pane_id: "p1".to_string(),
            command: "continue".to_string(),
            handle: Some("keep-going".to_string()),
        });
        round_trip(AgentsSendResult::Sent);
        round_trip(AgentsSendResult::Pending {
            confirm_id: 3,
            handle: Some("keep-going".to_string()),
        });
        round_trip(AgentsFocusParams {
            target_id: "p1".to_string(),
        });
        round_trip(WatchParams {
            pane_id: "p1".to_string(),
        });
        round_trip(WatchResult {
            watched: vec!["w1:p1".to_string()],
        });
        round_trip(ConfirmReplyParams {
            confirm_id: 3,
            approve: true,
            nonce: Some("ab".repeat(32)),
        });
        round_trip(ConfirmCancelParams { confirm_id: 3 });
        round_trip(OutcomeResult {
            outcome: Outcome::Sent,
        });
        round_trip(ConfirmStatusParams {
            confirm_id: None,
            handle: Some("keep-going".to_string()),
        });
        round_trip(ConfirmStatusResult {
            confirm_id: 3,
            handle: None,
            outcome: Outcome::Expired,
        });
        round_trip(EventsSubscribeParams {
            topics: vec![topics::CONFIRM.to_string()],
        });
        round_trip(CancelParams {
            id: crate::envelope::Id::String("r9".to_string()),
        });
        round_trip(CancelResult { cancelled: true });
        round_trip(Empty {});
        round_trip(AgentStatusChanged {
            pane_id: "p1".to_string(),
            agent: "claude".to_string(),
            status: "blocked".to_string(),
        });
        round_trip(ConfirmRequested {
            confirm_id: 3,
            pane_id: "p1".to_string(),
            agent: "claude".to_string(),
            project: None,
            command: "continue".to_string(),
            requested_by: "local:mcp".to_string(),
            handle: Some("keep-going".to_string()),
            token: Some("copper-lantern-ninety".to_string()),
            reminder: false,
        });
        round_trip(ConfirmResolved {
            confirm_id: 3,
            handle: None,
            outcome: Outcome::Denied,
            detail: Some("too many wrong nonces".to_string()),
        });
    }

    #[test]
    fn wire_shapes() {
        assert_eq!(
            serde_json::to_value(AgentsSendResult::Pending {
                confirm_id: 3,
                handle: None
            })
            .unwrap(),
            json!({"status": "pending", "confirm_id": 3})
        );
        assert_eq!(
            serde_json::to_value(AgentsSendResult::Sent).unwrap(),
            json!({"status": "sent"})
        );
        // Lines default when omitted; the token is absent when not shown.
        let read: AgentsReadParams = serde_json::from_value(json!({"pane_id": "p1"})).unwrap();
        assert_eq!(read.lines, READ_LINES_DEFAULT);
        let requested: ConfirmRequested = serde_json::from_value(json!({
            "confirm_id": 1, "pane_id": "p1", "agent": "claude",
            "command": "continue", "requested_by": "local:mcp"
        }))
        .unwrap();
        assert_eq!(requested.token, None);
        assert!(!requested.reminder);
        assert_eq!(serde_json::to_value(Empty {}).unwrap(), json!({}));
    }
}
