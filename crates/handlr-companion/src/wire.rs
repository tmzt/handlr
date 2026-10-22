//! The `handlr.companion` messages that `libhandlr_proto::common` does not
//! have (plan 20261022_02, decision 8, and its amendments).
//!
//! **Temporary.** These types live here only until plan 20261022_02 step 4
//! adds `libhandlr_proto::companion`; then this module is replaced by that
//! one. The envelope and the common methods (`confirm.reply`,
//! `confirm.resolved`) already come from `libhandlr_proto`.
//!
//! Every type is lenient where the plans leave a field open: optional
//! fields default, and unknown fields are ignored.

use serde::{Deserialize, Serialize};

/// The protocol version this client speaks.
pub const VERSION: u32 = 1;

/// Method and notification names of `handlr.companion`.
pub mod methods {
    pub const TURN_START: &str = "turn.start";
    pub const TURN_INTERRUPT: &str = "turn.interrupt";
    /// Notifications from the daemon.
    pub const TURN_SAY: &str = "turn.say";
    pub const NOTICE: &str = "notice";
    pub const VOCABULARY_CHANGED: &str = "vocabulary.changed";
}

/// `hello`, as a companion says it: the common fields and the
/// `session_id` it had, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloParams {
    pub protocol: String,
    pub version: u32,
    pub client: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// `hello`'s result for a companion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloResult {
    pub server: String,
    #[serde(default)]
    pub server_version: String,
    #[serde(default)]
    pub protocol_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub vocabulary: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired: Option<bool>,
}

/// `turn.start`. `turn_id` is chosen by the client, unique within the
/// session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStartParams {
    pub turn_id: u64,
    pub text: String,
}

/// A command a turn sent to a pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ran {
    pub pane_id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// An agent a turn's interrupt sent its interrupt key to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interrupted {
    pub pane_id: String,
    #[serde(default)]
    pub name: String,
    /// The spoken name, as in "Stopped <spoken>."
    #[serde(default)]
    pub spoken: String,
}

/// `turn.start`'s result, sent when the turn ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnResult {
    /// `done`, `interrupted` or `error`.
    pub outcome: String,
    #[serde(default)]
    pub ran: Vec<Ran>,
    #[serde(default)]
    pub interrupted: Vec<Interrupted>,
}

/// `turn.interrupt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnInterruptParams {
    pub turn_id: u64,
}

/// `turn.say`: speak this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnSay {
    pub turn_id: u64,
    #[serde(default)]
    pub seq: u32,
    pub text: String,
}

/// `confirm.requested`, as sent to a companion principal: plan 01's fields
/// plus the question `text`, the `label` and the `phrase`. `label` and
/// `phrase` also accept the names `common::ConfirmRequested` has today
/// (`handle`, `token`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmRequested {
    pub confirm_id: u64,
    #[serde(default)]
    pub pane_id: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub requested_by: String,
    #[serde(default, alias = "handle", skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, alias = "token", skip_serializing_if = "Option::is_none")]
    pub phrase: Option<String>,
    /// The question to speak, built by the daemon.
    pub text: String,
    #[serde(default)]
    pub reminder: bool,
}

/// `notice`: speak when free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub text: String,
}

/// `vocabulary.changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyChanged {
    pub vocabulary: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn confirm_requested_takes_todays_field_names_too() {
        let c: ConfirmRequested = serde_json::from_value(json!({
            "confirm_id": 3, "pane_id": "w2:pZ", "agent": "claude",
            "command": "continue", "requested_by": "companion:s23",
            "handle": "keep-going", "token": "copper-lantern-ninety",
            "text": "Approve sending \"continue\" to Claude?"
        }))
        .unwrap();
        assert_eq!(c.label.as_deref(), Some("keep-going"));
        assert_eq!(c.phrase.as_deref(), Some("copper-lantern-ninety"));
    }

    #[test]
    fn a_turn_result_reads_with_or_without_interrupted() {
        let r: TurnResult = serde_json::from_value(json!({"outcome": "done", "ran": []})).unwrap();
        assert!(r.interrupted.is_empty());
        let r: TurnResult = serde_json::from_value(json!({
            "outcome": "interrupted", "ran": [{"pane_id": "p1", "command": "continue"}],
            "interrupted": [{"pane_id": "p1", "name": "handlr", "spoken": "the handlr agent"}]
        }))
        .unwrap();
        assert_eq!(r.interrupted[0].spoken, "the handlr agent");
        assert_eq!(r.ran[0].command, "continue");
    }

    #[test]
    fn hello_leaves_out_a_missing_session_id() {
        let h = HelloParams {
            protocol: "handlr.companion".into(),
            version: VERSION,
            client: "tty".into(),
            session_id: None,
        };
        assert_eq!(
            serde_json::to_value(h).unwrap(),
            json!({"protocol": "handlr.companion", "version": 1, "client": "tty"})
        );
    }
}
