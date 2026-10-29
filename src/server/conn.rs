//! One client connection: `hello`, then the common methods
//! (plan 20261022_01, decisions 4, 7 and 8).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use futures_util::{Stream, StreamExt};
use libflankr_proto::common::{self, methods, protocols, topics};
use libflankr_proto::companion;
use libflankr_proto::envelope::codes;
use libflankr_proto::link::{KEY_LEN, LinkError, LinkStream, Message};
use libflankr_proto::{ErrorObject, Id, Incoming, Notification, Request, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpStream, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::AbortHandle;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::sync::CancellationToken;

use super::companion::{self as turns, Session};
use super::link::auth::AuthEntry;
use super::link::pair_mode;
use super::subscriptions::{self, Subscription};
use super::{CoreEvent, Daemon};
use crate::core::agents::{self, extract_agent_name_for_pane};
use crate::core::confirm::{
    self, ConfirmError, ConfirmEvent, HandleRequest, Lifetime, Outcome, StatusKey,
};
use crate::core::naming::ResolveError;
use crate::core::policy;
use crate::core::principal::Principal;
use crate::notify::Announcer;

/// The longest line a client may send: 1 MiB.
pub const MAX_LINE: usize = 1 << 20;

/// The protocol versions served.
const HARNESS_VERSION: u32 = 1;
const COMPANION_VERSION: u32 = companion::VERSION;

/// Connection ids, for logs.
static NEXT_CONN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Whether a peer with `peer_uid` may connect to a daemon running as `own_uid`.
pub fn peer_allowed(peer_uid: u32, own_uid: u32) -> bool {
    peer_uid == own_uid
}

/// Where a connection comes from, which decides who it is at `hello`.
enum Origin {
    /// The daemon socket: a peer with the daemon's uid, `local:<client>`.
    Local,
    /// A link connection, with its `auth.toml` entry: `paired:<name>`.
    Link {
        entry: AuthEntry,
        /// The client's static key. Its first `hello` since `link.pair_add`
        /// added it answers `paired: true`.
        key: [u8; KEY_LEN],
    },
}

/// What a connection's writer sends: JSON-RPC lines, and on a link
/// connection terminal frames (plan 20261027_07, decision 3) and binary
/// transfer messages (plan 20261028_05, decision 4.5). Control messages go
/// out in the order they were queued; a bulk message (a CHUNK) only when
/// no control message waits.
pub(super) enum Outgoing {
    Line(String),
    Terminal(Vec<u8>),
    /// A kind-2 message; `written` fires once the writer has written it.
    /// Which queue it is on says whether it is bulk.
    Binary {
        bytes: Vec<u8>,
        written: Option<tokio::sync::oneshot::Sender<()>>,
    },
}

/// What a connection reads: JSON-RPC lines, and on a link connection
/// binary transfer messages.
pub(super) enum Inbound {
    Line(String),
    Binary(Vec<u8>),
}

pub(super) struct Conn {
    pub(super) daemon: Arc<Daemon>,
    id: u64,
    origin: Origin,
    out: mpsc::UnboundedSender<Outgoing>,
    /// The writer's bulk queue: CHUNKs (plan 20261028_05, decision 4.5).
    bulk: mpsc::UnboundedSender<Outgoing>,
    principal: OnceLock<Principal>,
    /// Set at `hello` for `flankr.companion`: its session.
    session: OnceLock<Arc<Session>>,
    topics: Mutex<HashSet<String>>,
    inflight: Mutex<HashMap<Id, AbortHandle>>,
    /// Confirmations opened on this connection, cancelled when it closes.
    opened: Mutex<HashSet<u64>>,
    /// A companion's vocabulary as last sent.
    pub(super) vocabulary: Mutex<Vec<String>>,
    /// herdr's focus as a companion last had it, by `focus.changed` or
    /// `agents.list` (plan 20261023_02, decision 4).
    pub(super) focus: Mutex<Option<common::FocusInfo>>,
    /// A companion's notices, held while a confirmation is pending.
    pub(super) notices: Mutex<Announcer<companion::Notice>>,
    pub(super) next_notice: std::sync::atomic::AtomicU64,
    /// Held while a `speech.summarize` runs: one at a time per connection
    /// (plan 20261023_05, decision 5).
    pub(super) speech: tokio::sync::Mutex<()>,
    /// A companion's `client.options` `spoken_output`: read aloud is on,
    /// so `speech.summarize` may call Gemini (plan 20261023_05, A4). Off
    /// when the connection opens.
    spoken_output: std::sync::atomic::AtomicBool,
    /// A companion's `client.options` `background`: its link sleeps, so it
    /// gets only what needs the user (owner, 2026-10-23: "only blockages or
    /// done messages should be sent on a sleeping conn").
    pub(super) background: std::sync::atomic::AtomicBool,
    /// A companion's `client.options` `general_agent`: while false, its
    /// `turn.start` is refused, so no Gemini turn starts for it; spoken
    /// summaries go on (plan 20261024_02, decision 7, as amended at
    /// 11:37 EDT). On when the connection opens.
    general_agent: std::sync::atomic::AtomicBool,
    /// A companion's `client.options` `tab_follows_focus`: the general
    /// agent's `focus_project` also moves herdr's focus (plan 20261027_04,
    /// the 11:20 and 11:25 EDT amendments). Off when the connection opens.
    tab_follows_focus: std::sync::atomic::AtomicBool,
    /// When the client last asked anything but a ping.
    last_request: Mutex<std::time::Instant>,
    /// Session subscriptions, by id (plan 20261023_03).
    subscriptions: Mutex<HashMap<u64, Arc<Subscription>>>,
    next_subscription: std::sync::atomic::AtomicU64,
    /// Answers after whose writing something starts: a subscription's
    /// task waits for its `session.subscribe` answer (4.4).
    after_answer: Mutex<HashMap<Id, tokio::sync::oneshot::Sender<()>>>,
    /// Lines queued to the connection, and lines its writer has written:
    /// a subscription's delivery has left once `written` reaches its place.
    queued: std::sync::atomic::AtomicU64,
    written: Arc<std::sync::atomic::AtomicU64>,
    /// Open terminals (plan 20261027_07, decision 8).
    pub(super) terminals: super::terminal::Terminals,
    /// Running plugin calls (plan 20261028_04, decision 3.2, item 2).
    plugin_calls: crate::plugins::CallSlots,
    /// Binary transfers (plan 20261028_05).
    pub(super) blob: super::blob::Transfers,
}

impl Conn {
    pub(super) fn send(&self, value: &impl serde::Serialize) {
        self.send_counted(value);
    }

    /// Send, and give the line's place in the queue (see [`Conn::written`]).
    pub(super) fn send_counted(&self, value: &impl serde::Serialize) -> u64 {
        let place = self
            .queued
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        if let Ok(line) = serde_json::to_string(value) {
            let _ = self.out.send(Outgoing::Line(line));
        }
        place
    }

    /// Queue a terminal frame's bytes; only on a link connection, whose
    /// writer sends it as the link's terminal kind. Frames are not counted
    /// in [`Conn::written`].
    pub(super) fn send_terminal(&self, frame: Vec<u8>) {
        if self.is_link() {
            let _ = self.out.send(Outgoing::Terminal(frame));
        }
    }

    /// Queue a binary transfer message; only on a link connection. A
    /// `bulk` one (a CHUNK) waits for the control queue to be empty;
    /// `written` fires once it is written.
    pub(super) fn send_binary(
        &self,
        bytes: Vec<u8>,
        bulk: bool,
        written: Option<tokio::sync::oneshot::Sender<()>>,
    ) {
        if !self.is_link() {
            return;
        }
        let message = Outgoing::Binary { bytes, written };
        let _ = if bulk {
            self.bulk.send(message)
        } else {
            self.out.send(message)
        };
    }

    /// The connection's number, unique in the daemon's run.
    pub(super) fn id(&self) -> u64 {
        self.id
    }

    /// Whether the connection came over the link.
    pub(super) fn is_link(&self) -> bool {
        matches!(self.origin, Origin::Link { .. })
    }

    /// `ready` fires once the answer to `request_id` is written.
    pub(super) fn on_answered(&self, request_id: Id, ready: tokio::sync::oneshot::Sender<()>) {
        self.after_answer.lock().unwrap().insert(request_id, ready);
    }

    /// How many lines the connection's writer has written.
    pub(super) fn written(&self) -> u64 {
        self.written.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) fn is_local(&self) -> bool {
        matches!(self.origin, Origin::Local)
    }

    pub(super) fn subscription_count(&self) -> usize {
        self.subscriptions.lock().unwrap().len()
    }

    pub(super) fn next_subscription_id(&self) -> u64 {
        self.next_subscription
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
    }

    /// Hold `sub`; `ready` fires once the answer to `request_id` is written.
    pub(super) fn add_subscription(
        &self,
        sub: Arc<Subscription>,
        request_id: Id,
        ready: tokio::sync::oneshot::Sender<()>,
    ) {
        self.after_answer.lock().unwrap().insert(request_id, ready);
        self.subscriptions.lock().unwrap().insert(sub.id, sub);
    }

    pub(super) fn subscription(&self, id: u64) -> Option<Arc<Subscription>> {
        self.subscriptions.lock().unwrap().get(&id).cloned()
    }

    pub(super) fn remove_subscription(&self, id: u64) -> Option<Arc<Subscription>> {
        self.subscriptions.lock().unwrap().remove(&id)
    }

    /// Whether the companion's link sleeps: it said so (`background`), or
    /// it has asked nothing but pings for [`ASLEEP_AFTER`] and holds no
    /// session subscription. An awake phone without one always asks within
    /// about 30 s (a list or a ping), so a minute of silence means it is not
    /// awake, though it may not have had the chance to say so (owner,
    /// 2026-10-23: "do we consider a daemon not getting a ping for 60s
    /// enough to background the link?"). One with a subscription is pushed
    /// to and need not ask (CHANGES.md, plan 20261023_03, step 4).
    pub(super) fn asleep(&self) -> bool {
        self.background.load(std::sync::atomic::Ordering::Relaxed)
            || (self.is_companion()
                && self.last_request.lock().unwrap().elapsed() >= ASLEEP_AFTER
                && self.subscriptions.lock().unwrap().is_empty())
    }

    pub(super) fn tab_follows_focus(&self) -> bool {
        self.tab_follows_focus
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) fn spoken_output(&self) -> bool {
        self.spoken_output
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(super) fn principal(&self) -> &Principal {
        self.principal.get().expect("only called after hello")
    }

    /// The companion session; only called on a companion connection.
    pub(super) fn session(&self) -> &Arc<Session> {
        self.session
            .get()
            .expect("only called after a companion hello")
    }

    /// Confirmations a turn opened on this connection: cancelled if it
    /// closes, and their resolution is sent to it.
    pub(super) fn opened(&self, ids: &[u64]) {
        self.opened.lock().unwrap().extend(ids.iter().copied());
    }

    pub(super) fn is_companion(&self) -> bool {
        self.session.get().is_some()
    }

    fn subscribed(&self, topic: &str) -> bool {
        self.topics.lock().unwrap().contains(topic)
    }
}

pub async fn handle(daemon: Arc<Daemon>, stream: UnixStream) {
    match stream.peer_cred() {
        Ok(cred) if peer_allowed(cred.uid(), daemon.uid) => {}
        Ok(cred) => {
            log::warn!("Refused a connection from uid {}", cred.uid());
            return;
        }
        Err(e) => {
            log::warn!("Refused a connection: cannot read its credentials: {e}");
            return;
        }
    }

    let (read, mut write) = stream.into_split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<Outgoing>();
    let written = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counted = Arc::clone(&written);
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            // Terminal frames are never queued here (`Conn::send_terminal`).
            let Outgoing::Line(mut line) = message else {
                continue;
            };
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let lines = FramedRead::new(read, LinesCodec::new_with_max_length(MAX_LINE)).map(|line| {
        line.map(Inbound::Line)
            .map_err(|e| format!("unreadable line: {e}"))
    });
    // The daemon socket carries no kind 2: its bulk queue is never read.
    let (bulk, _) = mpsc::unbounded_channel::<Outgoing>();
    serve(
        daemon,
        Origin::Local,
        lines,
        out,
        bulk,
        written,
        CancellationToken::new(),
    )
    .await;
    let _ = writer.await;
}

/// A link connection that finished its handshake as `entry`, with the
/// client's static key `key`. `first` is
/// the message carried in the handshake, already checked to be `hello`.
/// Each link message is one JSON-RPC object, from the daemon a terminal
/// frame (plan 20261027_07, decision 3), or either way a binary transfer
/// message (plan 20261028_05), on the same writer; `close` ends the
/// connection (its entry was revoked or replaced). The writer drains its
/// control queue first and sends a CHUNK only when it is empty (decision
/// 4.5). A terminal frame from the client is a protocol error, answered as
/// an unreadable message is (`INVALID_REQUEST`, no id), and closes it.
pub(crate) async fn handle_link(
    daemon: Arc<Daemon>,
    stream: LinkStream<TcpStream>,
    first: Option<Vec<u8>>,
    entry: AuthEntry,
    key: [u8; KEY_LEN],
    close: CancellationToken,
) {
    let (reader, mut writer) = stream.into_split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<Outgoing>();
    let (bulk, mut bulk_rx) = mpsc::unbounded_channel::<Outgoing>();
    let written = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counted = Arc::clone(&written);
    let write_task = tokio::spawn(async move {
        let mut bulk_open = true;
        loop {
            // Control first: JSON-RPC and terminal frames wait at most one
            // CHUNK (plan 20261028_05, decision 4.5).
            let message = tokio::select! {
                biased;
                m = out_rx.recv() => match m {
                    Some(m) => m,
                    None => break,
                },
                m = bulk_rx.recv(), if bulk_open => match m {
                    Some(m) => m,
                    None => {
                        bulk_open = false;
                        continue;
                    }
                },
            };
            match message {
                Outgoing::Line(line) => {
                    if writer.send_message(line.as_bytes()).await.is_err() {
                        return;
                    }
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                Outgoing::Terminal(frame) => {
                    if writer.send_terminal(&frame).await.is_err() {
                        return;
                    }
                }
                Outgoing::Binary { bytes, written } => {
                    if writer.send_binary(&bytes).await.is_err() {
                        return;
                    }
                    if let Some(written) = written {
                        let _ = written.send(());
                    }
                }
            }
        }
        let _ = writer.shutdown().await;
    });
    let first = futures_util::stream::iter(first.map(|m| decode_message(m).map(Inbound::Line)));
    let rest = futures_util::stream::unfold(Some(reader), |reader| async move {
        let mut reader = reader?;
        match reader.recv().await {
            Ok(Message::Json(message)) => {
                Some((decode_message(message).map(Inbound::Line), Some(reader)))
            }
            Ok(Message::Binary(message)) => Some((Ok(Inbound::Binary(message)), Some(reader))),
            Ok(_) => Some((
                Err("a terminal frame from the client; only the daemon sends them".to_string()),
                None,
            )),
            Err(LinkError::Closed) => None,
            Err(e) => Some((Err(e.to_string()), None)),
        }
    });
    serve(
        daemon,
        Origin::Link { entry, key },
        Box::pin(first.chain(rest)),
        out,
        bulk,
        written,
        close,
    )
    .await;
    let _ = write_task.await;
}

fn decode_message(message: Vec<u8>) -> Result<String, String> {
    String::from_utf8(message).map_err(|_| "a message that is not UTF-8".to_string())
}

/// Serve one connection's messages until it closes or `close` fires.
async fn serve<S>(
    daemon: Arc<Daemon>,
    origin: Origin,
    mut lines: S,
    out: mpsc::UnboundedSender<Outgoing>,
    bulk: mpsc::UnboundedSender<Outgoing>,
    written: Arc<std::sync::atomic::AtomicU64>,
    close: CancellationToken,
) where
    S: Stream<Item = Result<Inbound, String>> + Unpin,
{
    let conn = Arc::new(Conn {
        daemon,
        id: NEXT_CONN.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        origin,
        out,
        bulk,
        principal: OnceLock::new(),
        session: OnceLock::new(),
        topics: Mutex::new(HashSet::new()),
        inflight: Mutex::new(HashMap::new()),
        opened: Mutex::new(HashSet::new()),
        vocabulary: Mutex::new(Vec::new()),
        focus: Mutex::new(None),
        notices: Mutex::new(Announcer::default()),
        next_notice: std::sync::atomic::AtomicU64::new(0),
        speech: tokio::sync::Mutex::new(()),
        spoken_output: std::sync::atomic::AtomicBool::new(false),
        background: std::sync::atomic::AtomicBool::new(false),
        general_agent: std::sync::atomic::AtomicBool::new(true),
        tab_follows_focus: std::sync::atomic::AtomicBool::new(false),
        last_request: Mutex::new(std::time::Instant::now()),
        subscriptions: Mutex::new(HashMap::new()),
        next_subscription: std::sync::atomic::AtomicU64::new(0),
        after_answer: Mutex::new(HashMap::new()),
        queued: std::sync::atomic::AtomicU64::new(0),
        written,
        terminals: Default::default(),
        plugin_calls: Default::default(),
        blob: Default::default(),
    });
    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        let line = tokio::select! {
            _ = close.cancelled() => break,
            line = lines.next() => line,
        };
        let Some(line) = line else { break };
        let line = match line {
            Ok(Inbound::Line(line)) => line,
            // A binary transfer message (plan 20261028_05): from a
            // companion after hello only, applied before the next message
            // is read; an unreadable header closes the connection.
            Ok(Inbound::Binary(bytes)) => {
                if conn.principal.get().is_none() || !conn.is_companion() {
                    log::warn!(
                        "Connection {} closed: a binary message before a companion hello",
                        conn.id
                    );
                    break;
                }
                if super::blob::on_message(&conn, bytes).await.is_err() {
                    break;
                }
                continue;
            }
            Err(e) => {
                conn.send(&Response::err(
                    None,
                    ErrorObject::new(codes::INVALID_REQUEST, e),
                ));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Incoming>(&line) {
            Ok(Incoming::Request(request)) => request,
            // The one notification a client sends (plan 20261027_07).
            Ok(Incoming::Notification(note))
                if note.method == methods::TERMINAL_ACK
                    && conn.principal.get().is_some()
                    && conn.is_companion() =>
            {
                super::terminal::ack(&conn, note.params);
                continue;
            }
            Ok(_) => continue,
            Err(e) => {
                conn.send(&Response::err(
                    None,
                    ErrorObject::new(codes::PARSE_ERROR, format!("not a JSON-RPC request: {e}")),
                ));
                continue;
            }
        };

        if conn.principal.get().is_none() {
            if request.method != methods::HELLO {
                conn.send(&Response::err(
                    Some(request.id),
                    ErrorObject::new(codes::HELLO_REQUIRED, "the first request must be hello"),
                ));
                break;
            }
            match hello(&conn, &request).await {
                Ok((principal, result)) => {
                    log::info!("Connection {} says hello as {}", conn.id, principal.id);
                    let _ = conn.principal.set(principal);
                    forwarder = Some(tokio::spawn(forward_events(Arc::clone(&conn))));
                    conn.send(&Response::ok(request.id, result));
                }
                Err(error) => {
                    conn.send(&Response::err(Some(request.id), error));
                    break;
                }
            }
            continue;
        }

        if request.method == methods::CANCEL {
            let result = cancel(&conn, &request);
            conn.send(&result);
            continue;
        }

        let id = request.id.clone();
        let task_conn = Arc::clone(&conn);
        let task = tokio::spawn(async move {
            let response =
                match call(&task_conn, &request.id, &request.method, request.params).await {
                    Ok(result) => Response::ok(request.id.clone(), result),
                    Err(error) => Response::err(Some(request.id.clone()), error),
                };
            task_conn.inflight.lock().unwrap().remove(&request.id);
            task_conn.send(&response);
            let after = task_conn.after_answer.lock().unwrap().remove(&request.id);
            if let Some(after) = after {
                let _ = after.send(());
            }
        });
        conn.inflight
            .lock()
            .unwrap()
            .insert(id, task.abort_handle());
    }

    // Closed: stop what is still running, and withdraw what this
    // connection was waiting on.
    for (_, task) in conn.inflight.lock().unwrap().drain() {
        task.abort();
    }
    for (_, sub) in conn.subscriptions.lock().unwrap().drain() {
        sub.cancel();
    }
    conn.terminals.close_all(&conn.daemon);
    conn.plugin_calls.close();
    conn.blob.close();
    if let Some(link) = &conn.daemon.link {
        link.end_pair_mode(conn.id);
    }
    if let Some(principal) = conn.principal.get() {
        let opened: Vec<u64> = conn.opened.lock().unwrap().drain().collect();
        for confirm_id in opened {
            let _ = conn.daemon.broker.cancel(principal, confirm_id);
        }
        log::info!("Connection from {} closed", principal.id);
    }
    if let Some(forwarder) = forwarder {
        forwarder.abort();
    }
}

async fn hello(conn: &Conn, request: &Request) -> Result<(Principal, Value), ErrorObject> {
    let origin = &conn.origin;
    let params: common::HelloParams = params(request.params.clone())?;
    if let Origin::Link { entry, .. } = origin
        && !entry.allows(&params.protocol)
    {
        return Err(ErrorObject::new(
            codes::NOT_PERMITTED,
            format!(
                "'{}' is not paired for {}; it may use {:?}",
                entry.name, params.protocol, entry.protocols
            ),
        ));
    }
    let served = (params.protocol == protocols::HARNESS && params.version == HARNESS_VERSION)
        || (params.protocol == protocols::COMPANION && params.version == COMPANION_VERSION);
    if !served {
        return Err(ErrorObject::new(
            codes::UNSUPPORTED_PROTOCOL,
            format!(
                "unsupported protocol {} version {}; this daemon serves {} version {} and {} version {}",
                params.protocol,
                params.version,
                protocols::HARNESS,
                HARNESS_VERSION,
                protocols::COMPANION,
                COMPANION_VERSION
            ),
        ));
    }
    let client = params.client.trim();
    if client.is_empty() {
        return Err(ErrorObject::new(
            codes::INVALID_PARAMS,
            "hello needs a client name",
        ));
    }
    let principal = match origin {
        Origin::Local => Principal::local(client),
        Origin::Link { entry, .. } => entry.principal(&params.protocol),
    };
    // Plan 20261022_02, amendment of 23:07 EDT: the first hello after
    // `link.pair_add` added this key, once it is sure to be answered.
    let first_hello = || match (origin, &conn.daemon.link) {
        (Origin::Link { key, .. }, Some(link)) => link.first_hello(key),
        _ => false,
    };
    if params.protocol == protocols::COMPANION {
        // Plan 20261022_02, decisions 8 and 13: the session follows the
        // principal, whatever session_id the client remembered.
        let asked: companion::HelloParams = self::params(request.params.clone())?;
        let session_id = turns::session_id(&principal);
        if asked.session_id.as_deref().is_some_and(|s| s != session_id) {
            log::info!(
                "{} asked for session {:?}; it continues {session_id}",
                principal.id,
                asked.session_id
            );
        }
        let (vocabulary, focus) = turns::vocabulary(&conn.daemon).await;
        *conn.vocabulary.lock().unwrap() = vocabulary.clone();
        *conn.focus.lock().unwrap() = focus;
        let _ = conn.session.set(conn.daemon.turns.session(&session_id));
        let paired_now = first_hello();
        let result = ok(companion::HelloResult {
            server: "flankr".to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: COMPANION_VERSION,
            session_id: Some(session_id),
            vocabulary,
            paired: paired_now.then_some(true),
            llm: Some(conn.daemon.llm.info()),
            terminal: super::terminal::info(&conn.daemon, conn.is_link()),
            plugins: Some(conn.daemon.plugins.infos()),
            blob: super::blob::info(&conn.daemon, conn.is_link()),
        })?;
        return Ok((principal, result));
    }
    let mut result = ok(common::HelloResult {
        server: "flankr".to_string(),
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: HARNESS_VERSION,
    })?;
    if first_hello()
        && let Value::Object(map) = &mut result
    {
        map.insert("paired".to_string(), Value::Bool(true));
    }
    Ok((principal, result))
}

fn cancel(conn: &Conn, request: &Request) -> Response {
    let target: common::CancelParams = match params(request.params.clone()) {
        Ok(target) => target,
        Err(error) => return Response::err(Some(request.id.clone()), error),
    };
    // `cancel` on a turn.start id interrupts the turn, which answers it
    // with `outcome: "interrupted"` (decision 10).
    if conn.is_companion() && conn.session().interrupt_request(&target.id) {
        return Response::ok(request.id.clone(), common::CancelResult { cancelled: true });
    }
    let task = conn.inflight.lock().unwrap().remove(&target.id);
    let cancelled = match task {
        Some(task) => {
            task.abort();
            conn.send(&Response::err(
                Some(target.id),
                ErrorObject::new(codes::CANCELLED, "cancelled"),
            ));
            true
        }
        None => false,
    };
    Response::ok(request.id.clone(), common::CancelResult { cancelled })
}

pub(super) fn params<T: DeserializeOwned>(value: Value) -> Result<T, ErrorObject> {
    let value = if value.is_null() {
        Value::Object(Default::default())
    } else {
        value
    };
    serde_json::from_value(value)
        .map_err(|e| ErrorObject::new(codes::INVALID_PARAMS, format!("invalid params: {e}")))
}

pub(super) fn ok(value: impl serde::Serialize) -> Result<Value, ErrorObject> {
    serde_json::to_value(value).map_err(|e| ErrorObject::new(codes::INTERNAL_ERROR, e.to_string()))
}

fn herdr_unavailable(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(codes::HERDR_UNAVAILABLE, format!("herdr: {e}"))
}

fn confirm_error(e: ConfirmError) -> ErrorObject {
    ErrorObject::new(e.code(), e.to_string())
}

fn wire_outcome(outcome: Outcome) -> common::Outcome {
    match outcome {
        Outcome::Pending => common::Outcome::Pending,
        Outcome::Approved => common::Outcome::Approved,
        Outcome::Sent => common::Outcome::Sent,
        Outcome::Failed => common::Outcome::Failed,
        Outcome::Denied => common::Outcome::Denied,
        Outcome::Expired => common::Outcome::Expired,
        Outcome::Cancelled => common::Outcome::Cancelled,
    }
}

/// An outcome's word on the wire (`approved`, `sent`, `denied` …).
pub(super) fn outcome_word(outcome: Outcome) -> String {
    serde_json::to_value(wire_outcome(outcome))
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

pub(super) fn wire_status(status: &libherdr::protocol::AgentStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Whether session files are on this machine: not for a remote herdr
/// (plan 20261022_03, decision 4).
pub(super) fn sessions_here(daemon: &Daemon) -> bool {
    !matches!(
        daemon.client.transport(),
        libherdr::transport::HerdrTransport::RemoteSsh { .. }
    )
}

/// What `agents.list` gives of an agent's own session.
#[derive(Debug, Default)]
struct Listed {
    session: Option<common::AgentSessionInfo>,
    turn: Option<common::Turn>,
    idle_ms: Option<u64>,
    subagents_running: Option<u32>,
    summary: Option<String>,
    children: Vec<common::ChildInfo>,
}

/// The session of the agent in `pane`, its turn (plan 20261023_04, A4),
/// and its activity and latest summary as of `now` (plan 20261028_03,
/// 3.12 item 47), for `agents.list`. The summary is only one already
/// known; none is asked for because of a list.
fn agent_session(daemon: &Daemon, pane: &agents::AgentRef, now: SystemTime) -> Listed {
    let Some(reader) = sessions_here(daemon)
        .then(|| daemon.sessions.reader_for(&pane.agent))
        .flatten()
    else {
        return Listed::default();
    };
    let Some(mut peeked) = daemon.sessions.peek(pane, 1) else {
        return Listed::default();
    };
    peeked.link(daemon.repowiki.as_deref(), pane.cwd.as_deref());
    let activity = crate::sessions::activity(reader, &peeked.located, now);
    // Its subagents (the subagents plan, decision 2).
    let children = daemon.sessions.children_info(reader, &peeked.located);
    let summary = peeked
        .last_reply
        .and_then(|line| {
            daemon
                .hub
                .summaries()
                .latest_spoken(&peeked.located.path, line)
        })
        .and_then(|spoken| crate::sessions::turn::summary_line(&spoken));
    Listed {
        session: Some(common::AgentSessionInfo {
            session_id: peeked.located.session_id,
            matched: peeked.located.matched.into(),
            pending_question: peeked.pending_question,
        }),
        turn: Some(peeked.turn),
        idle_ms: activity.idle_ms,
        subagents_running: activity.subagents_running,
        summary,
        children,
    }
}

pub(super) async fn snapshot(conn: &Conn) -> Result<Value, ErrorObject> {
    conn.daemon
        .client
        .list_panes()
        .await
        .map_err(herdr_unavailable)
}

/// `-32002` for a subagent `child` that the session of the pane named
/// `shown` does not have (the subagents plan, step 3), or for any subagent
/// when the session files are not on this machine.
pub(super) fn unknown_child(shown: &str, child: Option<&str>) -> ErrorObject {
    ErrorObject::new(
        codes::UNKNOWN_AGENT,
        format!("{shown} has no subagent '{}'", child.unwrap_or_default()),
    )
}

/// `-32002` for a target that names no pane or more than one. An
/// ambiguous target's message, and `data.candidates`, list the panes by
/// their shown names.
pub(super) fn resolve_error(e: ResolveError) -> ErrorObject {
    let mut error = ErrorObject::new(codes::UNKNOWN_AGENT, e.to_string());
    if let ResolveError::Ambiguous { candidates, .. } = &e {
        error.data = Some(serde_json::json!({ "candidates": candidates }));
    }
    error
}

/// Resolve a method's target to a pane id (plan 20261022_05, decision 10):
/// a pane id, a tab label as written, an alias value, a spoken form or its
/// capitals, or a tab number.
pub(super) fn resolve_target(
    daemon: &Daemon,
    snap: &Value,
    target: &str,
) -> Result<String, ErrorObject> {
    daemon
        .names(snap)
        .resolve(target)
        .map(|a| a.agent.pane_id.clone())
        .map_err(resolve_error)
}

/// A companion that has asked nothing but pings for this long is taken as
/// asleep (see `Conn::asleep`).
pub(super) const ASLEEP_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

async fn call(
    conn: &Arc<Conn>,
    request_id: &Id,
    method: &str,
    raw: Value,
) -> Result<Value, ErrorObject> {
    let daemon = &conn.daemon;
    // Anything but a ping shows the phone is awake, and so does a ping
    // that says it is active (see `asleep`).
    let ping_active = (method == methods::PING)
        .then(|| serde_json::from_value::<common::PingParams>(raw.clone()).ok())
        .flatten()
        .and_then(|p| p.active);
    if method != methods::PING || ping_active == Some(true) {
        *conn.last_request.lock().unwrap() = std::time::Instant::now();
    }
    if let Some(active) = ping_active {
        conn.background
            .store(!active, std::sync::atomic::Ordering::Relaxed);
    }
    match method {
        companion::methods::TURN_START if conn.is_companion() => {
            let p: companion::TurnStartParams = params(raw)?;
            // Plan 20261027_02, decision 8: no general agent, no turn.
            if let Err(reason) = daemon.llm.available(crate::llm::Use::GeneralAgent) {
                return Err(ErrorObject::new(codes::LLM_UNAVAILABLE, reason));
            }
            if !conn
                .general_agent
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "the general agent is off for this client",
                ));
            }
            turns::turn_start(conn, request_id, p).await
        }
        companion::methods::TURN_INTERRUPT if conn.is_companion() => {
            let p: companion::TurnInterruptParams = params(raw)?;
            if !conn.session().interrupt(p.turn_id) {
                log::debug!(
                    "turn.interrupt for turn {}, which is not running",
                    p.turn_id
                );
            }
            ok(common::Empty {})
        }
        // Plan 20261027_07: companions only (a harness client is answered
        // METHOD_NOT_FOUND below); `terminal::served` narrows to the link.
        methods::TERMINAL_OPEN if conn.is_companion() => {
            super::terminal::open(conn, request_id, raw).await
        }
        methods::TERMINAL_CLOSE if conn.is_companion() => super::terminal::close(conn, raw),
        methods::TERMINAL_INPUT_ARM if conn.is_companion() => super::terminal::input_arm(conn, raw),
        methods::TERMINAL_INPUT if conn.is_companion() => super::terminal::input(conn, raw).await,
        methods::TERMINAL_SCROLL if conn.is_companion() => super::terminal::scroll(conn, raw).await,
        // Plan 20261027_08, steps 7 and 8.
        methods::TERMINAL_MOUSE if conn.is_companion() => super::terminal::mouse(conn, raw).await,
        methods::TERMINAL_RESIZE if conn.is_companion() => super::terminal::resize(conn, raw),
        methods::TERMINAL_PASTE if conn.is_companion() => super::terminal::paste(conn, raw).await,
        // Plan 20261029_01, decision 2: for companions; anyone else gets
        // METHOD_NOT_FOUND below.
        methods::CHOICE_ANSWER if conn.is_companion() => super::choice::answer(conn, raw).await,
        companion::methods::PLUGIN_CALL if conn.is_companion() => {
            plugin_call(conn, request_id, params(raw)?)
        }
        companion::methods::PLUGIN_SET_ENABLED if conn.is_companion() => {
            let p: companion::PluginSetEnabledParams = params(raw)?;
            let daemon = Arc::clone(&conn.daemon);
            let principal = conn.principal().clone();
            // The write is small and local; it runs off the async workers.
            let result =
                tokio::task::spawn_blocking(move || daemon.plugins.set_enabled(&principal, p))
                    .await
                    .map_err(|e| ErrorObject::new(codes::INTERNAL_ERROR, e.to_string()))??;
            ok(result)
        }
        methods::AGENTS_INTERRUPT => {
            let p: common::AgentsInterruptParams = params(raw)?;
            let snap = snapshot(conn).await?;
            ok(turns::agents_interrupt(daemon, conn.principal(), &snap, &p.target).await?)
        }
        methods::PING => ok(common::Empty {}),
        methods::HELLO => Err(ErrorObject::new(
            codes::INVALID_REQUEST,
            "hello was already said on this connection",
        )),
        methods::AGENTS_LIST => {
            let snap = snapshot(conn).await?;
            let names = daemon.names(&snap);
            let focus = super::focus_info(&snap, &names);
            if conn.is_companion() {
                *conn.focus.lock().unwrap() = focus.clone();
            }
            let now = SystemTime::now();
            let agents = names
                .agents
                .into_iter()
                .map(|named| {
                    let a = named.agent;
                    let listed = agent_session(daemon, &a, now);
                    common::AgentInfo {
                        session: listed.session,
                        turn: listed.turn,
                        idle_ms: listed.idle_ms,
                        subagents_running: listed.subagents_running,
                        summary: listed.summary,
                        children: listed.children,
                        status: wire_status(&a.status),
                        pane_id: a.pane_id,
                        name: Some(named.shown),
                        spoken: Some(named.spoken),
                        agent: a.agent,
                        // The project as named (`[naming] project_from`:
                        // the directory by default), not herdr's raw label.
                        project: named
                            .project
                            .as_ref()
                            .map(|p| p.shown.clone())
                            .or(a.project),
                        cwd: a.cwd,
                        workspace_id: a.workspace_id,
                        tab_id: a.tab_id,
                        tab_label: a.tab_label,
                        tab_number: a.tab_number,
                        focused: a.focused,
                    }
                })
                .collect();
            ok(common::AgentsListResult { agents, focus })
        }
        methods::AGENTS_READ => {
            let p: common::AgentsReadParams = params(raw)?;
            if !(1..=common::READ_LINES_MAX).contains(&p.lines) {
                return Err(ErrorObject::new(
                    codes::INVALID_PARAMS,
                    format!("lines must be 1 to {}", common::READ_LINES_MAX),
                ));
            }
            let snap = snapshot(conn).await?;
            let pane_id = resolve_target(daemon, &snap, &p.pane_id)?;
            let read = daemon
                .client
                .inspect_pane(&pane_id, p.lines)
                .await
                .map_err(herdr_unavailable)?;
            let text = agents::pane_text(&read).unwrap_or_default().to_string();
            ok(common::AgentsReadResult { text })
        }
        methods::SESSION_PEEK => {
            // Plan 20261022_03, decision 9: companions and local clients,
            // read-only. Paired harness clients get it with step 5.
            if !(conn.is_companion() || matches!(conn.origin, Origin::Local)) {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "session.peek is served to companions and on the daemon socket",
                ));
            }
            let p: common::SessionPeekParams = params(raw)?;
            if !(1..=common::PEEK_ENTRIES_MAX).contains(&p.entries) {
                return Err(ErrorObject::new(
                    codes::INVALID_PARAMS,
                    format!("entries must be 1 to {}", common::PEEK_ENTRIES_MAX),
                ));
            }
            let snap = snapshot(conn).await?;
            let pane_id = resolve_target(daemon, &snap, &p.pane_id)?;
            let names = daemon.names(&snap);
            let Some(named) = names.get(&pane_id) else {
                return Err(ErrorObject::new(
                    codes::UNKNOWN_AGENT,
                    format!("no pane '{pane_id}'"),
                ));
            };
            let pane = &named.agent;
            if !conn.principal().may_reach(&pane.agent) {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    format!("{} may not reach {}", conn.principal().id, named.shown),
                ));
            }
            // A subagent's session (the subagents plan, decision 2), read
            // with its own status; one the session lacks is unknown.
            let peeked = match sessions_here(daemon)
                .then(|| daemon.sessions.peek_in(pane, p.child.as_deref(), p.entries))
            {
                Some(Ok(peeked)) => peeked,
                Some(Err(crate::sessions::UnknownChild)) | None if p.child.is_some() => {
                    return Err(unknown_child(&named.shown, p.child.as_deref()));
                }
                _ => None,
            };
            let status = match (&p.child, &peeked) {
                (Some(_), Some(peeked)) => peeked.status.clone(),
                _ => pane.status.clone(),
            };
            let session = peeked
                .map(|mut peeked| {
                    peeked.link(daemon.repowiki.as_deref(), pane.cwd.as_deref());
                    peeked
                })
                .map(|peeked| common::SessionPeek {
                    session_id: peeked.located.session_id,
                    matched: peeked.located.matched.into(),
                    entries: peeked.entries,
                    pending_question: peeked.pending_question,
                });
            ok(common::SessionPeekResult {
                session,
                status: wire_status(&status),
            })
        }
        methods::SESSION_SUBSCRIBE => {
            let p: common::SessionSubscribeParams = params(raw)?;
            subscriptions::subscribe(conn, request_id, p).await
        }
        methods::SESSION_UNSUBSCRIBE => {
            let p: common::SessionUnsubscribeParams = params(raw)?;
            subscriptions::unsubscribe(conn, p)
        }
        methods::SESSION_HISTORY => {
            let p: common::SessionHistoryParams = params(raw)?;
            subscriptions::history(conn, p).await
        }
        methods::SESSION_CONTENT => {
            let p: common::SessionContentParams = params(raw)?;
            subscriptions::content(conn, p).await
        }
        methods::SPEECH_SUMMARIZE => {
            // Plan 20261023_05, decision 4: companions only, with
            // session.peek's permission for the pane.
            if !conn.is_companion() {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "speech.summarize is served to companions",
                ));
            }
            let p: common::SpeechSummarizeParams = params(raw)?;
            super::speech::speech_summarize(conn, p).await
        }
        methods::CLIENT_OPTIONS => {
            // Plan 20261023_05, A4: a companion's options for this
            // connection; unknown ones are ignored.
            if !conn.is_companion() {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "client.options is served to companions",
                ));
            }
            let p: common::ClientOptionsParams = params(raw)?;
            conn.spoken_output
                .store(p.spoken_output, std::sync::atomic::Ordering::Relaxed);
            conn.background
                .store(p.background, std::sync::atomic::Ordering::Relaxed);
            conn.general_agent
                .store(p.general_agent, std::sync::atomic::Ordering::Relaxed);
            conn.tab_follows_focus
                .store(p.tab_follows_focus, std::sync::atomic::Ordering::Relaxed);
            ok(common::Empty {})
        }
        methods::AGENTS_SEND => {
            let p: common::AgentsSendParams = params(raw)?;
            let snap = snapshot(conn).await?;
            let pane_id = resolve_target(daemon, &snap, &p.pane_id)?;
            let agent = extract_agent_name_for_pane(&snap, &pane_id);
            // Plan 20261029_01, decision 6: not while a choice question
            // is open, whatever the kind.
            if let Some(reason) = crate::sessions::choice_refusal(
                &daemon.sessions,
                !matches!(
                    daemon.client.transport(),
                    libherdr::transport::HerdrTransport::RemoteSsh { .. }
                ),
                &snap,
                &pane_id,
            ) {
                return Err(ErrorObject::new(codes::NOT_PERMITTED, reason));
            }
            if !p.parts.is_empty() {
                return send_parts(conn, &snap, &pane_id, &agent, p).await;
            }
            if p.kind == common::SendKind::Message {
                return send_message(conn, &snap, &pane_id, &agent, p).await;
            }
            let principal = conn.principal();
            let checked = policy::check(principal, &daemon.agents, &agent, &pane_id, &p.command)
                .map_err(|denied| ErrorObject::new(codes::NOT_PERMITTED, denied.to_string()))?;
            let by_tap = tapped(conn, &p);
            if by_tap {
                log::info!(
                    "Command '{}' for pane '{pane_id}' confirmed by the tap",
                    checked.command
                );
            }
            if checked.require_confirmation && !by_tap {
                let confirm_id = daemon
                    .broker
                    .request(
                        principal,
                        &pane_id,
                        &agent,
                        &checked.command,
                        p.handle.as_deref(),
                        Lifetime::Standing,
                    )
                    .map_err(confirm_error)?;
                conn.opened.lock().unwrap().insert(confirm_id);
                return ok(common::AgentsSendResult::Pending {
                    confirm_id,
                    handle: p.handle,
                });
            }
            daemon
                .client
                .run_command(&pane_id, &checked.command)
                .await
                .map_err(herdr_unavailable)?;
            ok(common::AgentsSendResult::Sent)
        }
        methods::AGENTS_FOCUS => {
            let p: common::AgentsFocusParams = params(raw)?;
            let snap = snapshot(conn).await?;
            let target = if agents::is_workspace_or_tab_id(&snap, &p.target_id) {
                p.target_id.trim().to_string()
            } else {
                resolve_target(daemon, &snap, &p.target_id)?
            };
            daemon
                .client
                .focus(&target)
                .await
                .map_err(herdr_unavailable)?;
            if conn.is_companion() {
                tokio::spawn(turns::check_vocabulary(Arc::clone(conn)));
            }
            ok(common::Empty {})
        }
        methods::WATCH_ADD | methods::WATCH_REMOVE => {
            let p: common::WatchParams = params(raw)?;
            let snap = snapshot(conn).await?;
            let pane_id = resolve_target(daemon, &snap, &p.pane_id)?;
            if method == methods::WATCH_ADD {
                daemon.watches.watch(&pane_id);
            } else {
                daemon.watches.unwatch(&pane_id);
            }
            ok(common::WatchResult {
                watched: daemon.watches.watched(),
            })
        }
        methods::CONFIRM_REPLY => {
            let p: common::ConfirmReplyParams = params(raw)?;
            let outcome = daemon
                .broker
                .reply(
                    conn.principal(),
                    p.confirm_id,
                    p.approve,
                    p.nonce.as_deref(),
                )
                .await
                .map_err(confirm_error)?;
            ok(common::OutcomeResult {
                outcome: wire_outcome(outcome),
            })
        }
        methods::CONFIRM_CANCEL => {
            let p: common::ConfirmCancelParams = params(raw)?;
            let outcome = daemon
                .broker
                .cancel(conn.principal(), p.confirm_id)
                .map_err(confirm_error)?;
            conn.opened.lock().unwrap().remove(&p.confirm_id);
            ok(common::OutcomeResult {
                outcome: wire_outcome(outcome),
            })
        }
        methods::CONFIRM_STATUS => {
            let p: common::ConfirmStatusParams = params(raw)?;
            let key = match (p.confirm_id, p.handle) {
                (Some(id), None) => StatusKey::Id(id),
                (None, Some(handle)) => StatusKey::Handle(handle),
                _ => {
                    return Err(ErrorObject::new(
                        codes::INVALID_PARAMS,
                        "give exactly one of confirm_id and handle",
                    ));
                }
            };
            let found = daemon.broker.find(conn.principal(), &key).map_err(|e| {
                // Say which handles are pending, newest first (plan
                // 20261022_08, 23:44 EDT amendment; plan 20261023_01,
                // section 3.3).
                let mut pending = daemon.broker.pending_handles(&conn.principal().id);
                let by_handle = matches!(key, StatusKey::Handle(_));
                let mut error = match (&e, by_handle) {
                    (ConfirmError::NotFound, true) if !pending.is_empty() => {
                        ErrorObject::new(e.code(), format!("{e}; pending: {}", pending.join(", ")))
                    }
                    _ => confirm_error(e.clone()),
                };
                if by_handle {
                    pending.reverse();
                    let mut data = serde_json::Map::new();
                    data.insert("pending".to_string(), serde_json::json!(pending));
                    if let ConfirmError::AmbiguousHandle(candidates) = &e {
                        data.insert("candidates".to_string(), serde_json::json!(candidates));
                    }
                    error.data = Some(Value::Object(data));
                }
                error
            })?;
            // How it was found, for `flankr ctl` (plan 20261023_01, section 4).
            let mut result = ok(common::ConfirmStatusResult {
                confirm_id: found.view.confirm_id,
                handle: found.view.handle,
                outcome: wire_outcome(found.outcome),
            })?;
            if let Value::Object(obj) = &mut result {
                obj.insert(
                    "matched".to_string(),
                    Value::String(found.matched.as_str().to_string()),
                );
                if let Some(bits) = found.matched.bits() {
                    obj.insert("bits".to_string(), serde_json::json!(bits));
                }
            }
            Ok(result)
        }
        methods::EVENTS_SUBSCRIBE => {
            let p: common::EventsSubscribeParams = params(raw)?;
            if let Some(unknown) = p
                .topics
                .iter()
                .find(|t| ![topics::AGENT_STATUS, topics::CONFIRM].contains(&t.as_str()))
            {
                return Err(ErrorObject::new(
                    codes::INVALID_PARAMS,
                    format!("unknown topic '{unknown}'"),
                ));
            }
            conn.topics.lock().unwrap().extend(p.topics);
            ok(common::Empty {})
        }
        methods::LINK_PAIR_BEGIN
        | methods::LINK_PAIR_ADD
        | pair_mode::methods::LINK_PAIR_CODE
        | pair_mode::methods::LINK_PAIR_ANSWER => {
            if !matches!(conn.origin, Origin::Local) {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    format!("{method} is served only on the daemon socket"),
                ));
            }
            let Some(link) = &daemon.link else {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "the link is off ([link] listen = [])",
                ));
            };
            if method == methods::LINK_PAIR_BEGIN {
                // Plan 20261028_01, the 14:37 to 14:44 EDT amendments: a
                // fresh offer, and pair mode for this connection.
                let p: pair_mode::PairBeginParams = params(raw)?;
                let begun = link
                    .pair_begin(conn.id, p)
                    .await
                    .map_err(|e| ErrorObject::new(codes::INVALID_PARAMS, e))?;
                let out = conn.out.downgrade();
                let mut events = begun.events;
                tokio::spawn(async move {
                    while let Some(event) = events.recv().await {
                        let line = match event {
                            pair_mode::PairEvent::Request(info) => serde_json::to_string(
                                &Notification::new(pair_mode::methods::LINK_PAIR_REQUEST, info),
                            ),
                            pair_mode::PairEvent::Ended(ended) => serde_json::to_string(
                                &Notification::new(pair_mode::methods::LINK_PAIR_ENDED, ended),
                            ),
                        };
                        match (out.upgrade(), line) {
                            (Some(out), Ok(line)) => {
                                let _ = out.send(Outgoing::Line(line));
                            }
                            (None, _) => break,
                            _ => {}
                        }
                    }
                });
                let mut result = serde_json::to_value(begun.result)
                    .map_err(|e| ErrorObject::new(codes::INTERNAL_ERROR, e.to_string()))?;
                if let (Some(obj), Ok(Value::Object(extra))) =
                    (result.as_object_mut(), serde_json::to_value(begun.extra))
                {
                    obj.extend(extra);
                }
                return Ok(result);
            }
            if method == pair_mode::methods::LINK_PAIR_CODE {
                let p: pair_mode::PairCodeParams = params(raw)?;
                let info = link
                    .pair_code(conn.id, &p.code)
                    .map_err(|e| ErrorObject::new(codes::INVALID_PARAMS, e))?;
                return ok(info);
            }
            if method == pair_mode::methods::LINK_PAIR_ANSWER {
                let p: pair_mode::PairAnswerParams = params(raw)?;
                let answered = link
                    .pair_answer(conn.id, p)
                    .await
                    .map_err(|e| ErrorObject::new(codes::INVALID_PARAMS, e))?;
                return ok(answered);
            }
            let p: common::LinkPairAddParams = params(raw)?;
            let added = link
                .pair_add(p)
                .map_err(|e| ErrorObject::new(codes::INVALID_PARAMS, e))?;
            // Weak, so a device that never connects does not hold this
            // connection's writer open after it closes.
            let out = conn.out.downgrade();
            let done = added.done;
            tokio::spawn(async move {
                if let Ok(done) = done.await
                    && let (Some(out), Ok(line)) = (
                        out.upgrade(),
                        serde_json::to_string(&Notification::new(methods::LINK_PAIR_DONE, done)),
                    )
                {
                    let _ = out.send(Outgoing::Line(line));
                }
            });
            ok(added.result)
        }
        methods::LINK_UNPAIR => {
            // Plan 20261024_02, decision 6: a paired device forgets itself.
            // Not on the daemon socket, which has no device to forget.
            let Origin::Link { key, .. } = &conn.origin else {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "link.unpair is served only over a paired link",
                ));
            };
            let _: common::LinkUnpairParams = params(raw)?;
            let Some(link) = &daemon.link else {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "the link is off ([link] listen = [])",
                ));
            };
            link.unpair(key)
                .map_err(|e| ErrorObject::new(codes::INTERNAL_ERROR, e))?;
            // The link closes once the answer is queued: its writer sends
            // what is queued before it shuts down.
            let (answered, after) = tokio::sync::oneshot::channel();
            conn.after_answer
                .lock()
                .unwrap()
                .insert(request_id.clone(), answered);
            let (link, key) = (Arc::clone(link), *key);
            tokio::spawn(async move {
                let _ = after.await;
                link.close_key(&key, "it unpaired");
            });
            ok(common::LinkUnpairResult {})
        }
        other => Err(ErrorObject::new(
            codes::METHOD_NOT_FOUND,
            format!("unknown method '{other}'"),
        )),
    }
}

/// Whether `agents.send` comes with the user's approval already: `confirmed`
/// from a companion (plan 20261023_02, the 2026-10-27 13:25 EDT amendment),
/// trusted exactly as far as it could approve the confirmation itself: its
/// own message (a voice-turn confirmation), or a command when it may
/// approve. From any other client the flag is ignored.
fn tapped(conn: &Conn, p: &common::AgentsSendParams) -> bool {
    p.confirmed
        && conn.is_companion()
        && (p.kind == common::SendKind::Message || conn.principal().can_approve)
}

/// `agents.send` with `kind: "message"` (plan 20261023_02, decisions 5 and
/// 6): checked by `policy::check_message`, then always a confirmation,
/// whose handle is settled as a voice turn's is. A companion's own message
/// is a `Lifetime::VoiceTurn` confirmation, which it may approve itself as
/// the user confirming what they dictated; its question is sent to it here,
/// since the forwarder leaves a companion's own voice-turn questions alone.
/// Approved, the text is sent as a command is: text, then Enter.
async fn send_message(
    conn: &Arc<Conn>,
    snap: &Value,
    pane_id: &str,
    agent: &str,
    p: common::AgentsSendParams,
) -> Result<Value, ErrorObject> {
    let daemon = &conn.daemon;
    let principal = conn.principal();
    // Paths in the text are checked against the agent's project.
    let root = crate::core::agents::agents(snap)
        .into_iter()
        .find(|a| crate::watch::pane_ids_match(&a.pane_id, pane_id))
        .and_then(|a| a.cwd);
    let checked = policy::check_message(
        principal,
        &daemon.agents,
        agent,
        pane_id,
        &p.command,
        root.as_deref(),
    )
    .map_err(|denied| ErrorObject::new(codes::NOT_PERMITTED, denied.to_string()))?;
    if tapped(conn, &p) {
        log::info!(
            "Message for pane '{pane_id}' confirmed by the tap ({} characters)",
            checked.command.len()
        );
        daemon
            .client
            .run_command(pane_id, &checked.command)
            .await
            .map_err(herdr_unavailable)?;
        return ok(common::AgentsSendResult::Sent);
    }
    let names = daemon.names(snap);
    let named = names.get(pane_id);
    let spoken = named
        .map(|a| a.spoken.clone())
        .unwrap_or_else(|| crate::core::naming::capitalise(agent));
    let tab_code = named.and_then(|a| {
        a.tab.as_ref().map(|tab| {
            if tab.word_form.is_empty() {
                tab.spoken.clone()
            } else {
                crate::core::naming::spell(&tab.word_form, &daemon.naming)
            }
        })
    });
    // Without a handle: "message" and the first words, then the agent.
    let first_words: Vec<&str> = checked.command.split(' ').take(3).collect();
    let fallback = format!("message {}", first_words.join(" "));
    let request = HandleRequest {
        given: p.handle.as_deref(),
        command: &fallback,
        agent: &spoken,
        tab_code: tab_code.as_deref(),
    };
    let lifetime = if conn.is_companion() {
        Lifetime::VoiceTurn
    } else {
        Lifetime::Standing
    };
    let priors = daemon.broker.word_priors();
    let mut attempt = 0;
    let (confirm_id, handle) = loop {
        let pending = daemon.broker.pending_handle_infos(&principal.id);
        let settled = confirm::settle_handle(&request, &pending, &priors, confirm::random_word);
        match daemon.broker.request_message(
            principal,
            pane_id,
            agent,
            &checked.command,
            Some(&settled),
            lifetime,
        ) {
            Ok(id) => break (id, settled.handle),
            Err(ConfirmError::DuplicateHandle(_)) if attempt < 3 => attempt += 1,
            Err(e) => return Err(confirm_error(e)),
        }
    };
    conn.opened.lock().unwrap().insert(confirm_id);
    if lifetime == Lifetime::VoiceTurn
        && let Some((view, token)) = daemon.broker.pending(confirm_id)
    {
        let pane_names = named.map(|a| super::PaneNames {
            name: a.shown.clone(),
            spoken: a.spoken.clone(),
            project: a.agent.project.clone(),
            tab: a.tab.as_ref().map(|t| t.shown.clone()),
        });
        conn.send(&Notification::new(
            methods::CONFIRM_REQUESTED,
            turns::confirm_requested(&view, &token, principal, false, pane_names),
        ));
    }
    log::info!(
        "Message confirmation {confirm_id} handle '{handle}' for pane '{pane_id}' ({} characters)",
        checked.command.len()
    );
    ok(common::AgentsSendResult::Pending {
        confirm_id,
        handle: Some(handle),
    })
}

/// One piece of a composed message (plan 20261028_05, decision 7.3).
enum Piece {
    Text(String),
    /// An image's absolute path, quoted when it leads the message (7.4).
    Path(String),
}

/// `agents.send` with `parts` (plan 20261028_05, decision 7 and the 21:29
/// EDT amendment, items 2-4): the parts checked by `policy::check_parts`,
/// then each image reference this principal holds for this pane replaced
/// by its file's absolute path, the first one quoted when the message
/// starts with it. The composed line is not run through `check_paths`.
/// A tap sends at once, the paths as herdr pastes when the attachment is
/// up, else typed; otherwise the line is confirmed as a message, its card
/// showing `[image N]` for each path.
async fn send_parts(
    conn: &Arc<Conn>,
    snap: &Value,
    pane_id: &str,
    agent: &str,
    p: common::AgentsSendParams,
) -> Result<Value, ErrorObject> {
    let daemon = &conn.daemon;
    let principal = conn.principal();
    if !p.command.trim().is_empty() || p.kind != common::SendKind::Message {
        return Err(ErrorObject::new(
            codes::INVALID_PARAMS,
            "a send with parts has an empty command and kind \"message\"",
        ));
    }
    let mut parts = Vec::with_capacity(p.parts.len());
    for part in &p.parts {
        parts.push(match part {
            common::SendPart::Text(t) => policy::Part::Text(t.clone()),
            common::SendPart::Image(r) => policy::Part::Image(r.clone()),
            _ => {
                return Err(ErrorObject::new(
                    codes::INVALID_PARAMS,
                    "a part this daemon does not know",
                ));
            }
        });
    }
    let root = crate::core::agents::agents(snap)
        .into_iter()
        .find(|a| crate::watch::pane_ids_match(&a.pane_id, pane_id))
        .and_then(|a| a.cwd);
    let checked = policy::check_parts(
        principal,
        &daemon.agents,
        agent,
        pane_id,
        &parts,
        root.as_deref(),
    )
    .map_err(|denied| ErrorObject::new(codes::NOT_PERMITTED, denied.to_string()))?;
    // Only paths the daemon made, from live references, are inserted.
    let store = &daemon.blob.images;
    let mut pieces = Vec::new();
    let mut references = Vec::new();
    let mut shown = Vec::new();
    for part in &checked.parts {
        match part {
            policy::Part::Text(t) => {
                shown.push(t.clone());
                pieces.push(Piece::Text(t.clone()));
            }
            policy::Part::Image(r) => {
                let path = store.check(r, pane_id, &principal.id).map_err(|e| {
                    ErrorObject::new(
                        codes::NOT_PERMITTED,
                        match e {
                            super::blob::images::RefError::NotYours => {
                                "That image was not uploaded for this session"
                            }
                            super::blob::images::RefError::Expired => {
                                "That image has expired; attach it again"
                            }
                        },
                    )
                })?;
                if references.contains(r) {
                    return Err(ErrorObject::new(
                        codes::NOT_PERMITTED,
                        "That image is in the message twice",
                    ));
                }
                references.push(r.clone());
                let path = path.to_string_lossy().into_owned();
                shown.push(format!("[image {}]", references.len()));
                pieces.push(Piece::Path(if pieces.is_empty() {
                    format!("\"{path}\"")
                } else {
                    path
                }));
            }
        }
    }
    let line = pieces
        .iter()
        .map(|p| match p {
            Piece::Text(t) | Piece::Path(t) => t.as_str(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let shown = shown.join(" ");
    let count = references.len();
    if tapped(conn, &p) {
        deliver_pieces(daemon, pane_id, &pieces, &line).await?;
        store.consume(&references);
        log::info!(
            "Message for pane '{pane_id}' with {count} image(s) confirmed by the tap ({} characters)",
            line.chars().count()
        );
        return ok(common::AgentsSendResult::Sent);
    }
    // Confirmed later: the approval types the line (no paste); the card
    // shows `[image N]`. The references are used now, by this send.
    let lifetime = if conn.is_companion() {
        Lifetime::VoiceTurn
    } else {
        Lifetime::Standing
    };
    let names = daemon.names(snap);
    let named = names.get(pane_id);
    let spoken = named
        .map(|a| a.spoken.clone())
        .unwrap_or_else(|| crate::core::naming::capitalise(agent));
    let first_words: Vec<&str> = shown.split(' ').take(3).collect();
    let fallback = format!("message {}", first_words.join(" "));
    let request = HandleRequest {
        given: p.handle.as_deref(),
        command: &fallback,
        agent: &spoken,
        tab_code: None,
    };
    let priors = daemon.broker.word_priors();
    let mut attempt = 0;
    let (confirm_id, handle) = loop {
        let pending = daemon.broker.pending_handle_infos(&principal.id);
        let settled = confirm::settle_handle(&request, &pending, &priors, confirm::random_word);
        match daemon.broker.request_message_typed(
            principal,
            pane_id,
            agent,
            &shown,
            &line,
            Some(&settled),
            lifetime,
        ) {
            Ok(id) => break (id, settled.handle),
            Err(ConfirmError::DuplicateHandle(_)) if attempt < 3 => attempt += 1,
            Err(e) => return Err(confirm_error(e)),
        }
    };
    store.consume(&references);
    conn.opened.lock().unwrap().insert(confirm_id);
    if lifetime == Lifetime::VoiceTurn
        && let Some((view, token)) = daemon.broker.pending(confirm_id)
    {
        let pane_names = named.map(|a| super::PaneNames {
            name: a.shown.clone(),
            spoken: a.spoken.clone(),
            project: a.agent.project.clone(),
            tab: a.tab.as_ref().map(|t| t.shown.clone()),
        });
        conn.send(&Notification::new(
            methods::CONFIRM_REQUESTED,
            turns::confirm_requested(&view, &token, principal, false, pane_names),
        ));
    }
    log::info!(
        "Message confirmation {confirm_id} handle '{handle}' for pane '{pane_id}' with {count} \
         image(s) ({} characters)",
        line.chars().count()
    );
    ok(common::AgentsSendResult::Pending {
        confirm_id,
        handle: Some(handle),
    })
}

/// A composed message into `pane_id`: with herdr's attachment up, in
/// order on it, each text run as keys, each path as a herdr paste
/// (bracketed when the pane has bracketed paste on), then Enter (the 21:29
/// EDT amendment, item 2); else `line` typed as any message is.
async fn deliver_pieces(
    daemon: &super::Daemon,
    pane_id: &str,
    pieces: &[Piece],
    line: &str,
) -> Result<(), ErrorObject> {
    use libherdr_term::KeyToken;
    use libherdr_term::herdr::{ClientKeyCode, ClientPaneInputEvent};
    if let Some(attach) = daemon.herdr_attach.as_ref().filter(|a| a.usable()) {
        let mut events: Vec<ClientPaneInputEvent> = Vec::new();
        for (i, piece) in pieces.iter().enumerate() {
            if i > 0 {
                events.extend(Vec::<ClientPaneInputEvent>::from(KeyToken::Text(
                    " ".into(),
                )));
            }
            match piece {
                Piece::Text(t) => {
                    events.extend(Vec::<ClientPaneInputEvent>::from(KeyToken::Text(t.clone())))
                }
                Piece::Path(path) => events.push(ClientPaneInputEvent::Paste(path.clone())),
            }
        }
        events.extend(Vec::<ClientPaneInputEvent>::from(KeyToken::Named(
            ClientKeyCode::Enter,
        )));
        match attach.input_pane(pane_id, events) {
            Ok(()) => return Ok(()),
            Err(libherdr_term::AttachError::Closed) => {}
            Err(e) => return Err(herdr_unavailable(e)),
        }
    }
    daemon
        .client
        .run_command(pane_id, line)
        .await
        .map_err(herdr_unavailable)?;
    Ok(())
}

/// `plugin.call` (plan 20261028_04, decision 3.2): answered at once; the
/// plugin starts once that answer is written, and its result and
/// notifications come on this connection only. Its project is the call's
/// `session`, else the conversation's current agent.
fn plugin_call(
    conn: &Arc<Conn>,
    request_id: &Id,
    call: companion::PluginCall,
) -> Result<Value, ErrorObject> {
    let pane = call
        .session
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| conn.session().current_agent.lock().unwrap().clone());
    let weak = Arc::downgrade(conn);
    let (answered, ready) = tokio::sync::oneshot::channel();
    let caller = crate::plugins::Caller {
        principal: conn.principal().clone(),
        pane,
        lookup: Arc::new(crate::plugins::HerdrLookup::new(conn.daemon.client.clone())),
        sink: Arc::new(move |note: Notification| {
            if let Some(conn) = weak.upgrade() {
                conn.send(&note);
            }
        }),
        answered: Some(ready),
    };
    let accepted = conn.daemon.plugins.call(call, caller, &conn.plugin_calls)?;
    conn.on_answered(request_id.clone(), answered);
    ok(accepted)
}

/// Send this connection the events it is entitled to: what it subscribed
/// to, confirmations if it can approve them, and the resolution of its own.
async fn forward_events(conn: Arc<Conn>) {
    let mut confirms = conn.daemon.broker.subscribe();
    let mut core = conn.daemon.events.subscribe();
    let principal = conn.principal().clone();
    let companion = conn.is_companion();
    // What `hello` said of `[llm]` is the starting point; a change is sent
    // as `server.capabilities` (plan 20261027_02, decision 7).
    let mut llm = conn.daemon.llm.subscribe();
    llm.mark_unchanged();
    let mut llm_open = true;
    let mut plugins = conn.daemon.plugins.subscribe();
    plugins.mark_unchanged();
    let mut plugins_open = true;
    let mut vocabulary_check = tokio::time::interval_at(
        tokio::time::Instant::now() + turns::VOCABULARY_INTERVAL,
        turns::VOCABULARY_INTERVAL,
    );
    loop {
        tokio::select! {
            _ = vocabulary_check.tick(), if companion => {
                turns::check_vocabulary(Arc::clone(&conn)).await;
            }
            changed = llm.changed(), if companion && llm_open => match changed {
                Ok(()) => {
                    let info = llm.borrow_and_update().clone();
                    conn.send(&Notification::new(
                        companion::methods::SERVER_CAPABILITIES,
                        companion::ServerCapabilities {
                            llm: Some(info),
                            terminal: super::terminal::info(&conn.daemon, conn.is_link()),
                            plugins: Some(conn.daemon.plugins.infos()),
                            blob: super::blob::info(&conn.daemon, conn.is_link()),
                        },
                    ));
                }
                Err(_) => llm_open = false,
            },
            // The plugin list changed: a toggle or a config reload (plan
            // 20261028_04, decision 3.2, item 5).
            changed = plugins.changed(), if companion && plugins_open => match changed {
                Ok(()) => {
                    let list = plugins.borrow_and_update().clone();
                    conn.send(&Notification::new(
                        companion::methods::SERVER_CAPABILITIES,
                        companion::ServerCapabilities {
                            llm: Some(conn.daemon.llm.info()),
                            terminal: super::terminal::info(&conn.daemon, conn.is_link()),
                            plugins: Some(list),
                            blob: super::blob::info(&conn.daemon, conn.is_link()),
                        },
                    ));
                }
                Err(_) => plugins_open = false,
            },
            event = confirms.recv() => match event {
                // A companion's own voice-turn question is sent by its turn,
                // after the reply.
                Ok(ref e) if companion && turns::is_own_voice_turn(e, &principal) => {}
                Ok(ConfirmEvent::Requested { view, token, reminder }) if companion => {
                    if principal.can_approve || conn.subscribed(topics::CONFIRM) {
                        let names = conn.daemon.names_for(&view.pane_id).await;
                        conn.send(&Notification::new(
                            methods::CONFIRM_REQUESTED,
                            turns::confirm_requested(&view, &token, &principal, reminder, names),
                        ));
                    }
                }
                Ok(ConfirmEvent::Requested { view, token, reminder }) => {
                    if principal.can_approve || conn.subscribed(topics::CONFIRM) {
                        let names = conn.daemon.names_for(&view.pane_id).await;
                        let (name, spoken, project) = match names {
                            Some(n) => (Some(n.name), Some(n.spoken), n.project),
                            None => (None, None, None),
                        };
                        conn.send(&Notification::new(
                            methods::CONFIRM_REQUESTED,
                            common::ConfirmRequested {
                                confirm_id: view.confirm_id,
                                pane_id: view.pane_id,
                                agent: view.agent,
                                name,
                                spoken,
                                project,
                                command: view.command,
                                requested_by: view.requested_by,
                                handle: view.handle,
                                token: token.reveal_to(&principal).map(str::to_string),
                                reminder,
                            },
                        ));
                    }
                }
                Ok(ConfirmEvent::Resolved { view, outcome, detail }) => {
                    let own = conn.opened.lock().unwrap().remove(&view.confirm_id);
                    if own || principal.can_approve || conn.subscribed(topics::CONFIRM) {
                        conn.send(&Notification::new(
                            methods::CONFIRM_RESOLVED,
                            common::ConfirmResolved {
                                confirm_id: view.confirm_id,
                                handle: view.handle,
                                outcome: wire_outcome(outcome),
                                detail,
                            },
                        ));
                    }
                    if companion {
                        turns::flush_notices(&conn);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("{} missed {n} confirmation event(s)", principal.id);
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            event = core.recv() => match event {
                // herdr's focus moved: a companion hears at once (plan
                // 20261023_03, decision 12).
                Ok(CoreEvent::Focus { .. }) if companion => {
                    tokio::spawn(turns::check_vocabulary(Arc::clone(&conn)));
                }
                Ok(CoreEvent::Focus { .. })
                | Ok(CoreEvent::Pane { .. })
                | Ok(CoreEvent::Output { .. }) => {}
                Ok(CoreEvent::AgentStatus { pane_id, agent, status }) if companion => {
                    if conn.daemon.watches.is_watched(&pane_id) {
                        turns::agent_status(&conn, &pane_id, &agent, &status).await;
                    }
                }
                Ok(CoreEvent::AgentStatus { pane_id, agent, status }) => {
                    if conn.subscribed(topics::AGENT_STATUS) {
                        conn.send(&Notification::new(
                            methods::AGENT_STATUS_CHANGED,
                            common::AgentStatusChanged {
                                pane_id,
                                agent,
                                status: wire_status(&status),
                            },
                        ));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("{} missed {n} agent event(s)", principal.id);
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_daemons_own_uid_may_connect() {
        assert!(peer_allowed(501, 501));
        assert!(!peer_allowed(502, 501));
        assert!(!peer_allowed(0, 501));
    }
}
