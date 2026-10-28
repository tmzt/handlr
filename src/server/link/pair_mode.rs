//! The daemon's pair mode: one `flankr pair` run's offer, its expiry and
//! the one request it holds (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md,
//! the amendments of 14:37, 14:40, 14:42 and 14:44 EDT).
//!
//! * `link.pair_begin` makes a fresh offer for the run and puts the daemon
//!   in pair mode, scoped to the daemon-socket connection that asked: when
//!   that connection closes, pair mode ends. A later `link.pair_begin`
//!   replaces the run.
//! * The offer lasts `timeout_secs` (60 by default, 1 to 300). When it
//!   expires the run ends: a held request is dropped (the phone is told
//!   `Expired`) and `link.pair_ended` `{"reason": "expired"}` goes to the
//!   run's connection; until that connection closes, a request with the
//!   offer is refused `Expired`.
//! * A pairing request (Noise NK) is held only with the run's current,
//!   unexpired offer, and only while nothing else is held: that is the
//!   run's one attempt. The run's connection gets `link.pair_request` with
//!   the device's name and the six pairing words; the phone shows the same
//!   words. Other requests are refused at once and do not count: a wrong
//!   offer, an expired one, a malformed request, or any request once one
//!   is held (`Busy`). With no run in pair mode, a request gets no reply.
//! * `link.pair_code` holds a pasted device code instead (the fallback),
//!   with the same words from the code's key and the run's offer.
//! * `link.pair_answer` answers the held request: on `yes` the key is
//!   added to `auth.toml` and the phone told `Added`; otherwise nothing is
//!   added and the phone is told `Refused`. Either way the run is over.
//!   The phone closing its connection before the answer (its Cancel, or a
//!   dropped connection) ends the run with `link.pair_ended`
//!   `{"reason": "cancelled"}`.
//!
//! The JSON-RPC shapes of these messages are defined here, beside
//! `libflankr_proto::common`'s `link.pair_begin` ones, until they move into
//! it.

use std::sync::{Arc, Weak};
use std::time::Duration;

use libflankr_proto::link::{
    KEY_LEN, LinkStream, OFFER_LEN, PAIR_TIMEOUT_DEFAULT_SECS, PAIR_TIMEOUT_MAX_SECS, PairAck,
    PairIncoming, PairRefusal, PairRequest, PairResult, pairing_words_text, send_pair_result,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::Link;

/// Method names of the pair-mode messages on the daemon socket.
pub mod methods {
    /// Local only: hold a pasted device code as the run's request.
    pub const LINK_PAIR_CODE: &str = "link.pair_code";
    /// Local only: the owner's answer to the held request.
    pub const LINK_PAIR_ANSWER: &str = "link.pair_answer";
    /// Notification to the run's connection: a request is held.
    pub const LINK_PAIR_REQUEST: &str = "link.pair_request";
    /// Notification to the run's connection: the run ended without an
    /// answer.
    pub const LINK_PAIR_ENDED: &str = "link.pair_ended";
}

/// How long the daemon waits to tell the phone its result.
const TELL_PHONE_TIMEOUT: Duration = Duration::from_secs(5);

/// `link.pair_begin`'s params. `{}` is the default timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PairBeginParams {
    /// How long the offer lasts, 1 to 300 seconds; 60 by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u16>,
}

/// What `link.pair_begin` adds to `libflankr_proto`'s
/// `LinkPairBeginResult` (flattened beside it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairBeginExtra {
    /// The seconds the run's offer lasts.
    pub expires_in_secs: u16,
}

/// A held request, as `link.pair_request` and `link.pair_code` report it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequestInfo {
    /// The device's name, from its request or its code.
    pub name: String,
    /// The six pairing words, space-separated.
    pub words: String,
    /// `link` for a pairing request, `code` for a pasted device code.
    pub via: String,
    /// The seconds left on the offer.
    pub expires_in_secs: u16,
}

/// `link.pair_code`'s params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCodeParams {
    /// The device code, `flankrkey1…`.
    pub code: String,
}

/// `link.pair_answer`'s params.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PairAnswerParams {
    /// The owner typed `yes`.
    pub yes: bool,
    /// Pair as this name instead of the device's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `companion`, `harness`; `companion` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocols: Option<Vec<String>>,
    /// Replace an entry of the same name.
    #[serde(default)]
    pub replace: bool,
}

/// `link.pair_answer`'s result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairAnswerResult {
    /// The key is in `auth.toml`.
    pub added: bool,
    /// The name it was added as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What it may say `hello` with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocols: Option<Vec<String>>,
}

/// `link.pair_ended`'s params.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairEnded {
    /// `expired`, `cancelled` (the phone closed) or `replaced` (another
    /// `link.pair_begin`).
    pub reason: String,
}

/// What the run's connection is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairEvent {
    Request(PairRequestInfo),
    Ended(PairEnded),
}

/// What the phone's task is asked to send: the result, and a sender for
/// when it has been sent.
type TellPhone = oneshot::Sender<(PairResult, oneshot::Sender<()>)>;

/// The request a run holds.
struct Held {
    key: [u8; KEY_LEN],
    name: String,
    /// The phone's open session; `None` for a pasted device code.
    phone: Option<TellPhone>,
}

enum RunState {
    /// No request yet.
    Waiting,
    /// The run's one attempt.
    Held(Held),
    /// Answered, expired or cancelled: the run is over.
    Finished,
}

/// A `flankr pair` run in pair mode.
pub(super) struct PairRun {
    id: u64,
    /// The daemon-socket connection that began it.
    owner: u64,
    offer: [u8; OFFER_LEN],
    expires: Instant,
    state: RunState,
    events: mpsc::UnboundedSender<PairEvent>,
    timer: tokio::task::AbortHandle,
}

impl Drop for PairRun {
    fn drop(&mut self) {
        self.timer.abort();
    }
}

impl PairRun {
    /// Ends the run, telling a held phone `result`.
    fn finish(&mut self, result: PairResult) {
        if let RunState::Held(Held {
            phone: Some(tell), ..
        }) = std::mem::replace(&mut self.state, RunState::Finished)
        {
            let (done, _) = oneshot::channel();
            let _ = tell.send((result, done));
        }
    }

    fn expires_in_secs(&self, now: Instant) -> u16 {
        let left = self.expires.saturating_duration_since(now).as_millis();
        u16::try_from(left.div_ceil(1000)).unwrap_or(u16::MAX)
    }
}

/// The timeout `link.pair_begin` was given, checked.
pub fn timeout_secs(given: Option<u16>) -> Result<u16, String> {
    match given {
        None => Ok(PAIR_TIMEOUT_DEFAULT_SECS),
        Some(secs) if (1..=PAIR_TIMEOUT_MAX_SECS).contains(&secs) => Ok(secs),
        Some(secs) => Err(format!(
            "a pairing offer lasts 1 to {PAIR_TIMEOUT_MAX_SECS} seconds, not {secs}"
        )),
    }
}

/// What the listener does with a request.
enum Decision {
    /// No run: no reply.
    Drop,
    Refuse(PairRefusal),
    Hold {
        run: u64,
        info: PairRequestInfo,
        told: oneshot::Receiver<(PairResult, oneshot::Sender<()>)>,
        expires_in_secs: u16,
    },
}

impl Link {
    /// Starts a run for the connection `owner`, replacing any other, with
    /// a fresh `offer` lasting `secs`. Returns the receiver of the run's
    /// events; it ends when the run does.
    pub(super) fn start_run(
        self: &Arc<Self>,
        owner: u64,
        offer: [u8; OFFER_LEN],
        secs: u16,
    ) -> mpsc::UnboundedReceiver<PairEvent> {
        let (events, rx) = mpsc::unbounded_channel();
        let id = self
            .next_conn
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let expires = Instant::now() + Duration::from_secs(u64::from(secs));
        let weak: Weak<Link> = Arc::downgrade(self);
        let timer = tokio::spawn(async move {
            tokio::time::sleep_until(expires).await;
            if let Some(link) = weak.upgrade() {
                link.expire(id);
            }
        })
        .abort_handle();
        let run = PairRun {
            id,
            owner,
            offer,
            expires,
            state: RunState::Waiting,
            events,
            timer,
        };
        let old = self.pair_run.lock().unwrap().replace(run);
        if let Some(mut old) = old {
            let was_over = matches!(old.state, RunState::Finished);
            old.finish(PairResult::Refused);
            if !was_over {
                let _ = old.events.send(PairEvent::Ended(PairEnded {
                    reason: "replaced".to_string(),
                }));
            }
        }
        log::info!("Pair mode for {secs} s");
        rx
    }

    /// The run's offer has expired.
    fn expire(&self, id: u64) {
        let mut guard = self.pair_run.lock().unwrap();
        let Some(run) = guard.as_mut().filter(|r| r.id == id) else {
            return;
        };
        if matches!(run.state, RunState::Finished) {
            return;
        }
        log::info!("The pairing offer expired");
        run.finish(PairResult::Expired);
        let _ = run.events.send(PairEvent::Ended(PairEnded {
            reason: "expired".to_string(),
        }));
    }

    /// The daemon-socket connection `owner` closed: its run, if any, ends.
    pub fn end_pair_mode(&self, owner: u64) {
        let mut guard = self.pair_run.lock().unwrap();
        if guard.as_ref().is_some_and(|r| r.owner == owner) {
            if let Some(mut run) = guard.take() {
                run.finish(PairResult::Refused);
            }
            log::info!("Pair mode ended");
        }
    }

    /// Whether the daemon is in pair mode.
    pub fn in_pair_mode(&self) -> bool {
        self.pair_run.lock().unwrap().is_some()
    }

    fn decide(&self, request: &Result<PairRequest, impl std::fmt::Display>) -> Decision {
        let mut guard = self.pair_run.lock().unwrap();
        let Some(run) = guard.as_mut() else {
            return Decision::Drop;
        };
        let request = match request {
            Ok(request) => request,
            Err(_) => return Decision::Refuse(PairRefusal::Malformed),
        };
        if request.offer != run.offer {
            return Decision::Refuse(PairRefusal::WrongOffer);
        }
        let now = Instant::now();
        if now >= run.expires {
            return Decision::Refuse(PairRefusal::Expired);
        }
        if !matches!(run.state, RunState::Waiting) {
            return Decision::Refuse(PairRefusal::Busy);
        }
        let (tell, told) = oneshot::channel();
        run.state = RunState::Held(Held {
            key: request.device_key,
            name: request.name.clone(),
            phone: Some(tell),
        });
        let expires_in_secs = run.expires_in_secs(now);
        Decision::Hold {
            run: run.id,
            info: PairRequestInfo {
                name: request.name.clone(),
                words: pairing_words_text(&self.keypair.public, &run.offer, &request.device_key),
                via: "link".to_string(),
                expires_in_secs,
            },
            told,
            expires_in_secs,
        }
    }

    /// Tells the run's connection of its held request.
    fn announce(&self, id: u64, info: PairRequestInfo) {
        if let Some(run) = self
            .pair_run
            .lock()
            .unwrap()
            .as_ref()
            .filter(|r| r.id == id)
        {
            let _ = run.events.send(PairEvent::Request(info));
        }
    }

    /// The phone of run `id`'s held request closed before an answer.
    fn phone_gone(&self, id: u64) {
        let mut guard = self.pair_run.lock().unwrap();
        let Some(run) = guard.as_mut().filter(|r| r.id == id) else {
            return;
        };
        if matches!(run.state, RunState::Held(_)) {
            log::info!("The phone cancelled its pairing request");
            run.state = RunState::Finished;
            let _ = run.events.send(PairEvent::Ended(PairEnded {
                reason: "cancelled".to_string(),
            }));
        }
    }

    /// A pairing request on the link listener: answer it, and hold an
    /// accepted one's session until the run answers it or the phone
    /// closes.
    pub(super) async fn pair_incoming<S>(&self, incoming: PairIncoming<S>, peer: &str)
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        match self.decide(&incoming.request) {
            Decision::Drop => {
                log::info!("Dropped a pairing request from {peer}: not in pair mode");
            }
            Decision::Refuse(reason) => {
                log::info!("Refused a pairing request from {peer}: {reason:?}");
                let _ = tokio::time::timeout(
                    super::HANDSHAKE_TIMEOUT,
                    incoming.respond(PairAck::Refused(reason)),
                )
                .await;
            }
            Decision::Hold {
                run,
                info,
                told,
                expires_in_secs,
            } => {
                log::info!("Holding a pairing request from '{}' ({peer})", info.name);
                let ack = PairAck::Accepted { expires_in_secs };
                match tokio::time::timeout(super::HANDSHAKE_TIMEOUT, incoming.respond(ack)).await {
                    Ok(Ok(stream)) => {
                        self.announce(run, info);
                        if hold_phone(stream, told).await.is_err() {
                            self.phone_gone(run);
                        }
                    }
                    _ => self.phone_gone(run),
                }
            }
        }
    }

    /// `link.pair_code`: holds a pasted device code as the run's request.
    pub fn pair_code(&self, owner: u64, code: &str) -> Result<PairRequestInfo, String> {
        let mut guard = self.pair_run.lock().unwrap();
        let run = guard
            .as_mut()
            .filter(|r| r.owner == owner)
            .ok_or("this connection is not in pair mode; call link.pair_begin")?;
        let now = Instant::now();
        if matches!(run.state, RunState::Finished) || now >= run.expires {
            return Err("this pairing has ended".to_string());
        }
        if matches!(run.state, RunState::Held(_)) {
            return Err("a pairing request is already waiting for an answer".to_string());
        }
        let code = libflankr_proto::link::DeviceCode::decode(code, &self.keypair.public)
            .map_err(super::code_error)?;
        let info = PairRequestInfo {
            name: code.name.clone(),
            words: pairing_words_text(&self.keypair.public, &run.offer, &code.key),
            via: "code".to_string(),
            expires_in_secs: run.expires_in_secs(now),
        };
        run.state = RunState::Held(Held {
            key: code.key,
            name: code.name,
            phone: None,
        });
        Ok(info)
    }

    /// `link.pair_answer`: the owner's answer to the held request. Adds
    /// the key only on `yes`, within the offer's time; the run is over
    /// either way.
    pub async fn pair_answer(
        &self,
        owner: u64,
        params: PairAnswerParams,
    ) -> Result<PairAnswerResult, String> {
        let held = {
            let mut guard = self.pair_run.lock().unwrap();
            let run = guard
                .as_mut()
                .filter(|r| r.owner == owner)
                .ok_or("this connection is not in pair mode")?;
            if !matches!(run.state, RunState::Held(_)) {
                return Err("no pairing request is waiting for an answer".to_string());
            }
            if Instant::now() >= run.expires {
                run.finish(PairResult::Expired);
                return Err("the pairing offer expired".to_string());
            }
            match std::mem::replace(&mut run.state, RunState::Finished) {
                RunState::Held(held) => held,
                _ => unreachable!("checked above"),
            }
        };
        if !params.yes {
            log::info!("Pairing '{}' refused by the owner", held.name);
            tell_phone(held.phone, PairResult::Refused).await;
            return Ok(PairAnswerResult {
                added: false,
                name: None,
                protocols: None,
            });
        }
        let name = match params.name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => held.name.clone(),
        };
        match self.add_key(&held.key, &name, params.protocols, params.replace) {
            Ok(added) => {
                tell_phone(held.phone, PairResult::Added).await;
                Ok(PairAnswerResult {
                    added: true,
                    name: Some(added.name),
                    protocols: Some(added.protocols),
                })
            }
            Err(e) => {
                tell_phone(held.phone, PairResult::Refused).await;
                Err(e)
            }
        }
    }
}

/// Sends `result` to a held phone and waits, briefly, until it is sent.
async fn tell_phone(phone: Option<TellPhone>, result: PairResult) {
    let Some(tell) = phone else { return };
    let (done, sent) = oneshot::channel();
    if tell.send((result, done)).is_ok() {
        let _ = tokio::time::timeout(TELL_PHONE_TIMEOUT, sent).await;
    }
}

/// Holds an accepted request's session: sends the result it is told to,
/// or returns `Err` when the phone closes first (or sends anything).
async fn hold_phone<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: LinkStream<S>,
    told: oneshot::Receiver<(PairResult, oneshot::Sender<()>)>,
) -> Result<(), ()> {
    tokio::select! {
        told = told => {
            if let Ok((result, done)) = told {
                let _ = tokio::time::timeout(
                    TELL_PHONE_TIMEOUT,
                    send_pair_result(&mut stream, result),
                )
                .await;
                let _ = done.send(());
            }
            Ok(())
        }
        _ = stream.recv_message() => Err(()),
    }
}
