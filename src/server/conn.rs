//! One client connection: `hello`, then the common methods
//! (plan 20261022_01, decisions 4, 7 and 8).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use futures_util::{Stream, StreamExt};
use libhandlr_proto::common::{self, methods, protocols, topics};
use libhandlr_proto::envelope::codes;
use libhandlr_proto::link::{LinkError, LinkStream, PairingId};
use libhandlr_proto::{ErrorObject, Id, Incoming, Notification, Request, Response};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpStream, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::AbortHandle;
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::sync::CancellationToken;

use super::link::auth::AuthEntry;
use super::{CoreEvent, Daemon};
use crate::core::agents::{self, extract_agent_name_for_pane};
use crate::core::confirm::{ConfirmError, ConfirmEvent, Lifetime, Outcome, StatusKey};
use crate::core::naming::ResolveError;
use crate::core::policy;
use crate::core::principal::Principal;

/// The longest line a client may send: 1 MiB.
pub const MAX_LINE: usize = 1 << 20;

/// The protocol versions served.
const HARNESS_VERSION: u32 = 1;

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
        /// It paired on this connection; `hello` answers `paired: true`.
        paired_now: bool,
    },
}

struct Conn {
    daemon: Arc<Daemon>,
    origin: Origin,
    out: mpsc::UnboundedSender<String>,
    principal: OnceLock<Principal>,
    topics: Mutex<HashSet<String>>,
    inflight: Mutex<HashMap<Id, AbortHandle>>,
    /// Confirmations opened on this connection, cancelled when it closes.
    opened: Mutex<HashSet<u64>>,
    /// Pairings begun on this connection, withdrawn when it closes.
    pairings: Mutex<Vec<PairingId>>,
}

impl Conn {
    fn send(&self, value: &impl serde::Serialize) {
        if let Ok(line) = serde_json::to_string(value) {
            let _ = self.out.send(line);
        }
    }

    fn principal(&self) -> &Principal {
        self.principal.get().expect("only called after hello")
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
    let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(mut line) = out_rx.recv().await {
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let lines = FramedRead::new(read, LinesCodec::new_with_max_length(MAX_LINE))
        .map(|line| line.map_err(|e| format!("unreadable line: {e}")));
    serve(daemon, Origin::Local, lines, out, CancellationToken::new()).await;
    let _ = writer.await;
}

/// A link connection that finished its handshake as `entry`. `first` is
/// the message carried in the handshake, already checked to be `hello`.
/// Each link message is one JSON-RPC object; `close` ends the connection
/// (its entry was revoked or replaced).
pub(crate) async fn handle_link(
    daemon: Arc<Daemon>,
    stream: LinkStream<TcpStream>,
    first: Option<Vec<u8>>,
    entry: AuthEntry,
    paired_now: bool,
    close: CancellationToken,
) {
    let (reader, mut writer) = stream.into_split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
    let write_task = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if writer.send_message(message.as_bytes()).await.is_err() {
                return;
            }
        }
        let _ = writer.shutdown().await;
    });
    let first = futures_util::stream::iter(first.map(decode_message));
    let rest = futures_util::stream::unfold(Some(reader), |reader| async move {
        let mut reader = reader?;
        match reader.recv_message().await {
            Ok(message) => Some((decode_message(message), Some(reader))),
            Err(LinkError::Closed) => None,
            Err(e) => Some((Err(e.to_string()), None)),
        }
    });
    serve(
        daemon,
        Origin::Link { entry, paired_now },
        Box::pin(first.chain(rest)),
        out,
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
    out: mpsc::UnboundedSender<String>,
    close: CancellationToken,
) where
    S: Stream<Item = Result<String, String>> + Unpin,
{
    let conn = Arc::new(Conn {
        daemon,
        origin,
        out,
        principal: OnceLock::new(),
        topics: Mutex::new(HashSet::new()),
        inflight: Mutex::new(HashMap::new()),
        opened: Mutex::new(HashSet::new()),
        pairings: Mutex::new(Vec::new()),
    });
    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        let line = tokio::select! {
            _ = close.cancelled() => break,
            line = lines.next() => line,
        };
        let Some(line) = line else { break };
        let line = match line {
            Ok(line) => line,
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
            match hello(&request, &conn.origin) {
                Ok((principal, result)) => {
                    log::info!("Connection says hello as {}", principal.id);
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
            let response = match call(&task_conn, &request.method, request.params).await {
                Ok(result) => Response::ok(request.id.clone(), result),
                Err(error) => Response::err(Some(request.id.clone()), error),
            };
            task_conn.inflight.lock().unwrap().remove(&request.id);
            task_conn.send(&response);
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
    if let Some(principal) = conn.principal.get() {
        let opened: Vec<u64> = conn.opened.lock().unwrap().drain().collect();
        for confirm_id in opened {
            let _ = conn.daemon.broker.cancel(principal, confirm_id);
        }
        log::info!("Connection from {} closed", principal.id);
    }
    if let Some(link) = &conn.daemon.link {
        let pairings: Vec<PairingId> = conn.pairings.lock().unwrap().drain(..).collect();
        link.withdraw(&pairings);
    }
    if let Some(forwarder) = forwarder {
        forwarder.abort();
    }
}

fn hello(request: &Request, origin: &Origin) -> Result<(Principal, Value), ErrorObject> {
    let params: common::HelloParams = params(request.params.clone())?;
    if let Origin::Link { entry, .. } = origin {
        if !entry.allows(&params.protocol) {
            return Err(ErrorObject::new(
                codes::NOT_PERMITTED,
                format!(
                    "'{}' is not paired for {}; it may use {:?}",
                    entry.name, params.protocol, entry.protocols
                ),
            ));
        }
    }
    if params.protocol != protocols::HARNESS || params.version != HARNESS_VERSION {
        return Err(ErrorObject::new(
            codes::UNSUPPORTED_PROTOCOL,
            format!(
                "unsupported protocol {} version {}; this daemon serves {} version {}",
                params.protocol,
                params.version,
                protocols::HARNESS,
                HARNESS_VERSION
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
    let (principal, paired_now) = match origin {
        Origin::Local => (Principal::local(client), false),
        Origin::Link { entry, paired_now } => (entry.principal(&params.protocol), *paired_now),
    };
    let mut result = ok(common::HelloResult {
        server: "handlr".to_string(),
        server_version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: HARNESS_VERSION,
    })?;
    if paired_now {
        // Plan 20261022_02, decision 5: the first hello after pairing.
        if let Value::Object(map) = &mut result {
            map.insert("paired".to_string(), Value::Bool(true));
        }
    }
    Ok((principal, result))
}

fn cancel(conn: &Conn, request: &Request) -> Response {
    let target: common::CancelParams = match params(request.params.clone()) {
        Ok(target) => target,
        Err(error) => return Response::err(Some(request.id.clone()), error),
    };
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

fn params<T: DeserializeOwned>(value: Value) -> Result<T, ErrorObject> {
    let value = if value.is_null() {
        Value::Object(Default::default())
    } else {
        value
    };
    serde_json::from_value(value)
        .map_err(|e| ErrorObject::new(codes::INVALID_PARAMS, format!("invalid params: {e}")))
}

fn ok(value: impl serde::Serialize) -> Result<Value, ErrorObject> {
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

fn wire_status(status: &libherdr::protocol::AgentStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

async fn snapshot(conn: &Conn) -> Result<Value, ErrorObject> {
    conn.daemon
        .client
        .list_panes()
        .await
        .map_err(herdr_unavailable)
}

/// `-32002` for a target that names no pane or more than one. An
/// ambiguous target's message, and `data.candidates`, list the panes by
/// their shown names.
fn resolve_error(e: ResolveError) -> ErrorObject {
    let mut error = ErrorObject::new(codes::UNKNOWN_AGENT, e.to_string());
    if let ResolveError::Ambiguous { candidates, .. } = &e {
        error.data = Some(serde_json::json!({ "candidates": candidates }));
    }
    error
}

/// Resolve a method's target to a pane id (plan 20261022_05, decision 10):
/// a pane id, a tab label as written, an alias value, a spoken form or its
/// capitals, or a tab number.
fn resolve_target(daemon: &Daemon, snap: &Value, target: &str) -> Result<String, ErrorObject> {
    daemon
        .names(snap)
        .resolve(target)
        .map(|a| a.agent.pane_id.clone())
        .map_err(resolve_error)
}

/// Whether `target` is, as written, a workspace or tab id in `snap`.
/// `agents.focus` passes those through unresolved.
fn is_workspace_or_tab_id(snap: &Value, target: &str) -> bool {
    let target = target.trim();
    agents::tab_infos(snap).contains_key(target)
        || agents::snapshot_root(snap)
            .get("workspaces")
            .and_then(|w| w.as_array())
            .into_iter()
            .flatten()
            .any(|ws| ws.get("workspace_id").and_then(|v| v.as_str()) == Some(target))
}

async fn call(conn: &Conn, method: &str, raw: Value) -> Result<Value, ErrorObject> {
    let daemon = &conn.daemon;
    match method {
        methods::PING => ok(common::Empty {}),
        methods::HELLO => Err(ErrorObject::new(
            codes::INVALID_REQUEST,
            "hello was already said on this connection",
        )),
        methods::AGENTS_LIST => {
            let snap = snapshot(conn).await?;
            let agents = daemon
                .names(&snap)
                .agents
                .into_iter()
                .map(|named| {
                    let a = named.agent;
                    common::AgentInfo {
                        status: wire_status(&a.status),
                        pane_id: a.pane_id,
                        name: Some(named.shown),
                        spoken: Some(named.spoken),
                        agent: a.agent,
                        project: a.project,
                        cwd: a.cwd,
                        workspace_id: a.workspace_id,
                        tab_id: a.tab_id,
                        tab_label: a.tab_label,
                        tab_number: a.tab_number,
                        focused: a.focused,
                    }
                })
                .collect();
            ok(common::AgentsListResult { agents })
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
        methods::AGENTS_SEND => {
            let p: common::AgentsSendParams = params(raw)?;
            let snap = snapshot(conn).await?;
            let pane_id = resolve_target(daemon, &snap, &p.pane_id)?;
            let agent = extract_agent_name_for_pane(&snap, &pane_id);
            let principal = conn.principal();
            let checked = policy::check(principal, &daemon.agents, &agent, &pane_id, &p.command)
                .map_err(|denied| ErrorObject::new(codes::NOT_PERMITTED, denied.to_string()))?;
            if checked.require_confirmation {
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
            let target = if is_workspace_or_tab_id(&snap, &p.target_id) {
                p.target_id.trim().to_string()
            } else {
                resolve_target(daemon, &snap, &p.target_id)?
            };
            daemon
                .client
                .focus(&target)
                .await
                .map_err(herdr_unavailable)?;
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
                    ))
                }
            };
            let (view, outcome) = daemon
                .broker
                .status(conn.principal(), &key)
                .ok_or_else(|| confirm_error(ConfirmError::NotFound))?;
            ok(common::ConfirmStatusResult {
                confirm_id: view.confirm_id,
                handle: view.handle,
                outcome: wire_outcome(outcome),
            })
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
        methods::LINK_PAIR_BEGIN => {
            if !matches!(conn.origin, Origin::Local) {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "link.pair_begin is served only on the daemon socket",
                ));
            }
            let Some(link) = &daemon.link else {
                return Err(ErrorObject::new(
                    codes::NOT_PERMITTED,
                    "the link is off ([link] listen = [])",
                ));
            };
            let p: common::LinkPairBeginParams = params(raw)?;
            let begun = link
                .pair_begin(p)
                .await
                .map_err(|e| ErrorObject::new(codes::INVALID_PARAMS, e))?;
            conn.pairings.lock().unwrap().push(begun.id);
            let out = conn.out.clone();
            let done = begun.done;
            tokio::spawn(async move {
                if let Ok(done) = done.await {
                    if let Ok(line) =
                        serde_json::to_string(&Notification::new(methods::LINK_PAIR_DONE, done))
                    {
                        let _ = out.send(line);
                    }
                }
            });
            ok(begun.result)
        }
        other => Err(ErrorObject::new(
            codes::METHOD_NOT_FOUND,
            format!("unknown method '{other}'"),
        )),
    }
}

/// Send this connection the events it is entitled to: what it subscribed
/// to, confirmations if it can approve them, and the resolution of its own.
async fn forward_events(conn: Arc<Conn>) {
    let mut confirms = conn.daemon.broker.subscribe();
    let mut core = conn.daemon.events.subscribe();
    let principal = conn.principal().clone();
    loop {
        tokio::select! {
            event = confirms.recv() => match event {
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
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("{} missed {n} confirmation event(s)", principal.id);
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            event = core.recv() => match event {
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
