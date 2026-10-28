//! The methods every flankr protocol shares (plan 20261022_01, decision 7,
//! as amended; the confirmation fields are named in CHANGES.md,
//! 2026-10-22, "Confirmations: handle, token and nonce").

use serde::{Deserialize, Serialize};

/// Protocol names, as `hello` declares them.
pub mod protocols {
    pub const HARNESS: &str = "flankr.harness";
    pub const COMPANION: &str = "flankr.companion";
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
    /// A companion's options for its connection, such as whether its
    /// output is spoken (plan 20261023_05, the 16:28 EDT amendment, A4).
    /// Answered `{}`.
    pub const CLIENT_OPTIONS: &str = "client.options";

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
    /// Over a paired link only: the daemon forgets the calling device,
    /// removing it from `auth.toml`, and closes the link (plan 20261024_02,
    /// decision 6). No params; answered `{}`.
    pub const LINK_UNPAIR: &str = "link.unpair";

    /// Subscribe to one pane's session: a backfill of sections, then each
    /// new or changed one pushed as `session.delivery` (plan 20261023_03,
    /// section 4, as amended at 14:54 EDT, A5). Companions and local
    /// clients.
    pub const SESSION_SUBSCRIBE: &str = "session.subscribe";
    /// End a subscription. Answered `{}`.
    pub const SESSION_UNSUBSCRIBE: &str = "session.unsubscribe";
    /// A page of a subscription's sections from before a serial.
    pub const SESSION_HISTORY: &str = "session.history";
    /// Section contents, or outline slots, by index range (plan
    /// 20261024_01, decision 4, as amended at 00:03 EDT).
    pub const SESSION_CONTENT: &str = "session.content";
    /// Notification: a subscription's new or changed sections.
    pub const SESSION_DELIVERY: &str = "session.delivery";

    /// Start mirroring a session's pane on this connection (plan
    /// 20261027_07, decision 2): [`TerminalOpenParams`](super::TerminalOpenParams)
    /// → [`TerminalOpenResult`](super::TerminalOpenResult). Companions only;
    /// frames then arrive as the link's terminal kind. An older daemon
    /// answers `METHOD_NOT_FOUND`.
    pub const TERMINAL_OPEN: &str = "terminal.open";
    /// Stop a terminal: [`TerminalCloseParams`](super::TerminalCloseParams),
    /// answered `{}`.
    pub const TERMINAL_CLOSE: &str = "terminal.close";
    /// Notification, phone → daemon: the newest frame the phone holds
    /// ([`TerminalAckParams`](super::TerminalAckParams)).
    pub const TERMINAL_ACK: &str = "terminal.ack";
    /// Arm keyboard input for 60 s
    /// ([`TerminalInputArmParams`](super::TerminalInputArmParams) →
    /// [`TerminalInputArmResult`](super::TerminalInputArmResult); plan
    /// 20261027_07, decision 6).
    pub const TERMINAL_INPUT_ARM: &str = "terminal.input_arm";
    /// Keys into an armed terminal
    /// ([`TerminalInputParams`](super::TerminalInputParams)), answered `{}`.
    pub const TERMINAL_INPUT: &str = "terminal.input";
    /// Scroll an armed terminal's pane, as a mouse wheel does
    /// ([`TerminalScrollParams`](super::TerminalScrollParams) →
    /// [`TerminalScrollResult`](super::TerminalScrollResult)). Gated as
    /// [`TERMINAL_INPUT`] is.
    pub const TERMINAL_SCROLL: &str = "terminal.scroll";
    /// Notification, daemon → phone: a terminal ended
    /// ([`TerminalClosedParams`](super::TerminalClosedParams)).
    pub const TERMINAL_CLOSED: &str = "terminal.closed";
    /// A mouse event into an armed, attached terminal
    /// ([`TerminalMouseParams`](super::TerminalMouseParams)), answered `{}`
    /// (plan 20261027_08, decision 10). Gated as [`TERMINAL_INPUT`] is; a
    /// daemon offers it when its `TerminalInfo::mouse` is true. Added in
    /// 0.4.2.
    pub const TERMINAL_MOUSE: &str = "terminal.mouse";
    /// The phone's fit for a terminal changed
    /// ([`TerminalResizeParams`](super::TerminalResizeParams)), answered
    /// `{}` (plan 20261027_08, decision 10 and the amendment of 23:24
    /// EDT). Added in 0.4.2.
    pub const TERMINAL_RESIZE: &str = "terminal.resize";
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
    /// Always `flankr`.
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
    /// The agent's turn, from its own session, never herdr's status (plan
    /// 20261023_04, the 17:04 EDT amendment, A4). Absent when the pane has
    /// no session on the daemon's machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<Turn>,
    /// Milliseconds since the session file (or any of its subagents'
    /// files) was last written, on the daemon's clock, as this list was
    /// made (plan 20261028_03, 3.12). Absent without a session on the
    /// daemon's machine. An age, not a time, so the client's clock and the
    /// daemon's need not agree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_ms: Option<u64>,
    /// The subagents of the session that are running: not marked
    /// finished, and written within 60 s. Absent when the agent's reader
    /// does not read subagents (only Claude Code's does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagents_running: Option<u32>,
    /// The first line of the latest Finished card's spoken summary, when
    /// the daemon has one; redacted, at most [`TURN_TEXT_MAX`] characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Where an agent's turn stands, from its own session (plan 20261023_04,
/// A4), the same for every agent's reader. Each text is redacted and cut
/// to [`TURN_TEXT_MAX`] characters by the daemon.
///
/// On the wire it is an object tagged by `state`, as `AgentsSendResult` is
/// by `status`: `{"state": "working", "line": "…"}`,
/// `{"state": "stopped", "line": "…"}` and
/// `{"state": "asking", "question": "…"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Turn {
    /// The turn is going on: `line` is the session's newest line.
    Working { line: String },
    /// The turn ended with no question: `line` is the first line of the
    /// last reply.
    Stopped { line: String },
    /// The turn ended on a question: the pending question's text.
    Asking { question: String },
}

/// The longest text a [`Turn`] carries, in characters.
pub const TURN_TEXT_MAX: usize = 200;

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
    /// The reply under the reading rules: always (plan 20261023_05, the
    /// 16:56 EDT amendment, "both versions in the answer").
    pub read: String,
    /// Gemini's spoken summary: only when the reply is complex, spoken
    /// output is on, and the summary was made. Said instead of `read`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Whether the daemon measured the reply as complex.
    #[serde(default)]
    pub complex: bool,
}

/// `client.options`: a companion's options for its connection (plan
/// 20261023_05, A4). Each starts as its default when the connection opens:
/// off, except `general_agent`. Unknown options are ignored, so more can be
/// added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientOptionsParams {
    /// Read aloud is on: `speech.summarize` may ask Gemini for a summary
    /// of a complex reply.
    #[serde(default)]
    pub spoken_output: bool,
    /// The app is in the background and its link sleeps: the daemon sends
    /// only what needs the user (a confirmation, an agent blocked, asking
    /// or done), and holds the rest until it is off again.
    #[serde(default)]
    pub background: bool,
    /// The general agent (Gemini) is on (plan 20261024_02, decision 7, as
    /// amended at 11:37 EDT). While false, the daemon starts no Gemini turn
    /// for this client; spoken summaries go on as the daemon decides. On
    /// when left out.
    #[serde(default = "general_agent_default")]
    pub general_agent: bool,
    /// Tab follows focus (plan 20261024_02, the amendment of 2026-10-27
    /// 11:27 EDT): when the general agent focuses a project
    /// (`focus_project`, plan 20261027_04), herdr's focused tab moves to
    /// that agent's tab too. Off when left out.
    #[serde(default)]
    pub tab_follows_focus: bool,
}

fn general_agent_default() -> bool {
    true
}

impl Default for ClientOptionsParams {
    fn default() -> Self {
        Self {
            spoken_output: false,
            background: false,
            general_agent: true,
            tab_follows_focus: false,
        }
    }
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
    /// The links in `summary` (plan 20261022_03, the 21:57 EDT amendment):
    /// its `http://` and `https://` URLs, and its `docs/…` paths' repowiki
    /// pages when the daemon has a `[urls] repowiki_url`. Left out of the wire
    /// when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Where an entry that is context, not the user's own words nor the
    /// agent's, comes from: `"task"` (a task, subagent or background
    /// command finished), `"peer:<name>"` (a message from another session),
    /// `"agent:<id>"` (a subagent's hand-back), `"recap"` (the harness's
    /// recap of the session) or `"compact"` (the conversation was
    /// compacted). Such an entry has `role: user` but is never the user
    /// speaking. Left out of the wire when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The entry's full redacted text, up to the reader's limits (32 KiB
    /// for text, 4,000 characters for a tool call or result), where
    /// `summary` is cut (plan 20261022_03, the L3 amendment of 2026-10-24
    /// 11:54 EDT). Left out of the wire when absent, as from an older
    /// daemon: a client then shows `summary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A link in an entry's or question's text: `text` is what is written
/// there (a URL, a Markdown link's text, or a `docs/…` path), `url` where
/// it goes (the URL, or the path's page on repowiki).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub text: String,
    pub url: String,
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
    /// The question is about a plan: the daemon's `plan` rule in
    /// `SUGGESTIONS.md` took it (plan 20261027_06, decision 5), which by
    /// default means it says "approve" with "plan" or "draft" as whole
    /// words. A companion without [`PendingQuestion::answers`] offers
    /// approve, discuss and implement in place of Yes and No.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub about_plan: bool,
    /// The plan the question is about, as its answers name it: "plan 06"
    /// when the question, or the rest of its reply, gives an id after
    /// "plan" or "draft"; else "the draft" or "the plan". Only with
    /// `about_plan`. An older daemon gave a plan's spoken name ("the
    /// overview screen plan").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The links in `text`, as [`Entry::links`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// The answers to offer, in order, their texts filled (plan
    /// 20261027_06, decision 5). Empty from an older daemon: the companion
    /// then offers its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<Answer>,
}

/// One suggested answer to a pending question (plan 20261027_06,
/// decision 3). Exactly one of `sends` and `starts` is given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    /// What the button or text shows.
    pub label: String,
    /// `approve`, `deny`, `green`, `link` or `yellow`, mapped by the
    /// companion to its theme's roles: `approve` and `deny` as buttons,
    /// the others as plain coloured text. A companion draws a style it
    /// does not know as `link`.
    pub style: String,
    /// The text sent when tapped, at most 200 characters, with
    /// `confirmed: true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sends: Option<String>,
    /// The text the message card opens with, for the user to finish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starts: Option<String>,
    /// `sends` is one of the agent's allowed commands, not a message.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub command: bool,
    /// A tap answers the question, so its card goes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub answers: bool,
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
    /// and `-`. Required by `flankr.harness`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// The user approved this in the app already, by tapping a button or
    /// submitting reviewed text (plan 20261023_02, the 2026-10-27 13:25 EDT
    /// amendment): a companion that may approve sends it without a
    /// confirmation. Ignored from any other client. Off when left out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirmed: bool,
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

/// `ping`'s params: whether the app is in front. A companion pings with
/// `active: false` while its link sleeps in the background, and with
/// `active: true` the moment the user resumes the app; absent, the ping
/// says nothing about it (owner, 2026-10-23: "ping should have active flag
/// ... and send it when user resumes app").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PingParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
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

/// The identity and place of a section of an agent's session (plan
/// 20261023_03, the 14:54 EDT amendment, A2): derived from the session
/// file, never counted, so the same line gives the same serial on every
/// read, for every client, and after a daemon restart.
///
/// * `file`: the 32-bit FNV-1a hash of the session file's name (not its
///   path), [`Serial::file_hash`];
/// * `line`: the 1-based number of the JSONL line the section starts on,
///   counting every line; at most [`SERIAL_LINE_MAX`];
/// * `slot`: the section's place among those starting on that line.
///
/// On the wire it is a string with the hash as its prefix,
/// `"3fa9c21e:412.2"`: a packed u64 would pass 2^53, which many JSON
/// readers cannot hold exactly. In memory it packs into a u64
/// ([`Serial::to_u64`]) that orders as the serials of one file do.
///
/// Serials of two files have no order: `partial_cmp` gives `None`, and a
/// different prefix means a different session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Serial {
    pub file: u32,
    pub line: u32,
    pub slot: u8,
}

/// The largest `Serial::line`: it packs into 24 bits.
pub const SERIAL_LINE_MAX: u32 = (1 << 24) - 1;

/// A serial string that does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSerialError(pub String);

impl std::fmt::Display for ParseSerialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad serial: {}", self.0)
    }
}

impl std::error::Error for ParseSerialError {}

impl Serial {
    pub fn new(file: u32, line: u32, slot: u8) -> Self {
        Serial { file, line, slot }
    }

    /// The 32-bit FNV-1a hash of a session file's name, such as
    /// `992c3f44-18e7-4cf8-814f-e437b90209b6.jsonl`.
    pub fn file_hash(name: &str) -> u32 {
        let mut hash: u32 = 0x811c_9dc5;
        for b in name.bytes() {
            hash ^= u32::from(b);
            hash = hash.wrapping_mul(0x0100_0193);
        }
        hash
    }

    /// The packed form: `file` in bits 63–32, `line` in 31–8, `slot` in
    /// 7–0. A `line` past [`SERIAL_LINE_MAX`] does not fit and is cut to
    /// its low 24 bits.
    pub fn to_u64(self) -> u64 {
        (u64::from(self.file) << 32)
            | (u64::from(self.line & SERIAL_LINE_MAX) << 8)
            | u64::from(self.slot)
    }

    pub fn from_u64(v: u64) -> Self {
        Serial {
            file: (v >> 32) as u32,
            line: ((v >> 8) as u32) & SERIAL_LINE_MAX,
            slot: v as u8,
        }
    }

    /// Whether both are of the same session file.
    pub fn same_file(self, other: Serial) -> bool {
        self.file == other.file
    }
}

impl PartialOrd for Serial {
    /// `None` across files; within one, by line, then slot.
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.same_file(*other)
            .then(|| (self.line, self.slot).cmp(&(other.line, other.slot)))
    }
}

impl std::fmt::Display for Serial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:08x}:{}.{}", self.file, self.line, self.slot)
    }
}

impl std::str::FromStr for Serial {
    type Err = ParseSerialError;

    /// `"3fa9c21e:412.2"`: 8 hex digits, then the line and the slot in
    /// decimal.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || ParseSerialError(s.to_string());
        let (file, rest) = s.split_once(':').ok_or_else(bad)?;
        let (line, slot) = rest.split_once('.').ok_or_else(bad)?;
        let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        if file.len() != 8 || !file.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        if !digits(line) || !digits(slot) {
            return Err(bad());
        }
        let file = u32::from_str_radix(file, 16).map_err(|_| bad())?;
        let line: u32 = line.parse().map_err(|_| bad())?;
        let slot: u8 = slot.parse().map_err(|_| bad())?;
        if line > SERIAL_LINE_MAX {
            return Err(ParseSerialError(format!(
                "{s}: line {line} is past {SERIAL_LINE_MAX}"
            )));
        }
        Ok(Serial { file, line, slot })
    }
}

impl Serialize for Serial {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Serial {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = std::borrow::Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// What a [`Section`] is (A1, and the 22:55 EDT amendment's `context`).
/// Open: a kind this crate does not know reads as `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SectionKind {
    /// A user prompt: the start of a turn.
    Turn,
    /// A block of the agent's output: consecutive assistant text entries
    /// with no tool call between them.
    Text,
    /// A tool call and, once it arrives, its result.
    Tool,
    /// A priority card.
    Card,
    /// A context entry (one with an `origin`): never part of a turn and
    /// never a card.
    Context,
    /// A kind from a newer daemon.
    #[serde(other)]
    Unknown,
}

/// A card's kind, highest priority first (the order compares so). Open:
/// a kind this crate does not know reads as `Unknown`, ordered last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CardKind {
    /// A flankr confirmation for the pane.
    Approval,
    /// The agent asked a question.
    Question,
    /// An error result, failing tests, or the pane blocked.
    Broke,
    /// The agent finished a turn with a reply.
    Finished,
    #[serde(other)]
    Unknown,
}

/// A card section's body (A3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardBody {
    pub card: CardKind,
    /// Live until resolved; a resolved card is shown faded.
    pub live: bool,
    /// "Approval needed", "<name> is asking", "Something broke",
    /// "Finished".
    pub title: String,
    pub text: String,
    /// The confirmation an Approval card answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_id: Option<u64>,
    /// A Question card's question is about a plan (see
    /// `PendingQuestion::about_plan`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub about_plan: bool,
    /// The links in `text`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// A point-by-point summary of the reply, at most `CARD_POINTS_MAX`
    /// short lines, filled by the daemon's `[summary] cards` (plan
    /// 20261022_03, the L3 amendment of 2026-10-24 12:06 EDT). Empty, and
    /// left out of the wire, until then or without Gemini.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<String>,
    /// The text to read aloud for the card, filled with `points`. Left out
    /// of the wire when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
}

/// The most `CardBody::points`.
pub const CARD_POINTS_MAX: usize = 6;

/// One unit of the agent view (A1, A3): sections are flat and ordered by
/// serial. A section that changes (a text block that grows, a tool that
/// gets its result, a card resolved) is sent again with the same serial,
/// and replaces the one held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub serial: Serial,
    pub kind: SectionKind,
    /// Redacted, oldest first; empty for a card.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// For kind `card`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<CardBody>,
    /// The section's place among its file's sections, from 0 with no
    /// gaps (plan 20261024_01, decision 1). Set in outline mode and in
    /// `session.content`'s answers; 0, and left out of the wire, otherwise.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub index: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// One section in a session's outline (plan 20261024_01, decision 2):
/// its place, identity and size, without its contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Slot {
    pub index: u64,
    pub serial: Serial,
    pub kind: SectionKind,
    /// The content's size: its lines (after the readers' line cuts) and
    /// characters, for the client's height estimate.
    pub lines: u32,
    pub chars: u32,
    /// Bumped each time the section is replaced, from 0.
    #[serde(default)]
    pub rev: u32,
    /// A Finished card's slot: a bookend, which closes a section of the
    /// session, for the scrollbar's section labels (plan 20261022_03, the
    /// L3 amendment of 2026-10-24 12:27 EDT). False, and left out of the
    /// wire, otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bookend: bool,
}

/// The most slots one outline delivery carries.
pub const SLOTS_MAX: usize = 2000;
/// The newest indexes whose contents ride along with outline deliveries.
pub const LIVE_INLINE: u64 = 8;
/// Sections `session.content` returns by default.
pub const CONTENT_LIMIT_DEFAULT: usize = 50;

fn content_limit_default() -> usize {
    CONTENT_LIMIT_DEFAULT
}

/// `session.content` (plan 20261024_01, decision 4, as amended at 00:03
/// EDT): one inclusive index range of the subscription's current file.
///
/// * `from` only: the next `limit` sections from `from` on;
/// * `to` only: the `limit` sections ending at `to`, oldest first;
/// * both: the range, cut at `limit` from `from`;
/// * neither: the newest `limit` sections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContentParams {
    pub subscription_id: u64,
    /// Inclusive bounds; `None` is open on that side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<u64>,
    /// 1 to `SECTIONS_MAX`.
    #[serde(default = "content_limit_default")]
    pub limit: usize,
    /// Slots only, no contents (to fill a gap in the outline).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub slots_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContentResult {
    /// In index order, each carrying its index; empty when `slots_only`.
    #[serde(default)]
    pub sections: Vec<Section>,
    /// When `slots_only`, in index order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<Slot>,
    /// Indexes in the range walked that were removed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_index: Vec<u64>,
    /// The session's lowest and highest index; `last` is null for no
    /// sections.
    #[serde(default)]
    pub first: u64,
    #[serde(default)]
    pub last: Option<u64>,
    /// The range held more than `limit`: ask again past the last one
    /// (before the first one, for `to` only or neither bound).
    #[serde(default)]
    pub more: bool,
}

/// Sections a subscription's backfill holds by default, and the most a
/// backfill, a history page or one delivery may hold.
pub const SUBSCRIBE_SECTIONS_DEFAULT: usize = 50;
pub const SECTIONS_MAX: usize = 200;
/// Sections a history page holds by default.
pub const HISTORY_SECTIONS_DEFAULT: usize = 100;
/// Subscriptions one connection may hold.
pub const SUBSCRIPTIONS_MAX: usize = 8;

fn subscribe_sections_default() -> usize {
    SUBSCRIBE_SECTIONS_DEFAULT
}

fn history_sections_default() -> usize {
    HISTORY_SECTIONS_DEFAULT
}

/// `session.subscribe` (4.1, as amended by A5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSubscribeParams {
    /// A pane id or a name, resolved once, as `session.peek` resolves it.
    pub pane_id: String,
    /// The backfill's size, 1 to `SECTIONS_MAX`.
    #[serde(default = "subscribe_sections_default")]
    pub sections: usize,
    /// The newest serial the client holds: when it is of the pane's
    /// current session file, the backfill is only what follows it;
    /// otherwise the first delivery is a reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Serial>,
    /// Outline mode (plan 20261024_01, decision 3): deliveries carry
    /// slots for the whole session, and contents only for the newest
    /// `LIVE_INLINE` indexes; the rest come by `session.content`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outline: bool,
    /// In outline mode, the index the outline starts at: the client's
    /// newest held index plus one, to resume. It is honoured only when
    /// `after` is of the pane's current file; otherwise the outline
    /// starts at 0 with a reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<u64>,
    /// In outline mode, the epoch of the outline the client holds (as
    /// its last delivery gave it). `from` is honoured only when it is
    /// the daemon's current one; otherwise the outline starts again at
    /// 0 with a reset, which drops only the outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSubscribeResult {
    pub subscription_id: u64,
    /// The first delivery.
    pub delivery: SessionDelivery,
}

/// The session a delivery is from, as `session.peek` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub matched: SessionMatched,
}

/// Why `SessionDelivery::session` is null.
pub mod delivery_reasons {
    /// An agent without a session reader.
    pub const NO_READER: &str = "no_reader";
    /// No session file was found.
    pub const NO_SESSION: &str = "no_session";
    /// The principal may not reach the pane's agent.
    pub const NOT_PERMITTED: &str = "not_permitted";
    /// The pane is gone: no more deliveries follow.
    pub const PANE_CLOSED: &str = "pane_closed";
    /// The session file is longer than serials can number.
    pub const TOO_LONG: &str = "too_long";
}

/// `session.delivery`, and the first delivery in `session.subscribe`'s
/// result (4.2, as amended by A5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDelivery {
    pub subscription_id: u64,
    /// The subscription's pane.
    pub pane_id: String,
    /// The pane's agent.
    pub agent: String,
    /// Drop what was held for this subscription: `sections` is a fresh
    /// backfill.
    #[serde(default)]
    pub reset: bool,
    /// Null when the pane has no session, with `reason` saying why.
    pub session: Option<SessionRef>,
    /// One of [`delivery_reasons`], or null.
    pub reason: Option<String>,
    /// New sections, above the newest serial already delivered, and
    /// changed ones, at or below it; in serial order.
    #[serde(default)]
    pub sections: Vec<Section>,
    /// herdr's status: present when it changed since the last delivery,
    /// or on a reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Absent when unchanged since the last delivery; null when it was
    /// cleared.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub pending_question: Option<Option<PendingQuestion>>,
    /// Sections held that no longer stand (a card the session went past,
    /// such as Finished when the agent carried on), to drop. Left out of
    /// the wire when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<Serial>,
    /// Outline mode: new and replaced slots, in index order, at most
    /// `SLOTS_MAX`; a reset's outline continues over the next deliveries
    /// until it reaches `last` (plan 20261024_01, decision 3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slots: Vec<Slot>,
    /// Outline mode: the indexes of sections that no longer stand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_index: Vec<u64>,
    /// Outline mode: the session's lowest and highest index; `last` is
    /// absent for no sections, and both are left out otherwise.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub first: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<u64>,
    /// Outline mode: the epoch the indexes are of. It changes when the
    /// daemon restarts or the session file is rewritten, and the outline
    /// is then sent again from 0 with `reset`. In outline mode, `reset`
    /// drops only the outline: contents stay valid by serial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
}

/// An `Option<Option<T>>` field: absent is `None`, null is `Some(None)`.
mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(
        value: &Option<Option<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

/// `session.unsubscribe` (4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUnsubscribeParams {
    pub subscription_id: u64,
}

/// `session.history` (A5): the sections just below `before`, from the
/// subscription's current session file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryParams {
    pub subscription_id: u64,
    pub before: Serial,
    /// 1 to `SECTIONS_MAX`.
    #[serde(default = "history_sections_default")]
    pub sections: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHistoryResult {
    /// Oldest first.
    pub sections: Vec<Section>,
    /// The page reaches the file's first line.
    #[serde(default)]
    pub start: bool,
}

/// `link.pair_begin` takes no parameters; it serialises as `{}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LinkPairBeginParams {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPairBeginResult {
    /// The pairing URL, `flankr://<host>:<port>/pair#flankr1…`, also shown
    /// as the QR code.
    pub url: String,
    /// The pairing string, `flankr1…`: the server's key and addresses.
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
    /// The device code, `flankrkey1…`, made for this server's key.
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

/// `link.unpair` takes no parameters; it serialises as `{}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LinkUnpairParams {}

/// `link.unpair`'s result, `{}`: the device is forgotten, and the link
/// closes after it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LinkUnpairResult {}

/// `link.pair_done`: the device added by `link.pair_add` said `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPairDone {
    pub name: String,
    pub paired: bool,
    /// Why it did not pair, if it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `terminal.open` (plan 20261027_07, decision 2): mirror the pane of a
/// session on this connection. One terminal per session per connection;
/// a second `open` returns the same id. Refused with the codes
/// `agents.read` uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOpenParams {
    /// A pane id or a name, resolved as `session.peek` resolves it; also
    /// read as `session`.
    #[serde(alias = "session")]
    pub pane_id: String,
    /// The columns the phone could show. A hint only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols_hint: Option<u16>,
    /// The rows the phone could show. A hint only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_hint: Option<u16>,
    /// The phone reads libflankr_term 0.1.1's cursor trailer, so the
    /// daemon may write it (plan 20261027_08, decision 10). Absent (false)
    /// from older phones, whose frames never carry it. Added in 0.4.2.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cursor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalOpenResult {
    /// Names the terminal in the other `terminal.*` calls and in its
    /// frames.
    pub terminal_id: u32,
    /// The pane's size as the daemon reads it (today the longest row and
    /// the row count of the visible frame).
    pub cols: u16,
    pub rows: u16,
    /// This terminal's frames carry the cursor trailer (only when the
    /// phone asked, and only while the frames come from a source that
    /// knows the cursor). Added in 0.4.2.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cursor: bool,
    /// Where the daemon reads the terminal from. An older daemon sends
    /// none: [`TerminalSource::Poll`]. Added in 0.4.2.
    #[serde(default)]
    pub source: TerminalSource,
}

/// Where a terminal's frames come from (plan 20261027_08, decisions 10
/// and 11). Open: a source this crate does not know reads as `Unknown`.
/// Added in 0.4.2.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalSource {
    /// herdr's `pane.read` poll (plan 20261027_07).
    #[default]
    Poll,
    /// The daemon's attachment to herdr as a client: the surface, with
    /// the cursor, and mouse input.
    Attached,
    /// A source from a newer daemon.
    #[serde(other)]
    Unknown,
}

/// `terminal.close`, answered with [`Empty`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCloseParams {
    pub terminal_id: u32,
}

/// `terminal.ack`, a notification from the phone: `seq` is the newest
/// frame it holds, which the daemon diffs from thereafter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalAckParams {
    pub terminal_id: u32,
    pub seq: u32,
}

/// How long one `terminal.input_arm` lets `terminal.input` through, in
/// seconds (plan 20261027_07, decision 6).
pub const TERMINAL_INPUT_ARM_SECS: u32 = 60;

/// `terminal.input_arm`: arm keyboard input on a terminal opened on this
/// connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInputArmParams {
    pub terminal_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInputArmResult {
    /// Seconds from the daemon's answer until input is refused again
    /// unless armed anew: [`TERMINAL_INPUT_ARM_SECS`] today. Relative, so
    /// the phone's clock does not matter.
    pub expires_in_s: u32,
}

/// The most bytes of `keys` in one `terminal.input`.
pub const TERMINAL_INPUT_MAX: usize = 64;

/// `terminal.input`, answered with [`Empty`]: keys into an armed terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInputParams {
    pub terminal_id: u32,
    /// herdr's `send_keys` syntax, at most [`TERMINAL_INPUT_MAX`] bytes.
    pub keys: String,
}

/// `terminal.scroll`: scroll an armed terminal's pane by `lines`, as a
/// mouse wheel does: positive goes back into the scrollback, negative
/// towards the bottom. The daemon sets herdr's `pane.scroll` offset, so the
/// pane scrolls wherever it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalScrollParams {
    pub terminal_id: u32,
    pub lines: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalScrollResult {
    /// The pane's scroll offset afterwards, in rows above the bottom.
    pub offset_from_bottom: u64,
    /// The furthest it can go back, when herdr reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_offset_from_bottom: Option<u64>,
}

/// Why a terminal ended. Open: a reason this crate does not know reads as
/// `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalClosedReason {
    /// The pane closed.
    PaneClosed,
    /// The pane's agent changed.
    AgentChanged,
    /// Keyboard input was revoked.
    InputRevoked,
    /// The daemon closed it for another reason, such as shutting down.
    Closed,
    /// A reason from a newer daemon.
    #[serde(other)]
    Unknown,
}

/// `terminal.closed`, a notification from the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalClosedParams {
    pub terminal_id: u32,
    pub reason: TerminalClosedReason,
}

/// `terminal.mouse`, answered with [`Empty`]: one mouse event into an
/// armed terminal the daemon reads from herdr's attachment
/// ([`TerminalSource::Attached`]); on a polled terminal it is refused
/// (plan 20261027_08, decisions 10 and 12). Added in 0.4.2.
///
/// `col` and `row` are cells of the pane, from 0 at its top left, as herdr
/// expects. `modifiers` is the `modifiers` byte of herdr's
/// `ClientPaneInputEvent`, passed through as herdr numbers its bits; 0
/// (none) when absent. The kind is
/// flattened into the object, its button beside it:
///
/// ```json
/// {"terminal_id": 1, "kind": "down", "button": "left", "col": 4, "row": 2}
/// {"terminal_id": 1, "kind": "moved", "col": 5, "row": 2, "modifiers": 4}
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalMouseParams {
    pub terminal_id: u32,
    #[serde(flatten)]
    pub kind: TerminalMouseKind,
    pub col: u16,
    pub row: u16,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub modifiers: u8,
}

fn is_zero_u8(n: &u8) -> bool {
    *n == 0
}

/// A [`TerminalMouseParams`] event, tagged by `kind` (snake case), a
/// button's event with its `button` beside it. Open: a kind this crate
/// does not know reads as `Unknown`, which the daemon ignores. Added in
/// 0.4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalMouseKind {
    /// A button pressed.
    Down { button: MouseButton },
    /// A button released.
    Up { button: MouseButton },
    /// The pointer moved with a button held.
    Drag { button: MouseButton },
    /// The pointer moved with no button held.
    Moved,
    /// A kind from a newer phone.
    #[serde(other)]
    Unknown,
}

/// A mouse button, snake case on the wire. Added in 0.4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// `terminal.resize`, answered with [`Empty`]: the columns and rows that
/// fit the phone's terminal area now, keyboard included (plan 20261027_08,
/// the amendment of 23:24 EDT). The daemon clamps them and applies them
/// to an attached terminal; a polled one cannot be sized. Added in 0.4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalResizeParams {
    pub terminal_id: u32,
    pub cols: u16,
    pub rows: u16,
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
    fn serials_round_trip_through_the_string_the_u64_and_serde() {
        let s = Serial::new(0x3fa9_c21e, 412, 2);
        assert_eq!(s.to_string(), "3fa9c21e:412.2");
        assert_eq!("3fa9c21e:412.2".parse::<Serial>().unwrap(), s);
        assert_eq!(Serial::from_u64(s.to_u64()), s);
        assert_eq!(s.to_u64(), 0x3fa9_c21e_0001_9c02);
        assert_eq!(serde_json::to_value(s).unwrap(), json!("3fa9c21e:412.2"));
        assert_eq!(
            serde_json::from_value::<Serial>(json!("3fa9c21e:412.2")).unwrap(),
            s
        );
        // The extremes.
        for s in [
            Serial::new(0, 0, 0),
            Serial::new(u32::MAX, SERIAL_LINE_MAX, 255),
            Serial::new(1, 1, 0),
        ] {
            assert_eq!(s.to_string().parse::<Serial>().unwrap(), s);
            assert_eq!(Serial::from_u64(s.to_u64()), s);
        }
        assert_eq!(Serial::new(7, 3, 1).to_string(), "00000007:3.1");
    }

    #[test]
    fn serials_order_within_a_file_and_not_across() {
        use std::cmp::Ordering;
        let a = Serial::new(9, 412, 2);
        let b = Serial::new(9, 412, 3);
        let c = Serial::new(9, 413, 0);
        assert!(a < b && b < c && a < c);
        assert_eq!(a.partial_cmp(&a), Some(Ordering::Equal));
        // The packed form orders the same within a file.
        assert!(a.to_u64() < b.to_u64() && b.to_u64() < c.to_u64());
        let other = Serial::new(10, 1, 0);
        assert_eq!(a.partial_cmp(&other), None);
        assert!(
            a.lt(&other) == a.ge(&other) && a != other,
            "neither below nor above"
        );
        assert!(a.same_file(c) && !a.same_file(other));
    }

    #[test]
    fn serials_keep_the_24_bit_line_limit_and_refuse_bad_text() {
        let max = format!("00000001:{SERIAL_LINE_MAX}.0");
        assert_eq!(max.parse::<Serial>().unwrap().line, SERIAL_LINE_MAX);
        let past = format!("00000001:{}.0", SERIAL_LINE_MAX + 1);
        assert!(past.parse::<Serial>().is_err());
        for bad in [
            "",
            "3fa9c21e",
            "3fa9c21e:412",
            "3fa9c21e:412.",
            "3fa9c21:412.2",
            "3fa9c21e0:412.2",
            "3fa9c21g:412.2",
            "3fa9c21e:-1.2",
            "3fa9c21e:412.256",
            "3fa9c21e:41 2.2",
            "3fa9c21e:+412.2",
        ] {
            assert!(bad.parse::<Serial>().is_err(), "{bad:?} parsed");
        }
        assert!(serde_json::from_value::<Serial>(json!(17)).is_err());
        assert!(serde_json::from_value::<Serial>(json!("x")).is_err());
    }

    #[test]
    fn the_file_hash_is_fnv_1a() {
        // FNV-1a 32 of "" and "a", the published test values.
        assert_eq!(Serial::file_hash(""), 0x811c_9dc5);
        assert_eq!(Serial::file_hash("a"), 0xe40c_292c);
        assert_eq!(Serial::file_hash("foobar"), 0xbf9c_f968);
    }

    fn section(serial: Serial, kind: SectionKind) -> Section {
        Section {
            serial,
            kind,
            entries: vec![Entry {
                ts: "t".into(),
                role: Role::Assistant,
                kind: EntryKind::Text,
                tool: None,
                summary: "Done.".into(),
                links: vec![],
                origin: None,
                text: None,
            }],
            card: None,
            index: 0,
        }
    }

    #[test]
    fn the_subscription_types_round_trip() {
        let s = Serial::new(0x3fa9_c21e, 412, 0);
        round_trip(SessionSubscribeParams {
            pane_id: "w2:pZ".into(),
            sections: 200,
            after: Some(s),
            outline: false,
            from: None,
            epoch: None,
        });
        let p: SessionSubscribeParams =
            serde_json::from_value(json!({"pane_id": "w2:pZ"})).unwrap();
        assert_eq!((p.sections, p.after), (SUBSCRIBE_SECTIONS_DEFAULT, None));
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({"pane_id": "w2:pZ", "sections": 50})
        );
        let card = Section {
            serial: Serial::new(0x3fa9_c21e, 412, 192),
            kind: SectionKind::Card,
            entries: vec![],
            card: Some(CardBody {
                card: CardKind::Question,
                live: true,
                title: "Claude is asking".into(),
                text: "Approve docs/design/X_PLAN.md?".into(),
                confirm_id: None,
                about_plan: true,
                links: vec![Link {
                    text: "docs/design/X_PLAN.md".into(),
                    url: "u".into(),
                }],
                points: vec![],
                spoken: None,
            }),
            index: 0,
        };
        let full = SessionDelivery {
            subscription_id: 3,
            pane_id: "w2:pZ".into(),
            agent: "claude".into(),
            reset: true,
            session: Some(SessionRef {
                session_id: Some("992c".into()),
                matched: SessionMatched::Id,
            }),
            reason: None,
            sections: vec![section(s, SectionKind::Text), card],
            status: Some("idle".into()),
            pending_question: Some(Some(PendingQuestion {
                text: "Go on?".into(),
                ts: "t".into(),
                about_plan: false,
                plan: None,
                links: vec![],
                answers: vec![],
            })),
            removed: vec![Serial::new(0x3fa9_c21e, 400, 195)],
            slots: vec![],
            removed_index: vec![],
            first: 0,
            last: None,
            epoch: None,
        };
        round_trip(full.clone());
        round_trip(SessionSubscribeResult {
            subscription_id: 3,
            delivery: full.clone(),
        });
        // Every optional field absent: status and pending_question are
        // left out; session and reason are null.
        let bare = SessionDelivery {
            reset: false,
            session: None,
            reason: Some(delivery_reasons::NO_READER.into()),
            sections: vec![],
            status: None,
            pending_question: None,
            removed: vec![],
            ..full.clone()
        };
        round_trip(bare.clone());
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({"subscription_id": 3, "pane_id": "w2:pZ", "agent": "claude",
                   "reset": false, "session": null, "reason": "no_reader",
                   "sections": []})
        );
        // A cleared question is null, and reads back as cleared.
        let cleared = SessionDelivery {
            pending_question: Some(None),
            ..bare.clone()
        };
        round_trip(cleared.clone());
        assert_eq!(
            serde_json::to_value(&cleared).unwrap()["pending_question"],
            serde_json::Value::Null
        );
        let read: SessionDelivery = serde_json::from_value(json!({
            "subscription_id": 3, "pane_id": "w2:pZ", "agent": "claude",
            "session": null, "reason": null, "pending_question": null
        }))
        .unwrap();
        assert_eq!(read.pending_question, Some(None));
        assert!(!read.reset && read.sections.is_empty() && read.removed.is_empty());
        round_trip(SessionUnsubscribeParams { subscription_id: 3 });
        round_trip(SessionHistoryParams {
            subscription_id: 3,
            before: s,
            sections: 20,
        });
        let h: SessionHistoryParams =
            serde_json::from_value(json!({"subscription_id": 3, "before": "3fa9c21e:412.0"}))
                .unwrap();
        assert_eq!(h.sections, HISTORY_SECTIONS_DEFAULT);
        assert!(
            serde_json::from_value::<SessionHistoryParams>(
                json!({"subscription_id": 3, "before": "nope"})
            )
            .is_err()
        );
        round_trip(SessionHistoryResult {
            sections: vec![section(s, SectionKind::Turn)],
            start: true,
        });
    }

    #[test]
    fn the_outline_types_round_trip_and_old_messages_read_as_before() {
        let s = Serial::new(0x3fa9_c21e, 412, 0);
        let slot = Slot {
            index: 7,
            serial: s,
            kind: SectionKind::Tool,
            lines: 12,
            chars: 480,
            rev: 2,
            bookend: false,
        };
        round_trip(slot);
        assert_eq!(
            serde_json::to_value(slot).unwrap(),
            json!({"index": 7, "serial": "3fa9c21e:412.0", "kind": "tool",
                   "lines": 12, "chars": 480, "rev": 2})
        );
        // A section with its index carries it; one without reads as 0.
        let mut indexed = section(s, SectionKind::Text);
        indexed.index = 7;
        round_trip(indexed.clone());
        assert_eq!(serde_json::to_value(&indexed).unwrap()["index"], 7);
        let plain = section(s, SectionKind::Text);
        assert!(serde_json::to_value(&plain).unwrap().get("index").is_none());
        // Outline subscribe params.
        let p = SessionSubscribeParams {
            pane_id: "w2:pZ".into(),
            sections: 50,
            after: Some(s),
            outline: true,
            from: Some(8),
            epoch: Some(0x5eed),
        };
        round_trip(p.clone());
        assert_eq!(serde_json::to_value(&p).unwrap()["from"], 8);
        let old: SessionSubscribeParams =
            serde_json::from_value(json!({"pane_id": "w2:pZ", "after": "3fa9c21e:412.0"})).unwrap();
        assert!(!old.outline && old.from.is_none() && old.epoch.is_none());
        // An outline delivery.
        let d = SessionDelivery {
            subscription_id: 3,
            pane_id: "w2:pZ".into(),
            agent: "claude".into(),
            reset: true,
            session: None,
            reason: None,
            sections: vec![indexed.clone()],
            status: None,
            pending_question: None,
            removed: vec![],
            slots: vec![slot],
            removed_index: vec![4, 5],
            first: 0,
            last: Some(7),
            epoch: Some(0x5eed),
        };
        round_trip(d.clone());
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["removed_index"], json!([4, 5]));
        assert_eq!(v["last"], 7);
        // An old delivery has none of it.
        let old: SessionDelivery = serde_json::from_value(json!({
            "subscription_id": 3, "pane_id": "w2:pZ", "agent": "claude",
            "session": null, "reason": null
        }))
        .unwrap();
        assert!(old.slots.is_empty() && old.removed_index.is_empty());
        assert_eq!((old.first, old.last), (0, None));
        // session.content.
        let c = SessionContentParams {
            subscription_id: 3,
            from: Some(10),
            to: None,
            limit: 200,
            slots_only: true,
        };
        round_trip(c.clone());
        let bare: SessionContentParams =
            serde_json::from_value(json!({"subscription_id": 3})).unwrap();
        assert_eq!(
            bare,
            SessionContentParams {
                subscription_id: 3,
                from: None,
                to: None,
                limit: CONTENT_LIMIT_DEFAULT,
                slots_only: false,
            }
        );
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            json!({"subscription_id": 3, "limit": 50})
        );
        round_trip(SessionContentResult {
            sections: vec![indexed],
            slots: vec![slot],
            removed_index: vec![6],
            first: 0,
            last: Some(9),
            more: true,
        });
        let r: SessionContentResult = serde_json::from_value(json!({})).unwrap();
        assert!(r.sections.is_empty() && !r.more && r.last.is_none());
        assert_eq!(methods::SESSION_CONTENT, "session.content");
        assert!(LIVE_INLINE as usize <= SECTIONS_MAX && SECTIONS_MAX <= SLOTS_MAX);
    }

    #[test]
    fn section_and_card_kinds_are_open() {
        for (kind, wire) in [
            (SectionKind::Turn, "turn"),
            (SectionKind::Text, "text"),
            (SectionKind::Tool, "tool"),
            (SectionKind::Card, "card"),
            (SectionKind::Context, "context"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<SectionKind>(json!(wire)).unwrap(),
                kind
            );
        }
        assert_eq!(
            serde_json::from_value::<SectionKind>(json!("diagram")).unwrap(),
            SectionKind::Unknown
        );
        assert_eq!(
            serde_json::from_value::<CardKind>(json!("celebration")).unwrap(),
            CardKind::Unknown
        );
        assert!(CardKind::Approval < CardKind::Question);
        assert!(CardKind::Broke < CardKind::Finished && CardKind::Finished < CardKind::Unknown);
        // A text section without entries or card leaves both out.
        let empty = Section {
            serial: Serial::new(1, 2, 3),
            kind: SectionKind::Text,
            entries: vec![],
            card: None,
            index: 0,
        };
        assert_eq!(
            serde_json::to_value(&empty).unwrap(),
            json!({"serial": "00000001:2.3", "kind": "text"})
        );
    }

    #[test]
    fn entry_origin_is_left_out_when_absent_and_read_when_missing() {
        let entry = Entry {
            ts: "t".into(),
            role: Role::User,
            kind: EntryKind::Text,
            tool: None,
            summary: "hi".into(),
            links: vec![],
            origin: None,
            text: None,
        };
        let text = serde_json::to_string(&entry).unwrap();
        assert!(!text.contains("origin"), "{text}");
        let old: Entry = serde_json::from_value(
            json!({"ts": "t", "role": "user", "kind": "text", "summary": "hi"}),
        )
        .unwrap();
        assert_eq!(old, entry);
        let peer: Entry = serde_json::from_value(json!({
            "ts": "t", "role": "user", "kind": "text", "summary": "hi",
            "origin": "peer:alpha"
        }))
        .unwrap();
        assert_eq!(peer.origin.as_deref(), Some("peer:alpha"));
    }

    #[test]
    fn session_entries_round_trip() {
        round_trip(Entry {
            ts: "2026-10-23T06:00:00.000Z".into(),
            role: Role::Assistant,
            kind: EntryKind::ToolUse,
            tool: Some("Bash".into()),
            summary: "Bash {\"command\":\"ls\"}".into(),
            links: vec![],
            origin: None,
            text: None,
        });
        round_trip(Entry {
            ts: "t".into(),
            role: Role::User,
            kind: EntryKind::Text,
            tool: None,
            summary: "Build finished".into(),
            links: vec![],
            origin: Some("task".into()),
            text: None,
        });
        round_trip(Entry {
            ts: "t".into(),
            role: Role::Assistant,
            kind: EntryKind::Text,
            tool: None,
            summary: "See docs/design/X_PLAN.md".into(),
            links: vec![Link {
                text: "docs/design/X_PLAN.md".into(),
                url: "http://wiki/#/src--flankr/docs/design/X_PLAN".into(),
            }],
            origin: None,
            text: None,
        });
        round_trip(PendingQuestion {
            text: "Shall I go on?".into(),
            ts: "2026-10-23T06:00:00.000Z".into(),
            about_plan: false,
            plan: None,
            links: vec![],
            answers: vec![],
        });
        round_trip(PendingQuestion {
            text: "Approve docs/design/X_PLAN.md?".into(),
            ts: "t".into(),
            about_plan: true,
            plan: Some("the x plan".into()),
            links: vec![Link {
                text: "docs/design/X_PLAN.md".into(),
                url: "u".into(),
            }],
            answers: vec![],
        });
        // With answers (plan 20261027_06, step 2).
        let with_answers = PendingQuestion {
            text: "Approve plan 04?".into(),
            ts: "t".into(),
            about_plan: true,
            plan: Some("plan 04".into()),
            links: vec![],
            answers: vec![
                Answer {
                    label: "approve".into(),
                    style: "green".into(),
                    sends: Some("Approve plan 04".into()),
                    starts: None,
                    command: false,
                    answers: true,
                },
                Answer {
                    label: "discuss".into(),
                    style: "link".into(),
                    sends: None,
                    starts: Some("Discuss plan 04:".into()),
                    command: false,
                    answers: false,
                },
                Answer {
                    label: "cont".into(),
                    style: "yellow".into(),
                    sends: Some("continue".into()),
                    starts: None,
                    command: true,
                    answers: false,
                },
            ],
        };
        round_trip(with_answers.clone());
        assert_eq!(
            serde_json::to_value(&with_answers.answers).unwrap(),
            json!([
                {"label": "approve", "style": "green", "sends": "Approve plan 04", "answers": true},
                {"label": "discuss", "style": "link", "starts": "Discuss plan 04:"},
                {"label": "cont", "style": "yellow", "sends": "continue", "command": true}
            ])
        );
        // Old JSON reads, and empty new fields stay off the wire.
        let q: PendingQuestion =
            serde_json::from_value(json!({"text": "Go on?", "ts": "t"})).unwrap();
        assert!(!q.about_plan && q.plan.is_none() && q.links.is_empty());
        assert!(q.answers.is_empty());
        assert_eq!(
            serde_json::to_value(&q).unwrap(),
            json!({"text": "Go on?", "ts": "t"})
        );
        assert_eq!(
            serde_json::to_value(EntryKind::ToolResult).unwrap(),
            json!("tool_result")
        );
        let text: Entry = serde_json::from_value(json!({
            "ts": "t", "role": "user", "kind": "text", "summary": "hi"
        }))
        .unwrap();
        assert_eq!(text.tool, None);
        assert!(text.links.is_empty());
        assert_eq!(
            serde_json::to_value(&text).unwrap(),
            json!({"ts": "t", "role": "user", "kind": "text", "summary": "hi"})
        );
    }

    #[test]
    fn every_type_round_trips() {
        round_trip(HelloParams {
            protocol: protocols::HARNESS.to_string(),
            version: 1,
            client: "ctl".to_string(),
        });
        round_trip(HelloResult {
            server: "flankr".to_string(),
            server_version: "0.1.0".to_string(),
            protocol_version: 1,
        });
        round_trip(AgentsListResult {
            agents: vec![AgentInfo {
                pane_id: "w2:pZ".to_string(),
                name: Some("the flankr agent".to_string()),
                spoken: Some("the flankr agent".to_string()),
                agent: "claude".to_string(),
                project: Some("flankr".to_string()),
                cwd: Some("/src/flankr".to_string()),
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
                        about_plan: false,
                        plan: None,
                        links: vec![],
                        answers: vec![],
                    }),
                }),
                turn: Some(Turn::Asking {
                    question: "Shall I go on?".to_string(),
                }),
                idle_ms: Some(4_000),
                subagents_running: Some(2),
                summary: Some("The plan is written.".to_string()),
            }],
            focus: Some(FocusInfo {
                pane_id: "w2:pZ".to_string(),
                project: Some("flankr".to_string()),
                agent: Some("Claude".to_string()),
                tab_label: Some("17-hr".to_string()),
                tab_number: Some(17),
                name: "the flankr agent".to_string(),
                spoken: "the flankr agent".to_string(),
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
                    links: vec![],
                    origin: None,
                    text: None,
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
            read: "I added the prompts module.".to_string(),
            summary: Some("The prompts module is added.".to_string()),
            complex: true,
        });
        round_trip(SpeechSummarizeResult {
            read: "Done.".to_string(),
            summary: None,
            complex: false,
        });
        round_trip(PingParams {
            active: Some(false),
        });
        assert_eq!(
            serde_json::to_value(PingParams::default()).unwrap(),
            serde_json::json!({})
        );
        round_trip(ClientOptionsParams {
            spoken_output: true,
            background: true,
            general_agent: false,
            tab_follows_focus: true,
        });
        // Left out, Tab follows focus is off.
        let old: ClientOptionsParams =
            serde_json::from_value(serde_json::json!({"general_agent": true})).unwrap();
        assert!(!old.tab_follows_focus);
        round_trip(LinkUnpairParams {});
        round_trip(LinkUnpairResult {});
        round_trip(AgentsReadResult {
            text: "ok".to_string(),
        });
        round_trip(AgentsSendParams {
            pane_id: "p1".to_string(),
            command: "continue".to_string(),
            kind: SendKind::Command,
            handle: Some("keep-going".to_string()),
            confirmed: false,
        });
        round_trip(AgentsSendParams {
            pane_id: "w2:pZ".to_string(),
            command: "please run the tests".to_string(),
            kind: SendKind::Message,
            handle: Some("message-claude-flankr-please-run".to_string()),
            confirmed: true,
        });
        // Left out, it is off, and off is not written.
        let old: AgentsSendParams =
            serde_json::from_value(serde_json::json!({"pane_id": "p1", "command": "continue"}))
                .unwrap();
        assert!(!old.confirmed);
        assert!(
            serde_json::to_value(&old)
                .unwrap()
                .get("confirmed")
                .is_none()
        );
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
            name: "the flankr agent".to_string(),
            spoken: "the flankr agent".to_string(),
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
            url: "flankr://127.0.0.1:7347/pair#flankr1abc".to_string(),
            pairing: "flankr1abc".to_string(),
            addresses: vec!["127.0.0.1:7347".to_string()],
            warnings: vec!["no forward".to_string()],
        });
        round_trip(LinkPairAddParams {
            code: "flankrkey1abc".to_string(),
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
        for key in [
            "name",
            "spoken",
            "tab_label",
            "tab_number",
            "session",
            "turn",
            "idle_ms",
            "subagents_running",
            "summary",
        ] {
            assert!(shown.get(key).is_none(), "{key} in {shown}");
        }
        assert_eq!(agent.turn, None);
        // The drawer's fields (plan 20261028_03, 3.12) are additive: an
        // older list without them reads as `None`, and they are on the
        // wire, by these names, only when set.
        assert_eq!(
            (&agent.idle_ms, &agent.subagents_running, &agent.summary),
            (&None, &None, &None)
        );
        let active = serde_json::to_value(AgentInfo {
            idle_ms: Some(1_500),
            subagents_running: Some(0),
            summary: Some("Tests pass.".to_string()),
            ..agent.clone()
        })
        .unwrap();
        assert_eq!(active["idle_ms"], 1_500);
        assert_eq!(active["subagents_running"], 0);
        assert_eq!(active["summary"], "Tests pass.");
        let back: AgentInfo = serde_json::from_value(active).unwrap();
        assert_eq!(back.idle_ms, Some(1_500));
        assert_eq!(back.subagents_running, Some(0));
        assert_eq!(back.summary.as_deref(), Some("Tests pass."));
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

        // A turn is tagged by `state`.
        for (turn, wire) in [
            (
                Turn::Working {
                    line: "Bash cargo test".to_string(),
                },
                json!({"state": "working", "line": "Bash cargo test"}),
            ),
            (
                Turn::Stopped {
                    line: "Done.".to_string(),
                },
                json!({"state": "stopped", "line": "Done."}),
            ),
            (
                Turn::Asking {
                    question: "Push it?".to_string(),
                },
                json!({"state": "asking", "question": "Push it?"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&turn).unwrap(), wire);
            assert_eq!(serde_json::from_value::<Turn>(wire).unwrap(), turn);
        }

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

        // speech.summarize: a plain reply's answer has no `summary`, and the
        // client's text may be left out.
        assert_eq!(
            serde_json::to_value(SpeechSummarizeResult {
                read: "Done.".to_string(),
                summary: None,
                complex: false
            })
            .unwrap(),
            json!({"read": "Done.", "complex": false})
        );
        let summarize: SpeechSummarizeParams =
            serde_json::from_value(json!({"pane_id": "p1", "ts": "t"})).unwrap();
        assert_eq!(summarize.text, "");

        // client.options: an unknown option is ignored, a missing one off.
        let options: ClientOptionsParams =
            serde_json::from_value(json!({"spoken_output": true, "louder": 3})).unwrap();
        assert!(options.spoken_output);
        let options: ClientOptionsParams = serde_json::from_value(json!({})).unwrap();
        assert!(!options.spoken_output);
        // The general agent is on unless it is said to be off.
        assert!(options.general_agent);
        assert_eq!(options, ClientOptionsParams::default());
        let options: ClientOptionsParams =
            serde_json::from_value(json!({"general_agent": false})).unwrap();
        assert!(!options.general_agent);

        // link.unpair takes nothing and answers nothing.
        assert_eq!(
            serde_json::to_value(LinkUnpairParams {}).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(LinkUnpairResult {}).unwrap(),
            json!({})
        );
        let _: LinkUnpairParams = serde_json::from_value(json!({})).unwrap();

        // link.pair_begin takes nothing; link.pair_add needs only the code.
        assert_eq!(
            serde_json::to_value(LinkPairBeginParams {}).unwrap(),
            json!({})
        );
        let add: LinkPairAddParams =
            serde_json::from_value(json!({"code": "flankrkey1abc"})).unwrap();
        assert_eq!(
            add,
            LinkPairAddParams {
                code: "flankrkey1abc".to_string(),
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

    #[test]
    fn full_text_points_spoken_and_bookends_are_appended_and_defaulted() {
        // Old JSON reads with the new fields empty, and empty ones stay off
        // the wire.
        let old: Entry = serde_json::from_value(
            json!({"ts": "t", "role": "assistant", "kind": "text", "summary": "hi"}),
        )
        .unwrap();
        assert_eq!(old.text, None);
        assert!(serde_json::to_value(&old).unwrap().get("text").is_none());
        let full = Entry {
            text: Some("hi, and the rest".into()),
            ..old
        };
        round_trip(full.clone());
        assert_eq!(
            serde_json::to_value(&full).unwrap()["text"],
            "hi, and the rest"
        );

        let card: CardBody = serde_json::from_value(json!({
            "card": "finished", "live": false, "title": "Finished", "text": "Done."
        }))
        .unwrap();
        assert!(card.points.is_empty() && card.spoken.is_none());
        let value = serde_json::to_value(&card).unwrap();
        assert!(value.get("points").is_none() && value.get("spoken").is_none());
        let summarised = CardBody {
            points: vec!["Wrote it".into(), "Pushed it".into()],
            spoken: Some("Written and pushed.".into()),
            ..card
        };
        round_trip(summarised.clone());
        let value = serde_json::to_value(&summarised).unwrap();
        assert_eq!(value["points"], json!(["Wrote it", "Pushed it"]));
        assert_eq!(value["spoken"], "Written and pushed.");

        let slot: Slot = serde_json::from_value(json!({
            "index": 3, "serial": "3fa9c21e:412.0", "kind": "card",
            "lines": 2, "chars": 40
        }))
        .unwrap();
        assert!(!slot.bookend);
        let bookend = Slot {
            bookend: true,
            ..slot
        };
        round_trip(bookend);
        assert_eq!(serde_json::to_value(bookend).unwrap()["bookend"], true);
    }

    /// Plan 20261027_07, step 2: the `terminal.*` methods.
    #[test]
    fn terminal_methods_and_their_shapes() {
        assert_eq!(methods::TERMINAL_OPEN, "terminal.open");
        assert_eq!(methods::TERMINAL_CLOSE, "terminal.close");
        assert_eq!(methods::TERMINAL_ACK, "terminal.ack");
        assert_eq!(methods::TERMINAL_INPUT_ARM, "terminal.input_arm");
        assert_eq!(methods::TERMINAL_INPUT, "terminal.input");
        assert_eq!(methods::TERMINAL_CLOSED, "terminal.closed");
        assert_eq!(methods::TERMINAL_SCROLL, "terminal.scroll");

        let open = TerminalOpenParams {
            pane_id: "w2:p4".into(),
            cols_hint: Some(60),
            rows_hint: None,
            cursor: false,
        };
        round_trip(open.clone());
        assert_eq!(
            serde_json::to_value(&open).unwrap(),
            json!({"pane_id": "w2:p4", "cols_hint": 60})
        );
        let by_session: TerminalOpenParams =
            serde_json::from_value(json!({"session": "w2:p4"})).unwrap();
        assert_eq!(by_session.pane_id, "w2:p4");
        assert_eq!((by_session.cols_hint, by_session.rows_hint), (None, None));
        round_trip(TerminalOpenResult {
            terminal_id: 1,
            cols: 200,
            rows: 60,
            cursor: false,
            source: TerminalSource::Poll,
        });
        round_trip(TerminalCloseParams { terminal_id: 1 });
        assert_eq!(
            serde_json::to_value(TerminalAckParams {
                terminal_id: 1,
                seq: 7
            })
            .unwrap(),
            json!({"terminal_id": 1, "seq": 7})
        );
        round_trip(TerminalInputArmParams { terminal_id: 1 });
        round_trip(TerminalInputArmResult {
            expires_in_s: TERMINAL_INPUT_ARM_SECS,
        });
        round_trip(TerminalInputParams {
            terminal_id: 1,
            keys: "ls enter".into(),
        });
        assert_eq!(TERMINAL_INPUT_MAX, 64);
        round_trip(TerminalScrollParams {
            terminal_id: 1,
            lines: -3,
        });
        round_trip(TerminalScrollResult {
            offset_from_bottom: 12,
            max_offset_from_bottom: Some(400),
        });
        let bare: TerminalScrollResult =
            serde_json::from_value(json!({"offset_from_bottom": 0})).unwrap();
        assert_eq!(bare.max_offset_from_bottom, None);

        for (reason, wire) in [
            (TerminalClosedReason::PaneClosed, "pane_closed"),
            (TerminalClosedReason::AgentChanged, "agent_changed"),
            (TerminalClosedReason::InputRevoked, "input_revoked"),
            (TerminalClosedReason::Closed, "closed"),
        ] {
            let c = TerminalClosedParams {
                terminal_id: 3,
                reason,
            };
            round_trip(c);
            assert_eq!(
                serde_json::to_value(c).unwrap(),
                json!({"terminal_id": 3, "reason": wire})
            );
        }
        // A reason from a newer daemon reads as Unknown.
        let c: TerminalClosedParams =
            serde_json::from_value(json!({"terminal_id": 3, "reason": "resized_away"})).unwrap();
        assert_eq!(c.reason, TerminalClosedReason::Unknown);
    }

    /// Plan 20261027_08, step 6: the appended fields default from older
    /// JSON; `terminal.mouse` and `terminal.resize`.
    #[test]
    fn terminal_cursor_source_mouse_and_resize() {
        assert_eq!(methods::TERMINAL_MOUSE, "terminal.mouse");
        assert_eq!(methods::TERMINAL_RESIZE, "terminal.resize");

        // Older JSON.
        let old: TerminalOpenParams = serde_json::from_value(json!({"pane_id": "w1:p1"})).unwrap();
        assert!(!old.cursor);
        let old: TerminalOpenResult =
            serde_json::from_value(json!({"terminal_id": 1, "cols": 80, "rows": 24})).unwrap();
        assert!(!old.cursor);
        assert_eq!(old.source, TerminalSource::Poll);

        // New JSON.
        let open = TerminalOpenParams {
            pane_id: "w1:p1".into(),
            cols_hint: Some(48),
            rows_hint: Some(30),
            cursor: true,
        };
        round_trip(open.clone());
        assert_eq!(
            serde_json::to_value(&open).unwrap(),
            json!({"pane_id": "w1:p1", "cols_hint": 48, "rows_hint": 30, "cursor": true})
        );
        let attached = TerminalOpenResult {
            terminal_id: 2,
            cols: 48,
            rows: 30,
            cursor: true,
            source: TerminalSource::Attached,
        };
        round_trip(attached);
        assert_eq!(
            serde_json::to_value(attached).unwrap(),
            json!({"terminal_id": 2, "cols": 48, "rows": 30, "cursor": true, "source": "attached"})
        );
        assert_eq!(
            serde_json::to_value(TerminalOpenResult {
                cursor: false,
                source: TerminalSource::Poll,
                ..attached
            })
            .unwrap(),
            json!({"terminal_id": 2, "cols": 48, "rows": 30, "source": "poll"})
        );
        let newer: TerminalOpenResult = serde_json::from_value(
            json!({"terminal_id": 2, "cols": 48, "rows": 30, "source": "observed"}),
        )
        .unwrap();
        assert_eq!(newer.source, TerminalSource::Unknown);

        // Each mouse kind round-trips, flat.
        for (kind, wire) in [
            (
                TerminalMouseKind::Down {
                    button: MouseButton::Left,
                },
                json!({"kind": "down", "button": "left"}),
            ),
            (
                TerminalMouseKind::Up {
                    button: MouseButton::Right,
                },
                json!({"kind": "up", "button": "right"}),
            ),
            (
                TerminalMouseKind::Drag {
                    button: MouseButton::Middle,
                },
                json!({"kind": "drag", "button": "middle"}),
            ),
            (TerminalMouseKind::Moved, json!({"kind": "moved"})),
        ] {
            for modifiers in [0, 4] {
                let params = TerminalMouseParams {
                    terminal_id: 1,
                    kind,
                    col: 4,
                    row: 2,
                    modifiers,
                };
                round_trip(params);
                let mut expected = json!({"terminal_id": 1, "col": 4, "row": 2});
                for (k, v) in wire.as_object().unwrap() {
                    expected[k] = v.clone();
                }
                if modifiers != 0 {
                    expected["modifiers"] = json!(modifiers);
                }
                assert_eq!(serde_json::to_value(params).unwrap(), expected);
            }
        }
        // A kind from a newer phone reads as Unknown; modifiers default.
        let newer: TerminalMouseParams =
            serde_json::from_value(json!({"terminal_id": 1, "kind": "pinch", "col": 0, "row": 0}))
                .unwrap();
        assert_eq!(newer.kind, TerminalMouseKind::Unknown);
        assert_eq!(newer.modifiers, 0);
        // A button's kind needs its button.
        assert!(
            serde_json::from_value::<TerminalMouseParams>(
                json!({"terminal_id": 1, "kind": "down", "col": 0, "row": 0})
            )
            .is_err()
        );

        let resize = TerminalResizeParams {
            terminal_id: 1,
            cols: 48,
            rows: 22,
        };
        round_trip(resize);
        assert_eq!(
            serde_json::to_value(resize).unwrap(),
            json!({"terminal_id": 1, "cols": 48, "rows": 22})
        );
    }
}
