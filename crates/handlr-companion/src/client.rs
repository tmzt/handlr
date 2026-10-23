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

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use handlr_speech::LocalTime;
use libhandlr_proto::common::{self, methods, protocols};
use libhandlr_proto::{Id, Incoming, Notification, Request, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::view::{AgentCard, Cards, Facts, PendingConfirm};
use crate::wire;

/// What the app tells the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// What `handlr pair` shows: the pairing URL, the bare `handlr1…`
    /// string, or the QR code's `data:image/png;base64,` URL. The `net`
    /// thread stores the server, gives [`Event::PairingCode`] and connects;
    /// [`State`] itself cannot, and gives [`Event::PairFailed`].
    Pair(String),
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
    /// The Submit button: submit the message now.
    Submit,
    /// The Clear button: discard the message and leave the mode.
    Clear,
    /// The Scratch button: remove the last utterance.
    Scratch,
    /// Open the agent view of this pane (plan 20261022_03, step 4): its
    /// session is peeked now, and again while it is open.
    OpenAgent(String),
    /// Close the agent view.
    CloseAgent,
    /// A card's Approve or Deny: answers this confirmation, whichever is
    /// being asked.
    Answer { confirm_id: u64, approve: bool },
    /// A suggested response under the agent view: sent to the view's pane
    /// with `agents.send`, which the daemon confirms as any other.
    Respond(Suggestion),
    /// The play button: read the agent's new replies aloud, or stop. Only
    /// its prose is read, never tool calls, their output or code blocks.
    ReadAloud(bool),
    /// The play button while something is being said: stop it, and the
    /// next held reply follows.
    Skip,
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

/// The agent view's suggested responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Suggestion {
    Yes,
    No,
    Continue,
    Compact,
}

impl Suggestion {
    /// The row's short label.
    pub fn label(self) -> &'static str {
        match self {
            Suggestion::Yes => "yes",
            Suggestion::No => "no",
            Suggestion::Continue => "cont",
            Suggestion::Compact => "cmpct",
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
        }
    }
}

/// How often the `net` thread ticks while connected.
pub const TICK: Duration = Duration::from_millis(250);

/// The silence that submits a dictated message by default, and the least
/// it may be set to (plan 20261023_02, the 01:57 EDT amendment).
pub const DICTATION_SILENCE: Duration = Duration::from_secs(8);
/// The longest dictated message, in characters (decision 1).
pub const MESSAGE_CAP: usize = 4000;
/// The countdown shown before a silence submits, in seconds.
pub const COUNTDOWN_SECS: u64 = 3;
/// How long a Submit waits, after the voice ends, for what was said to
/// come back from STT.
pub const HEARD_WAIT: Duration = Duration::from_secs(3);

/// The least time between two peeks of the open agent view (plan
/// 20261022_03, step 4), and how often it is peeked while the agent works
/// or is blocked. A notice for the agent, or a change of its status, asks
/// for a peek at once, within this limit.
pub const PEEK_INTERVAL: Duration = Duration::from_secs(5);
/// How often the view is peeked while the agent is idle.
pub const PEEK_IDLE_INTERVAL: Duration = Duration::from_secs(15);
/// The entries the agent view asks for: the most `session.peek` gives.
pub const VIEW_ENTRIES: usize = common::PEEK_ENTRIES_MAX;

/// How often the agents are listed while the overview shows and nothing
/// else asked (plan 20261023_04, decision 6).
pub const LIST_INTERVAL: Duration = Duration::from_secs(15);
/// The least time between the last list sent and one asked for since (as
/// on `focus.changed`).
pub const LIST_MIN_INTERVAL: Duration = Duration::from_secs(5);

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

/// The dictation card (plan 20261023_02, decisions 3–5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictation {
    /// The mode is on. Off, the rest is empty and the card goes away.
    pub active: bool,
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
}

impl Dictation {
    fn off() -> Self {
        Dictation {
            active: false,
            text: String::new(),
            target: None,
            focus_moved: None,
            phase: DictationPhase::Collecting,
            submit_in: None,
        }
    }
}

/// What the client tells the app.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// The device's code, `handlrkey1…`, for the user to paste into
    /// `handlr pair` on `server` (show it with a Copy button). The client
    /// keeps connecting until the server knows its key.
    PairingCode {
        code: String,
        server: String,
    },
    /// The server answered `hello` with `paired: true`: the code was added.
    /// `name` is the name this device asked to be paired as.
    Paired {
        name: String,
    },
    /// A pairing input was refused, such as a string with a bad checksum.
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
    /// The agents, as `agents.list` gave them: on its first answer on each
    /// connection, then whenever the list changes (plan 20261023_04, decision 6).
    Agents(Vec<common::AgentInfo>),
    /// The panes with a handlr confirmation waiting, as
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
    /// The open agent view changed (plan 20261022_03, step 4): the agent's
    /// shown name and status, its entries (oldest first), the priority
    /// cards in flow order, and the index of the pinned one. `loading`
    /// until the session's first answer.
    AgentView {
        name: String,
        status: String,
        entries: Vec<common::Entry>,
        cards: Vec<AgentCard>,
        pinned: Option<usize>,
        loading: bool,
        /// A response sent from the view that the session has not shown
        /// yet, shown as typed input.
        typed: Option<String>,
    },
    Error(String),
}

/// Who said a line of [`Event::Conversation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    User,
    Gemini,
}

/// What the driver must do.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// Give this event to the app.
    Event(Event),
    /// Send this request to the daemon.
    Send(Request),
    /// Close the connection, for this reason, and reconnect later.
    Close(String),
}

/// Notices held at most, the oldest dropped beyond it (decision 4).
pub const NOTICE_CAP: usize = 8;

/// How long a reply read aloud waits for its spoken summary before it is
/// read as it is (plan 20261023_05, decision 5).
pub const SUMMARY_WAIT: Duration = Duration::from_secs(10);

/// How long the link may be quiet before a `ping`, and how long the ping
/// may go unanswered before the link is closed and made again.
pub const PING_IDLE: Duration = Duration::from_secs(30);
pub const PING_WAIT: Duration = Duration::from_secs(10);

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
    let handle = message_handle(target, response.label());
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

/// "Stopped the handlr agent.", "Stopped A and B.", "Stopped A, B and C."
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
    /// `session.peek` for the agent view of this pane.
    Peek(String),
    /// `agents.send` of a suggested response.
    Respond(Suggestion, String),
    /// `speech.summarize` of the reply being read with this serial.
    Summarize(u64),
    /// `client.options`, with read aloud as it was then.
    Options,
    /// `ping`, after a quiet spell.
    Ping,
}

/// A reply to read aloud, in its turn (plan 20261023_05, decisions 5 and
/// 6, as amended at 15:45 and 16:28 EDT): the daemon is asked how to say
/// it, and later replies wait for it, up to `SUMMARY_WAIT`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reading {
    serial: u64,
    /// The reply's time, which names it.
    ts: String,
    /// The reply as the view has it, read through `handlr_speech` when the
    /// daemon does not say how to read it.
    text: String,
    /// When the summary was asked for.
    asked_at: Duration,
    /// What to say, once known.
    ready: Option<String>,
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
    /// When anything last came from the daemon, and when a `ping` went
    /// unanswered so far.
    last_heard: Duration,
    ping_sent: Option<Duration>,
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
    notices: VecDeque<String>,
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
}

/// The open agent view (plan 20261022_03, step 4).
#[derive(Debug)]
struct AgentView {
    pane_id: String,
    status: String,
    entries: Vec<common::Entry>,
    pending_question: Option<common::PendingQuestion>,
    cards: Cards,
    /// A peek is on its way.
    peeking: bool,
    /// When the last peek was sent.
    peeked_at: Option<Duration>,
    /// The session has answered at least once.
    answered: bool,
    /// The time of the pending question a tapped Yes or No answered: its
    /// card goes at once, before the session shows the answer.
    replied_to: Option<String>,
    /// The response sent, until the session shows it or the agent works.
    typed: Option<String>,
    /// A notice or a status change asks for a peek as soon as allowed.
    wanted: bool,
    /// The event as last given.
    shown: Option<Event>,
    /// The newest reply already read aloud or seen before reading began:
    /// its time.
    read_upto: Option<String>,
}

impl State {
    /// A client that calls itself `client` in `hello`.
    pub fn new(client: impl Into<String>) -> Self {
        State {
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
            last_heard: Duration::ZERO,
            ping_sent: None,
            reading: VecDeque::new(),
            next_reading: 1,
            read_replies: VecDeque::new(),
            saying: Vec::new(),
            tapped: Vec::new(),
            notices: VecDeque::new(),
            now: Duration::ZERO,
            local_time: LocalTime::default(),
            focus: None,
            dictation: None,
            silence: DICTATION_SILENCE,
            shown: Dictation::off(),
            agents: Vec::new(),
            listed: false,
            listing: false,
            listed_at: None,
            list_wanted: false,
            pending_shown: Vec::new(),
            view: None,
        }
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

    /// Sets the session to continue, such as one stored from an earlier run.
    pub fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    /// Notices waiting to be said.
    pub fn held_notices(&self) -> usize {
        self.notices.len()
    }

    /// A connection was made: say `hello`.
    pub fn connected(&mut self) -> Vec<Output> {
        self.link = Link::Greeting;
        self.sent.clear();
        self.last_heard = self.now;
        self.ping_sent = None;
        let params = wire::HelloParams {
            protocol: protocols::COMPANION.to_string(),
            version: wire::VERSION,
            client: self.client.clone(),
            session_id: self.session_id.clone(),
        };
        vec![self.request(methods::HELLO, params, Sent::Hello)]
    }

    /// The connection closed or could not be made.
    pub fn disconnected(&mut self, retry_in_ms: u32) -> Vec<Output> {
        let mut out = Vec::new();
        self.link = Link::Offline;
        self.sent.clear();
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
        // The view keeps what it showed; its approvals are withdrawn.
        if let Some(v) = self.view.as_mut() {
            v.peeking = false;
            v.wanted = true;
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
            r.ready = Some(handlr_speech::read_aloud(&r.text, now));
        }
        self.read_in_order();
        self.flush_notices(&mut out);
        out
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
            Input::Submit => self.submit(true, &mut out),
            Input::Clear => self.clear_dictation(&mut out),
            Input::Scratch => self.scratch(&mut out),
            Input::OpenAgent(pane_id) => self.open_agent(pane_id, &mut out),
            Input::CloseAgent => {
                // Back on the overview: its list is refreshed.
                self.view = None;
                self.list_wanted = true;
                self.list_if_due(&mut out);
            }
            Input::Answer {
                confirm_id,
                approve,
            } => self.answer_id(confirm_id, approve, &mut out),
            Input::Respond(response) => self.respond(response, &mut out),
            Input::Skip => {
                if self.speaking {
                    out.push(event(Event::StopSpeaking));
                }
            }
            Input::ReadAloud(on) => {
                self.read_aloud = on;
                // From now: what is already in the view is not read.
                if let Some(v) = self.view.as_mut() {
                    v.read_upto = newest_reply(&v.entries);
                }
                if self.online() {
                    self.send_options(&mut out);
                }
            }
        }
        out
    }

    /// The agent view as it stands, if one is open.
    pub fn agent_view(&self) -> Option<Event> {
        let v = self.view.as_ref()?;
        let (cards, pinned) = v.cards.placed(&v.entries);
        Some(Event::AgentView {
            name: self.agent_name(&v.pane_id),
            status: v.status.clone(),
            entries: v.entries.clone(),
            cards,
            pinned,
            loading: !v.answered,
            typed: v.typed.clone(),
        })
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

    fn open_agent(&mut self, pane_id: String, out: &mut Vec<Output>) {
        if self.view.as_ref().is_some_and(|v| v.pane_id == pane_id) {
            return self.show_view(out);
        }
        let status = self
            .agents
            .iter()
            .find(|a| a.pane_id == pane_id)
            .map(|a| a.status.clone())
            .unwrap_or_default();
        self.view = Some(AgentView {
            pane_id,
            status,
            entries: Vec::new(),
            pending_question: None,
            cards: Cards::default(),
            peeking: false,
            peeked_at: None,
            answered: false,
            replied_to: None,
            typed: None,
            wanted: true,
            shown: None,
            read_upto: None,
        });
        if self.online() {
            // Names and statuses, for the view's title and its target.
            self.ask_list(out);
        }
        self.peek_if_due(out);
        self.recompute_view(out);
    }

    /// Peeks the open view's session if it is due: asked for, or its
    /// interval passed, and never within `PEEK_INTERVAL` of the last.
    fn peek_if_due(&mut self, out: &mut Vec<Output>) {
        let (online, now) = (self.online(), self.now);
        let Some(v) = self.view.as_mut() else {
            return;
        };
        if !online || v.peeking {
            return;
        }
        let interval = if matches!(v.status.as_str(), "idle" | "done") {
            PEEK_IDLE_INTERVAL
        } else {
            PEEK_INTERVAL
        };
        let due = match v.peeked_at {
            None => true,
            Some(at) => {
                let since = now.saturating_sub(at);
                since >= interval || (v.wanted && since >= PEEK_INTERVAL)
            }
        };
        if !due {
            return;
        }
        v.peeking = true;
        v.wanted = false;
        v.peeked_at = Some(now);
        let pane_id = v.pane_id.clone();
        let params = common::SessionPeekParams {
            pane_id: pane_id.clone(),
            entries: VIEW_ENTRIES,
        };
        out.push(self.request(methods::SESSION_PEEK, params, Sent::Peek(pane_id)));
    }

    /// Applies the card rules to the open view and gives it if it changed;
    /// gives the pending panes if they changed.
    fn recompute_view(&mut self, out: &mut Vec<Output>) {
        self.show_pending(out);
        let Some(v) = self.view.as_ref() else {
            return;
        };
        let name = self.agent_name(&v.pane_id);
        let confirms: Vec<PendingConfirm> = self
            .confirms
            .iter()
            .filter(|c| c.pane_id == v.pane_id)
            .map(|c| PendingConfirm {
                confirm_id: c.id,
                text: c.text.clone(),
            })
            .collect();
        let turn = self
            .agents
            .iter()
            .find(|a| a.pane_id == v.pane_id)
            .and_then(|a| a.turn.clone());
        let v = self.view.as_mut().expect("checked above");
        // A question answered from the view is no longer asked.
        let pending_question = v
            .pending_question
            .as_ref()
            .filter(|q| v.replied_to.as_deref() != Some(q.ts.as_str()));
        let facts = Facts {
            name: &name,
            status: &v.status,
            entries: &v.entries,
            pending_question,
            confirms: &confirms,
            turn: turn.as_ref(),
        };
        v.cards.update(&facts);
        self.show_view(out);
    }

    /// Gives the agent view if it changed since it was last given.
    fn show_view(&mut self, out: &mut Vec<Output>) {
        let Some(event) = self.agent_view() else {
            return;
        };
        let v = self.view.as_mut().expect("a view");
        if v.shown.as_ref() != Some(&event) {
            v.shown = Some(event.clone());
            out.push(Output::Event(event));
        }
    }

    /// A notice or a status change for `pane_id`: the open view of it
    /// wants a peek.
    fn view_wants_peek(&mut self, pane_id: &str, out: &mut Vec<Output>) {
        if let Some(v) = self.view.as_mut().filter(|v| v.pane_id == pane_id) {
            v.wanted = true;
            self.peek_if_due(out);
        }
    }

    /// Answers the confirmation `confirm_id` (a card's button), whether or
    /// not it is the one being asked.
    fn answer_id(&mut self, confirm_id: u64, approve: bool, out: &mut Vec<Output>) {
        if self.confirms.front().map(|c| c.id) == Some(confirm_id) {
            return self.answer(approve, None, out);
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
            text: d.text(),
            target: d.target.clone(),
            focus_moved,
            phase: d.phase,
            submit_in,
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
        running.then_some(d.quiet_from? + self.silence)
    }

    fn tick(&mut self, now: Duration, out: &mut Vec<Output>) {
        self.now = self.now.max(now);
        self.keep_alive(out);
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
        self.peek_if_due(out);
        self.list_if_due(out);
        if !self.reading.is_empty() {
            self.read_in_order();
            self.flush_notices(out);
        }
    }

    fn start_dictation(&mut self, out: &mut Vec<Output>) {
        if self.dictation.is_some() {
            return;
        }
        // From the agent view, the message is for that agent.
        let fixed_pane = self.view.as_ref().map(|v| v.pane_id.clone());
        let target = match &fixed_pane {
            Some(pane_id) => self.target_for(pane_id),
            None => self.focus.clone(),
        };
        self.dictation = Some(Dictating {
            fixed_pane,
            parts: Vec::new(),
            target,
            refresh_target: self.online(),
            phase: DictationPhase::Collecting,
            confirm_id: None,
            own_approval: false,
            quiet_from: None,
            read_back: ReadBack::No,
            submit_waiting: None,
        });
        if self.online() {
            self.ask_list(out);
        }
        self.show_dictation(out);
        out.push(event(Event::Arm));
    }

    fn clear_dictation(&mut self, out: &mut Vec<Output>) {
        if self.dictation.take().is_some() {
            self.show_dictation(out);
        }
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
        d.submit_waiting = None;
        // A refused submit waits for the next change before the silence
        // tries again.
        d.quiet_from = None;
        let refusal = if d.parts.is_empty() {
            Some("The message is empty.")
        } else if !online {
            Some("I'm not connected to handlr.")
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
        d.phase = DictationPhase::Submitting;
        d.own_approval = true;
        log::info!(
            "dictation: submitted by {}",
            if by_button {
                "the button"
            } else {
                "the silence"
            }
        );
        let params = serde_json::json!({
            "pane_id": target.pane_id,
            "text": text,
            "kind": common::SendKind::Message,
            "handle": message_handle(&target, &text),
        });
        out.push(self.request(methods::AGENTS_SEND, params, Sent::Message));
        self.show_dictation(out);
    }

    /// Sends a suggested response to the open view's pane. The tap is the
    /// approval: its confirmation is approved without asking.
    fn respond(&mut self, response: Suggestion, out: &mut Vec<Output>) {
        let Some(pane_id) = self.view.as_ref().map(|v| v.pane_id.clone()) else {
            return;
        };
        if !self.online() {
            out.push(event(Event::Say(
                "I'm not connected to handlr.".to_string(),
            )));
            return;
        }
        let (word, kind) = response.send();
        // A Yes or No answering the agent's question says what it answers.
        let question = self
            .view
            .as_ref()
            .and_then(|v| v.pending_question.as_ref())
            .map(|q| q.text.clone());
        let text = match (response, &question) {
            (Suggestion::Yes | Suggestion::No, Some(q)) => {
                answer_text(response == Suggestion::Yes, q)
            }
            _ => word.to_string(),
        };
        // The card goes at once, and the answer shows as typed input until
        // the session has it (owner, 2026-10-23: "tapping yes/no on a card
        // should dismiss the card on the phone side (so immediate)"; "convert
        // the responses to typed amber text with the input box").
        if let Some(v) = self.view.as_mut() {
            if matches!(response, Suggestion::Yes | Suggestion::No) {
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
            "handle": response_handle(&target, response),
        });
        if kind == common::SendKind::Message {
            params["kind"] = serde_json::json!(kind);
        }
        self.tapped.push(Tapped {
            pane_id: pane_id.clone(),
            text: text.to_string(),
            confirm_id: None,
        });
        out.push(self.request(methods::AGENTS_SEND, params, Sent::Respond(response, text)));
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
        out
    }

    fn tap(&mut self, out: &mut Vec<Output>) {
        if !self.online() {
            out.push(event(Event::Error("not connected to handlr".to_string())));
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
        if !self.online() {
            out.push(event(Event::Error("not connected to handlr".to_string())));
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
        let id = self.next_turn;
        self.next_turn += 1;
        self.turn = Some(Turn { id, tapped: false });
        out.push(event(Event::Conversation {
            speaker: Speaker::User,
            text: text.clone(),
        }));
        let params = wire::TurnStartParams { turn_id: id, text };
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
                Sent::Ping => {}
                Sent::Summarize(serial) => {
                    log::info!("speech.summarize: {message}; reading the reply here");
                    self.summarized(serial, None, out);
                }
                Sent::Peek(pane_id) => {
                    if let Some(v) = self.view.as_mut().filter(|v| v.pane_id == pane_id) {
                        v.peeking = false;
                    }
                    out.push(event(Event::Error(format!("session.peek: {message}"))));
                }
                Sent::Focus(target) => {
                    log::info!("dictation: focus {target:?}: {message}");
                    out.push(event(Event::Say(format!("I couldn't focus {target}."))));
                }
                Sent::Respond(response, text) => {
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
                        response.label(),
                        error.message
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
            Sent::Hello => match parse::<wire::HelloResult>(result) {
                Ok(hello) => {
                    self.link = Link::Online;
                    if hello.session_id.is_some() {
                        self.session_id = hello.session_id;
                    }
                    out.push(event(Event::Connected {
                        server: hello.server,
                    }));
                    out.push(event(Event::Vocabulary(hello.vocabulary)));
                    // The connection's options start off: say read aloud's.
                    self.send_options(out);
                    // herdr's focus, for the dictation mode's target.
                    self.ask_list(out);
                    // An open view is peeked again at once.
                    if let Some(v) = self.view.as_mut() {
                        v.peeked_at = None;
                    }
                    self.peek_if_due(out);
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
                    if !self.listed || list.agents != self.agents {
                        out.push(event(Event::Agents(list.agents.clone())));
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
                Ok(common::AgentsSendResult::Sent) => self.message_resolved("sent", out),
                Err(e) => {
                    out.push(event(Event::Error(format!(
                        "a bad agents.send result: {e}"
                    ))));
                    self.message_resolved("failed", out);
                }
            },
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
            Sent::Peek(pane_id) => match parse::<common::SessionPeekResult>(result) {
                Ok(peek) => self.peeked(&pane_id, peek, out),
                Err(e) => {
                    if let Some(v) = self.view.as_mut().filter(|v| v.pane_id == pane_id) {
                        v.peeking = false;
                    }
                    log::warn!("a bad session.peek result: {e}");
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
            Sent::Interrupt | Sent::ConfirmReply(_) | Sent::Options | Sent::Ping => {}
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
            Some(at) if self.now.saturating_sub(at) >= PING_WAIT => {
                log::info!(
                    "link: no answer to a ping in {} s; reconnecting",
                    PING_WAIT.as_secs()
                );
                self.ping_sent = None;
                out.push(Output::Close("no answer to a ping".to_string()));
            }
            Some(_) => {}
            None if self.link == Link::Online
                && self.now.saturating_sub(self.last_heard) >= PING_IDLE =>
            {
                self.ping_sent = Some(self.now);
                out.push(self.request(methods::PING, common::Empty {}, Sent::Ping));
            }
            None => {}
        }
    }

    /// `client.options` with read aloud as it is (plan 20261023_05, A4):
    /// the daemon may ask Gemini for summaries only while it is on.
    fn send_options(&mut self, out: &mut Vec<Output>) {
        let params = common::ClientOptionsParams {
            spoken_output: self.read_aloud,
        };
        out.push(self.request(methods::CLIENT_OPTIONS, params, Sent::Options));
    }

    /// Ask the daemon how to say a reply, and queue it to be read in its
    /// turn. Offline, it is read through `handlr_speech` here.
    fn read_reply(&mut self, pane_id: &str, ts: String, text: String, out: &mut Vec<Output>) {
        let serial = self.next_reading;
        self.next_reading += 1;
        let ready = if self.online() {
            let params = common::SpeechSummarizeParams {
                pane_id: pane_id.to_string(),
                ts: ts.clone(),
                text: text.clone(),
            };
            out.push(self.request(methods::SPEECH_SUMMARIZE, params, Sent::Summarize(serial)));
            None
        } else {
            Some(handlr_speech::read_aloud(&text, self.local_time))
        };
        self.reading.push_back(Reading {
            serial,
            ts,
            text,
            asked_at: self.now,
            ready,
        });
        while self.reading.len() > NOTICE_CAP {
            self.reading.pop_front();
        }
    }

    /// The daemon's answer for a reply being read: what to say (A3), or
    /// `None` to read it through `handlr_speech` here (no answer, or an
    /// error).
    fn summarized(&mut self, serial: u64, spoken: Option<String>, out: &mut Vec<Output>) {
        let now = self.local_time;
        if let Some(r) = self.reading.iter_mut().find(|r| r.serial == serial) {
            let spoken = spoken
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            r.ready = Some(spoken.unwrap_or_else(|| handlr_speech::read_aloud(&r.text, now)));
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
                    handlr_speech::read_aloud(&front.text, local_time)
                }
                None => break,
            };
            let ts = self.reading.pop_front().map(|r| r.ts).unwrap_or_default();
            if !text.is_empty() {
                self.read_replies.push_back((ts, text.clone()));
                while self.read_replies.len() > NOTICE_CAP {
                    self.read_replies.pop_front();
                }
                self.notices.push_back(text);
            }
        }
        while self.notices.len() > NOTICE_CAP {
            self.notices.pop_front();
        }
    }

    /// A `session.peek` answered for `pane_id`.
    fn peeked(&mut self, pane_id: &str, peek: common::SessionPeekResult, out: &mut Vec<Output>) {
        let Some(v) = self.view.as_mut().filter(|v| v.pane_id == pane_id) else {
            return; // the view closed, or moved to another agent
        };
        v.peeking = false;
        v.answered = true;
        v.status = peek.status;
        let first = v.entries.is_empty() && v.read_upto.is_none();
        match peek.session {
            Some(session) => {
                v.entries = session.entries;
                v.pending_question = session.pending_question;
            }
            None => {
                v.entries.clear();
                v.pending_question = None;
            }
        }
        // The typed response goes once the session shows it as the user's,
        // or the agent works on it.
        if let Some(t) = v.typed.as_deref() {
            let shown = v
                .entries
                .iter()
                .any(|e| e.role == common::Role::User && same_message(&e.summary, t));
            if shown || v.status == "working" {
                v.typed = None;
            }
        }
        // The replies after the last one read, read aloud in order, each
        // as the daemon says (plan 20261023_05); the first peek's backfill
        // is not.
        let newest = newest_reply(&v.entries);
        let mut replies = Vec::new();
        if self.read_aloud && !first {
            let after = match &v.read_upto {
                None => 0,
                Some(ts) => v
                    .entries
                    .iter()
                    .rposition(|e| is_reply(e) && e.ts == *ts)
                    .map_or(v.entries.len(), |i| i + 1),
            };
            replies = v.entries[after..]
                .iter()
                .filter(|e| is_reply(e))
                .map(|e| (e.ts.clone(), e.summary.clone()))
                .collect();
        }
        if newest.is_some() {
            v.read_upto = newest;
        }
        let ends_on_reply = v.status == "working" && v.entries.last().is_some_and(is_reply);
        for (ts, text) in replies {
            self.read_reply(pane_id, ts, text, out);
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
        self.recompute_view(out);
        self.flush_notices(out);
    }

    /// `agents.list` answered: the view's status, and a message target
    /// fixed to the viewed agent before its names were known.
    fn list_learned(&mut self, out: &mut Vec<Output>) {
        if let Some(v) = self.view.as_mut()
            && let Some(a) = self.agents.iter().find(|a| a.pane_id == v.pane_id)
        {
            if a.status != v.status && !v.status.is_empty() {
                v.wanted = true;
            }
            v.status = a.status.clone();
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
        self.peek_if_due(out);
        self.recompute_view(out);
    }

    fn end_turn(&mut self, id: u64, outcome: String, stopped: Vec<String>, out: &mut Vec<Output>) {
        let Some(turn) = self.turn.take_if(|t| t.id == id) else {
            log::warn!("turn {id} ended, but it is not the running turn");
            return;
        };
        if turn.tapped && !stopped.is_empty() {
            out.push(event(Event::Say(stopped_sentence(&stopped))));
        }
        out.push(event(Event::TurnEnded { outcome, stopped }));
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
                        out.push(event(Event::Say(say.text)));
                    } else {
                        log::debug!("dropping a late turn.say for turn {}", say.turn_id);
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
                    self.view_wants_peek(&n.pane_id, out);
                    self.notices.push_back(n.text);
                    while self.notices.len() > NOTICE_CAP {
                        self.notices.pop_front();
                    }
                    self.flush_notices(out);
                }
            }
            wire::methods::VOCABULARY_CHANGED => {
                if let Some(v) = parse_or_log::<wire::VocabularyChanged>(note) {
                    out.push(event(Event::Vocabulary(v.vocabulary)));
                }
            }
            wire::methods::FOCUS_CHANGED => {
                if let Some(f) = parse_or_log::<wire::FocusChanged>(note) {
                    self.set_focus(f.focus, out);
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

    /// Says the held notices, oldest first, unless speaking, listening or
    /// confirming.
    fn flush_notices(&mut self, out: &mut Vec<Output>) {
        if self.speaking || self.listening || !self.confirms.is_empty() {
            return;
        }
        for text in self.notices.drain(..) {
            if let Some((ts, _)) = self.read_replies.iter().find(|(_, t)| *t == text) {
                self.saying.push(ts.clone());
            }
            out.push(event(Event::Say(text)));
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
        self.notices.retain(|n| !texts.contains(n));
        if self.speaking && self.saying.iter().any(|t| t == ts) {
            log::info!("read aloud: the question is answered; skipping it");
            out.push(event(Event::StopSpeaking));
        }
    }

    /// The panes with a handlr confirmation waiting, each once, in the
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
        if !self.online() || self.listing || self.view.is_some() {
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
fn is_reply(e: &common::Entry) -> bool {
    e.role == common::Role::Assistant && e.kind == common::EntryKind::Text
}

/// The time of the newest reply in `entries`.
fn newest_reply(entries: &[common::Entry]) -> Option<String> {
    entries
        .iter()
        .rev()
        .find(|e| is_reply(e))
        .map(|e| e.ts.clone())
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

fn event(e: Event) -> Output {
    Output::Event(e)
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
            json!({"server": "handlr", "session_id": "companion:test", "vocabulary": ["claude"]}),
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
                "name": "Claude", "project": "handlr",
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
            json!({"protocol": "handlr.companion", "version": 1, "client": "test",
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
            json!({"server": "handlr", "session_id": "companion:test", "vocabulary": ["claude", "hr"]}),
        ));
        assert_eq!(
            events(&out),
            vec![
                Event::Connected {
                    server: "handlr".into()
                },
                Event::Vocabulary(vec!["claude".into(), "hr".into()]),
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
            libhandlr_proto::ErrorObject::new(-32011, "unsupported protocol"),
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
        let req = start(&mut s, "send continue to the handlr agent");
        let out = s.input(Input::Tap);
        assert_eq!(events(&out), vec![Event::StopSpeaking]);
        assert_eq!(sent(&out)[0].method, "turn.interrupt");
        let out = s.incoming(response(
            req.id,
            json!({
                "outcome": "interrupted",
                "ran": [{"pane_id": "w2:pZ", "command": "continue"}],
                "interrupted": [{"pane_id": "w2:pZ", "name": "handlr", "spoken": "the handlr agent"}]
            }),
        ));
        assert_eq!(
            events(&out),
            vec![
                say("Stopped the handlr agent."),
                Event::TurnEnded {
                    outcome: "interrupted".into(),
                    stopped: vec!["the handlr agent".into()]
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
                    project: Some("handlr".into()),
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
            libhandlr_proto::ErrorObject::new(-32008, "wrong nonce; 4 attempt(s) left"),
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
            events(&s.input(Input::Pair("handlr1abc".into())))[..],
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
            libhandlr_proto::ErrorObject::new(
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
        let out = s.incoming(response(id, json!({"server": "handlr", "vocabulary": []})));
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
        json!({"pane_id": pane, "name": "the handlr agent", "spoken": "the handlr agent",
               "agent": "claude", "project": "handlr", "tab_label": "17-hr",
               "tab_number": 17, "status": status, "focused": false})
    }

    fn requests<'a>(out: &'a [Output], method: &str) -> Vec<&'a Request> {
        sent(out)
            .into_iter()
            .filter(|r| r.method == method)
            .collect()
    }

    fn view_of(out: &[Output]) -> Option<(Vec<common::Entry>, Vec<AgentCard>, Option<usize>)> {
        out.iter().rev().find_map(|o| match o {
            Output::Event(Event::AgentView {
                entries,
                cards,
                pinned,
                ..
            }) => Some((entries.clone(), cards.clone(), *pinned)),
            _ => None,
        })
    }

    fn peek_answer(s: &mut State, out: &[Output], status: &str, session: Value) -> Vec<Output> {
        let peek = requests(out, "session.peek")[0].clone();
        s.incoming(response(
            peek.id,
            json!({"session": session, "status": status}),
        ))
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

    /// Online with the view of w2:pZ open and its first peek answered with
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
        peek_answer(&mut s, &out, status, session);
        s
    }

    #[test]
    fn opening_the_view_peeks_and_lists() {
        let mut s = online();
        let out = s.input(Input::OpenAgent("w2:pZ".into()));
        let peeks = requests(&out, "session.peek");
        assert_eq!(peeks.len(), 1, "{out:?}");
        assert_eq!(peeks[0].params, json!({"pane_id": "w2:pZ", "entries": 200}));
        assert_eq!(requests(&out, "agents.list").len(), 1);
        // Shown at once, empty, until the peek answers.
        let (entries, cards, pinned) = view_of(&out).expect("a view");
        assert!(entries.is_empty() && cards.is_empty() && pinned.is_none());

        let out = peek_answer(&mut s, &out, "idle", asking());
        let (entries, cards, pinned) = view_of(&out).expect("the view again");
        assert_eq!(entries.len(), 2);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, crate::view::CardKind::Question);
        assert_eq!(cards[0].at, 2);
        assert_eq!(pinned, Some(0));
        match s.agent_view() {
            Some(Event::AgentView { name, status, .. }) => {
                assert_eq!((name.as_str(), status.as_str()), ("the agent", "idle"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_view_is_named_from_agents_list_and_closes() {
        let mut s = viewing("idle", asking());
        match s.agent_view() {
            Some(Event::AgentView { name, .. }) => assert_eq!(name, "the handlr agent"),
            other => panic!("{other:?}"),
        }
        s.input(Input::CloseAgent);
        assert!(s.agent_view().is_none());
        // Closed, it is no longer peeked.
        assert!(requests(&s.input(Input::Tick(secs(200.0))), "session.peek").is_empty());
    }

    #[test]
    fn the_view_is_peeked_at_most_every_5_s() {
        let mut s = viewing("working", json!(null));
        // Working: every 5 s.
        assert!(requests(&s.input(Input::Tick(secs(104.9))), "session.peek").is_empty());
        let out = s.input(Input::Tick(secs(105.0)));
        assert_eq!(requests(&out, "session.peek").len(), 1);
        // One at a time: nothing more until it is answered.
        assert!(requests(&s.input(Input::Tick(secs(111.0))), "session.peek").is_empty());
        let out2 = peek_answer(&mut s, &out, "idle", json!(null));
        assert!(requests(&out2, "session.peek").is_empty());
        // Idle: every 15 s, from the last at 105 s.
        assert!(requests(&s.input(Input::Tick(secs(119.0))), "session.peek").is_empty());
        let out = s.input(Input::Tick(secs(120.0)));
        assert_eq!(requests(&out, "session.peek").len(), 1);
        peek_answer(&mut s, &out, "idle", json!(null));

        // A notice for the agent asks at once, within the 5 s limit.
        let notice = |s: &mut State, pane: &str| {
            s.incoming(note(
                "notice",
                json!({"id": 1, "kind": "agent_status", "pane_id": pane, "text": "x"}),
            ))
        };
        s.input(Input::Tick(secs(122.0)));
        assert!(requests(&notice(&mut s, "w2:pZ"), "session.peek").is_empty());
        let out = s.input(Input::Tick(secs(125.0)));
        assert_eq!(requests(&out, "session.peek").len(), 1, "due at 125 s");
        peek_answer(&mut s, &out, "idle", json!(null));
        s.input(Input::Tick(secs(131.0)));
        assert_eq!(requests(&notice(&mut s, "w2:pZ"), "session.peek").len(), 1);
        // Another agent's notice does not.
        let mut s = viewing("idle", json!(null));
        s.input(Input::Tick(secs(110.0)));
        assert!(requests(&notice(&mut s, "w9:p1"), "session.peek").is_empty());
    }

    #[test]
    fn a_pending_confirmation_for_the_agent_is_a_live_card_that_fades() {
        let mut s = viewing("working", json!(null));
        // confirm_requested is for w2:pZ.
        let out = s.incoming(confirm_requested(7));
        let (_, cards, pinned) = view_of(&out).expect("the view");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, crate::view::CardKind::Approval);
        assert_eq!(cards[0].confirm_id, Some(7));
        assert!(cards[0].live);
        assert_eq!(pinned, Some(0));

        let out = s.incoming(note(
            "confirm.resolved",
            json!({"confirm_id": 7, "outcome": "sent"}),
        ));
        let (_, cards, pinned) = view_of(&out).expect("the view");
        assert!(!cards[0].live, "resolved: faded");
        assert_eq!(pinned, None);
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
    fn a_question_resolves_when_the_user_answers() {
        let mut s = viewing("idle", asking());
        let out = s.input(Input::Tick(secs(115.0)));
        let answered = json!({
            "session_id": "s1", "matched": "id",
            "entries": [
                {"ts": "1", "role": "user", "kind": "text", "summary": "go"},
                {"ts": "2", "role": "assistant", "kind": "text", "summary": "Done.\n\nShall I go on?"},
                {"ts": "3", "role": "user", "kind": "text", "summary": "yes"}
            ]
        });
        let out = peek_answer(&mut s, &out, "working", answered);
        let (_, cards, pinned) = view_of(&out).expect("the view");
        assert_eq!(cards.len(), 1);
        assert!(!cards[0].live);
        assert_eq!(cards[0].at, 2, "still at its reply");
        assert_eq!(pinned, None);
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
        assert_eq!(target.name, "the handlr agent");
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
    }

    #[test]
    fn a_disconnect_keeps_the_view_and_peeks_again_on_hello() {
        let mut s = viewing("idle", asking());
        s.input(Input::Tick(secs(101.0)));
        s.disconnected(1000);
        assert!(s.agent_view().is_some());
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(id, json!({"server": "handlr"})));
        assert_eq!(requests(&out, "session.peek").len(), 1, "{out:?}");
    }

    #[test]
    fn a_suggestion_is_sent_to_the_views_pane() {
        let mut s = viewing("idle", asking());
        let out = s.input(Input::Respond(Suggestion::Continue));
        let sends = requests(&out, "agents.send");
        assert_eq!(sends.len(), 1, "{out:?}");
        // A command: no kind, so the daemon reads it as one.
        assert_eq!(
            sends[0].params,
            json!({"pane_id": "w2:pZ", "text": "continue",
                   "handle": "reply-claude-handlr-cont"})
        );
        let out = s.input(Input::Respond(Suggestion::Yes));
        assert_eq!(
            requests(&out, "agents.send")[0].params,
            json!({"pane_id": "w2:pZ", "text": "Yes, go on", "kind": "message",
                   "handle": "reply-claude-handlr-yes"})
        );
        // The tap is the approval: its question is answered, not asked.
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
            libhandlr_proto::ErrorObject::new(-32003, "not allowed"),
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
        // A later peek with a tool call, its output, and a new reply.
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
        let out = peek_answer(&mut s, &out, "idle", session);
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
        handlr_speech::read_aloud(text, NOON_EDT)
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
            json!({"server": "handlr", "session_id": "companion:test", "vocabulary": []}),
        ));
        assert_eq!(options(&out), [json!({"spoken_output": false})]);
        // Toggled: sent each time.
        let out = s.input(Input::ReadAloud(true));
        assert_eq!(options(&out), [json!({"spoken_output": true})]);
        let id = requests(&out, "client.options")[0].id.clone();
        assert!(s.incoming(response(id, json!({}))).is_empty());
        let out = s.input(Input::ReadAloud(false));
        assert_eq!(options(&out), [json!({"spoken_output": false})]);
        // Offline, nothing is sent; after the reconnect's hello, the value
        // as it is now.
        s.disconnected(1000);
        assert!(options(&s.input(Input::ReadAloud(true))).is_empty());
        let out = s.connected();
        assert!(options(&out).is_empty(), "not before hello");
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(
            id,
            json!({"server": "handlr", "session_id": "companion:test", "vocabulary": []}),
        ));
        assert_eq!(options(&out), [json!({"spoken_output": true})]);
        // A refused option is only logged.
        let id = requests(&out, "client.options")[0].id.clone();
        let refused = Response::err(
            Some(id),
            libhandlr_proto::ErrorObject::new(-32601, "no such method"),
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
    const PATHS: &str = "I changed three files:\n- src/server/speech.rs\n- src/sessions/mod.rs\n- crates/libhandlr_proto/src/common.rs";

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
        let out = peek_answer(&mut s, &out, "idle", session);
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
            libhandlr_proto::ErrorObject::new(-32015, "no summary: no key"),
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
        let out = peek_answer(&mut s, &out, "idle", asking());
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
        let out = peek_answer(&mut s, &out, "idle", Value::Null);
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
        let out = peek_answer(&mut s, &out, "idle", session);
        assert_eq!(typed(&out), Some(None));
    }

    #[test]
    fn a_refused_yes_brings_its_card_back() {
        let mut s = viewing("idle", asking());
        let out = s.input(Input::Respond(Suggestion::Yes));
        let id = requests(&out, "agents.send")[0].id.clone();
        let refusal = Response::err(
            Some(id),
            libhandlr_proto::ErrorObject::new(-32003, "not allowed"),
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
        let out = peek_answer(&mut s, &out, "idle", session);
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

    /// Online at 0 s, hello's list answered with one working agent.
    fn overview() -> State {
        let mut s = State::new("test");
        let out = s.connected();
        let id = sent(&out)[0].id.clone();
        let out = s.incoming(response(id, json!({"server": "handlr", "vocabulary": []})));
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
        peek_answer(&mut s, &out, "working", running());
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
    fn a_stopped_turn_finishes_the_view_while_herdr_says_working() {
        // Plan 20261023_04, A4: the view's session ends on a reply while
        // herdr says working; the list's turn says it stopped.
        let ended = json!({
            "session_id": "s1", "matched": "id",
            "entries": [
                {"ts": "1", "role": "user", "kind": "text", "summary": "go"},
                {"ts": "2", "role": "assistant", "kind": "text", "summary": "All 12 tests pass."}
            ]
        });
        let mut s = viewing("working", ended.clone());
        // herdr says working and the list's turn is not known: no card.
        assert!(matches!(
            s.agent_view(),
            Some(Event::AgentView { cards, .. }) if cards.is_empty()
        ));
        // The next peek, 5 s on, asks the list for the turn.
        let out = s.input(Input::Tick(secs(105.0)));
        let out = peek_answer(&mut s, &out, "working", ended.clone());
        let mut stopped = agent_info("w2:pZ", "working");
        stopped["turn"] = json!({"state": "stopped", "line": "All 12 tests pass."});
        let out = answer_list(&mut s, &out, json!([stopped]));
        let (_, cards, pinned) = view_of(&out).expect("the view again");
        assert_eq!(cards.len(), 1, "{out:?}");
        assert_eq!(cards[0].kind, crate::view::CardKind::Finished);
        assert_eq!(pinned, Some(0));
        // Known stopped: the next peeks ask no more lists.
        let out = s.input(Input::Tick(secs(110.0)));
        let out = peek_answer(&mut s, &out, "working", ended);
        assert!(requests(&out, "agents.list").is_empty());
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
}
