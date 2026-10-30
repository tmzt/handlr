//! The companion's logic as a pure state machine (plan 20261022_04,
//! decisions 2–4, as amended at 18:07 EDT).
//!
//! [`State`] has no threads, sockets or clocks. Whoever drives it (the
//! `net` thread, or a test) tells it what happened, as an [`Input`] from the
//! app, a message from the daemon, or the connection opening or closing,
//! and carries out the [`Output`]s it returns: events for the app, requests
//! for the daemon, and closing the connection.
//!
//! Time comes in as data too: [`Input::Tick`] carries the driver's
//! monotonic clock, which the dictation mode's silence timer reads (plan
//! 20261023_02, the 01:57 EDT amendment). The `net` thread ticks before
//! every input and message and every [`TICK`]; a test gives any times it
//! likes.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use flankr_speech::LocalTime;
use libflankr_proto::common::{self, methods, protocols};
use libflankr_proto::{Id, Incoming, Notification, Request, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::keys::RemoteId;
use crate::terminal::{Act, Terminals};
use crate::view::{Outline, Sections, ViewSection, ViewWindow};
use crate::wire;

/// What the app tells the client. Not exhaustive: a later variant is not
/// a breaking change (plan 20261028_03, the 18:48 EDT amendment, item 6).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Input {
    /// What `flankr pair` shows: the pairing URL, the bare `flankr1…`
    /// string, or the QR code's `data:image/png;base64,` URL. The `net`
    /// thread makes this remote's key and, for a link with an offer, sends
    /// the daemon a pairing request beside the open connection, giving
    /// [`Event::PairingWords`] once it is held; otherwise (or with the
    /// daemon not reached) it gives the device-code fallback and tries the
    /// remote until it answers `paired: true` (plan 20261028_01, decision
    /// 10 and the 14:37 EDT amendment); [`State`] itself cannot, and gives
    /// [`Event::PairFailed`].
    Pair(String),
    /// Leaving the pairing screen: the pairing in progress, if any, ends
    /// (a held request's connection is closed, which the daemon takes as a
    /// refusal), and its new key is deleted (plan 20261028_01, decision 10
    /// and the 14:37 EDT amendment).
    CancelPairing,
    /// A remote's X, confirmed (plan 20261028_01, decision 22): the daemon
    /// is asked to forget this phone (`link.unpair`: over the open link
    /// for the active remote, over one short connection for any other),
    /// then the remote's state directory, key and record are deleted here,
    /// answered or not, and [`Event::RemoteRemoved`] is given. Removing the
    /// active remote makes the most recently paired of the rest active.
    RemoveRemote(RemoteId),
    /// A remote's nickname, set (`Some`) or cleared back to the default
    /// (`None`, or empty) in its details (plan 20261024_02, the 2026-10-28
    /// 17:56 EDT amendment). The `net` thread keeps it as
    /// [`keys::nickname`](crate::keys::nickname) makes it, writes the
    /// remotes list, and gives [`Event::Remotes`] with the new
    /// [`RemoteInfo::display_name`](crate::RemoteInfo::display_name)s.
    SetNickname {
        id: RemoteId,
        nickname: Option<String>,
    },
    /// A tap on a remote's card in the drawer that is not in use (plan
    /// 20261028_03, 3.7 and the 18:51 EDT amendment): the `net` thread
    /// writes `active` first, puts the old remote's link in the background
    /// (its agent view, terminals and a prompt for one of its panes
    /// closed) and this one's in front, and gives this remote's sessions,
    /// cards and connection as they stand, then [`Event::Remotes`]. No
    /// link is closed or made: every listed remote stays connected.
    /// Ignored for the remote in use, one not listed, or one being
    /// removed.
    SwitchRemote(RemoteId),
    /// The drawer is shown (`true`) or hidden (plan 20261028_03, 3.12 item
    /// 48): while shown, the active remote's agents are listed every
    /// [`DRAWER_LIST_INTERVAL`], also with an agent view open, and once at
    /// once when it is shown.
    DrawerShown(bool),
    /// `input` for the link of `remote`, in use or not: an answer to one
    /// of its [`Event::Background`] events, such as an approval
    /// ([`Input::Answer`]). Handled by the `net` thread.
    ToRemote { remote: RemoteId, input: Box<Input> },
    /// What STT heard, or a typed line.
    Utterance(String),
    /// The tap, or Enter.
    Tap,
    /// The Approve button.
    Approve,
    /// The Deny button.
    Deny,
    /// TTS started (`true`) or finished (`false`).
    Speaking(bool),
    /// The STT gate opened (`true`) or went idle (`false`).
    Listening(bool),
    /// Voice is being heard (`true`: the gate is open on speech) or has
    /// stopped (`false`). An armed gate waiting for speech is not voice.
    /// Front ends with a VAD send it; once one has, the dictation silence
    /// counts from the end of voice, not from the gate going idle (plan
    /// 20261023_02, the VAD amendment).
    Voice(bool),
    /// The driver's monotonic clock: the time since any fixed start. It
    /// never goes back; an earlier time is taken as the latest one.
    Tick(Duration),
    /// The Start button: take a message (plan 20261023_02, decision 2).
    StartDictation,
    /// Start taking a typed text prompt (plan 20261022_03, living section L3).
    StartTextPrompt,
    /// Start taking a general agent prompt (plan 20261024_03, the 00:30 EDT amendment).
    StartGeneralAgentPrompt(PromptMode),
    /// The typed prompt open (the message or Gemini's) goes on by voice: the
    /// same prompt in voice mode, its text kept as its first part, the gate
    /// armed. Nothing with the microphone off or no typed prompt open (plan
    /// 20261028_06, the 2026-10-29 15:09 EDT amendment, item 5).
    ContinueByVoice,
    /// Set the current prompt text after an edit.
    SetText(String),
    /// The Submit button: submit the message now.
    Submit,
    /// The Clear button: discard the message and leave the mode.
    Clear,
    /// The Scratch button: remove the last utterance.
    Scratch,
    /// Open the agent view of this pane (plan 20261022_03, step 4): its
    /// session is subscribed to (plan 20261023_03, section 6), and the
    /// daemon pushes its sections while the view is open.
    OpenAgent(String),
    /// Close the agent view: its subscription ends.
    CloseAgent,
    /// Show the open view's subagent with this id (`ChildInfo::id`, from
    /// [`Event::AgentView`]'s `children`) in the same view (the subagents
    /// plan, decision 3 and step 4): the parent's subscription ends and
    /// the subagent's session is subscribed to. Its pane stays the
    /// parent's: what is sent from the view goes to the parent pane.
    /// Nothing without an open view.
    OpenChild(String),
    /// Back from a subagent to its parent's main session in the same view.
    /// Nothing when the view shows the main session.
    CloseChild,
    /// The agent view's flow came within a screen of its top: page in the
    /// sections before the oldest held (`session.history`, A7).
    LoadEarlier,
    /// Follow herdr's focus (plan 20261023_03, decision 2 and section 6):
    /// on, an open view moves to the pane `focus.changed` names. Off by
    /// default.
    Follow(bool),
    /// A card's Approve or Deny: answers this confirmation, whichever is
    /// being asked.
    Answer { confirm_id: u64, approve: bool },
    /// A suggested response under the agent view: sent to the view's pane
    /// with `agents.send`, which the daemon confirms as any other.
    Respond(Suggestion),
    /// One of a question's answers from the daemon (plan 20261027_06,
    /// step 4), tapped: its `sends` sent confirmed, or the message card
    /// opened with its `starts`.
    Suggested(common::Answer),
    /// A choice question's answers, from its card (plan 20261029_01, step
    /// 5): `choice.answer` for the open view's pane, `id` the card's
    /// [`common::ChoiceSet::id`] and one answer per question, in order. A
    /// tap is the confirmation. [`Event::ChoiceAnswered`] follows, with the
    /// result or the refusal; with no view open or offline, at once as
    /// `failed`. Added in 0.5.5.
    Choose {
        id: String,
        answers: Vec<common::ChoiceAnswer>,
    },
    /// The play button: read the agent's new replies aloud, or stop. Only
    /// its prose is read, never tool calls, their output or code blocks.
    ReadAloud(bool),
    /// The play button while something is being said: stop it, and the
    /// next held reply follows.
    Skip,
    /// Stop reading aloud (plan 20261028_06, the 2026-10-29 00:43 EDT
    /// amendment, items 1, 3 and 4): what is being said stops, as with
    /// [`Input::Skip`], and every reply and spoken notice waiting to be
    /// said is dropped, so nothing follows it. Replies that come after
    /// are read as usual.
    StopReading,
    /// The app went to the background (`true`) or came back (`false`).
    /// In the background the link is a single sleeping socket: no polling,
    /// a slow ping, and only what the daemon pushes.
    Background(bool),
    /// Where the agent view's window is on its ribbon (plan 20261024_01,
    /// decision 6): contents are fetched around it and forgotten far from
    /// it, and while it is scrolled back a new outline is held aside.
    Window(ViewWindow),
    /// The jump line's tap while a new outline is held aside: its
    /// sections newer than the shown ribbon's newest are added below the
    /// ribbon's end, and the rest is merged in once the window is back at
    /// the bottom, following (plan 20261022_03, the 01:06 EDT amendment,
    /// item 5; CHANGES.md, 2026-10-24 01:06 EDT).
    ApplyOutline,
    /// The settings screen's toggles, as they are now (plan 20261024_02,
    /// decisions 3 and 4). Given at start and on every change.
    Settings(Settings),
    /// Removes the active remote, as its [`Input::RemoveRemote`] does
    /// (plan 20261024_02's *Unpair this device*, which plan 20261028_01's
    /// Remotes section replaces). With no active remote, it gives
    /// [`Event::Unpaired`] at once.
    Unpair,
    /// The agent view was pulled past its end and let go (plan 20261022_03,
    /// the L3 amendment of 2026-10-24 12:22 EDT): a timed ping
    /// (reconnecting at once when offline), a resubscribe that resumes
    /// where the view is, and a fetch of the newest screen's contents,
    /// whatever their revs. [`Event::Refreshed`] says when it is done.
    Refresh,
    /// A summary card's play button: say its spoken text again (the
    /// 12:03 EDT amendment, item 3). Nothing with sound off.
    Replay(String),
    /// The phone is short of memory (the 12:21 EDT amendment): cached
    /// closed views go, least recently used first, and the open view keeps
    /// fewer contents, by the grade. Until libmorph 0.4.0 delivers the
    /// phone's events, nothing sends it but the tests.
    Memory(Pressure),
    /// *Clear cache* (plan 20261024_02, the 12:12 EDT amendment): the
    /// closed views kept in memory are dropped.
    ClearCache,
    /// A swipe down on another session's card (plan 20261022_03, the L3
    /// amendment of 2026-10-24 15:56 EDT, item 3): that pane's newest card
    /// goes. A question or approval stays away while the same is asked.
    DismissCard(String),
    /// Open the terminal of this session (a pane id or name), as
    /// `terminal.open` with what the phone could show (plan 20261027_07,
    /// step 4). It stays open across connections until
    /// [`Input::TerminalClose`] or the daemon's `terminal.closed`; see
    /// [`Terminals`].
    TerminalOpen {
        session: String,
        cols_hint: Option<u16>,
        rows_hint: Option<u16>,
    },
    /// Close the session's terminal: it is forgotten.
    TerminalClose(String),
    /// Arm keyboard input on the session's open terminal
    /// (`terminal.input_arm`, step 6): [`Terminals::status`]'s `armed_for`
    /// is set once answered, and stays until the terminal is closed (no
    /// expiry).
    TerminalArm(String),
    /// Keys into the session's armed terminal, in herdr's `send_keys`
    /// syntax: sent as `terminal.input` in batches of at most
    /// `TERMINAL_INPUT_MAX` bytes ([`crate::terminal::key_batches`]).
    TerminalKeys { session: String, keys: String },
    /// Scroll the session's armed terminal by `lines`, as a mouse wheel
    /// does (positive back into the scrollback): `terminal.scroll`, one in
    /// flight at a time, the lines asked for meanwhile summed into the next
    /// (plan 20261022_03, the L3 amendment of 2026-10-27 21:58 EDT).
    TerminalScroll { session: String, lines: i32 },
    /// The columns and rows that fit the phone's terminal area now (plan
    /// 20261027_08, the 23:24 EDT amendment): kept for the next open, and
    /// sent as `terminal.resize` when it differs from the size last sent,
    /// one in flight, the newest winning.
    TerminalResize {
        session: String,
        cols: u16,
        rows: u16,
    },
    /// A mouse event at a pane-local cell of the session's armed terminal
    /// (`terminal.mouse`, plan 20261027_08 decision 10): sent in order, one
    /// in flight, a drag behind a drag replacing it.
    TerminalMouse {
        session: String,
        kind: common::TerminalMouseKind,
        col: u16,
        row: u16,
        modifiers: u8,
    },
    /// A call to a plugin's own method (docs/design/20261028_04_PLUGINS_PLAN.md,
    /// 3.10 item 1), sent as `plugin.call` to the remote in use. With
    /// `id`, its answer comes as [`Event::PluginResult`] with the same
    /// `ns` and `id`, also when the call is refused at once (offline, a
    /// bad `ns` or `method`, `params` over
    /// [`PLUGIN_MAX_BYTES`](wire::PLUGIN_MAX_BYTES), or the daemon's
    /// refusal), or when the connection closes first (`CANCELLED`).
    /// Without `id` it is a notification to the plugin and nothing
    /// follows. `session` is the pane id of the agent view the phone has
    /// open, if any. Added in 0.5.1.
    PluginCall {
        ns: String,
        id: Option<u64>,
        method: String,
        params: Value,
        session: Option<String>,
    },
    /// Settings → Plugins' toggle: `plugin.set_enabled` (3.10 item 1).
    /// [`Event::Plugins`] follows either way: the daemon's list once
    /// written, or the list as it was, with [`Event::Error`], when it was
    /// refused. Added in 0.5.1.
    SetPluginEnabled { ns: String, enabled: bool },
    /// Attach an image (PNG, JPEG, WebP or GIF, `mime` as its bytes say)
    /// to the message for the open agent view's session
    /// (docs/design/20261028_05_BINARY_FILES_PLAN.md, decisions 9.2-9.3):
    /// it takes the lowest free number N, 1-4, and its `images` upload
    /// starts at once. [`Event::Attachments`] lists it; the app puts its
    /// token `[image N]` in the text ([`Input::SetText`]). Once the token
    /// has been in the text, taking it out detaches the image. Refused
    /// with [`Event::Error`] when the daemon takes no images, four are
    /// attached, or the bytes are not the type or over the cap. Added in
    /// 0.5.3.
    AttachImage { bytes: Vec<u8>, mime: String },
    /// The ✕ on image N: it goes, with its token in the text; an upload in
    /// flight is cancelled. Added in 0.5.4.
    DetachImage(u8),
    /// A tap on image N's failed upload: it starts again. Added in 0.5.4.
    RetryImage(u8),
    /// Fetch `path` (relative to the session's project, as `read_file`
    /// takes it) from the daemon's `files` for the preview (decision 6.1):
    /// [`Event::TransferStarted`], then [`Event::TransferProgress`] and
    /// [`Event::FileReady`], or [`Event::TransferFailed`]. `session` is
    /// the agent view's pane. Added in 0.5.4.
    RequestFile { session: String, path: String },
    /// Fetch a `git` output (`show <rev>`, `diff <rev>..<rev>`, `diff`,
    /// each optionally `-- <path>`; decision 6.5) for the session, as
    /// [`Input::RequestFile`] does. Added in 0.5.4.
    RequestGit { session: String, request: String },
    /// Cancel transfer `id` (an [`Event::TransferStarted`]'s): a download
    /// ends with [`Event::TransferFailed`] without a line; an image's
    /// upload is detached. Added in 0.5.4.
    CancelTransfer(u32),
    /// Paste `text` into the open agent view's armed terminal
    /// (`terminal.paste`; the 23:43 EDT amendment, item 2). Added in 0.5.4.
    TerminalPasteText(String),
    /// Paste the paths of these attached images (by number, uploaded for
    /// the open agent view's session) into its armed terminal
    /// (`terminal.paste`): once pasted they are used up and go. Added in
    /// 0.5.3.
    TerminalPasteImages(Vec<u8>),
}

/// A grade of memory pressure (plan 20261022_03, the L3 amendment of
/// 2026-10-24 12:21 EDT), in increasing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    /// libmorph's `on_trim_memory` at `UI_HIDDEN`: closed views go but the
    /// most recent; the open view keeps contents within 5 screens.
    UiHidden,
    /// `on_trim_memory` at `BACKGROUND`: every closed view goes; the open
    /// view keeps contents within 1 screen.
    Background,
    /// `on_low_memory`: a full drop, so the app is not killed. Every
    /// closed view goes; the open view keeps only what is on screen, and
    /// its outline.
    Low,
}

/// The closed agent views kept in memory, the most recently closed first
/// kept (plan 20261022_03, the L3 amendment of 2026-10-24 12:15 EDT, item
/// 1).
pub const CACHED_VIEWS: usize = 3;

/// How long a refresh may take before its result is given with what came.
pub const REFRESH_WAIT: Duration = Duration::from_secs(5);

/// The settings the client acts on (plan 20261024_02, decisions 3 and 4).
/// Each is on by default, except `tab_follows_focus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// *All sound*: off, nothing is said ([`Event::Say`]), no ding is
    /// given, and no reply is queued to be read aloud.
    pub sound: bool,
    /// *Ding when an agent is done*: [`Event::Ding`] once per new
    /// Finished card. Only while `sound` is on.
    pub ding: bool,
    /// *Microphone*: off, the STT gate is never armed ([`Event::Arm`]),
    /// utterances are ignored, and no message is dictated.
    pub microphone: bool,
    /// *General agent (Gemini)*: off, no Gemini turn is started, and
    /// `client.options` says so. An utterance goes to the agent in view as
    /// a message, and on the overview does nothing (the 11:37 EDT
    /// amendment).
    pub general_agent: bool,
    /// *Tab follows focus*, off by default: on, Gemini's `focus_project`
    /// also moves herdr's focused tab to that agent's tab; sent in
    /// `client.options` (plan 20261024_02, the 2026-10-27 11:27 EDT
    /// amendment; plan 20261027_04, step 6).
    pub tab_follows_focus: bool,
    /// *Speak agent (Gemini) responses*, on by default: off, a `turn.say`
    /// still gives [`Event::Conversation`] (the card shows the line) but
    /// no [`Event::Say`] (plan 20261028_06, decision 6). What is being
    /// said is not cut off; [`Input::Skip`] does that.
    pub speak_gemini: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound: true,
            ding: true,
            microphone: true,
            general_agent: true,
            tab_follows_focus: false,
            speak_gemini: true,
        }
    }
}

/// A tapped suggestion on its way: its own approval, once confirmed.
#[derive(Debug, Clone)]
struct Tapped {
    pane_id: String,
    /// What was sent.
    text: String,
    /// Known once `agents.send` answers.
    confirm_id: Option<u64>,
}

/// The agent view's suggested responses. A question whose daemon sends
/// its answers is answered with [`Input::Suggested`] instead; `Approve`,
/// `Discuss`, `Implement` and [`plan_answer`] serve only an older daemon
/// (plan 20261027_06, decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Suggestion {
    Yes,
    No,
    Continue,
    Compact,
    /// A plan question's answers (plan 20261022_03, the 21:57 EDT
    /// amendment): approve sends "Approve the <plan>", implement "Approve
    /// the <plan> and start building it", and discuss takes a message
    /// starting "Discuss the <plan>: ".
    Approve,
    Discuss,
    Implement,
}

impl Suggestion {
    /// The row's short label.
    pub fn label(self) -> &'static str {
        match self {
            Suggestion::Yes => "yes",
            Suggestion::No => "no",
            Suggestion::Continue => "cont",
            Suggestion::Compact => "cmpct",
            Suggestion::Approve => "approve",
            Suggestion::Discuss => "discuss",
            Suggestion::Implement => "implement",
        }
    }

    /// What is sent: the text, and whether it is a message (typed as the
    /// user's reply) or one of the agent's allowed commands.
    pub fn send(self) -> (&'static str, common::SendKind) {
        match self {
            Suggestion::Yes => ("yes", common::SendKind::Message),
            Suggestion::No => ("no", common::SendKind::Message),
            Suggestion::Continue => ("continue", common::SendKind::Command),
            Suggestion::Compact => ("/compact", common::SendKind::Command),
            Suggestion::Approve => ("Approve the plan", common::SendKind::Message),
            Suggestion::Discuss => ("Discuss the plan:", common::SendKind::Message),
            Suggestion::Implement => (
                "Approve the plan and start building it",
                common::SendKind::Message,
            ),
        }
    }

    /// Whether the tap answers the pending question, so its card goes.
    fn answers(self) -> bool {
        matches!(
            self,
            Suggestion::Yes | Suggestion::No | Suggestion::Approve | Suggestion::Implement
        )
    }
}

/// The plan as a plan answer names it: "plan 03" as the daemon gives its
/// id (plan 20261022_03, L3, the 2026-10-27 13:21 EDT amendment), an older
/// daemon's spoken name ("the overview screen plan"), or "the plan" with
/// none. Only letters, digits, spaces and `. , ' _ -` are kept.
fn plan_name(plan: Option<&str>) -> String {
    let plan: String = plan
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | ',' | '\'' | '_' | '-') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let plan = plan.split_whitespace().collect::<Vec<_>>().join(" ");
    if plan.is_empty() {
        "the plan".to_string()
    } else if plan.to_ascii_lowercase().starts_with("the ")
        || plan.to_ascii_lowercase().starts_with("plan ")
    {
        plan
    } else {
        format!("the {plan}")
    }
}

/// What a tapped approve, discuss or implement says (plan 20261022_03,
/// the 21:57 EDT amendment): "Approve the <plan>", the start of the
/// message "Discuss the <plan>:", or "Approve the <plan> and start
/// building it". At most 200 characters.
pub fn plan_answer(response: Suggestion, plan: Option<&str>) -> Option<String> {
    let plan = plan_name(plan);
    let text = match response {
        Suggestion::Approve => format!("Approve {plan}"),
        Suggestion::Discuss => format!("Discuss {plan}:"),
        Suggestion::Implement => format!("Approve {plan} and start building it"),
        _ => return None,
    };
    Some(text.chars().take(200).collect())
}

/// How often the `net` thread ticks while connected.
pub const TICK: Duration = Duration::from_millis(250);

/// The silence that submits a dictated message by default, and the least
/// it may be set to (plan 20261023_02, the 01:57 EDT amendment).
pub const DICTATION_SILENCE: Duration = Duration::from_secs(8);
/// The short silence that submits a general agent voice turn (plan 20261024_03, the 00:30 EDT amendment).
pub const GENERAL_AGENT_SILENCE: Duration = Duration::from_millis(1000);
/// The longest dictated message, in characters (decision 1).
pub const MESSAGE_CAP: usize = 4000;
/// The countdown shown before a silence submits, in seconds.
pub const COUNTDOWN_SECS: u64 = 3;
/// How long a Submit waits, after the voice ends, for what was said to
/// come back from STT.
pub const HEARD_WAIT: Duration = Duration::from_secs(3);

/// The sections the agent view's subscription backfills: the most one
/// delivery holds (plan 20261023_03, section 6).
pub const VIEW_SECTIONS: usize = common::SECTIONS_MAX;
/// The sections one history page asks for (A5).
pub const HISTORY_PAGE: usize = common::HISTORY_SECTIONS_DEFAULT;

/// How often the agents are listed while the overview shows and nothing
/// else asked (plan 20261023_04, decision 6).
pub const LIST_INTERVAL: Duration = Duration::from_secs(15);
/// The least time between the last list sent and one asked for since (as
/// on `focus.changed`).
pub const LIST_MIN_INTERVAL: Duration = Duration::from_secs(5);
/// How often the agents are listed while the drawer shows
/// ([`Input::DrawerShown`]; plan 20261028_03, 3.12 item 48).
pub const DRAWER_LIST_INTERVAL: Duration = Duration::from_secs(3);

/// Where a dictated message stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationPhase {
    /// Utterances are added to the message.
    Collecting,
    /// Submitted: `agents.send` is on its way.
    Submitting,
    /// The daemon asks for approval; "deny" resumes collecting.
    Confirming,
}

/// The prompt mode: voice dictation or typed message via keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMode {
    Voice,
    Keyboard,
}

/// The dictation card (plan 20261023_02, decisions 3–5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictation {
    /// The mode is on. Off, the rest is empty and the card goes away.
    pub active: bool,
    /// Whether the prompt was started by voice dictation or the keyboard.
    pub mode: PromptMode,
    /// The message so far: the utterances, joined with a space.
    pub text: String,
    /// The agent it goes to, fixed when the mode started. `None` (or one
    /// without an agent) and Submit is refused.
    pub target: Option<common::FocusInfo>,
    /// The shown name of herdr's focus, when it has moved off the target.
    pub focus_moved: Option<String>,
    pub phase: DictationPhase,
    /// Seconds left before the silence submits, for the last
    /// [`COUNTDOWN_SECS`].
    pub submit_in: Option<u32>,
    /// Whether this prompt goes to the general agent (Gemini).
    pub general_agent: bool,
}

impl Dictation {
    fn off() -> Self {
        Dictation {
            active: false,
            mode: PromptMode::Voice,
            text: String::new(),
            target: None,
            focus_moved: None,
            phase: DictationPhase::Collecting,
            submit_in: None,
            general_agent: false,
        }
    }
}

/// The device-code fallback of a pairing ([`Event::PairingWords`]): for a
/// phone that cannot reach the daemon directly (plan 20261028_01, the
/// 14:37 EDT amendment, item 2). The pairing screen keeps it in Details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeFallback {
    /// This phone's device code, `flankrkey1…`, to paste into
    /// `flankr pair` (with a Copy button).
    pub code: String,
    /// The remote is already in the list and this phone's key for it was
    /// lost, so this is a new key. The daemon still lists the old one under
    /// this device's name, and adds this code only with
    /// `flankr pair --replace` (the 14:37 EDT amendment, item 4).
    pub replace: bool,
}

/// What the client tells the app. Not exhaustive: a later variant is not
/// a breaking change (plan 20261028_03, the 18:48 EDT amendment, item 6).
///
/// `Eq` is implemented by hand: [`wire::PluginResult`]'s error object
/// derives only `PartialEq`, though everything in it is `Eq`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// `hello` was answered.
    Connected {
        server: String,
    },
    /// The connection closed or could not be made; the next attempt is in
    /// `retry_in_ms`.
    Disconnected {
        retry_in_ms: u32,
    },
    /// A pairing waits for the owner (plan 20261028_01, the 14:37 and
    /// 14:42 EDT amendments). The pairing screen shows `words` large, with
    /// one line saying to confirm on the Mac, and Cancel; everything else
    /// is for its Details.
    ///
    /// With `fallback` `None`, the daemon has accepted this phone's
    /// pairing request and holds it: `flankr pair` prints the same six
    /// words and asks for **yes**. [`Event::Paired`] or
    /// [`Event::PairFailed`] follows.
    ///
    /// With `fallback` `Some`, the daemon could not be asked directly (it
    /// was not reached, or the link is version 2, without an offer): the
    /// device code is pasted into `flankr pair`, which then prints the
    /// words for that code's key, and the client keeps trying the daemon
    /// until it knows the key.
    PairingWords {
        /// The daemon's name, from the pairing link.
        server: String,
        /// The daemon's `host:port`: the one that answered the request, or
        /// in the fallback the link's first.
        address: String,
        /// The six pairing words (BIP-39), from the daemon's key, the
        /// link's offer and this phone's new key, as `flankr pair` prints
        /// them. `None` only in the fallback for a link without an offer.
        words: Option<String>,
        /// The daemon's fingerprint as six words, for Details: a different
        /// check from `words`, labelled "daemon fingerprint".
        fingerprint: String,
        /// The whole fingerprint, BLAKE2s-256 of the daemon's key, as 64
        /// lower-case hex digits ([`crate::keys::fingerprint_hex`]), for
        /// Details.
        fingerprint_hex: String,
        /// The seconds the offer had left when the daemon accepted the
        /// request; `None` in the fallback.
        expires_in_secs: Option<u16>,
        /// The device-code fallback, if this is it.
        fallback: Option<DeviceCodeFallback>,
    },
    /// The daemon added this phone's key: the owner typed **yes** to a
    /// pairing request, or (the fallback) `hello` was answered with
    /// `paired: true`. `name` is the name this device asked to be paired
    /// as.
    Paired {
        name: String,
    },
    /// A pairing ended without adding the remote, and why: a string with a
    /// bad checksum, a request the daemon refused, an offer that expired,
    /// a connection that closed. Its new key is deleted.
    PairFailed(String),
    /// Speak this.
    Say(String),
    /// Stop playback now.
    StopSpeaking,
    /// Arm the STT gate.
    Arm,
    /// A confirmation is waiting for the user's answer. Its `text` is also
    /// given as [`Event::Say`]. `phrase` is empty when the daemon sent none.
    /// For its card (plan 20261023_01, section 4): `command`, `agent` (the
    /// agent's shown name, else its herdr name) and `project`; `label` is
    /// the handle, which is shown and never spoken.
    Confirm {
        confirm_id: u64,
        text: String,
        label: Option<String>,
        phrase: String,
        command: String,
        agent: String,
        project: Option<String>,
    },
    /// The user answered a confirmation; the reply is on its way.
    Answered {
        confirm_id: u64,
        approve: bool,
    },
    /// The daemon refused an answer (a wrong nonce says how many tries
    /// are left). The confirmation is no longer asked.
    ReplyFailed {
        confirm_id: u64,
        message: String,
    },
    /// A confirmation ended, with plan 01's outcome (`sent`, `denied` …).
    Resolved {
        confirm_id: u64,
        outcome: String,
    },
    /// A turn ended. `stopped` are the spoken names of the agents its
    /// interrupt stopped.
    TurnEnded {
        outcome: String,
        stopped: Vec<String>,
    },
    /// The STT vocabulary.
    Vocabulary(Vec<String>),
    /// herdr's focus changed, or was first learned.
    Focus(Option<common::FocusInfo>),
    /// What the daemon's `[llm]` makes available, from `hello` and each
    /// `server.capabilities` (plan 20261027_02, decision 9). Without the
    /// general agent the centre button and the Gemini strip are hidden and
    /// the settings toggle is dim; without summaries, cards show no points.
    Llm(wire::LlmInfo),
    /// The agents, as `agents.list` gave them: on its first answer on each
    /// connection, then whenever the list changes (plan 20261023_04, decision 6).
    Agents(Vec<common::AgentInfo>),
    /// The panes with a flankr confirmation waiting, as
    /// [`State::pending_panes`] gives them, whenever they change.
    PendingPanes(Vec<String>),
    /// A line of the conversation with Gemini (plan 20261023_04, A5): the
    /// user's words as a turn starts, and each thing Gemini says in it.
    /// Gemini's are also given as [`Event::Say`].
    Conversation {
        speaker: Speaker,
        text: String,
    },
    /// The dictation card changed.
    Dictation(Dictation),
    /// The open agent view changed (plan 20261022_03, step 4, and the
    /// 23:54 EDT amendment's ribbon): the agent's shown name and status,
    /// its sections in serial order (each its entries or its card, a card
    /// the user has just answered left out), the pinned card's serial, and
    /// the serials changed since the view was last given. `loading` until
    /// the session's first answer.
    AgentView {
        name: String,
        status: String,
        sections: Vec<ViewSection>,
        pinned: Option<common::Serial>,
        /// The serials whose sections came, were replaced with different
        /// content, or went since the view was last given (every serial
        /// the first time), in serial order. The rest are as last given.
        changed: Vec<common::Serial>,
        loading: bool,
        /// A response sent from the view that the session has not shown
        /// yet, shown as typed input.
        typed: Option<String>,
        /// A history page is on its way: "loading earlier…" at the top.
        loading_earlier: bool,
        /// A history page reached the session's first line: "start of
        /// session" at the top.
        start: bool,
        /// herdr's focus, shown, when the terminal is on another pane than
        /// the view's: the view does not follow it.
        terminal_on: Option<String>,
        /// A new outline (a reset, or a new session file) is held aside
        /// while the view is scrolled back: the jump line's tap sends
        /// [`Input::ApplyOutline`], which adds its newer sections below.
        new_content: bool,
        /// The end of the new outline held aside, until its tap: its live
        /// cards and its newest held section, in index order, for the jump
        /// line's state (plan 20261022_03, the 01:07 EDT amendment). Empty
        /// when none is held aside, or once its newer sections are below.
        held_end: Vec<ViewSection>,
        /// The slots of the new outline held aside that come after the
        /// shown ribbon's newest, in index order, with their kinds and
        /// sizes: the scrollbar counts their estimated heights in its
        /// length (plan 20261022_03, the 11:09 EDT amendment, item 2).
        /// Empty when none is held aside, or once they are below the end.
        held_newer: Vec<crate::view::Slot>,
        /// The Finished cards' slots, in index order, for the scrollbar's
        /// section labels (the 12:27 EDT amendment).
        bookends: Vec<crate::view::Bookend>,
        /// The subagent shown, by its id, when the view shows one instead
        /// of the main session (the subagents plan, step 4); `name` is
        /// still its parent's.
        child: Option<String>,
        /// The pane's subagents as last listed (`AgentInfo::children`):
        /// the running first, then the finished, newest first within
        /// each. Empty when it has none.
        children: Vec<common::ChildInfo>,
    },
    /// An agent finished a turn: a new Finished card, on the overview's
    /// list or in the agent view (plan 20261024_02, decision 3). Play the
    /// ding. Given once per card, and not with the ding or sound off.
    Ding,
    /// The last remote was removed (or [`Input::Unpair`] came with none):
    /// no remote is left, and the app shows the pairing screen. `daemon`:
    /// the last remote's daemon answered that it forgot this device too;
    /// otherwise it still lists it until it is removed from `auth.toml`.
    /// Given after that remote's [`Event::RemoteRemoved`].
    Unpaired {
        daemon: bool,
    },
    /// The paired remotes, in pairing order, at start and whenever the list,
    /// the active remote or its connection changes (plan 20261028_01,
    /// decision 26).
    Remotes(Vec<crate::keys::RemoteInfo>),
    /// A remote was removed ([`Input::RemoveRemote`]): its key, record and
    /// state directory are gone. `daemon`: its daemon answered that it
    /// forgot this phone; otherwise it lists it until the entry is removed
    /// from its `auth.toml` (decision 22).
    RemoteRemoved {
        id: RemoteId,
        name: String,
        daemon: bool,
    },
    /// A pairing named a remote already in the list: no row was added, and
    /// its name and addresses were refreshed from the pairing (decision
    /// 11, *Already paired with m4: its addresses are updated*). Given
    /// only once a connection to the new addresses with this phone's key
    /// for it was answered; one that was not gives [`Event::PairFailed`]
    /// and the old addresses stay (the 14:37 EDT amendment, item 4).
    RemoteUpdated {
        id: RemoteId,
        name: String,
    },
    /// A refresh ([`Input::Refresh`]) is done: `new` sections came that
    /// the view did not hold before it.
    Refreshed {
        new: usize,
    },
    /// The size of the closed views kept in memory, in bytes, whenever it
    /// changes: *Clear cache (n MB)*.
    Cache {
        bytes: u64,
    },
    /// Other sessions' cards, newest first, whenever they change (plan
    /// 20261022_03, the L3 amendments of 2026-10-24 15:56, 16:00 and 16:01
    /// EDT): the app shows the newest at the bottom, with "+N" for the
    /// rest. A new question or approval also gives [`Event::Ding`].
    SessionCards(Vec<crate::cross::SessionCard>),
    Error(String),
    /// An event of a remote that is not in use (plan 20261028_03, the
    /// 18:51 EDT amendment): its link stays open in the daemon's background
    /// mode, so these are mostly what needs the user (a confirmation, a
    /// question, a finished turn's notice) and its connection's comings
    /// and goings. The events of the remote in use come untagged.
    Background {
        remote: RemoteId,
        event: Box<Event>,
    },
    /// The daemon's built-in plugins, enabled or not
    /// (docs/design/20261028_04_PLUGINS_PLAN.md, 3.10 item 2): after each
    /// `hello` (empty from an older daemon, which has none), on each
    /// `server.capabilities` that carries them, and after
    /// [`Input::SetPluginEnabled`]. Added in 0.5.1.
    Plugins(Vec<wire::PluginInfo>),
    /// A plugin's answer to an [`Input::PluginCall`] with an `id`, or that
    /// call's refusal. Added in 0.5.1.
    PluginResult(wire::PluginResult),
    /// A plugin's notification during a call. Added in 0.5.1.
    PluginNotify(wire::PluginNotify),
    /// A transfer started (docs/design/20261028_05_BINARY_FILES_PLAN.md):
    /// its id, for the events that follow and [`Input::CancelTransfer`];
    /// its namespace (`files`, `git` or `images`), session and request
    /// (an image's is `image N`). Added in 0.5.4.
    TransferStarted {
        id: u32,
        ns: String,
        session: String,
        request: String,
    },
    /// `done` of `size` bytes: received, for a download; written by the
    /// phone's writer, for an upload. Added in 0.5.4.
    TransferProgress {
        id: u32,
        done: u64,
        size: u64,
    },
    /// Image N is uploaded: the daemon holds it, and the message can be
    /// sent. Added in 0.5.4.
    ImageReady {
        n: u8,
    },
    /// A file or `git` output arrived, verified, and is in the preview
    /// cache at `path` ([`PreviewCache`](crate::PreviewCache)): `mime` is
    /// `text/markdown`, `text/plain`, `text/x-diff` or an image's, `name`
    /// the path as the daemon resolved it (for `git`, the request as it
    /// normalised it). Also when the cache already held it. Added in 0.5.4.
    FileReady {
        id: u32,
        path: std::path::PathBuf,
        mime: String,
        name: String,
        size: u64,
    },
    /// A transfer ended without its file or upload: the daemon's code
    /// (decision 4.6) and the line to show, `None` for the codes that show
    /// none (`cancelled`). Added in 0.5.4.
    TransferFailed {
        id: u32,
        code: u16,
        line: Option<String>,
    },
    /// The images attached to the message, in number order, whenever one
    /// is attached, starts uploading, is ready, fails or goes. Added in
    /// 0.5.3.
    Attachments(Vec<crate::blob::Attachment>),
    /// What the daemon offers of binary transfers (`hello`'s or
    /// `server.capabilities`' `blob`): given after [`Event::Connected`] (and
    /// in a replay) only when the daemon offers them, so an app takes
    /// [`Event::Connected`] and [`Event::Disconnected`] as `None` until it
    /// comes. The preview's links and the + follow it. Always `Some` from
    /// this client; an `Option` so a later one may withdraw it. Added in
    /// 0.5.4.
    Blob(Option<wire::BlobInfo>),
    /// A choice question's answer ([`Input::Choose`]) came back (plan
    /// 20261029_01, step 5): `outcome` is `answered` (the card goes),
    /// `mismatch` (`got` is what the agent recorded, one line per
    /// question), `no_result` (the keys went, but the agent wrote no
    /// result in time), or `failed`: the daemon refused it, the link
    /// closed, or no answer came in [`CHOICE_ANSWER_WAIT`]; `got` is then
    /// the reason, one line, and `code` the daemon's error code when it
    /// gave one (`CHOICE_STALE`, `CHOICE_UNSUPPORTED`, `SCREEN_MISMATCH`,
    /// `NOT_PERMITTED`, …). Added in 0.5.5.
    ChoiceAnswered {
        id: String,
        /// The pane it was for; empty when no view was open.
        pane_id: String,
        outcome: String,
        got: Vec<String>,
        code: Option<i64>,
    },
}

impl Eq for Event {}

/// Who said a line of [`Event::Conversation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    User,
    Gemini,
}

/// What the driver must do.
// `Event::AgentView` is the large one (its subagents, the subagents plan,
// step 4); boxing it would change the public type.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// Give this event to the app.
    Event(Event),
    /// Send this request to the daemon.
    Send(Request),
    /// Close the connection, for this reason, and reconnect later.
    Close(String),
    /// Remove the active remote `id`: close the connection, delete its
    /// state directory, key and record, and give [`Event::RemoteRemoved`]
    /// with `daemon` (plan 20261028_01, decision 22).
    Remove { id: RemoteId, daemon: bool },
    /// Offline: connect again now, not after the backoff (a refresh).
    Reconnect,
    /// Send this notification to the daemon (`terminal.ack`).
    Notify(Notification),
}

/// Notices held at most, the oldest dropped beyond it (decision 4).
pub const NOTICE_CAP: usize = 8;
/// The daemon's notice kind for typing into a terminal, noted but never
/// said.
const TERMINAL_INPUT_NOTICE: &str = "terminal_input";

/// How long a reply read aloud waits for its spoken summary before it is
/// read as it is (plan 20261023_05, decision 5).
pub const SUMMARY_WAIT: Duration = Duration::from_secs(10);

/// How long the speech held during a voice input waits for its catch-up
/// before it is said as it is (plan 20261023_02, the 2026-10-30 13:52 EDT
/// amendment, item 4).
pub const CATCH_UP_WAIT: Duration = Duration::from_secs(8);

/// How long the link may be quiet before a `ping`, and how long the ping
/// may go unanswered before the link is closed and made again.
pub const PING_IDLE: Duration = Duration::from_secs(30);
/// The ping's quiet spell while the app is in the background: under the
/// usual NAT and carrier idle timeouts, so the one socket survives, but
/// rare enough to let the radio sleep.
pub const PING_IDLE_BACKGROUND: Duration = Duration::from_secs(270);
pub const PING_WAIT: Duration = Duration::from_secs(10);
/// How long the ping sent on coming back to the front may go unanswered:
/// short, because a socket that slept (or was frozen by the system) is
/// often dead, and the view would sit stale until it was found out.
pub const PING_WAIT_RESUME: Duration = Duration::from_secs(3);

/// How long `link.unpair` may go unanswered before the pairing is
/// forgotten anyway.
pub const UNPAIR_WAIT: Duration = Duration::from_secs(5);

/// How long a `choice.answer` may go unanswered before its card is told it
/// failed (plan 20261029_01, step 5: at least 15 s). The daemon checks the
/// screen for up to 1 s before each question and the review, waits 150 ms
/// between keys, and then up to `CHOICE_RESULT_WAIT_S` (5 s) for the
/// agent's result: four questions with Other text come to about 12 s.
pub const CHOICE_ANSWER_WAIT: Duration = Duration::from_secs(20);

/// Whether an utterance answers a confirmation with approval: the whole
/// utterance is "approve" or "approved", in any case, ignoring punctuation
/// and surrounding space (plan 20261022_02, 12:06 EDT amendment).
pub fn is_approval(utterance: &str) -> bool {
    let kept: String = utterance
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    matches!(kept.trim(), "approve" | "approved")
}

/// An utterance as control words are matched (plan 20261023_02, decision
/// 2, as "approve" is): lowercased, everything but letters, digits and
/// spaces dropped, spaces folded.
pub fn control_words(utterance: &str) -> String {
    utterance
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether an utterance starts dictation: "take a message" or "dictate".
pub fn is_dictation_start(utterance: &str) -> bool {
    matches!(
        control_words(utterance).as_str(),
        "take a message" | "dictate"
    )
}

/// A suggested response's handle: `reply`, the agent, its project and the
/// response, as [`message_handle`] builds them.
pub fn response_handle(target: &common::FocusInfo, response: Suggestion) -> String {
    label_handle(target, response.label())
}

/// A tapped answer's handle: `reply`, the agent, its project and the
/// answer's label.
pub fn label_handle(target: &common::FocusInfo, label: &str) -> String {
    let handle = message_handle(target, label);
    format!("reply{}", &handle["message".len()..])
}

/// What a tapped Yes or No says to the agent: the answer with the
/// question's context (owner, 2026-10-23: "the \"yes\" should carry the
/// context handle when typed, either a phrase such as \"Yes, install it on
/// the S23\" … the button should only show \"yes\""). "Install it on the
/// S23?" gives "Yes, install it on the S23" or "No, don't install it on
/// the S23"; "Should I write the amendment now?" gives "Yes, write the
/// amendment now". A question that is not a request ("Is that what you
/// meant?") gives "Yes, re: is that what you meant". Only the message
/// filter's characters are kept.
pub fn answer_text(yes: bool, question: &str) -> String {
    const ASKING: [&str; 9] = [
        "should i ",
        "shall i ",
        "do you want me to ",
        "would you like me to ",
        "want me to ",
        "can i ",
        "may i ",
        "could i ",
        "ok to ",
    ];
    const QUESTION_WORDS: [&str; 16] = [
        "is", "are", "was", "do", "does", "did", "what", "which", "where", "when", "why", "how",
        "who", "will", "would", "could",
    ];
    // The last sentence that asks, without its question mark or marks.
    let asked = question
        .split_inclusive(['?', '.', '!', '\n'])
        .map(str::trim)
        .rfind(|t| t.ends_with('?'))
        .unwrap_or(question.trim());
    let asked: String = asked
        .trim_end_matches('?')
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | ',' | '\'' | ':') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let asked = asked.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = asked.to_ascii_lowercase();
    let (yes_word, no_word) = ("Yes", "No");
    let answer = if yes { yes_word } else { no_word };
    let request = ASKING
        .iter()
        .find_map(|p| lower.strip_prefix(p).map(|_| &asked[p.len()..]))
        .or_else(|| {
            let first = lower.split(' ').next().unwrap_or("");
            (!QUESTION_WORDS.contains(&first) && !asked.is_empty()).then_some(asked.as_str())
        });
    let text = match request {
        Some(rest) => {
            let mut chars = rest.chars();
            let rest = chars
                .next()
                .map(|c| c.to_ascii_lowercase().to_string() + chars.as_str())
                .unwrap_or_default();
            if yes {
                format!("{answer}, {rest}")
            } else {
                format!("{answer}, don't {rest}")
            }
        }
        None if asked.is_empty() => answer.to_string(),
        None => {
            let mut chars = asked.chars();
            let rest = chars
                .next()
                .map(|c| c.to_ascii_lowercase().to_string() + chars.as_str())
                .unwrap_or_default();
            format!("{answer}, re: {rest}")
        }
    };
    text.chars().take(200).collect()
}

/// A dictated message's handle, under plan 20261023_01's rules: `message`,
/// the agent, its project and the message's first two words, lowercased,
/// joined with `-`, as many whole words as fit in 64 characters. The daemon
/// settles it as any other.
pub fn message_handle(target: &common::FocusInfo, text: &str) -> String {
    let mut words: Vec<String> = vec!["message".to_string()];
    let split = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    };
    for part in [target.agent.as_deref(), target.project.as_deref()]
        .into_iter()
        .flatten()
    {
        words.extend(split(part));
    }
    words.extend(split(text).into_iter().take(2));
    let mut out = String::new();
    for w in words {
        let extra = if out.is_empty() { w.len() } else { w.len() + 1 };
        if out.len() + extra > 64 {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w);
    }
    out
}

/// "Stopped the flankr agent.", "Stopped A and B.", "Stopped A, B and C."
pub fn stopped_sentence(names: &[String]) -> String {
    let joined = match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    format!("Stopped {joined}.")
}

/// What a request we sent was, so its response can be read.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sent {
    Hello,
    Turn(u64),
    Interrupt,
    ConfirmReply(u64),
    /// `agents.list`, for herdr's focus.
    List,
    /// `agents.focus`, from "focus <agent>" while dictating.
    Focus(String),
    /// `agents.send` of a dictated message.
    Message,
    /// `session.subscribe` for the agent view of this pane, or of its
    /// subagent.
    Subscribe(String, Option<String>),
    /// `session.unsubscribe`.
    Unsubscribe,
    /// `session.history` for this subscription.
    History(u64),
    /// `agents.send` of a suggested response: its label and its text.
    Respond(String, String),
    /// `choice.answer` of the choice set with this id, for this pane.
    Choose(String, String),
    /// `speech.summarize` of the reply being read with this serial.
    Summarize(u64),
    /// `speech.catch_up` for the held speech with this serial.
    CatchUp(u64),
    /// `client.options`, with read aloud as it was then.
    Options,
    /// `ping`, after a quiet spell.
    Ping,
    /// `link.unpair`.
    Unpair,
    /// `session.peek` of a pane whose turn just stopped on `line`: how long
    /// the turn ran, for another session's finished card.
    Peek {
        pane_id: String,
        line: String,
    },
    /// `session.content` for this subscription (plan 20261024_01): the
    /// slots asked for, as (index, serial, rev) when asked, under the
    /// view's outline generation `generation`.
    Content {
        subscription: u64,
        generation: u64,
        asked: Vec<(u64, common::Serial, u32)>,
        slots_only: bool,
        /// A refresh's fetch of the newest screen.
        refresh: bool,
    },
    /// `terminal.open` of this session.
    TerminalOpen(String),
    /// `terminal.close`.
    TerminalClose,
    /// `terminal.input_arm` of this session.
    TerminalArm(String),
    /// `terminal.input` of this session.
    TerminalInput(String),
    /// `terminal.scroll` of this session.
    TerminalScroll(String),
    /// `terminal.resize` of this session.
    TerminalResize(String),
    /// `terminal.mouse` of this session.
    TerminalMouse(String),
    /// `plugin.call` to `ns`, with the device's `id` if it has one.
    PluginCall {
        ns: String,
        id: Option<u64>,
    },
    /// `plugin.set_enabled` of `ns`.
    PluginSetEnabled {
        ns: String,
    },
    /// `terminal.paste` into this session's terminal, of these images.
    TerminalPaste {
        images: Vec<u8>,
    },
}

/// The most `session.content` requests in flight at once.
pub const CONTENT_IN_FLIGHT: usize = 2;
/// The most indexes one `session.content` range spans.
pub const CONTENT_RANGE: u64 = common::SECTIONS_MAX as u64;
/// Indexes needing contents this close together are asked for in one
/// range, with those held between them.
const CONTENT_MERGE_GAP: u64 = 16;
/// The indexes whose contents are fetched before the app gives a window:
/// the newest, as at the bottom.
const DEFAULT_NEAR: u64 = common::CONTENT_LIMIT_DEFAULT as u64;
/// How long an outline's first hole must stay unfilled before it is asked
/// for with `slots_only`: the chunks of an outline still being sent are
/// not asked for again.
pub const GAP_WAIT: Duration = Duration::from_secs(1);

/// A new outline that came while the view was scrolled back, held aside
/// with the contents that came with it (CHANGES.md, 2026-10-24 00:36 EDT,
/// items 3 and 4).
#[derive(Debug, Default)]
struct HeldOutline {
    outline: Outline,
    sections: Sections,
}

/// A reply to read aloud, in its turn (plan 20261023_05, decisions 5 and
/// 6, as amended at 15:45 and 16:28 EDT): the daemon is asked how to say
/// it, and later replies wait for it, up to `SUMMARY_WAIT`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reading {
    serial: u64,
    /// The reply's time, which names it.
    ts: String,
    /// The reply as the view has it, read through `flankr_speech` when the
    /// daemon does not say how to read it.
    text: String,
    /// When the summary was asked for.
    asked_at: Duration,
    /// The reply's section, whose line a Finished card with the spoken
    /// text shares (CHANGES.md, 2026-10-24 12:06 EDT: read aloud uses
    /// `CardBody.spoken`).
    section: Option<common::Serial>,
    /// What to say, once known.
    ready: Option<String>,
    /// The pane it came from, and its subagent if any: for a catch-up.
    pane_id: Option<String>,
    child: Option<String>,
}

/// Speech held in the notice queue: its text, and for a catch-up where it
/// came from (plan 20261023_02, the 2026-10-30 13:52 EDT amendment).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    text: String,
    /// The pane of a notice or a reply.
    pane_id: Option<String>,
    /// A reply's entry `ts`, when the daemon can read it from the pane's
    /// own session (not a subagent's).
    ts: Option<String>,
}

impl Held {
    /// Speech from no pane: a Gemini line, a "stopped" sentence, a
    /// catch-up.
    fn text(text: String) -> Self {
        Held {
            text,
            pane_id: None,
            ts: None,
        }
    }
}

/// A `speech.catch_up` on its way, and the held items it is about.
#[derive(Debug)]
struct CatchingUp {
    serial: u64,
    held: Vec<Held>,
    asked_at: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    /// No connection.
    Offline,
    /// Connected, `hello` not yet answered.
    Greeting,
    /// `hello` answered.
    Online,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Turn {
    id: u64,
    /// The user tapped during it.
    tapped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Confirmation {
    id: u64,
    /// A handle only, never shown: which agent view it belongs to.
    pane_id: String,
    text: String,
    label: Option<String>,
    phrase: String,
    command: String,
    agent: String,
    project: Option<String>,
}

/// What [`Input::Speaking`] means for a "read it back".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadBack {
    No,
    /// Said; the speech has not started.
    Asked,
    Speaking,
}

/// The dictation mode (plan 20261023_02).
#[derive(Debug, Clone)]
struct Dictating {
    mode: PromptMode,
    parts: Vec<String>,
    target: Option<common::FocusInfo>,
    /// The target is the focus as last known; the `agents.list` asked for
    /// at the start will fix it.
    refresh_target: bool,
    phase: DictationPhase,
    /// The own message's confirmation, once `agents.send` answered.
    confirm_id: Option<u64>,
    /// Submitted: the text was shown for review first, so its own
    /// confirmation is approved without asking (plan 20261023_02, the
    /// amendment on button submits; extended by the owner on 2026-10-23 to
    /// every submit of reviewed text: "a separate approval isn't needed for
    /// UI entered buttons or when text has been buffered for review
    /// first").
    own_approval: bool,
    /// Since when the gate has been closed, for the silence timer. `None`
    /// while it is open, and after a refused or denied submit until the
    /// next change.
    quiet_from: Option<Duration>,
    read_back: ReadBack,
    /// Started from the agent view of this pane: the target is that agent,
    /// whatever the focus does.
    fixed_pane: Option<String>,
    /// A Submit (by the button when `true`) waiting for speech that STT has
    /// not given back yet.
    submit_waiting: Option<bool>,
    /// Whether this prompt goes to the general agent (Gemini).
    general_agent: bool,
}

/// The waiting Submit, if any.
fn d_waiting(d: &Option<Dictating>) -> Option<bool> {
    d.as_ref().and_then(|d| d.submit_waiting)
}

impl Dictating {
    fn text(&self) -> String {
        self.parts.join(" ")
    }

    /// A target that can be sent to: a pane with an agent.
    fn has_target(&self) -> bool {
        self.target.as_ref().is_some_and(|t| t.agent.is_some())
    }
}

/// The client's state (decision 3). The plan's states map onto it as:
/// - `Idle`: online, no turn, no confirmation;
/// - `Waiting`: a turn is running and no confirmation is pending;
/// - `Confirming`: a confirmation is pending. It takes precedence over a
///   running turn, so the user can always answer the question just spoken.
///
/// `Unpaired` is the `net` thread's: without a paired server it uses the
/// daemon's local socket, and pairs when given [`Input::Pair`].
#[derive(Debug)]
pub struct State {
    client: String,
    link: Link,
    session_id: Option<String>,
    next_request: i64,
    next_turn: u64,
    sent: HashMap<Id, Sent>,
    turn: Option<Turn>,
    /// Pending confirmations, the one being asked first.
    confirms: VecDeque<Confirmation>,
    /// A denying utterance to start as a turn once the running one ends.
    deferred: Option<String>,
    speaking: bool,
    listening: bool,
    /// Voice heard right now, and whether the front end reports it at all.
    voice: bool,
    voice_reported: bool,
    /// When the voice last ended with no utterance since: STT may still be
    /// working on it.
    voice_ended: Option<Duration>,
    /// The play button: the agent view's new replies are read aloud.
    read_aloud: bool,
    /// The app is in the background: the link sleeps.
    background: bool,
    /// When anything last came from the daemon, and when a `ping` went
    /// unanswered so far.
    last_heard: Duration,
    ping_sent: Option<Duration>,
    /// How long the ping in `ping_sent` may go unanswered.
    ping_wait: Duration,
    /// Replies to read aloud, oldest first, each waiting for its summary
    /// or ready.
    reading: VecDeque<Reading>,
    next_reading: u64,
    /// The replies moved to the notices, by time, and those given to be
    /// said since the speaker was last quiet: so answering a question can
    /// skip its reading.
    read_replies: VecDeque<(String, String)>,
    saying: Vec<String>,
    /// Suggestions tapped and sent, until their confirmation is approved.
    tapped: Vec<Tapped>,
    notices: VecDeque<Held>,
    /// Voice input was in progress when the notices were last flushed: its
    /// end asks for a catch-up (plan 20261023_02, the 2026-10-30 13:52 EDT
    /// amendment).
    voice_was: bool,
    /// What the voice input that ended sent: the message and its pane, or
    /// Gemini's prompt.
    voice_said: Option<common::SaidInput>,
    /// The catch-up asked for, until answered or `CATCH_UP_WAIT`.
    catching_up: Option<CatchingUp>,
    next_catch_up: u64,
    /// The driver's clock, as last ticked.
    now: Duration,
    /// The phone's local time of day, as the driver last gave it: clock
    /// times in a reply read here are read relative to it.
    local_time: LocalTime,
    /// herdr's focus, as last learned.
    focus: Option<common::FocusInfo>,
    dictation: Option<Dictating>,
    /// The silence that submits a message; at least [`DICTATION_SILENCE`].
    silence: Duration,
    /// The dictation card as last given.
    shown: Dictation,
    /// The agents as `agents.list` last gave them.
    agents: Vec<common::AgentInfo>,
    /// `agents.list` has answered on this connection.
    listed: bool,
    /// A list the daemon gave that is new, not yet taken to be kept
    /// ([`State::take_new_list`]).
    new_list: bool,
    /// An `agents.list` is on its way.
    listing: bool,
    /// When the last `agents.list` was sent.
    listed_at: Option<Duration>,
    /// A list is asked for as soon as `LIST_MIN_INTERVAL` allows.
    list_wanted: bool,
    /// The pending panes as last given.
    pending_shown: Vec<String>,
    /// The open agent view.
    view: Option<AgentView>,
    /// Follow herdr's focus with the open view (off by default).
    follow: bool,
    /// The settings screen's toggles (plan 20261024_02).
    settings: Settings,
    /// What the daemon's `[llm]` makes available, as last told; both, from
    /// a daemon that does not say (plan 20261027_02, decision 7).
    llm: wire::LlmInfo,
    /// A `link.unpair` for the active remote is on its way, since this
    /// time.
    unpairing: Option<(Duration, RemoteId)>,
    /// A `choice.answer` on its way: its request id, its choice set's id,
    /// its pane and when it was sent ([`CHOICE_ANSWER_WAIT`]).
    choosing: Option<(Id, String, String, Duration)>,
    /// Panes whose finished turn the agent view has dinged: the list's
    /// next stopped turn for them is the same one, and is not dinged again.
    dinged: HashSet<String>,
    /// The closed views kept in memory, the least recently used first, at
    /// most [`CACHED_VIEWS`] (plan 20261022_03, the L3 amendments of
    /// 2026-10-24 12:12 and 12:15 EDT).
    closed: VecDeque<AgentView>,
    /// Their size as last given ([`Event::Cache`]).
    cache_shown: u64,
    /// A refresh running ([`Input::Refresh`]).
    refresh: Option<Refresh>,
    /// Other sessions' cards, and the interactions they follow.
    cross: crate::cross::Cross,
    /// The terminals, shared with the app (plan 20261027_07, step 4).
    terminals: Arc<Terminals>,
    /// The drawer is shown ([`Input::DrawerShown`]).
    drawer: bool,
    /// The server's name, vocabulary and terminal mode from the last
    /// `hello`, and the last retry delay: given again by
    /// [`State::replay`].
    server_name: String,
    vocabulary: Vec<String>,
    retry_in_ms: u32,
    /// The daemon's plugins, as last told (plan 20261028_04).
    plugins: Vec<wire::PluginInfo>,
    /// The plugin calls with an `id` sent on this connection whose
    /// `plugin.result` has not come, as `(ns, id)`.
    plugin_waiting: Vec<(String, u64)>,
    /// Binary transfers (plan 20261028_05, step 7).
    blobs: crate::blob::Blobs,
}

/// A refresh running (plan 20261022_03, the L3 amendment of 2026-10-24
/// 12:22 EDT): the serials the view held when it began, and which of its
/// resubscribe and its fetch have answered.
#[derive(Debug)]
struct Refresh {
    before: HashSet<u64>,
    subscribed: bool,
    fetched: bool,
    since: Duration,
}

/// The open agent view (plan 20261022_03, step 4), fed by its session
/// subscription (plan 20261023_03, section 6, A7).
#[derive(Debug)]
struct AgentView {
    pane_id: String,
    /// The subagent of the pane shown instead of its main session (the
    /// subagents plan, step 4).
    child: Option<String>,
    status: String,
    /// Every section given, by serial, for the life of the view.
    sections: Sections,
    /// The entries and cards of `sections`, flattened as last changed.
    entries: Vec<common::Entry>,
    pending_question: Option<common::PendingQuestion>,
    /// The subscription, once `session.subscribe` answered on this
    /// connection.
    subscription: Option<u64>,
    /// A `session.subscribe` is on its way.
    subscribing: bool,
    /// A `session.history` is on its way.
    history: bool,
    /// A history page reached the session's first line.
    start: bool,
    /// The session has answered at least once.
    answered: bool,
    /// The replies (by time) already in the view when they were seen:
    /// only replies that arrive later, live, are read aloud.
    seen_replies: std::collections::HashSet<String>,
    /// The time of the pending question a tapped Yes or No answered: its
    /// card goes at once, before the session shows the answer.
    replied_to: Option<String>,
    /// The response sent, until the session shows it or the agent works.
    typed: Option<String>,
    /// The event as last given.
    shown: Option<Event>,
    /// The daemon sends outlines (its deliveries carry an epoch): plan
    /// 20261024_01. Without, the view is as before: sections and history
    /// pages.
    outline_mode: bool,
    /// The outline shown, by index; contents by serial in `sections`.
    outline: Outline,
    /// A new outline held aside while scrolled back.
    incoming: Option<HeldOutline>,
    /// The jump line was tapped with `incoming` held: its sections newer
    /// than the shown ribbon's newest show below the ribbon's end until
    /// it is applied at the bottom.
    appending: bool,
    /// Contents with no slot in the outline are to be dropped once it is
    /// complete (a reset, merged by serial).
    orphans: bool,
    /// The app's window, as last given.
    window: Option<ViewWindow>,
    /// The indexes whose contents are being fetched.
    asking: BTreeSet<u64>,
    /// `session.content` requests in flight.
    in_flight: usize,
    /// A `slots_only` request for a hole is in flight.
    gap_asked: bool,
    /// The building outline's first hole as last seen, and since when.
    hole: (u64, Duration),
    /// Contents asked for and not given, by packed serial, at their rev:
    /// not asked again until the rev changes.
    unavailable: HashMap<u64, u32>,
    /// Bumped with each new outline: a `slots_only` answer for an older
    /// one is dropped.
    generation: u64,
    /// The live Finished card as last shown, by serial.
    finished: Option<common::Serial>,
    /// The agent has worked since the view opened, or since its last
    /// ding: a Finished card shown after that is new (plan 20261024_02).
    /// The one shown on opening is not.
    worked: bool,
    /// Reopened from the cache: the next delivery is the backfill of what
    /// happened while it was closed, and none of its replies is read.
    backfill: bool,
    /// The memory pressure the view is under, until the app is back in
    /// front: it keeps and fetches contents only that close to the window.
    pressure: Option<Pressure>,
}

impl AgentView {
    fn new(pane_id: String, child: Option<String>, status: String) -> Self {
        AgentView {
            pane_id,
            child,
            status,
            sections: Sections::default(),
            entries: Vec::new(),
            pending_question: None,
            subscription: None,
            subscribing: false,
            history: false,
            start: false,
            answered: false,
            seen_replies: Default::default(),
            replied_to: None,
            typed: None,
            shown: None,
            outline_mode: false,
            outline: Outline::default(),
            incoming: None,
            appending: false,
            orphans: false,
            window: None,
            asking: BTreeSet::new(),
            in_flight: 0,
            gap_asked: false,
            hole: (0, Duration::ZERO),
            unavailable: HashMap::new(),
            generation: 0,
            finished: None,
            worked: false,
            backfill: false,
            pressure: None,
        }
    }

    /// The outline being built, and the contents it takes: the one held
    /// aside, else the one shown.
    fn building(&mut self) -> (&mut Outline, &mut Sections) {
        match self.incoming.as_mut() {
            Some(i) => (&mut i.outline, &mut i.sections),
            None => (&mut self.outline, &mut self.sections),
        }
    }

    fn scrolled_back(&self) -> bool {
        self.window.is_some_and(|w| w.scrolled_back)
    }

    /// Shows the outline held aside, its contents merged in by serial.
    fn apply_incoming(&mut self) -> bool {
        self.appending = false;
        let Some(incoming) = self.incoming.take() else {
            return false;
        };
        log::info!(
            "view: the new outline applied, {} slot(s)",
            incoming.outline.len()
        );
        let other_file = matches!(
            (incoming.outline.file(), self.sections.file()),
            (Some(a), Some(b)) if a != b
        );
        if other_file {
            self.sections = incoming.sections;
        } else {
            self.sections.merge(incoming.sections);
        }
        self.outline = incoming.outline;
        self.orphans = true;
        self.drop_orphans();
        true
    }

    /// Once the outline shown is complete after a reset, the contents with
    /// no slot in it go.
    fn drop_orphans(&mut self) {
        if !self.orphans || !self.outline.complete() {
            return;
        }
        self.orphans = false;
        let outline = &self.outline;
        self.sections
            .retain(|s| outline.slot_of(s.serial).is_some());
    }

    /// Forgets contents outside the window's `keep` range, except the
    /// newest `LIVE_INLINE` indexes' and live cards' (the jump line and
    /// the pin need them). The outline is always kept.
    ///
    /// Under memory pressure (the 12:21 EDT amendment) the range is the
    /// grade's, and the newest are kept only within it.
    fn forget(&mut self) {
        let Some(w) = self.window.filter(|_| self.outline_mode) else {
            return;
        };
        let (lo, hi) = match self.pressure {
            None => w.keep,
            Some(Pressure::UiHidden) => w.near,
            Some(Pressure::Background) => w.close,
            Some(Pressure::Low) => w.shown,
        };
        let newest = match self.pressure {
            Some(_) => u64::MAX,
            None => self
                .outline
                .last()
                .map_or(0, |l| l.saturating_sub(common::LIVE_INLINE - 1)),
        };
        let outline = &self.outline;
        let before = self.sections.len();
        self.sections.retain(|s| {
            let live_card = s.card.as_ref().is_some_and(|c| c.live);
            match outline.slot_of(s.serial) {
                Some(slot) => (lo..=hi).contains(&slot.index) || slot.index >= newest || live_card,
                // No slot: an orphan, dropped once the outline is complete.
                None => true,
            }
        });
        let forgotten = before - self.sections.len();
        if forgotten > 0 {
            log::debug!("view: {forgotten} far section(s) forgotten");
        }
    }

    /// The indexes whose contents are fetched around the window `w`: those
    /// within `KEEP_SCREENS`, or fewer under memory pressure.
    fn fetch_span(&self, w: ViewWindow) -> (u64, u64) {
        match self.pressure {
            None | Some(Pressure::UiHidden) => w.near,
            Some(Pressure::Background) => w.close,
            Some(Pressure::Low) => w.shown,
        }
    }

    /// Closed, to be kept in the cache: it has no subscription, nothing in
    /// flight and no window, and an outline held aside is shown, as it
    /// reopens at the bottom (the 12:12 EDT amendment, items 1 and 3).
    fn park(&mut self) {
        self.subscription = None;
        self.subscribing = false;
        self.history = false;
        self.in_flight = 0;
        self.asking.clear();
        self.gap_asked = false;
        self.window = None;
        self.shown = None;
        self.typed = None;
        self.replied_to = None;
        self.pressure = None;
        if self.apply_incoming() {
            self.reflatten();
        }
        self.see_all();
    }

    /// Roughly how many bytes the view holds: its sections, twice (the map
    /// and the flattened entries), and its outline.
    fn bytes(&self) -> u64 {
        fn entry(e: &common::Entry) -> usize {
            64 + e.ts.len()
                + e.summary.len()
                + e.text.as_ref().map_or(0, String::len)
                + e.tool.as_ref().map_or(0, String::len)
                + e.links
                    .iter()
                    .map(|l| l.text.len() + l.url.len())
                    .sum::<usize>()
        }
        let sections: usize = self
            .sections
            .iter()
            .map(|s| {
                let card = s.card.as_ref().map_or(0, |c| {
                    c.title.len()
                        + c.text.len()
                        + c.points.iter().map(String::len).sum::<usize>()
                        + c.spoken.as_ref().map_or(0, String::len)
                });
                64 + card + s.entries.iter().map(entry).sum::<usize>()
            })
            .sum();
        let flattened: usize = self.entries.iter().map(entry).sum();
        let outline = self.outline.len() * (std::mem::size_of::<crate::view::Slot>() + 32);
        (sections + flattened + outline) as u64
    }

    /// The first hole of the outline being built moved: note when.
    fn note_hole(&mut self, now: Duration) {
        let at = match &self.incoming {
            Some(i) => i.outline.contiguous(),
            None => self.outline.contiguous(),
        };
        if at != self.hole.0 {
            self.hole = (at, now);
        }
    }

    /// Re-flatten the entries after the sections changed.
    fn reflatten(&mut self) {
        self.entries = self.sections.flatten(|_| false).0;
    }

    /// Every reply now held counts as seen.
    fn see_all(&mut self) {
        for e in self.entries.iter().filter(|e| is_reply(e)) {
            self.seen_replies.insert(e.ts.clone());
        }
    }
}

/// The serials of `now` whose sections differ from `before`'s, and those of
/// `before` that `now` no longer has, in serial order. The lists may be in
/// any order (in outline mode they are in index order, which a late card
/// can take out of serial order).
fn changed_serials(before: &[ViewSection], now: &[ViewSection]) -> Vec<common::Serial> {
    let was: HashMap<u64, &ViewSection> = before.iter().map(|v| (v.serial.to_u64(), v)).collect();
    let mut changed: Vec<common::Serial> = now
        .iter()
        // A new index alone (sections paged in before it) is not a
        // change: the block is as it was.
        .filter(|v| {
            was.get(&v.serial.to_u64())
                .is_none_or(|b| b.shows != v.shows)
        })
        .map(|v| v.serial)
        .collect();
    let is: std::collections::HashSet<u64> = now.iter().map(|v| v.serial.to_u64()).collect();
    changed.extend(
        before
            .iter()
            .filter(|v| !is.contains(&v.serial.to_u64()))
            .map(|v| v.serial),
    );
    changed.sort_by_key(|s| s.to_u64());
    changed
}

/// Whether two agent views show the same, whatever their `changed`.
fn same_view(a: &Event, b: &Event) -> bool {
    match (a, b) {
        (
            Event::AgentView {
                name,
                status,
                sections,
                pinned,
                changed: _,
                loading,
                typed,
                loading_earlier,
                start,
                terminal_on,
                new_content,
                held_end,
                held_newer,
                bookends,
                child,
                children,
            },
            Event::AgentView {
                name: name_b,
                status: status_b,
                sections: sections_b,
                pinned: pinned_b,
                changed: _,
                loading: loading_b,
                typed: typed_b,
                loading_earlier: loading_earlier_b,
                start: start_b,
                terminal_on: terminal_on_b,
                new_content: new_content_b,
                held_end: held_end_b,
                held_newer: held_newer_b,
                bookends: bookends_b,
                child: child_b,
                children: children_b,
            },
        ) => {
            name == name_b
                && status == status_b
                && pinned == pinned_b
                && loading == loading_b
                && typed == typed_b
                && loading_earlier == loading_earlier_b
                && start == start_b
                && terminal_on == terminal_on_b
                && new_content == new_content_b
                && held_end == held_end_b
                && held_newer == held_newer_b
                && bookends == bookends_b
                && child == child_b
                && children == children_b
                && sections == sections_b
        }
        _ => a == b,
    }
}

impl State {
    /// A client that calls itself `client` in `hello`.
    pub fn new(client: impl Into<String>) -> Self {
        Self::with_terminals(client, Arc::default())
    }

    /// As [`new`](Self::new), keeping its terminals in `terminals`, which
    /// the app reads from its own thread.
    pub fn with_terminals(client: impl Into<String>, terminals: Arc<Terminals>) -> Self {
        State {
            terminals,
            client: client.into(),
            link: Link::Offline,
            session_id: None,
            next_request: 1,
            next_turn: 1,
            sent: HashMap::new(),
            turn: None,
            confirms: VecDeque::new(),
            deferred: None,
            speaking: false,
            listening: false,
            voice: false,
            voice_reported: false,
            voice_ended: None,
            read_aloud: false,
            background: false,
            last_heard: Duration::ZERO,
            ping_sent: None,
            ping_wait: PING_WAIT,
            reading: VecDeque::new(),
            next_reading: 1,
            read_replies: VecDeque::new(),
            saying: Vec::new(),
            tapped: Vec::new(),
            notices: VecDeque::new(),
            voice_was: false,
            voice_said: None,
            catching_up: None,
            next_catch_up: 1,
            now: Duration::ZERO,
            local_time: LocalTime::default(),
            focus: None,
            dictation: None,
            silence: DICTATION_SILENCE,
            shown: Dictation::off(),
            agents: Vec::new(),
            listed: false,
            new_list: false,
            listing: false,
            listed_at: None,
            list_wanted: false,
            pending_shown: Vec::new(),
            view: None,
            follow: false,
            settings: Settings::default(),
            llm: wire::LlmInfo::available(),
            unpairing: None,
            choosing: None,
            dinged: HashSet::new(),
            cross: crate::cross::Cross::default(),
            closed: VecDeque::new(),
            cache_shown: 0,
            refresh: None,
            drawer: false,
            server_name: String::new(),
            vocabulary: Vec::new(),
            retry_in_ms: 0,
            plugins: Vec::new(),
            plugin_waiting: Vec::new(),
            blobs: crate::blob::Blobs::new(),
        }
    }

    /// Keeps the files fetched over the link in `cache_dir`'s
    /// `libmobile-share/` ([`PreviewCache`](crate::PreviewCache)). Without
    /// it, the system's temp folder's `flankr-companion/`. Added in 0.5.4.
    pub fn set_preview_cache(&mut self, cache_dir: std::path::PathBuf) {
        self.blobs
            .set_cache(crate::blob::PreviewCache::new(&cache_dir));
    }

    /// What the daemon offers of binary transfers on this connection:
    /// `None` offline, before `hello`, and from a daemon (or a local
    /// socket) without them. Added in 0.5.4.
    pub fn blob(&self) -> Option<&wire::BlobInfo> {
        self.blobs.info()
    }

    /// The images attached to the message, as [`Event::Attachments`]
    /// gives them. Added in 0.5.4.
    pub fn attachments(&self) -> Vec<crate::blob::Attachment> {
        self.blobs.attachments()
    }

    /// A message of the link's binary kind (kind 2) came. One whose header
    /// cannot be read closes the connection (plan 20261028_05, decision
    /// 4.1). Added in 0.5.4.
    pub fn binary(&mut self, bytes: &[u8]) -> Vec<Output> {
        let mut out = Vec::new();
        self.last_heard = self.now;
        self.ping_sent = None;
        if let Err(why) = self.blobs.on_binary(bytes, self.now, &mut out) {
            log::warn!("closing the connection: {why}");
            out.push(Output::Close(why));
        }
        out
    }

    /// The writer wrote one of `transfer`'s CHUNKs: its progress, and the
    /// next CHUNK queued (at most two at a time, decision 4.5). Added in
    /// 0.5.3.
    pub fn binary_written(&mut self, transfer: u32) -> Vec<Output> {
        let mut out = Vec::new();
        self.blobs.written(transfer, self.now, &mut out);
        out
    }

    /// The kind-2 messages to send, in order, each with `Some(transfer)`
    /// when it is a CHUNK for the writer's bulk queue (decision 4.5). The
    /// driver takes them after carrying out each batch of outputs. Added
    /// in 0.5.3.
    pub fn take_binary(&mut self) -> Vec<(Vec<u8>, Option<u32>)> {
        self.blobs.take()
    }

    /// Sets the phone's local time of day, for reading clock times in a
    /// reply as relative when the daemon does not say how to read it.
    /// The driver gives it with every [`Input::Tick`].
    pub fn set_local_time(&mut self, now: LocalTime) {
        self.local_time = now;
    }

    /// Sets the silence that submits a dictated message, raised to
    /// [`DICTATION_SILENCE`] if shorter.
    pub fn set_dictation_silence(&mut self, silence: Duration) {
        self.silence = silence.max(DICTATION_SILENCE);
    }

    /// The silence that submits a dictated message.
    pub fn dictation_silence(&self) -> Duration {
        self.silence
    }

    /// Whether `hello` has been answered on the current connection.
    pub fn online(&self) -> bool {
        self.link == Link::Online
    }

    /// The session to continue on the next connection.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Shows `agents`, a list kept from an earlier run, until the daemon
    /// gives one (plan 20261028_03, the 2026-10-30 13:15 EDT amendment):
    /// `Event::Agents` with it, or nothing once the daemon has listed.
    pub fn restore_agents(&mut self, agents: Vec<common::AgentInfo>) -> Vec<Output> {
        if self.listed {
            return Vec::new();
        }
        self.agents = agents;
        vec![event(Event::Agents(self.agents.clone()))]
    }

    /// The agent list, once, after the daemon gives one that differs from
    /// what showed: the list to keep.
    pub fn take_new_list(&mut self) -> Option<Vec<common::AgentInfo>> {
        std::mem::take(&mut self.new_list).then(|| self.agents.clone())
    }

    /// Sets the session to continue, such as one stored from an earlier run.
    pub fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    /// Notices waiting to be said, with those a catch-up is being made
    /// for.
    pub fn held_notices(&self) -> usize {
        self.notices.len() + self.catching_up.as_ref().map_or(0, |c| c.held.len())
    }

    /// The `hello` a pairing probe sends to a remote beside the open
    /// connection: this client's, with no session to continue, and the
    /// state left as it was (plan 20261028_01, decision 10).
    pub fn hello_request(&self) -> Request {
        let params = wire::HelloParams {
            protocol: protocols::COMPANION.to_string(),
            version: wire::VERSION,
            client: self.client.clone(),
            session_id: None,
        };
        Request::new(Id::Number(1), methods::HELLO, params)
    }

    /// A connection was made: say `hello`.
    pub fn connected(&mut self) -> Vec<Output> {
        self.link = Link::Greeting;
        self.sent.clear();
        self.blobs.connected();
        self.last_heard = self.now;
        self.ping_sent = None;
        let params = wire::HelloParams {
            protocol: protocols::COMPANION.to_string(),
            version: wire::VERSION,
            client: self.client.clone(),
            session_id: self.session_id.clone(),
        };
        let mut out = Vec::new();
        // A new connection answers nothing of the last one's.
        if let Some((_, id, pane_id, _)) = self.choosing.take() {
            out.push(event(choice_failed(
                id,
                pane_id,
                "the connection closed".to_string(),
                None,
            )));
        }
        out.push(self.request(methods::HELLO, params, Sent::Hello));
        out
    }

    /// The terminals, as the app reads them.
    pub fn terminals(&self) -> &Arc<Terminals> {
        &self.terminals
    }

    /// What the app shows of this daemon, as it stands, for when its
    /// remote comes into use (plan 20261028_03, the 18:51 EDT amendment):
    /// the connection (`Connected`, `Vocabulary` and `Llm` online, else
    /// `Disconnected` with the last retry delay), then the agents, the
    /// pending panes, herdr's focus, the other sessions'
    /// cards, the dictation card, the closed views' size and the plugins,
    /// each given even when empty so the app drops the last remote's.
    pub fn replay(&self) -> Vec<Event> {
        let mut events = Vec::new();
        if self.online() {
            events.push(Event::Connected {
                server: self.server_name.clone(),
            });
            events.push(Event::Vocabulary(self.vocabulary.clone()));
            events.push(Event::Llm(self.llm.clone()));
        } else {
            events.push(Event::Disconnected {
                retry_in_ms: self.retry_in_ms,
            });
        }
        events.push(Event::Agents(self.agents.clone()));
        events.push(Event::PendingPanes(self.pending_panes()));
        events.push(Event::Focus(self.focus.clone()));
        events.push(Event::SessionCards(self.cross.shown().to_vec()));
        events.push(Event::Dictation(self.shown.clone()));
        events.push(Event::Cache {
            bytes: self.cache_bytes(),
        });
        if let Some(blob) = self.blobs.info() {
            events.push(Event::Blob(Some(blob.clone())));
        }
        // Last, so the app's plugin views follow this remote's list.
        events.push(Event::Plugins(self.plugins.clone()));
        events
    }

    /// This daemon's remote goes out of use (plan 20261028_03, 3.7 item 28
    /// and the 18:51 EDT amendment): its agent view is closed (kept in the
    /// cache, as Back keeps it), a prompt for one of its panes is dropped
    /// (one for the general agent stays), the drawer is no longer its, and
    /// its terminals are closed on the daemon. The link stays; the caller
    /// puts it in the background ([`Input::Background`]).
    pub fn leave_foreground(&mut self) -> Vec<Output> {
        let mut out = Vec::new();
        self.close_view(&mut out);
        if self.dictation.as_ref().is_some_and(|d| !d.general_agent) {
            self.clear_dictation(&mut out);
        }
        self.drawer = false;
        let acts = self.terminals.update(|b| b.close_all());
        self.terminal_acts(acts, &mut out);
        out
    }

    /// Trades the terminals of `a` and `b`, contents and all, so that the
    /// [`Terminals`] the app reads stays the same object while it moves to
    /// the remote coming into use: each state keeps its own daemon's
    /// terminal mode and terminals in what it then holds.
    pub fn trade_terminals(a: &mut State, b: &mut State) {
        a.terminals.swap_with(&b.terminals);
        std::mem::swap(&mut a.terminals, &mut b.terminals);
    }

    /// A message of the link's terminal kind came: a frame, shown when it
    /// can be and then acknowledged. Bytes that are not a frame are logged
    /// and dropped.
    pub fn terminal_frame(&mut self, bytes: &[u8]) -> Vec<Output> {
        let mut out = Vec::new();
        self.last_heard = self.now;
        self.ping_sent = None;
        match libflankr_term::Frame::decode(bytes) {
            Ok(frame) => {
                let acts = self.terminals.update(|b| b.frame(frame));
                self.terminal_acts(acts, &mut out);
            }
            Err(e) => log::warn!("a terminal frame from the daemon does not decode: {e}"),
        }
        out
    }

    /// Carries out what the terminals asked for.
    fn terminal_acts(&mut self, acts: Vec<Act>, out: &mut Vec<Output>) {
        for act in acts {
            match act {
                Act::Open {
                    session,
                    cols_hint,
                    rows_hint,
                } => {
                    let params = common::TerminalOpenParams {
                        pane_id: session.clone(),
                        cols_hint,
                        rows_hint,
                        // libflankr_term 0.1.1 decodes the trailer.
                        cursor: true,
                    };
                    out.push(self.request(
                        methods::TERMINAL_OPEN,
                        params,
                        Sent::TerminalOpen(session),
                    ));
                }
                Act::Close { terminal_id } => out.push(self.request(
                    methods::TERMINAL_CLOSE,
                    common::TerminalCloseParams { terminal_id },
                    Sent::TerminalClose,
                )),
                Act::Ack { terminal_id, seq } => out.push(Output::Notify(Notification::new(
                    methods::TERMINAL_ACK,
                    common::TerminalAckParams { terminal_id, seq },
                ))),
                Act::Arm {
                    session,
                    terminal_id,
                } => out.push(self.request(
                    methods::TERMINAL_INPUT_ARM,
                    common::TerminalInputArmParams { terminal_id },
                    Sent::TerminalArm(session),
                )),
                Act::Keys {
                    session,
                    terminal_id,
                    keys,
                } => out.push(self.request(
                    methods::TERMINAL_INPUT,
                    common::TerminalInputParams { terminal_id, keys },
                    Sent::TerminalInput(session),
                )),
                Act::Scroll {
                    session,
                    terminal_id,
                    lines,
                } => out.push(self.request(
                    methods::TERMINAL_SCROLL,
                    common::TerminalScrollParams { terminal_id, lines },
                    Sent::TerminalScroll(session),
                )),
                Act::Resize {
                    session,
                    terminal_id,
                    cols,
                    rows,
                } => out.push(self.request(
                    methods::TERMINAL_RESIZE,
                    common::TerminalResizeParams {
                        terminal_id,
                        cols,
                        rows,
                    },
                    Sent::TerminalResize(session),
                )),
                Act::Mouse {
                    session,
                    terminal_id,
                    event: e,
                } => out.push(self.request(
                    methods::TERMINAL_MOUSE,
                    common::TerminalMouseParams {
                        terminal_id,
                        kind: e.kind,
                        col: e.col,
                        row: e.row,
                        modifiers: e.modifiers,
                    },
                    Sent::TerminalMouse(session),
                )),
                Act::Error(message) => out.push(event(Event::Error(message))),
            }
        }
    }

    /// The settings as last given.
    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// What the daemon's `[llm]` makes available, as last told.
    pub fn llm(&self) -> &wire::LlmInfo {
        &self.llm
    }

    /// The daemon's plugins, as last told: empty before the first `hello`
    /// and from a daemon without plugins (plan 20261028_04, 3.10).
    pub fn plugins(&self) -> &[wire::PluginInfo] {
        &self.plugins
    }

    /// Whether a general agent turn may start: the user's toggle is on and
    /// the daemon offers one (plan 20261027_02, decisions 8 and 9).
    pub fn general_agent(&self) -> bool {
        self.settings.general_agent && self.llm.general_agent
    }

    /// The daemon said what `[llm]` makes available.
    fn set_llm(&mut self, llm: wire::LlmInfo, out: &mut Vec<Output>) {
        if self.llm != llm {
            log::info!(
                "llm: general agent {}, summaries {}{}",
                llm.general_agent,
                llm.summaries,
                llm.reason
                    .as_deref()
                    .map(|r| format!(" ({r})"))
                    .unwrap_or_default()
            );
        }
        let lost = self.llm.general_agent && !llm.general_agent;
        self.llm = llm.clone();
        // A general agent prompt being dictated has nowhere to go.
        if lost && self.dictation.as_ref().is_some_and(|d| d.general_agent) {
            self.clear_dictation(out);
        }
        out.push(event(Event::Llm(llm)));
    }

    /// Drops what the settings silence (plan 20261024_02, decision 4):
    /// every [`Event::Say`] with sound off, the ding with it or sound off,
    /// and arming the STT gate with the microphone off.
    fn gate(&self, mut out: Vec<Output>) -> Vec<Output> {
        let s = self.settings;
        out.retain(|o| match o {
            Output::Event(Event::Say(_)) => s.sound,
            Output::Event(Event::Ding) => s.sound && s.ding,
            Output::Event(Event::Arm) => s.microphone,
            _ => true,
        });
        out
    }

    /// The connection closed or could not be made.
    pub fn disconnected(&mut self, retry_in_ms: u32) -> Vec<Output> {
        let mut out = Vec::new();
        self.link = Link::Offline;
        self.retry_in_ms = retry_in_ms;
        self.sent.clear();
        // Plugin calls waiting for their answer get none: they are
        // cancelled, so the device half is not left waiting.
        for (ns, id) in std::mem::take(&mut self.plugin_waiting) {
            out.push(event(Event::PluginResult(plugin_error(
                ns,
                id,
                libflankr_proto::envelope::codes::CANCELLED,
                "the connection closed",
            ))));
        }
        // A choice answer on its way gets none: the card is told, and its
        // choices stay for another try.
        if let Some((_, id, pane_id, _)) = self.choosing.take() {
            out.push(event(choice_failed(
                id,
                pane_id,
                "the connection closed".to_string(),
                None,
            )));
        }
        // Terminals are opened again on the next connection.
        self.terminals.update(|b| ((), b.disconnected()));
        // Downloads fail; uploads start again on the next connection.
        self.blobs.disconnected(&mut out);
        // The next connection's first list is given, changed or not.
        self.listing = false;
        self.listed = false;
        self.confirms.clear();
        self.deferred = None;
        // The daemon withdraws what this connection opened; the message is
        // kept to be submitted again.
        if let Some(d) = self.dictation.as_mut() {
            d.phase = DictationPhase::Collecting;
            d.confirm_id = None;
            d.quiet_from = None;
        }
        self.show_dictation(&mut out);
        // The view keeps what it showed; its subscription died with the
        // connection, and its approvals are withdrawn.
        if let Some(v) = self.view.as_mut() {
            v.subscription = None;
            v.subscribing = false;
            v.history = false;
            v.in_flight = 0;
            v.asking.clear();
            v.gap_asked = false;
        }
        self.recompute_view(&mut out);
        if self.turn.take().is_some() {
            out.push(Output::Event(Event::Error(
                "the connection closed during the turn".to_string(),
            )));
            out.push(Output::Event(Event::TurnEnded {
                outcome: "error".to_string(),
                stopped: Vec::new(),
            }));
        }
        out.push(Output::Event(Event::Disconnected { retry_in_ms }));
        // No summary will come on this connection: what waits is read as
        // it is.
        let now = self.local_time;
        for r in self.reading.iter_mut().filter(|r| r.ready.is_none()) {
            r.ready = Some(flankr_speech::read_aloud(&r.text, now));
        }
        // Nor a catch-up: what it was for is said as it is.
        if let Some(serial) = self.catching_up.as_ref().map(|c| c.serial) {
            self.caught_up(serial, None);
        }
        self.read_in_order();
        self.flush_notices(&mut out);
        // `link.unpair` will not be answered now: forgotten here.
        if let Some((_, id)) = self.unpairing.take() {
            out.push(Output::Remove { id, daemon: false });
        }
        self.gate(out)
    }

    /// Something the app said.
    pub fn input(&mut self, input: Input) -> Vec<Output> {
        let mut out = Vec::new();
        match input {
            Input::Pair(_) => out.push(event(Event::PairFailed(
                "pairing is done by the net thread, not the state machine".to_string(),
            ))),
            Input::Speaking(on) => {
                self.speaking = on;
                if !on {
                    self.saying.clear();
                }
                self.read_back_speaking(on, &mut out);
                self.flush_notices(&mut out);
            }
            Input::Listening(on) => {
                self.listening = on;
                let now = self.now;
                if !self.voice_reported
                    && let Some(d) = self.dictation.as_mut()
                {
                    d.quiet_from = (!on).then_some(now);
                }
                self.show_dictation(&mut out);
                self.flush_notices(&mut out);
            }
            Input::Voice(on) => {
                self.voice_reported = true;
                if self.voice && !on {
                    self.voice_ended = Some(self.now);
                }
                self.voice = on;
                let now = self.now;
                if let Some(d) = self.dictation.as_mut() {
                    if on {
                        d.quiet_from = None;
                    } else if d.quiet_from.is_none() {
                        d.quiet_from = Some(now);
                    }
                }
                self.show_dictation(&mut out);
            }
            Input::Tick(now) => self.tick(now, &mut out),
            Input::Tap => self.tap(&mut out),
            Input::Utterance(text) => self.utterance(text, &mut out),
            Input::Approve => self.answer(true, None, &mut out),
            Input::Deny => self.answer(false, None, &mut out),
            Input::StartDictation => self.start_dictation(&mut out),
            Input::StartTextPrompt => self.start_text_prompt(&mut out),
            Input::StartGeneralAgentPrompt(mode) => self.start_general_agent_prompt(mode, &mut out),
            Input::ContinueByVoice => self.continue_by_voice(&mut out),
            Input::SetText(text) => {
                self.blobs.text_changed(&text, &mut out);
                if let Some(d) = self.dictation.as_mut() {
                    d.parts = if text.is_empty() {
                        Vec::new()
                    } else {
                        vec![text]
                    };
                    self.show_dictation(&mut out);
                }
            }
            Input::Submit => self.submit(true, &mut out),
            Input::Clear => self.clear_dictation(&mut out),
            Input::Scratch => self.scratch(&mut out),
            Input::OpenAgent(pane_id) => self.open_agent(pane_id, &mut out),
            Input::OpenChild(id) => self.open_child(Some(id), &mut out),
            Input::CloseChild => self.open_child(None, &mut out),
            Input::CloseAgent => {
                self.close_view(&mut out);
                // Back on the overview: its list is refreshed.
                self.list_wanted = true;
                self.list_if_due(&mut out);
            }
            Input::LoadEarlier => self.load_earlier(&mut out),
            Input::Window(window) => self.set_window(window, &mut out),
            Input::ApplyOutline => {
                if let Some(v) = self.view.as_mut()
                    && v.incoming.is_some()
                    && !v.appending
                {
                    // Below the old end; merged once back at the bottom.
                    v.appending = true;
                    log::info!("view: the new outline's newer sections added below the end");
                    self.show_view(&mut out);
                }
            }
            Input::Follow(on) => {
                self.follow = on;
                if on {
                    let focus = self.focus.clone();
                    self.follow_focus(focus.as_ref(), &mut out);
                }
            }
            Input::Answer {
                confirm_id,
                approve,
            } => self.answer_id(confirm_id, approve, &mut out),
            Input::Respond(response) => self.respond(response, &mut out),
            Input::Suggested(answer) => self.suggested(answer, &mut out),
            Input::Choose { id, answers } => self.choose(id, answers, &mut out),
            Input::Background(on) => self.set_background(on, &mut out),
            Input::Skip => {
                if self.speaking {
                    out.push(event(Event::StopSpeaking));
                }
            }
            Input::StopReading => self.stop_reading(&mut out),
            Input::ReadAloud(on) => {
                self.read_aloud = on;
                // From now: what is already in the view is not read.
                if let Some(v) = self.view.as_mut() {
                    v.see_all();
                }
                if self.online() {
                    self.send_options(&mut out);
                }
            }
            Input::Settings(settings) => self.set_settings(settings, &mut out),
            Input::RemoveRemote(id) => self.unpair(id, &mut out),
            // The net thread's: it knows the remotes.
            Input::Unpair
            | Input::CancelPairing
            | Input::SetNickname { .. }
            | Input::SwitchRemote(_)
            | Input::ToRemote { .. } => {
                log::debug!("{input:?}: handled by the net thread");
            }
            Input::Refresh => self.refresh(&mut out),
            Input::DrawerShown(on) => {
                self.drawer = on;
                if on && self.online() && !self.background && !self.listing {
                    self.list_wanted = false;
                    self.ask_list(&mut out);
                }
            }
            Input::Replay(text) => {
                if self.settings.sound && !text.trim().is_empty() {
                    if self.speaking {
                        out.push(event(Event::StopSpeaking));
                    }
                    out.push(event(Event::Say(text)));
                }
            }
            Input::Memory(pressure) => self.memory(pressure, &mut out),
            Input::ClearCache => {
                log::info!("cache: {} closed view(s) cleared", self.closed.len());
                self.closed.clear();
                self.show_cache(&mut out);
            }
            Input::DismissCard(pane_id) => self.cross.dismiss(&pane_id, false),
            Input::TerminalOpen {
                session,
                cols_hint,
                rows_hint,
            } => {
                let acts = self
                    .terminals
                    .update(|b| b.open(session, cols_hint, rows_hint));
                self.terminal_acts(acts, &mut out);
            }
            Input::TerminalClose(session) => {
                let acts = self.terminals.update(|b| b.close(&session));
                self.terminal_acts(acts, &mut out);
            }
            Input::TerminalArm(session) => {
                let acts = self.terminals.update(|b| b.arm(&session));
                self.terminal_acts(acts, &mut out);
            }
            Input::TerminalKeys { session, keys } => {
                let acts = self.terminals.update(|b| b.send_keys(&session, &keys));
                self.terminal_acts(acts, &mut out);
            }
            Input::TerminalScroll { session, lines } => {
                let acts = self.terminals.update(|b| b.scroll(&session, lines));
                self.terminal_acts(acts, &mut out);
            }
            Input::TerminalResize {
                session,
                cols,
                rows,
            } => {
                let acts = self.terminals.update(|b| b.resize(&session, cols, rows));
                self.terminal_acts(acts, &mut out);
            }
            Input::TerminalMouse {
                session,
                kind,
                col,
                row,
                modifiers,
            } => {
                let e = crate::terminal::MouseEvent {
                    kind,
                    col,
                    row,
                    modifiers,
                };
                let acts = self.terminals.update(|b| b.mouse(&session, e));
                self.terminal_acts(acts, &mut out);
            }
            Input::PluginCall {
                ns,
                id,
                method,
                params,
                session,
            } => self.plugin_call(
                wire::PluginCall {
                    ns,
                    id,
                    method,
                    params,
                    session,
                },
                &mut out,
            ),
            Input::AttachImage { bytes, mime } => match self.view.as_ref() {
                Some(v) => {
                    let pane = v.pane_id.clone();
                    self.blobs.attach(&pane, bytes, &mime, self.now, &mut out);
                }
                None => out.push(event(Event::Error(
                    "Open a session to attach an image.".to_string(),
                ))),
            },
            Input::DetachImage(n) => {
                if self.blobs.detach(n, &mut out) {
                    self.drop_token(n, &mut out);
                }
            }
            Input::RetryImage(n) => self.blobs.retry(n, self.now, &mut out),
            Input::RequestFile { session, path } => {
                self.blobs.request(
                    libflankr_blob::NS_FILES,
                    &session,
                    &path,
                    self.now,
                    &mut out,
                );
            }
            Input::RequestGit { session, request } => {
                self.blobs.request(
                    libflankr_blob::NS_GIT,
                    &session,
                    &request,
                    self.now,
                    &mut out,
                );
            }
            Input::CancelTransfer(id) => self.blobs.cancel(id, &mut out),
            Input::TerminalPasteText(text) => self.terminal_paste(Some(text), Vec::new(), &mut out),
            Input::TerminalPasteImages(images) => self.terminal_paste(None, images, &mut out),
            Input::SetPluginEnabled { ns, enabled } => {
                if self.online() {
                    let params = wire::PluginSetEnabledParams {
                        ns: ns.clone(),
                        enabled,
                    };
                    out.push(self.request(
                        wire::methods::PLUGIN_SET_ENABLED,
                        params,
                        Sent::PluginSetEnabled { ns },
                    ));
                } else {
                    out.push(event(Event::Error("not connected to flankr".to_string())));
                    out.push(event(Event::Plugins(self.plugins.clone())));
                }
            }
        }
        self.show_cards(&mut out);
        // Speech held for voice input goes once it ends (plan 20261023_02,
        // the 2026-10-30 13:37 EDT amendment, item 3).
        self.flush_notices(&mut out);
        self.gate(out)
    }

    /// Image `n`'s token goes from the draft, with a space beside it.
    fn drop_token(&mut self, n: u8, out: &mut Vec<Output>) {
        let token = crate::blob::token(n);
        let Some(d) = self.dictation.as_mut() else {
            return;
        };
        let text = d.text();
        if !text.contains(&token) {
            return;
        }
        let mut cut = text.replacen(&format!(" {token} "), " ", 1);
        if cut == text {
            cut = text.replacen(&format!(" {token}"), "", 1);
        }
        if cut == text {
            cut = text.replacen(&format!("{token} "), "", 1);
        }
        if cut == text {
            cut = text.replacen(&token, "", 1);
        }
        d.parts = if cut.is_empty() {
            Vec::new()
        } else {
            vec![cut]
        };
        self.show_dictation(out);
    }

    /// `terminal.paste` of `text` or of the attached `images` into the open
    /// agent view's armed terminal (plan 20261028_05, the 23:43 EDT
    /// amendment, item 2).
    fn terminal_paste(&mut self, text: Option<String>, images: Vec<u8>, out: &mut Vec<Output>) {
        let Some(pane) = self.view.as_ref().map(|v| v.pane_id.clone()) else {
            out.push(event(Event::Error("No session is open.".to_string())));
            return;
        };
        let terminal_id = if !self.online() {
            Err("I'm not connected to flankr.".to_string())
        } else {
            self.terminals
                .armed_id(&pane)
                .ok_or_else(|| "The terminal is not open for typing.".to_string())
        };
        let refs = if images.is_empty() {
            Ok(Vec::new())
        } else {
            self.blobs.references(&images, &pane)
        };
        let text = match text {
            Some(t) if t.len() > common::TERMINAL_PASTE_TEXT_MAX => Err(format!(
                "At most {} bytes can be pasted at once.",
                common::TERMINAL_PASTE_TEXT_MAX
            )),
            Some(t) if t.is_empty() => Err("Nothing to paste.".to_string()),
            t => Ok(t),
        };
        match (terminal_id, refs, text) {
            (Ok(terminal_id), Ok(images_refs), Ok(text)) => {
                let params = common::TerminalPasteParams {
                    terminal_id,
                    text,
                    images: images_refs,
                };
                out.push(self.request(
                    methods::TERMINAL_PASTE,
                    params,
                    Sent::TerminalPaste { images },
                ));
            }
            (Err(why), _, _) | (_, Err(why), _) | (_, _, Err(why)) => {
                out.push(event(Event::Error(why)));
            }
        }
    }

    /// Sends `call` as `plugin.call`, or refuses it at once: offline, with
    /// a malformed `ns` or `method`, or with `params` over the cap (plan
    /// 20261028_04, 3.2 items 2 and 3). A refused call with an `id` is
    /// answered as [`Event::PluginResult`] with the error.
    fn plugin_call(&mut self, call: wire::PluginCall, out: &mut Vec<Output>) {
        use libflankr_proto::envelope::codes;
        let refusal = if !self.online() {
            Some((codes::UNAVAILABLE, "not connected to flankr".to_string()))
        } else if !wire::is_plugin_ns(&call.ns) {
            Some((codes::INVALID_PARAMS, "a malformed plugin name".to_string()))
        } else if !wire::is_plugin_method(&call.method) {
            Some((
                codes::INVALID_PARAMS,
                "a malformed plugin method".to_string(),
            ))
        } else if serde_json::to_vec(&call.params).map_or(usize::MAX, |b| b.len())
            > wire::PLUGIN_MAX_BYTES
        {
            Some((
                codes::PLUGIN_TOO_LARGE,
                "the call's params are over 256 KiB".to_string(),
            ))
        } else {
            None
        };
        if let Some((code, message)) = refusal {
            log::info!("plugin.call {} {}: {message}", call.ns, call.method);
            if let Some(id) = call.id {
                out.push(event(Event::PluginResult(plugin_error(
                    call.ns, id, code, &message,
                ))));
            }
            return;
        }
        if let Some(id) = call.id {
            self.plugin_waiting.push((call.ns.clone(), id));
        }
        let sent = Sent::PluginCall {
            ns: call.ns.clone(),
            id: call.id,
        };
        out.push(self.request(wire::methods::PLUGIN_CALL, call, sent));
    }

    /// The daemon's plugin list, as told; given as [`Event::Plugins`].
    fn set_plugins(&mut self, plugins: Vec<wire::PluginInfo>, out: &mut Vec<Output>) {
        self.plugins = plugins;
        out.push(event(Event::Plugins(self.plugins.clone())));
    }

    /// Closes the open view, if any: its subscription ends, and it is kept
    /// in the cache of closed views, the least recently used going beyond
    /// [`CACHED_VIEWS`] (plan 20261022_03, the L3 amendment of 2026-10-24
    /// 12:15 EDT, item 1).
    fn close_view(&mut self, out: &mut Vec<Output>) {
        self.unsubscribe(out);
        self.refresh = None;
        let Some(mut v) = self.view.take() else {
            return;
        };
        v.park();
        self.closed
            .retain(|c| c.pane_id != v.pane_id || c.child != v.child);
        self.closed.push_back(v);
        while self.closed.len() > CACHED_VIEWS {
            // TODO(plan 20261023_09): page the least recent out to the
            // phone's redb store (its decision 6) instead of dropping it.
            if let Some(gone) = self.closed.pop_front() {
                log::info!(
                    "cache: the least recent closed view dropped ({} bytes)",
                    gone.bytes()
                );
            }
        }
        self.show_cache(out);
    }

    /// The closed views' size, when it changed since last given.
    fn show_cache(&mut self, out: &mut Vec<Output>) {
        let bytes = self.cache_bytes();
        if self.cache_shown != bytes {
            self.cache_shown = bytes;
            out.push(event(Event::Cache { bytes }));
        }
    }

    /// The closed views' size in memory, in bytes.
    pub fn cache_bytes(&self) -> u64 {
        self.closed.iter().map(AgentView::bytes).sum()
    }

    /// The panes of the closed views kept, the least recently used first.
    pub fn cached_panes(&self) -> Vec<String> {
        self.closed.iter().map(|v| v.pane_id.clone()).collect()
    }

    /// Memory pressure (plan 20261022_03, the L3 amendment of 2026-10-24
    /// 12:21 EDT): the closed views go, the least recently used first,
    /// keeping only the most recent at `UiHidden`; the open view keeps
    /// contents only as close to its window as the grade says, until the
    /// app is back in front.
    fn memory(&mut self, pressure: Pressure, out: &mut Vec<Output>) {
        let keep = usize::from(pressure == Pressure::UiHidden);
        log::info!(
            "memory: {pressure:?}; {} closed view(s), {keep} kept",
            self.closed.len()
        );
        while self.closed.len() > keep {
            // TODO(plan 20261023_09): page out to the phone's redb store
            // instead of dropping it, so reopening reads it back at once.
            self.closed.pop_front();
        }
        if let Some(v) = self.view.as_mut() {
            if v.pressure.is_none_or(|p| p < pressure) {
                v.pressure = Some(pressure);
            }
            v.forget();
            v.reflatten();
        }
        self.show_cache(out);
        self.show_view(out);
    }

    /// The serials the open view shows, packed.
    fn view_serials(&self) -> HashSet<u64> {
        match self.agent_view() {
            Some(Event::AgentView { sections, .. }) => {
                sections.iter().map(|v| v.serial.to_u64()).collect()
            }
            _ => HashSet::new(),
        }
    }

    /// Pull to refresh (plan 20261022_03, the L3 amendment of 2026-10-24
    /// 12:22 EDT, item 2): a ping timed at `PING_WAIT_RESUME`, or, offline,
    /// a connection now; a resubscribe resuming where the view is; and,
    /// once it answers, a fetch of the newest screen's contents.
    fn refresh(&mut self, out: &mut Vec<Output>) {
        if self.view.is_none() {
            out.push(event(Event::Refreshed { new: 0 }));
            return;
        }
        let before = self.view_serials();
        self.refresh = Some(Refresh {
            before,
            subscribed: false,
            fetched: false,
            since: self.now,
        });
        if !self.online() {
            log::info!("refresh: offline; connecting now");
            out.push(Output::Reconnect);
            return;
        }
        log::info!("refresh: a ping and a resubscribe");
        let params = common::PingParams {
            active: Some(!self.background),
        };
        out.push(self.request(methods::PING, params, Sent::Ping));
        self.ping_sent = Some(self.now);
        self.ping_wait = PING_WAIT_RESUME;
        self.unsubscribe(out);
        if let Some(v) = self.view.as_mut() {
            // What was asked on the old subscription is not answered for
            // the new one.
            v.in_flight = 0;
            v.asking.clear();
            v.gap_asked = false;
        }
        self.subscribe(out);
    }

    /// The refresh's resubscribe answered: the newest screen's contents
    /// are fetched, whatever their revs.
    fn refresh_subscribed(&mut self, out: &mut Vec<Output>) {
        if self.refresh.as_ref().is_none_or(|r| r.subscribed) {
            return;
        }
        let fetching = self.refresh_fetch(out);
        if let Some(r) = self.refresh.as_mut() {
            r.subscribed = true;
            r.fetched |= !fetching;
        }
        self.finish_refresh(out);
    }

    /// Asks for the contents of the newest screen, from the window's top
    /// (the newest `DEFAULT_NEAR` without one) to the outline's end.
    /// Whether it asked.
    fn refresh_fetch(&mut self, out: &mut Vec<Output>) -> bool {
        if !self.online() {
            return false;
        }
        let Some(v) = self.view.as_mut() else {
            return false;
        };
        let (Some(id), Some(last)) = (v.subscription.filter(|_| v.outline_mode), v.outline.last())
        else {
            return false;
        };
        let lo = v
            .window
            .map_or(last.saturating_sub(DEFAULT_NEAR - 1), |w| w.shown.0)
            .max(last.saturating_sub(CONTENT_RANGE - 1));
        let asked: Vec<(u64, common::Serial, u32)> = v
            .outline
            .range(lo, last)
            .map(|s| (s.index, s.serial, s.rev))
            .collect();
        if asked.is_empty() {
            return false;
        }
        v.asking.extend(asked.iter().map(|a| a.0));
        v.in_flight += 1;
        let params = common::SessionContentParams {
            subscription_id: id,
            from: Some(lo),
            to: Some(last),
            limit: common::SECTIONS_MAX,
            slots_only: false,
        };
        let sent = Sent::Content {
            subscription: id,
            generation: v.generation,
            asked,
            slots_only: false,
            refresh: true,
        };
        out.push(self.request(methods::SESSION_CONTENT, params, sent));
        true
    }

    /// Gives the refresh's result once its resubscribe and fetch have
    /// answered, or `REFRESH_WAIT` has passed.
    fn finish_refresh(&mut self, out: &mut Vec<Output>) {
        let Some(r) = &self.refresh else {
            return;
        };
        let timed_out = self.now.saturating_sub(r.since) >= REFRESH_WAIT;
        if !(timed_out || r.subscribed && r.fetched) {
            return;
        }
        let before = self.refresh.take().map(|r| r.before).unwrap_or_default();
        let new = self.view_serials().difference(&before).count();
        log::info!(
            "refresh: done, {new} new{}",
            if timed_out { " (timed out)" } else { "" }
        );
        out.push(event(Event::Refreshed { new }));
    }

    /// The settings changed (plan 20261024_02, decisions 3 and 4).
    fn set_settings(&mut self, settings: Settings, out: &mut Vec<Output>) {
        let before = std::mem::replace(&mut self.settings, settings);
        if before == settings {
            return;
        }
        log::info!(
            "settings: sound {}, ding {}, microphone {}, general agent {}, tab follows focus {}, \
             speak gemini {}",
            settings.sound,
            settings.ding,
            settings.microphone,
            settings.general_agent,
            settings.tab_follows_focus,
            settings.speak_gemini
        );
        if before.sound && !settings.sound {
            // Nothing more is read: what waits is dropped, and what is
            // being said stops.
            self.reading.clear();
            self.notices.clear();
            self.catching_up = None;
            if self.speaking {
                out.push(event(Event::StopSpeaking));
            }
            let now = self.now;
            if let Some(d) = self.dictation.as_mut()
                && d.read_back != ReadBack::No
            {
                d.read_back = ReadBack::No;
                d.quiet_from = Some(now);
            }
        }
        if before.microphone && !settings.microphone {
            // No dictation without the microphone.
            self.clear_dictation(out);
        }
        // Read aloud is off in effect without sound, so the daemon makes no
        // summaries for it.
        let spoken = |s: Settings| self.read_aloud && s.sound;
        let options = before.general_agent != settings.general_agent
            || before.tab_follows_focus != settings.tab_follows_focus
            || spoken(before) != spoken(settings);
        if options && self.online() {
            self.send_options(out);
        }
    }

    /// Asks the daemon of the active remote `id` to forget this device,
    /// or, offline, removes the remote at once (plan 20261024_02, decision
    /// 6; plan 20261028_01, decision 22).
    fn unpair(&mut self, id: RemoteId, out: &mut Vec<Output>) {
        if self.unpairing.is_some() {
            return;
        }
        // Another daemon's sessions are not to be shown from the cache.
        self.closed.clear();
        self.show_cache(out);
        if self.online() {
            log::info!("unpairing: asking the daemon to forget this device");
            self.unpairing = Some((self.now, id));
            out.push(self.request(
                methods::LINK_UNPAIR,
                common::LinkUnpairParams {},
                Sent::Unpair,
            ));
        } else {
            log::info!("unpairing: offline; forgetting the pairing here only");
            out.push(Output::Remove { id, daemon: false });
        }
    }

    /// The agent view as it stands, if one is open.
    pub fn agent_view(&self) -> Option<Event> {
        let v = self.view.as_ref()?;
        // A question answered from the view goes at once.
        let answered = v
            .replied_to
            .as_ref()
            .is_some_and(|ts| v.pending_question.as_ref().is_some_and(|q| &q.ts == ts));
        let hide = |c: &crate::view::AgentCard| {
            answered && c.live && c.kind == crate::view::CardKind::Question
        };
        let (sections, pinned) = if v.outline_mode {
            let newer = v
                .incoming
                .as_ref()
                .filter(|_| v.appending)
                .map(|i| (&i.outline, &i.sections));
            crate::view::outline_view_appending(&v.outline, &v.sections, newer, hide)
        } else {
            v.sections.view(hide)
        };
        // The live question card offers its pending question's answers
        // (plan 20261027_06, step 4).
        // A choice question's card carries its choices the same way (plan
        // 20261029_01, step 5).
        let mut sections = sections;
        if let Some(q) = v
            .pending_question
            .as_ref()
            .filter(|q| !q.answers.is_empty() || q.choices.is_some())
        {
            for section in &mut sections {
                if let crate::view::Shows::Card(c) = &mut section.shows
                    && c.live
                    && c.kind == crate::view::CardKind::Question
                    && c.text == q.text
                {
                    c.answers = q.answers.clone();
                    c.choices = q.choices.clone();
                }
            }
        }
        let changed = match &v.shown {
            Some(Event::AgentView {
                sections: before, ..
            }) => changed_serials(before, &sections),
            _ => sections.iter().map(|v| v.serial).collect(),
        };
        let terminal_on = self
            .focus
            .as_ref()
            .filter(|f| f.pane_id != v.pane_id)
            .map(|f| f.name.clone())
            .filter(|n| !n.is_empty());
        Some(Event::AgentView {
            name: self.agent_name(&v.pane_id),
            status: v.status.clone(),
            sections,
            pinned,
            changed,
            loading: !v.answered,
            typed: v.typed.clone(),
            loading_earlier: v.history && !v.outline_mode,
            // In outline mode, once the outline is held from index 0.
            start: if v.outline_mode {
                v.outline.contiguous() > 0
            } else {
                v.start
            },
            terminal_on,
            new_content: v.incoming.is_some(),
            held_end: v
                .incoming
                .as_ref()
                .filter(|_| !v.appending)
                .map(|i| crate::view::held_end(&i.outline, &i.sections, hide))
                .unwrap_or_default(),
            held_newer: v
                .incoming
                .as_ref()
                .filter(|_| v.outline_mode && !v.appending)
                .map(|i| {
                    crate::view::newer_slots(&v.outline, &i.outline)
                        .into_iter()
                        .copied()
                        .collect()
                })
                .unwrap_or_default(),
            bookends: self.bookends(v),
            child: v.child.clone(),
            children: self
                .agents
                .iter()
                .find(|a| a.pane_id == v.pane_id)
                .map(|a| a.children.clone())
                .unwrap_or_default(),
        })
    }

    /// The open view's bookends (plan 20261022_03, the L3 amendment of
    /// 2026-10-24 12:27 EDT): the outline's Finished-card slots, or, from a
    /// daemon without outlines, the Finished cards held; each with its
    /// card's short summary and its reply's time on the phone's clock, as
    /// far as they are held.
    fn bookends(&self, v: &AgentView) -> Vec<crate::view::Bookend> {
        let serials: Vec<common::Serial> = if v.outline_mode {
            v.outline
                .slots()
                .filter(|s| s.bookend)
                .map(|s| s.serial)
                .collect()
        } else {
            v.sections
                .iter()
                .filter(|s| {
                    s.card
                        .as_ref()
                        .is_some_and(|c| c.card == common::CardKind::Finished)
                })
                .map(|s| s.serial)
                .collect()
        };
        let offset = self.local_time.utc_offset_minutes;
        serials
            .into_iter()
            .map(|serial| {
                let summary = v
                    .sections
                    .get(serial)
                    .and_then(|s| s.card.as_ref())
                    .and_then(|c| c.spoken.as_deref())
                    .and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
                    .map(str::to_string);
                // The reply the card closes shares its line.
                let time = v
                    .sections
                    .on_line(serial)
                    .filter(|s| s.card.is_none())
                    .flat_map(|s| s.entries.iter())
                    .rev()
                    .find_map(|e| crate::view::clock_time(&e.ts, offset));
                crate::view::Bookend {
                    serial,
                    summary,
                    time,
                }
            })
            .collect()
    }

    /// The shown name of the agent in `pane_id`, from `agents.list` or the
    /// focus.
    fn agent_name(&self, pane_id: &str) -> String {
        self.agents
            .iter()
            .find(|a| a.pane_id == pane_id)
            .and_then(|a| a.name.clone())
            .or_else(|| {
                self.focus
                    .as_ref()
                    .filter(|f| f.pane_id == pane_id)
                    .map(|f| f.name.clone())
            })
            .unwrap_or_else(|| "the agent".to_string())
    }

    /// The agent in `pane_id` as a message target: from `agents.list`, else
    /// the focus when it is that pane.
    fn target_for(&self, pane_id: &str) -> Option<common::FocusInfo> {
        if let Some(a) = self.agents.iter().find(|a| a.pane_id == pane_id) {
            return Some(focus_of(a));
        }
        self.focus.clone().filter(|f| f.pane_id == pane_id)
    }

    /// Whether the pane's session is terminal only, as `agents.list` gave
    /// it ([`crate::cross::is_terminal_only`]).
    fn terminal_only(&self, pane_id: &str) -> bool {
        self.agents
            .iter()
            .any(|a| a.pane_id == pane_id && crate::cross::is_terminal_only(a))
    }

    /// Whether the open view is of a terminal-only session: it takes no
    /// message, and has no transcript to subscribe to.
    fn viewing_terminal_only(&self) -> bool {
        self.view
            .as_ref()
            .is_some_and(|v| self.terminal_only(&v.pane_id))
    }

    fn open_agent(&mut self, pane_id: String, out: &mut Vec<Output>) {
        self.open_view(pane_id, None, out);
    }

    /// Show the open view's subagent `id` ([`Input::OpenChild`]), or, with
    /// `None`, its parent's main session ([`Input::CloseChild`]).
    fn open_child(&mut self, id: Option<String>, out: &mut Vec<Output>) {
        let Some(pane_id) = self.view.as_ref().map(|v| v.pane_id.clone()) else {
            return;
        };
        self.open_view(pane_id, id, out);
    }

    /// Open the view of `pane_id`'s main session, or of its subagent
    /// `child` (the subagents plan, step 4).
    fn open_view(&mut self, pane_id: String, child: Option<String>, out: &mut Vec<Output>) {
        self.interact(&pane_id);
        // Its own cards are seen.
        self.cross.dismiss(&pane_id, true);
        if self
            .view
            .as_ref()
            .is_some_and(|v| v.pane_id == pane_id && v.child == child)
        {
            return self.show_view(out);
        }
        // Another agent's view, or another session of this one: its
        // subscription ends, and it is cached.
        self.close_view(out);
        let agent = self.agents.iter().find(|a| a.pane_id == pane_id);
        // A subagent's own status, until its subscription says.
        let status = match (&child, agent) {
            (None, Some(a)) => a.status.clone(),
            (Some(id), Some(a)) => match a.children.iter().find(|c| &c.id == id) {
                Some(c) if c.finished => "done".to_string(),
                Some(_) => "working".to_string(),
                None => String::new(),
            },
            (_, None) => String::new(),
        };
        // A view closed lately reopens from the cache at once, and resumes
        // with what came while it was closed (the 12:12 EDT amendment).
        let cached = self
            .closed
            .iter()
            .position(|c| c.pane_id == pane_id && c.child == child);
        let view = match cached {
            Some(i) => {
                let mut v = self.closed.remove(i).expect("a cached view");
                log::info!(
                    "view: reopened from the cache, {} section(s), {} slot(s)",
                    v.sections.len(),
                    v.outline.len()
                );
                v.backfill = true;
                v
            }
            None => AgentView::new(pane_id, child, status),
        };
        self.view = Some(view);
        self.show_cache(out);
        if self.online() {
            // Names and statuses, for the view's title and its target.
            self.ask_list(out);
        }
        self.subscribe(out);
        self.recompute_view(out);
    }

    /// Subscribes the open view to its pane's session, if it has no
    /// subscription on this connection: from its newest serial when it
    /// holds any (a reconnect), so only what it missed comes.
    fn subscribe(&mut self, out: &mut Vec<Output>) {
        if !self.online() {
            return;
        }
        // A shell has no transcript (plan 20261022_03, the 2026-10-28
        // 00:47 EDT amendment).
        if self.viewing_terminal_only() {
            return;
        }
        let Some(v) = self.view.as_mut() else {
            return;
        };
        if v.subscription.is_some() || v.subscribing {
            return;
        }
        v.subscribing = true;
        let pane_id = v.pane_id.clone();
        let child = v.child.clone();
        // In outline mode, from the first hole of the newest outline held
        // (plan 20261024_01, decision 6, and its epoch, CHANGES.md
        // 2026-10-24 00:36 EDT).
        let (after, from, epoch) = if v.outline_mode {
            let o = v.incoming.as_ref().map_or(&v.outline, |i| &i.outline);
            match o.resume() {
                Some((after, from)) => (Some(after), Some(from), o.epoch()),
                None => (None, None, None),
            }
        } else {
            (v.sections.newest(), None, None)
        };
        let params = common::SessionSubscribeParams {
            pane_id: pane_id.clone(),
            sections: VIEW_SECTIONS,
            after,
            outline: true,
            from,
            epoch,
            child: child.clone(),
        };
        let sent = Sent::Subscribe(pane_id, child);
        out.push(self.request(methods::SESSION_SUBSCRIBE, params, sent));
    }

    /// Ends the open view's subscription, if it has one.
    fn unsubscribe(&mut self, out: &mut Vec<Output>) {
        let Some(id) = self.view.as_mut().and_then(|v| v.subscription.take()) else {
            return;
        };
        if self.online() {
            let params = common::SessionUnsubscribeParams {
                subscription_id: id,
            };
            out.push(self.request(methods::SESSION_UNSUBSCRIBE, params, Sent::Unsubscribe));
        }
    }

    /// Pages in the sections before the oldest held (A7): one page at a
    /// time, and none once a page reached the session's start.
    fn load_earlier(&mut self, out: &mut Vec<Output>) {
        if !self.online() {
            return;
        }
        // In outline mode the whole session is in the outline: no pages.
        let Some(v) = self.view.as_mut().filter(|v| !v.outline_mode) else {
            return;
        };
        let (Some(id), Some(before)) = (v.subscription, v.sections.oldest()) else {
            return;
        };
        if v.history || v.start {
            return;
        }
        v.history = true;
        let params = common::SessionHistoryParams {
            subscription_id: id,
            before,
            sections: HISTORY_PAGE,
        };
        out.push(self.request(methods::SESSION_HISTORY, params, Sent::History(id)));
        self.show_view(out);
    }

    /// The app's window moved (plan 20261024_01, decision 6): back at the
    /// bottom, an outline held aside is shown at once; far contents are
    /// forgotten and near ones fetched.
    fn set_window(&mut self, window: ViewWindow, out: &mut Vec<Output>) {
        let Some(v) = self.view.as_mut() else {
            return;
        };
        if v.window == Some(window) {
            return;
        }
        v.window = Some(window);
        if !window.scrolled_back && v.apply_incoming() {
            v.reflatten();
            v.see_all();
        }
        v.forget();
        v.reflatten();
        self.fetch_contents(out);
        self.show_view(out);
    }

    /// Asks `session.content` for the contents of the window's near slots
    /// that are missing or older than their slot's `rev` (plan 20261024_01,
    /// decision 6): ranges of at most `CONTENT_RANGE` indexes, the nearest
    /// the window first, with at most `CONTENT_IN_FLIGHT` requests in
    /// flight. Not while the outline shown is incomplete or another is
    /// held aside: indexes are the daemon's newest outline's.
    fn fetch_contents(&mut self, out: &mut Vec<Output>) {
        if !self.online() {
            return;
        }
        let Some(v) = self.view.as_mut() else {
            return;
        };
        let Some(id) = v.subscription.filter(|_| v.outline_mode) else {
            return;
        };
        if v.incoming.is_some() || !v.outline.complete() || v.in_flight >= CONTENT_IN_FLIGHT {
            return;
        }
        let Some(last) = v.outline.last() else {
            return;
        };
        let (shown, near) = match v.window {
            Some(w) => (w.shown, v.fetch_span(w)),
            None => {
                let near = (last.saturating_sub(DEFAULT_NEAR - 1), last);
                (near, near)
            }
        };
        let needed: Vec<u64> = v
            .outline
            .range(near.0, near.1)
            .filter(|slot| !v.asking.contains(&slot.index))
            .filter(|slot| v.sections.rev(slot.serial) != Some(slot.rev))
            .filter(|slot| v.unavailable.get(&slot.serial.to_u64()) != Some(&slot.rev))
            .map(|slot| slot.index)
            .collect();
        // Runs of the needed indexes, close together and at most a range.
        let mut runs: Vec<(u64, u64)> = Vec::new();
        for i in needed {
            match runs.last_mut() {
                Some((lo, hi)) if i - *hi <= CONTENT_MERGE_GAP && i - *lo < CONTENT_RANGE => {
                    *hi = i
                }
                _ => runs.push((i, i)),
            }
        }
        let distance = |&(lo, hi): &(u64, u64)| {
            if hi < shown.0 {
                shown.0 - hi
            } else {
                lo.saturating_sub(shown.1)
            }
        };
        runs.sort_by_key(distance);
        let room = CONTENT_IN_FLIGHT - v.in_flight;
        let mut requests = Vec::new();
        for (lo, hi) in runs.into_iter().take(room) {
            let asked: Vec<(u64, common::Serial, u32)> = v
                .outline
                .range(lo, hi)
                .map(|s| (s.index, s.serial, s.rev))
                .collect();
            v.asking.extend(asked.iter().map(|a| a.0));
            v.in_flight += 1;
            let params = common::SessionContentParams {
                subscription_id: id,
                from: Some(lo),
                to: Some(hi),
                limit: common::SECTIONS_MAX,
                slots_only: false,
            };
            let sent = Sent::Content {
                subscription: id,
                generation: v.generation,
                asked,
                slots_only: false,
                refresh: false,
            };
            requests.push((params, sent));
        }
        for (params, sent) in requests {
            out.push(self.request(methods::SESSION_CONTENT, params, sent));
        }
    }

    /// Asks for the slots of the building outline's first hole, once it
    /// has stayed for `GAP_WAIT` (plan 20261024_01, decisions 3 and 6):
    /// a delivery was missed, not an outline still on its way.
    fn fill_gaps(&mut self, out: &mut Vec<Output>) {
        if !self.online() {
            return;
        }
        let now = self.now;
        let Some(v) = self.view.as_mut() else {
            return;
        };
        let Some(id) = v.subscription.filter(|_| v.outline_mode) else {
            return;
        };
        v.note_hole(now);
        if v.gap_asked || v.in_flight >= CONTENT_IN_FLIGHT {
            return;
        }
        let since = v.hole.1;
        let generation = v.generation;
        let (outline, _) = v.building();
        let Some(last) = outline.last().filter(|_| !outline.complete()) else {
            return;
        };
        if now.saturating_sub(since) < GAP_WAIT {
            return;
        }
        let from = outline.contiguous();
        let to = last.min(from + CONTENT_RANGE - 1);
        log::info!("view: outline hole at {from}; asking for {from}..={to}");
        v.gap_asked = true;
        v.in_flight += 1;
        let params = common::SessionContentParams {
            subscription_id: id,
            from: Some(from),
            to: Some(to),
            limit: common::SECTIONS_MAX,
            slots_only: true,
        };
        let sent = Sent::Content {
            subscription: id,
            generation,
            asked: Vec::new(),
            slots_only: true,
            refresh: false,
        };
        out.push(self.request(methods::SESSION_CONTENT, params, sent));
    }

    /// A `session.content` answer (plan 20261024_01, decision 4): slots
    /// for a hole, or contents, each kept at the rev it was asked at.
    fn content_answered(
        &mut self,
        sent: Sent,
        result: Option<common::SessionContentResult>,
        out: &mut Vec<Output>,
    ) {
        let Sent::Content {
            subscription,
            generation,
            asked,
            slots_only,
            refresh,
        } = sent
        else {
            return;
        };
        if refresh && let Some(r) = self.refresh.as_mut() {
            r.fetched = true;
        }
        let now = self.now;
        let Some(v) = self
            .view
            .as_mut()
            .filter(|v| v.subscription == Some(subscription))
        else {
            return;
        };
        v.in_flight = v.in_flight.saturating_sub(1);
        for (i, _, _) in &asked {
            v.asking.remove(i);
        }
        if slots_only {
            v.gap_asked = false;
        }
        if let Some(r) = result.filter(|_| generation == v.generation) {
            if slots_only {
                log::info!(
                    "view: {} slot(s) for a hole, {} removed",
                    r.slots.len(),
                    r.removed_index.len()
                );
                let (outline, sections) = v.building();
                for slot in r.slots {
                    outline.put(slot);
                }
                for i in r.removed_index {
                    if let Some(slot) = outline.remove(i) {
                        sections.remove(slot.serial);
                    }
                }
                if r.last > outline.last() {
                    outline.set_last(r.last);
                }
                v.note_hole(now);
                v.drop_orphans();
            } else {
                log::info!(
                    "view: contents of {} section(s) for {} asked",
                    r.sections.len(),
                    asked.len()
                );
                let mut given = std::collections::HashSet::new();
                for s in r.sections {
                    let Some(&(_, _, rev)) = asked.iter().find(|a| a.1 == s.serial) else {
                        continue;
                    };
                    if v.outline.slot_of(s.serial).is_some() {
                        given.insert(s.serial.to_u64());
                        v.sections.put_rev(s, rev);
                    }
                }
                for (_, serial, rev) in &asked {
                    if !given.contains(&serial.to_u64()) {
                        v.unavailable.insert(serial.to_u64(), *rev);
                    }
                }
                for i in r.removed_index {
                    if let Some(slot) = v.outline.remove(i) {
                        v.sections.remove(slot.serial);
                    }
                }
                v.reflatten();
                // Scrollback, not live: none of it is read aloud.
                v.see_all();
            }
        }
        self.fetch_contents(out);
        self.fill_gaps(out);
        self.show_view(out);
        self.finish_refresh(out);
    }

    /// With Follow on, an open view moves to herdr's focus when it names
    /// another pane with an agent (plan 20261023_03, decision 2).
    fn follow_focus(&mut self, focus: Option<&common::FocusInfo>, out: &mut Vec<Output>) {
        if !self.follow {
            return;
        }
        let Some(focus) = focus.filter(|f| f.agent.is_some()) else {
            return;
        };
        if self
            .view
            .as_ref()
            .is_some_and(|v| v.pane_id != focus.pane_id)
        {
            self.open_agent(focus.pane_id.clone(), out);
        }
    }

    /// Gives the pending panes if they changed, and the view if it did.
    fn recompute_view(&mut self, out: &mut Vec<Output>) {
        self.show_pending(out);
        self.show_view(out);
    }

    /// Gives the agent view if it changed since it was last given.
    fn show_view(&mut self, out: &mut Vec<Output>) {
        let Some(event) = self.agent_view() else {
            return;
        };
        self.view_ding(&event, out);
        let v = self.view.as_mut().expect("a view");
        if !v
            .shown
            .as_ref()
            .is_some_and(|shown| same_view(shown, &event))
        {
            v.shown = Some(event.clone());
            out.push(Output::Event(event));
        }
    }

    /// A ding for the view's agent when a new live Finished card shows:
    /// one that was not shown before, after the agent has worked since the
    /// view opened or since its last ding (plan 20261024_02, decision 3).
    fn view_ding(&mut self, view: &Event, out: &mut Vec<Output>) {
        let Event::AgentView {
            sections, status, ..
        } = view
        else {
            return;
        };
        let finished = sections.iter().rev().find_map(|x| match &x.shows {
            crate::view::Shows::Card(c) if c.live && c.kind == common::CardKind::Finished => {
                Some(x.serial)
            }
            _ => None,
        });
        let Some(v) = self.view.as_mut() else {
            return;
        };
        if status == "working" {
            v.worked = true;
        }
        if finished == v.finished {
            return;
        }
        v.finished = finished;
        if finished.is_some() && v.worked {
            v.worked = false;
            // A subagent's finish is not its parent's turn.
            if v.child.is_none() {
                self.dinged.insert(v.pane_id.clone());
            }
            log::info!("ding: the viewed agent finished");
            out.push(event(Event::Ding));
        }
    }

    /// A ding for each agent whose turn newly finished since the agents
    /// were last listed: stopped on a reply now, and not before, or before
    /// on another reply (plan 20261024_02, decision 3). An agent not listed
    /// before is not dinged, nor a finish the agent view already dinged.
    fn ding_finished(&mut self, agents: &[common::AgentInfo], out: &mut Vec<Output>) {
        for a in agents {
            let Some(before) = self.agents.iter().find(|b| b.pane_id == a.pane_id) else {
                continue;
            };
            // The open view's agent, once its session answered, is dinged
            // by its own Finished card: a list while it is open (watching
            // other sessions) does not ding it again.
            if self
                .view
                .as_ref()
                .is_some_and(|v| v.pane_id == a.pane_id && v.child.is_none() && v.answered)
            {
                continue;
            }
            match &a.turn {
                Some(common::Turn::Stopped { line }) if before.turn != a.turn => {
                    self.peek_finished(a, line, out);
                    if !self.dinged.remove(&a.pane_id) {
                        log::info!("ding: an agent finished");
                        out.push(event(Event::Ding));
                    }
                }
                Some(common::Turn::Stopped { .. }) => {}
                // Working again: a ding the view gave is for an older turn.
                _ => {
                    self.dinged.remove(&a.pane_id);
                }
            }
        }
    }

    /// Answers the confirmation `confirm_id` (a card's button), whether or
    /// not it is the one being asked.
    fn answer_id(&mut self, confirm_id: u64, approve: bool, out: &mut Vec<Output>) {
        if self.confirms.front().map(|c| c.id) == Some(confirm_id) {
            return self.answer(approve, None, out);
        }
        if let Some(pane_id) = self
            .confirms
            .iter()
            .find(|c| c.id == confirm_id)
            .map(|c| c.pane_id.clone())
        {
            self.interact(&pane_id);
        }
        let before = self.confirms.len();
        self.confirms.retain(|c| c.id != confirm_id);
        if self.confirms.len() == before {
            log::debug!("no confirmation {confirm_id} to answer");
            return;
        }
        self.reply(confirm_id, approve, out);
        self.recompute_view(out);
    }

    /// The dictation card as it stands now.
    pub fn dictation(&self) -> Dictation {
        let Some(d) = &self.dictation else {
            return Dictation::off();
        };
        let focus_moved = match (&d.target, &self.focus) {
            (Some(target), Some(focus)) if focus.pane_id != target.pane_id => {
                Some(focus.name.clone())
            }
            _ => None,
        };
        let submit_in = self.deadline().and_then(|deadline| {
            let left = deadline.saturating_sub(self.now);
            (left <= Duration::from_secs(COUNTDOWN_SECS))
                .then(|| left.as_millis().div_ceil(1000) as u32)
        });
        Dictation {
            active: true,
            mode: d.mode,
            text: d.text(),
            target: d.target.clone(),
            focus_moved,
            phase: d.phase,
            submit_in,
            general_agent: d.general_agent,
        }
    }

    /// Gives the dictation card if it changed.
    fn show_dictation(&mut self, out: &mut Vec<Output>) {
        let view = self.dictation();
        if view != self.shown {
            self.shown = view.clone();
            out.push(event(Event::Dictation(view)));
        }
    }

    /// Whether speech may not have reached STT's result yet: the voice is
    /// on, or ended less than [`HEARD_WAIT`] ago with no utterance since.
    /// Only front ends with a VAD can tell.
    fn unheard(&self) -> bool {
        self.voice_reported
            && (self.voice || self.voice_ended.is_some_and(|t| self.now < t + HEARD_WAIT))
    }

    /// Whether no one is speaking: no voice when the front end reports it,
    /// else the gate idle.
    fn quiet(&self) -> bool {
        if self.voice_reported {
            !self.voice
        } else {
            !self.listening
        }
    }

    /// When the silence submits the message, if it is running: collecting,
    /// something to submit, the gate closed, and not reading it back.
    fn deadline(&self) -> Option<Duration> {
        let d = self.dictation.as_ref()?;
        let running = d.phase == DictationPhase::Collecting
            && !d.parts.is_empty()
            && self.quiet()
            && d.read_back == ReadBack::No;
        let silence = if d.general_agent {
            GENERAL_AGENT_SILENCE
        } else {
            self.silence
        };
        running.then_some(d.quiet_from? + silence)
    }

    fn tick(&mut self, now: Duration, out: &mut Vec<Output>) {
        self.now = self.now.max(now);
        self.keep_alive(out);
        self.blobs.tick(self.now, out);
        if self
            .unpairing
            .is_some_and(|(at, _)| self.now >= at + UNPAIR_WAIT)
            && let Some((_, id)) = self.unpairing.take()
        {
            log::warn!("unpairing: no answer in time; forgetting the pairing here");
            out.push(Output::Remove { id, daemon: false });
        }
        if self
            .choosing
            .as_ref()
            .is_some_and(|(_, _, _, at)| self.now >= *at + CHOICE_ANSWER_WAIT)
            && let Some((request, id, pane_id, _)) = self.choosing.take()
        {
            log::warn!("choice.answer: no answer in time");
            self.sent.remove(&request);
            out.push(event(choice_failed(
                id,
                pane_id,
                format!(
                    "no answer from the daemon in {} s",
                    CHOICE_ANSWER_WAIT.as_secs()
                ),
                None,
            )));
        }
        if let Some(by_button) = d_waiting(&self.dictation).filter(|_| !self.unheard()) {
            self.submit(by_button, out);
        }
        if self.deadline().is_some_and(|deadline| self.now >= deadline) {
            log::info!(
                "dictation: {} s of silence; submitting",
                self.silence.as_secs()
            );
            self.submit(false, out);
        }
        self.show_dictation(out);
        self.list_if_due(out);
        self.fill_gaps(out);
        self.finish_refresh(out);
        if !self.reading.is_empty() {
            self.read_in_order();
            self.flush_notices(out);
        }
    }

    fn start_dictation(&mut self, out: &mut Vec<Output>) {
        if let Some(d) = &self.dictation
            && !d.general_agent
            && d.mode == PromptMode::Voice
        {
            return;
        }
        self.dictation = None;
        self.start_dictation_with(Vec::new(), out);
    }

    fn start_text_prompt(&mut self, out: &mut Vec<Output>) {
        if self.viewing_terminal_only() {
            log::info!("dictation: a shell takes no message");
            return;
        }
        if let Some(d) = &self.dictation
            && !d.general_agent
            && d.mode == PromptMode::Keyboard
        {
            return;
        }
        let existing_parts = self
            .dictation
            .take()
            .filter(|d| !d.general_agent)
            .map(|d| d.parts)
            .unwrap_or_default();
        let fixed_pane = self.view.as_ref().map(|v| v.pane_id.clone());
        let target = match &fixed_pane {
            Some(pane_id) => self.target_for(pane_id),
            None => self.focus.clone(),
        };
        self.dictation = Some(Dictating {
            mode: PromptMode::Keyboard,
            fixed_pane,
            parts: existing_parts,
            target,
            refresh_target: self.online(),
            phase: DictationPhase::Collecting,
            confirm_id: None,
            own_approval: false,
            quiet_from: None,
            read_back: ReadBack::No,
            submit_waiting: None,
            general_agent: false,
        });
        if self.online() {
            self.ask_list(out);
        }
        self.show_dictation(out);
    }

    fn start_general_agent_prompt(&mut self, mode: PromptMode, out: &mut Vec<Output>) {
        if !self.general_agent() {
            log::info!("dictation: the general agent is off");
            return;
        }
        self.dictation = None;
        self.shown = Dictation::off();
        if mode == PromptMode::Voice && !self.settings.microphone {
            log::info!("dictation: the microphone is off");
            return;
        }
        if let Some(turn) = self.turn.as_mut() {
            out.push(event(Event::StopSpeaking));
            if !turn.tapped {
                turn.tapped = true;
                let params = wire::TurnInterruptParams { turn_id: turn.id };
                out.push(self.request(wire::methods::TURN_INTERRUPT, params, Sent::Interrupt));
            }
        }
        self.dictation = Some(Dictating {
            mode,
            fixed_pane: None,
            parts: Vec::new(),
            target: None,
            refresh_target: false,
            phase: DictationPhase::Collecting,
            confirm_id: None,
            own_approval: true,
            quiet_from: None,
            read_back: ReadBack::No,
            submit_waiting: None,
            general_agent: true,
        });
        self.show_dictation(out);
        if mode == PromptMode::Voice {
            self.stop_for_voice_input(out);
            out.push(event(Event::Arm));
        }
    }

    /// The typed prompt open goes on by voice (`Input::ContinueByVoice`).
    fn continue_by_voice(&mut self, out: &mut Vec<Output>) {
        if !self.settings.microphone {
            log::info!("dictation: the microphone is off");
            return;
        }
        let Some(d) = self
            .dictation
            .as_mut()
            .filter(|d| d.mode == PromptMode::Keyboard)
        else {
            return;
        };
        d.mode = PromptMode::Voice;
        d.phase = DictationPhase::Collecting;
        d.quiet_from = None;
        self.show_dictation(out);
        self.stop_for_voice_input(out);
        out.push(event(Event::Arm));
    }

    /// Take a message whose first parts are `parts`, already in it.
    fn start_dictation_with(&mut self, parts: Vec<String>, out: &mut Vec<Output>) {
        if let Some(d) = &self.dictation
            && !d.general_agent
            && d.mode == PromptMode::Voice
            && d.parts == parts
        {
            return;
        }
        self.dictation = None;
        if self.viewing_terminal_only() {
            log::info!("dictation: a shell takes no message");
            return;
        }
        if !self.settings.microphone {
            log::info!("dictation: the microphone is off");
            return;
        }
        // From the agent view, the message is for that agent.
        let fixed_pane = self.view.as_ref().map(|v| v.pane_id.clone());
        let target = match &fixed_pane {
            Some(pane_id) => self.target_for(pane_id),
            None => self.focus.clone(),
        };
        self.dictation = Some(Dictating {
            mode: PromptMode::Voice,
            fixed_pane,
            parts,
            target,
            refresh_target: self.online(),
            phase: DictationPhase::Collecting,
            confirm_id: None,
            own_approval: false,
            quiet_from: None,
            read_back: ReadBack::No,
            submit_waiting: None,
            general_agent: false,
        });
        if self.online() {
            self.ask_list(out);
        }
        self.show_dictation(out);
        self.stop_for_voice_input(out);
        out.push(event(Event::Arm));
    }

    fn clear_dictation(&mut self, out: &mut Vec<Output>) {
        if self.dictation.take().is_some() {
            self.show_dictation(out);
        }
        // The images go with the card (decision 9.5).
        self.blobs.clear(out);
    }

    fn scratch(&mut self, out: &mut Vec<Output>) {
        let now = self.now;
        let quiet = self.quiet();
        if let Some(d) = self.dictation.as_mut() {
            d.parts.pop();
            if quiet {
                d.quiet_from = Some(now);
            }
        }
        self.show_dictation(out);
    }

    fn read_back(&mut self, out: &mut Vec<Output>) {
        // Nothing is said with sound off, so nothing waits for it.
        if !self.settings.sound {
            return;
        }
        let Some(d) = self.dictation.as_mut() else {
            return;
        };
        if d.parts.is_empty() {
            out.push(event(Event::Say("The message is empty.".to_string())));
            return;
        }
        d.read_back = ReadBack::Asked;
        let text = d.text();
        out.push(event(Event::Say(text)));
        self.show_dictation(out);
    }

    /// Speech started or ended: a "read it back" ends, the timer starts
    /// again from its end, and the gate is armed for more.
    fn read_back_speaking(&mut self, on: bool, out: &mut Vec<Output>) {
        let now = self.now;
        let Some(d) = self.dictation.as_mut() else {
            return;
        };
        match (d.read_back, on) {
            (ReadBack::Asked, true) => d.read_back = ReadBack::Speaking,
            (ReadBack::Asked | ReadBack::Speaking, false) => {
                d.read_back = ReadBack::No;
                d.quiet_from = Some(now);
                out.push(event(Event::Arm));
            }
            _ => {}
        }
        self.show_dictation(out);
    }

    /// Submits the message (decision 5): one `agents.send` of kind
    /// `message`, which the daemon confirms.
    fn submit(&mut self, by_button: bool, out: &mut Vec<Output>) {
        let online = self.online();
        let unheard = self.unheard();
        let Some(d) = self.dictation.as_mut() else {
            return;
        };
        if d.phase != DictationPhase::Collecting {
            return;
        }
        // Speech still on its way from STT: submit once it lands.
        if unheard {
            log::info!("dictation: submit waits for what was said");
            d.submit_waiting = Some(by_button);
            return;
        }
        if d.general_agent {
            d.submit_waiting = None;
            d.quiet_from = None;
            if d.parts.is_empty() {
                out.push(event(Event::Say("The message is empty.".to_string())));
                self.dictation = None;
                self.show_dictation(out);
                return;
            }
            let text = d.text();
            if d.mode == PromptMode::Voice {
                self.voice_said = Some(common::SaidInput {
                    text: text.clone(),
                    pane_id: None,
                });
            }
            self.dictation = None;
            self.show_dictation(out);
            self.start_turn(text, out);
            return;
        }
        d.submit_waiting = None;
        // A refused submit waits for the next change before the silence
        // tries again.
        d.quiet_from = None;
        let refusal = if d.parts.is_empty() {
            Some("The message is empty.")
        } else if !online {
            Some("I'm not connected to flankr.")
        } else {
            match &d.target {
                None => Some("No agent is focused. Say focus, and the agent's name."),
                Some(t) if t.agent.is_none() => {
                    Some("The focused pane has no agent. Say focus, and the agent's name.")
                }
                Some(_) => None,
            }
        };
        if let Some(why) = refusal {
            out.push(event(Event::Say(why.to_string())));
            self.show_dictation(out);
            return;
        }
        let target = d.target.clone().expect("checked above");
        let text = d.text();
        // Images in the text go as `parts` (plan 20261028_05, decision
        // 9.4), once every one is uploaded.
        let parts = match self.blobs.parts(&text, &target.pane_id) {
            Some(Err(why)) => {
                out.push(event(Event::Say(why)));
                self.show_dictation(out);
                return;
            }
            Some(Ok(parts)) => Some(parts),
            None => None,
        };
        let Some(d) = self.dictation.as_mut() else {
            return;
        };
        d.phase = DictationPhase::Submitting;
        d.own_approval = true;
        if d.mode == PromptMode::Voice {
            self.voice_said = Some(common::SaidInput {
                text: text.clone(),
                pane_id: Some(target.pane_id.clone()),
            });
        }
        log::info!(
            "dictation: submitted by {}",
            if by_button {
                "the button"
            } else {
                "the silence"
            }
        );
        let mut params = serde_json::json!({
            "pane_id": target.pane_id,
            "text": text,
            "kind": common::SendKind::Message,
            "handle": message_handle(&target, &text),
        });
        if let Some(parts) = parts {
            params["text"] = serde_json::json!("");
            params["parts"] = serde_json::json!(parts);
        }
        // The Submit button is the approval (plan 20261023_02, the 13:25
        // EDT amendment); a submit by silence keeps its own approval.
        if by_button {
            params["confirmed"] = serde_json::json!(true);
        }
        self.interact(&target.pane_id);
        out.push(self.request(methods::AGENTS_SEND, params, Sent::Message));
        self.show_dictation(out);
    }

    /// Sends a suggested response to the open view's pane. The tap is the
    /// approval: its confirmation is approved without asking.
    fn respond(&mut self, response: Suggestion, out: &mut Vec<Output>) {
        let Some(pane_id) = self.view.as_ref().map(|v| v.pane_id.clone()) else {
            return;
        };
        // A subagent takes no input from the pane: its answers are
        // inactive (the subagents plan, decision 5).
        if self.view.as_ref().is_some_and(|v| v.child.is_some()) {
            log::info!("respond: a subagent's question is answered through its parent");
            return;
        }
        if !self.online() {
            out.push(event(Event::Say(
                "I'm not connected to flankr.".to_string(),
            )));
            return;
        }
        self.interact(&pane_id);
        let (word, kind) = response.send();
        // A Yes or No answering the agent's question says what it answers;
        // a plan answer names the plan.
        let pending = self.view.as_ref().and_then(|v| v.pending_question.as_ref());
        let question = pending.map(|q| q.text.clone());
        let plan = pending.and_then(|q| q.plan.clone());
        if response == Suggestion::Discuss {
            // A message in reply to the card, its start already in it, to
            // dictate the rest.
            let start = plan_answer(response, plan.as_deref()).unwrap_or_default();
            return self.start_dictation_with(vec![start], out);
        }
        let text = match (response, &question) {
            (Suggestion::Yes | Suggestion::No, Some(q)) => {
                answer_text(response == Suggestion::Yes, q)
            }
            (Suggestion::Approve | Suggestion::Implement, _) => {
                plan_answer(response, plan.as_deref()).unwrap_or_else(|| word.to_string())
            }
            _ => word.to_string(),
        };
        self.send_response(
            pane_id,
            response.label(),
            text,
            kind,
            response.answers(),
            out,
        );
    }

    /// Sends a tapped answer of the open view's question (plan
    /// 20261027_06, step 4): its `sends` as a message, or as a command
    /// with `command`, confirmed; or, with `starts`, a message in reply to
    /// the card with that start, to dictate the rest. A `sends` of just
    /// "yes" or "no" says what it answers, as the Yes and No buttons do.
    fn suggested(&mut self, answer: common::Answer, out: &mut Vec<Output>) {
        let Some(pane_id) = self.view.as_ref().map(|v| v.pane_id.clone()) else {
            return;
        };
        if let Some(start) = answer.starts.filter(|s| !s.trim().is_empty()) {
            if !self.online() {
                out.push(event(Event::Say(
                    "I'm not connected to flankr.".to_string(),
                )));
                return;
            }
            self.interact(&pane_id);
            return self.start_dictation_with(vec![start], out);
        }
        let Some(sends) = answer.sends.filter(|s| !s.trim().is_empty()) else {
            return;
        };
        if !self.online() {
            out.push(event(Event::Say(
                "I'm not connected to flankr.".to_string(),
            )));
            return;
        }
        self.interact(&pane_id);
        let question = self
            .view
            .as_ref()
            .and_then(|v| v.pending_question.as_ref())
            .map(|q| q.text.clone());
        let lower = sends.trim().to_ascii_lowercase();
        let text = match (&question, lower.as_str()) {
            (Some(q), "yes" | "no") if !answer.command => answer_text(lower == "yes", q),
            _ => sends,
        };
        let kind = if answer.command {
            common::SendKind::Command
        } else {
            common::SendKind::Message
        };
        self.send_response(pane_id, &answer.label, text, kind, answer.answers, out);
    }

    /// A choice question's answers (plan 20261029_01, step 5):
    /// `choice.answer` for the open view's pane. A tap is the confirmation;
    /// the daemon checks the rest. One at a time: a second while one is on
    /// its way is refused here, as the daemon would.
    fn choose(&mut self, id: String, answers: Vec<common::ChoiceAnswer>, out: &mut Vec<Output>) {
        let Some(pane_id) = self.view.as_ref().map(|v| v.pane_id.clone()) else {
            out.push(event(choice_failed(
                id,
                String::new(),
                "no session is open".to_string(),
                None,
            )));
            return;
        };
        if self.view.as_ref().is_some_and(|v| v.child.is_some()) {
            out.push(event(choice_failed(
                id,
                pane_id,
                "a subagent takes no answers; answer the main agent".to_string(),
                Some(libflankr_proto::envelope::codes::NOT_PERMITTED),
            )));
            return;
        }
        if !self.online() {
            out.push(event(choice_failed(
                id,
                pane_id,
                "offline".to_string(),
                None,
            )));
            return;
        }
        if self.choosing.is_some() {
            out.push(event(choice_failed(
                id,
                pane_id,
                "an answer is already on its way".to_string(),
                Some(libflankr_proto::envelope::codes::NOT_PERMITTED),
            )));
            return;
        }
        log::info!(
            "choice.answer: {} answer(s) for {id} in {pane_id}",
            answers.len()
        );
        let params = common::ChoiceAnswerParams {
            pane_id: pane_id.clone(),
            id: id.clone(),
            answers,
        };
        let sent = Sent::Choose(id.clone(), pane_id.clone());
        let request = self.request(methods::CHOICE_ANSWER, params, sent);
        if let Output::Send(r) = &request {
            self.choosing = Some((r.id.clone(), id, pane_id, self.now));
        }
        out.push(request);
    }

    /// `choice.answer`'s result for the set `id`. Answered, the question's
    /// card goes at once, as an answered text question's does.
    fn choice_answered(
        &mut self,
        id: String,
        pane_id: String,
        result: Value,
        out: &mut Vec<Output>,
    ) {
        let (outcome, got) = match parse::<common::ChoiceAnswerResult>(result) {
            Ok(common::ChoiceAnswerResult::Answered) => {
                if let Some(v) = self.view.as_mut()
                    && let Some(q) = v
                        .pending_question
                        .as_ref()
                        .filter(|q| q.choices.as_ref().is_some_and(|c| c.id == id))
                {
                    v.replied_to = Some(q.ts.clone());
                }
                self.recompute_view(out);
                ("answered", Vec::new())
            }
            Ok(common::ChoiceAnswerResult::Mismatch { got }) => ("mismatch", got),
            Ok(common::ChoiceAnswerResult::NoResult) => ("no_result", Vec::new()),
            Ok(other) => {
                log::warn!("choice.answer: a result this client does not know: {other:?}");
                ("no_result", Vec::new())
            }
            Err(e) => {
                out.push(event(choice_failed(
                    id,
                    pane_id,
                    format!("a bad choice.answer result: {e}"),
                    None,
                )));
                return;
            }
        };
        log::info!("choice.answer: {outcome}");
        out.push(event(Event::ChoiceAnswered {
            id,
            pane_id,
            outcome: outcome.to_string(),
            got,
            code: None,
        }));
    }

    /// Sends `text` to `pane_id` as a tapped response labelled `label`,
    /// confirmed; with `answers`, the question's card goes at once.
    fn send_response(
        &mut self,
        pane_id: String,
        label: &str,
        text: String,
        kind: common::SendKind,
        answers: bool,
        out: &mut Vec<Output>,
    ) {
        // The card goes at once, and the answer shows as typed input until
        // the session has it (owner, 2026-10-23: "tapping yes/no on a card
        // should dismiss the card on the phone side (so immediate)"; "convert
        // the responses to typed amber text with the input box").
        if let Some(v) = self.view.as_mut() {
            if answers {
                v.replied_to = v.pending_question.as_ref().map(|q| q.ts.clone());
            }
            v.typed = Some(text.clone());
        }
        // An answer to a question being read aloud skips its reading.
        if let Some(ts) = self.view.as_ref().and_then(|v| v.replied_to.clone()) {
            self.skip_reading(&ts, out);
        }
        self.recompute_view(out);
        let agent = self.agents.iter().find(|a| a.pane_id == pane_id);
        let target = common::FocusInfo {
            pane_id: pane_id.clone(),
            project: agent.and_then(|a| a.project.clone()),
            agent: agent.map(|a| a.agent.clone()),
            tab_label: None,
            tab_number: None,
            name: String::new(),
            spoken: String::new(),
        };
        let mut params = serde_json::json!({
            "pane_id": pane_id,
            "text": text,
            "handle": label_handle(&target, label),
        });
        if kind == common::SendKind::Message {
            params["kind"] = serde_json::json!(kind);
        }
        // The tap is the approval (plan 20261023_02, the 13:25 EDT
        // amendment). An older daemon ignores it and asks, and the tap is
        // matched to that confirmation below, as before.
        params["confirmed"] = serde_json::json!(true);
        self.tapped.push(Tapped {
            pane_id: pane_id.clone(),
            text: text.to_string(),
            confirm_id: None,
        });
        let sent = Sent::Respond(label.to_string(), text);
        out.push(self.request(methods::AGENTS_SEND, params, sent));
    }

    /// An utterance while dictating (decision 2, as amended).
    fn dictation_utterance(&mut self, text: String, out: &mut Vec<Output>) {
        let words = control_words(&text);
        match words.as_str() {
            "cancel" | "clear" => return self.clear_dictation(out),
            "scratch that" => return self.scratch(out),
            "read it back" => return self.read_back(out),
            "use the focus" => {
                let focus = self.focus.clone();
                if let Some(d) = self.dictation.as_mut() {
                    d.target = focus;
                    d.refresh_target = false;
                    d.fixed_pane = None;
                }
                self.show_dictation(out);
                out.push(event(Event::Arm));
                return;
            }
            // Answers with nothing to answer, and the start words again:
            // never text.
            "approve" | "approved" | "deny" | "take a message" | "dictate" => return,
            _ => {}
        }
        let Some(d) = self.dictation.as_mut() else {
            return;
        };
        if !d.has_target() && words.starts_with("focus ") && self.link == Link::Online {
            // "focus <agent>" sets a missing target, through agents.focus.
            let target = text.trim()[text.trim().find(char::is_whitespace).unwrap_or(0)..]
                .trim()
                .trim_end_matches(['.', '?', '!'])
                .to_string();
            let params = common::AgentsFocusParams {
                target_id: target.clone(),
            };
            out.push(self.request(methods::AGENTS_FOCUS, params, Sent::Focus(target)));
            return;
        }
        if d.phase == DictationPhase::Submitting {
            log::debug!("dictation: dropping an utterance while submitting");
            return;
        }
        let joined = d.text().chars().count();
        let added = text.chars().count() + usize::from(joined > 0);
        if joined + added > MESSAGE_CAP {
            out.push(event(Event::Say("The message is full.".to_string())));
            return;
        }
        let (quiet, now) = (self.quiet(), self.now);
        let d = self.dictation.as_mut().expect("dictating");
        d.parts.push(text);
        if quiet && d.quiet_from.is_none() {
            d.quiet_from = Some(now);
        }
        self.show_dictation(out);
        // A Submit that waited for this sentence goes now.
        if let Some(by_button) = d_waiting(&self.dictation).filter(|_| !self.unheard()) {
            return self.submit(by_button, out);
        }
        // Hands free: listen for the next sentence.
        out.push(event(Event::Arm));
    }

    /// herdr's focus, learned from `agents.list` or `focus.changed`. A
    /// message with no target takes it.
    fn set_focus(&mut self, focus: Option<common::FocusInfo>, out: &mut Vec<Output>) {
        if focus != self.focus {
            self.focus = focus.clone();
            out.push(event(Event::Focus(focus.clone())));
        }
        if let Some(d) = self.dictation.as_mut().filter(|d| d.fixed_pane.is_none())
            && (!d.has_target() || d.refresh_target)
        {
            d.target = focus;
        }
        self.show_dictation(out);
    }

    /// The own message's confirmation ended.
    fn message_resolved(&mut self, outcome: &str, out: &mut Vec<Output>) {
        if outcome == "sent" {
            // Sent to the viewed agent: it shows as typed input until the
            // session has it (owner, 2026-10-23: "Spoken messages should be
            // included as text in the flow after they're submitted").
            if let Some(d) = self.dictation.take()
                && let Some(v) = self.view.as_mut()
                && d.target.as_ref().is_some_and(|t| t.pane_id == v.pane_id)
            {
                v.typed = Some(d.text());
                self.show_view(out);
            }
        } else if let Some(d) = self.dictation.as_mut() {
            // Denied, expired, failed: the text is kept to be fixed and
            // submitted again, after the next change.
            d.phase = DictationPhase::Collecting;
            d.confirm_id = None;
            d.quiet_from = None;
            if outcome == "failed" {
                out.push(event(Event::Say("Sending the message failed.".to_string())));
            }
        }
        self.show_dictation(out);
    }

    /// A message from the daemon.
    pub fn incoming(&mut self, message: Incoming) -> Vec<Output> {
        let mut out = Vec::new();
        // Anything from the daemon shows the link is alive.
        self.last_heard = self.now;
        self.ping_sent = None;
        match message {
            Incoming::Response(response) => self.response(response, &mut out),
            Incoming::Notification(note) => self.notification(note, &mut out),
            Incoming::Request(request) => {
                log::warn!("ignoring a request from the daemon: {}", request.method);
            }
        }
        self.show_cards(&mut out);
        // Speech held for voice input goes once it ends (plan 20261023_02,
        // the 2026-10-30 13:37 EDT amendment, item 3).
        self.flush_notices(&mut out);
        self.gate(out)
    }

    fn tap(&mut self, out: &mut Vec<Output>) {
        if !self.online() {
            out.push(event(Event::Error("not connected to flankr".to_string())));
        } else if !self.confirms.is_empty() {
            out.push(event(Event::Arm));
        } else if let Some(turn) = self.turn.as_mut() {
            out.push(event(Event::StopSpeaking));
            if !turn.tapped {
                turn.tapped = true;
                let params = wire::TurnInterruptParams { turn_id: turn.id };
                out.push(self.request(wire::methods::TURN_INTERRUPT, params, Sent::Interrupt));
            }
        } else {
            out.push(event(Event::Arm));
        }
    }

    fn utterance(&mut self, text: String, out: &mut Vec<Output>) {
        self.voice_ended = None;
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        // A late utterance from before the microphone went off.
        if !self.settings.microphone {
            log::debug!("ignoring an utterance: the microphone is off");
            return;
        }
        if !self.online() {
            out.push(event(Event::Error("not connected to flankr".to_string())));
        } else if !self.confirms.is_empty() {
            let approve = is_approval(&text);
            if self.dictation.is_some() {
                // "approve" and "deny" answer; anything else denies, then
                // is dictation.
                self.answer(approve, None, out);
                if !approve && control_words(&text) != "deny" {
                    self.dictation_utterance(text, out);
                }
            } else {
                self.answer(approve, (!approve).then_some(text), out);
            }
        } else if self.dictation.is_some() {
            self.dictation_utterance(text, out);
        } else if self.turn.is_none() && is_dictation_start(&text) {
            self.start_dictation(out);
        } else if self.turn.is_some() {
            // The gate is closed while waiting: this can only be late.
            log::debug!("ignoring an utterance during a turn: {text:?}");
        } else {
            self.start_turn(text, out);
        }
    }

    /// Answers the confirmation being asked. A denying utterance is then
    /// started as a turn.
    fn answer(&mut self, approve: bool, then: Option<String>, out: &mut Vec<Output>) {
        let Some(confirm) = self.confirms.pop_front() else {
            log::debug!("no confirmation to answer");
            return;
        };
        self.interact(&confirm.pane_id);
        let params = common::ConfirmReplyParams {
            confirm_id: confirm.id,
            approve,
            nonce: None,
        };
        out.push(event(Event::Answered {
            confirm_id: confirm.id,
            approve,
        }));
        out.push(self.request(
            methods::CONFIRM_REPLY,
            params,
            Sent::ConfirmReply(confirm.id),
        ));
        if let Some(text) = then {
            if self.turn.is_some() {
                self.deferred = Some(text);
            } else {
                self.start_turn(text, out);
            }
        }
        self.ask_next(out);
        self.recompute_view(out);
        self.flush_notices(out);
    }

    fn start_turn(&mut self, text: String, out: &mut Vec<Output>) {
        if !self.general_agent() {
            // No Gemini (plan 20261024_02, the 11:37 EDT amendment), by the
            // toggle or the daemon's `[llm]` (plan 20261027_02): in the
            // agent view, what was said is a message to its agent; on the
            // overview, nothing.
            if self.view.is_some() {
                self.start_dictation_with(vec![text], out);
            } else {
                log::info!("the general agent is off: no turn");
            }
            return;
        }
        let id = self.next_turn;
        self.next_turn += 1;
        self.turn = Some(Turn { id, tapped: false });
        out.push(event(Event::Conversation {
            speaker: Speaker::User,
            text: text.clone(),
        }));
        // From the agent view, its session is the turn's current agent
        // (plan 20261027_04, decision 3 and step 6); none from the
        // overview.
        let params = wire::TurnStartParams {
            turn_id: id,
            text,
            session: self.view.as_ref().map(|v| v.pane_id.clone()),
        };
        out.push(self.request(wire::methods::TURN_START, params, Sent::Turn(id)));
    }

    fn response(&mut self, response: Response, out: &mut Vec<Output>) {
        let Some(sent) = response.id.as_ref().and_then(|id| self.sent.remove(id)) else {
            log::warn!("a response to no request of ours: {:?}", response.id);
            return;
        };
        if let Some(error) = response.error {
            let message = format!("{} ({})", error.message, error.code);
            match sent {
                Sent::Hello => {
                    out.push(event(Event::Error(format!("hello refused: {message}"))));
                    out.push(Output::Close(message));
                }
                Sent::PluginCall { ns, id } => {
                    log::info!("plugin.call {ns}: {message}");
                    if let Some(id) = id {
                        self.plugin_waiting.retain(|w| !(w.0 == ns && w.1 == id));
                        out.push(event(Event::PluginResult(wire::PluginResult {
                            ns,
                            id,
                            result: None,
                            error: Some(error),
                        })));
                    }
                }
                Sent::PluginSetEnabled { ns } => {
                    log::warn!("plugin.set_enabled {ns}: {message}");
                    out.push(event(Event::Error(error.message)));
                    out.push(event(Event::Plugins(self.plugins.clone())));
                }
                Sent::Turn(id) => {
                    out.push(event(Event::Error(message)));
                    self.end_turn(id, "error".to_string(), Vec::new(), out);
                }
                Sent::Interrupt => out.push(event(Event::Error(message))),
                Sent::ConfirmReply(confirm_id) => out.push(event(Event::ReplyFailed {
                    confirm_id,
                    message,
                })),
                Sent::List => {
                    self.listing = false;
                    log::warn!("agents.list: {message}");
                }
                Sent::Options => log::warn!("client.options: {message}"),
                Sent::Peek { .. } => log::info!("session.peek: {message}"),
                Sent::Ping => {}
                Sent::Unpair => {
                    log::warn!("link.unpair: {message}; forgetting the pairing here");
                    if let Some((_, id)) = self.unpairing.take() {
                        out.push(Output::Remove { id, daemon: false });
                    }
                }
                sent @ Sent::Content { .. } => {
                    // The session changed under the range: the reset that
                    // follows brings the new outline.
                    if error.code != libflankr_proto::envelope::codes::SESSION_CHANGED {
                        log::warn!("session.content: {message}");
                    }
                    self.content_answered(sent, None, out);
                }
                Sent::Summarize(serial) => {
                    log::info!("speech.summarize: {message}; reading the reply here");
                    self.summarized(serial, None, out);
                }
                Sent::CatchUp(serial) => {
                    log::info!("speech.catch_up: {message}; the held speech is said as it is");
                    self.caught_up(serial, None);
                }
                Sent::Subscribe(pane_id, child) => {
                    if let Some(v) = self
                        .view
                        .as_mut()
                        .filter(|v| v.pane_id == pane_id && v.child == child)
                    {
                        v.subscribing = false;
                        v.answered = true;
                    }
                    out.push(event(Event::Error(format!("session.subscribe: {message}"))));
                    self.show_view(out);
                    if let Some(r) = self.refresh.as_mut() {
                        r.subscribed = true;
                        r.fetched = true;
                    }
                    self.finish_refresh(out);
                }
                Sent::Unsubscribe => log::debug!("session.unsubscribe: {message}"),
                Sent::TerminalOpen(session) => {
                    let no_method =
                        error.code == libflankr_proto::envelope::codes::METHOD_NOT_FOUND;
                    let acts = self
                        .terminals
                        .update(|b| b.open_failed(&session, message, no_method));
                    self.terminal_acts(acts, out);
                }
                Sent::TerminalClose => log::debug!("terminal.close: {message}"),
                Sent::TerminalPaste { .. } => {
                    let line = if error.code == libflankr_proto::envelope::codes::METHOD_NOT_FOUND {
                        "This flankr can't paste into the terminal.".to_string()
                    } else {
                        format!("terminal.paste: {message}")
                    };
                    out.push(event(Event::Error(line)));
                }
                Sent::TerminalArm(session) => {
                    let acts = self.terminals.update(|b| b.armed(&session, Err(message)));
                    self.terminal_acts(acts, out);
                }
                Sent::TerminalInput(session) => {
                    let acts = self
                        .terminals
                        .update(|b| b.keys_sent(&session, Err(message)));
                    self.terminal_acts(acts, out);
                }
                Sent::TerminalScroll(session) => {
                    let acts = self
                        .terminals
                        .update(|b| b.scrolled(&session, Err(message)));
                    self.terminal_acts(acts, out);
                }
                Sent::TerminalResize(session) => {
                    let no_method =
                        error.code == libflankr_proto::envelope::codes::METHOD_NOT_FOUND;
                    let acts = self
                        .terminals
                        .update(|b| b.resized(&session, Err(message), no_method));
                    self.terminal_acts(acts, out);
                }
                Sent::TerminalMouse(session) => {
                    let acts = self
                        .terminals
                        .update(|b| b.mouse_sent(&session, Err(message)));
                    self.terminal_acts(acts, out);
                }
                Sent::History(id) => {
                    if let Some(v) = self.view.as_mut().filter(|v| v.subscription == Some(id)) {
                        v.history = false;
                    }
                    // The session changed under the page: the reset that
                    // follows brings the new one.
                    if error.code != libflankr_proto::envelope::codes::SESSION_CHANGED {
                        log::warn!("session.history: {message}");
                    }
                    self.show_view(out);
                }
                Sent::Focus(target) => {
                    log::info!("dictation: focus {target:?}: {message}");
                    out.push(event(Event::Say(format!("I couldn't focus {target}."))));
                }
                Sent::Choose(id, pane_id) => {
                    log::info!("choice.answer: {message}");
                    self.choosing = None;
                    out.push(event(choice_failed(
                        id,
                        pane_id,
                        error.message.clone(),
                        Some(error.code),
                    )));
                }
                Sent::Respond(label, text) => {
                    // Not sent: the card comes back and the input goes.
                    if let Some(v) = self.view.as_mut() {
                        v.replied_to = None;
                        v.typed = None;
                    }
                    self.recompute_view(out);
                    self.tapped
                        .retain(|t| t.confirm_id.is_some() || t.text != text);
                    out.push(event(Event::Error(message)));
                    out.push(event(Event::Say(format!(
                        "I can't send {}. {}",
                        label, error.message
                    ))));
                }
                Sent::Message => {
                    // Refused, such as a character outside the filter: the
                    // message is kept to be fixed.
                    if let Some(d) = self.dictation.as_mut() {
                        d.phase = DictationPhase::Collecting;
                        d.quiet_from = None;
                    }
                    out.push(event(Event::Error(message)));
                    out.push(event(Event::Say(format!(
                        "I can't send that message. {}",
                        error.message
                    ))));
                    self.show_dictation(out);
                }
            }
            return;
        }
        let result = response.result.unwrap_or(Value::Null);
        match sent {
            Sent::TerminalOpen(session) => {
                let acts = match parse::<common::TerminalOpenResult>(result) {
                    Ok(r) => self.terminals.update(|b| b.opened(&session, r)),
                    Err(e) => self
                        .terminals
                        .update(|b| b.open_failed(&session, format!("a bad result: {e}"), false)),
                };
                self.terminal_acts(acts, out);
            }
            Sent::TerminalClose => {}
            Sent::TerminalPaste { images } => self.blobs.used(&images, out),
            Sent::TerminalArm(session) => {
                let secs = parse::<common::TerminalInputArmResult>(result)
                    .map(|r| r.expires_in_s)
                    .map_err(|e| format!("a bad result: {e}"));
                let acts = self.terminals.update(|b| b.armed(&session, secs));
                self.terminal_acts(acts, out);
            }
            Sent::TerminalInput(session) => {
                let acts = self.terminals.update(|b| b.keys_sent(&session, Ok(())));
                self.terminal_acts(acts, out);
            }
            Sent::TerminalScroll(session) => {
                let r = parse::<common::TerminalScrollResult>(result)
                    .map_err(|e| format!("a bad result: {e}"));
                let acts = self.terminals.update(|b| b.scrolled(&session, r));
                self.terminal_acts(acts, out);
            }
            Sent::TerminalResize(session) => {
                let acts = self
                    .terminals
                    .update(|b| b.resized(&session, Ok(()), false));
                self.terminal_acts(acts, out);
            }
            Sent::TerminalMouse(session) => {
                let acts = self.terminals.update(|b| b.mouse_sent(&session, Ok(())));
                self.terminal_acts(acts, out);
            }
            // Handed to the plugin: its answer follows as `plugin.result`.
            Sent::PluginCall { .. } => {}
            Sent::PluginSetEnabled { ns } => {
                match parse::<wire::PluginSetEnabledResult>(result) {
                    Ok(r) => {
                        if let Some(p) = self.plugins.iter_mut().find(|p| p.ns == ns) {
                            p.enabled = r.enabled;
                        }
                    }
                    Err(e) => log::warn!("a bad plugin.set_enabled result: {e}"),
                }
                out.push(event(Event::Plugins(self.plugins.clone())));
            }
            Sent::Hello => match parse::<wire::HelloResult>(result) {
                Ok(hello) => {
                    self.link = Link::Online;
                    if hello.session_id.is_some() {
                        self.session_id = hello.session_id;
                    }
                    self.server_name = hello.server.clone();
                    self.vocabulary = hello.vocabulary.clone();
                    out.push(event(Event::Connected {
                        server: hello.server,
                    }));
                    out.push(event(Event::Vocabulary(hello.vocabulary)));
                    // What `[llm]` offers; an older daemon's is both.
                    let llm = hello.llm.unwrap_or_else(wire::LlmInfo::available);
                    self.set_llm(llm, out);
                    // The plugins; an older daemon has none.
                    self.set_plugins(hello.plugins.unwrap_or_default(), out);
                    // Binary transfers, if offered: uploads waiting start.
                    self.blobs.hello(hello.blob, self.now, out);
                    // The terminal mode, if offered: what was open opens
                    // again (plan 20261027_07, step 4).
                    let acts = self.terminals.update(|b| b.set_info(hello.terminal));
                    self.terminal_acts(acts, out);
                    // The connection's options start off: say read aloud's.
                    self.send_options(out);
                    // herdr's focus, for the dictation mode's target.
                    self.ask_list(out);
                    // An open view subscribes again, from what it holds.
                    self.subscribe(out);
                }
                Err(e) => {
                    let message = format!("a bad hello result: {e}");
                    out.push(event(Event::Error(message.clone())));
                    out.push(Output::Close(message));
                }
            },
            Sent::Turn(id) => match parse::<wire::TurnResult>(result) {
                Ok(r) => {
                    let stopped = r.interrupted.into_iter().map(|i| i.spoken).collect();
                    self.end_turn(id, r.outcome.as_str().to_string(), stopped, out);
                }
                Err(e) => {
                    out.push(event(Event::Error(format!("a bad turn result: {e}"))));
                    self.end_turn(id, "error".to_string(), Vec::new(), out);
                }
            },
            Sent::List => match parse::<common::AgentsListResult>(result) {
                Ok(list) => {
                    self.listing = false;
                    self.ding_finished(&list.agents, out);
                    if !self.listed || list.agents != self.agents {
                        out.push(event(Event::Agents(list.agents.clone())));
                        self.new_list = true;
                    }
                    self.listed = true;
                    self.agents = list.agents;
                    self.list_learned(out);
                    self.set_focus(list.focus, out);
                    if let Some(d) = self.dictation.as_mut() {
                        d.refresh_target = false;
                    }
                }
                Err(e) => {
                    self.listing = false;
                    log::warn!("a bad agents.list result: {e}");
                }
            },
            // Focused: learn the focus, which a message without a target
            // takes.
            Sent::Focus(_) => {
                self.ask_list(out);
            }
            Sent::Message => match parse::<common::AgentsSendResult>(result) {
                Ok(common::AgentsSendResult::Pending { confirm_id, .. }) => {
                    // The daemon used the images' references.
                    self.blobs.clear(out);
                    if let Some(d) = self.dictation.as_mut() {
                        d.phase = DictationPhase::Confirming;
                        d.confirm_id = Some(confirm_id);
                    }
                    // Its question may have arrived first.
                    if self.confirms.iter().any(|c| c.id == confirm_id) {
                        self.approve_own_message(confirm_id, out);
                    }
                    self.show_dictation(out);
                }
                Ok(common::AgentsSendResult::Sent) => {
                    self.blobs.clear(out);
                    self.message_resolved("sent", out);
                }
                Err(e) => {
                    out.push(event(Event::Error(format!(
                        "a bad agents.send result: {e}"
                    ))));
                    self.message_resolved("failed", out);
                }
            },
            Sent::Choose(id, pane_id) => {
                self.choosing = None;
                self.choice_answered(id, pane_id, result, out);
            }
            Sent::Respond(_, text) => match parse::<common::AgentsSendResult>(result) {
                Ok(common::AgentsSendResult::Pending { confirm_id, .. }) => {
                    let queued = self.confirms.iter().any(|c| c.id == confirm_id);
                    if let Some(i) = self
                        .tapped
                        .iter()
                        .position(|t| t.confirm_id.is_none() && t.text == text)
                    {
                        if queued {
                            // Its question came first and is queued.
                            self.tapped.remove(i);
                            self.approve_own(confirm_id, out);
                        } else {
                            self.tapped[i].confirm_id = Some(confirm_id);
                        }
                    }
                }
                Ok(common::AgentsSendResult::Sent) => {
                    self.tapped
                        .retain(|t| t.confirm_id.is_some() || t.text != text);
                }
                Err(e) => out.push(event(Event::Error(format!(
                    "a bad agents.send result: {e}"
                )))),
            },
            Sent::Subscribe(pane_id, child) => {
                match parse::<common::SessionSubscribeResult>(result) {
                    Ok(r) => {
                        let Some(v) = self
                            .view
                            .as_mut()
                            .filter(|v| v.pane_id == pane_id && v.child == child)
                        else {
                            // The view closed or moved on while it was asked.
                            let params = common::SessionUnsubscribeParams {
                                subscription_id: r.subscription_id,
                            };
                            out.push(self.request(
                                methods::SESSION_UNSUBSCRIBE,
                                params,
                                Sent::Unsubscribe,
                            ));
                            return;
                        };
                        v.subscribing = false;
                        v.subscription = Some(r.subscription_id);
                        self.delivered(r.delivery, out);
                        self.refresh_subscribed(out);
                    }
                    Err(e) => {
                        if let Some(v) = self
                            .view
                            .as_mut()
                            .filter(|v| v.pane_id == pane_id && v.child == child)
                        {
                            v.subscribing = false;
                        }
                        log::warn!("a bad session.subscribe result: {e}");
                    }
                }
            }
            Sent::History(id) => match parse::<common::SessionHistoryResult>(result) {
                Ok(page) => self.paged(id, page, out),
                Err(e) => {
                    if let Some(v) = self.view.as_mut().filter(|v| v.subscription == Some(id)) {
                        v.history = false;
                    }
                    log::warn!("a bad session.history result: {e}");
                }
            },
            Sent::Unsubscribe => {}
            Sent::CatchUp(serial) => match parse::<common::SpeechCatchUpResult>(result) {
                Ok(r) => {
                    if r.summary.is_none() {
                        log::info!("speech.catch_up: none made; the held speech is said as it is");
                    }
                    self.caught_up(serial, r.summary);
                }
                Err(e) => {
                    log::warn!("a bad speech.catch_up result: {e}");
                    self.caught_up(serial, None);
                }
            },
            Sent::Summarize(serial) => match parse::<common::SpeechSummarizeResult>(result) {
                // Gemini's version when there is one, else the rules'.
                Ok(r) => self.summarized(serial, r.summary.or(Some(r.read)), out),
                Err(e) => {
                    log::warn!("a bad speech.summarize result: {e}");
                    self.summarized(serial, None, out);
                }
            },
            sent @ Sent::Content { .. } => match parse::<common::SessionContentResult>(result) {
                Ok(r) => self.content_answered(sent, Some(r), out),
                Err(e) => {
                    log::warn!("a bad session.content result: {e}");
                    self.content_answered(sent, None, out);
                }
            },
            Sent::Interrupt | Sent::ConfirmReply(_) | Sent::Options | Sent::Ping => {}
            Sent::Peek { pane_id, line } => match parse::<common::SessionPeekResult>(result) {
                Ok(r) => self.peeked(&pane_id, line, r),
                Err(e) => log::warn!("a bad session.peek result: {e}"),
            },
            Sent::Unpair => {
                // The daemon forgot this device and closes the link.
                log::info!("unpairing: the daemon forgot this device");
                if let Some((_, id)) = self.unpairing.take() {
                    out.push(Output::Remove { id, daemon: true });
                }
            }
        }
    }

    /// To the background and back (owner, 2026-10-23: "in any case it should
    /// be a single sleeping socket"). Back in front: one catch-up list (the
    /// daemon itself sends the open view what it held), then the usual
    /// timers.
    fn set_background(&mut self, on: bool, out: &mut Vec<Output>) {
        if on == self.background {
            return;
        }
        self.background = on;
        log::info!(
            "link: {}",
            if on {
                "sleeping in the background"
            } else {
                "awake"
            }
        );
        // The daemon holds back all but what needs the user while asleep;
        // on resume a ping says at once that the user is back.
        if self.online() {
            self.send_options(out);
            if !on {
                // Timed, so a dead socket is closed and made again within
                // PING_WAIT_RESUME, not left to the network to notice.
                let params = common::PingParams { active: Some(true) };
                out.push(self.request(methods::PING, params, Sent::Ping));
                self.ping_sent = Some(self.now);
                self.ping_wait = PING_WAIT_RESUME;
            }
        }
        // The daemon holds the view's deliveries while the link sleeps and
        // sends the catch-up itself on waking.
        if !on {
            // Back in front: memory pressure is over.
            if let Some(v) = self.view.as_mut()
                && v.pressure.take().is_some()
            {
                log::info!("memory: back in front; contents fetched as usual");
                self.fetch_contents(out);
            }
            self.list_wanted = true;
            self.listed_at = None;
            self.list_if_due(out);
        }
    }

    /// After [`PING_IDLE`] with nothing from the daemon, a `ping`; with no
    /// answer in [`PING_WAIT`], the link is taken as dead and closed, to be
    /// made again (owner, 2026-10-23: "ping 30s is fine, but should only
    /// be sent if nothing received for that time"). This departs from plan
    /// 20261022_02's liveness, which left it to `SO_KEEPALIVE`.
    fn keep_alive(&mut self, out: &mut Vec<Output>) {
        if self.link == Link::Offline {
            return;
        }
        match self.ping_sent {
            Some(at) if self.now.saturating_sub(at) >= self.ping_wait => {
                log::info!(
                    "link: no answer to a ping in {} s; reconnecting",
                    self.ping_wait.as_secs()
                );
                self.ping_sent = None;
                out.push(Output::Close("no answer to a ping".to_string()));
            }
            Some(_) => {}
            None if self.link == Link::Online
                && self.now.saturating_sub(self.last_heard)
                    >= if self.background {
                        PING_IDLE_BACKGROUND
                    } else {
                        PING_IDLE
                    } =>
            {
                self.ping_sent = Some(self.now);
                self.ping_wait = PING_WAIT;
                let params = common::PingParams {
                    active: Some(!self.background),
                };
                out.push(self.request(methods::PING, params, Sent::Ping));
            }
            None => {}
        }
    }

    /// `client.options` with read aloud as it is (plan 20261023_05, A4):
    /// the daemon may ask Gemini for summaries only while it is on.
    fn send_options(&mut self, out: &mut Vec<Output>) {
        let params = common::ClientOptionsParams {
            spoken_output: self.read_aloud && self.settings.sound,
            background: self.background,
            general_agent: self.settings.general_agent,
            tab_follows_focus: self.settings.tab_follows_focus,
        };
        out.push(self.request(methods::CLIENT_OPTIONS, params, Sent::Options));
    }

    /// Ask the daemon how to say a reply, and queue it to be read in its
    /// turn. Offline, it is read through `flankr_speech` here.
    #[allow(clippy::too_many_arguments)]
    fn read_reply(
        &mut self,
        pane_id: &str,
        child: Option<String>,
        section: common::Serial,
        ts: String,
        text: String,
        out: &mut Vec<Output>,
    ) {
        let serial = self.next_reading;
        self.next_reading += 1;
        let params_child = child.clone();
        let ready = if self.online() {
            let params = common::SpeechSummarizeParams {
                pane_id: pane_id.to_string(),
                ts: ts.clone(),
                text: text.clone(),
                child,
            };
            out.push(self.request(methods::SPEECH_SUMMARIZE, params, Sent::Summarize(serial)));
            None
        } else {
            Some(flankr_speech::read_aloud(&text, self.local_time))
        };
        self.reading.push_back(Reading {
            serial,
            ts,
            text,
            asked_at: self.now,
            section: Some(section),
            ready,
            pane_id: Some(pane_id.to_string()),
            child: params_child,
        });
        while self.reading.len() > NOTICE_CAP {
            self.reading.pop_front();
        }
    }

    /// Queues a reply to be read in its turn as its card's spoken text,
    /// with nothing asked of the daemon.
    fn queue_spoken(
        &mut self,
        (pane_id, child): (&str, Option<String>),
        section: common::Serial,
        ts: String,
        text: String,
        spoken: String,
    ) {
        let serial = self.next_reading;
        self.next_reading += 1;
        log::info!("read aloud: the card's spoken text, not asked again");
        self.reading.push_back(Reading {
            serial,
            ts,
            text,
            asked_at: self.now,
            section: Some(section),
            ready: Some(spoken),
            pane_id: Some(pane_id.to_string()),
            child,
        });
        while self.reading.len() > NOTICE_CAP {
            self.reading.pop_front();
        }
    }

    /// The daemon's answer for a reply being read: what to say (A3), or
    /// `None` to read it through `flankr_speech` here (no answer, or an
    /// error).
    fn summarized(&mut self, serial: u64, spoken: Option<String>, out: &mut Vec<Output>) {
        let now = self.local_time;
        if let Some(r) = self.reading.iter_mut().find(|r| r.serial == serial) {
            let spoken = spoken
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            r.ready = Some(spoken.unwrap_or_else(|| flankr_speech::read_aloud(&r.text, now)));
        }
        self.read_in_order();
        self.flush_notices(out);
    }

    /// Move the replies that are ready to the notices, in order: the first
    /// still waiting holds the rest, until `SUMMARY_WAIT` reads it here.
    fn read_in_order(&mut self) {
        let now = self.now;
        let local_time = self.local_time;
        while let Some(front) = self.reading.front_mut() {
            let text = match front.ready.take() {
                Some(text) => text,
                None if now.saturating_sub(front.asked_at) >= SUMMARY_WAIT => {
                    log::info!("no answer in time; reading the reply here");
                    flankr_speech::read_aloud(&front.text, local_time)
                }
                None => break,
            };
            let Some(r) = self.reading.pop_front() else {
                break;
            };
            if !text.is_empty() {
                self.read_replies.push_back((r.ts.clone(), text.clone()));
                while self.read_replies.len() > NOTICE_CAP {
                    self.read_replies.pop_front();
                }
                self.notices.push_back(Held {
                    text,
                    pane_id: r.pane_id,
                    ts: r.child.is_none().then_some(r.ts),
                });
            }
        }
        while self.notices.len() > NOTICE_CAP {
            self.notices.pop_front();
        }
    }

    /// A delivery for the open view's subscription, or the first one in
    /// `session.subscribe`'s result (plan 20261023_03, 4.2 as amended by
    /// A5 and A6): a reset replaces what is held; otherwise each section
    /// goes in at its serial, replacing the one held there, and `removed`
    /// ones go. Sections of another session file are ignored without a
    /// reset.
    fn delivered(&mut self, mut d: common::SessionDelivery, out: &mut Vec<Output>) {
        // Nothing is queued to be read with sound off (plan 20261024_02).
        let read_aloud = self.read_aloud && self.settings.sound;
        let Some(v) = self.view.as_mut() else {
            return;
        };
        let first = !v.answered || std::mem::take(&mut v.backfill);
        v.answered = true;
        if let Some(status) = d.status.take() {
            v.status = status;
        }
        if let Some(question) = d.pending_question.take() {
            v.pending_question = question;
        }
        if d.epoch.is_some() && !v.outline_mode {
            log::info!("view: the daemon sends outlines");
            v.outline_mode = true;
        }
        let now = self.now;
        if d.session.is_none() {
            // No session to show: why is in `reason`.
            v.sections.clear();
            v.start = false;
            v.outline = Outline::default();
            v.incoming = None;
            v.appending = false;
            v.orphans = false;
            v.generation += 1;
            if d.reason.as_deref() == Some(common::delivery_reasons::PANE_CLOSED) {
                // No more deliveries come; the subscription is over.
                v.subscription = None;
            }
        } else if v.outline_mode {
            take_outline(v, d, now);
        } else {
            log::info!(
                "view: delivery, reset {}, {} section(s), {} removed, {} held",
                d.reset,
                d.sections.len(),
                d.removed.len(),
                v.sections.len()
            );
            if d.reset {
                v.sections.clear();
                v.start = false;
            }
            for serial in d.removed {
                v.sections.remove(serial);
            }
            for section in d.sections {
                v.sections.put(section);
            }
        }
        v.reflatten();
        // The typed response goes once the session shows it as the user's,
        // or the agent works on it.
        if let Some(t) = v.typed.as_deref() {
            let shown = v
                .entries
                .iter()
                .any(|e| crate::view::is_user_said(e) && same_message(&e.summary, t));
            if shown || v.status == "working" {
                v.typed = None;
            }
        }
        // Replies not seen before, read aloud in order, each as the daemon
        // says (plan 20261023_05); the first backfill's are not. A reply
        // whose Finished card carries the spoken text is read as that,
        // without asking again (CHANGES.md, 2026-10-24 12:06 EDT).
        let mut replies = Vec::new();
        for section in v.sections.iter().filter(|s| s.card.is_none()) {
            for e in section.entries.iter().filter(|e| is_reply(e)) {
                if v.seen_replies.insert(e.ts.clone()) && read_aloud && !first {
                    let spoken = card_spoken(&v.sections, section.serial);
                    replies.push((section.serial, e.ts.clone(), e.summary.clone(), spoken));
                }
            }
        }
        let pane_id = v.pane_id.clone();
        let child = v.child.clone();
        let ends_on_reply = v.status == "working" && v.entries.last().is_some_and(is_reply);
        // A card's spoken text that came for a reply still waiting.
        for r in self.reading.iter_mut().filter(|r| r.ready.is_none()) {
            if let Some(spoken) = r.section.and_then(|s| card_spoken(&v.sections, s)) {
                log::info!("read aloud: the card's spoken text came");
                r.ready = Some(spoken);
            }
        }
        for (serial, ts, text, spoken) in replies {
            match spoken {
                Some(spoken) => {
                    self.queue_spoken((&pane_id, child.clone()), serial, ts, text, spoken)
                }
                None => self.read_reply(&pane_id, child.clone(), serial, ts, text, out),
            }
        }
        // herdr says working but the session ends on a reply: the daemon's
        // turn says whether it stopped (plan 20261023_04, A4), so the list
        // is asked again, at most once in `LIST_MIN_INTERVAL`.
        let stopped = self
            .agents
            .iter()
            .find(|a| a.pane_id == pane_id)
            .is_some_and(|a| matches!(a.turn, Some(common::Turn::Stopped { .. })));
        let listed_since = self
            .listed_at
            .map_or(LIST_MIN_INTERVAL, |at| self.now.saturating_sub(at));
        if ends_on_reply && !stopped && !self.listing && listed_since >= LIST_MIN_INTERVAL {
            self.ask_list(out);
        }
        self.read_in_order();
        self.fetch_contents(out);
        self.fill_gaps(out);
        self.recompute_view(out);
        self.flush_notices(out);
    }

    /// A `session.history` page for subscription `id` (A5, A7): its
    /// sections go in at their serials, and none of its replies is read
    /// aloud.
    fn paged(&mut self, id: u64, page: common::SessionHistoryResult, out: &mut Vec<Output>) {
        let Some(v) = self.view.as_mut().filter(|v| v.subscription == Some(id)) else {
            return;
        };
        v.history = false;
        log::info!(
            "view: history page, {} section(s), start {}, {} held",
            page.sections.len(),
            page.start,
            v.sections.len()
        );
        if page.start {
            v.start = true;
        }
        for section in page.sections {
            v.sections.put(section);
        }
        v.reflatten();
        v.see_all();
        self.show_view(out);
    }

    /// `agents.list` answered: the view's status, and a message target
    /// fixed to the viewed agent before its names were known.
    fn list_learned(&mut self, out: &mut Vec<Output>) {
        if let Some(v) = self.view.as_mut()
            && let Some(a) = self.agents.iter().find(|a| a.pane_id == v.pane_id)
        {
            // The subscription's own status is newer, once it answered.
            if !v.answered {
                v.status = a.status.clone();
            }
        }
        let fixed = self
            .dictation
            .as_ref()
            .filter(|d| d.target.is_none())
            .and_then(|d| d.fixed_pane.clone());
        if let Some(pane_id) = fixed {
            let target = self.target_for(&pane_id);
            if let Some(d) = self.dictation.as_mut() {
                d.target = target;
            }
        }
        self.recompute_view(out);
    }

    fn end_turn(&mut self, id: u64, outcome: String, stopped: Vec<String>, out: &mut Vec<Output>) {
        let Some(turn) = self.turn.take_if(|t| t.id == id) else {
            log::warn!("turn {id} ended, but it is not the running turn");
            return;
        };
        if turn.tapped && !stopped.is_empty() {
            self.say_or_hold(stopped_sentence(&stopped), out);
        }
        out.push(event(Event::TurnEnded { outcome, stopped }));
        if self.dictation.as_ref().is_some_and(|d| d.general_agent) {
            self.dictation = None;
            self.show_dictation(out);
        }
        if let Some(text) = self.deferred.take() {
            self.start_turn(text, out);
        }
        self.flush_notices(out);
    }

    fn notification(&mut self, note: Notification, out: &mut Vec<Output>) {
        match note.method.as_str() {
            wire::methods::TURN_SAY => {
                if let Some(say) = parse_or_log::<wire::TurnSay>(note) {
                    // Only the running turn speaks. A say for a turn the user
                    // tapped to interrupt, or one that has ended, would talk
                    // over them.
                    let running = self
                        .turn
                        .as_ref()
                        .is_some_and(|t| t.id == say.turn_id && !t.tapped);
                    if running {
                        out.push(event(Event::Conversation {
                            speaker: Speaker::Gemini,
                            text: say.text.clone(),
                        }));
                        // Spoken only while *Speak agent (Gemini)
                        // responses* is on (plan 20261028_06, decision 6);
                        // `sound` gates it downstream.
                        if self.settings.speak_gemini {
                            self.say_or_hold(say.text, out);
                        }
                    } else {
                        log::debug!("dropping a late turn.say for turn {}", say.turn_id);
                    }
                }
            }
            methods::SESSION_DELIVERY => {
                if let Some(d) = parse_or_log::<common::SessionDelivery>(note) {
                    let ours = self
                        .view
                        .as_ref()
                        .is_some_and(|v| v.subscription == Some(d.subscription_id));
                    if ours {
                        self.delivered(d, out);
                    } else {
                        log::debug!(
                            "a delivery for subscription {}, not ours",
                            d.subscription_id
                        );
                    }
                }
            }
            methods::CONFIRM_REQUESTED => {
                if let Some(c) = parse_or_log::<wire::ConfirmRequested>(note) {
                    self.requested(c, out);
                }
            }
            methods::CONFIRM_RESOLVED => {
                if let Some(r) = parse_or_log::<common::ConfirmResolved>(note) {
                    self.resolved(r, out);
                }
            }
            wire::methods::NOTICE => {
                if let Some(n) = parse_or_log::<wire::Notice>(note) {
                    // Typing into a terminal is the owner's own doing: it is
                    // noted, not said (the owner, 2026-10-28: "why is it
                    // speaking the keys?").
                    if n.kind == TERMINAL_INPUT_NOTICE {
                        log::info!("notice: {}", n.text);
                        return;
                    }
                    self.notices.push_back(Held {
                        text: n.text,
                        pane_id: (!n.pane_id.is_empty()).then_some(n.pane_id),
                        ts: None,
                    });
                    while self.notices.len() > NOTICE_CAP {
                        self.notices.pop_front();
                    }
                    self.flush_notices(out);
                }
            }
            wire::methods::VOCABULARY_CHANGED => {
                if let Some(v) = parse_or_log::<wire::VocabularyChanged>(note) {
                    self.vocabulary = v.vocabulary.clone();
                    out.push(event(Event::Vocabulary(v.vocabulary)));
                }
            }
            wire::methods::SERVER_CAPABILITIES => {
                if let Some(c) = parse_or_log::<wire::ServerCapabilities>(note) {
                    // `terminal` is always sent while offered: none is
                    // none.
                    let acts = self.terminals.update(|b| b.set_info(c.terminal));
                    self.terminal_acts(acts, out);
                    if let Some(llm) = c.llm {
                        self.set_llm(llm, out);
                    }
                    if let Some(plugins) = c.plugins {
                        self.set_plugins(plugins, out);
                    }
                    if let Some(blob) = c.blob {
                        self.blobs.capabilities(blob, self.now, out);
                    }
                }
            }
            wire::methods::PLUGIN_RESULT => {
                if let Some(r) = parse_or_log::<wire::PluginResult>(note) {
                    let key = (r.ns.clone(), r.id);
                    match self.plugin_waiting.iter().position(|w| *w == key) {
                        Some(i) => {
                            self.plugin_waiting.remove(i);
                            out.push(event(Event::PluginResult(r)));
                        }
                        None => log::warn!("plugin.result for no call of ours: {} {}", r.ns, r.id),
                    }
                }
            }
            wire::methods::PLUGIN_NOTIFY => {
                if let Some(n) = parse_or_log::<wire::PluginNotify>(note) {
                    out.push(event(Event::PluginNotify(n)));
                }
            }
            methods::TERMINAL_CLOSED => {
                if let Some(c) = parse_or_log::<common::TerminalClosedParams>(note) {
                    log::info!("terminal {} closed: {:?}", c.terminal_id, c.reason);
                    self.terminals.update(|b| ((), b.closed(c)));
                }
            }
            wire::methods::FOCUS_CHANGED => {
                if let Some(f) = parse_or_log::<wire::FocusChanged>(note) {
                    self.set_focus(f.focus.clone(), out);
                    // Follow on: the view moves to it.
                    self.follow_focus(f.focus.as_ref(), out);
                    // The overview's focus bar moves with it.
                    self.list_wanted = true;
                    self.list_if_due(out);
                }
            }
            other => log::debug!("ignoring the notification {other}"),
        }
    }

    fn requested(&mut self, c: wire::ConfirmRequested, out: &mut Vec<Output>) {
        if self.confirms.iter().any(|known| known.id == c.confirm_id) {
            return; // a reminder of one already being handled
        }
        let own_message = self.dictation.as_ref().is_some_and(|d| {
            d.own_approval
                && (d.confirm_id == Some(c.confirm_id)
                    || (d.phase == DictationPhase::Submitting
                        && same_message(&c.command, &d.text())))
        });
        if own_message {
            log::info!("dictation: approving the submitted message");
            self.reply(c.confirm_id, true, out);
            return;
        }
        // A suggestion tapped in the view is its own approval too.
        let own_tap = self.tapped.iter().position(|t| {
            t.confirm_id == Some(c.confirm_id)
                || (t.confirm_id.is_none()
                    && t.pane_id == c.pane_id
                    && same_message(&c.command, &t.text))
        });
        if let Some(i) = own_tap {
            let t = self.tapped.remove(i);
            log::info!("agent view: approving the tapped {}", t.text);
            self.reply(c.confirm_id, true, out);
            return;
        }
        self.confirms.push_back(Confirmation {
            id: c.confirm_id,
            pane_id: c.pane_id,
            text: c.text,
            label: c.handle,
            phrase: c.token.unwrap_or_default(),
            command: c.command,
            agent: c.name.unwrap_or(c.agent),
            project: c.project,
        });
        if self.confirms.len() == 1 {
            self.ask_next(out);
        }
        self.recompute_view(out);
    }

    /// Approves the own message's confirmation, if it is already queued.
    fn approve_own_message(&mut self, confirm_id: u64, out: &mut Vec<Output>) {
        if !self.dictation.as_ref().is_some_and(|d| d.own_approval) {
            return;
        }
        self.approve_own(confirm_id, out);
    }

    /// Approves `confirm_id` as the user's own, taking it off the queue.
    fn approve_own(&mut self, confirm_id: u64, out: &mut Vec<Output>) {
        let was_asked = self.confirms.front().map(|c| c.id) == Some(confirm_id);
        self.confirms.retain(|c| c.id != confirm_id);
        self.reply(confirm_id, true, out);
        if was_asked {
            self.ask_next(out);
        }
        self.show_pending(out);
    }

    /// Sends `confirm.reply` for `confirm_id` without asking anyone.
    fn reply(&mut self, confirm_id: u64, approve: bool, out: &mut Vec<Output>) {
        let params = common::ConfirmReplyParams {
            confirm_id,
            approve,
            nonce: None,
        };
        out.push(event(Event::Answered {
            confirm_id,
            approve,
        }));
        out.push(self.request(
            methods::CONFIRM_REPLY,
            params,
            Sent::ConfirmReply(confirm_id),
        ));
    }

    fn resolved(&mut self, r: common::ConfirmResolved, out: &mut Vec<Output>) {
        let was_asked = self.confirms.front().map(|c| c.id) == Some(r.confirm_id);
        self.confirms.retain(|c| c.id != r.confirm_id);
        let outcome = serde_json::to_value(r.outcome)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        out.push(event(Event::Resolved {
            confirm_id: r.confirm_id,
            outcome: outcome.clone(),
        }));
        let own = self
            .dictation
            .as_ref()
            .is_some_and(|d| d.confirm_id == Some(r.confirm_id));
        if own {
            self.message_resolved(&outcome, out);
        }
        if was_asked {
            self.ask_next(out);
        }
        self.recompute_view(out);
        self.flush_notices(out);
    }

    /// Asks the confirmation now at the front, if any.
    fn ask_next(&mut self, out: &mut Vec<Output>) {
        if let Some(c) = self.confirms.front() {
            out.push(event(Event::Confirm {
                confirm_id: c.id,
                text: c.text.clone(),
                label: c.label.clone(),
                phrase: c.phrase.clone(),
                command: c.command.clone(),
                agent: c.agent.clone(),
                project: c.project.clone(),
            }));
            out.push(event(Event::Say(c.text.clone())));
        }
    }

    /// Says the held notices, oldest first, unless speaking, listening,
    /// confirming or taking voice input. When voice input has ended with
    /// speech held, a catch-up is asked for first, and the notices wait for
    /// it (plan 20261023_02, the 2026-10-30 13:52 EDT amendment).
    fn flush_notices(&mut self, out: &mut Vec<Output>) {
        if !self.settings.sound {
            self.notices.clear();
            self.catching_up = None;
            return;
        }
        if self.voice_input() {
            self.voice_was = true;
            return;
        }
        if self
            .catching_up
            .as_ref()
            .is_some_and(|c| self.now >= c.asked_at + CATCH_UP_WAIT)
            && let Some(serial) = self.catching_up.as_ref().map(|c| c.serial)
        {
            log::info!("speech.catch_up: no answer in time; the held speech is said as it is");
            self.caught_up(serial, None);
        }
        // Speech that arrives while a catch-up is made waits for it.
        if self.catching_up.is_some() {
            return;
        }
        if std::mem::take(&mut self.voice_was) {
            let said = self.voice_said.take();
            if !self.notices.is_empty() && self.ask_catch_up(said, out) {
                return;
            }
        }
        if self.speaking || self.listening || !self.confirms.is_empty() {
            return;
        }
        for held in self.notices.drain(..) {
            let text = held.text;
            if let Some((ts, _)) = self.read_replies.iter().find(|(_, t)| *t == text) {
                self.saying.push(ts.clone());
            }
            out.push(event(Event::Say(text)));
        }
    }

    /// Asks the daemon for one catch-up for the held notices, oldest
    /// first, and `said` (the 13:52 EDT amendment, item 1). `false`, and
    /// nothing asked, when it cannot be made: offline, the daemon makes no
    /// summaries, or spoken output (read aloud) is off (item 4).
    fn ask_catch_up(&mut self, said: Option<common::SaidInput>, out: &mut Vec<Output>) -> bool {
        if !self.online() || !self.llm.summaries || !self.read_aloud {
            log::info!(
                "speech.catch_up: not asked ({}); the held speech is said as it is",
                if !self.online() {
                    "offline"
                } else if !self.llm.summaries {
                    "no summaries"
                } else {
                    "read aloud is off"
                }
            );
            return false;
        }
        let held: Vec<Held> = self.notices.drain(..).collect();
        let params = common::SpeechCatchUpParams {
            held: held
                .iter()
                .map(|h| common::HeldSpeech {
                    pane_id: h.pane_id.clone(),
                    ts: h.ts.clone(),
                    text: h.text.clone(),
                })
                .collect(),
            said,
        };
        let serial = self.next_catch_up;
        self.next_catch_up += 1;
        log::info!("speech.catch_up: asked for {} held item(s)", held.len());
        out.push(self.request(methods::SPEECH_CATCH_UP, params, Sent::CatchUp(serial)));
        self.catching_up = Some(CatchingUp {
            serial,
            held,
            asked_at: self.now,
        });
        true
    }

    /// The catch-up `serial` is answered with `summary`, or failed: the
    /// summary, or the held items as they are, go before what came since
    /// (item 3), to be said in their turn. A catch-up given up or dropped
    /// is not answered again.
    fn caught_up(&mut self, serial: u64, summary: Option<String>) {
        let Some(c) = self.catching_up.take_if(|c| c.serial == serial) else {
            log::debug!("speech.catch_up: a late answer for {serial}, dropped");
            return;
        };
        match summary
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
        {
            Some(summary) => self.notices.push_front(Held::text(summary)),
            None => {
                for held in c.held.into_iter().rev() {
                    self.notices.push_front(held);
                }
            }
        }
        while self.notices.len() > NOTICE_CAP {
            self.notices.pop_front();
        }
    }

    /// Whether voice input is in progress (plan 20261023_02, the
    /// 2026-10-30 13:37 EDT amendment, item 1): a prompt open in voice
    /// mode, the message or Gemini's, still collecting.
    fn voice_input(&self) -> bool {
        self.dictation
            .as_ref()
            .is_some_and(|d| d.mode == PromptMode::Voice && d.phase == DictationPhase::Collecting)
    }

    /// Says `text` now, or during voice input holds it with the notices,
    /// to be said once the input ends (the 13:37 EDT amendment, item 2).
    fn say_or_hold(&mut self, text: String, out: &mut Vec<Output>) {
        if self.voice_input() {
            log::info!("say: held while voice input is in progress");
            self.notices.push_back(Held::text(text));
            while self.notices.len() > NOTICE_CAP {
                self.notices.pop_front();
            }
        } else {
            out.push(event(Event::Say(text)));
        }
    }

    /// Voice input starts: what the speaker plays stops (the 13:37 EDT
    /// amendment, item 5).
    fn stop_for_voice_input(&mut self, out: &mut Vec<Output>) {
        let stopped = out
            .iter()
            .any(|o| matches!(o, Output::Event(Event::StopSpeaking)));
        if self.speaking && !stopped {
            out.push(event(Event::StopSpeaking));
        }
    }

    /// [`Input::StopReading`]: the replies waiting (for their summary or
    /// their turn) and the held notices are dropped, and what is being
    /// said stops.
    fn stop_reading(&mut self, out: &mut Vec<Output>) {
        log::info!(
            "read aloud: stopped; {} repl(ies) and {} notice(s) dropped",
            self.reading.len(),
            self.notices.len()
        );
        self.reading.clear();
        self.notices.clear();
        self.catching_up = None;
        if self.speaking {
            out.push(event(Event::StopSpeaking));
        }
    }

    /// The reply at `ts` is answered: it is not read, and if it is being
    /// read, that stops (owner, 2026-10-23: "if an answer is to a spoke
    /// question, it should skip the question playback").
    fn skip_reading(&mut self, ts: &str, out: &mut Vec<Output>) {
        self.reading.retain(|r| r.ts != ts);
        let texts: Vec<String> = self
            .read_replies
            .iter()
            .filter(|(t, _)| t == ts)
            .map(|(_, text)| text.clone())
            .collect();
        self.notices.retain(|n| !texts.contains(&n.text));
        if let Some(c) = self.catching_up.as_mut() {
            c.held.retain(|n| !texts.contains(&n.text));
        }
        if self.speaking && self.saying.iter().any(|t| t == ts) {
            log::info!("read aloud: the question is answered; skipping it");
            out.push(event(Event::StopSpeaking));
        }
    }

    /// The user interacted with the session in `pane_id` (plan
    /// 20261022_03, the L3 amendment of 2026-10-24 15:56 EDT, item 2).
    fn interact(&mut self, pane_id: &str) {
        let agent = self.agents.iter().find(|a| a.pane_id == pane_id);
        self.cross.interact(pane_id, agent, self.now);
    }

    /// When the session in `pane_id` was last interacted with, on the
    /// driver's clock.
    pub fn last_interaction(&self, pane_id: &str) -> Option<Duration> {
        self.cross.last_interaction(pane_id)
    }

    /// The turn of `a` stopped on `line`: if its session was used within
    /// the hour and is not the one viewed, how long the turn ran is asked
    /// for, for its finished card (the 16:01 EDT amendment).
    fn peek_finished(&mut self, a: &common::AgentInfo, line: &str, out: &mut Vec<Output>) {
        let viewed = self
            .view
            .as_ref()
            .filter(|v| v.child.is_none())
            .map(|v| v.pane_id.as_str());
        if Some(a.pane_id.as_str()) == viewed
            || !self.cross.recent(a, self.now)
            || a.session.is_none()
        {
            return;
        }
        let params = common::SessionPeekParams {
            pane_id: a.pane_id.clone(),
            entries: common::PEEK_ENTRIES_MAX,
            child: None,
        };
        let sent = Sent::Peek {
            pane_id: a.pane_id.clone(),
            line: line.to_string(),
        };
        out.push(self.request(methods::SESSION_PEEK, params, sent));
    }

    /// The finished turn's session, read: a finished card when it ran at
    /// least [`crate::cross::LONG_TURN`].
    fn peeked(&mut self, pane_id: &str, line: String, r: common::SessionPeekResult) {
        let Some(span) = r
            .session
            .as_ref()
            .and_then(|p| crate::cross::turn_span(&p.entries))
        else {
            return;
        };
        let Some(a) = self.agents.iter().find(|a| a.pane_id == pane_id).cloned() else {
            return;
        };
        // Working again, or asking, since: not finished.
        if !matches!(a.turn, Some(common::Turn::Stopped { .. })) {
            return;
        }
        let viewed = self.view.as_ref().map(|v| v.pane_id.clone());
        log::info!("cards: a turn of {} s finished", span.as_secs());
        self.cross
            .finished(&a, line, span, viewed.as_deref(), self.now);
    }

    /// Gives other sessions' cards if they changed, with a ding for a new
    /// question or approval.
    fn show_cards(&mut self, out: &mut Vec<Output>) {
        let approvals: Vec<(String, String)> = self
            .confirms
            .iter()
            .map(|c| (c.pane_id.clone(), c.command.clone()))
            .collect();
        let viewed = self.view.as_ref().map(|v| v.pane_id.clone());
        let came = self
            .cross
            .update(&self.agents, &approvals, viewed.as_deref(), self.now);
        self.cross.expire(self.now);
        if let Some(cards) = self.cross.take_changed() {
            if came {
                log::info!("ding: another session asks");
                out.push(event(Event::Ding));
            }
            out.push(event(Event::SessionCards(cards)));
        }
    }

    /// The panes with a flankr confirmation waiting, each once, in the
    /// order they were asked (plan 20261023_04, section 4).
    pub fn pending_panes(&self) -> Vec<String> {
        let mut panes: Vec<String> = Vec::new();
        for c in &self.confirms {
            if !panes.contains(&c.pane_id) {
                panes.push(c.pane_id.clone());
            }
        }
        panes
    }

    /// Gives the pending panes if they changed since last given.
    fn show_pending(&mut self, out: &mut Vec<Output>) {
        let panes = self.pending_panes();
        if panes != self.pending_shown {
            self.pending_shown = panes.clone();
            out.push(event(Event::PendingPanes(panes)));
        }
    }

    /// Sends `agents.list`.
    fn ask_list(&mut self, out: &mut Vec<Output>) {
        self.listing = true;
        self.listed_at = Some(self.now);
        out.push(self.request(methods::AGENTS_LIST, common::Empty {}, Sent::List));
    }

    /// Lists the agents for the overview if it is due (plan 20261023_04,
    /// decision 6): only while no agent view is open and no list is on its
    /// way; every `LIST_INTERVAL` since the last list sent or, once asked
    /// for, `LIST_MIN_INTERVAL` after it.
    fn list_if_due(&mut self, out: &mut Vec<Output>) {
        if self.background || !self.online() || self.listing {
            return;
        }
        // In a view, only while another session used within the hour is
        // watched for its questions and turns (plan 20261022_03, the L3
        // amendment of 2026-10-24 15:56 EDT, item 5).
        if self.drawer {
            let due = self
                .listed_at
                .is_none_or(|at| self.now.saturating_sub(at) >= DRAWER_LIST_INTERVAL);
            if due {
                self.list_wanted = false;
                self.ask_list(out);
            }
            return;
        }
        let viewed = self.view.as_ref().map(|v| v.pane_id.as_str());
        // A view of an agent whose reader reads subagents keeps its
        // children current (the subagents plan, step 4).
        let children = viewed.is_some_and(|p| {
            self.agents
                .iter()
                .any(|a| a.pane_id == p && a.subagents_running.is_some())
        });
        if viewed.is_some() && !children && !self.cross.watching(viewed, self.now) {
            return;
        }
        let due = match self.listed_at {
            None => true,
            Some(at) => {
                let since = self.now.saturating_sub(at);
                since >= LIST_INTERVAL || (self.list_wanted && since >= LIST_MIN_INTERVAL)
            }
        };
        if due {
            self.list_wanted = false;
            self.ask_list(out);
        }
    }

    fn request(&mut self, method: &str, params: impl serde::Serialize, sent: Sent) -> Output {
        let id = Id::Number(self.next_request);
        self.next_request += 1;
        self.sent.insert(id.clone(), sent);
        Output::Send(Request::new(id, method, params))
    }
}

/// An agent's reply: its prose, which is what reading aloud reads.
/// An outline delivery for view `v` at `now` (plan 20261024_01, decision
/// 3, as CHANGES.md 2026-10-24 00:36 EDT changes it). A reset drops only
/// the outline: the new one is merged by serial, the contents held staying
/// valid unless their slot's `rev` changed, and those with no slot in it
/// dropped once it is complete. A reset while scrolled back is held aside,
/// and the old ribbon stays until it is applied. Otherwise its slots and
/// removed indexes go into the outline being built, and the contents that
/// ride along go in at their slots' revs.
fn take_outline(v: &mut AgentView, d: common::SessionDelivery, now: Duration) {
    log::info!(
        "view: outline delivery, reset {}, {} slot(s), {} section(s), {} removed, last {:?}, {} held",
        d.reset,
        d.slots.len(),
        d.sections.len(),
        d.removed_index.len(),
        d.last,
        v.outline.len()
    );
    if d.reset {
        v.generation += 1;
        v.unavailable.clear();
        let fresh = Outline::new(d.epoch);
        if v.scrolled_back() && !v.outline.is_empty() {
            // Held aside: the old ribbon stays until the jump line's tap.
            let sections = v.incoming.take().map(|i| i.sections).unwrap_or_default();
            v.incoming = Some(HeldOutline {
                outline: fresh,
                sections,
            });
        } else {
            if let Some(incoming) = v.incoming.take() {
                v.sections.merge(incoming.sections);
            }
            v.outline = fresh;
            v.orphans = true;
        }
        v.hole = (0, now);
    }
    let (outline, sections) = v.building();
    outline.set_epoch(d.epoch);
    for slot in &d.slots {
        outline.put(*slot);
    }
    for &i in &d.removed_index {
        if let Some(slot) = outline.remove(i) {
            sections.remove(slot.serial);
        }
    }
    outline.set_last(d.last);
    // A new session file: nothing held is of it.
    if let (Some(file), Some(held)) = (outline.file(), sections.file())
        && file != held
    {
        sections.clear();
    }
    for serial in d.removed {
        sections.remove(serial);
    }
    for s in d.sections {
        let rev = outline
            .slot_of(s.serial)
            .or_else(|| d.slots.iter().find(|slot| slot.serial == s.serial))
            .map_or(0, |slot| slot.rev);
        sections.put_rev(s, rev);
    }
    v.note_hole(now);
    if v.incoming.is_some() && !v.scrolled_back() {
        v.apply_incoming();
    }
    v.drop_orphans();
    v.forget();
}

/// The spoken text of the Finished card on the line of the reply section
/// `reply`, when the daemon summarised it (`[summary] cards`).
fn card_spoken(sections: &Sections, reply: common::Serial) -> Option<String> {
    sections
        .on_line(reply)
        .filter_map(|s| s.card.as_ref())
        .filter(|c| c.card == common::CardKind::Finished)
        .find_map(|c| c.spoken.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn is_reply(e: &common::Entry) -> bool {
    e.role == common::Role::Assistant && e.kind == common::EntryKind::Text
}

/// An agent of `agents.list` as a message target, as the focus names it.
fn focus_of(a: &common::AgentInfo) -> common::FocusInfo {
    let shown = |agent: &str| {
        let mut chars = agent.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().chain(chars).collect::<String>())
            .unwrap_or_default()
    };
    common::FocusInfo {
        pane_id: a.pane_id.clone(),
        project: a.project.clone(),
        agent: (a.agent != "shell").then(|| shown(&a.agent)),
        tab_label: a.tab_label.clone(),
        tab_number: a.tab_number,
        name: a.name.clone().unwrap_or_else(|| shown(&a.agent)),
        spoken: a.spoken.clone().unwrap_or_else(|| shown(&a.agent)),
    }
}

/// A refused or cancelled plugin call's answer.
fn plugin_error(ns: String, id: u64, code: i64, message: &str) -> wire::PluginResult {
    wire::PluginResult {
        ns,
        id,
        result: None,
        error: Some(libflankr_proto::ErrorObject::new(code, message)),
    }
}

/// What answers `input` given for a remote that is not in use, if it is
/// a plugin input (plan 20261028_04, 3.10 item 3: plugins act on the
/// active remote only): a plugin call with an `id` is answered as refused,
/// a toggle with an error. `None` for every other input.
pub fn plugin_refusal(input: &Input) -> Option<Vec<Event>> {
    const MESSAGE: &str = "plugins act on the remote in use only";
    match input {
        Input::PluginCall { ns, id, .. } => Some(
            id.map(|id| {
                Event::PluginResult(plugin_error(
                    ns.clone(),
                    id,
                    libflankr_proto::envelope::codes::NOT_PERMITTED,
                    MESSAGE,
                ))
            })
            .into_iter()
            .collect(),
        ),
        Input::SetPluginEnabled { .. } => Some(vec![Event::Error(MESSAGE.to_string())]),
        _ => None,
    }
}

fn event(e: Event) -> Output {
    Output::Event(e)
}

/// [`Event::ChoiceAnswered`] for an answer that did not go through: its
/// reason, and the daemon's code when it gave one.
fn choice_failed(id: String, pane_id: String, reason: String, code: Option<i64>) -> Event {
    Event::ChoiceAnswered {
        id,
        pane_id,
        outcome: "failed".to_string(),
        got: vec![reason],
        code,
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, serde_json::Error> {
    serde_json::from_value(value)
}

fn parse_or_log<T: DeserializeOwned>(note: Notification) -> Option<T> {
    match parse(note.params) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("a bad {} notification: {e}", note.method);
            None
        }
    }
}

/// Whether the daemon's copy of a message is the text submitted, allowing
/// for its cleaning: runs of whitespace folded, and trailing `.` and `,`
/// dropped (`validate_message`).
fn same_message(daemon: &str, submitted: &str) -> bool {
    fn clean(s: &str) -> String {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end_matches(['.', ','])
            .to_string()
    }
    clean(daemon) == clean(submitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A state that has said hello and been answered.
    fn online() -> State {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        s.incoming(response(
            id,
            json!({"server": "flankr", "session_id": "companion:test", "vocabulary": ["claude"]}),
        ));
        assert!(s.online());
        s
    }

    fn response(id: Id, result: Value) -> Incoming {
        Incoming::Response(Response::ok(id, result))
    }

    fn note(method: &str, params: Value) -> Incoming {
        Incoming::Notification(Notification::new(method, params))
    }

    fn sent(out: &[Output]) -> Vec<&Request> {
        out.iter()
            .filter_map(|o| match o {
                Output::Send(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    fn events(out: &[Output]) -> Vec<Event> {
        out.iter()
            .filter_map(|o| match o {
                Output::Event(e) => Some(e.clone()),
                _ => None,
            })
            .collect()
    }

    fn say(text: &str) -> Event {
        Event::Say(text.to_string())
    }

    /// Starts a turn with `text`, returning the turn.start request.
    fn start(s: &mut State, text: &str) -> Request {
        let out = s.input(Input::Utterance(text.to_string()));
        let reqs = sent(&out);
        assert_eq!(reqs.len(), 1, "{out:?}");
        assert_eq!(reqs[0].method, "turn.start");
        reqs[0].clone()
    }

    fn confirm_requested(id: u64) -> Incoming {
        note(
            "confirm.requested",
            json!({
                "confirm_id": id, "pane_id": "w2:pZ", "agent": "claude",
                "name": "Claude", "project": "flankr",
                "command": "continue", "requested_by": "companion:test",
                "label": "keep-going", "phrase": "copper-lantern-ninety",
                "text": "Approve sending \"continue\" to Claude? For keep going."
            }),
        )
    }

    #[test]
    fn hello_names_the_protocol_and_the_session_it_had() {
        let mut s = online();
        assert_eq!(s.session_id(), Some("companion:test"));
        s.disconnected(1000);
        let out = s.connected();
        let hello = sent(&out)[0];
        assert_eq!(hello.method, "hello");
        assert_eq!(
            hello.params,
            json!({"protocol": "flankr.companion", "version": 1, "client": "test",
                   "session_id": "companion:test"})
        );
    }

    #[test]
    fn hello_gives_connected_and_the_vocabulary() {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(
            id,
            json!({"server": "flankr", "session_id": "companion:test", "vocabulary": ["claude", "hr"]}),
        ));
        assert_eq!(
            events(&out),
            vec![
                Event::Connected {
                    server: "flankr".into()
                },
                Event::Vocabulary(vec!["claude".into(), "hr".into()]),
                // No `llm` from this daemon: both, as before.
                Event::Llm(wire::LlmInfo::available()),
                // No `plugins`: none.
                Event::Plugins(vec![]),
            ]
        );
    }

    #[test]
    fn a_refused_hello_closes_the_connection() {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let refusal = Response::err(
            Some(id),
            libflankr_proto::ErrorObject::new(-32011, "unsupported protocol"),
        );
        let out = s.incoming(Incoming::Response(refusal));
        assert!(out.iter().any(|o| matches!(o, Output::Close(_))), "{out:?}");
        assert!(!s.online());
    }

    #[test]
    fn a_turn_and_its_say() {
        let mut s = online();
        assert_eq!(events(&s.input(Input::Tap)), vec![Event::Arm]);
        let req = start(&mut s, "what is claude doing");
        assert_eq!(
            req.params,
            json!({"turn_id": 1, "text": "what is claude doing"})
        );
        let out = s.incoming(note(
            "turn.say",
            json!({"turn_id": 1, "seq": 0, "text": "Claude is running the tests."}),
        ));
        assert_eq!(
            events(&out),
            vec![
                Event::Conversation {
                    speaker: Speaker::Gemini,
                    text: "Claude is running the tests.".into()
                },
                say("Claude is running the tests.")
            ]
        );
        let out = s.incoming(response(req.id, json!({"outcome": "done", "ran": []})));
        assert_eq!(
            events(&out),
            vec![Event::TurnEnded {
                outcome: "done".into(),
                stopped: vec![]
            }]
        );
        // Idle again: the next utterance is a new turn.
        let next = start(&mut s, "and codex");
        assert_eq!(next.params["turn_id"], 2);
    }

    #[test]
    fn a_say_with_speak_gemini_off_is_shown_but_not_spoken() {
        let mut s = online();
        s.input(Input::Settings(Settings {
            speak_gemini: false,
            ..Settings::default()
        }));
        s.input(Input::Tap);
        start(&mut s, "what is claude doing");
        let out = s.incoming(note(
            "turn.say",
            json!({"turn_id": 1, "seq": 0, "text": "Claude is running the tests."}),
        ));
        assert_eq!(
            events(&out),
            vec![Event::Conversation {
                speaker: Speaker::Gemini,
                text: "Claude is running the tests.".into()
            }]
        );
    }

    #[test]
    fn a_say_after_a_tap_or_after_its_turn_is_not_spoken() {
        let mut s = online();
        let req = start(&mut s, "run the tests");
        s.input(Input::Tap);
        let out = s.incoming(note(
            "turn.say",
            json!({"turn_id": 1, "seq": 0, "text": "Running them now."}),
        ));
        assert!(events(&out).is_empty(), "{out:?}");
        s.incoming(response(
            req.id,
            json!({"outcome": "interrupted", "ran": []}),
        ));
        // A say that arrives after its turn ended is dropped too.
        let out = s.incoming(note(
            "turn.say",
            json!({"turn_id": 1, "seq": 1, "text": "Done."}),
        ));
        assert!(events(&out).is_empty(), "{out:?}");
    }

    #[test]
    fn utterances_while_waiting_are_ignored() {
        let mut s = online();
        start(&mut s, "hello");
        assert!(s.input(Input::Utterance("late".into())).is_empty());
    }

    #[test]
    fn tap_while_waiting_stops_speaking_and_interrupts() {
        let mut s = online();
        let req = start(&mut s, "send continue to claude");
        let out = s.input(Input::Tap);
        assert_eq!(events(&out), vec![Event::StopSpeaking]);
        let reqs = sent(&out);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "turn.interrupt");
        assert_eq!(reqs[0].params, json!({"turn_id": 1}));
        // A second tap stops playback again but does not interrupt twice.
        let out = s.input(Input::Tap);
        assert_eq!(events(&out), vec![Event::StopSpeaking]);
        assert!(sent(&out).is_empty());
        let out = s.incoming(response(
            req.id,
            json!({"outcome": "interrupted", "ran": [], "interrupted": []}),
        ));
        assert_eq!(
            events(&out),
            vec![Event::TurnEnded {
                outcome: "interrupted".into(),
                stopped: vec![]
            }]
        );
    }

    #[test]
    fn an_interrupt_that_stopped_an_agent_says_so() {
        let mut s = online();
        let req = start(&mut s, "send continue to the flankr agent");
        let out = s.input(Input::Tap);
        assert_eq!(events(&out), vec![Event::StopSpeaking]);
        assert_eq!(sent(&out)[0].method, "turn.interrupt");
        let out = s.incoming(response(
            req.id,
            json!({
                "outcome": "interrupted",
                "ran": [{"pane_id": "w2:pZ", "command": "continue"}],
                "interrupted": [{"pane_id": "w2:pZ", "name": "flankr", "spoken": "the flankr agent"}]
            }),
        ));
        assert_eq!(
            events(&out),
            vec![
                say("Stopped the flankr agent."),
                Event::TurnEnded {
                    outcome: "interrupted".into(),
                    stopped: vec!["the flankr agent".into()]
                }
            ]
        );
    }

    #[test]
    fn stopped_names_are_joined_in_english() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(stopped_sentence(&names(&["A", "B"])), "Stopped A and B.");
        assert_eq!(
            stopped_sentence(&names(&["A", "B", "C"])),
            "Stopped A, B and C."
        );
    }

    #[test]
    fn without_a_tap_no_stopped_sentence_is_said() {
        let mut s = online();
        let req = start(&mut s, "hi");
        let out = s.incoming(response(
            req.id,
            json!({"outcome": "interrupted", "interrupted": [{"pane_id": "p", "name": "n", "spoken": "Claude"}]}),
        ));
        assert!(!events(&out).iter().any(|e| matches!(e, Event::Say(_))));
    }

    /// Runs a turn that asks for a confirmation and ends, leaving the state
    /// confirming.
    fn confirming() -> State {
        let mut s = online();
        let req = start(&mut s, "send continue to claude");
        let out = s.incoming(confirm_requested(7));
        let text = "Approve sending \"continue\" to Claude? For keep going.";
        assert_eq!(
            events(&out),
            vec![
                Event::Confirm {
                    confirm_id: 7,
                    text: text.into(),
                    label: Some("keep-going".into()),
                    phrase: "copper-lantern-ninety".into(),
                    command: "continue".into(),
                    agent: "Claude".into(),
                    project: Some("flankr".into()),
                },
                say(text),
                Event::PendingPanes(vec!["w2:pZ".into()]),
            ]
        );
        s.incoming(response(req.id, json!({"outcome": "done", "ran": []})));
        s
    }

    #[test]
    fn tap_while_confirming_arms() {
        let mut s = confirming();
        let out = s.input(Input::Tap);
        assert_eq!(events(&out), vec![Event::Arm]);
        assert!(sent(&out).is_empty());
    }

    #[test]
    fn approve_approved_and_approve_in_capitals_approve() {
        for heard in ["approve", "Approved.", "APPROVE", "  approve!  "] {
            let mut s = confirming();
            let out = s.input(Input::Utterance(heard.into()));
            let reqs = sent(&out);
            assert_eq!(reqs.len(), 1, "{heard}: {out:?}");
            assert_eq!(reqs[0].method, "confirm.reply");
            assert_eq!(
                reqs[0].params,
                json!({"confirm_id": 7, "approve": true}),
                "{heard}"
            );
        }
    }

    #[test]
    fn yes_and_approve_it_deny_and_start_a_turn() {
        for heard in ["yes", "approve it"] {
            let mut s = confirming();
            let out = s.input(Input::Utterance(heard.into()));
            let reqs = sent(&out);
            assert_eq!(reqs.len(), 2, "{heard}: {out:?}");
            assert_eq!(reqs[0].method, "confirm.reply");
            assert_eq!(reqs[0].params, json!({"confirm_id": 7, "approve": false}));
            assert_eq!(reqs[1].method, "turn.start");
            assert_eq!(reqs[1].params["text"], heard);
        }
    }

    #[test]
    fn answers_and_refusals_are_told_to_the_card() {
        let mut s = confirming();
        let out = s.input(Input::Utterance("approve".into()));
        assert_eq!(
            events(&out),
            vec![
                Event::Answered {
                    confirm_id: 7,
                    approve: true
                },
                Event::PendingPanes(vec![])
            ]
        );
        let reply = sent(&out)[0].id.clone();
        let out = s.incoming(Incoming::Response(Response::err(
            Some(reply),
            libflankr_proto::ErrorObject::new(-32008, "wrong nonce; 4 attempt(s) left"),
        )));
        assert_eq!(
            events(&out),
            vec![Event::ReplyFailed {
                confirm_id: 7,
                message: "wrong nonce; 4 attempt(s) left (-32008)".into()
            }]
        );
    }

    #[test]
    fn the_buttons_answer_without_matching() {
        let mut s = confirming();
        let out = s.input(Input::Approve);
        assert_eq!(sent(&out)[0].params["approve"], true);
        let mut s = confirming();
        let out = s.input(Input::Deny);
        let reqs = sent(&out);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].params["approve"], false);
        // Answered: the next utterance is an ordinary turn.
        start(&mut s, "what now");
    }

    #[test]
    fn a_denial_during_a_turn_starts_its_turn_when_that_one_ends() {
        let mut s = online();
        let req = start(&mut s, "send continue to claude");
        s.incoming(confirm_requested(7));
        let out = s.input(Input::Utterance("no, compact it".into()));
        let reqs = sent(&out);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].params["approve"], false);
        let out = s.incoming(response(req.id, json!({"outcome": "done"})));
        let reqs = sent(&out);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "turn.start");
        assert_eq!(reqs[0].params["text"], "no, compact it");
    }

    #[test]
    fn resolved_gives_its_outcome_and_asks_the_next() {
        let mut s = confirming();
        s.incoming(confirm_requested(8));
        // A reminder of one already held is not asked twice.
        assert!(s.incoming(confirm_requested(8)).is_empty());
        let out = s.incoming(note(
            "confirm.resolved",
            json!({"confirm_id": 7, "outcome": "expired"}),
        ));
        let evs = events(&out);
        assert_eq!(
            evs[0],
            Event::Resolved {
                confirm_id: 7,
                outcome: "expired".into()
            }
        );
        assert!(matches!(evs[1], Event::Confirm { .. }));
        let out = s.input(Input::Approve);
        assert_eq!(sent(&out)[0].params["confirm_id"], 8);
    }

    fn notice(text: &str) -> Incoming {
        note(
            "notice",
            json!({"id": 1, "kind": "agent_status", "pane_id": "p1", "text": text}),
        )
    }

    #[test]
    fn notices_are_said_at_once_when_free() {
        let mut s = online();
        assert_eq!(
            events(&s.incoming(notice("Claude is done."))),
            vec![say("Claude is done.")]
        );
    }

    #[test]
    fn typing_into_a_terminal_is_not_said() {
        let mut s = online();
        let typed = note(
            "notice",
            json!({"id": 2, "kind": "terminal_input", "pane_id": "p1",
                   "text": "Typing into Shell's terminal."}),
        );
        assert!(events(&s.incoming(typed)).is_empty());
    }

    #[test]
    fn notices_wait_for_speaking() {
        let mut s = online();
        s.input(Input::Speaking(true));
        assert!(s.incoming(notice("one")).is_empty());
        assert!(s.incoming(notice("two")).is_empty());
        assert_eq!(
            events(&s.input(Input::Speaking(false))),
            vec![say("one"), say("two")]
        );
    }

    #[test]
    fn notices_wait_for_listening() {
        let mut s = online();
        s.input(Input::Listening(true));
        assert!(s.incoming(notice("one")).is_empty());
        assert_eq!(events(&s.input(Input::Listening(false))), vec![say("one")]);
    }

    #[test]
    fn notices_wait_for_confirming() {
        let mut s = confirming();
        assert!(s.incoming(notice("one")).is_empty());
        let out = s.input(Input::Utterance("approve".into()));
        assert_eq!(
            events(&out),
            vec![
                Event::Answered {
                    confirm_id: 7,
                    approve: true
                },
                Event::PendingPanes(vec![]),
                say("one")
            ]
        );
    }

    #[test]
    fn notices_wait_while_both_speaking_and_listening_end() {
        let mut s = online();
        s.input(Input::Speaking(true));
        s.input(Input::Listening(true));
        s.incoming(notice("one"));
        assert!(s.input(Input::Speaking(false)).is_empty());
        assert_eq!(events(&s.input(Input::Listening(false))), vec![say("one")]);
    }

    #[test]
    fn the_notice_queue_is_capped_at_8_dropping_the_oldest() {
        let mut s = online();
        s.input(Input::Speaking(true));
        for i in 0..10 {
            s.incoming(notice(&format!("n{i}")));
        }
        assert_eq!(s.held_notices(), NOTICE_CAP);
        let said = events(&s.input(Input::Speaking(false)));
        let expected: Vec<Event> = (2..10).map(|i| say(&format!("n{i}"))).collect();
        assert_eq!(said, expected);
    }

    #[test]
    fn vocabulary_changed_gives_the_new_words() {
        let mut s = online();
        let out = s.incoming(note("vocabulary.changed", json!({"vocabulary": ["codex"]})));
        assert_eq!(events(&out), vec![Event::Vocabulary(vec!["codex".into()])]);
    }

    #[test]
    fn offline_taps_and_utterances_are_errors() {
        let mut s = State::new("test");
        assert!(matches!(
            events(&s.input(Input::Tap))[..],
            [Event::Error(_)]
        ));
        let out = s.input(Input::Utterance("hi".into()));
        assert!(sent(&out).is_empty());
        assert!(matches!(events(&out)[..], [Event::Error(_)]));
    }

    #[test]
    fn a_disconnect_ends_the_running_turn() {
        let mut s = online();
        start(&mut s, "hi");
        let evs = events(&s.disconnected(2000));
        assert!(evs.contains(&Event::TurnEnded {
            outcome: "error".into(),
            stopped: vec![]
        }));
        assert_eq!(evs.last(), Some(&Event::Disconnected { retry_in_ms: 2000 }));
    }

    #[test]
    fn the_state_machine_does_not_pair() {
        let mut s = State::new("test");
        assert!(matches!(
            events(&s.input(Input::Pair("flankr1abc".into())))[..],
            [Event::PairFailed(_)]
        ));
    }

    // ── Dictation (plan 20261023_02, step 3) ─────────────────────────────

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    fn focus(pane: &str, agent: Option<&str>, name: &str) -> Value {
        let mut f = json!({"pane_id": pane, "project": "libmorph", "tab_label": "-ad",
                           "tab_number": 4, "name": name, "spoken": name});
        if let Some(agent) = agent {
            f["agent"] = json!(agent);
        }
        f
    }

    fn claude() -> Value {
        focus("w2:p4", Some("Claude"), "Claude in libmorph")
    }

    fn dictation_of(out: &[Output]) -> Option<Dictation> {
        events(out).into_iter().rev().find_map(|e| match e {
            Event::Dictation(d) => Some(d),
            _ => None,
        })
    }

    /// Answers the `agents.list` in `out` with `focus`.
    fn list_answer(s: &mut State, out: &[Output], focus: Value) -> Vec<Output> {
        let list = sent(out)
            .into_iter()
            .find(|r| r.method == "agents.list")
            .expect("an agents.list")
            .clone();
        s.incoming(response(list.id, json!({"agents": [], "focus": focus})))
    }

    /// Online, focused on Claude in libmorph, dictating, the gate closed at
    /// 10 s; the clock at 10 s.
    fn dictating() -> State {
        let mut s = online();
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        s.input(Input::Tick(secs(10.0)));
        let out = s.input(Input::Utterance("Take a message.".into()));
        assert!(events(&out).contains(&Event::Arm), "{out:?}");
        assert!(!sent(&out).iter().any(|r| r.method == "turn.start"));
        let card = dictation_of(&out).unwrap();
        assert!(card.active);
        assert_eq!(card.target.unwrap().name, "Claude in libmorph");
        list_answer(&mut s, &out, claude());
        s
    }

    /// Says `text` as the audio does: the gate opens at `from`, closes at
    /// `to`, and the utterance follows.
    fn speak(s: &mut State, from: f64, to: f64, text: &str) -> Vec<Output> {
        let mut out = s.input(Input::Tick(secs(from)));
        out.extend(s.input(Input::Listening(true)));
        out.extend(s.input(Input::Tick(secs(to))));
        out.extend(s.input(Input::Listening(false)));
        out.extend(s.input(Input::Utterance(text.into())));
        out
    }

    fn message_send(out: &[Output]) -> Option<Request> {
        sent(out)
            .into_iter()
            .find(|r| r.method == "agents.send")
            .cloned()
    }

    #[test]
    fn utterances_collect_into_one_message() {
        let mut s = dictating();
        let out = speak(&mut s, 10.0, 12.0, "Please run the tests.");
        assert!(sent(&out).is_empty(), "nothing reaches the daemon: {out:?}");
        // Re-armed for the next sentence.
        assert!(events(&out).contains(&Event::Arm));
        speak(&mut s, 13.0, 15.0, "Then send it to me.");
        let card = s.dictation();
        assert_eq!(card.text, "Please run the tests. Then send it to me.");
        assert_eq!(card.phase, DictationPhase::Collecting);
        // "send it" is only text: there is no spoken submit.
        assert!(s.dictation().active);
    }

    #[test]
    fn the_control_words() {
        let mut s = dictating();
        speak(&mut s, 10.0, 11.0, "one");
        speak(&mut s, 11.5, 12.0, "two");
        speak(&mut s, 12.5, 13.0, "Scratch that.");
        assert_eq!(s.dictation().text, "one");
        let out = s.input(Input::Utterance("read it back".into()));
        assert_eq!(events(&out), vec![say("one")]);
        assert_eq!(s.dictation().text, "one", "reading changes nothing");
        // Buttons do the same.
        s.input(Input::Utterance("three".into()));
        s.input(Input::Scratch);
        assert_eq!(s.dictation().text, "one");
        let out = s.input(Input::Utterance("Cancel".into()));
        assert_eq!(dictation_of(&out).map(|d| d.active), Some(false));
        // The mode is over: the next utterance is a turn.
        start(&mut s, "what is claude doing");
    }

    #[test]
    fn clear_and_the_start_button() {
        let mut s = online();
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        let out = s.input(Input::StartDictation);
        assert!(dictation_of(&out).unwrap().active);
        s.input(Input::Utterance("hello there".into()));
        let out = s.input(Input::Clear);
        assert_eq!(dictation_of(&out), Some(Dictation::off()));
        s.input(Input::StartDictation);
        let out = s.input(Input::Utterance("clear".into()));
        assert!(!dictation_of(&out).unwrap().active);
    }

    #[test]
    fn start_text_prompt_and_set_text() {
        let mut s = online();
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        let out = s.input(Input::StartTextPrompt);
        let d = dictation_of(&out).unwrap();
        assert!(d.active);
        assert_eq!(d.mode, PromptMode::Keyboard);
        assert_eq!(d.text, "");
        let out = s.input(Input::SetText("Please run tests".into()));
        let d = dictation_of(&out).unwrap();
        assert!(d.active);
        assert_eq!(d.text, "Please run tests");
        let out = s.input(Input::Submit);
        let send = message_send(&out).expect("submitted text");
        assert_eq!(send.params["text"], "Please run tests");
    }

    #[test]
    fn eight_seconds_of_silence_submit_and_7_9_do_not() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "Please run the tests.");
        assert!(message_send(&s.input(Input::Tick(secs(19.9)))).is_none());
        let out = s.input(Input::Tick(secs(20.0)));
        let send = message_send(&out).expect("submitted at 8.0 s");
        assert_eq!(
            send.params,
            json!({"pane_id": "w2:p4", "text": "Please run the tests.", "kind": "message",
                   "handle": "message-claude-libmorph-please-run"})
        );
        assert_eq!(s.dictation().phase, DictationPhase::Submitting);
        // Submitted once.
        assert!(message_send(&s.input(Input::Tick(secs(40.0)))).is_none());
    }

    #[test]
    fn speech_before_the_silence_runs_out_resets_it() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "one");
        // Speech at 7 s: the gate opens at 19 and closes at 21.
        let out = speak(&mut s, 19.0, 21.0, "two");
        assert!(message_send(&out).is_none());
        assert!(message_send(&s.input(Input::Tick(secs(28.9)))).is_none());
        let send = message_send(&s.input(Input::Tick(secs(29.0)))).unwrap();
        assert_eq!(send.params["text"], "one two");
    }

    /// Says `text` as an audio front end with a VAD does: the gate is
    /// re-armed and waits (listening, no voice), voice runs from `from` to
    /// `to`, and the utterance follows.
    fn speak_with_voice(s: &mut State, from: f64, to: f64, text: &str) -> Vec<Output> {
        let mut out = s.input(Input::Tick(secs(from)));
        out.extend(s.input(Input::Listening(true)));
        out.extend(s.input(Input::Voice(true)));
        out.extend(s.input(Input::Tick(secs(to))));
        out.extend(s.input(Input::Voice(false)));
        out.extend(s.input(Input::Utterance(text.into())));
        out
    }

    #[test]
    fn an_armed_gate_waiting_for_voice_does_not_hold_the_silence() {
        // The re-armed gate stays "listening" for its arm time-out with no
        // voice. The silence counts from the end of voice, so it submits 8 s
        // after the speaker stopped, not 8 s after the gate gave up.
        let mut s = dictating();
        speak_with_voice(&mut s, 10.0, 12.0, "Please run the tests.");
        // Re-armed and still waiting at 18 s: listening, but no voice.
        s.input(Input::Tick(secs(13.0)));
        s.input(Input::Listening(true));
        assert!(message_send(&s.input(Input::Tick(secs(19.9)))).is_none());
        let send = message_send(&s.input(Input::Tick(secs(20.0)))).expect("8 s after voice");
        assert_eq!(send.params["text"], "Please run the tests.");
        // The gate going idle later changes nothing.
        assert!(message_send(&s.input(Input::Listening(false))).is_none());
    }

    #[test]
    fn voice_resets_the_silence_even_while_armed() {
        let mut s = dictating();
        speak_with_voice(&mut s, 10.0, 12.0, "one");
        s.input(Input::Listening(true));
        let out = speak_with_voice(&mut s, 18.0, 21.0, "two");
        assert!(message_send(&out).is_none());
        assert!(message_send(&s.input(Input::Tick(secs(28.9)))).is_none());
        let send = message_send(&s.input(Input::Tick(secs(29.0)))).unwrap();
        assert_eq!(send.params["text"], "one two");
    }

    #[test]
    fn the_silence_is_at_least_8_s() {
        let mut s = State::new("t");
        s.set_dictation_silence(secs(5.0));
        assert_eq!(s.dictation_silence(), secs(8.0));
        let mut s = dictating();
        s.set_dictation_silence(secs(12.0));
        speak(&mut s, 10.0, 12.0, "one");
        assert!(message_send(&s.input(Input::Tick(secs(23.9)))).is_none());
        assert!(message_send(&s.input(Input::Tick(secs(24.0)))).is_some());
    }

    #[test]
    fn the_card_counts_down_the_last_3_seconds() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "one");
        let mut shown = Vec::new();
        for tenth in 121..=199 {
            let out = s.input(Input::Tick(secs(tenth as f64 / 10.0)));
            if let Some(d) = dictation_of(&out) {
                shown.push(d.submit_in);
            }
        }
        assert_eq!(shown, vec![Some(3), Some(2), Some(1)]);
        // Scratch cancels the countdown, and the timer starts again.
        let out = s.input(Input::Scratch);
        assert_eq!(dictation_of(&out).unwrap().submit_in, None);
    }

    #[test]
    fn the_submit_button_submits_at_once() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "one");
        let send = message_send(&s.input(Input::Submit)).expect("submitted");
        assert_eq!(send.params["text"], "one");
        // An empty message is not submitted.
        let mut s = dictating();
        let out = s.input(Input::Submit);
        assert!(message_send(&out).is_none());
        assert_eq!(events(&out), vec![say("The message is empty.")]);
    }

    #[test]
    fn reading_it_back_pauses_the_timer() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "one");
        s.input(Input::Tick(secs(13.0)));
        let out = s.input(Input::Utterance("Read it back.".into()));
        assert_eq!(events(&out), vec![say("one")]);
        s.input(Input::Speaking(true));
        assert!(message_send(&s.input(Input::Tick(secs(30.0)))).is_none());
        s.input(Input::Tick(secs(31.0)));
        let out = s.input(Input::Speaking(false));
        assert!(events(&out).contains(&Event::Arm));
        // Counted again from the end of the reading.
        assert!(message_send(&s.input(Input::Tick(secs(38.9)))).is_none());
        assert!(message_send(&s.input(Input::Tick(secs(39.0)))).is_some());
    }

    /// Submits "one" from [`dictating`] by the silence, which asks, and
    /// answers it as pending, confirmation 9, asked as the daemon asks it.
    /// A message submitted by the silence and approved as its own, waiting
    /// for the daemon to send it.
    fn submitted_message() -> State {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "one");
        let send = message_send(&s.input(Input::Tick(secs(20.0)))).unwrap();
        let out = s.incoming(own_question(9, "one"));
        assert_eq!(
            approvals(&out),
            vec![json!({"confirm_id": 9, "approve": true})]
        );
        s.incoming(response(
            send.id,
            json!({"status": "pending", "confirm_id": 9,
                   "handle": "message-claude-libmorph-one"}),
        ));
        assert_eq!(s.dictation().phase, DictationPhase::Confirming);
        s
    }

    fn own_question(id: u64, command: &str) -> Incoming {
        note(
            "confirm.requested",
            json!({"confirm_id": id, "pane_id": "w2:p4", "agent": "claude",
                   "name": "Claude in libmorph", "command": command,
                   "requested_by": "companion:test",
                   "handle": "message-claude-libmorph-one",
                   "text": "Approve sending your message to Claude in libmorph?"}),
        )
    }

    fn approvals(out: &[Output]) -> Vec<serde_json::Value> {
        sent(out)
            .into_iter()
            .filter(|r| r.method == "confirm.reply")
            .map(|r| r.params.clone())
            .collect()
    }

    #[test]
    fn a_button_submit_is_its_own_approval_question_first() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "Please run the tests.");
        let send = message_send(&s.input(Input::Submit)).unwrap();
        // The daemon cleans the trailing full stop; it is still ours.
        let out = s.incoming(own_question(9, "Please run the tests"));
        assert!(
            !events(&out)
                .iter()
                .any(|e| matches!(e, Event::Confirm { .. } | Event::Say(_))),
            "not asked: {out:?}"
        );
        assert_eq!(
            approvals(&out),
            vec![json!({"confirm_id": 9, "approve": true})]
        );
        let out = s.incoming(response(
            send.id,
            json!({"status": "pending", "confirm_id": 9, "handle": "message-claude-libmorph-one"}),
        ));
        assert!(approvals(&out).is_empty(), "approved once");
        let out = s.incoming(note(
            "confirm.resolved",
            json!({"confirm_id": 9, "outcome": "sent"}),
        ));
        assert_eq!(dictation_of(&out).map(|d| d.active), Some(false));
    }

    #[test]
    fn a_button_submit_is_its_own_approval_answer_first() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "one");
        let send = message_send(&s.input(Input::Submit)).unwrap();
        s.incoming(response(
            send.id,
            json!({"status": "pending", "confirm_id": 9, "handle": "message-claude-libmorph-one"}),
        ));
        let out = s.incoming(own_question(9, "one"));
        assert!(
            !events(&out)
                .iter()
                .any(|e| matches!(e, Event::Confirm { .. })),
            "{out:?}"
        );
        assert_eq!(
            approvals(&out),
            vec![json!({"confirm_id": 9, "approve": true})]
        );
    }

    #[test]
    fn a_silence_submit_is_its_own_approval_too() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "one");
        message_send(&s.input(Input::Tick(secs(20.0)))).unwrap();
        let out = s.incoming(own_question(9, "one"));
        assert!(
            !events(&out)
                .iter()
                .any(|e| matches!(e, Event::Confirm { .. })),
            "{out:?}"
        );
        assert_eq!(
            approvals(&out),
            vec![json!({"confirm_id": 9, "approve": true})]
        );
    }

    #[test]
    fn denied_elsewhere_resumes_the_mode_with_the_text_kept() {
        // Approved here, but denied by another client before it was sent.
        let mut s = submitted_message();
        let out = s.incoming(note(
            "confirm.resolved",
            json!({"confirm_id": 9, "outcome": "denied"}),
        ));
        let card = dictation_of(&out).unwrap();
        assert!(card.active);
        assert_eq!(card.text, "one");
        assert_eq!(card.phase, DictationPhase::Collecting);
        // Not asked again by the silence alone.
        assert!(message_send(&s.input(Input::Tick(secs(100.0)))).is_none());
        // Edited, it is submitted again after the silence.
        speak(&mut s, 101.0, 102.0, "two");
        let send = message_send(&s.input(Input::Tick(secs(110.0)))).unwrap();
        assert_eq!(send.params["text"], "one two");
    }

    #[test]
    fn approve_during_dictation_answers_another_confirmation() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "one");
        s.incoming(confirm_requested(7));
        let out = s.input(Input::Utterance("approve".into()));
        assert_eq!(
            sent(&out)[0].params,
            json!({"confirm_id": 7, "approve": true})
        );
        assert_eq!(s.dictation().text, "one");
        // With nothing to answer, approve and deny are not text either.
        s.input(Input::Utterance("approve".into()));
        s.input(Input::Utterance("deny".into()));
        assert_eq!(s.dictation().text, "one");
        // Anything else while a confirmation waits denies it, and is text.
        s.incoming(confirm_requested(8));
        let out = s.input(Input::Utterance("and two".into()));
        assert_eq!(
            sent(&out)[0].params,
            json!({"confirm_id": 8, "approve": false})
        );
        assert!(!sent(&out).iter().any(|r| r.method == "turn.start"));
        assert_eq!(s.dictation().text, "one and two");
    }

    #[test]
    fn a_refused_message_is_kept_and_the_reason_said() {
        let mut s = dictating();
        speak(&mut s, 10.0, 12.0, "see src/main.rs");
        let send = message_send(&s.input(Input::Submit)).unwrap();
        let refusal = Response::err(
            Some(send.id),
            libflankr_proto::ErrorObject::new(
                -32001,
                "Message rejected: the character '/' is not permitted.",
            ),
        );
        let out = s.incoming(Incoming::Response(refusal));
        assert!(
            events(&out)
                .iter()
                .any(|e| matches!(e, Event::Say(t) if t.contains("the character '/'")))
        );
        let card = s.dictation();
        assert_eq!(card.phase, DictationPhase::Collecting);
        assert_eq!(card.text, "see src/main.rs");
        s.input(Input::Utterance("scratch that".into()));
        assert_eq!(s.dictation().text, "");
    }

    #[test]
    fn the_message_is_capped_at_4000_characters() {
        let mut s = dictating();
        s.input(Input::Utterance("a".repeat(3990)));
        s.input(Input::Utterance("b".repeat(9)));
        assert_eq!(s.dictation().text.len(), 4000);
        let out = s.input(Input::Utterance("c".into()));
        assert_eq!(events(&out), vec![say("The message is full.")]);
        assert_eq!(s.dictation().text.len(), 4000);
    }

    #[test]
    fn the_target_is_fixed_and_a_moved_focus_is_shown() {
        let mut s = dictating();
        let elsewhere = focus("w2:p9", Some("Claude"), "Claude in keypad");
        let out = s.incoming(note("focus.changed", json!({"focus": elsewhere})));
        let card = dictation_of(&out).unwrap();
        assert_eq!(card.target.as_ref().unwrap().pane_id, "w2:p4");
        assert_eq!(card.focus_moved.as_deref(), Some("Claude in keypad"));
        s.input(Input::Utterance("Use the focus.".into()));
        let card = s.dictation();
        assert_eq!(card.target.unwrap().pane_id, "w2:p9");
        assert_eq!(card.focus_moved, None);
        assert_eq!(card.text, "", "not text");
    }

    #[test]
    fn without_a_target_submit_is_refused_until_focus_sets_one() {
        let mut s = online();
        let out = s.input(Input::Utterance("dictate".into()));
        // Nothing focused, nor in the fresh list.
        list_answer(&mut s, &out, Value::Null);
        assert_eq!(s.dictation().target, None);
        speak(&mut s, 10.0, 11.0, "one");
        let out = s.input(Input::Tick(secs(19.0)));
        assert!(message_send(&out).is_none());
        assert!(
            events(&out)
                .iter()
                .any(|e| matches!(e, Event::Say(t) if t.starts_with("No agent is focused")))
        );
        // Said once, not on every tick.
        assert!(events(&s.input(Input::Tick(secs(30.0)))).is_empty());

        // "focus Claude in libmorph" focuses it through the daemon.
        let out = s.input(Input::Utterance("Focus Claude in libmorph.".into()));
        let focus_req = sent(&out)[0].clone();
        assert_eq!(focus_req.method, "agents.focus");
        assert_eq!(focus_req.params, json!({"target_id": "Claude in libmorph"}));
        assert_eq!(s.dictation().text, "one", "not text");
        let out = s.incoming(response(focus_req.id, json!({})));
        list_answer(&mut s, &out, claude());
        assert_eq!(s.dictation().target.unwrap().pane_id, "w2:p4");
        assert!(message_send(&s.input(Input::Submit)).is_some());

        // A focused pane with no agent is no target either.
        let mut s = online();
        let shell = focus("w1:p1", None, "the home shell");
        s.incoming(note("focus.changed", json!({"focus": shell})));
        s.input(Input::StartDictation);
        s.input(Input::Utterance("one".into()));
        let out = s.input(Input::Submit);
        assert!(message_send(&out).is_none());
        assert!(
            events(&out).iter().any(
                |e| matches!(e, Event::Say(t) if t.starts_with("The focused pane has no agent"))
            )
        );
        // Focus moving to an agent sets it.
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        assert_eq!(s.dictation().target.unwrap().pane_id, "w2:p4");
    }

    #[test]
    fn the_focus_is_learned_after_hello() {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(id, json!({"server": "flankr", "vocabulary": []})));
        let out = list_answer(&mut s, &out, claude());
        assert!(
            matches!(&events(&out)[..], [Event::Agents(a), Event::Focus(Some(f))] if a.is_empty() && f.pane_id == "w2:p4")
        );
    }

    #[test]
    fn message_handles() {
        let target: common::FocusInfo = serde_json::from_value(claude()).unwrap();
        assert_eq!(
            message_handle(&target, "Please run the tests."),
            "message-claude-libmorph-please-run"
        );
        let long = common::FocusInfo {
            project: Some("a-very-long-project-name-that-goes-on-and-on".into()),
            ..target
        };
        let h = message_handle(&long, "Please run");
        assert!(h.len() <= 64, "{h}");
        assert!(h.starts_with("message-claude-a-very-long"), "{h}");
    }

    #[test]
    fn approval_matching() {
        for yes in ["approve", "Approved.", "APPROVE", " approved ", "Approve!"] {
            assert!(is_approval(yes), "{yes}");
        }
        for no in ["yes", "approve it", "ok, approve", "", "approves"] {
            assert!(!is_approval(no), "{no}");
        }
    }

    // The agent view (plan 20261022_03, step 4).

    fn agent_info(pane: &str, status: &str) -> Value {
        json!({"pane_id": pane, "name": "the flankr agent", "spoken": "the flankr agent",
               "agent": "claude", "project": "flankr", "tab_label": "17-fl",
               "tab_number": 17, "status": status, "focused": false})
    }

    fn requests<'a>(out: &'a [Output], method: &str) -> Vec<&'a Request> {
        sent(out)
            .into_iter()
            .filter(|r| r.method == method)
            .collect()
    }

    fn view_of(
        out: &[Output],
    ) -> Option<(
        Vec<common::Entry>,
        Vec<crate::view::AgentCard>,
        Option<usize>,
    )> {
        out.iter().rev().find_map(|o| match o {
            Output::Event(Event::AgentView {
                sections, pinned, ..
            }) => Some(crate::view::flatten_view(sections, *pinned)),
            _ => None,
        })
    }

    /// The `changed` of the last view given in `out`, as (line, slot).
    fn changed_of(out: &[Output]) -> Option<Vec<(u32, u8)>> {
        out.iter().rev().find_map(|o| match o {
            Output::Event(Event::AgentView { changed, .. }) => {
                Some(changed.iter().map(|s| (s.line, s.slot)).collect())
            }
            _ => None,
        })
    }

    /// The file hash the scripted sessions' serials carry.
    const FILE: u32 = 0x3fa9_c21e;

    fn serial(line: u32, slot: u8) -> String {
        common::Serial::new(FILE, line, slot).to_string()
    }

    /// The sections the daemon would give for `session` (the shape of a
    /// `session.peek` answer: `entries` and `pending_question`): each entry
    /// a section on its own line, and the pending question a live Question
    /// card after its reply.
    fn sections_of(session: &Value) -> Vec<Value> {
        let mut sections = Vec::new();
        let entries = session["entries"].as_array().cloned().unwrap_or_default();
        let q = &session["pending_question"];
        for (i, e) in entries.iter().enumerate() {
            let line = i as u32 + 1;
            let kind = match (e["role"].as_str(), e["kind"].as_str()) {
                _ if e.get("origin").is_some() => "context",
                (Some("user"), Some("text")) => "turn",
                (_, Some("text")) => "text",
                _ => "tool",
            };
            sections.push(json!({"serial": serial(line, 0), "kind": kind, "entries": [e]}));
            if q.is_object() && q["ts"] == e["ts"] && kind == "text" {
                let mut card = json!({"card": "question", "live": true,
                    "title": "the flankr agent is asking", "text": q["text"]});
                if q["about_plan"] == true {
                    card["about_plan"] = json!(true);
                }
                if q.get("links").is_some() {
                    card["links"] = q["links"].clone();
                }
                sections.push(json!({"serial": serial(line, 128), "kind": "card", "card": card}));
            }
        }
        sections
    }

    /// A reset delivery of `session` (null: no session) for subscription 1.
    fn delivery_of(status: &str, session: &Value) -> Value {
        if session.is_null() {
            return json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
                "reset": true, "session": null, "reason": "no_session", "sections": [],
                "status": status, "pending_question": null});
        }
        json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
            "reset": true, "session": {"session_id": "s1", "matched": "id"}, "reason": null,
            "sections": sections_of(session), "status": status,
            "pending_question": session["pending_question"]})
    }

    /// The daemon gives the open view `session`: as `session.subscribe`'s
    /// answer when `out` asked for it, else as a `session.delivery`.
    fn session_answer(s: &mut State, out: &[Output], status: &str, session: Value) -> Vec<Output> {
        let delivery = delivery_of(status, &session);
        match requests(out, "session.subscribe").first() {
            Some(sub) => {
                let id = sub.id.clone();
                s.incoming(response(
                    id,
                    json!({"subscription_id": 1, "delivery": delivery}),
                ))
            }
            None => s.incoming(note("session.delivery", delivery)),
        }
    }

    /// A delivery for subscription 1 of these sections, not a reset.
    fn deliver(s: &mut State, sections: Vec<Value>, removed: Vec<String>) -> Vec<Output> {
        s.incoming(note(
            "session.delivery",
            json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
                "reset": false, "session": {"session_id": "s1", "matched": "id"},
                "reason": null, "sections": sections, "removed": removed}),
        ))
    }

    fn text_at(line: u32, slot: u8, summary: &str) -> Value {
        json!({"serial": serial(line, slot), "kind": "text", "entries": [
            {"ts": format!("{line}.{slot}"), "role": "assistant", "kind": "text", "summary": summary}
        ]})
    }

    fn summaries(s: &State) -> Vec<String> {
        match s.agent_view() {
            Some(Event::AgentView {
                sections, pinned, ..
            }) => crate::view::flatten_view(&sections, pinned)
                .0
                .into_iter()
                .map(|e| e.summary)
                .collect(),
            other => panic!("{other:?}"),
        }
    }

    fn asking() -> Value {
        json!({
            "session_id": "s1", "matched": "id",
            "entries": [
                {"ts": "1", "role": "user", "kind": "text", "summary": "go"},
                {"ts": "2", "role": "assistant", "kind": "text", "summary": "Done.\n\nShall I go on?"}
            ],
            "pending_question": {"text": "Shall I go on?", "ts": "2"}
        })
    }

    /// Online with the view of w2:pZ open and its subscription answered with
    /// `session`; the clock at 100 s.
    fn viewing(status: &str, session: Value) -> State {
        let mut s = online();
        s.input(Input::Tick(secs(100.0)));
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let list = requests(&out, "agents.list")[0].clone();
        s.incoming(response(
            list.id,
            json!({"agents": [agent_info("w2:pZ", status)]}),
        ));
        session_answer(&mut s, &out, status, session);
        s
    }

    #[test]
    fn another_agents_confirmation_is_no_card() {
        let mut s = viewing("working", json!(null));
        let out = s.incoming(note(
            "confirm.requested",
            json!({"confirm_id": 8, "pane_id": "w9:p1", "agent": "codex",
                   "command": "continue", "text": "Approve?"}),
        ));
        assert!(view_of(&out).is_none(), "{out:?}");
    }

    #[test]
    fn a_cards_buttons_answer_its_own_confirmation() {
        let mut s = viewing("working", json!(null));
        s.incoming(note(
            "confirm.requested",
            json!({"confirm_id": 8, "pane_id": "w9:p1", "agent": "codex",
                   "command": "continue", "text": "Approve for codex?"}),
        ));
        s.incoming(confirm_requested(7));
        // 8 is being asked; the card answers 7.
        let out = s.input(Input::Answer {
            confirm_id: 7,
            approve: false,
        });
        let replies = requests(&out, "confirm.reply");
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].params["confirm_id"], 7);
        assert_eq!(replies[0].params["approve"], false);
        // 8 is still asked, and Approve answers it.
        let out = s.input(Input::Approve);
        assert_eq!(requests(&out, "confirm.reply")[0].params["confirm_id"], 8);
    }

    #[test]
    fn opening_the_view_subscribes_and_lists() {
        let mut s = online();
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let subs = requests(&out, "session.subscribe");
        assert_eq!(subs.len(), 1, "{out:?}");
        assert_eq!(
            subs[0].params,
            json!({"pane_id": "w2:pZ", "sections": 200, "outline": true})
        );
        assert_eq!(requests(&out, "agents.list").len(), 1);
        assert!(requests(&out, "session.peek").is_empty());
        // Shown at once, empty and loading, until the subscription answers.
        let (entries, cards, pinned) = view_of(&out).expect("a view");
        assert!(entries.is_empty() && cards.is_empty() && pinned.is_none());
        assert!(matches!(
            s.agent_view(),
            Some(Event::AgentView { loading: true, .. })
        ));

        let out = session_answer(&mut s, &out, "idle", asking());
        let (entries, cards, pinned) = view_of(&out).expect("the view again");
        assert_eq!(entries.len(), 2);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, crate::view::CardKind::Question);
        assert_eq!(cards[0].at, 2);
        assert_eq!(pinned, Some(0));
        match s.agent_view() {
            Some(Event::AgentView {
                name,
                status,
                loading,
                ..
            }) => {
                assert_eq!((name.as_str(), status.as_str()), ("the agent", "idle"));
                assert!(!loading);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_view_is_named_from_agents_list_and_closing_unsubscribes() {
        let mut s = viewing("idle", asking());
        match s.agent_view() {
            Some(Event::AgentView { name, .. }) => assert_eq!(name, "the flankr agent"),
            other => panic!("{other:?}"),
        }
        let out = s.input(Input::CloseAgent);
        assert_eq!(
            requests(&out, "session.unsubscribe")[0].params,
            json!({"subscription_id": 1})
        );
        assert!(s.agent_view().is_none());
        // Closed, nothing is asked for it, and its late deliveries are
        // ignored.
        assert!(requests(&s.input(Input::Tick(secs(200.0))), "session.peek").is_empty());
        let out = deliver(&mut s, vec![text_at(9, 0, "late")], vec![]);
        assert!(view_of(&out).is_none());
    }

    #[test]
    fn an_approval_card_comes_from_the_daemon_and_fades() {
        let mut s = viewing("working", asking());
        // A confirmation for the pane makes no card of the client's own.
        let out = s.incoming(confirm_requested(7));
        assert!(view_of(&out).is_none(), "{out:?}");
        let approval = |live: bool| {
            json!({"serial": serial(2, 64), "kind": "card", "card": {
                "card": "approval", "live": live, "title": "Approval needed",
                "text": "Approve sending \"continue\" to Claude?", "confirm_id": 7}})
        };
        let out = deliver(&mut s, vec![approval(true)], vec![]);
        let (_, cards, pinned) = view_of(&out).expect("the view");
        let kinds: Vec<_> = cards.iter().map(|c| (c.kind, c.live)).collect();
        assert_eq!(
            kinds,
            [
                (crate::view::CardKind::Approval, true),
                (crate::view::CardKind::Question, true)
            ]
        );
        assert_eq!(cards[0].confirm_id, Some(7));
        assert_eq!(pinned, Some(0), "an approval outranks the question");
        let out = deliver(&mut s, vec![approval(false)], vec![]);
        let (_, cards, pinned) = view_of(&out).expect("the view");
        assert_eq!(cards.len(), 2, "resent in place, not added");
        assert!(!cards[0].live, "resolved: faded");
        assert_eq!(pinned, Some(1));
    }

    #[test]
    fn a_question_resolves_when_the_daemon_says_so() {
        let mut s = viewing("idle", asking());
        let resolved = json!({"serial": serial(2, 128), "kind": "card", "card": {
            "card": "question", "live": false, "title": "the flankr agent is asking",
            "text": "Shall I go on?"}});
        let yes = json!({"serial": serial(3, 0), "kind": "turn", "entries": [
            {"ts": "3", "role": "user", "kind": "text", "summary": "yes"}]});
        let out = deliver(&mut s, vec![resolved, yes], vec![]);
        let (entries, cards, pinned) = view_of(&out).expect("the view");
        assert_eq!(entries.len(), 3);
        assert_eq!(cards.len(), 1);
        assert!(!cards[0].live);
        assert_eq!(cards[0].at, 2, "still at its reply");
        assert_eq!(pinned, None);
        // A card that no longer stands goes.
        let out = deliver(&mut s, vec![], vec![serial(2, 128)]);
        let (_, cards, _) = view_of(&out).expect("the view");
        assert!(cards.is_empty());
    }

    #[test]
    fn a_disconnect_keeps_the_view_and_subscribes_from_its_newest_on_hello() {
        let mut s = viewing("idle", asking());
        s.input(Input::Tick(secs(101.0)));
        s.disconnected(1000);
        let before = summaries(&s);
        assert_eq!(before.len(), 2);
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(id, json!({"server": "flankr"})));
        let subs = requests(&out, "session.subscribe");
        assert_eq!(subs.len(), 1, "{out:?}");
        assert_eq!(
            subs[0].params,
            json!({"pane_id": "w2:pZ", "sections": 200, "after": serial(2, 128), "outline": true})
        );
        assert!(requests(&out, "session.peek").is_empty());
        // Only what was missed comes, not a reset: what was shown stays.
        let sub = subs[0].id.clone();
        s.incoming(response(
            sub,
            json!({"subscription_id": 4, "delivery": {
                "subscription_id": 4, "pane_id": "w2:pZ", "agent": "claude",
                "session": {"session_id": "s1", "matched": "id"}, "reason": null,
                "sections": [text_at(3, 0, "missed")]}}),
        ));
        assert_eq!(
            summaries(&s),
            [&before[..], &["missed".to_string()]].concat()
        );
        // Deliveries for the old subscription are ignored.
        deliver(&mut s, vec![text_at(4, 0, "old")], vec![]);
        assert_eq!(summaries(&s).len(), 3);
    }

    #[test]
    fn sections_append_reset_replace_and_drop() {
        let mut s = viewing("working", json!(null));
        assert!(summaries(&s).is_empty());
        // Appended, in serial order.
        deliver(
            &mut s,
            vec![text_at(1, 0, "one"), text_at(2, 0, "two")],
            vec![],
        );
        assert_eq!(summaries(&s), ["one", "two"]);
        // A resent section replaces in place; a removed one goes.
        deliver(
            &mut s,
            vec![text_at(1, 0, "one, grown")],
            vec![serial(2, 0)],
        );
        assert_eq!(summaries(&s), ["one, grown"]);
        // Another session file's sections, without a reset: ignored.
        let mut other = text_at(5, 0, "other file");
        other["serial"] = json!(common::Serial::new(FILE + 1, 5, 0).to_string());
        deliver(&mut s, vec![other.clone()], vec![]);
        assert_eq!(summaries(&s), ["one, grown"]);
        // With a reset, they replace everything.
        s.incoming(note(
            "session.delivery",
            json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
                "reset": true, "session": {"session_id": "s2", "matched": "id"},
                "reason": null, "sections": [other]}),
        ));
        assert_eq!(summaries(&s), ["other file"]);
        // No session: nothing shown, and not loading.
        let out = s.incoming(note(
            "session.delivery",
            json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
                "session": null, "reason": "pane_closed"}),
        ));
        assert!(matches!(
            view_of(&out),
            Some((entries, _, _)) if entries.is_empty()
        ));
        // A closed pane ends the subscription: no unsubscribe on close.
        let out = s.input(Input::CloseAgent);
        assert!(requests(&out, "session.unsubscribe").is_empty());
    }

    #[test]
    fn status_and_question_change_only_when_given() {
        let mut s = viewing("idle", asking());
        // Absent: unchanged.
        deliver(&mut s, vec![], vec![]);
        assert!(s.view.as_ref().unwrap().pending_question.is_some());
        s.incoming(note(
            "session.delivery",
            json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
                "session": {"session_id": "s1", "matched": "id"}, "reason": null,
                "status": "working", "pending_question": null}),
        ));
        let v = s.view.as_ref().unwrap();
        assert_eq!(v.status, "working");
        assert!(v.pending_question.is_none(), "null clears it");
    }

    /// The sections, as (line, slot, kind), in the order the view holds.
    fn held(s: &State) -> Vec<String> {
        s.view
            .as_ref()
            .unwrap()
            .sections
            .iter()
            .map(|x| x.serial.to_string())
            .collect()
    }

    #[test]
    fn the_view_is_in_serial_order_however_sections_arrive() {
        // Twelve sections, two to a line, and a card.
        let all: Vec<Value> = (1..=6)
            .flat_map(|line| [text_at(line, 0, "a"), text_at(line, 1, "b")])
            .chain([json!({"serial": serial(6, 194), "kind": "card", "card": {
                "card": "finished", "live": true, "title": "Finished", "text": "b"}})])
            .collect();
        let sorted: Vec<String> = all
            .iter()
            .map(|x| x["serial"].as_str().unwrap().to_string())
            .collect();
        // In order, in one delivery.
        let mut a = viewing("idle", json!(null));
        deliver(&mut a, all.clone(), vec![]);
        // Shuffled, duplicated, and split between live deliveries and a
        // history page.
        let mut b = viewing("idle", json!(null));
        let order = [9, 3, 12, 0, 7, 7, 5, 11, 1, 3, 10, 9];
        deliver(
            &mut b,
            order[..4].iter().map(|&i| all[i].clone()).collect(),
            vec![],
        );
        deliver(
            &mut b,
            order[4..8].iter().map(|&i| all[i].clone()).collect(),
            vec![],
        );
        let out = b.input(Input::LoadEarlier);
        let page = requests(&out, "session.history")[0].clone();
        b.incoming(response(
            page.id,
            json!({"sections": [all[2].clone(), all[4].clone(), all[6].clone(), all[8].clone(),
                                all[0].clone()],
                   "start": false}),
        ));
        deliver(
            &mut b,
            order[8..].iter().map(|&i| all[i].clone()).collect(),
            vec![],
        );
        assert_eq!(held(&a), sorted);
        assert_eq!(held(&b), sorted);
        assert_eq!(a.agent_view(), b.agent_view());
        // A resent section replaces in place, never adds a copy.
        deliver(&mut b, vec![text_at(3, 1, "b, again")], vec![]);
        assert_eq!(held(&b), sorted);
        assert_eq!(summaries(&b)[5], "b, again");
    }

    fn history_marks(s: &State) -> (bool, bool) {
        match s.agent_view() {
            Some(Event::AgentView {
                loading_earlier,
                start,
                ..
            }) => (loading_earlier, start),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_view_marks_the_serials_changed_since_it_was_last_given() {
        let mut s = online();
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        assert_eq!(changed_of(&out), Some(vec![]), "nothing held yet");
        // The first answer: every section came.
        let out = session_answer(&mut s, &out, "idle", asking());
        assert_eq!(changed_of(&out), Some(vec![(1, 0), (2, 0), (2, 128)]));
        // A new section: only it.
        let out = deliver(&mut s, vec![text_at(5, 0, "five")], vec![]);
        assert_eq!(changed_of(&out), Some(vec![(5, 0)]));
        // Resent as it was: nothing to give.
        let out = deliver(&mut s, vec![text_at(5, 0, "five")], vec![]);
        assert_eq!(changed_of(&out), None);
        // Replaced with different content, and another removed.
        let out = deliver(
            &mut s,
            vec![text_at(5, 0, "five, grown")],
            vec![serial(1, 0)],
        );
        assert_eq!(changed_of(&out), Some(vec![(1, 0), (5, 0)]));
        // A page landing before them: only its sections, though every
        // index after them moves on.
        let out = s.input(Input::LoadEarlier);
        let page = requests(&out, "session.history")[0].clone();
        let out = s.incoming(response(
            page.id,
            json!({"sections": [text_at(0, 1, "older"), text_at(0, 2, "old")], "start": true}),
        ));
        assert_eq!(changed_of(&out), Some(vec![(0, 1), (0, 2)]));
        let Some(Event::AgentView { sections, .. }) = s.agent_view() else {
            panic!("a view");
        };
        let placed: Vec<(u64, u32)> = sections.iter().map(|v| (v.index, v.serial.line)).collect();
        assert_eq!(placed, [(0, 0), (1, 0), (2, 2), (3, 2), (4, 5)]);
        // The question answered from the view: its card goes, and is marked.
        let out = s.input(Input::Respond(Suggestion::Yes));
        assert_eq!(changed_of(&out), Some(vec![(2, 128)]));
    }

    #[test]
    fn scrolling_back_pages_in_history_one_page_at_a_time_to_the_start() {
        let mut s = viewing("idle", json!(null));
        // Nothing held: nothing to page back from.
        assert!(requests(&s.input(Input::LoadEarlier), "session.history").is_empty());
        deliver(&mut s, vec![text_at(300, 0, "newest")], vec![]);
        let out = s.input(Input::LoadEarlier);
        let pages = requests(&out, "session.history");
        assert_eq!(
            pages[0].params,
            json!({"subscription_id": 1, "before": serial(300, 0), "sections": 100})
        );
        assert_eq!(history_marks(&s), (true, false));
        // One in flight at a time.
        assert!(requests(&s.input(Input::LoadEarlier), "session.history").is_empty());
        let id = pages[0].id.clone();
        s.incoming(response(
            id,
            json!({"sections": [text_at(100, 0, "older")], "start": false}),
        ));
        assert_eq!(summaries(&s), ["older", "newest"]);
        assert_eq!(history_marks(&s), (false, false));
        // The next page is from the new oldest; its error SESSION_CHANGED
        // waits for the reset.
        let out = s.input(Input::LoadEarlier);
        let page = requests(&out, "session.history")[0].clone();
        assert_eq!(page.params["before"], json!(serial(100, 0)));
        let changed = Response::err(
            Some(page.id.clone()),
            libflankr_proto::ErrorObject::new(-32014, "the session changed"),
        );
        let out = s.incoming(Incoming::Response(changed));
        assert!(!events(&out).iter().any(|e| matches!(e, Event::Error(_))));
        assert_eq!(history_marks(&s), (false, false));
        // A page that reaches the start: no more are asked.
        let out = s.input(Input::LoadEarlier);
        let page = requests(&out, "session.history")[0].clone();
        s.incoming(response(
            page.id,
            json!({"sections": [text_at(1, 0, "first")], "start": true}),
        ));
        assert_eq!(history_marks(&s), (false, true));
        assert_eq!(summaries(&s), ["first", "older", "newest"]);
        assert!(requests(&s.input(Input::LoadEarlier), "session.history").is_empty());
        // A reset clears the start.
        session_answer(&mut s, &[], "idle", asking());
        assert_eq!(history_marks(&s), (false, false));
    }

    #[test]
    fn history_pages_are_not_read_aloud_but_live_replies_are() {
        let mut s = viewing("working", json!(null));
        s.input(Input::ReadAloud(true));
        deliver(&mut s, vec![text_at(50, 0, "live")], vec![]);
        let out = s.input(Input::LoadEarlier);
        let page = requests(&out, "session.history")[0].clone();
        let out = s.incoming(response(
            page.id,
            json!({"sections": [text_at(10, 0, "old")], "start": false}),
        ));
        assert!(requests(&out, "speech.summarize").is_empty());
        let out = deliver(&mut s, vec![text_at(51, 0, "newer")], vec![]);
        let asked = requests(&out, "speech.summarize");
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].params["text"], "newer");
    }

    /// The view's child and children, as last given.
    fn children_of(s: &State) -> (Option<String>, Vec<String>, String) {
        match s.agent_view() {
            Some(Event::AgentView {
                child,
                children,
                status,
                ..
            }) => (child, children.into_iter().map(|c| c.id).collect(), status),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_subagent_opens_in_the_view_is_read_and_returns_to_the_parent() {
        // The subagents plan, step 4.
        let mut s = online();
        s.input(Input::Tick(secs(100.0)));
        // Nothing without a view.
        assert!(sent(&s.input(Input::OpenChild("a1".into()))).is_empty());
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let list = requests(&out, "agents.list")[0].clone();
        let mut agent = agent_info("w2:pZ", "working");
        agent["subagents_running"] = json!(1);
        agent["children"] = json!([
            {"id": "b2", "short": "session readers", "description": "Find the session readers docs",
             "kind": "Explore", "started": 1_767_693_900u64,
             "turn": {"state": "working", "line": "Read"}},
            {"id": "a1", "short": "overview screen", "description": "Build the overview screen plan",
             "kind": "general-purpose", "background": true, "finished": true,
             "started": 1_767_693_601u64, "started_by": "toolu_f6_build",
             "turn": {"state": "asking", "question": "Shall I commit it?"}}
        ]);
        s.incoming(response(list.id, json!({"agents": [agent.clone()]})));
        session_answer(&mut s, &out, "working", asking());
        // The main session, with the pane's children.
        assert_eq!(
            children_of(&s),
            (
                None,
                vec!["b2".to_string(), "a1".to_string()],
                "working".to_string()
            )
        );
        // The main session's subscribe named no child.
        assert!(
            requests(&out, "session.subscribe")[0]
                .params
                .get("child")
                .is_none()
        );

        // Open a1: the parent's subscription ends, and a1's session is
        // subscribed to by pane and child.
        let out = s.input(Input::OpenChild("a1".into()));
        let ended = requests(&out, "session.unsubscribe");
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].params["subscription_id"], 1);
        let sub = requests(&out, "session.subscribe")[0].clone();
        assert_eq!(sub.params["pane_id"], "w2:pZ");
        assert_eq!(sub.params["child"], "a1");
        // Until it answers: the subagent's own status, and nothing of the
        // parent's.
        assert_eq!(children_of(&s).0.as_deref(), Some("a1"));
        assert_eq!(children_of(&s).2, "done");
        assert!(summaries(&s).is_empty());
        let child_session = json!({
            "session_id": "a1", "matched": "id",
            "entries": [
                {"ts": "10", "role": "user", "kind": "text", "summary": "Build the overview screen."},
                {"ts": "11", "role": "assistant", "kind": "text",
                 "summary": "Built it.\n\nShall I commit it?"}
            ],
            "pending_question": {"text": "Shall I commit it?", "ts": "11"}
        });
        let mut delivery = delivery_of("done", &child_session);
        delivery["subscription_id"] = json!(2);
        delivery["session"]["session_id"] = json!("a1");
        s.incoming(response(
            sub.id.clone(),
            json!({"subscription_id": 2, "delivery": delivery}),
        ));
        assert_eq!(
            summaries(&s),
            [
                "Build the overview screen.",
                "Built it.\n\nShall I commit it?"
            ]
        );
        assert_eq!(children_of(&s).2, "done");
        // A late delivery of the parent's subscription is not this view's.
        let out = deliver(&mut s, vec![text_at(60, 0, "the parent's")], vec![]);
        assert!(view_of(&out).is_none(), "{out:?}");
        assert!(!summaries(&s).contains(&"the parent's".to_string()));
        // Its replies are read aloud from its own transcript.
        s.input(Input::ReadAloud(true));
        let live = json!({"serial": serial(70, 0), "kind": "text", "entries": [
            {"ts": "12", "role": "assistant", "kind": "text", "summary": "Committed."}
        ]});
        let out = s.incoming(note(
            "session.delivery",
            json!({"subscription_id": 2, "pane_id": "w2:pZ", "agent": "claude",
                "reset": false, "session": {"session_id": "a1", "matched": "id"},
                "reason": null, "sections": [live], "removed": []}),
        ));
        let asked = requests(&out, "speech.summarize");
        assert_eq!(asked.len(), 1, "{out:?}");
        assert_eq!(
            (&asked[0].params["pane_id"], &asked[0].params["child"]),
            (&json!("w2:pZ"), &json!("a1"))
        );
        // A subagent takes no answers: no message, and a choice fails.
        let out = s.input(Input::Respond(Suggestion::Yes));
        assert!(requests(&out, "agents.send").is_empty(), "{out:?}");
        let out = s.input(Input::Choose {
            id: "toolu_x".into(),
            answers: vec![],
        });
        assert!(requests(&out, "choice.answer").is_empty());
        assert!(
            out.iter().any(|o| matches!(
                o,
                Output::Event(Event::ChoiceAnswered { outcome, .. }) if outcome == "failed"
            )),
            "{out:?}"
        );
        // The same child again: nothing new is asked.
        assert!(requests(&s.input(Input::OpenChild("a1".into())), "session.subscribe").is_empty());

        // Back: a1's subscription ends, and the main session shows again at
        // once from the cache, resubscribed from its newest serial.
        let out = s.input(Input::CloseChild);
        let ended = requests(&out, "session.unsubscribe");
        assert_eq!(ended[0].params["subscription_id"], 2);
        let sub = requests(&out, "session.subscribe")[0].clone();
        assert!(sub.params.get("child").is_none(), "{:?}", sub.params);
        assert_eq!(children_of(&s).0, None);
        assert_eq!(summaries(&s), ["go", "Done.\n\nShall I go on?"]);
        // Back again with no child shown: nothing.
        assert!(sent(&s.input(Input::CloseChild)).is_empty());
        // The child's view is cached too: it reopens at once.
        s.incoming(response(
            sub.id.clone(),
            json!({"subscription_id": 3, "delivery": delivery_of("working", &asking())}),
        ));
        s.input(Input::OpenChild("a1".into()));
        assert_eq!(summaries(&s)[0], "Build the overview screen.");

        // A list that changes the children gives the view again. (Each
        // switch asks for one; in a view of an agent with a subagents
        // reader, the list is polled.)
        let out = s.input(Input::CloseChild);
        let list = requests(&out, "agents.list")[0].clone();
        s.incoming(response(list.id, json!({"agents": [agent.clone()]})));
        let out = s.input(Input::Tick(secs(200.0)));
        let list = requests(&out, "agents.list")[0].clone();
        agent["children"] = json!([]);
        let out = s.incoming(response(list.id, json!({"agents": [agent]})));
        match out.iter().find_map(|o| match o {
            Output::Event(Event::AgentView { children, .. }) => Some(children.clone()),
            _ => None,
        }) {
            Some(children) => assert!(children.is_empty()),
            None => panic!("no view given: {out:?}"),
        }
    }

    fn focus_on(pane: &str) -> Value {
        json!({"pane_id": pane, "agent": "Claude", "name": format!("Claude in {pane}"),
               "spoken": "Claude"})
    }

    #[test]
    fn follow_off_leaves_the_subscription_where_it_is() {
        let mut s = viewing("idle", asking());
        let out = s.incoming(note("focus.changed", json!({"focus": focus_on("w9:p1")})));
        assert!(requests(&out, "session.subscribe").is_empty());
        assert!(requests(&out, "session.unsubscribe").is_empty());
        match s.agent_view() {
            Some(Event::AgentView { terminal_on, .. }) => {
                assert_eq!(terminal_on.as_deref(), Some("Claude in w9:p1"))
            }
            other => panic!("{other:?}"),
        }
        // Back on the view's pane: no line.
        s.incoming(note("focus.changed", json!({"focus": focus_on("w2:pZ")})));
        assert!(matches!(
            s.agent_view(),
            Some(Event::AgentView {
                terminal_on: None,
                ..
            })
        ));
    }

    #[test]
    fn follow_on_moves_the_subscription_to_the_focus() {
        let mut s = viewing("idle", asking());
        s.input(Input::Follow(true));
        let out = s.incoming(note("focus.changed", json!({"focus": focus_on("w9:p1")})));
        assert_eq!(
            requests(&out, "session.unsubscribe")[0].params,
            json!({"subscription_id": 1})
        );
        let subs = requests(&out, "session.subscribe");
        assert_eq!(
            subs[0].params,
            json!({"pane_id": "w9:p1", "sections": 200, "outline": true})
        );
        // A fresh view of the new pane.
        assert!(summaries(&s).is_empty());
        // A pane with no agent is not followed.
        let out = s.incoming(note(
            "focus.changed",
            json!({"focus": {"pane_id": "w9:p2", "name": "a shell"}}),
        ));
        assert!(requests(&out, "session.subscribe").is_empty());
    }

    #[test]
    fn a_stopped_turn_asks_the_list_while_herdr_says_working() {
        // Plan 20261023_04, A4: the view's session ends on a reply while
        // herdr says working; the list's turn says whether it stopped.
        let ended = json!({
            "session_id": "s1", "matched": "id",
            "entries": [
                {"ts": "1", "role": "user", "kind": "text", "summary": "go"},
                {"ts": "2", "role": "assistant", "kind": "text", "summary": "All 12 tests pass."}
            ]
        });
        let mut s = viewing("working", ended.clone());
        let out = s.input(Input::Tick(secs(105.0)));
        let out = session_answer(&mut s, &out, "working", ended.clone());
        assert_eq!(requests(&out, "agents.list").len(), 1);
        let mut stopped = agent_info("w2:pZ", "working");
        stopped["turn"] = json!({"state": "stopped", "line": "All 12 tests pass."});
        answer_list(&mut s, &out, json!([stopped]));
        // Known stopped: the next deliveries ask no more lists.
        s.input(Input::Tick(secs(110.0)));
        let out = session_answer(&mut s, &[], "working", ended);
        assert!(requests(&out, "agents.list").is_empty());
    }

    #[test]
    fn take_a_message_from_the_view_is_for_that_agent() {
        let mut s = viewing("idle", asking());
        // herdr's focus is elsewhere.
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        let out = s.input(Input::StartDictation);
        let d = dictation_of(&out).expect("the card");
        let target = d.target.expect("a target");
        assert_eq!(target.pane_id, "w2:pZ");
        assert_eq!(target.agent.as_deref(), Some("Claude"));
        assert_eq!(target.name, "the flankr agent");
        // The list asked for at the start does not move it to the focus.
        let out = list_answer(&mut s, &out, claude());
        if let Some(d) = dictation_of(&out) {
            assert_eq!(d.target.unwrap().pane_id, "w2:pZ");
        }
        assert_eq!(s.dictation().target.unwrap().pane_id, "w2:pZ");
        s.input(Input::Utterance("please go on".into()));
        let out = s.input(Input::Submit);
        let send = message_send(&out).expect("agents.send");
        assert_eq!(send.params["pane_id"], "w2:pZ");
        // The Submit button is the approval.
        assert_eq!(send.params["confirmed"], true);
    }

    #[test]
    fn a_suggestion_is_sent_to_the_views_pane() {
        let mut s = viewing("idle", asking());
        let out = s.input(Input::Respond(Suggestion::Continue));
        let sends = requests(&out, "agents.send");
        assert_eq!(sends.len(), 1, "{out:?}");
        // A command: no kind, so the daemon reads it as one; confirmed, since
        // the tap is the approval (plan 20261023_02, the 13:25 EDT amendment).
        assert_eq!(
            sends[0].params,
            json!({"pane_id": "w2:pZ", "text": "continue",
                   "handle": "reply-claude-flankr-cont", "confirmed": true})
        );
        let out = s.input(Input::Respond(Suggestion::Yes));
        assert_eq!(
            requests(&out, "agents.send")[0].params,
            json!({"pane_id": "w2:pZ", "text": "Yes, go on", "kind": "message",
                   "handle": "reply-claude-flankr-yes", "confirmed": true})
        );
        // An older daemon asks anyway: its question is answered, not asked.
        let id = requests(&out, "agents.send")[0].id.clone();
        s.incoming(response(id, json!({"status": "pending", "confirm_id": 9})));
        let out = s.incoming(note(
            "confirm.requested",
            json!({"confirm_id": 9, "pane_id": "w2:pZ", "agent": "claude",
                   "command": "Yes, go on", "requested_by": "companion:test",
                   "text": "Approve sending your message to Claude?"}),
        ));
        assert_eq!(
            approvals(&out),
            vec![json!({"confirm_id": 9, "approve": true})]
        );
        assert!(
            !events(&out)
                .iter()
                .any(|e| matches!(e, Event::Confirm { .. }))
        );
        // Question first works too.
        let out = s.input(Input::Respond(Suggestion::Continue));
        let id = requests(&out, "agents.send")[0].id.clone();
        let out = s.incoming(note(
            "confirm.requested",
            json!({"confirm_id": 10, "pane_id": "w2:pZ", "agent": "claude",
                   "command": "continue", "requested_by": "companion:test",
                   "text": "Approve continue?"}),
        ));
        assert_eq!(
            approvals(&out),
            vec![json!({"confirm_id": 10, "approve": true})]
        );
        let out = s.incoming(response(id, json!({"status": "pending", "confirm_id": 10})));
        assert!(approvals(&out).is_empty(), "approved once");
    }

    #[test]
    fn a_refused_suggestion_is_said() {
        let mut s = viewing("idle", asking());
        let out = s.input(Input::Respond(Suggestion::Compact));
        let id = requests(&out, "agents.send")[0].id.clone();
        let refusal = Response::err(
            Some(id),
            libflankr_proto::ErrorObject::new(-32003, "not allowed"),
        );
        let out = s.incoming(Incoming::Response(refusal));
        assert!(
            events(&out)
                .iter()
                .any(|e| matches!(e, Event::Say(t) if t.starts_with("I can't send cmpct")))
        );
    }

    #[test]
    fn no_view_no_suggestion() {
        let mut s = online();
        assert!(s.input(Input::Respond(Suggestion::Yes)).is_empty());
    }

    #[test]
    fn a_submit_waits_for_speech_still_in_stt() {
        let mut s = dictating();
        s.input(Input::Tick(secs(20.0)));
        s.input(Input::Voice(true));
        s.input(Input::Tick(secs(21.0)));
        s.input(Input::Voice(false));
        // Submitted before STT gives the sentence back: no "empty".
        let out = s.input(Input::Submit);
        assert!(message_send(&out).is_none(), "{out:?}");
        assert!(!events(&out).iter().any(|e| matches!(e, Event::Say(_))));
        let out = s.input(Input::Utterance("yes, go ahead".into()));
        let send = message_send(&out).expect("sent once heard");
        assert_eq!(send.params["text"], "yes, go ahead");
    }

    #[test]
    fn a_waiting_submit_with_nothing_heard_is_refused_after_the_wait() {
        let mut s = dictating();
        s.input(Input::Tick(secs(20.0)));
        s.input(Input::Voice(true));
        s.input(Input::Voice(false));
        assert!(message_send(&s.input(Input::Submit)).is_none());
        assert!(events(&s.input(Input::Tick(secs(22.0)))).is_empty());
        let out = s.input(Input::Tick(secs(23.5)));
        assert!(message_send(&out).is_none());
        assert!(
            events(&out)
                .iter()
                .any(|e| matches!(e, Event::Say(t) if t == "The message is empty."))
        );
    }

    #[test]
    fn replies_are_read_aloud_from_when_play_is_on() {
        let mut s = viewing("working", asking());
        s.input(Input::ReadAloud(true));
        // A later delivery with a tool call, its output, and a new reply.
        let out = s.input(Input::Tick(secs(200.0)));
        let session = json!({
            "session_id": "s1", "matched": "id",
            "entries": [
                {"ts": "1", "role": "user", "kind": "text", "summary": "go"},
                {"ts": "2", "role": "assistant", "kind": "text", "summary": "Done.\n\nShall I go on?"},
                {"ts": "3", "role": "assistant", "kind": "tool_use", "tool": "Bash", "summary": "cargo test"},
                {"ts": "4", "role": "tool", "kind": "tool_result", "summary": "test result: ok"},
                {"ts": "5", "role": "assistant", "kind": "text",
                 "summary": "## Result\n\nThe **tests** pass:\n```\nok\n```\n- `fmt` is clean"}
            ]
        });
        let out = session_answer(&mut s, &out, "idle", session);
        // The daemon is asked how to say the new reply, and only that one.
        let asked = requests(&out, "speech.summarize");
        assert_eq!(asked.len(), 1, "{out:?}");
        assert_eq!(
            asked[0].params,
            json!({"pane_id": "w2:pZ", "ts": "5",
                   "text": "## Result\n\nThe **tests** pass:\n```\nok\n```\n- `fmt` is clean"})
        );
        assert!(said(&out).is_empty());
        // No `spoken` (an older daemon): read through the rules here.
        let out = s.incoming(response(asked[0].id.clone(), json!({"complex": false})));
        assert_eq!(said(&out), ["Result The tests pass: fmt is clean"]);
    }

    /// The rules as the client reads a reply itself, at noon EDT.
    fn here(text: &str) -> String {
        flankr_speech::read_aloud(text, NOON_EDT)
    }

    const NOON_EDT: LocalTime = LocalTime {
        hour: 12,
        minute: 0,
        second: 0,
        utc_offset_minutes: -240,
    };

    fn options(out: &[Output]) -> Vec<Value> {
        requests(out, "client.options")
            .into_iter()
            .map(|r| r.params.clone())
            .collect()
    }

    #[test]
    fn read_aloud_is_sent_as_an_option_on_toggling_and_after_hello() {
        // Off at hello: said as off.
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(
            id,
            json!({"server": "flankr", "session_id": "companion:test", "vocabulary": []}),
        ));
        assert_eq!(
            options(&out),
            [
                json!({"spoken_output": false, "background": false, "general_agent": true, "tab_follows_focus": false})
            ]
        );
        // Toggled: sent each time.
        let out = s.input(Input::ReadAloud(true));
        assert_eq!(
            options(&out),
            [
                json!({"spoken_output": true, "background": false, "general_agent": true, "tab_follows_focus": false})
            ]
        );
        let id = requests(&out, "client.options")[0].id.clone();
        assert!(s.incoming(response(id, json!({}))).is_empty());
        let out = s.input(Input::ReadAloud(false));
        assert_eq!(
            options(&out),
            [
                json!({"spoken_output": false, "background": false, "general_agent": true, "tab_follows_focus": false})
            ]
        );
        // Offline, nothing is sent; after the reconnect's hello, the value
        // as it is now.
        s.disconnected(1000);
        assert!(options(&s.input(Input::ReadAloud(true))).is_empty());
        let out = s.connected();
        assert!(options(&out).is_empty(), "not before hello");
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(
            id,
            json!({"server": "flankr", "session_id": "companion:test", "vocabulary": []}),
        ));
        assert_eq!(
            options(&out),
            [
                json!({"spoken_output": true, "background": false, "general_agent": true, "tab_follows_focus": false})
            ]
        );
        // A refused option is only logged.
        let id = requests(&out, "client.options")[0].id.clone();
        let refused = Response::err(
            Some(id),
            libflankr_proto::ErrorObject::new(-32601, "no such method"),
        );
        assert!(s.incoming(Incoming::Response(refused)).is_empty());
    }

    #[test]
    fn a_reply_is_read_as_the_daemon_says() {
        let (mut s, asked, _) = reading(&[PATHS]);
        let out = s.incoming(response(
            asked[0].id.clone(),
            json!({"read": "I changed three files: the speech the mod the common", "complex": true}),
        ));
        assert_eq!(
            said(&out),
            ["I changed three files: the speech the mod the common"]
        );
    }

    #[test]
    fn a_reply_read_here_reads_times_by_the_phones_clock() {
        let (mut s, asked, _) = reading(&["Pushed at 11:57:00."]);
        s.set_local_time(NOON_EDT);
        let out = s.incoming(response(asked[0].id.clone(), json!({"complex": false})));
        assert_eq!(said(&out), ["Pushed three minutes ago."]);
    }

    fn said(out: &[Output]) -> Vec<String> {
        events(out)
            .into_iter()
            .filter_map(|e| match e {
                Event::Say(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// Replies as agents gave them (plan 20261023_05, step 4).
    const PLAIN: &str = "I ran the tests and they all pass. Shall I go on?";
    const BACKQUOTES: &str = "Done: `Sessions::reply_text` now joins the parts.";
    const PATHS: &str = "I changed three files:\n- src/server/speech.rs\n- src/sessions/mod.rs\n- crates/libflankr_proto/src/common.rs";

    /// Viewing w2:pZ with play on, then a peek at 200 s with `replies`
    /// new after the first: the `speech.summarize` requests it sent.
    fn reading(replies: &[&str]) -> (State, Vec<Request>, Vec<Output>) {
        let mut s = viewing("working", asking());
        s.input(Input::ReadAloud(true));
        let out = s.input(Input::Tick(secs(200.0)));
        let mut entries = asking()["entries"].as_array().unwrap().clone();
        for (i, text) in replies.iter().enumerate() {
            entries.push(json!({"ts": (10 + i).to_string(), "role": "assistant",
                                "kind": "text", "summary": text}));
        }
        let session = json!({"session_id": "s1", "matched": "id", "entries": entries});
        let out = session_answer(&mut s, &out, "idle", session);
        let asked = requests(&out, "speech.summarize")
            .into_iter()
            .cloned()
            .collect();
        (s, asked, out)
    }

    #[test]
    fn every_reply_is_sent_to_be_measured_and_a_plain_one_read_as_it_is() {
        let (mut s, asked, out) = reading(&[PLAIN]);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].params["text"], PLAIN);
        assert!(said(&out).is_empty());
        let out = s.incoming(response(
            asked[0].id.clone(),
            json!({"read": PLAIN, "complex": false}),
        ));
        assert_eq!(said(&out), [PLAIN]);
    }

    #[test]
    fn complex_replies_are_read_as_their_summaries_in_order() {
        let (mut s, asked, _) = reading(&[BACKQUOTES, PATHS]);
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[0].params["ts"], "10");
        assert_eq!(asked[1].params["ts"], "11");
        // The second answers first: it waits for the first.
        let out = s.incoming(response(
            asked[1].id.clone(),
            json!({"read": "I changed three files.", "summary": "Three files changed.", "complex": true}),
        ));
        assert!(said(&out).is_empty(), "{out:?}");
        let out = s.incoming(response(
            asked[0].id.clone(),
            json!({"read": "Done: reply text joins.", "summary": "Reply text now joins the parts.", "complex": true}),
        ));
        assert_eq!(
            said(&out),
            ["Reply text now joins the parts.", "Three files changed."]
        );
    }

    #[test]
    fn a_refused_summary_is_read_through_the_rules_here() {
        let (mut s, asked, _) = reading(&[BACKQUOTES]);
        let refused = Incoming::Response(Response::err(
            Some(asked[0].id.clone()),
            libflankr_proto::ErrorObject::new(-32015, "no summary: no key"),
        ));
        let out = s.incoming(refused);
        assert_eq!(said(&out), [here(BACKQUOTES)]);
        assert!(!events(&out).iter().any(|e| matches!(e, Event::Error(_))));
    }

    #[test]
    fn a_summary_that_does_not_come_holds_the_next_reply_until_the_wait() {
        let (mut s, asked, _) = reading(&[PATHS, PLAIN]);
        // The second is ready; the first holds it.
        let out = s.incoming(response(
            asked[1].id.clone(),
            json!({"read": PLAIN, "complex": false}),
        ));
        assert!(said(&out).is_empty());
        assert!(said(&s.input(Input::Tick(secs(209.0)))).is_empty());
        // At the wait, the first is read as it is, then the second.
        let out = s.input(Input::Tick(secs(210.0)));
        assert_eq!(said(&out), [here(PATHS), PLAIN.to_string()]);
        // Its summary arriving late is not read again.
        let out = s.incoming(response(
            asked[0].id.clone(),
            json!({"read": "I changed three files.", "summary": "Three files changed.", "complex": true}),
        ));
        assert!(said(&out).is_empty());
    }

    #[test]
    fn a_closed_connection_reads_what_waits_as_it_is() {
        let (mut s, _, _) = reading(&[BACKQUOTES]);
        let out = s.disconnected(1000);
        assert_eq!(said(&out), [here(BACKQUOTES)]);
    }

    #[test]
    fn nothing_is_read_with_play_off() {
        let mut s = viewing("working", asking());
        let out = s.input(Input::Tick(secs(200.0)));
        let out = session_answer(&mut s, &out, "idle", asking());
        assert!(!events(&out).iter().any(|e| matches!(e, Event::Say(_))));
    }

    #[test]
    fn skip_stops_what_is_said_and_the_next_follows() {
        let mut s = online();
        assert!(s.input(Input::Skip).is_empty(), "nothing to skip");
        s.input(Input::Speaking(true));
        s.incoming(notice("one"));
        assert_eq!(events(&s.input(Input::Skip)), vec![Event::StopSpeaking]);
        // Stopped: the held reply is said next.
        assert_eq!(events(&s.input(Input::Speaking(false))), vec![say("one")]);
    }

    #[test]
    fn stop_reading_stops_what_is_said_and_drops_what_waits() {
        let mut s = online();
        assert!(s.input(Input::StopReading).is_empty(), "nothing to stop");
        s.input(Input::Speaking(true));
        s.incoming(notice("one"));
        s.incoming(notice("two"));
        assert_eq!(s.held_notices(), 2);
        assert_eq!(
            events(&s.input(Input::StopReading)),
            vec![Event::StopSpeaking]
        );
        assert_eq!(s.held_notices(), 0);
        // Stopped: nothing follows.
        assert!(said(&s.input(Input::Speaking(false))).is_empty());
        // What comes after is said as usual.
        assert_eq!(events(&s.incoming(notice("three"))), vec![say("three")]);
    }

    #[test]
    fn stop_reading_drops_the_replies_waiting_to_be_read() {
        let (mut s, asked, _) = reading(&[BACKQUOTES, PATHS]);
        assert!(
            events(&s.input(Input::StopReading)).is_empty(),
            "not speaking"
        );
        // The summary comes, the wait runs out, or the connection closes:
        // nothing is read.
        let out = s.incoming(response(
            asked[0].id.clone(),
            json!({"read": "Done: reply text joins.", "complex": true}),
        ));
        assert!(said(&out).is_empty(), "{out:?}");
        assert!(said(&s.input(Input::Tick(secs(400.0)))).is_empty());
        assert!(said(&s.disconnected(1000)).is_empty());
    }

    #[test]
    fn the_view_is_loading_until_the_session_answers() {
        let mut s = online();
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let loading = |out: &[Output]| {
            out.iter().rev().find_map(|o| match o {
                Output::Event(Event::AgentView { loading, .. }) => Some(*loading),
                _ => None,
            })
        };
        assert_eq!(loading(&out), Some(true));
        // No session at all: answered, so no longer loading.
        let out = session_answer(&mut s, &out, "idle", Value::Null);
        assert_eq!(loading(&out), Some(false));
    }

    #[test]
    fn a_tapped_yes_or_no_carries_the_question() {
        assert_eq!(
            answer_text(true, "Install it on the S23?"),
            "Yes, install it on the S23"
        );
        assert_eq!(
            answer_text(false, "Install it on the S23?"),
            "No, don't install it on the S23"
        );
        assert_eq!(
            answer_text(
                true,
                "Should I write the L3 amendment for today's changes now?"
            ),
            "Yes, write the L3 amendment for today's changes now"
        );
        assert_eq!(
            answer_text(true, "Is that what you meant?"),
            "Yes, re: is that what you meant"
        );
        // The last question of the paragraph, in the filter's characters.
        assert_eq!(
            answer_text(
                true,
                "Built (`a1d3d2a`). Restart the daemon and install the new build?"
            ),
            "Yes, restart the daemon and install the new build"
        );
        assert_eq!(answer_text(true, ""), "Yes");
    }

    /// A plan question about the overview screen plan, with its link.
    fn asking_about_a_plan(plan: Option<&str>) -> Value {
        let mut q = json!({
            "text": "Shall I build docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md?",
            "ts": "2", "about_plan": true,
            "links": [{"text": "docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md",
                       "url": "http://w/#/src--flankr/docs/design/20261023_04_OVERVIEW_SCREEN_PLAN"}]
        });
        if let Some(p) = plan {
            q["plan"] = json!(p);
        }
        json!({
            "session_id": "s1", "matched": "id",
            "entries": [
                {"ts": "1", "role": "user", "kind": "text", "summary": "go"},
                {"ts": "2", "role": "assistant", "kind": "text",
                 "summary": "Done.\n\nShall I build docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md?",
                 "links": q["links"].clone()}
            ],
            "pending_question": q
        })
    }

    #[test]
    fn plan_answers_name_the_plan() {
        let plan = Some("the overview screen plan");
        assert_eq!(
            plan_answer(Suggestion::Approve, plan).unwrap(),
            "Approve the overview screen plan"
        );
        assert_eq!(
            plan_answer(Suggestion::Implement, plan).unwrap(),
            "Approve the overview screen plan and start building it"
        );
        assert_eq!(
            plan_answer(Suggestion::Discuss, plan).unwrap(),
            "Discuss the overview screen plan:"
        );
        assert_eq!(
            plan_answer(Suggestion::Approve, None).unwrap(),
            "Approve the plan"
        );
        // A plan by the id the question gave it.
        assert_eq!(
            plan_answer(Suggestion::Approve, Some("plan 03")).unwrap(),
            "Approve plan 03"
        );
        assert_eq!(
            plan_answer(Suggestion::Implement, Some("plan 20261027_04")).unwrap(),
            "Approve plan 20261027_04 and start building it"
        );
        assert_eq!(
            plan_answer(Suggestion::Discuss, Some("plan 5")).unwrap(),
            "Discuss plan 5:"
        );
        assert_eq!(
            plan_answer(Suggestion::Implement, Some("harness `protocol` plan")).unwrap(),
            "Approve the harness protocol plan and start building it"
        );
        assert_eq!(plan_answer(Suggestion::Yes, plan), None);
    }

    #[test]
    fn a_plan_question_card_carries_about_plan_and_its_links() {
        let s = viewing(
            "idle",
            asking_about_a_plan(Some("the overview screen plan")),
        );
        let Some(Event::AgentView {
            sections, pinned, ..
        }) = s.agent_view()
        else {
            panic!("a view");
        };
        let (entries, cards, _) = crate::view::flatten_view(&sections, pinned);
        let card = cards
            .iter()
            .find(|c| c.kind == crate::view::CardKind::Question)
            .unwrap();
        assert!(card.about_plan);
        assert_eq!(card.links.len(), 1);
        assert_eq!(entries[1].links.len(), 1, "entries keep their links");
        // A plain question is not about a plan.
        let s = viewing("idle", asking());
        let Some(Event::AgentView {
            sections, pinned, ..
        }) = s.agent_view()
        else {
            panic!("a view");
        };
        let (_, cards, _) = crate::view::flatten_view(&sections, pinned);
        assert!(cards.iter().all(|c| !c.about_plan && c.links.is_empty()));
    }

    #[test]
    fn approve_and_implement_send_the_plan_answer_and_dismiss_the_card() {
        for (tapped, said) in [
            (Suggestion::Approve, "Approve the overview screen plan"),
            (
                Suggestion::Implement,
                "Approve the overview screen plan and start building it",
            ),
        ] {
            let mut s = viewing(
                "idle",
                asking_about_a_plan(Some("the overview screen plan")),
            );
            let out = s.input(Input::Respond(tapped));
            let send = requests(&out, "agents.send")[0].clone();
            assert_eq!(send.params["text"], said);
            assert_eq!(send.params["kind"], "message");
            let (_, cards, _) = view_of(&out).expect("the view again");
            assert!(
                !cards
                    .iter()
                    .any(|c| c.live && c.kind == crate::view::CardKind::Question),
                "{out:?}"
            );
            // The tap is its own approval.
            s.incoming(response(
                send.id.clone(),
                json!({"status": "pending", "confirm_id": 41}),
            ));
            let out = s.incoming(note(
                "confirm.requested",
                json!({"confirm_id": 41, "pane_id": "w2:pZ", "agent": "claude",
                       "command": said, "requested_by": "companion:test",
                       "text": "Approve sending your message to Claude?"}),
            ));
            assert_eq!(
                approvals(&out),
                vec![json!({"confirm_id": 41, "approve": true})],
                "own approval"
            );
        }
        // No plan name: "the plan".
        let mut s = viewing("idle", asking_about_a_plan(None));
        let out = s.input(Input::Respond(Suggestion::Approve));
        assert_eq!(
            requests(&out, "agents.send")[0].params["text"],
            "Approve the plan"
        );
    }

    #[test]
    fn discuss_takes_a_message_starting_with_the_plan() {
        let mut s = viewing(
            "idle",
            asking_about_a_plan(Some("the overview screen plan")),
        );
        let out = s.input(Input::Respond(Suggestion::Discuss));
        assert!(requests(&out, "agents.send").is_empty(), "nothing sent yet");
        let d = dictation_of(&out).expect("taking a message");
        assert!(d.active);
        assert_eq!(d.text, "Discuss the overview screen plan:");
        assert_eq!(d.target.as_ref().unwrap().pane_id, "w2:pZ");
        // The rest is dictated after it.
        let out = s.input(Input::Utterance("split step two".into()));
        let d = dictation_of(&out).expect("the message");
        assert_eq!(d.text, "Discuss the overview screen plan: split step two");
    }

    /// A plan question whose daemon sent its answers (plan 20261027_06).
    fn asking_with_answers() -> Value {
        let mut session = asking_about_a_plan(Some("plan 04"));
        session["pending_question"]["answers"] = json!([
            {"label": "approve", "style": "green", "sends": "Approve plan 04", "answers": true},
            {"label": "discuss", "style": "link", "starts": "Discuss plan 04:"},
            {"label": "cont", "style": "yellow", "sends": "continue", "command": true},
            {"label": "Yes", "style": "approve", "sends": "yes", "answers": true}
        ]);
        session
    }

    fn answer_of(session: &Value, label: &str) -> common::Answer {
        serde_json::from_value(
            session["pending_question"]["answers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["label"] == label)
                .unwrap()
                .clone(),
        )
        .unwrap()
    }

    #[test]
    fn the_live_question_card_carries_the_answers_in_order() {
        let s = viewing("idle", asking_with_answers());
        let Some(Event::AgentView {
            sections, pinned, ..
        }) = s.agent_view()
        else {
            panic!("a view");
        };
        let (_, cards, _) = crate::view::flatten_view(&sections, pinned);
        let card = cards
            .iter()
            .find(|c| c.kind == crate::view::CardKind::Question && c.live)
            .unwrap();
        let drawn: Vec<(&str, &str)> = card
            .answers
            .iter()
            .map(|a| (a.label.as_str(), a.style.as_str()))
            .collect();
        assert_eq!(
            drawn,
            [
                ("approve", "green"),
                ("discuss", "link"),
                ("cont", "yellow"),
                ("Yes", "approve")
            ]
        );
        // Without answers (an older daemon): none, and the app keeps its own.
        let s = viewing("idle", asking_about_a_plan(Some("plan 04")));
        let Some(Event::AgentView {
            sections, pinned, ..
        }) = s.agent_view()
        else {
            panic!("a view");
        };
        let (_, cards, _) = crate::view::flatten_view(&sections, pinned);
        assert!(cards.iter().all(|c| c.answers.is_empty()));
    }

    /// A choice question (plan 20261029_01) as the daemon gives it: the
    /// `AskUserQuestion` call at line 2, its live Question card at slot 128
    /// with the first question's text, and the pending question with its
    /// choices. `card_text` stands for the card's text, to test the match.
    fn choosing(card_text: &str) -> Value {
        let choices = json!({
            "id": "toolu_01abc", "source": "claude_code/AskUserQuestion",
            "questions": [
                {"header": "Colour", "text": "Which colour?", "other": true,
                 "options": [{"label": "Red", "detail": "warm"}, {"label": "Blue"}]},
                {"header": "Extras", "text": "Which extras?", "multi": true, "other": true,
                 "options": [{"label": "Nuts"}, {"label": "Cream", "preview": "a\nb"}]}
            ]
        });
        json!({
            "session_id": "s1", "matched": "id",
            "sections": [
                {"serial": serial(1, 0), "kind": "turn", "entries": [
                    {"ts": "1", "role": "user", "kind": "text", "summary": "go"}]},
                {"serial": serial(2, 0), "kind": "tool", "entries": [
                    {"ts": "2", "role": "assistant", "kind": "tool_use",
                     "summary": "AskUserQuestion {}", "call_id": "toolu_01abc"}]},
                {"serial": serial(2, 128), "kind": "card", "card": {
                    "card": "question", "live": true, "title": "Claude is asking",
                    "text": card_text}}
            ],
            "pending_question": {"text": "Which colour?", "ts": "2", "choices": choices}
        })
    }

    /// Online with the view of w2:pZ open on a choice question whose card
    /// reads `card_text`.
    fn viewing_choices(card_text: &str) -> State {
        let mut s = online();
        s.input(Input::Tick(secs(100.0)));
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let list = requests(&out, "agents.list")[0].clone();
        s.incoming(response(
            list.id,
            json!({"agents": [agent_info("w2:pZ", "blocked")]}),
        ));
        let session = choosing(card_text);
        let delivery = json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
            "reset": true, "session": {"session_id": "s1", "matched": "id"}, "reason": null,
            "sections": session["sections"], "status": "blocked",
            "pending_question": session["pending_question"]});
        let sub = requests(&out, "session.subscribe")[0].id.clone();
        s.incoming(response(
            sub,
            json!({"subscription_id": 1, "delivery": delivery}),
        ));
        s
    }

    fn live_question(s: &State) -> Option<crate::view::AgentCard> {
        let Some(Event::AgentView {
            sections, pinned, ..
        }) = s.agent_view()
        else {
            panic!("a view");
        };
        let (_, cards, _) = crate::view::flatten_view(&sections, pinned);
        cards
            .into_iter()
            .find(|c| c.kind == crate::view::CardKind::Question && c.live)
    }

    fn choice_answered(out: &[Output]) -> Vec<(String, String, Vec<String>, Option<i64>)> {
        events(out)
            .into_iter()
            .filter_map(|e| match e {
                Event::ChoiceAnswered {
                    id,
                    pane_id,
                    outcome,
                    got,
                    code,
                } => {
                    assert!(pane_id == "w2:pZ" || pane_id.is_empty(), "{pane_id}");
                    Some((id, outcome, got, code))
                }
                _ => None,
            })
            .collect()
    }

    fn two_answers() -> Vec<common::ChoiceAnswer> {
        vec![
            common::ChoiceAnswer {
                picked: vec![1],
                ..Default::default()
            },
            common::ChoiceAnswer {
                picked: vec![0],
                other: Some("sprinkles".into()),
            },
        ]
    }

    #[test]
    fn the_live_question_card_carries_its_choices_while_its_text_matches() {
        let s = viewing_choices("Which colour?");
        let card = live_question(&s).expect("the live card");
        let set = card.choices.expect("its choices");
        assert_eq!(set.id, "toolu_01abc");
        assert_eq!(set.source, "claude_code/AskUserQuestion");
        assert_eq!(set.questions.len(), 2);
        assert_eq!(set.questions[0].header, "Colour");
        assert_eq!(set.questions[0].options[0].detail, "warm");
        assert!(set.questions[1].multi && set.questions[1].other);
        assert_eq!(set.questions[1].options[1].preview.as_deref(), Some("a\nb"));
        assert!(card.answers.is_empty(), "a choice question has no answers");
        // A card that is not the pending question's: no choices.
        let s = viewing_choices("Something else?");
        let card = live_question(&s).expect("the live card");
        assert!(card.choices.is_none());
        // Every other card has none.
        let s = viewing("idle", asking());
        assert!(live_question(&s).expect("a card").choices.is_none());
    }

    #[test]
    fn an_older_daemons_question_has_no_choices_and_behaves_as_today() {
        let s = viewing("idle", asking());
        let card = live_question(&s).expect("a card");
        assert!(card.choices.is_none() && card.answers.is_empty());
        // Yes still answers it with a message.
        let mut s = viewing("idle", asking());
        let out = s.input(Input::Respond(Suggestion::Yes));
        assert_eq!(requests(&out, "agents.send").len(), 1);
        assert!(requests(&out, "choice.answer").is_empty());
    }

    #[test]
    fn choose_sends_choice_answer_with_the_views_pane() {
        let mut s = viewing_choices("Which colour?");
        let out = s.input(Input::Choose {
            id: "toolu_01abc".into(),
            answers: two_answers(),
        });
        let sent = requests(&out, "choice.answer");
        assert_eq!(sent.len(), 1, "{out:?}");
        assert_eq!(
            sent[0].params,
            json!({"pane_id": "w2:pZ", "id": "toolu_01abc", "answers": [
                {"picked": [1]}, {"picked": [0], "other": "sprinkles"}]})
        );
        assert!(choice_answered(&out).is_empty(), "nothing yet");
        // A second while the first is on its way is refused here.
        let out = s.input(Input::Choose {
            id: "toolu_01abc".into(),
            answers: two_answers(),
        });
        assert!(requests(&out, "choice.answer").is_empty());
        let got = choice_answered(&out);
        assert_eq!(got[0].1, "failed");
        assert_eq!(
            got[0].3,
            Some(libflankr_proto::envelope::codes::NOT_PERMITTED)
        );
    }

    #[test]
    fn each_choice_result_gives_its_event() {
        let answer = |result: Value| {
            let mut s = viewing_choices("Which colour?");
            let out = s.input(Input::Choose {
                id: "toolu_01abc".into(),
                answers: two_answers(),
            });
            let id = requests(&out, "choice.answer")[0].id.clone();
            let out = s.incoming(response(id, result));
            (s, out)
        };
        // Answered: the event, and the card goes at once.
        let (s, out) = answer(json!({"status": "answered"}));
        assert_eq!(
            choice_answered(&out),
            [("toolu_01abc".into(), "answered".into(), vec![], None)]
        );
        assert!(live_question(&s).is_none(), "the card goes");
        // Mismatch: what the agent recorded; the card stays until the
        // session says otherwise.
        let (s, out) = answer(json!({"status": "mismatch", "got": ["Colour: Red"]}));
        assert_eq!(
            choice_answered(&out),
            [(
                "toolu_01abc".into(),
                "mismatch".into(),
                vec!["Colour: Red".to_string()],
                None
            )]
        );
        assert!(live_question(&s).is_some());
        // No result: the card stays, with its choices.
        let (s, out) = answer(json!({"status": "no_result"}));
        assert_eq!(choice_answered(&out)[0].1, "no_result");
        assert!(live_question(&s).unwrap().choices.is_some());
        // A result this client cannot read: failed, with why.
        let (_, out) = answer(json!({"status": 7}));
        let got = choice_answered(&out);
        assert_eq!(got[0].1, "failed");
        assert!(got[0].2[0].contains("choice.answer"), "{got:?}");
    }

    #[test]
    fn a_refused_choice_gives_failed_with_its_code_and_message() {
        for code in [
            libflankr_proto::envelope::codes::CHOICE_STALE,
            libflankr_proto::envelope::codes::CHOICE_UNSUPPORTED,
            libflankr_proto::envelope::codes::SCREEN_MISMATCH,
            libflankr_proto::envelope::codes::NOT_PERMITTED,
        ] {
            let mut s = viewing_choices("Which colour?");
            let out = s.input(Input::Choose {
                id: "toolu_01abc".into(),
                answers: two_answers(),
            });
            let id = requests(&out, "choice.answer")[0].id.clone();
            let out = s.incoming(Incoming::Response(Response::err(
                Some(id),
                libflankr_proto::ErrorObject::new(code, "question 2 did not appear"),
            )));
            assert_eq!(
                choice_answered(&out),
                [(
                    "toolu_01abc".into(),
                    "failed".into(),
                    vec!["question 2 did not appear".to_string()],
                    Some(code)
                )]
            );
            // The card stays live with its choices for another try.
            assert!(live_question(&s).unwrap().choices.is_some());
            // And another answer may go.
            let out = s.input(Input::Choose {
                id: "toolu_01abc".into(),
                answers: two_answers(),
            });
            assert_eq!(requests(&out, "choice.answer").len(), 1);
        }
    }

    #[test]
    fn a_choice_with_no_answer_in_time_or_a_closed_link_fails() {
        let mut s = viewing_choices("Which colour?");
        let out = s.input(Input::Choose {
            id: "toolu_01abc".into(),
            answers: two_answers(),
        });
        let id = requests(&out, "choice.answer")[0].id.clone();
        // The daemon may take 15 s: nothing yet.
        assert!(choice_answered(&s.input(Input::Tick(secs(115.0)))).is_empty());
        let out = s.input(Input::Tick(secs(120.0)));
        let got = choice_answered(&out);
        assert_eq!(got[0].1, "failed");
        assert!(got[0].2[0].contains("20 s"), "{got:?}");
        assert!(CHOICE_ANSWER_WAIT >= Duration::from_secs(15));
        // A late answer is no one's.
        assert!(
            choice_answered(&s.incoming(response(id, json!({"status": "answered"})))).is_empty()
        );
        // The link closing: failed at once.
        let mut s = viewing_choices("Which colour?");
        s.input(Input::Choose {
            id: "toolu_01abc".into(),
            answers: two_answers(),
        });
        let got = choice_answered(&s.disconnected(1000));
        assert_eq!(got[0].1, "failed");
        assert_eq!(got[0].2, ["the connection closed"]);
        // With no view open, or offline: failed, nothing sent.
        let mut s = online();
        let out = s.input(Input::Choose {
            id: "x".into(),
            answers: vec![],
        });
        assert!(requests(&out, "choice.answer").is_empty());
        assert_eq!(choice_answered(&out)[0].1, "failed");
        let mut s = viewing_choices("Which colour?");
        s.disconnected(1000);
        let out = s.input(Input::Choose {
            id: "toolu_01abc".into(),
            answers: two_answers(),
        });
        assert!(sent(&out).is_empty());
        assert_eq!(choice_answered(&out)[0].2, ["offline"]);
    }

    #[test]
    fn a_sends_answer_is_sent_confirmed_and_dismisses_the_card() {
        let session = asking_with_answers();
        let mut s = viewing("idle", session.clone());
        let out = s.input(Input::Suggested(answer_of(&session, "approve")));
        let send = requests(&out, "agents.send")[0].clone();
        assert_eq!(send.params["text"], "Approve plan 04");
        assert_eq!(send.params["kind"], "message");
        assert_eq!(send.params["confirmed"], true);
        assert_eq!(send.params["handle"], "reply-claude-flankr-approve");
        let (_, cards, _) = view_of(&out).expect("the view again");
        assert!(
            !cards
                .iter()
                .any(|c| c.live && c.kind == crate::view::CardKind::Question),
            "{out:?}"
        );
        // A command: no kind, and the card stays.
        let mut s = viewing("idle", session.clone());
        let out = s.input(Input::Suggested(answer_of(&session, "cont")));
        let send = requests(&out, "agents.send")[0].clone();
        assert_eq!(send.params["text"], "continue");
        assert!(send.params.get("kind").is_none());
        assert_eq!(send.params["confirmed"], true);
        let (_, cards, _) = view_of(&out).expect("the view again");
        assert!(
            cards
                .iter()
                .any(|c| c.live && c.kind == crate::view::CardKind::Question)
        );
        // A plain "yes" says what it answers, as the Yes button does.
        let mut s = viewing("idle", session.clone());
        let out = s.input(Input::Suggested(answer_of(&session, "Yes")));
        let text = &requests(&out, "agents.send")[0].params["text"];
        assert!(text.as_str().unwrap().starts_with("Yes, "), "{text}");
    }

    #[test]
    fn a_starts_answer_opens_the_message_card_with_its_text() {
        let session = asking_with_answers();
        let mut s = viewing("idle", session.clone());
        let out = s.input(Input::Suggested(answer_of(&session, "discuss")));
        assert!(requests(&out, "agents.send").is_empty(), "nothing sent yet");
        let d = dictation_of(&out).expect("taking a message");
        assert!(d.active);
        assert_eq!(d.text, "Discuss plan 04:");
        assert_eq!(d.target.as_ref().unwrap().pane_id, "w2:pZ");
    }

    #[test]
    fn the_tapped_yes_sends_the_answer_with_its_context() {
        let mut s = viewing("idle", asking());
        let out = s.input(Input::Respond(Suggestion::Yes));
        assert_eq!(
            requests(&out, "agents.send")[0].params["text"],
            "Yes, go on"
        );
    }

    #[test]
    fn a_tapped_yes_dismisses_its_card_at_once_and_shows_as_typed() {
        let mut s = viewing("idle", asking());
        let card_kinds = |out: &[Output]| {
            view_of(out).map(|(_, cards, _)| {
                cards
                    .iter()
                    .filter(|c| c.live)
                    .map(|c| c.kind)
                    .collect::<Vec<_>>()
            })
        };
        let typed = |out: &[Output]| {
            out.iter().rev().find_map(|o| match o {
                Output::Event(Event::AgentView { typed, .. }) => Some(typed.clone()),
                _ => None,
            })
        };
        let out = s.input(Input::Respond(Suggestion::Yes));
        // No Question card any more, before the daemon has answered.
        assert!(
            !card_kinds(&out)
                .unwrap()
                .contains(&crate::view::CardKind::Question),
            "{out:?}"
        );
        assert_eq!(typed(&out), Some(Some("Yes, go on".to_string())));
        // A second tap has no card to answer: plain yes, not the question.
        // The session shows it as the user's: the typed input goes.
        let send = requests(&out, "agents.send")[0].clone();
        s.incoming(response(send.id, json!({"status": "sent"})));
        let out = s.input(Input::Tick(secs(200.0)));
        let session = json!({
            "session_id": "s1", "matched": "id",
            "entries": [
                {"ts": "1", "role": "user", "kind": "text", "summary": "go"},
                {"ts": "2", "role": "assistant", "kind": "text", "summary": "Done.\n\nShall I go on?"},
                {"ts": "3", "role": "user", "kind": "text", "summary": "Yes, go on"}
            ]
        });
        let out = session_answer(&mut s, &out, "idle", session);
        assert_eq!(typed(&out), Some(None));
    }

    #[test]
    fn a_refused_yes_brings_its_card_back() {
        let mut s = viewing("idle", asking());
        let out = s.input(Input::Respond(Suggestion::Yes));
        let id = requests(&out, "agents.send")[0].id.clone();
        let refusal = Response::err(
            Some(id),
            libflankr_proto::ErrorObject::new(-32003, "not allowed"),
        );
        let out = s.incoming(Incoming::Response(refusal));
        let (_, cards, _) = view_of(&out).expect("the view again");
        assert!(
            cards
                .iter()
                .any(|c| c.live && c.kind == crate::view::CardKind::Question)
        );
    }

    /// Read aloud on, and a new reply asking a question, its summary
    /// answered with `spoken`.
    fn asked_aloud(speaking_first: bool) -> State {
        let mut s = viewing("working", asking());
        s.input(Input::ReadAloud(true));
        if speaking_first {
            s.input(Input::Speaking(true));
        }
        let out = s.input(Input::Tick(secs(200.0)));
        let mut entries = asking()["entries"].as_array().unwrap().clone();
        entries.push(json!({"ts": "10", "role": "assistant", "kind": "text",
                            "summary": "Built it.\n\nInstall it on the S23?"}));
        let session = json!({"session_id": "s1", "matched": "id", "entries": entries,
                             "pending_question": {"text": "Install it on the S23?", "ts": "10"}});
        let out = session_answer(&mut s, &out, "idle", session);
        let asked = requests(&out, "speech.summarize")[0].clone();
        s.incoming(response(
            asked.id,
            json!({"read": "Built it. Install it on the S23?", "complex": false}),
        ));
        s
    }

    #[test]
    fn answering_a_question_being_read_stops_its_reading() {
        let mut s = asked_aloud(false);
        s.input(Input::Speaking(true));
        let out = s.input(Input::Respond(Suggestion::Yes));
        assert!(events(&out).contains(&Event::StopSpeaking), "{out:?}");
        assert_eq!(
            requests(&out, "agents.send")[0].params["text"],
            "Yes, install it on the S23"
        );
    }

    #[test]
    fn answering_a_question_still_queued_drops_its_reading() {
        // Something else is being said, so the question waits.
        let mut s = asked_aloud(true);
        assert_eq!(s.held_notices(), 1);
        let out = s.input(Input::Respond(Suggestion::Yes));
        assert!(!events(&out).contains(&Event::StopSpeaking), "not this one");
        assert_eq!(s.held_notices(), 0);
        assert!(events(&s.input(Input::Speaking(false))).is_empty());
    }

    #[test]
    fn a_message_sent_from_the_view_shows_as_typed_until_the_session_has_it() {
        let mut s = viewing("idle", asking());
        s.input(Input::StartDictation);
        speak(&mut s, 110.0, 112.0, "please run the tests");
        let send = message_send(&s.input(Input::Submit)).unwrap();
        let target = send.params["pane_id"].clone();
        assert_eq!(target, "w2:pZ");
        s.incoming(response(send.id, json!({"status": "sent"})));
        let typed = s.agent_view().and_then(|e| match e {
            Event::AgentView { typed, .. } => typed,
            _ => None,
        });
        assert_eq!(typed.as_deref(), Some("please run the tests"));
    }

    // ── The overview's list (plan 20261023_04, step 2) ───────────────────

    /// The `Agents` events in `out`.
    fn agents_of(out: &[Output]) -> Vec<Vec<common::AgentInfo>> {
        events(out)
            .into_iter()
            .filter_map(|e| match e {
                Event::Agents(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    /// Answers the `agents.list` in `out` with `agents`.
    fn answer_list(s: &mut State, out: &[Output], agents: Value) -> Vec<Output> {
        let list = requests(out, "agents.list")[0].clone();
        s.incoming(response(list.id, json!({ "agents": agents })))
    }

    #[test]
    fn a_kept_list_shows_until_the_daemon_lists_and_a_new_list_is_taken_once() {
        let kept: Vec<common::AgentInfo> =
            serde_json::from_value(json!([agent_info("w2:pK", "idle")])).unwrap();
        let mut s = State::new("test");
        let out = s.restore_agents(kept.clone());
        assert_eq!(agents_of(&out), vec![kept.clone()]);
        assert_eq!(s.take_new_list(), None, "a kept list is not new");
        // The daemon's first list replaces it, and is taken once to keep.
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(id, json!({"server": "flankr", "vocabulary": []})));
        answer_list(&mut s, &out, json!([agent_info("w2:pZ", "working")]));
        let new = s.take_new_list().expect("the daemon's list");
        assert_eq!(new[0].pane_id, "w2:pZ");
        assert_eq!(s.take_new_list(), None, "once");
        // Listed: a kept list no longer replaces what the daemon gave.
        assert!(s.restore_agents(kept).is_empty());
    }

    /// Online at 0 s, hello's list answered with one working agent.
    fn overview() -> State {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(id, json!({"server": "flankr", "vocabulary": []})));
        let out = answer_list(&mut s, &out, json!([agent_info("w2:pZ", "working")]));
        assert_eq!(agents_of(&out).len(), 1, "the first answer: {out:?}");
        s
    }

    #[test]
    fn each_list_answer_that_changes_gives_the_agents() {
        let mut s = overview();
        // 15 s on, nothing else asked: listed again.
        assert!(requests(&s.input(Input::Tick(secs(14.0))), "agents.list").is_empty());
        let out = s.input(Input::Tick(secs(15.0)));
        assert_eq!(requests(&out, "agents.list").len(), 1);
        // The same list: no event.
        let same = answer_list(&mut s, &out, json!([agent_info("w2:pZ", "working")]));
        assert!(agents_of(&same).is_empty(), "{same:?}");
        // A changed one: the event, with the turn.
        let out = s.input(Input::Tick(secs(30.0)));
        let mut idle = agent_info("w2:pZ", "idle");
        idle["turn"] = json!({"state": "stopped", "line": "Done."});
        let changed = answer_list(&mut s, &out, json!([idle]));
        let given = agents_of(&changed);
        assert_eq!(given.len(), 1, "{changed:?}");
        assert_eq!(
            given[0][0].turn,
            Some(common::Turn::Stopped {
                line: "Done.".into()
            })
        );
        // A list asked for by something else counts: the next is 15 s
        // after it.
        let out = s.input(Input::StartDictation);
        answer_list(&mut s, &out, json!([idle]));
        s.input(Input::Clear);
        assert!(requests(&s.input(Input::Tick(secs(44.0))), "agents.list").is_empty());
        assert_eq!(
            requests(&s.input(Input::Tick(secs(45.0))), "agents.list").len(),
            1
        );
    }

    #[test]
    fn the_drawer_lists_at_once_and_every_3_s_also_with_a_view_open() {
        let mut s = overview();
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        if !requests(&out, "agents.list").is_empty() {
            answer_list(&mut s, &out, json!([agent_info("w2:pZ", "working")]));
        }
        s.input(Input::Tick(secs(1.0)));
        // Shown: at once, though the last list was a second ago.
        let out = s.input(Input::DrawerShown(true));
        assert_eq!(requests(&out, "agents.list").len(), 1);
        answer_list(&mut s, &out, json!([agent_info("w2:pZ", "working")]));
        assert!(requests(&s.input(Input::Tick(secs(3.9))), "agents.list").is_empty());
        let out = s.input(Input::Tick(secs(4.0)));
        assert_eq!(requests(&out, "agents.list").len(), 1);
        answer_list(&mut s, &out, json!([agent_info("w2:pZ", "working")]));
        // Hidden: with the view open and nothing watched, none.
        s.input(Input::DrawerShown(false));
        for t in [7.0, 20.0, 40.0] {
            assert!(requests(&s.input(Input::Tick(secs(t))), "agents.list").is_empty());
        }
        // Not while the app is in the background.
        s.input(Input::Background(true));
        let out = s.input(Input::DrawerShown(true));
        assert!(requests(&out, "agents.list").is_empty());
    }

    #[test]
    fn a_replay_gives_what_the_daemon_showed_and_leaving_closes_the_view() {
        let mut s = overview();
        let replay = s.replay();
        assert_eq!(
            replay[..3],
            [
                Event::Connected {
                    server: "flankr".into()
                },
                Event::Vocabulary(vec![]),
                Event::Llm(wire::LlmInfo::available()),
            ]
        );
        assert!(matches!(&replay[3], Event::Agents(a) if a.len() == 1 && a[0].pane_id == "w2:pZ"));
        assert_eq!(replay[4], Event::PendingPanes(vec![]));
        let n = replay.len();
        assert_eq!(replay[n - 2], Event::Cache { bytes: 0 });
        assert_eq!(replay[n - 1], Event::Plugins(vec![]));

        s.input(Input::OpenAgent("w2:pZ".into()));
        s.input(Input::DrawerShown(true));
        s.leave_foreground();
        assert_eq!(s.cached_panes(), vec!["w2:pZ".to_string()]);
        // The drawer is no longer this daemon's.
        assert!(requests(&s.input(Input::Tick(secs(4.0))), "agents.list").is_empty());

        // Offline, the replay says so with the last retry delay.
        s.disconnected(2000);
        assert_eq!(s.replay()[0], Event::Disconnected { retry_in_ms: 2000 });
    }

    #[test]
    fn focus_changes_list_again_at_most_once_in_five_seconds() {
        let mut s = overview();
        // At 1 s: too soon after hello's list; it waits for 5 s.
        s.input(Input::Tick(secs(1.0)));
        let out = s.incoming(note("focus.changed", json!({"focus": claude()})));
        assert!(requests(&out, "agents.list").is_empty(), "{out:?}");
        assert!(requests(&s.input(Input::Tick(secs(4.9))), "agents.list").is_empty());
        let out = s.input(Input::Tick(secs(5.0)));
        assert_eq!(requests(&out, "agents.list").len(), 1);
        // Another change while it is on its way: none more.
        let more = s.incoming(note("focus.changed", json!({"focus": null})));
        assert!(requests(&more, "agents.list").is_empty());
        answer_list(&mut s, &out, json!([]));
        // Then one more, 5 s after the last.
        assert!(requests(&s.input(Input::Tick(secs(9.0))), "agents.list").is_empty());
        let out = s.input(Input::Tick(secs(10.0)));
        assert_eq!(requests(&out, "agents.list").len(), 1);
        answer_list(&mut s, &out, json!([]));
        // Long after: a change lists at once.
        s.input(Input::Tick(secs(16.0)));
        let out = s.incoming(note("focus.changed", json!({"focus": claude()})));
        assert_eq!(requests(&out, "agents.list").len(), 1);
    }

    /// A session whose turn goes on: a tool call after the reply.
    fn running() -> Value {
        json!({
            "session_id": "s1", "matched": "id",
            "entries": [
                {"ts": "1", "role": "user", "kind": "text", "summary": "go"},
                {"ts": "2", "role": "assistant", "kind": "text", "summary": "Testing."},
                {"ts": "3", "role": "assistant", "kind": "tool_use", "tool": "Bash",
                 "summary": "Bash cargo test"}
            ]
        })
    }

    #[test]
    fn no_list_is_asked_while_the_agent_view_is_open() {
        let mut s = overview();
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        // Opening the view lists, as before, for its name.
        answer_list(&mut s, &out, json!([agent_info("w2:pZ", "working")]));
        session_answer(&mut s, &out, "working", running());
        let mut asked = 0;
        for t in 1..=60 {
            let out = s.input(Input::Tick(secs(f64::from(t))));
            asked += requests(&out, "agents.list").len();
            for peek in requests(&out, "session.peek") {
                s.incoming(response(
                    peek.id.clone(),
                    json!({"session": running(), "status": "working"}),
                ));
            }
        }
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        assert_eq!(asked, 0);
        // Back on the overview: listed at once.
        let out = s.input(Input::CloseAgent);
        assert_eq!(requests(&out, "agents.list").len(), 1);
    }

    #[test]
    fn the_pending_panes_follow_the_confirmations() {
        let mut s = online();
        assert!(s.pending_panes().is_empty());
        let out = s.incoming(confirm_requested(7));
        assert_eq!(s.pending_panes(), vec!["w2:pZ".to_string()]);
        assert!(events(&out).contains(&Event::PendingPanes(vec!["w2:pZ".into()])));
        // A second for the same pane: no change.
        let out = s.incoming(note(
            "confirm.requested",
            json!({"confirm_id": 8, "pane_id": "w2:pZ", "agent": "claude",
                   "command": "/compact", "requested_by": "companion:test",
                   "text": "Approve sending \"/compact\" to Claude?"}),
        ));
        assert!(
            !events(&out)
                .iter()
                .any(|e| matches!(e, Event::PendingPanes(_)))
        );
        s.incoming(note(
            "confirm.resolved",
            json!({"confirm_id": 7, "outcome": "sent"}),
        ));
        assert_eq!(s.pending_panes(), vec!["w2:pZ".to_string()]);
        let out = s.incoming(note(
            "confirm.resolved",
            json!({"confirm_id": 8, "outcome": "denied"}),
        ));
        assert!(s.pending_panes().is_empty());
        assert!(events(&out).contains(&Event::PendingPanes(vec![])));
    }

    #[test]
    fn the_conversation_gives_the_users_words_and_geminis() {
        let mut s = online();
        let out = s.input(Input::Utterance("what is claude doing".into()));
        assert_eq!(
            events(&out),
            vec![Event::Conversation {
                speaker: Speaker::User,
                text: "what is claude doing".into()
            }]
        );
        let out = s.incoming(note(
            "turn.say",
            json!({"turn_id": 1, "seq": 0, "text": "Running the tests."}),
        ));
        assert_eq!(
            events(&out)[0],
            Event::Conversation {
                speaker: Speaker::Gemini,
                text: "Running the tests.".into()
            }
        );
        // Notices are said, not part of the conversation.
        let out = s.incoming(note(
            "notice",
            json!({"pane_id": "w2:pZ", "kind": "agent_status", "text": "Claude is done."}),
        ));
        assert!(
            !events(&out)
                .iter()
                .any(|e| matches!(e, Event::Conversation { .. }))
        );
    }

    #[test]
    fn a_quiet_link_is_pinged_and_closed_when_the_ping_is_unanswered() {
        let mut s = online();
        s.input(Input::Tick(secs(10.0)));
        let pings = |out: &[Output]| requests(out, "ping").len();
        // Heard from at 10 s: nothing sent until 30 s of quiet.
        s.incoming(note("focus.changed", json!({"focus": null})));
        assert_eq!(pings(&s.input(Input::Tick(secs(39.0)))), 0);
        let out = s.input(Input::Tick(secs(40.0)));
        assert_eq!(pings(&out), 1);
        // Not again while it is waiting.
        assert_eq!(pings(&s.input(Input::Tick(secs(45.0)))), 0);
        // Unanswered for 10 s: closed, to reconnect.
        let out = s.input(Input::Tick(secs(50.0)));
        assert!(out.iter().any(|o| matches!(o, Output::Close(_))), "{out:?}");
    }

    #[test]
    fn coming_back_to_the_front_finds_a_dead_link_within_3_s() {
        // On the S23 the system froze the app in the background; back in
        // front, the old socket was dead and the view sat stale for 13 s.
        let mut s = online();
        s.input(Input::Tick(secs(10.0)));
        s.input(Input::Background(true));
        s.input(Input::Tick(secs(4000.0)));
        let out = s.input(Input::Background(false));
        assert_eq!(requests(&out, "ping").len(), 1);
        let out = s.input(Input::Tick(secs(4002.0)));
        assert!(!out.iter().any(|o| matches!(o, Output::Close(_))));
        let out = s.input(Input::Tick(secs(4003.0)));
        assert!(out.iter().any(|o| matches!(o, Output::Close(_))), "{out:?}");
        // Answered in time: kept.
        let mut t = online();
        t.input(Input::Background(true));
        t.input(Input::Tick(secs(100.0)));
        t.input(Input::Background(false));
        t.incoming(note("focus.changed", json!({"focus": null})));
        let out = t.input(Input::Tick(secs(104.0)));
        assert!(!out.iter().any(|o| matches!(o, Output::Close(_))));
    }

    #[test]
    fn anything_heard_keeps_the_link_from_being_pinged() {
        let mut s = online();
        for t in [10.0, 30.0, 50.0, 70.0] {
            s.input(Input::Tick(secs(t)));
            s.incoming(note("focus.changed", json!({"focus": null})));
        }
        assert!(requests(&s.input(Input::Tick(secs(95.0))), "ping").is_empty());
        // A ping answered: the next comes only after another quiet spell.
        let out = s.input(Input::Tick(secs(100.0)));
        let ping = requests(&out, "ping")[0].clone();
        let out = s.incoming(response(ping.id, json!({})));
        assert!(!out.iter().any(|o| matches!(o, Output::Close(_))));
        assert!(requests(&s.input(Input::Tick(secs(115.0))), "ping").is_empty());
        assert!(
            !s.input(Input::Tick(secs(125.0)))
                .iter()
                .any(|o| matches!(o, Output::Close(_)))
        );
    }

    #[test]
    fn in_the_background_the_link_sleeps_and_wakes_with_one_catch_up() {
        let mut s = viewing("working", asking());
        let out = s.input(Input::Background(true));
        // The daemon is told, so it holds back all but what needs the user.
        assert_eq!(
            requests(&out, "client.options")[0].params["background"],
            true
        );
        let mut sent = Vec::new();
        // Four minutes asleep: no peeks, no lists, no pings.
        for t in (110..=360).step_by(10) {
            sent.extend(s.input(Input::Tick(secs(t as f64))));
        }
        for method in ["session.peek", "session.subscribe", "agents.list", "ping"] {
            assert!(requests(&sent, method).is_empty(), "{method}: {sent:?}");
        }
        // After 4.5 minutes of quiet, one ping keeps the socket, saying the
        // app is not in front.
        let out = s.input(Input::Tick(secs(380.0)));
        assert_eq!(requests(&out, "ping").len(), 1);
        assert_eq!(requests(&out, "ping")[0].params["active"], false);
        // Back in front: the view catches up at once.
        let ping = requests(&out, "ping")[0].clone();
        s.incoming(response(ping.id, json!({})));
        let out = s.input(Input::Background(false));
        // The daemon held the view's deliveries and sends them itself: the
        // client asks nothing for the view.
        assert!(requests(&out, "session.peek").is_empty(), "{out:?}");
        assert!(requests(&out, "session.subscribe").is_empty(), "{out:?}");
        // Resumed: a ping says so at once.
        let pings = requests(&out, "ping");
        assert_eq!(pings.len(), 1, "{out:?}");
        assert_eq!(pings[0].params["active"], true);
    }

    // Outline mode (plan 20261024_01, step 4, and CHANGES.md 2026-10-24
    // 00:36 EDT).

    const EPOCH: u64 = 77;

    /// The serial of index `i` in file `file`: on line `i + 1`.
    fn serial_in(file: u32, i: u64) -> String {
        common::Serial::new(file, i as u32 + 1, 0).to_string()
    }

    fn slot_in(file: u32, i: u64, rev: u32) -> Value {
        json!({"index": i, "serial": serial_in(file, i), "kind": "text",
               "lines": 2, "chars": 40, "rev": rev})
    }

    fn slots(range: std::ops::Range<u64>) -> Vec<Value> {
        range.map(|i| slot_in(FILE, i, 0)).collect()
    }

    fn content_in(file: u32, i: u64, summary: &str) -> Value {
        json!({"serial": serial_in(file, i), "index": i, "kind": "text", "entries": [
            {"ts": format!("t{i}"), "role": "assistant", "kind": "text", "summary": summary}
        ]})
    }

    fn content(i: u64) -> Value {
        content_in(FILE, i, &format!("s{i}"))
    }

    /// An outline delivery for subscription 1.
    fn outline_delivery(
        reset: bool,
        slots: Vec<Value>,
        sections: Vec<Value>,
        last: Option<u64>,
        epoch: u64,
    ) -> Value {
        json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
            "reset": reset, "session": {"session_id": "s1", "matched": "id"},
            "reason": null, "status": "idle", "sections": sections, "slots": slots,
            "last": last, "epoch": epoch})
    }

    fn deliver_outline(s: &mut State, delivery: Value) -> Vec<Output> {
        s.incoming(note("session.delivery", delivery))
    }

    /// The view of w2:pZ open, its subscription answered with `first`.
    fn outlined(first: Value) -> (State, Vec<Output>) {
        let mut s = online();
        s.input(Input::Tick(secs(100.0)));
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let list = requests(&out, "agents.list")[0].clone();
        s.incoming(response(
            list.id,
            json!({"agents": [agent_info("w2:pZ", "idle")]}),
        ));
        let sub = requests(&out, "session.subscribe")[0].id.clone();
        let out = s.incoming(response(
            sub,
            json!({"subscription_id": 1, "delivery": first}),
        ));
        (s, out)
    }

    /// A complete outline of `n` sections, the newest `LIVE_INLINE`'s
    /// contents with it.
    fn outline_of(n: u64) -> (State, Vec<Output>) {
        let inline = (n.saturating_sub(common::LIVE_INLINE)..n)
            .map(content)
            .collect();
        outlined(outline_delivery(
            true,
            slots(0..n),
            inline,
            n.checked_sub(1),
            EPOCH,
        ))
    }

    /// The view's sections as (index, held or pending).
    fn ribbon(s: &State) -> Vec<(u64, bool)> {
        match s.agent_view() {
            Some(Event::AgentView { sections, .. }) => sections
                .iter()
                .map(|v| {
                    (
                        v.index,
                        !matches!(v.shows, crate::view::Shows::Pending { .. }),
                    )
                })
                .collect(),
            other => panic!("{other:?}"),
        }
    }

    fn held_indexes(s: &State) -> Vec<u64> {
        ribbon(s)
            .into_iter()
            .filter(|(_, held)| *held)
            .map(|(i, _)| i)
            .collect()
    }

    fn new_content(s: &State) -> bool {
        matches!(
            s.agent_view(),
            Some(Event::AgentView {
                new_content: true,
                ..
            })
        )
    }

    /// The `session.content` requests in `out`, as (from, to, slots_only).
    fn content_asks(out: &[Output]) -> Vec<(u64, u64, bool)> {
        requests(out, "session.content")
            .iter()
            .map(|r| {
                (
                    r.params["from"].as_u64().unwrap(),
                    r.params["to"].as_u64().unwrap(),
                    r.params["slots_only"] == true,
                )
            })
            .collect()
    }

    /// Answers the `session.content` request `r` with the contents of
    /// every index it asked for.
    fn answer_content(s: &mut State, r: &Request) -> Vec<Output> {
        let from = r.params["from"].as_u64().unwrap();
        let to = r.params["to"].as_u64().unwrap();
        s.incoming(response(
            r.id.clone(),
            json!({"sections": (from..=to).map(content).collect::<Vec<_>>(),
                   "first": 0, "last": to, "more": false}),
        ))
    }

    fn window(shown: (u64, u64), near: (u64, u64), keep: (u64, u64), back: bool) -> Input {
        window_closely(shown, shown, near, keep, back)
    }

    /// As [`window`], with the indexes within 1 screen, `close`.
    fn window_closely(
        shown: (u64, u64),
        close: (u64, u64),
        near: (u64, u64),
        keep: (u64, u64),
        back: bool,
    ) -> Input {
        Input::Window(ViewWindow {
            shown,
            close,
            near,
            keep,
            scrolled_back: back,
        })
    }

    #[test]
    fn the_view_subscribes_for_an_outline() {
        let mut s = online();
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        assert_eq!(
            requests(&out, "session.subscribe")[0].params,
            json!({"pane_id": "w2:pZ", "sections": 200, "outline": true})
        );
    }

    #[test]
    fn the_outline_is_held_complete_across_chunked_deliveries() {
        // 3000 sections: 2000 slots, then the rest with the newest
        // contents.
        let (mut s, out) = outlined(outline_delivery(
            true,
            slots(0..2000),
            vec![],
            Some(2999),
            EPOCH,
        ));
        // Still arriving: no contents asked for, no hole filled.
        assert!(content_asks(&out).is_empty(), "{out:?}");
        assert_eq!(ribbon(&s).len(), 2000);
        let inline: Vec<Value> = (2992..3000).map(content).collect();
        let out = deliver_outline(
            &mut s,
            outline_delivery(false, slots(2000..3000), inline, Some(2999), EPOCH),
        );
        let r = ribbon(&s);
        assert_eq!(r.len(), 3000);
        assert!(
            r.iter()
                .enumerate()
                .all(|(i, (index, _))| *index == i as u64)
        );
        assert_eq!(held_indexes(&s), (2992..3000).collect::<Vec<_>>());
        // Complete: the newest screens' contents are asked for, the held
        // ones not again.
        assert_eq!(content_asks(&out), [(2950, 2991, false)]);
        // No hole: nothing more on later ticks.
        let out = s.input(Input::Tick(secs(105.0)));
        assert!(content_asks(&out).is_empty(), "{out:?}");
        // In outline mode, the start is known: index 0 is held.
        assert!(matches!(
            s.agent_view(),
            Some(Event::AgentView {
                start: true,
                loading_earlier: false,
                ..
            })
        ));
        // And no history pages are asked.
        assert!(requests(&s.input(Input::LoadEarlier), "session.history").is_empty());
    }

    #[test]
    fn a_hole_in_the_outline_is_asked_for_with_slots_only() {
        let (mut s, _) = outline_of(10);
        // A slot past a hole: 10..=14 were missed.
        let out = deliver_outline(
            &mut s,
            outline_delivery(false, vec![slot_in(FILE, 15, 0)], vec![], Some(15), EPOCH),
        );
        assert!(
            content_asks(&out).is_empty(),
            "the hole may still be on its way"
        );
        let out = s.input(Input::Tick(secs(100.5)));
        assert!(content_asks(&out).is_empty());
        let out = s.input(Input::Tick(secs(101.5)));
        let asks = requests(&out, "session.content");
        assert_eq!(content_asks(&out), [(10, 15, true)]);
        // Once in flight, not again.
        assert!(content_asks(&s.input(Input::Tick(secs(103.0)))).is_empty());
        let out = s.incoming(response(
            asks[0].id.clone(),
            json!({"slots": slots(10..14), "removed_index": [14], "first": 0, "last": 15}),
        ));
        let r = ribbon(&s);
        assert_eq!(
            r.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15]
        );
        // Complete now: the contents near the bottom are fetched (0 and 1
        // are still on their way from the first answer).
        assert_eq!(content_asks(&out), [(10, 15, false)]);
        assert!(content_asks(&s.input(Input::Tick(secs(110.0)))).is_empty());
    }

    #[test]
    fn contents_are_asked_for_around_the_window_two_requests_at_most() {
        let (mut s, out) = outline_of(1000);
        let first = requests(&out, "session.content");
        assert_eq!(content_asks(&out), [(950, 991, false)]);
        answer_content(&mut s, first[0]);
        // The window scrolled back: the near slots, nearest first, in
        // ranges of at most 200, two in flight.
        let out = s.input(window((500, 510), (400, 620), (0, 999), true));
        assert_eq!(content_asks(&out), [(400, 599, false), (600, 620, false)]);
        let asked = requests(&out, "session.content")
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        // Moved again while both are in flight: nothing more.
        let out = s.input(window((100, 110), (50, 160), (0, 999), true));
        assert!(content_asks(&out).is_empty());
        // One answered: the new window's are asked, the nearest first.
        let out = answer_content(&mut s, &asked[1]);
        assert_eq!(content_asks(&out), [(50, 160, false)]);
        let held = held_indexes(&s);
        assert!(held.contains(&600) && held.contains(&620) && !held.contains(&400));
        // Only around the window: nothing between is held.
        assert!(!held.iter().any(|i| (161..400).contains(i)));
    }

    #[test]
    fn a_replaced_slots_stale_contents_are_fetched_again() {
        let (mut s, out) = outline_of(100);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        assert!(held_indexes(&s).contains(&60));
        // Slot 60 replaced, its contents not along (not among the newest).
        let out = deliver_outline(
            &mut s,
            outline_delivery(false, vec![slot_in(FILE, 60, 1)], vec![], Some(99), EPOCH),
        );
        assert_eq!(content_asks(&out), [(60, 60, false)]);
        // Shown as held until then: the old contents stand.
        assert!(held_indexes(&s).contains(&60));
        let r = requests(&out, "session.content")[0].clone();
        let out = answer_content(&mut s, &r);
        assert!(content_asks(&out).is_empty(), "{out:?}");
    }

    #[test]
    fn far_contents_are_forgotten_and_the_outline_kept() {
        let (mut s, out) = outline_of(1000);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        let out = s.input(window((100, 110), (50, 160), (0, 400), true));
        for r in requests(&out, "session.content") {
            answer_content(&mut s, r);
        }
        assert!(held_indexes(&s).contains(&100));
        // The far end's contents go, but the newest LIVE_INLINE stay.
        let held = held_indexes(&s);
        assert!(!held.iter().any(|i| (401..992).contains(i)), "{held:?}");
        assert!(held.contains(&999));
        // Back to the bottom: the near ones go far, and are forgotten.
        s.input(window((990, 999), (900, 999), (500, 999), false));
        let held = held_indexes(&s);
        assert!(!held.contains(&100));
        assert_eq!(ribbon(&s).len(), 1000, "the outline is all kept");
    }

    #[test]
    fn a_reset_is_merged_by_serial_and_unchanged_revs_are_not_fetched_again() {
        let (mut s, out) = outline_of(60);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        assert_eq!(held_indexes(&s), (10..60).collect::<Vec<_>>());
        s.input(window((40, 50), (10, 59), (0, 59), false));
        // The daemon restarted: a new epoch's outline. Index 20's section is
        // gone (one fewer below 20 moves the rest), 30's has a new rev.
        let mut fresh: Vec<Value> = Vec::new();
        for i in (0..60).filter(|&i| i != 20) {
            let index = if i < 20 { i } else { i - 1 };
            let rev = u32::from(i == 30);
            fresh.push(
                json!({"index": index, "serial": serial_in(FILE, i), "kind": "text",
                              "lines": 2, "chars": 40, "rev": rev}),
            );
        }
        let out = deliver_outline(&mut s, outline_delivery(true, fresh, vec![], Some(58), 78));
        let r = ribbon(&s);
        assert_eq!(r.len(), 59);
        // Held by serial: only the changed rev is asked for again.
        assert_eq!(content_asks(&out), [(29, 29, false)]);
        let held = held_indexes(&s);
        assert_eq!(held.len(), 49, "the 49 still in the outline, 20's dropped");
        assert!(held.contains(&19) && held.contains(&57));
        // The reset's epoch is the one held now.
        s.disconnected(1000);
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(id, json!({"server": "flankr"})));
        let params = &requests(&out, "session.subscribe")[0].params;
        assert_eq!(params["epoch"], 78);
        assert_eq!(params["from"], 59);
        assert_eq!(params["after"], json!(serial_in(FILE, 59)));
    }

    #[test]
    fn a_new_outline_while_scrolled_back_is_held_aside_until_applied() {
        let (mut s, out) = outline_of(30);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        s.input(window((5, 10), (0, 29), (0, 29), true));
        let before = ribbon(&s);
        // A reset: the old ribbon stays, and the event says one waits.
        let out = deliver_outline(
            &mut s,
            outline_delivery(true, slots(0..40), vec![content(39)], Some(39), 78),
        );
        assert!(new_content(&s));
        assert_eq!(ribbon(&s), before);
        assert!(
            content_asks(&out).is_empty(),
            "nothing fetched for the old ribbon"
        );
        // A later delivery goes to the one held aside too.
        deliver_outline(
            &mut s,
            outline_delivery(false, vec![slot_in(FILE, 40, 0)], vec![], Some(40), 78),
        );
        assert_eq!(ribbon(&s), before);
        // The jump line's tap (plan 20261022_03, the 01:06 EDT amendment,
        // item 5): no swap. The new outline's sections after the old
        // ribbon's newest go below its end, at their own indexes (they
        // come after 29), with the contents held for them.
        let out = s.input(Input::ApplyOutline);
        assert!(content_asks(&out).is_empty(), "{out:?}");
        assert!(new_content(&s), "the rest still held aside");
        let r = ribbon(&s);
        assert_eq!(&r[..30], &before[..]);
        assert_eq!(
            r[30..].iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            (30..41).collect::<Vec<_>>()
        );
        assert!(held_indexes(&s).contains(&39));
        let appended = serials(&s);
        // Scrolling there reports nothing; landing at the bottom,
        // following, merges the rest in silently: the same sections in the
        // same order, so the view at the bottom does not move.
        let out = s.input(window((31, 40), (0, 40), (0, 40), false));
        assert!(!new_content(&s));
        assert_eq!(serials(&s), appended);
        let r = ribbon(&s);
        assert_eq!(r.len(), 41);
        assert!(held_indexes(&s).contains(&39) && held_indexes(&s).contains(&5));
        // Merged by serial at unchanged revs: only what was never held is
        // fetched.
        let asked: Vec<u64> = content_asks(&out)
            .iter()
            .flat_map(|&(lo, hi, _)| lo..=hi)
            .collect();
        assert!(
            asked
                .iter()
                .all(|i| (0..10).contains(i) || (30..=40).contains(i)),
            "{asked:?}"
        );
    }

    /// The view's serials, in ribbon order.
    fn serials(s: &State) -> Vec<common::Serial> {
        match s.agent_view() {
            Some(Event::AgentView { sections, .. }) => sections.iter().map(|v| v.serial).collect(),
            other => panic!("{other:?}"),
        }
    }

    fn held_end_of(s: &State) -> Vec<(u64, crate::view::Shows)> {
        match s.agent_view() {
            Some(Event::AgentView { held_end, .. }) => {
                held_end.into_iter().map(|v| (v.index, v.shows)).collect()
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn newer_sections_whose_indexes_come_before_the_old_end_keep_their_order_below_it() {
        let (mut s, out) = outline_of(30);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        s.input(window((5, 10), (0, 29), (0, 29), true));
        let before = serials(&s);
        // A reset in which two old sections are gone: the newer serials
        // (lines 31..35) have indexes 28..32, not after the old end's 29.
        let fresh: Vec<Value> = (0..35u64)
            .filter(|i| !(5..7).contains(i))
            .map(|i| {
                let index = if i < 5 { i } else { i - 2 };
                json!({"index": index, "serial": serial_in(FILE, i), "kind": "text",
                       "lines": 2, "chars": 40, "rev": 0})
            })
            .collect();
        deliver_outline(
            &mut s,
            outline_delivery(true, fresh, vec![content(34)], Some(32), 78),
        );
        assert_eq!(serials(&s), before);
        s.input(Input::ApplyOutline);
        let r = ribbon(&s);
        assert_eq!(r.len(), 35);
        // Placed at the old last index + 1 + their own: 58..=62, in order.
        assert_eq!(
            r[30..].iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            (58..63).collect::<Vec<_>>()
        );
        let tail: Vec<common::Serial> = serials(&s)[30..].to_vec();
        assert_eq!(
            tail,
            (30..35u64)
                .map(|i| common::Serial::new(FILE, i as u32 + 1, 0))
                .collect::<Vec<_>>()
        );
        assert!(r[34].1, "34's contents, held aside, shown");
        // At the bottom: merged, the newest sections still last, in order.
        s.input(window((50, 62), (0, 62), (0, 62), false));
        let now = serials(&s);
        assert_eq!(now.len(), 33);
        assert_eq!(&now[28..], &tail[..]);
        assert_eq!(ribbon(&s).last().unwrap().0, 32);
    }

    #[test]
    fn the_end_of_an_outline_held_aside_is_given_for_the_jump_line() {
        let (mut s, out) = outline_of(30);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        assert!(held_end_of(&s).is_empty(), "none held aside");
        s.input(window((5, 10), (0, 29), (0, 29), true));
        let mut fresh = slots(0..32);
        fresh[30] = json!({"index": 30, "serial": serial_in(FILE, 30), "kind": "card",
                           "lines": 1, "chars": 20, "rev": 0});
        let question = json!({"serial": serial_in(FILE, 30), "index": 30, "kind": "card", "card": {
            "card": "question", "live": true, "title": "Claude is asking",
            "text": "Which one?"}});
        deliver_outline(
            &mut s,
            outline_delivery(true, fresh, vec![question, content(31)], Some(31), 78),
        );
        let end = held_end_of(&s);
        assert_eq!(end.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [30, 31]);
        assert!(
            matches!(&end[0].1, crate::view::Shows::Card(c) if c.live && c.text == "Which one?")
        );
        // Once its newer sections are below the end, they are in the
        // ribbon itself.
        s.input(Input::ApplyOutline);
        assert!(held_end_of(&s).is_empty());
        assert_eq!(ribbon(&s).len(), 32);
    }

    fn held_newer_of(s: &State) -> Vec<crate::view::Slot> {
        match s.agent_view() {
            Some(Event::AgentView { held_newer, .. }) => held_newer,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_newer_slots_of_an_outline_held_aside_are_given_for_the_scrollbar() {
        let (mut s, out) = outline_of(30);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        assert!(held_newer_of(&s).is_empty(), "none held aside");
        s.input(window((5, 10), (0, 29), (0, 29), true));
        let mut fresh = slots(0..33);
        fresh[31] = json!({"index": 31, "serial": serial_in(FILE, 31), "kind": "tool",
                           "lines": 7, "chars": 300, "rev": 0});
        deliver_outline(
            &mut s,
            outline_delivery(true, fresh, vec![content(32)], Some(32), 78),
        );
        let newer = held_newer_of(&s);
        // Only those after the shown ribbon's newest, with their sizes,
        // whether or not their contents are held.
        assert_eq!(
            newer.iter().map(|slot| slot.index).collect::<Vec<_>>(),
            [30, 31, 32]
        );
        assert_eq!(
            (newer[1].kind, newer[1].lines, newer[1].chars),
            (crate::view::SectionKind::Tool, 7, 300)
        );
        assert_eq!(ribbon(&s).len(), 30, "the ribbon itself unchanged");
        // Once they are below the end, they are in the ribbon instead.
        s.input(Input::ApplyOutline);
        assert!(held_newer_of(&s).is_empty());
        assert_eq!(ribbon(&s).len(), 33);
    }

    #[test]
    fn a_new_outline_while_following_is_shown_at_once() {
        let (mut s, _) = outline_of(30);
        s.input(window((20, 29), (0, 29), (0, 29), false));
        deliver_outline(
            &mut s,
            outline_delivery(true, slots(0..35), vec![], Some(34), 78),
        );
        assert!(!new_content(&s));
        assert_eq!(ribbon(&s).len(), 35);
        // Held aside while back, applied by returning to the bottom.
        s.input(window((5, 10), (0, 29), (0, 34), true));
        deliver_outline(
            &mut s,
            outline_delivery(true, slots(0..36), vec![], Some(35), 79),
        );
        assert!(new_content(&s));
        s.input(window((26, 35), (0, 35), (0, 35), false));
        assert!(!new_content(&s));
        assert_eq!(ribbon(&s).len(), 36);
    }

    #[test]
    fn a_new_session_file_while_scrolled_back_waits_and_replaces_everything() {
        let (mut s, out) = outline_of(30);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        s.input(window((5, 10), (0, 29), (0, 29), true));
        let before = ribbon(&s);
        let other = FILE + 1;
        let fresh: Vec<Value> = (0..3).map(|i| slot_in(other, i, 0)).collect();
        let inline: Vec<Value> = (0..3).map(|i| content_in(other, i, "new")).collect();
        deliver_outline(&mut s, outline_delivery(true, fresh, inline, Some(2), 78));
        assert!(new_content(&s));
        assert_eq!(ribbon(&s), before, "the old session stays until the tap");
        assert_eq!(summaries(&s)[0], "s0");
        // The tap: the new session's sections below the old one's end, in
        // order after its last index.
        s.input(Input::ApplyOutline);
        let r = ribbon(&s);
        assert_eq!(&r[..30], &before[..]);
        assert_eq!(&r[30..], [(30, true), (31, true), (32, true)]);
        assert_eq!(summaries(&s)[30..], ["new", "new", "new"]);
        // At the bottom, following: the old session goes.
        s.input(window((30, 32), (0, 32), (0, 32), false));
        assert_eq!(ribbon(&s), [(0, true), (1, true), (2, true)]);
        assert_eq!(summaries(&s), ["new", "new", "new"]);
    }

    // The settings screen (plan 20261024_02, step 4).

    fn settings(sound: bool, ding: bool, microphone: bool, general_agent: bool) -> Input {
        Input::Settings(Settings {
            sound,
            ding,
            microphone,
            general_agent,
            tab_follows_focus: false,
            speak_gemini: true,
        })
    }

    #[test]
    fn settings_start_on() {
        let s = State::new("test");
        assert_eq!(s.settings(), Settings::default());
        assert_eq!(
            Settings::default(),
            Settings {
                sound: true,
                ding: true,
                microphone: true,
                general_agent: true,
                tab_follows_focus: false,
                speak_gemini: true
            }
        );
    }

    #[test]
    fn with_sound_off_nothing_is_said_or_queued_to_be_read() {
        let mut s = viewing("working", asking());
        s.input(Input::ReadAloud(true));
        // Read aloud is off in effect: the daemon is told.
        let out = s.input(settings(false, true, true, true));
        assert_eq!(
            options(&out),
            [
                json!({"spoken_output": false, "background": false, "general_agent": true, "tab_follows_focus": false})
            ]
        );
        // A new reply is not sent to be measured, nor read.
        let out = s.input(Input::Tick(secs(200.0)));
        let mut entries = asking()["entries"].as_array().unwrap().clone();
        entries.push(json!({"ts": "10", "role": "assistant", "kind": "text",
                            "summary": "All done."}));
        let session = json!({"session_id": "s1", "matched": "id", "entries": entries});
        let out = session_answer(&mut s, &out, "idle", session);
        assert!(requests(&out, "speech.summarize").is_empty(), "{out:?}");
        assert!(said(&out).is_empty());
        // Nor a notice, nor a confirmation's question; its card still comes.
        let out = s.incoming(notice("Claude is done."));
        assert!(said(&out).is_empty(), "{out:?}");
        let out = s.incoming(confirm_requested(7));
        assert!(said(&out).is_empty(), "{out:?}");
        assert!(
            events(&out)
                .iter()
                .any(|e| matches!(e, Event::Confirm { confirm_id: 7, .. }))
        );
        // Back on: said again, and read aloud is on again for the daemon.
        let out = s.input(settings(true, true, true, true));
        assert_eq!(options(&out)[0]["spoken_output"], true);
        // The notice from while it was off is not kept; a new one waits
        // for the confirmation, then is said.
        let out = s.incoming(notice("Codex is done."));
        assert!(said(&out).is_empty(), "the confirmation holds it: {out:?}");
        let out = s.input(Input::Answer {
            confirm_id: 7,
            approve: false,
        });
        assert_eq!(said(&out), ["Codex is done."]);
    }

    #[test]
    fn turning_sound_off_stops_what_is_being_said() {
        let mut s = online();
        s.input(Input::Speaking(true));
        let out = s.input(settings(false, true, true, true));
        assert!(events(&out).contains(&Event::StopSpeaking), "{out:?}");
        // Off again changes nothing more.
        assert!(s.input(settings(false, true, true, true)).is_empty());
    }

    #[test]
    fn with_the_microphone_off_nothing_listens_or_is_dictated() {
        let mut s = online();
        s.input(settings(true, true, false, true));
        assert!(!events(&s.input(Input::Tap)).contains(&Event::Arm));
        let out = s.input(Input::Utterance("hello".into()));
        assert!(requests(&out, "turn.start").is_empty(), "{out:?}");
        let out = s.input(Input::StartDictation);
        assert!(dictation_of(&out).is_none(), "{out:?}");
        assert!(!events(&out).contains(&Event::Arm));
        // On again: a tap arms the gate.
        s.input(settings(true, true, true, true));
        assert!(events(&s.input(Input::Tap)).contains(&Event::Arm));
    }

    #[test]
    fn turning_the_microphone_off_ends_a_dictation() {
        let mut s = dictating();
        let out = s.input(settings(true, true, false, true));
        assert!(!dictation_of(&out).unwrap().active, "{out:?}");
    }

    #[test]
    fn with_the_general_agent_off_no_turn_starts() {
        let mut s = online();
        let out = s.input(settings(true, true, true, false));
        assert_eq!(
            options(&out),
            [
                json!({"spoken_output": false, "background": false, "general_agent": false, "tab_follows_focus": false})
            ]
        );
        // On the overview, an utterance does nothing.
        let out = s.input(Input::Utterance("what's going on".into()));
        assert!(requests(&out, "turn.start").is_empty(), "{out:?}");
        assert!(dictation_of(&out).is_none());
        // Other settings changing leave it off for the daemon.
        let out = s.input(settings(true, false, true, false));
        assert!(options(&out).is_empty(), "{out:?}");
    }

    // Tab follows focus and the turn's session (plan 20261027_04, step 6).

    fn with_tab_follows_focus(on: bool) -> Input {
        Input::Settings(Settings {
            tab_follows_focus: on,
            ..Settings::default()
        })
    }

    #[test]
    fn tab_follows_focus_is_sent_as_an_option_on_toggling_and_after_hello() {
        let mut s = online();
        // Toggled on, then off: sent each time.
        let out = s.input(with_tab_follows_focus(true));
        assert_eq!(
            options(&out),
            [
                json!({"spoken_output": false, "background": false, "general_agent": true, "tab_follows_focus": true})
            ]
        );
        // The same again sends nothing.
        assert!(options(&s.input(with_tab_follows_focus(true))).is_empty());
        let out = s.input(with_tab_follows_focus(false));
        assert_eq!(options(&out)[0]["tab_follows_focus"], false);
        // Offline, nothing is sent; after the reconnect's hello, the value
        // as it is now.
        s.disconnected(1000);
        assert!(options(&s.input(with_tab_follows_focus(true))).is_empty());
        let out = s.connected();
        assert!(options(&out).is_empty(), "not before hello");
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(
            id,
            json!({"server": "flankr", "session_id": "companion:test", "vocabulary": []}),
        ));
        assert_eq!(
            options(&out),
            [
                json!({"spoken_output": false, "background": false, "general_agent": true, "tab_follows_focus": true})
            ]
        );
    }

    #[test]
    fn a_turn_from_the_agent_view_carries_its_session() {
        let mut s = viewing("idle", json!(null));
        let turn = start(&mut s, "what is this project about");
        assert_eq!(turn.params["session"], "w2:pZ", "{turn:?}");
    }

    #[test]
    fn a_turn_from_the_overview_carries_no_session() {
        let mut s = online();
        let turn = start(&mut s, "what is going on");
        assert!(turn.params.get("session").is_none(), "{turn:?}");
        // Nor after a view is closed.
        let mut s = viewing("idle", json!(null));
        s.input(Input::CloseAgent);
        let out = s.input(Input::Utterance("and now".into()));
        let turn = requests(&out, "turn.start");
        assert_eq!(turn.len(), 1, "{out:?}");
        assert!(turn[0].params.get("session").is_none(), "{out:?}");
    }

    /// `server.capabilities` with `llm`.
    fn capabilities(general_agent: bool, summaries: bool) -> Incoming {
        let reason = (!general_agent || !summaries).then_some("[llm] enabled = true is not set");
        note(
            "server.capabilities",
            json!({"llm": {"general_agent": general_agent, "summaries": summaries,
                           "reason": reason}}),
        )
    }

    fn llm_events(out: &[Output]) -> Vec<wire::LlmInfo> {
        events(out)
            .into_iter()
            .filter_map(|e| match e {
                Event::Llm(i) => Some(i.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn hello_says_what_llm_offers_and_an_older_daemon_offers_both() {
        // An older daemon: no `llm`, both.
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(id, json!({"server": "flankr", "vocabulary": []})));
        assert_eq!(llm_events(&out), [wire::LlmInfo::available()]);
        assert!(s.general_agent());
        // A daemon with [llm] off.
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(
            id,
            json!({"server": "flankr", "vocabulary": [],
                   "llm": {"general_agent": false, "summaries": false, "reason": "off"}}),
        ));
        let info = &llm_events(&out)[0];
        assert!(!info.general_agent && !info.summaries);
        assert_eq!(info.reason.as_deref(), Some("off"));
        assert!(!s.general_agent());
        // No turn starts, though the toggle is on and is still said as on.
        assert!(s.settings().general_agent);
        let out = s.input(Input::Utterance("what's going on".into()));
        assert!(requests(&out, "turn.start").is_empty(), "{out:?}");
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Keyboard));
        assert!(dictation_of(&out).is_none(), "{out:?}");
    }

    #[test]
    fn server_capabilities_change_the_general_agent_while_connected() {
        let mut s = online();
        assert!(s.general_agent());
        let out = s.incoming(capabilities(false, true));
        let info = &llm_events(&out)[0];
        assert!(!info.general_agent && info.summaries);
        assert!(!s.general_agent());
        let out = s.input(Input::Utterance("hello".into()));
        assert!(requests(&out, "turn.start").is_empty(), "{out:?}");
        // Back: a turn starts again.
        let out = s.incoming(capabilities(true, true));
        assert_eq!(llm_events(&out), [wire::LlmInfo::available()]);
        let out = s.input(Input::Utterance("hello".into()));
        assert_eq!(requests(&out, "turn.start").len(), 1, "{out:?}");
        // One without `llm` changes nothing.
        let out = s.incoming(note("server.capabilities", json!({})));
        assert!(llm_events(&out).is_empty());
    }

    #[test]
    fn losing_the_general_agent_ends_its_prompt() {
        let mut s = online();
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Keyboard));
        assert!(dictation_of(&out).unwrap().general_agent);
        let out = s.incoming(capabilities(false, false));
        assert!(!dictation_of(&out).expect("cleared").active, "{out:?}");
    }

    #[test]
    fn with_the_general_agent_off_an_utterance_in_the_view_is_a_message() {
        let mut s = viewing("idle", asking());
        s.input(settings(true, true, true, false));
        let out = s.input(Input::Utterance("Run the tests again.".into()));
        assert!(requests(&out, "turn.start").is_empty(), "{out:?}");
        let card = dictation_of(&out).expect("a message");
        assert!(card.active);
        assert_eq!(card.text, "Run the tests again.");
        assert_eq!(card.target.unwrap().pane_id, "w2:pZ");
    }

    /// An agent's entry in `agents.list` with its turn.
    fn agent_turn(pane: &str, turn: Value) -> Value {
        let mut a = agent_info(pane, "idle");
        a["turn"] = turn;
        a
    }

    fn working() -> Value {
        json!({"state": "working", "line": "Reading files"})
    }

    fn stopped(line: &str) -> Value {
        json!({"state": "stopped", "line": line})
    }

    fn dings(out: &[Output]) -> usize {
        events(out).iter().filter(|e| **e == Event::Ding).count()
    }

    /// Online, the hello's `agents.list` answered with `agents`; the clock
    /// at 0.
    fn listed(agents: Value) -> (State, Vec<Output>) {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(
            id,
            json!({"server": "flankr", "session_id": "companion:test", "vocabulary": []}),
        ));
        let list = requests(&out, "agents.list")[0].id.clone();
        let out = s.incoming(response(list, json!({"agents": agents})));
        (s, out)
    }

    /// The next list, due at `at` seconds, answered with `agents`.
    fn relist(s: &mut State, at: f64, agents: Value) -> Vec<Output> {
        let out = s.input(Input::Tick(secs(at)));
        let list = requests(&out, "agents.list")[0].id.clone();
        s.incoming(response(list, json!({"agents": agents})))
    }

    /// Plan 20261022_03, the 2026-10-28 00:47 EDT amendment: a shell is
    /// listed as a session, terminal only; its view subscribes to no
    /// transcript and takes no message, spoken or typed.
    #[test]
    fn a_shell_is_listed_and_its_view_is_terminal_only() {
        let shell = json!({"pane_id": "w1:p3", "name": "Shell in src, tab 3",
                           "agent": "shell", "cwd": "/Users/me/src",
                           "status": "unknown", "focused": false});
        let (mut s, out) = listed(json!([shell, agent_info("w2:pZ", "idle")]));
        let agents = events(&out)
            .into_iter()
            .find_map(|e| match e {
                Event::Agents(a) => Some(a),
                _ => None,
            })
            .expect("the list");
        assert_eq!(agents.len(), 2);
        assert!(crate::is_terminal_only(&agents[0]));
        assert!(!crate::is_terminal_only(&agents[1]));

        let out = s.input(Input::OpenAgent("w1:p3".into()));
        assert!(requests(&out, "session.subscribe").is_empty(), "{out:?}");
        assert!(s.viewing_terminal_only());
        s.input(Input::StartDictation);
        assert!(s.dictation.is_none());
        s.input(Input::StartTextPrompt);
        assert!(s.dictation.is_none());
        let out = s.input(Input::Tick(secs(30.0)));
        assert!(requests(&out, "agents.send").is_empty(), "{out:?}");

        // An agent's view still subscribes.
        s.input(Input::CloseAgent);
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        assert_eq!(requests(&out, "session.subscribe").len(), 1, "{out:?}");
        s.input(Input::StartTextPrompt);
        assert!(s.dictation.is_some());
    }

    #[test]
    fn a_ding_once_per_finished_turn_on_the_list() {
        // Finished before the app knew it: no ding.
        let (mut s, out) = listed(json!([agent_turn("w1:p1", stopped("Old."))]));
        assert_eq!(dings(&out), 0);
        let out = relist(&mut s, 15.0, json!([agent_turn("w1:p1", working())]));
        assert_eq!(dings(&out), 0);
        // Working, then stopped: one ding, and none for the same turn again.
        let out = relist(&mut s, 30.0, json!([agent_turn("w1:p1", stopped("Done."))]));
        assert_eq!(dings(&out), 1, "{out:?}");
        let out = relist(&mut s, 45.0, json!([agent_turn("w1:p1", stopped("Done."))]));
        assert_eq!(dings(&out), 0);
        // A turn missed between two lists still dings; so does each agent.
        let out = relist(
            &mut s,
            60.0,
            json!([
                agent_turn("w1:p1", stopped("Again.")),
                agent_turn("w2:p2", working())
            ]),
        );
        assert_eq!(dings(&out), 1);
        let out = relist(
            &mut s,
            75.0,
            json!([
                agent_turn("w1:p1", working()),
                agent_turn("w2:p2", stopped("Ok."))
            ]),
        );
        assert_eq!(dings(&out), 1);
    }

    #[test]
    fn no_ding_with_the_ding_or_sound_off() {
        for (sound, ding) in [(true, false), (false, true)] {
            let (mut s, _) = listed(json!([agent_turn("w1:p1", working())]));
            s.input(settings(sound, ding, true, true));
            let out = relist(&mut s, 15.0, json!([agent_turn("w1:p1", stopped("Done."))]));
            assert_eq!(dings(&out), 0, "sound {sound}, ding {ding}");
        }
    }

    fn finished_at(line: u32) -> Value {
        json!({"serial": serial(line, 0), "kind": "card", "card": {
            "card": "finished", "live": true, "title": "Finished", "text": "Done."}})
    }

    /// A delivery for subscription 1 with herdr's `status`.
    fn deliver_status(s: &mut State, status: &str, sections: Vec<Value>) -> Vec<Output> {
        s.incoming(note(
            "session.delivery",
            json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
                "reset": false, "session": {"session_id": "s1", "matched": "id"},
                "reason": null, "sections": sections, "removed": [], "status": status}),
        ))
    }

    #[test]
    fn a_ding_once_per_new_finished_card_in_the_view() {
        // The card shown on opening is not new.
        let mut s = viewing("idle", json!(null));
        let out = deliver_status(&mut s, "idle", vec![text_at(1, 0, "a"), finished_at(2)]);
        assert_eq!(dings(&out), 0, "{out:?}");
        // The agent works, then finishes: one ding.
        let out = deliver_status(&mut s, "working", vec![text_at(3, 0, "b")]);
        assert_eq!(dings(&out), 0);
        let out = deliver_status(&mut s, "idle", vec![finished_at(4)]);
        assert_eq!(dings(&out), 1, "{out:?}");
        // The same card again, or more of the session: no more.
        let out = deliver_status(&mut s, "idle", vec![finished_at(4), text_at(1, 1, "c")]);
        assert_eq!(dings(&out), 0);

        // Back on the overview, the list's stopped turn is the same one.
        let out = s.input(Input::CloseAgent);
        let out = match requests(&out, "agents.list").first() {
            Some(list) => list.id.clone(),
            None => {
                let out = s.input(Input::Tick(secs(130.0)));
                requests(&out, "agents.list")[0].id.clone()
            }
        };
        let out = s.incoming(response(
            out,
            json!({"agents": [agent_turn("w2:pZ", stopped("Done."))]}),
        ));
        assert_eq!(dings(&out), 0, "{out:?}");
    }

    /// The active remote of the unpairing tests.
    const ID: RemoteId = RemoteId([7; 8]);

    #[test]
    fn the_probes_hello_has_no_session_and_changes_nothing() {
        let mut s = online();
        s.set_session_id(Some("companion:phone".into()));
        let hello = s.hello_request();
        assert_eq!(hello.method, "hello");
        assert_eq!(hello.params["client"], "test");
        assert!(
            hello.params.get("session_id").is_none(),
            "{:?}",
            hello.params
        );
        assert!(s.online());
        assert_eq!(s.session_id(), Some("companion:phone"));
        // The net thread's inputs do nothing here.
        assert!(s.input(Input::Unpair).is_empty());
        assert!(s.input(Input::CancelPairing).is_empty());
    }

    #[test]
    fn unpairing_online_asks_the_daemon_then_forgets() {
        let mut s = online();
        let out = s.input(Input::RemoveRemote(ID));
        let asked = requests(&out, "link.unpair");
        assert_eq!(asked.len(), 1, "{out:?}");
        assert_eq!(asked[0].params, json!({}));
        let id = asked[0].id.clone();
        // Asked once.
        assert!(requests(&s.input(Input::RemoveRemote(ID)), "link.unpair").is_empty());
        let out = s.incoming(response(id, json!({})));
        assert_eq!(
            out,
            [Output::Remove {
                id: ID,
                daemon: true
            }]
        );
    }

    #[test]
    fn unpairing_forgets_on_a_refusal_a_timeout_or_a_disconnect() {
        // Refused (the local socket): forgotten here.
        let mut s = online();
        let out = s.input(Input::RemoveRemote(ID));
        let id = requests(&out, "link.unpair")[0].id.clone();
        let out = s.incoming(Incoming::Response(Response::err(
            Some(id),
            libflankr_proto::ErrorObject::new(
                -32001,
                "link.unpair is served only over a paired link",
            ),
        )));
        assert_eq!(
            out,
            [Output::Remove {
                id: ID,
                daemon: false
            }]
        );

        // No answer in UNPAIR_WAIT.
        let mut s = online();
        s.input(Input::Tick(secs(10.0)));
        s.input(Input::RemoveRemote(ID));
        assert!(!s.input(Input::Tick(secs(14.9))).contains(&Output::Remove {
            id: ID,
            daemon: false
        }));
        assert!(s.input(Input::Tick(secs(15.0))).contains(&Output::Remove {
            id: ID,
            daemon: false
        }));

        // The link closes first.
        let mut s = online();
        s.input(Input::RemoveRemote(ID));
        assert!(s.disconnected(1000).contains(&Output::Remove {
            id: ID,
            daemon: false
        }));
    }

    #[test]
    fn unpairing_offline_forgets_at_once() {
        let mut s = State::new("test");
        assert_eq!(
            s.input(Input::RemoveRemote(ID)),
            [Output::Remove {
                id: ID,
                daemon: false
            }]
        );
    }

    /// The last agent view given in `out`.
    fn last_view(out: &[Output]) -> Option<Event> {
        out.iter().rev().find_map(|o| match o {
            Output::Event(e @ Event::AgentView { .. }) => Some(e.clone()),
            _ => None,
        })
    }

    fn cache_of(out: &[Output]) -> Option<u64> {
        out.iter().rev().find_map(|o| match o {
            Output::Event(Event::Cache { bytes }) => Some(*bytes),
            _ => None,
        })
    }

    fn refreshed(out: &[Output]) -> Option<usize> {
        out.iter().find_map(|o| match o {
            Output::Event(Event::Refreshed { new }) => Some(*new),
            _ => None,
        })
    }

    /// The view of w2:pZ, an outline of 10 with every content held.
    fn held_outline() -> State {
        let (mut s, out) = outline_of(10);
        for r in requests(&out, "session.content") {
            answer_content(&mut s, r);
        }
        assert_eq!(held_indexes(&s), (0..10).collect::<Vec<_>>());
        s
    }

    #[test]
    fn a_closed_view_reopens_from_the_cache_at_once_and_resumes_with_from() {
        let mut s = held_outline();
        let out = s.input(Input::CloseAgent);
        assert_eq!(requests(&out, "session.unsubscribe").len(), 1);
        assert!(cache_of(&out).is_some_and(|b| b > 0), "{out:?}");
        assert_eq!(s.cached_panes(), ["w2:pZ"]);
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        // At once: no loading, its sections held, none pending.
        match last_view(&out) {
            Some(Event::AgentView {
                loading, sections, ..
            }) => {
                assert!(!loading);
                assert_eq!(sections.len(), 10);
                assert!(
                    sections
                        .iter()
                        .all(|v| !matches!(v.shows, crate::view::Shows::Pending { .. }))
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(s.cached_panes().is_empty());
        assert_eq!(cache_of(&out), Some(0));
        // It resumes after what it holds.
        let sub = requests(&out, "session.subscribe")[0].clone();
        assert_eq!(sub.params["from"], 10);
        assert_eq!(sub.params["after"], json!(serial_in(FILE, 9)));
        assert_eq!(sub.params["epoch"], EPOCH);
        // What came while it was closed goes below, and is not read aloud.
        s.input(Input::ReadAloud(true));
        let out = s.incoming(response(
            sub.id,
            json!({"subscription_id": 2, "delivery": outline_delivery(
                false, slots(10..11), vec![content(10)], Some(10), EPOCH)}),
        ));
        assert_eq!(held_indexes(&s), (0..11).collect::<Vec<_>>());
        assert!(requests(&out, "speech.summarize").is_empty(), "{out:?}");
    }

    #[test]
    fn a_fourth_closed_view_drops_the_least_recent() {
        let mut s = online();
        for pane in ["a", "b", "c", "d"] {
            s.input(Input::OpenAgent(pane.into()));
            s.input(Input::CloseAgent);
        }
        assert_eq!(s.cached_panes(), ["b", "c", "d"]);
        // Opening another moves the open one into the cache too.
        s.input(Input::OpenAgent("b".into()));
        s.input(Input::OpenAgent("e".into()));
        assert_eq!(s.cached_panes(), ["c", "d", "b"]);
        // One dropped opens as new: loading.
        let out = s.input(Input::OpenAgent("a".into()));
        assert!(matches!(
            last_view(&out),
            Some(Event::AgentView { loading: true, .. })
        ));
    }

    #[test]
    fn a_reopened_view_merges_a_new_epoch_by_serial() {
        let mut s = held_outline();
        s.input(Input::CloseAgent);
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let sub = requests(&out, "session.subscribe")[0].id.clone();
        // A reset of another epoch: the same serials, one more, no
        // contents along.
        s.incoming(response(
            sub,
            json!({"subscription_id": 2, "delivery": outline_delivery(
                true, slots(0..11), vec![], Some(10), EPOCH + 1)}),
        ));
        let r = ribbon(&s);
        assert_eq!(r.len(), 11);
        assert_eq!(
            held_indexes(&s),
            (0..10).collect::<Vec<_>>(),
            "kept by serial"
        );
    }

    #[test]
    fn clearing_the_cache_empties_it() {
        let mut s = held_outline();
        s.input(Input::CloseAgent);
        assert!(s.cache_bytes() > 0);
        let out = s.input(Input::ClearCache);
        assert_eq!(cache_of(&out), Some(0));
        assert!(s.cached_panes().is_empty());
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        assert!(matches!(
            last_view(&out),
            Some(Event::AgentView { loading: true, .. })
        ));
    }

    /// The view of w2:pZ, an outline of 1000 held around 500 (from 400 to
    /// 620) and at its end, with three views closed before it: "a", "b"
    /// and "c", the least recent first.
    fn under_pressure() -> State {
        let mut s = online();
        for pane in ["a", "b", "c"] {
            s.input(Input::OpenAgent(pane.into()));
            s.input(Input::CloseAgent);
        }
        s.input(Input::Tick(secs(100.0)));
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let sub = requests(&out, "session.subscribe")[0].id.clone();
        let inline = (992..1000).map(content).collect();
        let out = s.incoming(response(
            sub,
            json!({"subscription_id": 1, "delivery": outline_delivery(
                true, slots(0..1000), inline, Some(999), EPOCH)}),
        ));
        for r in requests(&out, "session.content") {
            answer_content(&mut s, r);
        }
        let out = s.input(window_closely(
            (500, 510),
            (480, 530),
            (400, 620),
            (0, 999),
            true,
        ));
        for r in requests(&out, "session.content") {
            let out = answer_content(&mut s, r);
            for r in requests(&out, "session.content") {
                answer_content(&mut s, r);
            }
        }
        let held = held_indexes(&s);
        assert!((400..=620).all(|i| held.contains(&i)), "{held:?}");
        assert!(held.contains(&999));
        s
    }

    #[test]
    fn each_memory_grade_drops_the_least_recent_and_keeps_its_range() {
        let mut s = under_pressure();
        assert_eq!(s.cached_panes(), ["a", "b", "c"]);
        s.input(Input::Memory(Pressure::UiHidden));
        assert_eq!(s.cached_panes(), ["c"], "the most recent stays");
        let held = held_indexes(&s);
        assert!(held.iter().all(|i| (400..=620).contains(i)), "{held:?}");
        assert!(held.contains(&400) && held.contains(&620));
        s.input(Input::Memory(Pressure::Background));
        assert!(s.cached_panes().is_empty());
        assert_eq!(s.cache_bytes(), 0);
        assert_eq!(held_indexes(&s), (480..=530).collect::<Vec<_>>());
    }

    #[test]
    fn low_memory_leaves_the_screen_and_the_outline_and_scrolling_fetches_again() {
        let mut s = under_pressure();
        s.input(Input::Memory(Pressure::Low));
        assert!(s.cached_panes().is_empty());
        assert_eq!(held_indexes(&s), (500..=510).collect::<Vec<_>>());
        assert_eq!(ribbon(&s).len(), 1000, "the outline is kept");
        // Scrolled on: only what is on screen is fetched.
        let out = s.input(window_closely(
            (100, 110),
            (90, 120),
            (50, 160),
            (0, 999),
            true,
        ));
        assert_eq!(content_asks(&out), [(100, 110, false)]);
        answer_content(&mut s, requests(&out, "session.content")[0]);
        assert_eq!(held_indexes(&s), (100..=110).collect::<Vec<_>>());
        // Back in front: contents are fetched as usual again.
        s.input(Input::Background(true));
        let out = s.input(Input::Background(false));
        assert_eq!(content_asks(&out), [(50, 160, false)]);
    }

    #[test]
    fn a_refresh_pings_resubscribes_and_fetches_the_newest_screen() {
        let mut s = held_outline();
        s.input(window((5, 9), (0, 9), (0, 9), false));
        let out = s.input(Input::Refresh);
        assert_eq!(requests(&out, "ping").len(), 1);
        assert_eq!(requests(&out, "session.unsubscribe").len(), 1);
        let sub = requests(&out, "session.subscribe")[0].clone();
        assert_eq!(sub.params["from"], 10);
        assert_eq!(sub.params["epoch"], EPOCH);
        assert_eq!(refreshed(&out), None);
        // The ping unanswered for 3 s closes the link.
        let out = s.input(Input::Tick(secs(103.0)));
        assert!(out.iter().any(|o| matches!(o, Output::Close(_))), "{out:?}");
        let mut s = held_outline();
        s.input(window((5, 9), (0, 9), (0, 9), false));
        let out = s.input(Input::Refresh);
        let sub = requests(&out, "session.subscribe")[0].id.clone();
        // Two new sections come with the resubscribe.
        let out = s.incoming(response(
            sub,
            json!({"subscription_id": 2, "delivery": outline_delivery(
                false, slots(10..12), vec![content(10)], Some(11), EPOCH)}),
        ));
        // The newest screen's contents, held or not.
        let asks = content_asks(&out);
        assert!(asks.contains(&(5, 11, false)), "{asks:?}");
        assert_eq!(refreshed(&out), None);
        let r = requests(&out, "session.content")
            .into_iter()
            .find(|r| r.params["from"] == 5)
            .unwrap()
            .clone();
        let out = answer_content(&mut s, &r);
        assert_eq!(refreshed(&out), Some(2));
    }

    #[test]
    fn a_refresh_offline_connects_now_and_gives_up_after_its_wait() {
        let mut s = held_outline();
        s.disconnected(30_000);
        let out = s.input(Input::Refresh);
        assert!(out.contains(&Output::Reconnect), "{out:?}");
        assert_eq!(refreshed(&s.input(Input::Tick(secs(104.0)))), None);
        assert_eq!(refreshed(&s.input(Input::Tick(secs(105.0)))), Some(0));
        // Without a view: done at once.
        let mut s = online();
        assert_eq!(refreshed(&s.input(Input::Refresh)), Some(0));
    }

    fn finished_spoken(line: u32, spoken: &str) -> Value {
        json!({"serial": serial(line, 194), "kind": "card", "card": {
            "card": "finished", "live": true, "title": "Finished", "text": "Long.",
            "points": ["One.", "Two."], "spoken": spoken}})
    }

    #[test]
    fn read_aloud_uses_the_cards_spoken_text_and_asks_nothing() {
        let mut s = viewing("working", asking());
        s.input(Input::ReadAloud(true));
        s.input(Input::Tick(secs(200.0)));
        let out = deliver(
            &mut s,
            vec![
                text_at(5, 0, "A long reply, with `code` and paths."),
                finished_spoken(5, "All done."),
            ],
            vec![],
        );
        assert!(requests(&out, "speech.summarize").is_empty(), "{out:?}");
        assert_eq!(said(&out), ["All done."]);
        // A reply whose card comes after it: the card's text is read, and
        // the daemon's later answer adds nothing.
        let out = deliver(&mut s, vec![text_at(7, 0, "Another reply.")], vec![]);
        let asked = requests(&out, "speech.summarize")[0].clone();
        assert!(said(&out).is_empty());
        let out = deliver(&mut s, vec![finished_spoken(7, "Also done.")], vec![]);
        assert_eq!(said(&out), ["Also done."]);
        let out = s.incoming(response(asked.id, json!({"read": "Another reply."})));
        assert!(said(&out).is_empty(), "{out:?}");
    }

    #[test]
    fn the_cards_points_and_spoken_text_reach_the_view() {
        let mut s = viewing("idle", asking());
        let out = deliver(&mut s, vec![finished_spoken(5, "All done.")], vec![]);
        let (_, cards, _) = view_of(&out).unwrap();
        let card = cards
            .iter()
            .find(|c| c.kind == common::CardKind::Finished)
            .unwrap();
        assert_eq!(card.points, ["One.", "Two."]);
        assert_eq!(card.spoken.as_deref(), Some("All done."));
        assert!(card.is_summary());
    }

    #[test]
    fn bookends_come_from_the_outline_with_their_summary_and_time() {
        let mut s = online();
        s.set_local_time(LocalTime {
            hour: 12,
            minute: 0,
            second: 0,
            utc_offset_minutes: -240,
        });
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let sub = requests(&out, "session.subscribe")[0].id.clone();
        let card_serial = common::Serial::new(FILE, 2, 194).to_string();
        let slots = json!([
            {"index": 0, "serial": serial(2, 0), "kind": "text", "lines": 1, "chars": 9},
            {"index": 1, "serial": card_serial, "kind": "card", "lines": 1, "chars": 9,
             "bookend": true},
            {"index": 2, "serial": serial(3, 0), "kind": "text", "lines": 1, "chars": 9},
            {"index": 3, "serial": serial(3, 194), "kind": "card", "lines": 1, "chars": 9,
             "bookend": true},
        ]);
        let reply = json!({"serial": serial(2, 0), "kind": "text", "entries": [
            {"ts": "2026-10-24T16:41:05.120Z", "role": "assistant", "kind": "text",
             "summary": "Landed it."}]});
        let mut card = finished_spoken(2, "Landed the settings screen.\nMore.");
        card["serial"] = json!(card_serial);
        let delivery = json!({"subscription_id": 1, "pane_id": "w2:pZ", "agent": "claude",
            "reset": true, "session": {"session_id": "s1", "matched": "id"},
            "reason": null, "status": "idle", "sections": [reply, card], "slots": slots,
            "last": 3, "epoch": EPOCH});
        let out = s.incoming(response(
            sub,
            json!({"subscription_id": 1, "delivery": delivery}),
        ));
        let Some(Event::AgentView { bookends, .. }) = last_view(&out) else {
            panic!("{out:?}");
        };
        assert_eq!(
            bookends,
            [
                crate::view::Bookend {
                    serial: common::Serial::new(FILE, 2, 194),
                    summary: Some("Landed the settings screen.".into()),
                    time: Some("12:41".into()),
                },
                crate::view::Bookend {
                    serial: common::Serial::new(FILE, 3, 194),
                    summary: None,
                    time: None,
                },
            ]
        );
    }

    #[test]
    fn replay_says_the_text_again_only_with_sound() {
        let mut s = online();
        assert_eq!(
            said(&s.input(Input::Replay("All done.".into()))),
            ["All done."]
        );
        s.input(settings(false, true, true, true));
        assert!(said(&s.input(Input::Replay("All done.".into()))).is_empty());
    }

    // ── Other sessions' cards (plan 20261022_03, the L3 amendments of
    // 2026-10-24 15:56, 16:00 and 16:01 EDT) ─────────────────────────────

    /// An agent in `pane` with its own session `session`, and `turn`.
    fn in_session(pane: &str, session: &str, turn: Value) -> Value {
        let mut a = agent_info(pane, "idle");
        a["session"] = json!({"session_id": session, "matched": "id"});
        if !turn.is_null() {
            a["turn"] = turn;
        }
        a
    }

    fn asks(q: &str) -> Value {
        json!({"state": "asking", "question": q})
    }

    /// The last cards given, if any were.
    fn cards_of(out: &[Output]) -> Option<Vec<crate::cross::SessionCard>> {
        events(out).into_iter().rev().find_map(|e| match e {
            Event::SessionCards(c) => Some(c),
            _ => None,
        })
    }

    /// On the overview at 0 s, `w2:pA` (session sA) opened and closed:
    /// used. `w2:pB` (sB) is listed but never used.
    fn used_a() -> State {
        let mut s = overview();
        let out = s.input(Input::OpenAgent("w2:pA".into()));
        answer_list(
            &mut s,
            &out,
            json!([
                in_session("w2:pA", "sA", working()),
                in_session("w2:pB", "sB", working())
            ]),
        );
        s.input(Input::CloseAgent);
        s
    }

    /// `w2:pB` opened and closed too.
    fn use_b(s: &mut State) {
        let out = s.input(Input::OpenAgent("w2:pB".into()));
        answer_list(
            s,
            &out,
            json!([
                in_session("w2:pA", "sA", working()),
                in_session("w2:pB", "sB", working())
            ]),
        );
        s.input(Input::CloseAgent);
    }

    /// Lists at `t` s, answered with `agents`.
    fn list_at(s: &mut State, t: f64, agents: Value) -> Vec<Output> {
        let mut out = s.input(Input::Tick(secs(t)));
        assert_eq!(
            requests(&out, "agents.list").len(),
            1,
            "a list at {t} s: {out:?}"
        );
        let answered = answer_list(s, &out, agents);
        out.extend(answered);
        out
    }

    #[test]
    fn interactions_are_kept_for_each_session() {
        let mut s = overview();
        assert_eq!(s.last_interaction("w2:pZ"), None);
        s.input(Input::Tick(secs(3.0)));
        s.input(Input::OpenAgent("w2:pZ".into()));
        assert_eq!(s.last_interaction("w2:pZ"), Some(secs(3.0)));
        s.input(Input::Tick(secs(4.0)));
        s.input(Input::Respond(Suggestion::Continue));
        assert_eq!(s.last_interaction("w2:pZ"), Some(secs(4.0)));
        s.input(Input::CloseAgent);
        // A card answered.
        let mut s = overview();
        s.input(Input::Tick(secs(6.0)));
        s.incoming(confirm_requested(7));
        s.input(Input::Answer {
            confirm_id: 7,
            approve: true,
        });
        assert_eq!(s.last_interaction("w2:pZ"), Some(secs(6.0)));
    }

    #[test]
    fn a_question_from_a_session_used_within_the_hour_shows_a_card_in_its_colour() {
        let mut s = used_a();
        let out = list_at(
            &mut s,
            15.0,
            json!([
                in_session("w2:pA", "sA", asks("Ship it?")),
                in_session("w2:pB", "sB", asks("Merge?"))
            ]),
        );
        let cards = cards_of(&out).expect("cards");
        // Only the session used: pB was never opened.
        assert_eq!(cards.len(), 1, "{cards:?}");
        let c = &cards[0];
        assert_eq!(c.pane_id, "w2:pA");
        assert_eq!(c.kind, crate::cross::SessionCardKind::Question);
        assert_eq!(c.colour, crate::cross::session_colour("sA"));
        assert_eq!(
            (c.agent.as_str(), c.text.as_str()),
            ("the flankr agent", "Ship it?")
        );
        assert_eq!(c.at, secs(15.0));
        assert_eq!(dings(&out), 1);
        // Listed again, the same: no new card and no ding.
        let out = list_at(
            &mut s,
            30.0,
            json!([
                in_session("w2:pA", "sA", asks("Ship it?")),
                in_session("w2:pB", "sB", asks("Merge?"))
            ]),
        );
        assert!(cards_of(&out).is_none());
        assert_eq!(dings(&out), 0);
        // The ding follows the settings.
        let mut s = used_a();
        s.input(settings(true, false, true, true));
        let out = list_at(
            &mut s,
            15.0,
            json!([in_session("w2:pA", "sA", asks("Ship it?"))]),
        );
        assert!(cards_of(&out).is_some());
        assert_eq!(dings(&out), 0);
    }

    #[test]
    fn a_session_used_over_an_hour_ago_gives_no_card() {
        let mut s = used_a();
        let mut t = 15.0;
        while t < 3600.0 {
            list_at(&mut s, t, json!([in_session("w2:pA", "sA", working())]));
            t += 15.0;
        }
        let out = list_at(
            &mut s,
            3615.0,
            json!([in_session("w2:pA", "sA", asks("Ship it?"))]),
        );
        assert!(cards_of(&out).is_none_or(|c| c.is_empty()), "{out:?}");
    }

    #[test]
    fn in_another_sessions_view_the_list_is_watched_and_its_own_question_gives_none() {
        let mut s = used_a();
        let out = s.input(Input::OpenAgent("w2:pB".into()));
        answer_list(
            &mut s,
            &out,
            json!([
                in_session("w2:pA", "sA", working()),
                in_session("w2:pB", "sB", working())
            ]),
        );
        // pA was used: listed every 15 s while pB's view is open.
        let out = list_at(
            &mut s,
            15.0,
            json!([
                in_session("w2:pA", "sA", asks("Ship it?")),
                in_session("w2:pB", "sB", asks("Merge?"))
            ]),
        );
        let cards = cards_of(&out).expect("cards");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].pane_id, "w2:pA");
    }

    #[test]
    fn a_tap_opens_the_session_and_a_swipe_dismisses_its_card() {
        let ask = json!([in_session("w2:pA", "sA", asks("Ship it?"))]);
        // The tap: the app opens the view; its card goes.
        let mut s = used_a();
        list_at(&mut s, 15.0, ask.clone());
        let out = s.input(Input::OpenAgent("w2:pA".into()));
        assert_eq!(requests(&out, "session.subscribe").len(), 1);
        assert_eq!(cards_of(&out), Some(vec![]));
        // The swipe: gone, and not back while the same is asked.
        let mut s = used_a();
        list_at(&mut s, 15.0, ask.clone());
        let out = s.input(Input::DismissCard("w2:pA".into()));
        assert_eq!(cards_of(&out), Some(vec![]));
        let out = list_at(&mut s, 30.0, ask.clone());
        assert!(cards_of(&out).is_none());
        // A new question comes again.
        let out = list_at(
            &mut s,
            45.0,
            json!([in_session("w2:pA", "sA", asks("And now?"))]),
        );
        assert_eq!(cards_of(&out).expect("cards")[0].text, "And now?");
    }

    #[test]
    fn several_cards_come_newest_first_for_the_plus_n() {
        let mut s = used_a();
        use_b(&mut s);
        list_at(
            &mut s,
            15.0,
            json!([
                in_session("w2:pA", "sA", asks("One?")),
                in_session("w2:pB", "sB", working())
            ]),
        );
        let out = list_at(
            &mut s,
            30.0,
            json!([
                in_session("w2:pA", "sA", asks("One?")),
                in_session("w2:pB", "sB", asks("Two?"))
            ]),
        );
        let cards = cards_of(&out).expect("cards");
        assert_eq!(
            cards.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            ["Two?", "One?"]
        );
    }

    #[test]
    fn an_approval_for_a_used_session_shows_as_needing_approval() {
        let mut s = overview();
        s.input(Input::OpenAgent("w2:pZ".into()));
        s.input(Input::CloseAgent);
        let out = s.incoming(confirm_requested(7));
        let cards = cards_of(&out).expect("cards");
        assert_eq!(cards[0].kind, crate::cross::SessionCardKind::Approval);
        assert_eq!(cards[0].text, "continue");
    }

    /// The `session.peek` in `out`, answered with a turn from `from` to
    /// `to` (times of day on 2026-10-24, UTC).
    fn answer_peek(s: &mut State, out: &[Output], from: &str, to: &str) -> Vec<Output> {
        let peek = requests(out, "session.peek");
        assert_eq!(peek.len(), 1, "{out:?}");
        assert_eq!(peek[0].params["pane_id"], "w2:pA");
        let session = json!({
            "session_id": "sA", "matched": "id",
            "entries": [
                {"ts": "2026-10-24T09:00:00Z", "role": "user", "kind": "text", "summary": "old"},
                {"ts": format!("2026-10-24T{from}Z"), "role": "user", "kind": "text",
                 "summary": "build it"},
                {"ts": format!("2026-10-24T{to}Z"), "role": "assistant", "kind": "text",
                 "summary": "Done."}
            ]
        });
        let id = peek[0].id.clone();
        s.incoming(response(id, json!({"session": session, "status": "idle"})))
    }

    #[test]
    fn a_long_turn_finished_gives_a_one_line_card_that_hides_after_10_s() {
        let mut s = used_a();
        let out = list_at(
            &mut s,
            15.0,
            json!([in_session("w2:pA", "sA", stopped("Done."))]),
        );
        let out = answer_peek(&mut s, &out, "10:00:00", "10:03:00");
        let cards = cards_of(&out).expect("a finished card");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, crate::cross::SessionCardKind::Finished);
        assert_eq!(cards[0].text, "Done.");
        assert_eq!(cards[0].colour, crate::cross::session_colour("sA"));
        assert!(cards_of(&s.input(Input::Tick(secs(24.9)))).is_none());
        assert_eq!(cards_of(&s.input(Input::Tick(secs(25.0)))), Some(vec![]));
    }

    #[test]
    fn a_turn_under_3_minutes_gives_no_card() {
        let mut s = used_a();
        let out = list_at(
            &mut s,
            15.0,
            json!([in_session("w2:pA", "sA", stopped("Done."))]),
        );
        let out = answer_peek(&mut s, &out, "10:00:00", "10:02:59");
        assert!(cards_of(&out).is_none(), "{out:?}");
    }

    #[test]
    fn a_question_takes_the_place_of_a_finished_card() {
        let mut s = used_a();
        use_b(&mut s);
        let out = list_at(
            &mut s,
            15.0,
            json!([
                in_session("w2:pA", "sA", stopped("Done.")),
                in_session("w2:pB", "sB", working())
            ]),
        );
        let out = answer_peek(&mut s, &out, "10:00:00", "10:05:00");
        assert_eq!(cards_of(&out).expect("finished").len(), 1);
        s.input(Input::Tick(secs(20.0)));
        let out = list_at(
            &mut s,
            30.0,
            json!([
                in_session("w2:pA", "sA", stopped("Done.")),
                in_session("w2:pB", "sB", asks("Merge?"))
            ]),
        );
        let cards = cards_of(&out).expect("cards");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, crate::cross::SessionCardKind::Question);
    }

    #[test]
    fn continue_by_voice_keeps_the_typed_text_and_arms() {
        // The message card.
        let mut s = online();
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        s.input(Input::StartTextPrompt);
        s.input(Input::SetText("Please run".into()));
        let out = s.input(Input::ContinueByVoice);
        let d = dictation_of(&out).unwrap();
        assert_eq!(d.mode, PromptMode::Voice);
        assert!(!d.general_agent);
        assert_eq!(d.text, "Please run");
        assert!(events(&out).contains(&Event::Arm));
        // What is said comes after it.
        let out = s.input(Input::Utterance("the tests".into()));
        assert_eq!(dictation_of(&out).unwrap().text, "Please run the tests");
        // Already by voice: nothing.
        assert!(s.input(Input::ContinueByVoice).is_empty());
        // Gemini's card.
        let mut s = online();
        s.input(Input::StartGeneralAgentPrompt(PromptMode::Keyboard));
        s.input(Input::SetText("explain".into()));
        let out = s.input(Input::ContinueByVoice);
        let d = dictation_of(&out).unwrap();
        assert!(d.general_agent);
        assert_eq!(d.mode, PromptMode::Voice);
        assert_eq!(d.text, "explain");
        assert!(events(&out).contains(&Event::Arm));
        // The microphone off: the typed prompt stays typed.
        let mut s = online();
        s.input(settings(true, true, false, true));
        s.input(Input::StartTextPrompt);
        assert!(s.input(Input::ContinueByVoice).is_empty());
        // No prompt open: nothing.
        let mut s = online();
        assert!(s.input(Input::ContinueByVoice).is_empty());
    }

    #[test]
    fn general_agent_text_prompt_submits_to_turn() {
        let mut s = online();
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Keyboard));
        let d = dictation_of(&out).expect("dictation card");
        assert!(d.active);
        assert!(d.general_agent);
        assert_eq!(d.mode, PromptMode::Keyboard);
        s.input(Input::SetText("explain the architecture".into()));
        let out = s.input(Input::Submit);
        // The dictation card goes away.
        assert_eq!(dictation_of(&out), Some(Dictation::off()));
        // A turn is started.
        assert!(events(&out).contains(&Event::Conversation {
            speaker: Speaker::User,
            text: "explain the architecture".into(),
        }));
        let turn = requests(&out, wire::methods::TURN_START);
        assert_eq!(turn.len(), 1);
        assert_eq!(turn[0].params["text"], "explain the architecture");
    }

    #[test]
    fn general_agent_voice_prompt_short_silence_submits() {
        let mut s = online();
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Voice));
        assert!(events(&out).contains(&Event::Arm));
        let d = dictation_of(&out).expect("dictation card");
        assert!(d.active);
        assert!(d.general_agent);

        // Speak from t=10.0 to t=12.0.
        speak(&mut s, 10.0, 12.0, "what is handlr");
        // At 12.9 s (0.9 s of silence), not submitted yet.
        assert!(requests(&s.input(Input::Tick(secs(12.9))), wire::methods::TURN_START).is_empty());
        // At 13.0 s (1.0 s of silence = GENERAL_AGENT_SILENCE), submitted.
        let out = s.input(Input::Tick(secs(13.0)));
        let turn = requests(&out, wire::methods::TURN_START);
        assert_eq!(turn.len(), 1);
        assert_eq!(turn[0].params["text"], "what is handlr");
    }

    #[test]
    fn general_agent_prompt_clear() {
        let mut s = online();
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Keyboard));
        assert!(dictation_of(&out).unwrap().active);
        let out = s.input(Input::Clear);
        assert_eq!(dictation_of(&out), Some(Dictation::off()));
    }

    #[test]
    fn general_agent_prompt_reactivates_cleanly() {
        let mut s = online();
        // First turn starts:
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Voice));
        assert!(events(&out).contains(&Event::Arm));
        assert!(dictation_of(&out).unwrap().active);

        // Turn cancelled or cleared:
        let out = s.input(Input::Clear);
        assert_eq!(dictation_of(&out), Some(Dictation::off()));

        // Second turn activates immediately without blockage:
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Voice));
        assert!(events(&out).contains(&Event::Arm));
        assert!(dictation_of(&out).unwrap().active);

        // Repeated press also restarts voice prompt cleanly:
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Voice));
        assert!(events(&out).contains(&Event::Arm));
        assert!(dictation_of(&out).unwrap().active);
    }

    // Nothing is spoken over voice input (plan 20261023_02, the
    // 2026-10-30 13:37 EDT amendment).

    fn says(out: &[Output]) -> Vec<String> {
        events(out)
            .into_iter()
            .filter_map(|e| match e {
                Event::Say(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_notice_in_the_gap_between_dictated_sentences_waits_for_the_submit() {
        let mut s = dictating();
        speak(&mut s, 11.0, 12.0, "Run the tests");
        // The gate closed after the sentence, before it is armed again.
        s.input(Input::Listening(false));
        assert!(says(&s.incoming(notice("Codex is done."))).is_empty());
        let out = speak(&mut s, 13.0, 14.0, "and fix what fails.");
        assert!(says(&out).is_empty(), "{out:?}");
        let out = s.input(Input::Submit);
        assert!(message_send(&out).is_some());
        assert_eq!(says(&out), vec!["Codex is done.".to_string()]);
    }

    #[test]
    fn held_speech_comes_out_in_order_when_the_dictation_is_cleared() {
        let mut s = dictating();
        s.incoming(notice("one"));
        s.incoming(notice("two"));
        assert_eq!(s.held_notices(), 2);
        assert_eq!(
            says(&s.input(Input::Clear)),
            vec!["one".to_string(), "two".to_string()]
        );
    }

    #[test]
    fn the_dictations_own_speech_is_still_said() {
        let mut s = dictating();
        speak(&mut s, 11.0, 12.0, "Run the tests.");
        s.incoming(notice("Codex is done."));
        let out = s.input(Input::Utterance("Read it back.".into()));
        assert_eq!(says(&out), vec!["Run the tests.".to_string()]);
        // The read-back ends; the gate is armed again, and the notice
        // still waits for the message.
        s.input(Input::Speaking(true));
        let out = s.input(Input::Speaking(false));
        assert!(events(&out).contains(&Event::Arm));
        assert!(says(&out).is_empty(), "{out:?}");
        assert_eq!(s.held_notices(), 1);
    }

    #[test]
    fn a_gemini_line_during_a_dictated_message_waits_for_it() {
        let mut s = online();
        s.input(Input::Tap);
        start(&mut s, "what is claude doing");
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        let out = s.input(Input::StartDictation);
        assert!(events(&out).contains(&Event::Arm));
        let out = s.incoming(note(
            "turn.say",
            json!({"turn_id": 1, "seq": 0, "text": "Claude is running the tests."}),
        ));
        // Shown, not said.
        assert!(events(&out).contains(&Event::Conversation {
            speaker: Speaker::Gemini,
            text: "Claude is running the tests.".into()
        }));
        assert!(says(&out).is_empty(), "{out:?}");
        assert_eq!(
            says(&s.input(Input::Clear)),
            vec!["Claude is running the tests.".to_string()]
        );
    }

    #[test]
    fn notices_wait_for_gemini_voice_prompt_until_it_is_submitted() {
        let mut s = online();
        s.input(Input::StartGeneralAgentPrompt(PromptMode::Voice));
        speak(&mut s, 10.0, 12.0, "what is flankr");
        assert!(says(&s.incoming(notice("Claude is done."))).is_empty());
        let out = s.input(Input::Tick(secs(13.0)));
        assert_eq!(requests(&out, wire::methods::TURN_START).len(), 1);
        assert_eq!(says(&out), vec!["Claude is done.".to_string()]);
    }

    #[test]
    fn a_typed_prompt_is_not_voice_input() {
        let mut s = online();
        s.input(Input::StartGeneralAgentPrompt(PromptMode::Keyboard));
        assert_eq!(
            says(&s.incoming(notice("Claude is done."))),
            vec!["Claude is done.".to_string()]
        );
    }

    #[test]
    fn starting_voice_input_stops_what_is_playing() {
        let mut s = online();
        s.incoming(note("focus.changed", json!({"focus": claude()})));
        s.input(Input::Speaking(true));
        let out = s.input(Input::StartDictation);
        let ev = events(&out);
        let stop = ev.iter().position(|e| *e == Event::StopSpeaking);
        let arm = ev.iter().position(|e| *e == Event::Arm);
        assert!(stop.is_some() && stop < arm, "{out:?}");
        // Not while quiet.
        let mut s = online();
        let out = s.input(Input::StartGeneralAgentPrompt(PromptMode::Voice));
        assert!(!events(&out).contains(&Event::StopSpeaking));
    }

    #[test]
    fn with_sound_off_held_speech_is_never_said() {
        let mut s = dictating();
        s.incoming(notice("one"));
        s.input(Input::Settings(Settings {
            sound: false,
            ..Settings::default()
        }));
        assert!(says(&s.input(Input::Clear)).is_empty());
        assert_eq!(s.held_notices(), 0);
    }

    // After voice input, one catch-up instead of the held speech (plan
    // 20261023_02, the 2026-10-30 13:52 EDT amendment).

    /// `dictating()` with read aloud on (spoken output).
    fn dictating_aloud() -> State {
        let mut s = dictating();
        s.input(Input::ReadAloud(true));
        s
    }

    fn catch_up_of(out: &[Output]) -> Option<Request> {
        requests(out, "speech.catch_up")
            .first()
            .map(|r| (*r).clone())
    }

    #[test]
    fn after_a_dictated_message_one_catch_up_replaces_the_held_speech() {
        let mut s = dictating_aloud();
        speak(&mut s, 11.0, 12.0, "Run the tests");
        s.incoming(notice("Codex is done."));
        s.incoming(note(
            "notice",
            json!({"id": 2, "kind": "agent_status", "pane_id": "", "text": "Two agents wait."}),
        ));
        let out = s.input(Input::Submit);
        let sent = message_send(&out).unwrap();
        let ask = catch_up_of(&out).expect("a catch-up");
        assert_eq!(
            ask.params,
            json!({
                "held": [
                    {"pane_id": "p1", "text": "Codex is done."},
                    {"text": "Two agents wait."},
                ],
                "said": {"text": sent.params["text"], "pane_id": "w2:p4"},
            })
        );
        assert_eq!(sent.params["text"], "Run the tests");
        // Held while it is made.
        assert!(says(&out).is_empty(), "{out:?}");
        assert_eq!(s.held_notices(), 2);
        let out = s.incoming(response(
            ask.id,
            json!({"summary": " Codex finished, and your message is on its way. "}),
        ));
        assert_eq!(
            says(&out),
            ["Codex finished, and your message is on its way.".to_string()]
        );
        assert_eq!(s.held_notices(), 0);
    }

    #[test]
    fn the_catch_up_waits_for_the_speaker_as_any_notice_does() {
        let mut s = dictating_aloud();
        s.incoming(notice("Codex is done."));
        let ask = catch_up_of(&s.input(Input::Clear)).unwrap();
        // Cancelled: nothing was said.
        assert!(ask.params.get("said").is_none(), "{:?}", ask.params);
        s.input(Input::Speaking(true));
        let out = s.incoming(response(ask.id, json!({"summary": "Codex is done."})));
        assert!(says(&out).is_empty());
        assert_eq!(
            says(&s.input(Input::Speaking(false))),
            ["Codex is done.".to_string()]
        );
    }

    #[test]
    fn a_reply_read_aloud_during_voice_input_is_named_by_its_pane_and_time() {
        let mut s = viewing("working", json!(null));
        s.input(Input::ReadAloud(true));
        let out = s.input(Input::StartDictation);
        assert!(events(&out).contains(&Event::Arm), "{out:?}");
        let out = deliver(&mut s, vec![text_at(50, 0, "Built it.")], vec![]);
        let asked = requests(&out, "speech.summarize")[0].clone();
        let out = s.incoming(response(
            asked.id,
            json!({"read": "Built it.", "complex": false}),
        ));
        assert!(says(&out).is_empty(), "{out:?}");
        let ask = catch_up_of(&s.input(Input::Clear)).unwrap();
        assert_eq!(
            ask.params,
            json!({"held": [{"pane_id": "w2:pZ", "ts": "50.0", "text": "Built it."}]})
        );
    }

    #[test]
    fn gemini_voice_prompt_is_what_was_said() {
        let mut s = online();
        s.input(Input::ReadAloud(true));
        s.input(Input::StartGeneralAgentPrompt(PromptMode::Voice));
        speak(&mut s, 10.0, 12.0, "what is flankr");
        s.incoming(notice("Claude is done."));
        let out = s.input(Input::Tick(secs(13.0)));
        assert_eq!(requests(&out, wire::methods::TURN_START).len(), 1);
        let ask = catch_up_of(&out).unwrap();
        assert_eq!(ask.params["said"], json!({"text": "what is flankr"}));
        assert!(says(&out).is_empty(), "{out:?}");
    }

    #[test]
    fn speech_that_comes_while_the_catch_up_is_made_waits_for_the_next() {
        let mut s = dictating_aloud();
        s.incoming(notice("one"));
        let first = catch_up_of(&s.input(Input::Clear)).unwrap();
        // Not said, and not part of this catch-up.
        assert!(says(&s.incoming(notice("two"))).is_empty());
        assert_eq!(s.held_notices(), 2);
        // Answered: the catch-up, then what came since.
        let out = s.incoming(response(first.id, json!({"summary": "One is done."})));
        assert_eq!(says(&out), ["One is done.".to_string(), "two".to_string()]);

        // A new voice input while it is made: both go into its catch-up.
        let mut s = dictating_aloud();
        s.incoming(notice("one"));
        let first = catch_up_of(&s.input(Input::Clear)).unwrap();
        s.incoming(notice("two"));
        let out = s.input(Input::StartDictation);
        assert!(events(&out).contains(&Event::Arm), "{out:?}");
        let out = s.incoming(response(first.id, json!({"summary": "One is done."})));
        assert!(says(&out).is_empty(), "{out:?}");
        let out = s.input(Input::Clear);
        let second = catch_up_of(&out).unwrap();
        assert_eq!(
            second.params["held"],
            json!([{"text": "One is done."}, {"pane_id": "p1", "text": "two"}])
        );
        // None made: the held items as they are.
        let out = s.incoming(response(second.id, json!({})));
        assert_eq!(says(&out), ["One is done.".to_string(), "two".to_string()]);
    }

    #[test]
    fn a_failed_catch_up_or_none_says_the_held_speech_as_it_is() {
        let held = || vec!["one".to_string(), "two".to_string()];
        let asked = || {
            let mut s = dictating_aloud();
            s.incoming(notice("one"));
            s.incoming(notice("two"));
            let ask = catch_up_of(&s.input(Input::Clear)).unwrap();
            (s, ask)
        };
        // No catch-up made.
        let (mut s, ask) = asked();
        assert_eq!(says(&s.incoming(response(ask.id, json!({})))), held());
        // A blank one.
        let (mut s, ask) = asked();
        assert_eq!(
            says(&s.incoming(response(ask.id, json!({"summary": "  "})))),
            held()
        );
        // Refused, as by a daemon without the method.
        let (mut s, ask) = asked();
        let out = s.incoming(Incoming::Response(Response::err(
            Some(ask.id),
            libflankr_proto::ErrorObject::new(-32601, "unknown method speech.catch_up"),
        )));
        assert_eq!(says(&out), held());
        // The link closes.
        let (mut s, _) = asked();
        assert_eq!(says(&s.disconnected(1000)), held());
    }

    #[test]
    fn with_no_answer_in_8_s_the_held_speech_is_said_as_it_is() {
        let mut s = dictating_aloud();
        s.incoming(notice("one"));
        let ask = catch_up_of(&s.input(Input::Clear)).unwrap();
        assert!(says(&s.input(Input::Tick(secs(17.9)))).is_empty());
        assert_eq!(says(&s.input(Input::Tick(secs(18.0)))), ["one".to_string()]);
        // A late answer is dropped.
        let out = s.incoming(response(ask.id, json!({"summary": "Late."})));
        assert!(says(&out).is_empty(), "{out:?}");
        assert_eq!(s.held_notices(), 0);
    }

    #[test]
    fn no_catch_up_is_asked_when_it_cannot_be_made() {
        // Read aloud off: the held speech as it is, as before.
        let mut s = dictating();
        s.incoming(notice("one"));
        let out = s.input(Input::Clear);
        assert!(catch_up_of(&out).is_none());
        assert_eq!(says(&out), ["one".to_string()]);
        // The daemon makes no summaries.
        let mut s = dictating_aloud();
        s.incoming(capabilities(true, false));
        s.incoming(notice("one"));
        let out = s.input(Input::Clear);
        assert!(catch_up_of(&out).is_none());
        assert_eq!(says(&out), ["one".to_string()]);
        // Nothing held.
        let mut s = dictating_aloud();
        let out = s.input(Input::Clear);
        assert!(catch_up_of(&out).is_none());
        // Stop reading drops what was held: nothing to catch up on.
        let mut s = dictating_aloud();
        s.incoming(notice("one"));
        s.input(Input::StopReading);
        let out = s.input(Input::Clear);
        assert!(catch_up_of(&out).is_none() && says(&out).is_empty());
        // What the voice input itself says is never held for it.
        let mut s = dictating_aloud();
        speak(&mut s, 11.0, 12.0, "Run the tests.");
        let out = s.input(Input::Utterance("Read it back.".into()));
        assert_eq!(says(&out), ["Run the tests.".to_string()]);
        assert!(catch_up_of(&s.input(Input::Clear)).is_none());
    }

    #[test]
    fn stop_reading_or_sound_off_while_the_catch_up_is_made_drops_it() {
        for stop in [
            Input::StopReading,
            Input::Settings(Settings {
                sound: false,
                ..Settings::default()
            }),
        ] {
            let mut s = dictating_aloud();
            s.incoming(notice("one"));
            let ask = catch_up_of(&s.input(Input::Clear)).unwrap();
            s.input(stop);
            assert_eq!(s.held_notices(), 0);
            let out = s.incoming(response(ask.id, json!({"summary": "One is done."})));
            assert!(says(&out).is_empty(), "{out:?}");
        }
    }

    // Plugins (docs/design/20261028_04_PLUGINS_PLAN.md, 3.10 and section 5).

    const PLANNING: &str = "dev.flankr.plugins.planning";

    fn planning_json(enabled: bool) -> Value {
        json!({"ns": PLANNING, "name": "Planning",
               "description": "Lists this project's plans.", "version": "0.1.0",
               "enabled": enabled,
               "menu": [{"id": "plans", "label": "Plans in this project",
                         "opens": "modal", "screens": "both"}]})
    }

    fn plugin_events(out: &[Output]) -> Vec<Vec<wire::PluginInfo>> {
        events(out)
            .into_iter()
            .filter_map(|e| match e {
                Event::Plugins(p) => Some(p),
                _ => None,
            })
            .collect()
    }

    fn plugin_results(out: &[Output]) -> Vec<wire::PluginResult> {
        events(out)
            .into_iter()
            .filter_map(|e| match e {
                Event::PluginResult(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    fn call(id: Option<u64>, method: &str, params: Value) -> Input {
        Input::PluginCall {
            ns: PLANNING.into(),
            id,
            method: method.into(),
            params,
            session: Some("w2:pZ".into()),
        }
    }

    #[test]
    fn hello_and_capabilities_give_the_plugins() {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(
            id,
            json!({"server": "flankr", "vocabulary": [], "plugins": [planning_json(false)]}),
        ));
        let lists = plugin_events(&out);
        assert_eq!(lists.len(), 1, "{out:?}");
        assert_eq!(lists[0][0].ns, PLANNING);
        assert!(!lists[0][0].enabled);
        assert_eq!(lists[0][0].menu[0].id, "plans");
        assert_eq!(s.plugins().len(), 1);
        // Capabilities with `plugins` replace them; without, change nothing.
        let out = s.incoming(note(
            "server.capabilities",
            json!({"plugins": [planning_json(true)]}),
        ));
        assert!(plugin_events(&out)[0][0].enabled);
        let out = s.incoming(note("server.capabilities", json!({})));
        assert!(plugin_events(&out).is_empty());
        assert!(s.plugins()[0].enabled);
        // The replay ends with them.
        assert_eq!(
            s.replay().last(),
            Some(&Event::Plugins(s.plugins().to_vec()))
        );
    }

    #[test]
    fn a_plugin_call_is_sent_and_its_result_given_once() {
        let mut s = online();
        let out = s.input(call(
            Some(7),
            "plans.show",
            json!({"name": "20261028_04_PLUGINS_PLAN"}),
        ));
        let reqs = requests(&out, "plugin.call");
        assert_eq!(reqs.len(), 1, "{out:?}");
        assert_eq!(
            reqs[0].params,
            json!({"ns": PLANNING, "id": 7, "method": "plans.show",
                   "params": {"name": "20261028_04_PLUGINS_PLAN"}, "session": "w2:pZ"})
        );
        // Accepted: nothing yet.
        let out = s.incoming(response(reqs[0].id.clone(), json!({})));
        assert!(plugin_results(&out).is_empty(), "{out:?}");
        // A notification during it.
        let out = s.incoming(note(
            "plugin.notify",
            json!({"ns": PLANNING, "method": "progress", "params": {"n": 1}}),
        ));
        assert_eq!(
            events(&out),
            [Event::PluginNotify(wire::PluginNotify {
                ns: PLANNING.into(),
                method: "progress".into(),
                params: json!({"n": 1}),
            })]
        );
        // The result.
        let result = json!({"ns": PLANNING, "id": 7, "result": {"schema": 1}});
        let out = s.incoming(note("plugin.result", result.clone()));
        let r = plugin_results(&out);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].result, Some(json!({"schema": 1})));
        // Again, or for another namespace's id: not ours.
        assert!(plugin_results(&s.incoming(note("plugin.result", result))).is_empty());
        let other = json!({"ns": "dev.flankr.plugins.test", "id": 7, "result": {}});
        assert!(plugin_results(&s.incoming(note("plugin.result", other))).is_empty());
    }

    #[test]
    fn a_plugin_call_without_an_id_is_sent_and_answers_nothing() {
        let mut s = online();
        let out = s.input(call(None, "ping", json!(null)));
        let reqs = requests(&out, "plugin.call");
        assert!(reqs[0].params.get("id").is_none());
        let refusal = Response::err(
            Some(reqs[0].id.clone()),
            libflankr_proto::ErrorObject::new(-32031, "disabled"),
        );
        let out = s.incoming(Incoming::Response(refusal));
        assert!(plugin_results(&out).is_empty(), "{out:?}");
    }

    #[test]
    fn a_refused_plugin_call_is_a_plugin_result() {
        use libflankr_proto::envelope::codes;
        // By the daemon.
        let mut s = online();
        let out = s.input(call(Some(1), "plans.list", json!({})));
        let id = requests(&out, "plugin.call")[0].id.clone();
        let refusal = Response::err(
            Some(id),
            libflankr_proto::ErrorObject::new(codes::PLUGIN_DISABLED, "not enabled"),
        );
        let r = plugin_results(&s.incoming(Incoming::Response(refusal)));
        assert_eq!((r[0].ns.as_str(), r[0].id), (PLANNING, 1));
        assert_eq!(r[0].error.as_ref().unwrap().code, codes::PLUGIN_DISABLED);
        assert!(r[0].result.is_none());
        // Here, before sending: a bad method, a bad name, too large.
        for (input, code) in [
            (call(Some(2), "Plans", json!({})), codes::INVALID_PARAMS),
            (
                Input::PluginCall {
                    ns: "planning".into(),
                    id: Some(3),
                    method: "plans.list".into(),
                    params: json!({}),
                    session: None,
                },
                codes::INVALID_PARAMS,
            ),
            (
                call(
                    Some(4),
                    "plans.list",
                    json!("x".repeat(wire::PLUGIN_MAX_BYTES)),
                ),
                codes::PLUGIN_TOO_LARGE,
            ),
        ] {
            let out = s.input(input);
            assert!(requests(&out, "plugin.call").is_empty(), "{out:?}");
            assert_eq!(plugin_results(&out)[0].error.as_ref().unwrap().code, code);
        }
        // Offline.
        let mut s = State::new("test");
        let out = s.input(call(Some(5), "plans.list", json!({})));
        assert!(sent(&out).is_empty());
        assert_eq!(
            plugin_results(&out)[0].error.as_ref().unwrap().code,
            codes::UNAVAILABLE
        );
    }

    #[test]
    fn a_closed_connection_cancels_the_plugin_calls_waiting() {
        let mut s = online();
        let out = s.input(call(Some(9), "plans.list", json!({})));
        let id = requests(&out, "plugin.call")[0].id.clone();
        s.incoming(response(id, json!({})));
        let r = plugin_results(&s.disconnected(1000));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, 9);
        assert_eq!(
            r[0].error.as_ref().unwrap().code,
            libflankr_proto::envelope::codes::CANCELLED
        );
        // Once.
        s.connected();
        assert!(plugin_results(&s.disconnected(1000)).is_empty());
    }

    #[test]
    fn set_plugin_enabled_is_sent_and_settled_by_a_plugins_event() {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        s.incoming(response(
            id,
            json!({"server": "flankr", "vocabulary": [], "plugins": [planning_json(false)]}),
        ));
        let out = s.input(Input::SetPluginEnabled {
            ns: PLANNING.into(),
            enabled: true,
        });
        let reqs = requests(&out, "plugin.set_enabled");
        assert_eq!(reqs[0].params, json!({"ns": PLANNING, "enabled": true}));
        let out = s.incoming(response(reqs[0].id.clone(), json!({"enabled": true})));
        assert!(plugin_events(&out)[0][0].enabled);
        // Refused: the error, and the list as it was.
        let out = s.input(Input::SetPluginEnabled {
            ns: PLANNING.into(),
            enabled: false,
        });
        let id = requests(&out, "plugin.set_enabled")[0].id.clone();
        let refusal = Response::err(
            Some(id),
            libflankr_proto::ErrorObject::new(-32007, "may not approve"),
        );
        let out = s.incoming(Incoming::Response(refusal));
        assert!(events(&out).contains(&Event::Error("may not approve".into())));
        assert!(plugin_events(&out)[0][0].enabled);
        // Offline: refused here.
        s.disconnected(1000);
        let out = s.input(Input::SetPluginEnabled {
            ns: PLANNING.into(),
            enabled: false,
        });
        assert!(sent(&out).is_empty());
        assert_eq!(plugin_events(&out).len(), 1);
    }

    #[test]
    fn plugin_inputs_for_a_remote_not_in_use_are_refused() {
        let refused = plugin_refusal(&call(Some(3), "plans.list", json!({}))).unwrap();
        match &refused[..] {
            [Event::PluginResult(r)] => {
                assert_eq!((r.ns.as_str(), r.id), (PLANNING, 3));
                assert_eq!(
                    r.error.as_ref().unwrap().code,
                    libflankr_proto::envelope::codes::NOT_PERMITTED
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(
            plugin_refusal(&call(None, "x", json!({})))
                .unwrap()
                .is_empty()
        );
        let toggle = Input::SetPluginEnabled {
            ns: PLANNING.into(),
            enabled: true,
        };
        assert!(matches!(
            &plugin_refusal(&toggle).unwrap()[..],
            [Event::Error(_)]
        ));
        assert!(plugin_refusal(&Input::Tap).is_none());
    }
}
