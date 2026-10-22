//! The companion's logic as a pure state machine (plan 20261022_04,
//! decisions 2–4, as amended at 18:07 EDT).
//!
//! [`State`] has no threads, sockets or clocks. Whoever drives it (the
//! `net` thread, or a test) tells it what happened, as an [`Input`] from the
//! app, a message from the daemon, or the connection opening or closing,
//! and carries out the [`Output`]s it returns: events for the app, requests
//! for the daemon, and closing the connection.

use std::collections::{HashMap, VecDeque};

use libhandlr_proto::common::{self, methods, protocols};
use libhandlr_proto::{Id, Incoming, Notification, Request, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::wire;

/// What the app tells the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A `handlr1…` pairing string. Pairing needs the link (plan
    /// 20261022_04 step 2); until then it gives [`Event::PairFailed`].
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
    Paired {
        name: String,
    },
    PairFailed(String),
    /// Speak this.
    Say(String),
    /// Stop playback now.
    StopSpeaking,
    /// Arm the STT gate.
    Arm,
    /// A confirmation is waiting for the user's answer. Its `text` is also
    /// given as [`Event::Say`]. `phrase` is empty when the daemon sent none.
    Confirm {
        text: String,
        label: Option<String>,
        phrase: String,
    },
    /// A confirmation ended, with plan 01's outcome (`sent`, `denied` …).
    Resolved {
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
    Error(String),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sent {
    Hello,
    Turn(u64),
    Interrupt,
    ConfirmReply,
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
    text: String,
    label: Option<String>,
    phrase: String,
}

/// The client's state (decision 3). The plan's states map onto it as:
/// - `Idle`: online, no turn, no confirmation;
/// - `Waiting`: a turn is running and no confirmation is pending;
/// - `Confirming`: a confirmation is pending. It takes precedence over a
///   running turn, so the user can always answer the question just spoken.
///
/// `Unpaired` belongs to the link (step 2).
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
    notices: VecDeque<String>,
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
            notices: VecDeque::new(),
        }
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
        self.confirms.clear();
        self.deferred = None;
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
        self.flush_notices(&mut out);
        out
    }

    /// Something the app said.
    pub fn input(&mut self, input: Input) -> Vec<Output> {
        let mut out = Vec::new();
        match input {
            Input::Pair(_) => out.push(event(Event::PairFailed(
                "pairing needs the link (step 2)".to_string(),
            ))),
            Input::Speaking(on) => {
                self.speaking = on;
                self.flush_notices(&mut out);
            }
            Input::Listening(on) => {
                self.listening = on;
                self.flush_notices(&mut out);
            }
            Input::Tap => self.tap(&mut out),
            Input::Utterance(text) => self.utterance(text, &mut out),
            Input::Approve => self.answer(true, None, &mut out),
            Input::Deny => self.answer(false, None, &mut out),
        }
        out
    }

    /// A message from the daemon.
    pub fn incoming(&mut self, message: Incoming) -> Vec<Output> {
        let mut out = Vec::new();
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
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if !self.online() {
            out.push(event(Event::Error("not connected to handlr".to_string())));
        } else if !self.confirms.is_empty() {
            let approve = is_approval(&text);
            self.answer(approve, (!approve).then_some(text), out);
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
        out.push(self.request(methods::CONFIRM_REPLY, params, Sent::ConfirmReply));
        if let Some(text) = then {
            if self.turn.is_some() {
                self.deferred = Some(text);
            } else {
                self.start_turn(text, out);
            }
        }
        self.ask_next(out);
        self.flush_notices(out);
    }

    fn start_turn(&mut self, text: String, out: &mut Vec<Output>) {
        let id = self.next_turn;
        self.next_turn += 1;
        self.turn = Some(Turn { id, tapped: false });
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
                Sent::Interrupt | Sent::ConfirmReply => out.push(event(Event::Error(message))),
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
                    self.end_turn(id, r.outcome, stopped, out);
                }
                Err(e) => {
                    out.push(event(Event::Error(format!("a bad turn result: {e}"))));
                    self.end_turn(id, "error".to_string(), Vec::new(), out);
                }
            },
            Sent::Interrupt | Sent::ConfirmReply => {}
        }
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
            other => log::debug!("ignoring the notification {other}"),
        }
    }

    fn requested(&mut self, c: wire::ConfirmRequested, out: &mut Vec<Output>) {
        if self.confirms.iter().any(|known| known.id == c.confirm_id) {
            return; // a reminder of one already being handled
        }
        self.confirms.push_back(Confirmation {
            id: c.confirm_id,
            text: c.text,
            label: c.label,
            phrase: c.phrase.unwrap_or_default(),
        });
        if self.confirms.len() == 1 {
            self.ask_next(out);
        }
    }

    fn resolved(&mut self, r: common::ConfirmResolved, out: &mut Vec<Output>) {
        let was_asked = self.confirms.front().map(|c| c.id) == Some(r.confirm_id);
        self.confirms.retain(|c| c.id != r.confirm_id);
        let outcome = serde_json::to_value(r.outcome)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        out.push(event(Event::Resolved { outcome }));
        if was_asked {
            self.ask_next(out);
        }
        self.flush_notices(out);
    }

    /// Asks the confirmation now at the front, if any.
    fn ask_next(&mut self, out: &mut Vec<Output>) {
        if let Some(c) = self.confirms.front() {
            out.push(event(Event::Confirm {
                text: c.text.clone(),
                label: c.label.clone(),
                phrase: c.phrase.clone(),
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
        out.extend(self.notices.drain(..).map(|text| event(Event::Say(text))));
    }

    fn request(&mut self, method: &str, params: impl serde::Serialize, sent: Sent) -> Output {
        let id = Id::Number(self.next_request);
        self.next_request += 1;
        self.sent.insert(id.clone(), sent);
        Output::Send(Request::new(id, method, params))
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
        assert_eq!(events(&out), vec![say("Claude is running the tests.")]);
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
                    text: text.into(),
                    label: Some("keep-going".into()),
                    phrase: "copper-lantern-ninety".into()
                },
                say(text),
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
        assert_eq!(events(&out), vec![say("one")]);
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
    fn pairing_waits_for_the_link() {
        let mut s = State::new("test");
        assert!(matches!(
            events(&s.input(Input::Pair("handlr1abc".into())))[..],
            [Event::PairFailed(_)]
        ));
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
}
