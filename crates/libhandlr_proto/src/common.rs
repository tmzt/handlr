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
    /// Send an agent its interrupt key (plan 20261022_02, 18:03 EDT
    /// amendment).
    pub const AGENTS_INTERRUPT: &str = "agents.interrupt";
    pub const WATCH_ADD: &str = "watch.add";
    pub const WATCH_REMOVE: &str = "watch.remove";
    pub const CONFIRM_REPLY: &str = "confirm.reply";
    pub const CONFIRM_CANCEL: &str = "confirm.cancel";
    pub const CONFIRM_STATUS: &str = "confirm.status";
    pub const EVENTS_SUBSCRIBE: &str = "events.subscribe";
    /// An agent's own session: its last entries and pending question (plan
    /// 20261022_03, decision 9, the agent view). Read-only.
    pub const SESSION_PEEK: &str = "session.peek";
    /// A reply read aloud, measured and, when complex, summarised for the
    /// ear (plan 20261023_05, as amended at 15:45 EDT). Companions only.
    pub const SPEECH_SUMMARIZE: &str = "speech.summarize";

    /// Notifications from the daemon.
    pub const AGENT_STATUS_CHANGED: &str = "agent.status_changed";
    pub const CONFIRM_REQUESTED: &str = "confirm.requested";
    pub const CONFIRM_RESOLVED: &str = "confirm.resolved";

    /// Local only (the daemon socket): the pairing URL and string to show
    /// (plan 20261022_02, amendment of 23:07 EDT). Nothing is held pending.
    pub const LINK_PAIR_BEGIN: &str = "link.pair_begin";
    /// Local only: add a device from the code it shows.
    pub const LINK_PAIR_ADD: &str = "link.pair_add";
    /// Notification, to the connection that added the device: its first
    /// `hello` was answered.
    pub const LINK_PAIR_DONE: &str = "link.pair_done";
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
///
/// `pane_id` is a handle for later requests and is never shown or spoken;
/// a client presents `name` and says `spoken` (plan 20261022_05,
/// decision 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub pane_id: String,
    /// The shown name: the shortest unique name with labels as written,
    /// such as "Claude in libmorph, tab -mb".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The spoken form of the same name, such as "Claude in libmorph, tab
    /// em bee".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
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
    /// The tab's label as the user typed it (`17-hr`, `-mb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_label: Option<String>,
    /// The tab's number, unique within its workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_number: Option<u32>,
    /// herdr's `agent_status`: `working`, `idle`, `blocked`, `done`,
    /// `unknown`, or a runner's own word.
    pub status: String,
    pub focused: bool,
    /// The agent's own session, where one was found on the daemon's machine
    /// (plan 20261022_03, step 4). Absent for a shell, an agent without a
    /// reader, or a remote herdr.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<AgentSessionInfo>,
}

/// How an agent's session file was found (plan 20261022_03, the 16:23 EDT
/// amendment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMatched {
    /// By the session id herdr reports for the pane.
    Id,
    /// By the pane's cwd: the newest session there, which may belong to
    /// another agent in the same directory.
    Cwd,
}

/// An agent's session, as `agents.list` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionInfo {
    /// The session's id, where it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub matched: SessionMatched,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<PendingQuestion>,
}

/// Entries `session.peek` returns by default, and the most it may be asked
/// for.
pub const PEEK_ENTRIES_DEFAULT: usize = 40;
pub const PEEK_ENTRIES_MAX: usize = 200;

fn peek_entries_default() -> usize {
    PEEK_ENTRIES_DEFAULT
}

/// `session.peek`: the target is resolved like `agents.read`'s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPeekParams {
    pub pane_id: String,
    /// 1 to `PEEK_ENTRIES_MAX`.
    #[serde(default = "peek_entries_default")]
    pub entries: usize,
}

/// An agent's session as `session.peek` reads it. The entries are
/// redacted, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPeek {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub matched: SessionMatched,
    pub entries: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<PendingQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPeekResult {
    /// `None` for a shell, an agent without a reader or session file, or a
    /// remote herdr, whose session files are not on the daemon's machine.
    pub session: Option<SessionPeek>,
    /// herdr's `agent_status`, as `AgentInfo::status`.
    pub status: String,
}

/// `speech.summarize`: the reply to read aloud, named by its pane and its
/// entry's time (plan 20261023_05, decisions 3 and 4, as amended).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechSummarizeParams {
    pub pane_id: String,
    /// The entry's `ts`, RFC 3339.
    pub ts: String,
    /// The reply as the client has it: used only when the daemon cannot
    /// find the entry.
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechSummarizeResult {
    /// The spoken summary, when the reply is complex; `None` when it is
    /// not, and the client reads the reply as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
    /// Whether the daemon measured the reply as complex.
    #[serde(default)]
    pub complex: bool,
}

/// The pane herdr reports as focused, named as `AgentInfo` names it
/// (plan 20261023_02, decision 4).
///
/// `pane_id` is a handle for later requests and is never shown or spoken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusInfo {
    pub pane_id: String,
    /// The base name: the workspace label, or the cwd's basename (plan
    /// 20261022_05, decision 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The agent as shown, such as "Claude"; `None` for a pane without an
    /// agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The tab's label as the user typed it (`-ad`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_number: Option<u32>,
    /// The shown name (see `AgentInfo::name`).
    #[serde(default)]
    pub name: String,
    /// The spoken name (see `AgentInfo::spoken`).
    #[serde(default)]
    pub spoken: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsListResult {
    pub agents: Vec<AgentInfo>,
    /// herdr's focused pane, if any (plan 20261023_02, decision 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<FocusInfo>,
}

/// Who an [`Entry`] of an agent's session is from (plan 20261022_03,
/// decision 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    /// A tool's result, given back to the agent.
    Tool,
}

/// What an [`Entry`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Text,
    ToolUse,
    ToolResult,
}

/// One entry of an agent's own session transcript, the same shape for
/// every agent (plan 20261022_03, decision 2). Secrets are redacted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// RFC 3339, from the file.
    pub ts: String,
    pub role: Role,
    pub kind: EntryKind,
    /// The tool or function name, for `tool_use` and, where known,
    /// `tool_result`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// At most `ENTRY_SUMMARY_MAX` characters, cut at a character boundary
    /// with "…".
    pub summary: String,
}

/// The longest `Entry::summary`, in characters.
pub const ENTRY_SUMMARY_MAX: usize = 500;

/// The question an agent stopped on (plan 20261022_03, decision 8): the
/// final paragraph of its last reply, when it asks something and the user
/// has not answered. Redacted, and at most `PENDING_QUESTION_MAX`
/// characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub text: String,
    /// The reply's timestamp, RFC 3339.
    pub ts: String,
}

/// The longest `PendingQuestion::text`, in characters.
pub const PENDING_QUESTION_MAX: usize = 400;

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

/// What `agents.send` sends (plan 20261023_02, decision 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendKind {
    /// An allowed or arbitrary command, as before.
    #[default]
    Command,
    /// A dictated message: the strict character filter, at most 4000
    /// characters, always confirmed, gated by `allow_messages`.
    Message,
}

impl SendKind {
    pub fn is_command(&self) -> bool {
        *self == SendKind::Command
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsSendParams {
    pub pane_id: String,
    /// The command, or a message's text; also read as `text`.
    #[serde(alias = "text")]
    pub command: String,
    /// `command` (the default) or `message`.
    #[serde(default, skip_serializing_if = "SendKind::is_command")]
    pub kind: SendKind,
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

/// `agents.interrupt`: the target is resolved like `agents.send`'s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsInterruptParams {
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsInterruptResult {
    /// Whether the key was sent: `false` when the agent's `interrupt_key`
    /// is empty.
    pub interrupted: bool,
    /// The agent's shown and spoken names.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub spoken: String,
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
    /// A handle only; present `name` instead.
    pub pane_id: String,
    pub agent: String,
    /// The agent's shown name (see `AgentInfo::name`). Absent when herdr
    /// could not be read as the request was forwarded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The agent's spoken name (see `AgentInfo::spoken`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
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

/// `link.pair_begin` takes no parameters; it serialises as `{}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LinkPairBeginParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPairBeginResult {
    /// The pairing URL, `handlr://<host>:<port>/pair#handlr1…`, also shown
    /// as the QR code.
    pub url: String,
    /// The pairing string, `handlr1…`: the server's key and addresses.
    pub pairing: String,
    /// The addresses the pairing string carries.
    pub addresses: Vec<String>,
    /// Things the person pairing should know, such as a missing
    /// `tailscale serve` forward and the command that adds it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// `link.pair_add`: add the device whose code is `code` to `auth.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPairAddParams {
    /// The device code, `handlrkey1…`, made for this server's key.
    pub code: String,
    /// The name to pair it as. Defaults to the device's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What the device may say `hello` with: `companion`, `harness`.
    /// Defaults to `["companion"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocols: Option<Vec<String>>,
    /// Replace an `auth.toml` entry of the same name, and close the old
    /// key's connections.
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPairAddResult {
    /// The name it was added as.
    pub name: String,
    /// What it may say `hello` with.
    pub protocols: Vec<String>,
}

/// `link.pair_done`: the device added by `link.pair_add` said `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPairDone {
    pub name: String,
    pub paired: bool,
    /// Why it did not pair, if it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    fn session_entries_round_trip() {
        round_trip(Entry {
            ts: "2026-10-23T06:00:00.000Z".into(),
            role: Role::Assistant,
            kind: EntryKind::ToolUse,
            tool: Some("Bash".into()),
            summary: "Bash {\"command\":\"ls\"}".into(),
        });
        round_trip(PendingQuestion {
            text: "Shall I go on?".into(),
            ts: "2026-10-23T06:00:00.000Z".into(),
        });
        assert_eq!(
            serde_json::to_value(EntryKind::ToolResult).unwrap(),
            json!("tool_result")
        );
        let text: Entry = serde_json::from_value(json!({
            "ts": "t", "role": "user", "kind": "text", "summary": "hi"
        }))
        .unwrap();
        assert_eq!(text.tool, None);
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
                name: Some("the handlr agent".to_string()),
                spoken: Some("the handlr agent".to_string()),
                agent: "claude".to_string(),
                project: Some("handlr".to_string()),
                cwd: Some("/src/handlr".to_string()),
                workspace_id: Some("w2".to_string()),
                tab_id: Some("w2:t1".to_string()),
                tab_label: Some("17-hr".to_string()),
                tab_number: Some(17),
                status: "working".to_string(),
                focused: true,
                session: Some(AgentSessionInfo {
                    session_id: Some("00000000-0000-4000-8000-0000000000a1".to_string()),
                    matched: SessionMatched::Id,
                    pending_question: Some(PendingQuestion {
                        text: "Shall I go on?".to_string(),
                        ts: "2026-10-23T06:00:00.000Z".to_string(),
                    }),
                }),
            }],
            focus: Some(FocusInfo {
                pane_id: "w2:pZ".to_string(),
                project: Some("handlr".to_string()),
                agent: Some("Claude".to_string()),
                tab_label: Some("17-hr".to_string()),
                tab_number: Some(17),
                name: "the handlr agent".to_string(),
                spoken: "the handlr agent".to_string(),
            }),
        });
        round_trip(AgentsListResult {
            agents: vec![],
            focus: None,
        });
        round_trip(AgentsReadParams {
            pane_id: "p1".to_string(),
            lines: 12,
        });
        round_trip(SessionPeekParams {
            pane_id: "p1".to_string(),
            entries: 200,
        });
        round_trip(SessionPeekResult {
            session: Some(SessionPeek {
                session_id: None,
                matched: SessionMatched::Cwd,
                entries: vec![Entry {
                    ts: "t".to_string(),
                    role: Role::User,
                    kind: EntryKind::Text,
                    tool: None,
                    summary: "hi".to_string(),
                }],
                pending_question: None,
            }),
            status: "idle".to_string(),
        });
        round_trip(SessionPeekResult {
            session: None,
            status: "unknown".to_string(),
        });
        round_trip(SpeechSummarizeParams {
            pane_id: "w2:pZ".to_string(),
            ts: "2026-10-23T06:00:00.000Z".to_string(),
            text: "Done: `src/prompts.rs` added.".to_string(),
        });
        round_trip(SpeechSummarizeResult {
            spoken: Some("The prompts module is added.".to_string()),
            complex: true,
        });
        round_trip(SpeechSummarizeResult {
            spoken: None,
            complex: false,
        });
        round_trip(AgentsReadResult {
            text: "ok".to_string(),
        });
        round_trip(AgentsSendParams {
            pane_id: "p1".to_string(),
            command: "continue".to_string(),
            kind: SendKind::Command,
            handle: Some("keep-going".to_string()),
        });
        round_trip(AgentsSendParams {
            pane_id: "w2:pZ".to_string(),
            command: "please run the tests".to_string(),
            kind: SendKind::Message,
            handle: Some("message-claude-handlr-please-run".to_string()),
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
        round_trip(AgentsInterruptParams {
            target: "hr".to_string(),
        });
        round_trip(AgentsInterruptResult {
            interrupted: true,
            name: "the handlr agent".to_string(),
            spoken: "the handlr agent".to_string(),
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
            name: Some("Claude in libmorph, tab -mb".to_string()),
            spoken: Some("Claude in libmorph, tab em bee".to_string()),
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
        round_trip(LinkPairBeginParams {});
        round_trip(LinkPairBeginResult {
            url: "handlr://127.0.0.1:7347/pair#handlr1abc".to_string(),
            pairing: "handlr1abc".to_string(),
            addresses: vec!["127.0.0.1:7347".to_string()],
            warnings: vec!["no forward".to_string()],
        });
        round_trip(LinkPairAddParams {
            code: "handlrkey1abc".to_string(),
            name: Some("s23".to_string()),
            protocols: Some(vec!["harness".to_string()]),
            replace: true,
        });
        round_trip(LinkPairAddResult {
            name: "s23".to_string(),
            protocols: vec!["companion".to_string()],
        });
        round_trip(LinkPairDone {
            name: "s23".to_string(),
            paired: true,
            error: None,
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
        // Names are additive: older JSON without them still parses, and
        // they are left out when absent.
        assert_eq!((requested.name, requested.spoken), (None, None));
        let agent: AgentInfo = serde_json::from_value(json!({
            "pane_id": "p1", "agent": "claude", "status": "idle", "focused": false
        }))
        .unwrap();
        assert_eq!(
            (
                agent.name.as_deref(),
                agent.tab_label.as_deref(),
                agent.tab_number
            ),
            (None, None, None)
        );
        assert_eq!(agent.session, None);
        let shown = serde_json::to_value(&agent).unwrap();
        for key in ["name", "spoken", "tab_label", "tab_number", "session"] {
            assert!(shown.get(key).is_none(), "{key} in {shown}");
        }
        let named = serde_json::to_value(AgentInfo {
            name: Some("Claude".to_string()),
            spoken: Some("Claude".to_string()),
            tab_label: Some("-mb".to_string()),
            tab_number: Some(4),
            ..agent
        })
        .unwrap();
        assert_eq!(named["name"], "Claude");
        assert_eq!(named["tab_label"], "-mb");
        assert_eq!(named["tab_number"], 4);
        assert_eq!(serde_json::to_value(Empty {}).unwrap(), json!({}));

        // Focus is additive: a list without it reads, and none is left out.
        let list: AgentsListResult = serde_json::from_value(json!({"agents": []})).unwrap();
        assert_eq!(list.focus, None);
        assert_eq!(serde_json::to_value(&list).unwrap(), json!({"agents": []}));
        let focus: FocusInfo = serde_json::from_value(json!({"pane_id": "p1"})).unwrap();
        assert_eq!((focus.agent, focus.name.as_str()), (None, ""));

        // `kind` defaults to a command and is left out for one; a message
        // may give its text as `text`.
        let send: AgentsSendParams =
            serde_json::from_value(json!({"pane_id": "p1", "command": "continue"})).unwrap();
        assert_eq!(send.kind, SendKind::Command);
        assert_eq!(
            serde_json::to_value(&send).unwrap(),
            json!({"pane_id": "p1", "command": "continue"})
        );
        let message: AgentsSendParams = serde_json::from_value(json!({
            "pane_id": "p1", "text": "please run the tests", "kind": "message",
            "handle": "message-claude-please-run"
        }))
        .unwrap();
        assert_eq!(message.kind, SendKind::Message);
        assert_eq!(message.command, "please run the tests");
        assert_eq!(
            serde_json::to_value(&message).unwrap()["kind"],
            json!("message")
        );

        // session.peek: 40 entries by default; a session found by cwd has
        // no id, and the pending question is left out when there is none.
        let peek: SessionPeekParams = serde_json::from_value(json!({"pane_id": "p1"})).unwrap();
        assert_eq!(peek.entries, PEEK_ENTRIES_DEFAULT);
        assert_eq!(
            serde_json::to_value(AgentSessionInfo {
                session_id: None,
                matched: SessionMatched::Cwd,
                pending_question: None,
            })
            .unwrap(),
            json!({"matched": "cwd"})
        );
        assert_eq!(
            serde_json::to_value(SessionPeekResult {
                session: None,
                status: "idle".to_string()
            })
            .unwrap(),
            json!({"session": null, "status": "idle"})
        );

        // speech.summarize: a plain reply's answer has no `spoken`, and the
        // client's text may be left out.
        assert_eq!(
            serde_json::to_value(SpeechSummarizeResult {
                spoken: None,
                complex: false
            })
            .unwrap(),
            json!({"complex": false})
        );
        let summarize: SpeechSummarizeParams =
            serde_json::from_value(json!({"pane_id": "p1", "ts": "t"})).unwrap();
        assert_eq!(summarize.text, "");

        // link.pair_begin takes nothing; link.pair_add needs only the code.
        assert_eq!(
            serde_json::to_value(LinkPairBeginParams {}).unwrap(),
            json!({})
        );
        let add: LinkPairAddParams =
            serde_json::from_value(json!({"code": "handlrkey1abc"})).unwrap();
        assert_eq!(
            add,
            LinkPairAddParams {
                code: "handlrkey1abc".to_string(),
                name: None,
                protocols: None,
                replace: false,
            }
        );
        assert_eq!(
            serde_json::to_value(LinkPairDone {
                name: "s23".to_string(),
                paired: true,
                error: None
            })
            .unwrap(),
            json!({"name": "s23", "paired": true})
        );
    }
}
