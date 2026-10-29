//! Binary transfers over the link's kind 2
//! (docs/design/20261028_05_BINARY_FILES_PLAN.md, decisions 3-8): the
//! per-connection transfer table, its timeouts on a 1 s tick, and the
//! built-in namespaces `images` (phone to daemon, [`images`]), `files`
//! ([`files`]) and `git` ([`git`]), both daemon to phone.
//!
//! Kind 2 is read only from a companion over the link; the daemon sends it
//! only in answer to a message the phone sent, so only to a phone that
//! reads it. A message whose header cannot be read closes the connection;
//! one with a bad body is answered once with CANCEL `protocol`.
//! CHUNKs go to the writer's bulk queue, at most two per transfer at a
//! time; everything else to the control queue, which the writer drains
//! first.

pub mod files;
pub mod git;
pub mod images;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Instant, SystemTime};

use libflankr_blob::{
    ACTIVE_PER_SIDE, BlobMessage, Body, CHUNK_DEFAULT, CHUNK_MAX, Code, DecodeError, FILE_MAX,
    GIT_MAX, IMAGE_MAX, IMAGE_MIN, IMAGES_PER_SEND, ImageType, NS_FILES, NS_GIT, NS_IMAGES,
    Namespace, Offer, PROTOCOL_VERSION, Receiving, Sending, Step, TransferId,
};
use libflankr_proto::companion::BlobInfo;
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

use super::Daemon;
use super::conn::Conn;
use crate::core::agents::extract_agent_name_for_pane;
use crate::core::policy;

/// The daemon's half of binary transfers, shared by every connection.
pub struct BlobService {
    pub images: images::ImageStore,
    /// `DAEMON_GIT_RUNNING` `git` processes at once.
    git_running: Arc<tokio::sync::Semaphore>,
}

impl BlobService {
    /// The store under `temp` (the daemon's `temp_dir()`, read once at
    /// start), for `uid`.
    pub fn new(temp: PathBuf, uid: u32) -> BlobService {
        BlobService {
            images: images::ImageStore::new(temp, uid),
            git_running: Arc::new(tokio::sync::Semaphore::new(
                libflankr_blob::DAEMON_GIT_RUNNING,
            )),
        }
    }
}

/// The `[llm.uses.planning] read` patterns as configured (decision 6.1).
fn read_patterns(daemon: &Daemon) -> Vec<crate::llm::ReadPattern> {
    daemon.llm.settings().planning.read
}

/// `blob` for `hello` and `server.capabilities` (decision 3.3): on a link
/// connection only.
pub fn info(daemon: &Daemon, over_link: bool) -> Option<BlobInfo> {
    if !over_link {
        return None;
    }
    let read = read_patterns(daemon);
    let mut namespaces = Vec::new();
    if !daemon.blob.images.is_off() {
        namespaces.push(NS_IMAGES.to_string());
    }
    if !read.is_empty() {
        namespaces.push(NS_FILES.to_string());
    }
    if crate::git::binary().is_some() {
        namespaces.push(NS_GIT.to_string());
    }
    Some(BlobInfo {
        version: PROTOCOL_VERSION,
        namespaces,
        read: read.into_iter().map(|p| p.text).collect(),
        image_max: IMAGE_MAX,
        file_max: FILE_MAX,
        git_max: GIT_MAX,
        chunk_max: CHUNK_MAX,
        images_per_send: IMAGES_PER_SEND as u32,
    })
}

/// A session's pane, its agent and its folder, resolved as `agents.send`
/// resolves its target (decision 4.8), and reachable by this principal.
pub(super) struct Session {
    pub pane_id: String,
    pub cwd: Option<String>,
}

type Refusal = (Code, String);

/// `session` against a fresh snapshot: `no_session` when it names no pane,
/// `not_permitted` when the agent has no `[[agents]]` entry this principal
/// may reach.
pub(super) async fn session(conn: &Conn, session: &str) -> Result<Session, Refusal> {
    let daemon = &conn.daemon;
    let snap = daemon
        .client
        .list_panes()
        .await
        .map_err(|e| (Code::IO, format!("herdr is not answering: {e}")))?;
    let pane_id = super::conn::resolve_target(daemon, &snap, session)
        .map_err(|_| (Code::NO_SESSION, format!("no session '{session}'")))?;
    let agent = extract_agent_name_for_pane(&snap, &pane_id);
    policy::reach(conn.principal(), &daemon.agents, &agent, &pane_id)
        .map_err(|denied| (Code::NOT_PERMITTED, denied.to_string()))?;
    let cwd = crate::core::agents::agents(&snap)
        .into_iter()
        .find(|a| crate::watch::pane_ids_match(&a.pane_id, &pane_id))
        .and_then(|a| a.cwd);
    Ok(Session { pane_id, cwd })
}

/// The session's folder on this machine, or `not_permitted`.
pub(super) fn local_root(conn: &Conn, s: &Session) -> Result<String, Refusal> {
    crate::project::check_local(&conn.daemon.client).map_err(|e| (Code::NOT_PERMITTED, e))?;
    s.cwd.clone().ok_or_else(|| {
        (
            Code::NOT_PERMITTED,
            format!("the folder of pane {} is not known", s.pane_id),
        )
    })
}

/// One live transfer.
enum Live {
    /// A REQUEST or OFFER being checked; its task.
    Pending { ns: Namespace, task: AbortHandle },
    /// An image coming in.
    Upload(Box<Upload>),
    /// A file or `git` output going out.
    Download(Download),
}

impl Live {
    fn ns(&self) -> &Namespace {
        match self {
            Live::Pending { ns, .. } => ns,
            Live::Upload(u) => &u.ns,
            Live::Download(d) => &d.ns,
        }
    }
}

struct Upload {
    ns: Namespace,
    rx: Receiving,
    /// Taken while a CHUNK is written on the blocking pool.
    part: Option<images::Part>,
    pane_id: String,
    started: Instant,
    _slot: images::ActiveSlot,
}

struct Download {
    ns: Namespace,
    tx: Arc<Mutex<Sending>>,
    bytes: Arc<Vec<u8>>,
    task: Option<AbortHandle>,
    started: Instant,
    /// For the end's log line: the path or the request.
    what: String,
}

impl Download {
    fn stop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

#[derive(Default)]
struct Table {
    live: HashMap<TransferId, Live>,
    /// Ids answered with CANCEL `unknown_transfer` or `protocol`, once each.
    answered: HashSet<TransferId>,
}

impl Table {
    /// Live transfers the phone started.
    fn phone_started(&self) -> usize {
        self.live
            .keys()
            .filter(|id| !id.started_by_daemon())
            .count()
    }

    fn remove(&mut self, id: TransferId) -> Option<Live> {
        let mut gone = self.live.remove(&id);
        match &mut gone {
            Some(Live::Download(d)) => d.stop(),
            Some(Live::Pending { task, .. }) => task.abort(),
            _ => {}
        }
        gone
    }
}

/// One connection's transfers.
#[derive(Default)]
pub struct Transfers {
    table: Mutex<Table>,
    tick: Mutex<Option<AbortHandle>>,
}

impl Transfers {
    /// The connection closed: every transfer ends, nothing resumes, and a
    /// partial image is deleted (decision 4.4).
    pub(super) fn close(&self) {
        let mut table = self.table.lock().unwrap();
        let ids: Vec<TransferId> = table.live.keys().copied().collect();
        for id in ids {
            table.remove(id);
        }
        drop(table);
        if let Some(t) = self.tick.lock().unwrap().take() {
            t.abort();
        }
    }
}

fn send(conn: &Conn, id: TransferId, ns: &Namespace, body: Body) {
    log_refusal(conn, id, ns, &body);
    conn.send_binary(BlobMessage::new(id, ns.clone(), body).encode(), false, None);
}

fn log_refusal(conn: &Conn, id: TransferId, ns: &Namespace, body: &Body) {
    if let Body::Refuse { code, reason } | Body::Cancel { code, reason } = body {
        let what = if matches!(body, Body::Refuse { .. }) {
            "REFUSE"
        } else {
            "CANCEL"
        };
        log::info!(
            "Transfer {id} ({ns}) on connection {}: {what} {code} ({reason})",
            conn.id()
        );
    }
}

/// One kind-2 message from the phone, applied before the next is read
/// (decision 4.5). `Err` closes the connection: an unreadable header.
pub(super) async fn on_message(conn: &Arc<Conn>, bytes: Vec<u8>) -> Result<(), ()> {
    let msg = match BlobMessage::decode(&bytes) {
        Ok(msg) => msg,
        Err(DecodeError::Fatal) => {
            log::warn!(
                "Connection {} closed: an unreadable binary message header",
                conn.id()
            );
            return Err(());
        }
        Err(DecodeError::Answer(id, ns)) => {
            let first = {
                let mut table = conn.blob.table.lock().unwrap();
                table.remove(id);
                table.answered.insert(id)
            };
            if first {
                send(
                    conn,
                    id,
                    &ns,
                    Body::cancel(Code::PROTOCOL, "bad message body"),
                );
            }
            return Ok(());
        }
    };
    start_tick(conn);
    let BlobMessage {
        transfer: id,
        ns,
        body,
    } = msg;
    log::trace!("Transfer {id} ({ns}): {:?}", body.subcommand());
    // A live transfer under another namespace is `protocol`.
    let wrong_ns = {
        let table = conn.blob.table.lock().unwrap();
        table.live.get(&id).is_some_and(|l| l.ns() != &ns)
    };
    if wrong_ns {
        conn.blob.table.lock().unwrap().remove(id);
        send(
            conn,
            id,
            &ns,
            Body::cancel(Code::PROTOCOL, "wrong namespace"),
        );
        return Ok(());
    }
    match body {
        Body::Request {
            session,
            request,
            max_size,
        } => start(
            conn,
            id,
            ns,
            Started::Request {
                session,
                request,
                max_size,
            },
        ),
        Body::Offer(offer) => start(conn, id, ns, Started::Offer(offer)),
        Body::Chunk { offset, data } => chunk(conn, id, &ns, offset, data).await,
        Body::End { size } => end(conn, id, &ns, size).await,
        other => answer(conn, id, &ns, other),
    }
    Ok(())
}

/// The 1 s tick of this connection's timeouts, started with its first
/// kind-2 message.
fn start_tick(conn: &Arc<Conn>) {
    let mut tick = conn.blob.tick.lock().unwrap();
    if tick.is_some() {
        return;
    }
    let weak: Weak<Conn> = Arc::downgrade(conn);
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            interval.tick().await;
            let Some(conn) = weak.upgrade() else { return };
            on_tick(&conn, Instant::now());
        }
    });
    *tick = Some(task.abort_handle());
}

/// Every live transfer's clock (decision 4.4).
fn on_tick(conn: &Conn, now: Instant) {
    let mut aborts = Vec::new();
    {
        let mut table = conn.blob.table.lock().unwrap();
        for (id, live) in table.live.iter_mut() {
            let step = match live {
                Live::Upload(u) => u.rx.on_tick(now),
                Live::Download(d) => d.tx.lock().unwrap().on_tick(now),
                Live::Pending { .. } => Step::Continue,
            };
            if let Step::Abort(body) = step {
                aborts.push((*id, live.ns().clone(), body));
            }
        }
        for (id, _, _) in &aborts {
            table.remove(*id);
        }
    }
    for (id, ns, body) in aborts {
        send(conn, id, &ns, body);
    }
}

enum Started {
    Request {
        session: String,
        request: String,
        max_size: u32,
    },
    Offer(Offer),
}

/// A REQUEST or OFFER: checked for its id and the limits here, then by its
/// namespace in a task, so reading goes on.
fn start(conn: &Arc<Conn>, id: TransferId, ns: Namespace, started: Started) {
    let refuse = |code: Code, why: &str| send(conn, id, &ns, Body::refuse(code, why));
    if id.started_by_daemon() {
        return refuse(Code::PROTOCOL, "a daemon id from the phone");
    }
    let is_request = matches!(started, Started::Request { .. });
    // Which namespaces take which first message.
    let served = match (ns.as_str(), is_request) {
        (NS_FILES, true) => !read_patterns(&conn.daemon).is_empty(),
        (NS_GIT, true) => crate::git::binary().is_some(),
        (NS_IMAGES, false) => true,
        _ => false,
    };
    let mut table = conn.blob.table.lock().unwrap();
    if table.live.contains_key(&id) {
        drop(table);
        return refuse(Code::PROTOCOL, "the id is in use");
    }
    if !served {
        drop(table);
        // `files` with `read` empty says so (decision 6.1).
        if ns.as_str() == NS_FILES && is_request {
            return refuse(Code::FILES_OFF, "[llm.uses.planning] read is empty");
        }
        return refuse(Code::UNSUPPORTED, "not served");
    }
    if table.phone_started() >= ACTIVE_PER_SIDE {
        drop(table);
        return refuse(Code::BUSY, "too many transfers");
    }
    let task_conn = Arc::clone(conn);
    let task_ns = ns.clone();
    // The task waits until its Pending entry is in the table.
    let (go, ready) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        if ready.await.is_err() {
            return;
        }
        let conn = task_conn;
        match started {
            Started::Request {
                session,
                request,
                max_size,
            } => {
                let max = u64::from(max_size);
                let result = if task_ns.as_str() == NS_FILES {
                    files::request(&conn, &session, &request, max).await
                } else {
                    git::request(&conn, &session, &request, max).await
                };
                offer_download(&conn, id, task_ns, &session, result);
            }
            Started::Offer(offer) => accept_upload(&conn, id, task_ns, offer).await,
        }
    });
    table.live.insert(
        id,
        Live::Pending {
            ns,
            task: task.abort_handle(),
        },
    );
    drop(table);
    let _ = go.send(());
}

/// A file's or `git` output's OFFER, or the REFUSE (decision 6.3, 6.10).
fn offer_download(
    conn: &Arc<Conn>,
    id: TransferId,
    ns: Namespace,
    session: &str,
    result: Result<(String, &'static str, Vec<u8>, u64), Refusal>,
) {
    let mut table = conn.blob.table.lock().unwrap();
    // Cancelled or timed out meanwhile.
    if !matches!(table.live.get(&id), Some(Live::Pending { .. })) {
        return;
    }
    let (name, mime, bytes, modified_ms) = match result {
        Ok(r) => r,
        Err((code, reason)) => {
            table.live.remove(&id);
            drop(table);
            return send(conn, id, &ns, Body::refuse(code, reason));
        }
    };
    let offer = Offer::for_bytes(session, &name, mime, &bytes, modified_ms);
    let now = Instant::now();
    log::info!(
        "Transfer {id} ({ns}) on connection {} starts: pane '{session}', {mime}, {} bytes, {}",
        conn.id(),
        offer.size,
        libflankr_blob::hex(&offer.sha256[..4])
    );
    let download = Download {
        ns: ns.clone(),
        tx: Arc::new(Mutex::new(Sending::new(&offer, now))),
        bytes: Arc::new(bytes),
        task: None,
        started: now,
        what: name,
    };
    table.live.insert(id, Live::Download(download));
    drop(table);
    send(conn, id, &ns, Body::Offer(offer));
}

/// An image OFFER's checks (decision 5.5), then ACCEPT or REFUSE.
async fn accept_upload(conn: &Arc<Conn>, id: TransferId, ns: Namespace, offer: Offer) {
    let refuse = |code: Code, why: String| {
        conn.blob.table.lock().unwrap().live.remove(&id);
        send(conn, id, &ns, Body::refuse(code, why));
    };
    let store = &conn.daemon.blob.images;
    if store.is_off() {
        return refuse(Code::IO, "the image store is off".into());
    }
    if ImageType::from_mime(&offer.mime).is_none() {
        return refuse(
            Code::BAD_TYPE,
            format!("{} is not an image type taken", offer.mime),
        );
    }
    if offer.size > IMAGE_MAX {
        return refuse(Code::TOO_LARGE, format!("{} bytes", offer.size));
    }
    if offer.size < IMAGE_MIN {
        return refuse(Code::BAD_TYPE, "too small to be an image".into());
    }
    let s = match session(conn, &offer.session).await {
        Ok(s) => s,
        Err((code, why)) => return refuse(code, why),
    };
    if let Err((code, why)) = local_root(conn, &s) {
        return refuse(code, why);
    }
    let Some(slot) = store.take_slot() else {
        return refuse(Code::BUSY, "too many images at once".into());
    };
    let daemon = Arc::clone(&conn.daemon);
    let (size, pane) = (offer.size, s.pane_id.clone());
    let made = tokio::task::spawn_blocking(move || {
        daemon.blob.images.cleanup(size)?;
        daemon.blob.images.create_part(&pane)
    })
    .await
    .unwrap_or(Err(Code::IO));
    let part = match made {
        Ok(part) => part,
        Err(code) => return refuse(code, "the image cannot be stored".into()),
    };
    let now = Instant::now();
    let mut rx = Receiving::offered(offer.clone(), now);
    let accept = rx.accept(CHUNK_MAX, now).expect("deciding");
    let mut table = conn.blob.table.lock().unwrap();
    if !matches!(table.live.get(&id), Some(Live::Pending { .. })) {
        return;
    }
    log::info!(
        "Transfer {id} ({ns}) on connection {} starts: pane '{}', {}, {} bytes, {}",
        conn.id(),
        s.pane_id,
        offer.mime,
        offer.size,
        libflankr_blob::hex(&offer.sha256[..4])
    );
    log::debug!("Transfer {id}: ACCEPT chunk_max {CHUNK_MAX}");
    table.live.insert(
        id,
        Live::Upload(Box::new(Upload {
            ns: ns.clone(),
            rx,
            part: Some(part),
            pane_id: s.pane_id,
            started: now,
            _slot: slot,
        })),
    );
    drop(table);
    send(conn, id, &ns, accept);
}

/// A message for an id that is not live: CHUNK, END and ACCEPT are
/// answered with CANCEL `unknown_transfer` once per id; the rest ignored
/// (decision 4.3).
fn unknown(conn: &Conn, id: TransferId, ns: &Namespace, answer: bool) {
    if answer && conn.blob.table.lock().unwrap().answered.insert(id) {
        send(
            conn,
            id,
            ns,
            Body::cancel(Code::UNKNOWN_TRANSFER, "no such transfer"),
        );
    }
}

/// Ends `id` with CANCEL `protocol`.
fn protocol(conn: &Conn, id: TransferId, ns: &Namespace, why: &str) {
    conn.blob.table.lock().unwrap().remove(id);
    send(conn, id, ns, Body::cancel(Code::PROTOCOL, why));
}

/// An image's CHUNK: checked and hashed, its first bytes sniffed (decision
/// 5.4), then written on the blocking pool before the next message.
async fn chunk(conn: &Arc<Conn>, id: TransferId, ns: &Namespace, offset: u64, data: Vec<u8>) {
    log::trace!("Transfer {id}: CHUNK at {offset}, {} bytes", data.len());
    let part = {
        let mut table = conn.blob.table.lock().unwrap();
        let Some(live) = table.live.get_mut(&id) else {
            drop(table);
            return unknown(conn, id, ns, true);
        };
        let Live::Upload(up) = live else {
            let why = if matches!(live, Live::Pending { .. }) {
                "a CHUNK before ACCEPT"
            } else {
                "a CHUNK for a transfer the phone does not send"
            };
            drop(table);
            return protocol(conn, id, ns, why);
        };
        let first = up.rx.received() == 0;
        if let Err(code) = up.rx.chunk(offset, &data, Instant::now()) {
            table.remove(id);
            drop(table);
            return send(
                conn,
                id,
                ns,
                Body::cancel(code, "chunk out of order or too large"),
            );
        }
        if first {
            let want = up.rx.offer().and_then(|o| ImageType::from_mime(&o.mime));
            let got = ImageType::sniff(&data);
            if want.is_none() || got != want {
                table.remove(id);
                drop(table);
                return send(
                    conn,
                    id,
                    ns,
                    Body::cancel(Code::BAD_TYPE, "the bytes are not the offered type"),
                );
            }
        }
        match up.part.take() {
            Some(part) => part,
            None => {
                drop(table);
                return protocol(conn, id, ns, "a CHUNK while one is written");
            }
        }
    };
    let written = tokio::task::spawn_blocking(move || {
        let mut part = part;
        let r = part.write(&data);
        (part, r)
    })
    .await;
    let failed = match written {
        Ok((part, Ok(()))) => {
            let mut table = conn.blob.table.lock().unwrap();
            if let Some(Live::Upload(up)) = table.live.get_mut(&id) {
                up.part = Some(part);
            }
            // Else it ended meanwhile, and the part is dropped: deleted.
            None
        }
        Ok((_, Err((_, why)))) => Some(why),
        Err(e) => Some(e.to_string()),
    };
    if let Some(why) = failed {
        conn.blob.table.lock().unwrap().remove(id);
        send(conn, id, ns, Body::cancel(Code::IO, why));
    }
}

/// An image's END: size and SHA-256 checked, then the file synced and
/// linked to its name (decision 5.2), and DONE with a new reference.
async fn end(conn: &Arc<Conn>, id: TransferId, ns: &Namespace, size: u64) {
    let (part, hash, ty, pane_id, started) = {
        let mut table = conn.blob.table.lock().unwrap();
        let Some(live) = table.live.get_mut(&id) else {
            drop(table);
            return unknown(conn, id, ns, true);
        };
        let Live::Upload(up) = live else {
            let why = if matches!(live, Live::Pending { .. }) {
                "an END before ACCEPT"
            } else {
                "an END for a transfer the phone does not send"
            };
            drop(table);
            return protocol(conn, id, ns, why);
        };
        let hash = match up.rx.finish(size) {
            Ok(hash) => hash,
            Err(code) => {
                table.remove(id);
                drop(table);
                return send(
                    conn,
                    id,
                    ns,
                    Body::cancel(code, "the bytes do not match the offer"),
                );
            }
        };
        let ty = up
            .rx
            .offer()
            .and_then(|o| ImageType::from_mime(&o.mime))
            .unwrap_or(ImageType::Png);
        let Some(part) = up.part.take() else {
            drop(table);
            return protocol(conn, id, ns, "an END while a CHUNK is written");
        };
        (part, hash, ty, up.pane_id.clone(), up.started)
    };
    let name = images::file_name(SystemTime::now(), &hash, ty);
    let shown = name.clone();
    let finished = tokio::task::spawn_blocking(move || part.finish(&name)).await;
    let path = match finished {
        Ok(Ok(path)) => path,
        Ok(Err((_, why))) => {
            conn.blob.table.lock().unwrap().remove(id);
            return send(conn, id, ns, Body::cancel(Code::IO, why));
        }
        Err(e) => {
            conn.blob.table.lock().unwrap().remove(id);
            return send(conn, id, ns, Body::cancel(Code::IO, e.to_string()));
        }
    };
    if conn.blob.table.lock().unwrap().remove(id).is_none() {
        // Cancelled while it was linked: the file stays for cleanup.
        return;
    }
    let reference = conn
        .daemon
        .blob
        .images
        .remember(path, &pane_id, &conn.principal().id);
    log::info!(
        "Transfer {id} ({ns}) on connection {} DONE: {size} bytes in {} ms, stored as {shown}",
        conn.id(),
        started.elapsed().as_millis()
    );
    send(
        conn,
        id,
        ns,
        Body::Done {
            body: reference.to_vec(),
        },
    );
}

/// ACCEPT, REFUSE, DONE and CANCEL (and any later subcommand) for a live
/// transfer.
fn answer(conn: &Arc<Conn>, id: TransferId, ns: &Namespace, body: Body) {
    let now = Instant::now();
    let mut table = conn.blob.table.lock().unwrap();
    let Some(live) = table.live.get_mut(&id) else {
        drop(table);
        return unknown(conn, id, ns, matches!(body, Body::Accept { .. }));
    };
    let step = match live {
        Live::Pending { .. } => match &body {
            Body::Cancel { code, reason } => Step::Ended {
                code: *code,
                reason: reason.clone(),
            },
            _ => Step::Abort(Body::cancel(Code::PROTOCOL, "not answered yet")),
        },
        Live::Upload(up) => up.rx.on_message(&body, now),
        Live::Download(d) => {
            let step = d.tx.lock().unwrap().on_message(&body, now);
            if let Step::Accepted { chunk_max } = step {
                log::debug!("Transfer {id}: ACCEPT chunk_max {chunk_max}");
                d.task = Some(spawn_sender(
                    conn,
                    id,
                    ns.clone(),
                    Arc::clone(&d.tx),
                    Arc::clone(&d.bytes),
                ));
                return;
            }
            if let Step::Done(done) = &step {
                // `files` and `git` DONE bodies are empty.
                if !done.is_empty() {
                    table.remove(id);
                    drop(table);
                    return send(conn, id, ns, Body::cancel(Code::PROTOCOL, "a DONE body"));
                }
                log::info!(
                    "Transfer {id} ({ns}) on connection {} DONE: {} ({} bytes) in {} ms",
                    conn.id(),
                    d.what,
                    d.bytes.len(),
                    d.started.elapsed().as_millis()
                );
            }
            step
        }
    };
    match step {
        Step::Done(_) => {
            table.remove(id);
        }
        Step::Ended { code, reason } => {
            table.remove(id);
            log::info!(
                "Transfer {id} ({ns}) on connection {} ended by the phone: {code} ({reason})",
                conn.id()
            );
        }
        Step::Abort(b) => {
            table.remove(id);
            drop(table);
            send(conn, id, ns, b);
        }
        _ => {}
    }
}

/// Sends a download's CHUNKs from its buffer to the bulk queue, at most
/// two queued at once (decision 4.5), then END once all are written.
fn spawn_sender(
    conn: &Arc<Conn>,
    id: TransferId,
    ns: Namespace,
    tx: Arc<Mutex<Sending>>,
    bytes: Arc<Vec<u8>>,
) -> AbortHandle {
    let weak = Arc::downgrade(conn);
    tokio::spawn(async move {
        let mut queued: VecDeque<oneshot::Receiver<()>> = VecDeque::new();
        loop {
            let next = tx.lock().unwrap().next_chunk(CHUNK_DEFAULT as usize);
            let Some((at, n)) = next else { break };
            let Some(conn) = weak.upgrade() else { return };
            let at_usize = at as usize;
            let msg = BlobMessage::encode_chunk(id, &ns, at, &bytes[at_usize..at_usize + n]);
            let (written, done) = oneshot::channel();
            conn.send_binary(msg, true, Some(written));
            drop(conn);
            queued.push_back(done);
            if queued.len() >= 2 && queued.pop_front().expect("two").await.is_err() {
                return;
            }
        }
        // END goes to the control queue: only after every CHUNK is out.
        for done in queued {
            if done.await.is_err() {
                return;
            }
        }
        let end = tx.lock().unwrap().end(Instant::now());
        if let (Some(end), Some(conn)) = (end, weak.upgrade()) {
            send(&conn, id, &ns, end);
        }
    })
    .abort_handle()
}
