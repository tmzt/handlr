//! `flankr.companion` version 1: conversational turns for the phone
//! (docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md, decisions 8–13 and
//! their amendments; names from plan 20261022_05, decision 7).
//!
//! These add to the methods in [`crate::common`], which a companion may
//! also call. Optional fields default and unknown fields are ignored, so a
//! client stays readable across additive changes.

use serde::{Deserialize, Serialize};

/// The protocol version served and spoken.
pub const VERSION: u32 = 1;

/// Method and notification names of `flankr.companion`.
pub mod methods {
    /// Phone → daemon: a turn; answered when it ends.
    pub const TURN_START: &str = "turn.start";
    /// Phone → daemon: end the running turn at once.
    pub const TURN_INTERRUPT: &str = "turn.interrupt";
    /// Notifications from the daemon.
    pub const TURN_SAY: &str = "turn.say";
    /// What the running turn is doing: a tool it runs, or thinking again
    /// ([`TurnProgress`](super::TurnProgress); plan 20261024_03, the
    /// 2026-10-30 22:08 EDT amendment). Added in libflankr_proto 0.5.9.
    pub const TURN_PROGRESS: &str = "turn.progress";
    pub const NOTICE: &str = "notice";
    pub const VOCABULARY_CHANGED: &str = "vocabulary.changed";
    /// herdr's focus changed (plan 20261023_02, decision 4).
    pub const FOCUS_CHANGED: &str = "focus.changed";
    /// What the daemon offers changed: sent with
    /// [`ServerCapabilities`](super::ServerCapabilities) whenever it does
    /// (plan 20261027_02, decision 7).
    pub const SERVER_CAPABILITIES: &str = "server.capabilities";
    /// Phone → daemon: a call to a plugin's own method, answered at once
    /// with [`PluginAccepted`](super::PluginAccepted) or an error; the
    /// plugin's answer follows as `plugin.result` when the call has an `id`
    /// (docs/design/20261028_04_PLUGINS_PLAN.md, decision 3.2). Added in
    /// libflankr_proto 0.5.2.
    pub const PLUGIN_CALL: &str = "plugin.call";
    /// Daemon → phone, a notification: a plugin's answer to a `plugin.call`
    /// ([`PluginResult`](super::PluginResult)).
    pub const PLUGIN_RESULT: &str = "plugin.result";
    /// Daemon → phone, a notification from a plugin during a call
    /// ([`PluginNotify`](super::PluginNotify)).
    pub const PLUGIN_NOTIFY: &str = "plugin.notify";
    /// Phone → daemon: switch a plugin on or off in the daemon's config
    /// ([`PluginSetEnabledParams`](super::PluginSetEnabledParams)); only a
    /// principal that may approve.
    pub const PLUGIN_SET_ENABLED: &str = "plugin.set_enabled";
}

/// `notice.kind` values.
pub mod notice_kinds {
    /// A watched agent needs input or finished.
    pub const AGENT_STATUS: &str = "agent_status";
    /// A watched agent stopped on a new question (plan 20261022_03,
    /// decision 8): "<spoken name> is asking: <question>".
    pub const PENDING_QUESTION: &str = "pending_question";
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
    /// Always `flankr`.
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
    /// What the daemon's own LLM use makes available (plan 20261027_02,
    /// decision 7). `None`, from an older daemon, means as before: both
    /// available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmInfo>,
    /// The terminal mode, when the daemon offers it on this connection
    /// (plan 20261027_07, decision 4). `None`, from an older daemon or one
    /// that reaches herdr remotely, means no `terminal.*` methods.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalInfo>,
    /// The daemon's built-in plugins, enabled or not (plan 20261028_04,
    /// decision 3.2, item 5). `None`, from an older daemon, means no
    /// plugins. Added in libflankr_proto 0.5.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins: Option<Vec<PluginInfo>>,
    /// Binary transfers over the link's kind 2 (plan 20261028_05,
    /// decision 3.3). `None`, from an older daemon or on a local socket,
    /// means no kind 2: the phone must not send one. Added in
    /// libflankr_proto 0.5.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<BlobInfo>,
}

/// The binary transfers a daemon offers on a link connection (plan
/// 20261028_05, decision 3.3). Negotiation only: no transfer is started,
/// answered or ended in JSON-RPC. Added in libflankr_proto 0.5.4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobInfo {
    /// The binary protocol's version: 1.
    pub version: u32,
    /// The namespaces this daemon serves now, e.g. `["images", "files",
    /// "git"]`. `"files"` is listed only while `[llm.uses.planning] read`
    /// is non-empty; `"git"` only when a `git` binary was found at start.
    pub namespaces: Vec<String>,
    /// The `[llm.uses.planning] read` patterns as configured (the limited
    /// syntax of plan 20261027_04, 11:20 EDT), so the app can draw only
    /// readable paths as links. Not a secret: the daemon still checks
    /// every REQUEST.
    pub read: Vec<String>,
    /// Largest image accepted, bytes (20 MiB).
    pub image_max: u64,
    /// Largest file served, bytes (8 MiB).
    pub file_max: u64,
    /// Largest `git` output served before it is cut, bytes (8 MiB).
    pub git_max: u64,
    /// Largest CHUNK data the daemon accepts, bytes (983040).
    pub chunk_max: u32,
    /// Most images in one `agents.send` (4).
    pub images_per_send: u32,
}

impl BlobInfo {
    /// Whether the daemon serves `ns` now.
    pub fn serves(&self, ns: &str) -> bool {
        self.namespaces.iter().any(|n| n == ns)
    }
}

/// The terminal mode a daemon offers (plan 20261027_07, decision 4).
/// Fields appended later default, so an older daemon's object still reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerminalInfo {
    /// Whether keystrokes may be sent on this connection
    /// (`terminal.input_arm`, `terminal.input`): the daemon's
    /// `[terminal] input` setting. Each agent's own setting and the arm
    /// still apply.
    #[serde(default)]
    pub input: bool,
    /// Whether the daemon serves `terminal.mouse` (its terminals can be
    /// read from herdr's attachment; plan 20261027_08, decision 10). Added
    /// in libflankr_proto 0.4.2; not written when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mouse: bool,
}

/// What the daemon's `[llm]` makes available to a companion (plan
/// 20261027_02, decision 7, as amended at 00:59 EDT). Which model runs
/// what is not sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmInfo {
    /// Turns can be started (`turn.start`).
    #[serde(default)]
    pub general_agent: bool,
    /// Card or spoken summaries are made.
    #[serde(default)]
    pub summaries: bool,
    /// Why not, when either is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl LlmInfo {
    /// Both available, as a daemon that sends no `llm` is taken to mean.
    pub fn available() -> Self {
        LlmInfo {
            general_agent: true,
            summaries: true,
            reason: None,
        }
    }
}

/// `server.capabilities`: what the daemon offers, sent whenever it
/// changes, so the phone adapts without reconnecting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// As [`HelloResult::llm`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmInfo>,
    /// As [`HelloResult::terminal`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalInfo>,
    /// As [`HelloResult::plugins`]; sent when the list changes (a toggle,
    /// a config reload). Added in libflankr_proto 0.5.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins: Option<Vec<PluginInfo>>,
    /// As [`HelloResult::blob`]; sent when it changes (a config reload
    /// that turns `files` on or off). Added in libflankr_proto 0.5.4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<BlobInfo>,
}

/// The largest `params`, `result` or notification `params` of a plugin
/// message, serialised: 256 KiB (plan 20261028_04, decision 3.2, item 2).
pub const PLUGIN_MAX_BYTES: usize = 262_144;

/// The longest plugin namespace, in bytes (decision 3.1, item 1).
pub const PLUGIN_NS_MAX_BYTES: usize = 128;

/// The longest plugin-local method name, in bytes (decision 3.2, item 2).
pub const PLUGIN_METHOD_MAX_BYTES: usize = 64;

/// Whether `ns` is a well-formed plugin package name: lowercase ASCII
/// segments `[a-z][a-z0-9_]*` joined by `.`, at least three segments, at
/// most [`PLUGIN_NS_MAX_BYTES`] bytes (decision 3.1, item 1). Which
/// namespaces a daemon accepts beyond this is its own rule.
pub fn is_plugin_ns(ns: &str) -> bool {
    if ns.is_empty() || ns.len() > PLUGIN_NS_MAX_BYTES {
        return false;
    }
    let mut segments = 0;
    for segment in ns.split('.') {
        let mut bytes = segment.bytes();
        match bytes.next() {
            Some(b'a'..=b'z') => {}
            _ => return false,
        }
        if !bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_')) {
            return false;
        }
        segments += 1;
    }
    segments >= 3
}

/// Whether `method` is a well-formed plugin-local method name:
/// `[a-z][a-z0-9_.]*`, at most [`PLUGIN_METHOD_MAX_BYTES`] bytes
/// (decision 3.2, item 2).
pub fn is_plugin_method(method: &str) -> bool {
    let mut bytes = method.bytes();
    method.len() <= PLUGIN_METHOD_MAX_BYTES
        && matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.'))
}

/// `plugin.call`'s params (decision 3.2, item 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCall {
    /// The plugin's package name.
    pub ns: String,
    /// The device's id for the call, unique within the connection and the
    /// namespace. With it, `plugin.result` follows; without it, the call is
    /// a notification to the plugin and nothing follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// The plugin-local method, see [`is_plugin_method`].
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    /// The pane id of the agent view the phone has open, if any: the
    /// call's focused project (decision 3.3, item 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// `plugin.call`'s result: the call was handed to the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PluginAccepted {}

/// `plugin.result`: a plugin's answer to the call with this `id`; exactly
/// one of `result` and `error`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginResult {
    pub ns: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::envelope::ErrorObject>,
}

/// `plugin.notify`: a plugin's notification, sent during a call on the
/// connection that called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginNotify {
    pub ns: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A built-in plugin, as listed in `hello` and `server.capabilities`
/// (decision 3.2, item 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub ns: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `0.1.0`, or `g1a2b3c4` before a release; without `ns@`.
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub menu: Vec<PluginMenuItem>,
    /// Why it can't be used now, e.g. "No project is open".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One action a plugin adds to the ⋮ menu, or, with an
/// [`icon`](Self::icon), to the title bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginMenuItem {
    /// Unique within the plugin.
    pub id: String,
    /// Shown in the ⋮ menu, and for a title icon when it moves there.
    #[serde(default)]
    pub label: String,
    /// `"modal"`, `"card"` or `"menu"` (a popup menu of items the device
    /// half gives, anchored to the title icon). An app that does not know
    /// a value opens it as `"modal"`.
    #[serde(default)]
    pub opens: String,
    /// `"overview"`, `"agent"` or `"both"`.
    #[serde(default)]
    pub screens: String,
    /// One of the app's own icon names (e.g. `"plans"`, the ☑): the item
    /// is a title-bar icon rather than a ⋮ item. A plugin cannot send its
    /// own glyph; an app draws an unknown name as the label's first letter
    /// in a ring. Left off the wire when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// `plugin.set_enabled`'s params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSetEnabledParams {
    pub ns: String,
    pub enabled: bool,
}

/// `plugin.set_enabled`'s result, sent once the config is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSetEnabledResult {
    pub enabled: bool,
}

/// `turn.start`. `turn_id` is chosen by the client, unique within the
/// session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStartParams {
    pub turn_id: u64,
    pub text: String,
    /// The pane id of the session the phone has open, sent from the agent
    /// view, so that session is the turn's current agent (plan 20261027_04,
    /// decision 3). `None` from the overview and from older apps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
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

/// `turn.progress`: the running turn's engine is calling `tool`, shown as
/// `label` (e.g. "Looking at recent commits…"); an empty `tool` means the
/// call returned and the engine is thinking again. Nothing is spoken for
/// it. Added in libflankr_proto 0.5.9.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnProgress {
    pub turn_id: u64,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub label: String,
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

    fn blob_info() -> BlobInfo {
        BlobInfo {
            version: 1,
            namespaces: vec!["images".into(), "files".into(), "git".into()],
            read: vec!["*.md".into(), "docs/**/*.md".into()],
            image_max: 20_971_520,
            file_max: 8_388_608,
            git_max: 8_388_608,
            chunk_max: 983_040,
            images_per_send: 4,
        }
    }

    fn planning_info() -> PluginInfo {
        PluginInfo {
            ns: "dev.flankr.plugins.planning".into(),
            name: "Planning".into(),
            description:
                "Lists this project's plans and their amendments, from docs/design. Read-only."
                    .into(),
            version: "0.1.0".into(),
            enabled: true,
            menu: vec![PluginMenuItem {
                id: "plans".into(),
                label: "Plans in this project".into(),
                opens: "modal".into(),
                screens: "both".into(),
                icon: None,
            }],
            reason: None,
        }
    }

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: T) {
        let text = serde_json::to_string(&value).unwrap();
        let back: T = serde_json::from_str(&text).unwrap();
        assert_eq!(back, value, "{text}");
    }

    #[test]
    fn every_type_round_trips() {
        round_trip(HelloParams {
            protocol: "flankr.companion".into(),
            version: VERSION,
            client: "s23".into(),
            session_id: Some("companion:s23".into()),
        });
        round_trip(HelloResult {
            server: "flankr".into(),
            server_version: "0.1.0".into(),
            protocol_version: VERSION,
            session_id: Some("companion:s23".into()),
            vocabulary: vec!["claude".into(), "HR".into()],
            paired: Some(true),
            llm: Some(LlmInfo {
                general_agent: false,
                summaries: true,
                reason: Some("[llm.uses.general_agent] enabled = false".into()),
            }),
            terminal: Some(TerminalInfo {
                input: true,
                mouse: true,
            }),
            plugins: Some(vec![planning_info()]),
            blob: Some(blob_info()),
        });
        round_trip(ServerCapabilities {
            llm: Some(LlmInfo::available()),
            terminal: None,
            plugins: None,
            blob: None,
        });
        round_trip(ServerCapabilities {
            llm: None,
            terminal: Some(TerminalInfo {
                input: false,
                mouse: false,
            }),
            plugins: Some(vec![]),
            blob: None,
        });
        round_trip(ServerCapabilities {
            llm: None,
            terminal: None,
            plugins: Some(vec![planning_info()]),
            blob: Some(blob_info()),
        });
        round_trip(TurnStartParams {
            turn_id: 4,
            text: "send continue to claude".into(),
            session: None,
        });
        round_trip(TurnStartParams {
            turn_id: 5,
            text: "what does the plan say".into(),
            session: Some("w1:p6".into()),
        });
        // An older app's turn.start, without `session`, still parses.
        let old: TurnStartParams =
            serde_json::from_value(serde_json::json!({"turn_id": 6, "text": "hi"})).unwrap();
        assert_eq!(old.session, None);
        round_trip(TurnResult {
            outcome: TurnOutcome::Interrupted,
            ran: vec![Ran {
                pane_id: "w2:pZ".into(),
                command: "continue".into(),
                name: Some("the flankr agent".into()),
                spoken: Some("the flankr agent".into()),
            }],
            interrupted: vec![Interrupted {
                pane_id: "w2:pZ".into(),
                name: "the flankr agent".into(),
                spoken: "the flankr agent".into(),
            }],
            detail: None,
        });
        round_trip(TurnInterruptParams { turn_id: 4 });
        round_trip(TurnSay {
            turn_id: 4,
            seq: 0,
            text: "Done.".into(),
        });
        round_trip(TurnProgress {
            turn_id: 4,
            tool: "recent_commits".into(),
            label: "Looking at recent commits\u{2026}".into(),
        });
        // Thinking again: no tool, no label; both default.
        let thinking: TurnProgress =
            serde_json::from_value(serde_json::json!({"turn_id": 4})).unwrap();
        assert_eq!(thinking.tool, "");
        assert_eq!(thinking.label, "");
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
            protocol: "flankr.companion".into(),
            version: VERSION,
            client: "tty".into(),
            session_id: None,
        };
        assert_eq!(
            serde_json::to_value(h).unwrap(),
            json!({"protocol": "flankr.companion", "version": 1, "client": "tty"})
        );
        // A hello from an older daemon has no `llm`: None.
        let h: HelloResult = serde_json::from_value(json!({
            "server": "flankr", "server_version": "0.1.0", "protocol_version": 1,
            "session_id": "companion:s23", "vocabulary": []
        }))
        .unwrap();
        assert_eq!(h.llm, None);
        assert!(serde_json::to_value(&h).unwrap().get("llm").is_none());
        // ... and no `terminal` (plan 20261027_07, decision 4): None, not
        // written.
        assert_eq!(h.terminal, None);
        assert!(serde_json::to_value(&h).unwrap().get("terminal").is_none());
        // LlmInfo's shape; a reason only when there is one.
        assert_eq!(
            serde_json::to_value(ServerCapabilities {
                llm: Some(LlmInfo::available()),
                terminal: None,
                plugins: None,
                blob: None,
            })
            .unwrap(),
            json!({"llm": {"general_agent": true, "summaries": true}})
        );
        let i: LlmInfo = serde_json::from_value(json!({"reason": "off"})).unwrap();
        assert!(!i.general_agent && !i.summaries);
        assert_eq!(i.reason.as_deref(), Some("off"));
        assert_eq!(methods::SERVER_CAPABILITIES, "server.capabilities");
        // TerminalInfo's shape; an empty object reads as no input.
        assert_eq!(
            serde_json::to_value(ServerCapabilities {
                llm: None,
                terminal: Some(TerminalInfo {
                    input: false,
                    mouse: false,
                }),
                plugins: None,
                blob: None,
            })
            .unwrap(),
            json!({"terminal": {"input": false}})
        );
        let c: ServerCapabilities = serde_json::from_value(json!({"terminal": {}})).unwrap();
        assert_eq!(
            c.terminal,
            Some(TerminalInfo {
                input: false,
                mouse: false,
            })
        );
        let c: ServerCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(c.terminal, None);
        // Plan 20261027_08, step 6: `mouse` defaults from older JSON and is
        // written only when true.
        let c: ServerCapabilities =
            serde_json::from_value(json!({"terminal": {"input": true}})).unwrap();
        assert_eq!(
            c.terminal,
            Some(TerminalInfo {
                input: true,
                mouse: false,
            })
        );
        assert_eq!(
            serde_json::to_value(TerminalInfo {
                input: true,
                mouse: true,
            })
            .unwrap(),
            json!({"input": true, "mouse": true})
        );
    }

    #[test]
    fn plugin_messages_round_trip() {
        use crate::envelope::{ErrorObject, codes};
        round_trip(PluginCall {
            ns: "dev.flankr.plugins.planning".into(),
            id: Some(7),
            method: "plans.show".into(),
            params: json!({"name": "20261028_04_PLUGINS_PLAN"}),
            session: Some("w1:p6".into()),
        });
        round_trip(PluginCall {
            ns: "dev.flankr.plugins.planning".into(),
            id: None,
            method: "plans.list".into(),
            params: json!({}),
            session: None,
        });
        round_trip(PluginAccepted {});
        assert_eq!(serde_json::to_value(PluginAccepted {}).unwrap(), json!({}));
        round_trip(PluginResult {
            ns: "dev.flankr.plugins.planning".into(),
            id: 7,
            result: Some(json!({"schema": 1, "plans": []})),
            error: None,
        });
        round_trip(PluginResult {
            ns: "dev.flankr.plugins.planning".into(),
            id: 8,
            result: None,
            error: Some(ErrorObject::new(codes::PLUGIN_TOO_LARGE, "too large")),
        });
        round_trip(PluginNotify {
            ns: "dev.flankr.plugins.planning".into(),
            method: "progress".into(),
            params: json!({"done": 3, "of": 9}),
        });
        round_trip(planning_info());
        round_trip(PluginSetEnabledParams {
            ns: "dev.flankr.plugins.planning".into(),
            enabled: true,
        });
        round_trip(PluginSetEnabledResult { enabled: false });
        assert_eq!(methods::PLUGIN_CALL, "plugin.call");
        assert_eq!(methods::PLUGIN_RESULT, "plugin.result");
        assert_eq!(methods::PLUGIN_NOTIFY, "plugin.notify");
        assert_eq!(methods::PLUGIN_SET_ENABLED, "plugin.set_enabled");
        assert_eq!(PLUGIN_MAX_BYTES, 256 * 1024);
    }

    #[test]
    fn plugin_wire_shapes() {
        // No `id` and no `session`: not written; `params` defaults to null.
        assert_eq!(
            serde_json::to_value(PluginCall {
                ns: "dev.flankr.plugins.test".into(),
                id: None,
                method: "ping".into(),
                params: serde_json::Value::Null,
                session: None,
            })
            .unwrap(),
            json!({"ns": "dev.flankr.plugins.test", "method": "ping", "params": null})
        );
        let c: PluginCall =
            serde_json::from_value(json!({"ns": "dev.flankr.plugins.test", "method": "ping"}))
                .unwrap();
        assert_eq!(c.id, None);
        assert!(c.params.is_null());
        // A result carries `result` or `error`, the other not written.
        assert_eq!(
            serde_json::to_value(PluginResult {
                ns: "dev.flankr.plugins.test".into(),
                id: 1,
                result: Some(json!([1])),
                error: None,
            })
            .unwrap(),
            json!({"ns": "dev.flankr.plugins.test", "id": 1, "result": [1]})
        );
        // PluginInfo: `reason` only when set; `menu` defaults.
        let i: PluginInfo =
            serde_json::from_value(json!({"ns": "dev.flankr.plugins.test"})).unwrap();
        assert!(!i.enabled && i.menu.is_empty() && i.reason.is_none());
        assert!(
            serde_json::to_value(planning_info())
                .unwrap()
                .get("reason")
                .is_none()
        );
        // An older daemon's hello and capabilities have no `plugins`: None,
        // and not written.
        let h: HelloResult = serde_json::from_value(json!({
            "server": "flankr", "server_version": "0.1.0", "protocol_version": 1,
            "vocabulary": []
        }))
        .unwrap();
        assert_eq!(h.plugins, None);
        assert!(serde_json::to_value(&h).unwrap().get("plugins").is_none());
        let c: ServerCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(c.plugins, None);
        // An empty list is written and read as empty, not None.
        let c = ServerCapabilities {
            llm: None,
            terminal: None,
            plugins: Some(vec![]),
            blob: None,
        };
        assert_eq!(serde_json::to_value(&c).unwrap(), json!({"plugins": []}));
        let back: ServerCapabilities = serde_json::from_value(json!({"plugins": []})).unwrap();
        assert_eq!(back.plugins, Some(vec![]));
    }

    #[test]
    fn plugin_menu_item_icon() {
        // With an icon: written, and read back.
        let item = PluginMenuItem {
            id: "plans".into(),
            label: "Plans in this project".into(),
            opens: "menu".into(),
            screens: "both".into(),
            icon: Some("plans".into()),
        };
        round_trip(item.clone());
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            json!({"id": "plans", "label": "Plans in this project", "opens": "menu",
                   "screens": "both", "icon": "plans"})
        );
        // Without one: left off the wire.
        let plain = PluginMenuItem { icon: None, ..item };
        round_trip(plain.clone());
        assert!(serde_json::to_value(&plain).unwrap().get("icon").is_none());
        // An older daemon's item has no `icon`: None.
        let old: PluginMenuItem = serde_json::from_value(json!({
            "id": "plans", "label": "Plans in this project", "opens": "modal", "screens": "both"
        }))
        .unwrap();
        assert_eq!(old.icon, None);
        assert_eq!(old.opens, "modal");
        // And a PluginInfo carrying both kinds round trips.
        let mut info = planning_info();
        info.menu.push(PluginMenuItem {
            id: "plans_icon".into(),
            label: "Plans".into(),
            opens: "menu".into(),
            screens: "both".into(),
            icon: Some("plans".into()),
        });
        round_trip(info);
    }

    #[test]
    fn plugin_names() {
        for good in [
            "dev.flankr.plugins.planning",
            "dev.flankr.plugins.test",
            "com.example.a_1",
            "a.b.c",
        ] {
            assert!(is_plugin_ns(good), "{good}");
        }
        let long = format!("dev.flankr.{}", "a".repeat(120));
        for bad in [
            "",
            "dev.flankr",
            "Dev.flankr.plugins",
            "dev..flankr",
            "dev.flankr.plugins.",
            "dev.flankr.1plugin",
            "dev.flankr.plug-in",
            "dev.flankr.plugins/planning",
            long.as_str(),
        ] {
            assert!(!is_plugin_ns(bad), "{bad}");
        }
        for good in ["plans.list", "plans.show", "a", "check_2"] {
            assert!(is_plugin_method(good), "{good}");
        }
        let long = "a".repeat(65);
        for bad in [
            "",
            "Plans",
            "1plans",
            "plans-list",
            "plans/list",
            "plugin call",
            long.as_str(),
        ] {
            assert!(!is_plugin_method(bad), "{bad}");
        }
        assert!(is_plugin_method(&"a".repeat(64)));
    }

    // Plan 20261028_05, step 1: `blob` in hello and server.capabilities.
    #[test]
    fn blob_info_is_appended_defaulted_and_left_out_when_absent() {
        let b = blob_info();
        assert!(b.serves("git") && !b.serves("dev.x.y"));
        assert_eq!(
            serde_json::to_value(&b).unwrap(),
            json!({
                "version": 1,
                "namespaces": ["images", "files", "git"],
                "read": ["*.md", "docs/**/*.md"],
                "image_max": 20_971_520,
                "file_max": 8_388_608,
                "git_max": 8_388_608,
                "chunk_max": 983_040,
                "images_per_send": 4
            })
        );
        // An older daemon's hello and capabilities: None, and not written.
        let h: HelloResult = serde_json::from_value(json!({"server": "flankr"})).unwrap();
        assert_eq!(h.blob, None);
        assert!(serde_json::to_value(&h).unwrap().get("blob").is_none());
        let c: ServerCapabilities = serde_json::from_value(json!({})).unwrap();
        assert_eq!(c.blob, None);
        assert!(serde_json::to_value(&c).unwrap().get("blob").is_none());
        // Written when present.
        let c = ServerCapabilities {
            llm: None,
            terminal: None,
            plugins: None,
            blob: Some(blob_info()),
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["blob"]["namespaces"], json!(["images", "files", "git"]));
        let back: ServerCapabilities = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);
    }
}
