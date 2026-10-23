//! `handlr.companion` version 1: conversational turns for the phone
//! (docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md, decisions 8–13 and
//! their amendments; names from plan 20261022_05, decision 7).
//!
//! These add to the methods in [`crate::common`], which a companion may
//! also call. Optional fields default and unknown fields are ignored, so a
//! client stays readable across additive changes.

use serde::{Deserialize, Serialize};

/// The protocol version served and spoken.
pub const VERSION: u32 = 1;

/// Method and notification names of `handlr.companion`.
pub mod methods {
    /// Phone → daemon: a turn; answered when it ends.
    pub const TURN_START: &str = "turn.start";
    /// Phone → daemon: end the running turn at once.
    pub const TURN_INTERRUPT: &str = "turn.interrupt";
    /// Notifications from the daemon.
    pub const TURN_SAY: &str = "turn.say";
    pub const NOTICE: &str = "notice";
    pub const VOCABULARY_CHANGED: &str = "vocabulary.changed";
    /// herdr's focus changed (plan 20261023_02, decision 4).
    pub const FOCUS_CHANGED: &str = "focus.changed";
}

/// `notice.kind` values.
pub mod notice_kinds {
    /// A watched agent needs input or finished.
    pub const AGENT_STATUS: &str = "agent_status";
}

/// `hello`, as a companion says it: the common fields and the
/// `session_id` it had, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloParams {
    pub protocol: String,
    pub version: u32,
    /// A label for the client. Not a credential.
    pub client: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// `hello`'s result for a companion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloResult {
    /// Always `handlr`.
    pub server: String,
    #[serde(default)]
    pub server_version: String,
    #[serde(default)]
    pub protocol_version: u32,
    /// `companion:<name>`: the conversation this connection continues.
    /// The daemon always sends it; the session follows the principal, not
    /// the `session_id` the client asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Names, projects, tab labels and their spoken forms, for the STT's
    /// vocabulary boost (plan 20261022_05, decision 9).
    #[serde(default)]
    pub vocabulary: Vec<String>,
    /// `true` on the first `hello` after pairing on this connection.
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

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Done,
    Interrupted,
    Error,
}

impl TurnOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            TurnOutcome::Done => "done",
            TurnOutcome::Interrupted => "interrupted",
            TurnOutcome::Error => "error",
        }
    }
}

/// A command a turn sent to a pane, directly or once approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ran {
    /// A handle only; present `name` instead.
    pub pane_id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
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
    pub outcome: TurnOutcome,
    #[serde(default)]
    pub ran: Vec<Ran>,
    #[serde(default)]
    pub interrupted: Vec<Interrupted>,
    /// Why a turn ended in `error`, for logs and display. What to say is
    /// sent as a `turn.say` first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// `turn.interrupt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnInterruptParams {
    pub turn_id: u64,
}

/// `turn.say`: speak this. Version 1 sends the whole reply as `seq: 0`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnSay {
    pub turn_id: u64,
    #[serde(default)]
    pub seq: u32,
    pub text: String,
}

/// `confirm.requested`, as sent to a companion: plan 01's fields
/// (`common::ConfirmRequested`) plus the question to speak. `handle` and
/// `token` also read as `label` and `phrase`, plan 02's earlier names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmRequested {
    pub confirm_id: u64,
    /// A handle only; present `name` instead.
    #[serde(default)]
    pub pane_id: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub requested_by: String,
    #[serde(default, alias = "label", skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// Sent only to principals that can approve.
    #[serde(default, alias = "phrase", skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default)]
    pub reminder: bool,
    /// `Approve sending "<command>" to <spoken name>?`, then ` For <handle
    /// words>.` when there is a handle (plan 02, 12:06 EDT amendment).
    pub text: String,
}

/// `notice`: speak when free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notice {
    /// Increasing within a connection.
    #[serde(default)]
    pub id: u64,
    /// See [`notice_kinds`].
    #[serde(default)]
    pub kind: String,
    /// A handle only; present `name` instead.
    #[serde(default)]
    pub pane_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
    pub text: String,
}

/// `vocabulary.changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyChanged {
    pub vocabulary: Vec<String>,
}

/// `focus.changed`: herdr's focused pane, sent when it changes, alongside
/// `vocabulary.changed` after the daemon's snapshot polls. `None` when
/// nothing is focused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusChanged {
    #[serde(default)]
    pub focus: Option<crate::common::FocusInfo>,
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
            protocol: "handlr.companion".into(),
            version: VERSION,
            client: "s23".into(),
            session_id: Some("companion:s23".into()),
        });
        round_trip(HelloResult {
            server: "handlr".into(),
            server_version: "0.1.0".into(),
            protocol_version: VERSION,
            session_id: Some("companion:s23".into()),
            vocabulary: vec!["claude".into(), "HR".into()],
            paired: Some(true),
        });
        round_trip(TurnStartParams {
            turn_id: 4,
            text: "send continue to claude".into(),
        });
        round_trip(TurnResult {
            outcome: TurnOutcome::Interrupted,
            ran: vec![Ran {
                pane_id: "w2:pZ".into(),
                command: "continue".into(),
                name: Some("the handlr agent".into()),
                spoken: Some("the handlr agent".into()),
            }],
            interrupted: vec![Interrupted {
                pane_id: "w2:pZ".into(),
                name: "the handlr agent".into(),
                spoken: "the handlr agent".into(),
            }],
            detail: None,
        });
        round_trip(TurnInterruptParams { turn_id: 4 });
        round_trip(TurnSay {
            turn_id: 4,
            seq: 0,
            text: "Done.".into(),
        });
        round_trip(ConfirmRequested {
            confirm_id: 3,
            pane_id: "w2:pZ".into(),
            agent: "claude".into(),
            name: Some("Claude".into()),
            spoken: Some("Claude".into()),
            project: None,
            command: "continue".into(),
            requested_by: "paired:s23".into(),
            handle: Some("keep-going".into()),
            token: Some("copper-lantern-ninety".into()),
            reminder: false,
            text: "Approve sending \"continue\" to Claude? For keep going.".into(),
        });
        round_trip(Notice {
            id: 1,
            kind: notice_kinds::AGENT_STATUS.into(),
            pane_id: "w2:pZ".into(),
            name: Some("Claude".into()),
            spoken: Some("Claude".into()),
            text: "Claude finished.".into(),
        });
        round_trip(VocabularyChanged {
            vocabulary: vec!["claude".into()],
        });
        round_trip(FocusChanged {
            focus: Some(crate::common::FocusInfo {
                pane_id: "w2:p4".into(),
                project: Some("libmorph".into()),
                agent: Some("Claude".into()),
                tab_label: Some("-ad".into()),
                tab_number: Some(4),
                name: "Claude in libmorph".into(),
                spoken: "Claude in libmorph".into(),
            }),
        });
        round_trip(FocusChanged { focus: None });
    }

    #[test]
    fn wire_shapes() {
        assert_eq!(
            serde_json::to_value(TurnResult {
                outcome: TurnOutcome::Done,
                ran: vec![],
                interrupted: vec![],
                detail: None,
            })
            .unwrap(),
            json!({"outcome": "done", "ran": [], "interrupted": []})
        );
        // A result without `interrupted` still reads.
        let r: TurnResult = serde_json::from_value(json!({"outcome": "done", "ran": []})).unwrap();
        assert!(r.interrupted.is_empty());
        // Plan 02's earlier names for the handle and the token.
        let c: ConfirmRequested = serde_json::from_value(json!({
            "confirm_id": 3, "label": "keep-going", "phrase": "copper-lantern-ninety",
            "text": "Approve sending \"continue\" to Claude?"
        }))
        .unwrap();
        assert_eq!(c.handle.as_deref(), Some("keep-going"));
        assert_eq!(c.token.as_deref(), Some("copper-lantern-ninety"));
        // Nothing focused: `focus` is null, and an empty object reads too.
        assert_eq!(
            serde_json::to_value(FocusChanged { focus: None }).unwrap(),
            json!({"focus": null})
        );
        let f: FocusChanged = serde_json::from_value(json!({})).unwrap();
        assert_eq!(f.focus, None);
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
