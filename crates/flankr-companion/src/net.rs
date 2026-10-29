//! The `net` thread: a Tokio current-thread runtime that keeps the
//! connections to the daemons, reconnects each with its own backoff, and
//! drives a [`State`](crate::client::State) for each (plan 20261022_04,
//! decision 2, and the 18:10 EDT amendment's steps 1 and 2).
//!
//! The phone keeps a list of paired remotes ([`Remotes`], plan 20261028_01)
//! in the [`KeyStore`]. The thread is a connection manager (plan
//! 20261028_03, the 18:51 EDT amendment): every listed remote has its own
//! link over flankr link (TCP to the record's addresses in order, `IK`
//! with that remote's own phone key, and `hello` carried in the
//! handshake), its own state and its `state.redb` ([`RemoteState`]), open
//! while it is listed. The active remote's link is in front; the others
//! are in the daemon's background mode (`client.options` with
//! `background`), and their events reach the app as
//! [`Event::Background`]. [`Input::SwitchRemote`] swaps which is in front
//! without closing or making a link; the app's own backgrounding puts
//! every link in the background. With no remote listed, the thread
//! connects to the daemon's local socket instead.
//!
//! [`Input::Pair`] takes the pairing URL, the bare string or the QR code's
//! data URL (plan 20261022_06, step 5). The thread makes the new remote's
//! key (`noise.static.<id>`) and, beside the open connection, pairs it
//! (plan 20261028_01, the 14:37, 14:42 and 14:44 EDT amendments):
//!
//! - a link with an offer (version 3) gets a **pairing request**: one `NK`
//!   connection to the link's addresses in order, carrying the key, the
//!   device name and the offer. Once the daemon holds it,
//!   [`Event::PairingWords`] gives the six pairing words `flankr pair`
//!   prints too, and the connection waits for the owner's answer there:
//!   added, refused or expired. A refusal at once, a refusal or expiry
//!   later, a dropped connection or Cancel ends the pairing with the key
//!   deleted ([`Event::PairFailed`]).
//! - a link without an offer (version 2), or a request no address answered,
//!   gets the **device-code fallback**: [`Event::PairingWords`] with the
//!   device code (and the six words for its key when the link has an
//!   offer), and a **pairing probe**, a task that tries `IK` with `hello`
//!   every [`PAIRING_RETRY`] for the pairing window while the server drops
//!   it, until the code is pasted into `flankr pair`.
//!
//! Once the daemon adds the key, the remote is added to the list (and made
//! active only if none was) and [`Event::Paired`] and [`Event::Remotes`]
//! are given. Nothing is stored before then; a pairing that ends otherwise
//! or is cancelled ([`Input::CancelPairing`]) deletes its key (decisions
//! 10-12). The same link again keeps the pairing and its key and gives the
//! same words again; a new link (a new `flankr pair` run) for the daemon
//! being paired keeps the key and asks again.
//!
//! A link for a remote already in the list (and whose key this phone has)
//! starts an **address check** instead: one `IK` connection, with that
//! remote's key, to the link's addresses, within [`CHECK_TIMEOUT`]. Only
//! once its `hello` is answered are the link's name and addresses saved and
//! [`Event::RemoteUpdated`] given; until then, and if it fails
//! ([`Event::PairFailed`]), the old ones stay in use (plan 20261028_01, the
//! 14:37 EDT amendment, item 4). A listed remote whose key was lost is
//! paired again as a new one; in the fallback its code is given with
//! `replace`: the daemon still lists this phone's old key under its name,
//! so the code is pasted with `flankr pair --replace`.
//!
//! [`Input::RemoveRemote`] asks the remote's daemon to forget this phone:
//! the state sends `link.unpair` over the open link for the active remote
//! and answers with [`Output::Remove`]; any other is asked by a **revoke
//! probe**, one short connection bounded by [`REVOKE_TIMEOUT`]. Then its
//! state directory and key are deleted and the list written without it
//! (decision 22); removing the active remote makes the most recently
//! paired of the rest active, and the last one gives [`Event::Unpaired`].
//!
//! Liveness is the operating system's: link sockets have `SO_KEEPALIVE`,
//! and a connection ends only on an error or end of stream. The client does
//! not `ping`.
//!
//! Every message's bytes, both ways, are added to the count the app reads
//! as [`Companion::link_bytes`](crate::Companion::link_bytes): the
//! notification's "n MB this session" (CHANGES.md, 2026-10-24 12:59 EDT,
//! item 3). A socket line counts its newline; a link message counts its
//! plaintext, without the Noise framing, and `hello` in the handshake is
//! not counted. The probes' connections are not counted.
//!
//! The link is read with `recv`, by kind (plan 20261027_07, step 4): a
//! JSON-RPC message goes to the state as before, and a terminal frame to
//! [`State::terminal_frame`], which shows it on its terminal's screen and
//! acknowledges it. Terminal bytes count like any other message's. A
//! binary message (kind 2, docs/design/20261028_05_BINARY_FILES_PLAN.md)
//! goes to [`State::binary`], and what the state has to send of kind 2 is
//! taken with [`State::take_binary`] after each of its outputs.
//!
//! Each connection's writer is its own task with two queues (decision
//! 4.5): control (JSON-RPC and every kind-2 message but CHUNK) and bulk
//! (CHUNKs). It drains control first and takes one CHUNK only when
//! control is empty, so a JSON-RPC message waits at most one CHUNK; each
//! CHUNK written is reported back ([`State::binary_written`]), which is
//! how a transfer keeps at most two queued.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use libflankr_proto::link::{
    self, DeviceCode, LinkStream, LinkWriter, Message, OFFER_LEN, PairAck, PairRefusal,
    PairRequest, PairResult, StaticKey,
};
use libflankr_proto::{Id, Incoming, Request, common, common::methods};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{TcpSocket, TcpStream, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::CompanionConfig;
use crate::client::{DeviceCodeFallback, Event, Input, Output, Settings, State, TICK, UNPAIR_WAIT};
use crate::keys::{KeyStore, MAX_REMOTES, RemoteId, RemoteInfo, Remotes, ServerRecord};
use crate::remote_state::RemoteState;
use crate::scan::{self, Pairing};
use crate::terminal::Terminals;

/// The longest line read from the daemon's socket (the daemon's own limit).
/// The link has the same limit on its messages.
const MAX_LINE: usize = 1 << 20;

/// How long a TCP connection to one address may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a link handshake may take (as the daemon's, CHANGES.md 18:24).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// In the device-code fallback, how often to try again while the server drops the
/// connection (it does until the code is added).
pub const PAIRING_RETRY: Duration = Duration::from_secs(2);
/// How long a pairing tries at [`PAIRING_RETRY`] before it gives up.
pub const PAIRING_WINDOW: Duration = Duration::from_secs(10 * 60);
/// How long asking a remote that is not active to forget this phone may
/// take in all (plan 20261028_01, decision 22).
pub const REVOKE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long checking a listed remote's new addresses from a link may take
/// in all.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// How long past the offer's time left a held pairing request waits for
/// the daemon's answer before giving up.
pub const RESULT_GRACE: Duration = Duration::from_secs(10);

/// Delays between reconnects: `initial`, doubling up to `max` (1, 2, 4 …
/// 30 s by default), back to `initial` once a connection says hello.
#[derive(Debug, Clone)]
pub struct Backoff {
    initial: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(initial: Duration, max: Duration) -> Self {
        Backoff {
            initial,
            max,
            next: initial,
        }
    }

    /// The delay before the next attempt.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.max);
        delay
    }

    pub fn reset(&mut self) {
        self.next = self.initial;
    }
}
/// Runs the thread until the app drops its [`Companion`](crate::Companion).
pub(crate) fn run(
    config: CompanionConfig,
    keys: Box<dyn KeyStore>,
    inputs: UnboundedReceiver<Input>,
    events: mpsc::Sender<Event>,
    bytes: Arc<AtomicU64>,
    terminals: Arc<Terminals>,
    tap: Tap,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = events.send(Event::Error(format!("starting the net runtime: {e}")));
            return;
        }
    };
    let (tasks_tx, tasks) = tokio::sync::mpsc::unbounded_channel();
    let (wire_tx, wire) = tokio::sync::mpsc::unbounded_channel();
    let net = Net {
        start: tokio::time::Instant::now(),
        config,
        keys,
        remotes: Remotes::default(),
        links: Vec::new(),
        terminals,
        pending: VecDeque::new(),
        app_background: false,
        drawer: false,
        settings: None,
        probe: None,
        checks: Vec::new(),
        next_probe: 0,
        removing: Vec::new(),
        shown: None,
        tasks_tx,
        tasks,
        wire_tx,
        wire,
        inputs,
        events,
        bytes,
        tap,
    };
    runtime.block_on(net.run());
}

struct Net {
    config: CompanionConfig,
    keys: Box<dyn KeyStore>,
    /// The paired remotes, as last read or written.
    remotes: Remotes,
    /// One link per listed remote, or the local socket's while none is
    /// listed (plan 20261028_03, the 18:51 EDT amendment).
    links: Vec<LinkSlot>,
    /// The terminals the app reads: held by the active link's state.
    terminals: Arc<Terminals>,
    /// Outputs still to be carried out, by link, in order.
    pending: VecDeque<(LinkKey, Vec<Output>)>,
    /// The app is in the background ([`Input::Background`]): so is every
    /// link.
    app_background: bool,
    /// The drawer is shown ([`Input::DrawerShown`]): the active link's.
    drawer: bool,
    /// The settings last given, for links made later.
    settings: Option<Settings>,
    /// The pairing in progress.
    probe: Option<Probe>,
    /// The address checks in progress, at most one per remote.
    checks: Vec<Check>,
    /// The token of the next probe or check.
    next_probe: u64,
    /// Remotes whose removal has begun.
    removing: Vec<RemoteId>,
    /// The rows last given as [`Event::Remotes`].
    shown: Option<Vec<RemoteInfo>>,
    tasks_tx: UnboundedSender<Done>,
    tasks: UnboundedReceiver<Done>,
    /// What the links' tasks report.
    wire_tx: UnboundedSender<Wire>,
    wire: UnboundedReceiver<Wire>,
    /// The clock [`Input::Tick`] carries: the time since the thread began.
    start: tokio::time::Instant,
    inputs: UnboundedReceiver<Input>,
    events: mpsc::Sender<Event>,
    /// The bytes sent and received since the thread began, on every link.
    bytes: Arc<AtomicU64>,
    /// Sees each event, on this thread, before the app is given it.
    tap: Tap,
}

/// Which daemon a link is to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkKey {
    /// The daemon's local socket, used while no remote is listed.
    Local,
    /// A listed remote, over flankr link.
    Remote(RemoteId),
}

/// One daemon's connection and the state it drives.
struct LinkSlot {
    key: LinkKey,
    state: State,
    /// The remote's `state.redb`, open while it is listed.
    _remote_state: Option<RemoteState>,
    backoff: Backoff,
    /// Bumped with each attempt and each end: what an older attempt or
    /// connection reports is dropped.
    generation: u64,
    conn: LinkConn,
}

/// Where a link's connection stands.
enum LinkConn {
    /// Not connected: the next attempt is at this time.
    Waiting(tokio::time::Instant),
    /// An attempt is on its way.
    Connecting(JoinHandle<()>),
    /// Connected: its writer's queues, and the task reading it. The
    /// writer task ends once the queues are dropped and drained.
    Open {
        write: WriteQueue,
        reader: JoinHandle<()>,
    },
}

impl LinkSlot {
    /// Ends the attempt or connection, if any; the next attempt is at
    /// `next`.
    fn stop(&mut self, next: tokio::time::Instant) {
        self.generation += 1;
        match std::mem::replace(&mut self.conn, LinkConn::Waiting(next)) {
            LinkConn::Connecting(task) => task.abort(),
            LinkConn::Open { reader, .. } => reader.abort(),
            LinkConn::Waiting(_) => {}
        }
    }

    /// An attempt or a connection is under way.
    fn busy(&self) -> bool {
        !matches!(self.conn, LinkConn::Waiting(_))
    }
}

impl Drop for LinkSlot {
    fn drop(&mut self) {
        self.stop(tokio::time::Instant::now());
    }
}

/// What a link's tasks report to the thread.
enum Wire {
    /// The attempt `generation` connected, its handshake done.
    Connected {
        key: LinkKey,
        generation: u64,
        conn: Conn,
    },
    /// The attempt `generation` reached no address.
    Failed { key: LinkKey, generation: u64 },
    /// A message on connection `generation`.
    Message {
        key: LinkKey,
        generation: u64,
        message: Message,
    },
    /// Connection `generation` closed.
    Closed { key: LinkKey, generation: u64 },
    /// Connection `generation`'s writer wrote a CHUNK of this transfer.
    Written {
        key: LinkKey,
        generation: u64,
        transfer: u32,
    },
}

/// A pairing in progress beside the open connection (decision 10): a
/// pairing request while it is sent and held, or the device-code fallback's
/// probe (the 14:37 EDT amendment).
struct Probe {
    token: u64,
    /// The remote it pairs: a new record, or the listed one whose key was
    /// missing.
    record: ServerRecord,
    /// Whether the record is already in the list.
    listed: bool,
    /// The phone's new key for the remote, stored as `record.key_name` and
    /// deleted unless the pairing ends with the remote added.
    key: Arc<dyn StaticKey>,
    /// The link's offer; `None` for a version 2 link.
    offer: Option<[u8; OFFER_LEN]>,
    /// The [`Event::PairingWords`] it gave, given again for the same link;
    /// `None` while the request is on its way.
    shown: Option<Event>,
    task: JoinHandle<()>,
}

/// How a pairing request ended without the remote being added.
enum Ended {
    /// The daemon refused the request at once.
    Refused(PairRefusal),
    /// The daemon's answer once the owner answered, or the offer expired.
    Result(PairResult),
    /// The connection closed before the daemon's answer.
    Dropped,
    /// No answer came within the offer's time.
    TimedOut,
}

/// A check of a listed remote's name and addresses from a link: saved only
/// once a connection to them with that remote's key is answered.
struct Check {
    token: u64,
    id: RemoteId,
    name: String,
    addresses: Vec<String>,
    task: JoinHandle<()>,
}

/// What a probe task found.
enum Done {
    /// The pairing probe `token`'s `hello` was answered.
    Paired { token: u64 },
    /// The pairing probe `token`'s window ended.
    GaveUp { token: u64 },
    /// The pairing request `token` is held by the daemon at `address`,
    /// whose offer has `expires_in_secs` left.
    Asked {
        token: u64,
        address: String,
        expires_in_secs: u16,
    },
    /// The pairing request `token` reached no daemon that answered it: the
    /// device-code fallback follows.
    Unreachable { token: u64 },
    /// The pairing request `token` ended with nothing added.
    Ended { token: u64, ended: Ended },
    /// The address check `token` ended; `reached`: its `hello` was
    /// answered.
    Checked { token: u64, reached: bool },
    /// The revoke probe for `id` ended; `daemon`: the daemon answered
    /// `link.unpair`.
    Revoked { id: RemoteId, daemon: bool },
}

/// A function that sees each event on the `net` thread, with the bytes
/// over the link so far ([`Companion::spawn_tapped`](crate::Companion::spawn_tapped)).
pub(crate) type Tap = Box<dyn Fn(&Event, u64) + Send>;

/// A connection, its handshake done.
enum Conn {
    /// The daemon's socket. `hello` is still to be sent: this is it.
    Unix(UnixStream, Vec<u8>),
    /// The link. `hello` went in the handshake.
    Link(LinkStream<TcpStream>),
}

/// The sending half of a connection.
enum Writer {
    Unix(OwnedWriteHalf),
    Link(LinkWriter<WriteHalf<TcpStream>>),
}

impl Writer {
    /// Sends one JSON-RPC message: a line on the socket, a message on the
    /// link. Its bytes are added to `bytes` once sent.
    async fn send(&mut self, message: &[u8], bytes: &AtomicU64) -> Result<(), String> {
        match self {
            Writer::Unix(write) => {
                let mut line = Vec::with_capacity(message.len() + 1);
                line.extend_from_slice(message);
                line.push(b'\n');
                write.write_all(&line).await.map_err(|e| e.to_string())?;
                bytes.fetch_add(line.len() as u64, Ordering::Relaxed);
            }
            Writer::Link(write) => {
                write
                    .send_message(message)
                    .await
                    .map_err(|e| e.to_string())?;
                bytes.fetch_add(message.len() as u64, Ordering::Relaxed);
            }
        }
        Ok(())
    }
}

impl Writer {
    /// Sends one kind-2 message, on the link only: the socket carries no
    /// kind 2 (decision 3.2), so one for it is dropped.
    async fn send_binary(&mut self, message: &[u8], bytes: &AtomicU64) -> Result<(), String> {
        match self {
            Writer::Unix(_) => {
                log::warn!("a binary message for the local socket dropped");
            }
            Writer::Link(write) => {
                write
                    .send_binary(message)
                    .await
                    .map_err(|e| e.to_string())?;
                bytes.fetch_add(message.len() as u64, Ordering::Relaxed);
            }
        }
        Ok(())
    }
}

/// What goes on a writer's control queue.
enum Control {
    Json(Vec<u8>),
    Binary(Vec<u8>),
}

/// A connection's writer queues (decision 4.5): control first, then one
/// CHUNK (with its transfer) when control is empty.
struct WriteQueue {
    control: UnboundedSender<Control>,
    bulk: UnboundedSender<(Vec<u8>, u32)>,
}

impl WriteQueue {
    /// Starts `write`'s task. A failed write reports the connection
    /// closed; each CHUNK written is reported as [`Wire::Written`].
    fn spawn(
        mut write: Writer,
        bytes: Arc<AtomicU64>,
        wire: UnboundedSender<Wire>,
        key: LinkKey,
        generation: u64,
    ) -> WriteQueue {
        let (control, mut control_rx) = tokio::sync::mpsc::unbounded_channel::<Control>();
        let (bulk, mut bulk_rx) = tokio::sync::mpsc::unbounded_channel::<(Vec<u8>, u32)>();
        tokio::spawn(async move {
            let mut control_open = true;
            let mut bulk_open = true;
            while control_open || bulk_open {
                let result = tokio::select! {
                    biased;
                    m = control_rx.recv(), if control_open => match m {
                        Some(Control::Json(m)) => write.send(&m, &bytes).await,
                        Some(Control::Binary(m)) => write.send_binary(&m, &bytes).await,
                        None => {
                            control_open = false;
                            continue;
                        }
                    },
                    m = bulk_rx.recv(), if bulk_open => match m {
                        Some((m, transfer)) => {
                            let r = write.send_binary(&m, &bytes).await;
                            if r.is_ok() {
                                let _ = wire.send(Wire::Written {
                                    key,
                                    generation,
                                    transfer,
                                });
                            }
                            r
                        }
                        None => {
                            bulk_open = false;
                            continue;
                        }
                    },
                };
                if let Err(e) = result {
                    log::debug!("writing to the daemon: {e}");
                    let _ = wire.send(Wire::Closed { key, generation });
                    return;
                }
            }
        });
        WriteQueue { control, bulk }
    }
}

/// Unix seconds now.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Net {
    async fn run(mut self) {
        self.load_remotes();
        let mut ticks = tokio::time::interval(TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let wake = self.next_attempt();
            tokio::select! {
                _ = ticks.tick() => self.tick_all(),
                _ = tokio::time::sleep_until(wake) => self.start_due(),
                wire = self.wire.recv() => {
                    if let Some(wire) = wire {
                        self.on_wire(wire).await;
                    }
                }
                done = self.tasks.recv() => {
                    if let Some(done) = done {
                        self.task_done(done);
                    }
                }
                input = self.inputs.recv() => {
                    let Some(input) = input else { return };
                    self.on_input(input).await;
                }
            }
            self.flush().await;
            self.show_remotes();
        }
    }

    /// Reads the list (migrating the old single record), moves the data
    /// plan's app-wide store in, sweeps state directories no remote lists,
    /// makes a link (and opens the state) for each remote, and gives
    /// [`Event::Remotes`].
    fn load_remotes(&mut self) {
        match self.keys.remotes() {
            Ok(remotes) => self.remotes = remotes,
            Err(e) => {
                log::error!("keys: {e}");
                self.emit(Event::Error(format!("reading the paired remotes: {e}")));
            }
        }
        if let Some(dir) = self.config.data_dir.clone() {
            if let Err(e) = RemoteState::adopt_sessions(&dir, self.remotes.active_record()) {
                log::warn!("state: {e}");
            }
            RemoteState::sweep(&dir, &self.remotes);
        }
        self.add_links();
        self.show_remotes();
    }

    /// The links there should be: one per listed remote, or the local
    /// socket's while none is listed.
    fn wanted(&self) -> Vec<LinkKey> {
        if self.remotes.list.is_empty() {
            vec![LinkKey::Local]
        } else {
            self.remotes
                .list
                .iter()
                .map(|r| LinkKey::Remote(r.id))
                .collect()
        }
    }

    /// The link the app is using: the active remote's, or the local
    /// socket's while none is listed.
    fn active_key(&self) -> Option<LinkKey> {
        if self.remotes.list.is_empty() {
            Some(LinkKey::Local)
        } else {
            self.remotes.active.map(LinkKey::Remote)
        }
    }

    fn is_active(&self, key: LinkKey) -> bool {
        self.active_key() == Some(key)
    }

    fn index(&self, key: LinkKey) -> Option<usize> {
        self.links.iter().position(|l| l.key == key)
    }

    fn slot_mut(&mut self, key: LinkKey) -> Option<&mut LinkSlot> {
        self.links.iter_mut().find(|l| l.key == key)
    }

    /// Makes the links that are wanted and missing, each connecting at
    /// once: in the background unless it is the active one and the app is
    /// in front. The first, made while no state holds the app's terminals,
    /// is given them if it is the active one.
    fn add_links(&mut self) {
        for key in self.wanted() {
            if self.index(key).is_some() {
                continue;
            }
            let holder = self
                .links
                .iter()
                .any(|l| Arc::ptr_eq(l.state.terminals(), &self.terminals));
            let terminals = if !holder && self.is_active(key) {
                Arc::clone(&self.terminals)
            } else {
                Arc::default()
            };
            let mut state = State::with_terminals(self.config.client.clone(), terminals);
            state.set_dictation_silence(self.config.dictation_silence);
            state.set_preview_cache(self.config.preview_cache_dir());
            if let Some(settings) = self.settings {
                state.input(Input::Settings(settings));
            }
            let background = self.app_background || !self.is_active(key);
            state.input(Input::Background(background));
            let remote_state = match key {
                LinkKey::Remote(id) => self.open_state(id),
                LinkKey::Local => None,
            };
            log::info!(
                "links: {}",
                match key {
                    LinkKey::Local => "the local socket".to_string(),
                    LinkKey::Remote(id) => format!(
                        "{id}{}",
                        if background {
                            ", in the background"
                        } else {
                            ""
                        }
                    ),
                }
            );
            self.links.push(LinkSlot {
                key,
                state,
                _remote_state: remote_state,
                backoff: Backoff::new(self.config.backoff_initial, self.config.backoff_max),
                generation: 0,
                conn: LinkConn::Waiting(tokio::time::Instant::now()),
            });
        }
    }

    /// Closes the links no longer wanted, and their states.
    fn drop_links(&mut self) {
        let wanted = self.wanted();
        self.links.retain(|l| wanted.contains(&l.key));
    }

    /// Opens `id`'s `state.redb`.
    fn open_state(&self, id: RemoteId) -> Option<RemoteState> {
        let dir = self.config.data_dir.as_deref()?;
        let remote = self.remotes.get(id)?;
        match RemoteState::open(dir, remote) {
            Ok(state) => Some(state),
            Err(e) => {
                log::warn!("state: opening {}'s: {e}", remote.name);
                None
            }
        }
    }

    /// Moves the app's terminals to `key`'s state from the state holding
    /// them, each keeping its own daemon's terminal mode.
    fn give_terminals(&mut self, key: LinkKey) {
        let Some(to) = self.index(key) else {
            return;
        };
        let Some(from) = self
            .links
            .iter()
            .position(|l| Arc::ptr_eq(l.state.terminals(), &self.terminals))
        else {
            return;
        };
        if from == to {
            return;
        }
        let (a, b) = if from < to {
            let (left, right) = self.links.split_at_mut(to);
            (&mut left[from].state, &mut right[0].state)
        } else {
            let (left, right) = self.links.split_at_mut(from);
            (&mut right[0].state, &mut left[to].state)
        };
        State::trade_terminals(a, b);
    }

    /// Brings `key`'s link in front: the app's terminals move to it, what
    /// it holds is given to the app as it stands ([`State::replay`]), and
    /// it leaves the background unless the app is there.
    fn promote(&mut self, key: LinkKey) {
        self.give_terminals(key);
        let tick = self.tick_for(key);
        let (background, drawer) = (self.app_background, self.drawer);
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        let mut outputs: Vec<Output> = slot.state.replay().into_iter().map(Output::Event).collect();
        outputs.extend(slot.state.input(tick));
        outputs.extend(slot.state.input(Input::Background(background)));
        if drawer {
            outputs.extend(slot.state.input(Input::DrawerShown(true)));
        }
        self.pending.push_back((key, outputs));
    }

    /// Puts the active remote's link in the background and `id`'s in
    /// front (plan 20261028_03, the 18:51 EDT amendment, item 3): `active`
    /// is written first; no link is closed or made.
    async fn switch_remote(&mut self, id: RemoteId) {
        if self.remotes.active == Some(id) {
            log::info!("remotes: {id} is already in use");
            return;
        }
        if self.remotes.get(id).is_none() {
            log::info!("remotes: not switching to {id}: it is not paired");
            return;
        }
        if self.removing.contains(&id) {
            log::info!("remotes: not switching to {id}: it is being removed");
            return;
        }
        let old = self.active_key();
        let previous = self.remotes.active;
        self.remotes.active = Some(id);
        if !self.write_remotes() {
            self.remotes.active = previous;
            return;
        }
        log::info!("remotes: {id} in use; the one before in the background");
        if let Some(old) = old {
            let tick = self.tick_for(old);
            if let Some(slot) = self.slot_mut(old) {
                let mut outputs = slot.state.input(tick);
                outputs.extend(slot.state.leave_foreground());
                outputs.extend(slot.state.input(Input::Background(true)));
                self.pending.push_back((old, outputs));
            }
        }
        self.promote(LinkKey::Remote(id));
        self.flush().await;
    }

    /// Gives [`Event::Remotes`] if the rows changed since last given.
    fn show_remotes(&mut self) {
        let links = &self.links;
        let rows = self.remotes.info_with(|id| {
            links
                .iter()
                .any(|l| l.key == LinkKey::Remote(id) && l.state.online())
        });
        if self.shown.as_ref() != Some(&rows) {
            self.shown = Some(rows.clone());
            self.emit(Event::Remotes(rows));
        }
    }

    /// Writes the list; a failure is given as an error.
    fn write_remotes(&mut self) -> bool {
        match self.keys.set_remotes(&self.remotes) {
            Ok(()) => true,
            Err(e) => {
                log::error!("keys: writing the remotes: {e}");
                self.emit(Event::Error(format!("writing the paired remotes: {e}")));
                false
            }
        }
    }

    /// An input from the app: the thread's own (pairing, remotes, the
    /// switch), one for every link (the app's backgrounding, settings,
    /// the speaker and microphone, memory, the cache), one for a named
    /// remote's link ([`Input::ToRemote`]), or else the active link's.
    async fn on_input(&mut self, input: Input) {
        let Some(input) = self.net_input(input) else {
            return;
        };
        match input {
            Input::SwitchRemote(id) => self.switch_remote(id).await,
            Input::ToRemote { remote, input } => {
                let key = LinkKey::Remote(remote);
                // Plugins act on the remote in use only (plan 20261028_04,
                // 3.10 item 3).
                match crate::client::plugin_refusal(&input).filter(|_| !self.is_active(key)) {
                    Some(events) => events.into_iter().for_each(|e| self.emit(e)),
                    None => self.give(key, *input),
                }
            }
            Input::Background(on) => {
                self.app_background = on;
                for key in self.keys_of_links() {
                    let background = on || !self.is_active(key);
                    self.give(key, Input::Background(background));
                }
            }
            Input::Settings(settings) => {
                self.settings = Some(settings);
                self.give_all(Input::Settings(settings));
            }
            input @ (Input::Speaking(_)
            | Input::Listening(_)
            | Input::Memory(_)
            | Input::ClearCache) => self.give_all(input),
            Input::DrawerShown(on) => {
                self.drawer = on;
                if let Some(key) = self.active_key() {
                    self.give(key, Input::DrawerShown(on));
                }
            }
            input => match self.active_key() {
                Some(key) => self.give(key, input),
                None => log::info!("no remote in use for {input:?}"),
            },
        }
    }

    fn keys_of_links(&self) -> Vec<LinkKey> {
        self.links.iter().map(|l| l.key).collect()
    }

    /// Gives `input` to `key`'s state, after the clock.
    fn give(&mut self, key: LinkKey, input: Input) {
        let tick = self.tick_for(key);
        let Some(slot) = self.slot_mut(key) else {
            log::info!("no link for {input:?}");
            return;
        };
        let mut outputs = slot.state.input(tick);
        outputs.extend(slot.state.input(input));
        self.pending.push_back((key, outputs));
    }

    fn give_all(&mut self, input: Input) {
        for key in self.keys_of_links() {
            self.give(key, input.clone());
        }
    }

    /// The inputs the thread handles itself: pairing and removing remotes.
    /// Others are returned; so is [`Input::RemoveRemote`] of the active
    /// remote, which goes over its link.
    fn net_input(&mut self, input: Input) -> Option<Input> {
        match input {
            Input::Pair(text) => {
                if let Some(p) = self.check_pairing(&text) {
                    self.begin_pairing(p);
                }
                None
            }
            Input::CancelPairing => {
                self.end_probe("cancelled");
                for check in std::mem::take(&mut self.checks) {
                    check.task.abort();
                }
                None
            }
            Input::Unpair => match self.remotes.active {
                Some(id) => self.remove(id),
                None => {
                    self.emit(Event::Unpaired { daemon: false });
                    None
                }
            },
            Input::RemoveRemote(id) => self.remove(id),
            Input::SetNickname { id, nickname } => {
                self.set_nickname(id, nickname.as_deref());
                None
            }
            input => Some(input),
        }
    }

    /// Sets `id`'s nickname and writes the list; [`Event::Remotes`] follows
    /// when what it shows changed (plan 20261024_02, the 2026-10-28 17:56
    /// EDT amendment).
    fn set_nickname(&mut self, id: RemoteId, nickname: Option<&str>) {
        if self.remotes.get(id).is_none() {
            self.emit(Event::Error(format!("no remote {id} is paired")));
            return;
        }
        if !self.remotes.set_nickname(id, nickname) {
            return;
        }
        let kept = self.remotes.get(id).and_then(|r| r.nickname.clone());
        log::info!(
            "remotes: {id}'s nickname {}",
            if kept.is_some() { "set" } else { "cleared" }
        );
        if self.write_remotes() {
            self.show_remotes();
        }
    }

    /// Starts removing `id`: the active remote's `link.unpair` goes to the
    /// state (returned); any other gets a revoke probe.
    fn remove(&mut self, id: RemoteId) -> Option<Input> {
        let Some(record) = self.remotes.get(id).cloned() else {
            self.emit(Event::Error(format!("no remote {id} is paired")));
            return None;
        };
        if self.removing.contains(&id) {
            return None;
        }
        self.removing.push(id);
        if self.probe.as_ref().is_some_and(|p| p.record.id == id) {
            self.end_probe("its remote is being removed");
        }
        self.end_check(id);
        if self.remotes.active == Some(id) {
            log::info!("remotes: removing {} (in use)", record.name);
            return Some(Input::RemoveRemote(id));
        }
        log::info!(
            "remotes: removing {}; asking it to forget this phone",
            record.name
        );
        let key = match self.keys.static_key(&record.key_name) {
            Ok(key) => key,
            Err(e) => {
                log::warn!("remotes: {}'s key: {e}", record.name);
                None
            }
        };
        let tasks = self.tasks_tx.clone();
        match key {
            Some(key) => {
                let hello = self.probe_hello();
                tokio::spawn(async move {
                    let daemon = revoke(&record, key, &hello).await;
                    let _ = tasks.send(Done::Revoked { id, daemon });
                });
            }
            // No key: nothing can ask, and nothing can use its entry.
            None => {
                let _ = tasks.send(Done::Revoked { id, daemon: false });
            }
        }
        None
    }

    /// Removes `id` here: its link and state closed and its directory
    /// deleted, its key deleted, the list written without it (last), then
    /// the events (decisions 22 and 23). Removing the active remote brings
    /// the most recently paired of the rest in front.
    fn finish_remove(&mut self, id: RemoteId, daemon: bool) {
        self.removing.retain(|r| *r != id);
        let Some(record) = self.remotes.get(id).cloned() else {
            return;
        };
        let was_active = self.remotes.active == Some(id);
        self.remotes.list.retain(|r| r.id != id);
        if was_active {
            self.remotes.active = self.remotes.most_recent(&self.removing);
        }
        // The link coming in front (the local socket's, with none left) is
        // made before the removed one goes, so the app's terminals move
        // to it.
        self.add_links();
        if was_active && let Some(key) = self.active_key() {
            self.promote(key);
        }
        self.drop_links();
        if let Some(dir) = self.config.data_dir.as_deref()
            && let Err(e) = RemoteState::delete(dir, id)
        {
            // Swept at the next start.
            log::warn!("state: deleting {}'s: {e}", record.name);
        }
        if let Err(e) = self.keys.delete_static_key(&record.key_name) {
            log::error!("keys: deleting {}'s key: {e}", record.name);
            self.emit(Event::Error(format!(
                "deleting the key for {}: {e}",
                record.name
            )));
        }
        self.write_remotes();
        log::info!(
            "remotes: removed {}{}",
            record.name,
            if daemon { "" } else { " here only" }
        );
        self.emit(Event::RemoteRemoved {
            id,
            name: record.name,
            daemon,
        });
        self.show_remotes();
        if self.remotes.list.is_empty() {
            self.emit(Event::Unpaired { daemon });
        }
    }

    /// A probe task ended.
    fn task_done(&mut self, done: Done) {
        match done {
            Done::Paired { token } => {
                let Some(probe) = self.probe.take_if(|p| p.token == token) else {
                    return;
                };
                self.paired(probe);
            }
            Done::GaveUp { token } => {
                if self.probe.as_ref().is_some_and(|p| p.token == token) {
                    let server = self.probe.as_ref().map(|p| p.record.name.clone());
                    self.end_probe("not accepted in time");
                    self.emit(Event::PairFailed(format!(
                        "{} did not accept this phone's code in time; pair again",
                        server.unwrap_or_default()
                    )));
                }
            }
            Done::Asked {
                token,
                address,
                expires_in_secs,
            } => self.asked(token, address, expires_in_secs),
            Done::Unreachable { token } => self.unreachable(token),
            Done::Ended { token, ended } => {
                let Some(probe) = self.probe.as_ref().filter(|p| p.token == token) else {
                    return;
                };
                let why = ended_note(&probe.record.name, &ended);
                self.end_probe(&why);
                self.emit(Event::PairFailed(why));
            }
            Done::Checked { token, reached } => self.checked(token, reached),
            Done::Revoked { id, daemon } => self.finish_remove(id, daemon),
        }
    }

    /// An address check ended: the link's name and addresses are saved if
    /// they were reached; otherwise the old ones stay.
    fn checked(&mut self, token: u64, reached: bool) {
        let Some(at) = self.checks.iter().position(|c| c.token == token) else {
            return;
        };
        let check = self.checks.remove(at);
        if self.removing.contains(&check.id) {
            return;
        }
        let Some(listed) = self.remotes.get_mut(check.id) else {
            return;
        };
        if !reached {
            let name = listed.name.clone();
            log::info!("pairing: {name} was not reached at the link's addresses; unchanged");
            self.emit(Event::PairFailed(format!(
                "{name} was not reached at the addresses in the link, so its addresses \
                 are unchanged"
            )));
            return;
        }
        listed.name = check.name.clone();
        listed.addresses = check.addresses.clone();
        if !self.write_remotes() {
            return;
        }
        log::info!(
            "pairing: already paired with {}; addresses updated",
            check.name
        );
        self.emit(Event::RemoteUpdated {
            id: check.id,
            name: check.name,
        });
        self.show_remotes();
    }

    /// Ends the address check of `id`, if any.
    fn end_check(&mut self, id: RemoteId) {
        self.checks.retain(|c| {
            if c.id == id {
                c.task.abort();
            }
            c.id != id
        });
    }

    /// The probe's `hello` was answered: the remote goes into the list,
    /// active only if none was (decisions 8 and 10).
    fn paired(&mut self, probe: Probe) {
        let mut record = probe.record;
        record.paired_at = Some(unix_now());
        if probe.listed {
            match self.remotes.get_mut(record.id) {
                Some(listed) => *listed = record.clone(),
                None => self.remotes.list.push(record.clone()),
            }
        } else {
            if self.remotes.list.len() >= MAX_REMOTES {
                let _ = self.keys.delete_static_key(&record.key_name);
                self.emit(Event::PairFailed(full_note()));
                return;
            }
            self.remotes.list.push(record.clone());
        }
        let first = self.remotes.active.is_none();
        if first {
            self.remotes.active = Some(record.id);
        }
        if !self.write_remotes() {
            return;
        }
        log::info!(
            "remotes: paired with {} ({}){}",
            record.name,
            record.id.fingerprint(),
            if first { ", now in use" } else { "" }
        );
        self.emit(Event::Paired {
            name: self.config.device_name.clone(),
        });
        // Its link, in the background unless it is the first; the first
        // takes the app's place from the local socket's.
        self.add_links();
        if first {
            self.promote(LinkKey::Remote(record.id));
        }
        self.drop_links();
        self.show_remotes();
    }

    /// Ends the pairing probe, if any, deleting the key it made for a
    /// remote that is not in the list.
    fn end_probe(&mut self, why: &str) {
        let Some(probe) = self.probe.take() else {
            return;
        };
        probe.task.abort();
        log::info!("pairing with {}: {why}", probe.record.name);
        if let Err(e) = self.keys.delete_static_key(&probe.record.key_name) {
            log::warn!("pairing: deleting the probe's key: {e}");
        }
    }

    /// Reads a pairing input before any connection is made. A refusal is
    /// given as [`Event::PairFailed`].
    fn check_pairing(&self, text: &str) -> Option<Pairing> {
        match scan::read(text) {
            Ok(p) => Some(p),
            Err(why) => {
                self.emit(Event::PairFailed(why));
                None
            }
        }
    }

    /// Pairs with `p` beside the open connection (decisions 10-12 and the
    /// 14:37 EDT amendment). The same link again gives what it showed
    /// again, keeping the pairing and its key; a new link (a new offer) for
    /// the daemon being paired keeps the key and starts the request over. A
    /// listed remote whose key this phone has gets an address check
    /// ([`Net::check_addresses`]); a new one (or a listed one whose key is
    /// missing) gets its key and, for a link with an offer, a pairing
    /// request ([`Net::request`]), else the device-code fallback
    /// ([`Net::fallback`]). Nothing new is stored until the daemon adds the
    /// key.
    fn begin_pairing(&mut self, p: Pairing) {
        let server_key = p.payload.server_key;
        let offer = p.payload.offer;
        let id = RemoteId::of(&server_key);
        let kept = match self
            .probe
            .take_if(|probe| probe.record.public_key == server_key)
        {
            Some(probe) if probe.offer == offer => {
                log::info!(
                    "pairing: already pairing with {}; it stays",
                    probe.record.name
                );
                if let Some(shown) = probe.shown.clone() {
                    self.emit(shown);
                }
                self.probe = Some(probe);
                return;
            }
            Some(probe) => {
                // A new `flankr pair` run's link: the key stays.
                log::info!(
                    "pairing: a new link for {}; asking again with the same key",
                    probe.record.name
                );
                probe.task.abort();
                Some((probe.record.key_name, probe.key))
            }
            None => {
                self.end_probe("replaced by a new pairing");
                None
            }
        };
        let offered = ServerRecord::new(p.server.clone(), server_key, p.payload.addresses);
        let listed = match self.remotes.get(id) {
            Some(listed) if listed.public_key != server_key => {
                self.drop_kept(kept);
                self.emit(Event::PairFailed(
                    "a remote with the same id is already paired".to_string(),
                ));
                return;
            }
            Some(listed) => Some(listed.clone()),
            None => None,
        };
        let record = match listed {
            Some(listed) => {
                // A kept key is the pairing's, under the listed name: the
                // listed key itself was missing.
                if kept.is_none() {
                    match self.keys.static_key(&listed.key_name) {
                        Ok(Some(key)) => {
                            self.check_addresses(&listed, offered.name, offered.addresses, key);
                            return;
                        }
                        Ok(None) => {
                            log::info!("pairing: {}'s key is missing; pairing again", listed.name)
                        }
                        Err(e) => {
                            log::warn!("pairing: {}'s key: {e}; pairing again", listed.name)
                        }
                    }
                }
                // Saved with the link's name and addresses only once the
                // daemon adds the key.
                self.end_check(id);
                ServerRecord {
                    name: offered.name,
                    addresses: offered.addresses,
                    ..listed
                }
            }
            None if self.remotes.list.len() >= MAX_REMOTES => {
                self.drop_kept(kept);
                self.emit(Event::PairFailed(full_note()));
                return;
            }
            None => offered,
        };
        if !DeviceCode::valid_name(&self.config.device_name) {
            self.drop_kept(kept);
            self.emit(Event::PairFailed(format!(
                "the device name '{}': {}",
                self.config.device_name,
                link::PairingStringError::BadName
            )));
            return;
        }
        let listed = self.remotes.get(id).is_some();
        let key = match kept {
            Some((_, key)) => key,
            None => match self.keys.generate_static_key(&record.key_name) {
                Ok(key) => key,
                Err(e) => {
                    self.emit(Event::PairFailed(format!("this client's key: {e}")));
                    return;
                }
            },
        };
        let token = self.next_probe;
        self.next_probe += 1;
        // Replaced by the request's task or the fallback's probe below.
        let task = tokio::spawn(async {});
        self.probe = Some(Probe {
            token,
            record,
            listed,
            key,
            offer,
            shown: None,
            task,
        });
        match offer {
            Some(offer) => self.request(offer),
            None => self.fallback(),
        }
    }

    /// Deletes the key a replaced pairing kept, when the new link is
    /// refused after all.
    fn drop_kept(&mut self, kept: Option<(String, Arc<dyn StaticKey>)>) {
        if let Some((name, _)) = kept
            && let Err(e) = self.keys.delete_static_key(&name)
        {
            log::warn!("pairing: deleting the pairing's key: {e}");
        }
    }

    /// Sends the pairing request of the probe in progress to its link's
    /// addresses, in order, over `NK` (the 14:37 EDT amendment, item 1):
    /// [`Done::Asked`] once the daemon holds it, then [`Done::Paired`] or
    /// [`Done::Ended`] with the owner's answer; [`Done::Unreachable`] if no
    /// address answered it.
    fn request(&mut self, offer: [u8; OFFER_LEN]) {
        let Some(probe) = self.probe.as_mut() else {
            return;
        };
        log::info!(
            "pairing with {} ({}); sending the request",
            probe.record.name,
            probe.record.id.fingerprint()
        );
        let token = probe.token;
        let request = PairRequest {
            device_key: probe.key.public(),
            name: self.config.device_name.clone(),
            offer,
        };
        let addresses = probe.record.addresses.clone();
        let server_key = probe.record.public_key;
        let tasks = self.tasks_tx.clone();
        probe.task = tokio::spawn(async move {
            let done = pair_request(&addresses, &server_key, &request, token, &tasks).await;
            let _ = tasks.send(done);
        });
    }

    /// The daemon holds the probe `token`'s request: its six words are
    /// shown.
    fn asked(&mut self, token: u64, address: String, expires_in_secs: u16) {
        let Some(probe) = self.probe.as_mut().filter(|p| p.token == token) else {
            return;
        };
        let Some(offer) = probe.offer else {
            return;
        };
        let server_key = probe.record.public_key;
        log::info!(
            "pairing: {} at {address} holds the request ({expires_in_secs} s left); \
             waiting for yes on it",
            probe.record.name
        );
        let shown = Event::PairingWords {
            server: probe.record.name.clone(),
            address,
            words: Some(link::pairing_words_text(
                &server_key,
                &offer,
                &probe.key.public(),
            )),
            fingerprint: link::fingerprint_words_text(&server_key),
            fingerprint_hex: crate::keys::fingerprint_hex(&server_key),
            expires_in_secs: Some(expires_in_secs),
            fallback: None,
        };
        probe.shown = Some(shown.clone());
        self.emit(shown);
    }

    /// No address answered the probe `token`'s request: the device-code
    /// fallback.
    fn unreachable(&mut self, token: u64) {
        if self.probe.as_ref().is_some_and(|p| p.token == token) {
            log::info!("pairing: the request reached no daemon; the device code instead");
            self.fallback();
        }
    }

    /// The device-code fallback of the probe in progress (the 14:37 EDT
    /// amendment, item 2): its device code, the six words for its key and
    /// the link's offer (none without an offer), and a probe that tries
    /// `IK` until the daemon knows the key.
    fn fallback(&mut self) {
        let Some(probe) = self.probe.as_ref() else {
            return;
        };
        let server_key = probe.record.public_key;
        let code = DeviceCode {
            key: probe.key.public(),
            name: self.config.device_name.clone(),
        }
        .encode(&server_key);
        let code = match code {
            Ok(code) => code,
            Err(e) => {
                let why = format!("the device name '{}': {e}", self.config.device_name);
                self.end_probe(&why);
                self.emit(Event::PairFailed(why));
                return;
            }
        };
        let hello = self.probe_hello();
        let tasks = self.tasks_tx.clone();
        let window = self.config.pairing_window;
        let Some(probe) = self.probe.as_mut() else {
            return;
        };
        log::info!(
            "pairing with {} ({}); waiting for the device code to be added",
            probe.record.name,
            probe.record.id.fingerprint()
        );
        let token = probe.token;
        let addresses = probe.record.addresses.clone();
        let key = Arc::clone(&probe.key);
        probe.task = tokio::spawn(async move {
            let done = if self::probe(&addresses, key, &server_key, &hello, window).await {
                Done::Paired { token }
            } else {
                Done::GaveUp { token }
            };
            let _ = tasks.send(done);
        });
        let device_key = probe.key.public();
        let shown = Event::PairingWords {
            server: probe.record.name.clone(),
            address: probe.record.addresses.first().cloned().unwrap_or_default(),
            words: probe
                .offer
                .map(|offer| link::pairing_words_text(&server_key, &offer, &device_key)),
            fingerprint: link::fingerprint_words_text(&server_key),
            fingerprint_hex: crate::keys::fingerprint_hex(&server_key),
            expires_in_secs: None,
            fallback: Some(DeviceCodeFallback {
                code,
                replace: probe.listed,
            }),
        };
        probe.shown = Some(shown.clone());
        self.emit(shown);
    }

    /// Checks `listed`'s `name` and `addresses` from a link with one `IK`
    /// connection to them, with this phone's `key` for it, replacing any
    /// check of it in progress. Nothing is saved until it is answered
    /// ([`Net::checked`]); the same name and addresses need no check.
    fn check_addresses(
        &mut self,
        listed: &ServerRecord,
        name: String,
        addresses: Vec<String>,
        key: Arc<dyn StaticKey>,
    ) {
        let id = listed.id;
        self.end_check(id);
        if listed.name == name && listed.addresses == addresses {
            log::info!("pairing: already paired with {name}; nothing changed");
            self.emit(Event::RemoteUpdated { id, name });
            self.show_remotes();
            return;
        }
        log::info!(
            "pairing: already paired with {}; checking the link's addresses",
            listed.name
        );
        let token = self.next_probe;
        self.next_probe += 1;
        let hello = self.probe_hello();
        let tasks = self.tasks_tx.clone();
        let server_key = listed.public_key;
        let targets = addresses.clone();
        let task = tokio::spawn(async move {
            let reached = timeout(CHECK_TIMEOUT, check(&targets, key, &server_key, &hello))
                .await
                .unwrap_or(false);
            let _ = tasks.send(Done::Checked { token, reached });
        });
        self.checks.push(Check {
            token,
            id,
            name,
            addresses,
            task,
        });
    }

    /// The clock now, as an input for `key`'s state. Given before every
    /// input and message, so the state's times are those of what it is
    /// told. The phone's local time of day goes with it, for reading clock
    /// times as relative when the daemon cannot say how to read a reply.
    fn tick_for(&mut self, key: LinkKey) -> Input {
        if let Some(slot) = self.slot_mut(key) {
            slot.state.set_local_time(local_now());
        }
        Input::Tick(self.start.elapsed())
    }

    /// Every link's state is told the time.
    fn tick_all(&mut self) {
        for key in self.keys_of_links() {
            let tick = self.tick_for(key);
            if let Some(slot) = self.slot_mut(key) {
                let outputs = slot.state.input(tick);
                self.pending.push_back((key, outputs));
            }
        }
    }

    fn emit(&self, event: Event) {
        (self.tap)(&event, self.bytes.load(Ordering::Relaxed));
        let _ = self.events.send(event);
    }

    /// Gives `key`'s event to the app: as it is for the link in use,
    /// tagged with its remote otherwise ([`Event::Background`]).
    fn route(&self, key: LinkKey, event: Event) {
        match key {
            LinkKey::Remote(remote) if !self.is_active(key) => self.emit(Event::Background {
                remote,
                event: Box::new(event),
            }),
            _ => self.emit(event),
        }
    }

    /// The `hello` a probe beside the links sends: this client's, with no
    /// session to continue.
    fn probe_hello(&self) -> Vec<u8> {
        let request = State::new(self.config.client.clone()).hello_request();
        serde_json::to_vec(&request).unwrap_or_default()
    }

    /// When the next link waiting is to connect.
    fn next_attempt(&self) -> tokio::time::Instant {
        self.links
            .iter()
            .filter_map(|l| match l.conn {
                LinkConn::Waiting(at) => Some(at),
                _ => None,
            })
            .min()
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600))
    }

    /// Starts every link whose attempt is due.
    fn start_due(&mut self) {
        let now = tokio::time::Instant::now();
        let due: Vec<LinkKey> = self
            .links
            .iter()
            .filter(|l| matches!(l.conn, LinkConn::Waiting(at) if at <= now))
            .map(|l| l.key)
            .collect();
        for key in due {
            self.start_connect(key);
        }
    }

    /// Starts connecting `key`'s link: over flankr link to the remote's
    /// addresses in order with its own phone key and `hello` in the
    /// handshake, or to the daemon's socket.
    fn start_connect(&mut self, key: LinkKey) {
        let record = match key {
            LinkKey::Remote(id) => match self.remotes.get(id) {
                Some(record) => Some(record.clone()),
                None => return,
            },
            LinkKey::Local => None,
        };
        let static_key = match &record {
            None => None,
            Some(server) => match self.keys.static_key(&server.key_name) {
                Ok(Some(k)) => Some(k),
                Ok(None) => {
                    self.route(
                        key,
                        Event::Error(format!(
                            "paired with {}, but this client has no key; pair again",
                            server.name
                        )),
                    );
                    self.offline(key);
                    return;
                }
                Err(e) => {
                    self.route(key, Event::Error(format!("reading this client's key: {e}")));
                    self.offline(key);
                    return;
                }
            },
        };
        let wire = self.wire_tx.clone();
        let socket = self.config.socket.clone();
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        if let Some(server) = &record {
            slot.state.set_session_id(server.session_id.clone());
        }
        slot.generation += 1;
        let generation = slot.generation;
        let mut hello = None;
        let mut events = Vec::new();
        for output in slot.state.connected() {
            match output {
                Output::Send(request) if hello.is_none() => {
                    hello = serde_json::to_vec(&request).ok();
                }
                Output::Event(e) => events.push(e),
                other => log::warn!("not sent before hello: {other:?}"),
            }
        }
        let task = tokio::spawn(async move {
            let conn = match (hello, record, static_key) {
                (None, _, _) => {
                    log::error!("the state gave no hello");
                    None
                }
                (Some(hello), Some(server), Some(k)) => {
                    link_connect(&server, &k, &hello).await.map(Conn::Link)
                }
                (Some(hello), _, _) => local_connect(&socket)
                    .await
                    .map(|stream| Conn::Unix(stream, hello)),
            };
            let _ = wire.send(match conn {
                Some(conn) => Wire::Connected {
                    key,
                    generation,
                    conn,
                },
                None => Wire::Failed { key, generation },
            });
        });
        slot.conn = LinkConn::Connecting(task);
        for e in events {
            self.route(key, e);
        }
    }

    /// `key`'s attempt failed or its connection ended: the next attempt
    /// is after its backoff (from the start again once it had said hello).
    fn offline(&mut self, key: LinkKey) {
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        if slot.state.online() {
            slot.backoff.reset();
        }
        let delay = slot.backoff.next_delay();
        slot.stop(tokio::time::Instant::now() + delay);
        let retry_in_ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
        let outputs = slot.state.disconnected(retry_in_ms);
        self.pending.push_back((key, outputs));
    }

    /// Connects `key`'s link now, its backoff started over (a refresh).
    fn reconnect_now(&mut self, key: LinkKey) {
        if let Some(slot) = self.slot_mut(key)
            && !slot.busy()
        {
            slot.backoff.reset();
            slot.conn = LinkConn::Waiting(tokio::time::Instant::now());
        }
    }

    /// What a link's tasks reported.
    async fn on_wire(&mut self, wire: Wire) {
        match wire {
            Wire::Connected {
                key,
                generation,
                conn,
            } => self.opened(key, generation, conn).await,
            Wire::Failed { key, generation } => {
                if self
                    .slot_mut(key)
                    .is_some_and(|s| s.generation == generation)
                {
                    self.offline(key);
                }
            }
            Wire::Message {
                key,
                generation,
                message,
            } => {
                if self
                    .slot_mut(key)
                    .is_none_or(|s| s.generation != generation)
                {
                    return;
                }
                let tick = self.tick_for(key);
                let Some(slot) = self.slot_mut(key) else {
                    return;
                };
                let outputs = match message {
                    Message::Terminal(frame) => slot.state.terminal_frame(&frame),
                    Message::Binary(message) => {
                        let mut outputs = slot.state.input(tick);
                        outputs.extend(slot.state.binary(&message));
                        outputs
                    }
                    Message::Json(message) => match serde_json::from_slice::<Incoming>(&message) {
                        Ok(message) => {
                            let mut outputs = slot.state.input(tick);
                            outputs.extend(slot.state.incoming(message));
                            outputs
                        }
                        Err(e) => {
                            log::warn!("a message from the daemon is not JSON-RPC: {e}");
                            return;
                        }
                    },
                    other => {
                        log::debug!("ignoring a link message of kind {:?}", other.kind());
                        return;
                    }
                };
                self.pending.push_back((key, outputs));
                self.persist_session(key);
            }
            Wire::Closed { key, generation } => {
                if self
                    .slot_mut(key)
                    .is_some_and(|s| s.generation == generation)
                {
                    self.offline(key);
                }
            }
            Wire::Written {
                key,
                generation,
                transfer,
            } => {
                let Some(slot) = self.slot_mut(key).filter(|s| s.generation == generation) else {
                    return;
                };
                let outputs = slot.state.binary_written(transfer);
                self.pending.push_back((key, outputs));
            }
        }
    }

    /// `key`'s attempt `generation` connected: its reader starts, and on
    /// the socket `hello` is sent.
    async fn opened(&mut self, key: LinkKey, generation: u64, conn: Conn) {
        let bytes = Arc::clone(&self.bytes);
        let wire = self.wire_tx.clone();
        let Some(slot) = self
            .links
            .iter_mut()
            .find(|l| l.key == key && l.generation == generation)
        else {
            return;
        };
        let (messages_tx, mut messages) = tokio::sync::mpsc::unbounded_channel();
        let (reader, write, first) = match conn {
            Conn::Unix(stream, hello) => {
                let (read, write) = stream.into_split();
                (
                    read_lines(read, messages_tx, Arc::clone(&bytes)),
                    Writer::Unix(write),
                    Some(hello),
                )
            }
            Conn::Link(stream) => {
                let (read, write) = stream.into_split();
                let counted = Arc::clone(&bytes);
                let reader = tokio::spawn(async move {
                    let mut read = read;
                    loop {
                        match read.recv().await {
                            Ok(message) => {
                                counted.fetch_add(message.bytes().len() as u64, Ordering::Relaxed);
                                if messages_tx.send(message).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                log::debug!("reading from the link: {e}");
                                break;
                            }
                        }
                    }
                });
                (reader, Writer::Link(write), None)
            }
        };
        // Tags what the reader reads with the link; its end, once the
        // reader stops or is aborted.
        tokio::spawn(async move {
            while let Some(message) = messages.recv().await {
                let sent = wire.send(Wire::Message {
                    key,
                    generation,
                    message,
                });
                if sent.is_err() {
                    return;
                }
            }
            let _ = wire.send(Wire::Closed { key, generation });
        });
        let write = WriteQueue::spawn(write, bytes, self.wire_tx.clone(), key, generation);
        if let Some(hello) = first {
            let _ = write.control.send(Control::Json(hello));
        }
        slot.conn = LinkConn::Open { write, reader };
    }

    /// Stores a `session_id` that `hello` changed in `key`'s record.
    fn persist_session(&mut self, key: LinkKey) {
        let LinkKey::Remote(id) = key else {
            return;
        };
        let Some(current) = self
            .links
            .iter()
            .find(|l| l.key == key)
            .map(|l| l.state.session_id().map(str::to_string))
        else {
            return;
        };
        let Some(listed) = self.remotes.get_mut(id) else {
            return;
        };
        if listed.session_id == current {
            return;
        }
        listed.session_id = current;
        if let Err(e) = self.keys.set_remotes(&self.remotes) {
            log::warn!("storing the session id: {e}");
        }
    }

    /// Carries out the outputs waiting, in order, until none are left.
    async fn flush(&mut self) {
        while let Some((key, outputs)) = self.pending.pop_front() {
            self.carry_out(key, outputs).await;
            self.send_binary(key);
        }
    }

    /// Queues what `key`'s state has to send of kind 2: CHUNKs on the bulk
    /// queue, the rest on control. Dropped while the link is not open (the
    /// state ends its transfers when it hears so).
    fn send_binary(&mut self, key: LinkKey) {
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        let messages = slot.state.take_binary();
        if messages.is_empty() {
            return;
        }
        let LinkConn::Open { write, .. } = &slot.conn else {
            log::debug!(
                "not connected: {} binary message(s) dropped",
                messages.len()
            );
            return;
        };
        for (message, chunk_of) in messages {
            let _ = match chunk_of {
                Some(transfer) => write.bulk.send((message, transfer)).is_ok(),
                None => write.control.send(Control::Binary(message)).is_ok(),
            };
        }
    }

    /// Carries out `key`'s state's outputs: its events to the app, its
    /// requests on its link (logged and dropped while it is not open).
    async fn carry_out(&mut self, key: LinkKey, outputs: Vec<Output>) {
        for output in outputs {
            let message = match output {
                Output::Event(e) => {
                    self.route(key, e);
                    continue;
                }
                Output::Send(request) => match serde_json::to_vec(&request) {
                    Ok(message) => message,
                    Err(e) => {
                        log::error!("serializing {}: {e}", request.method);
                        continue;
                    }
                },
                Output::Notify(note) => match serde_json::to_vec(&note) {
                    Ok(message) => message,
                    Err(e) => {
                        log::error!("serializing {}: {e}", note.method);
                        continue;
                    }
                },
                Output::Close(reason) => {
                    if self.slot_mut(key).is_some_and(|s| s.busy()) {
                        log::warn!("closing the connection: {reason}");
                        self.offline(key);
                    }
                    continue;
                }
                // The remote is removed: its link with it.
                Output::Remove { id, daemon } => {
                    self.finish_remove(id, daemon);
                    continue;
                }
                Output::Reconnect => {
                    self.reconnect_now(key);
                    continue;
                }
            };
            let Some(slot) = self.slot_mut(key) else {
                continue;
            };
            let LinkConn::Open { write, .. } = &slot.conn else {
                log::debug!("not connected: a message to the daemon dropped");
                continue;
            };
            // A writer that failed has reported the connection closed.
            let _ = write.control.send(Control::Json(message));
        }
    }
}

/// The note when the list is full (decision 12).
fn full_note() -> String {
    format!("{MAX_REMOTES} remotes is the most this phone keeps: remove one to pair another")
}

/// Connects to `server` over flankr link: TCP to its addresses in order,
/// `IK` with this phone's `key` for it, and `hello` in the handshake.
async fn link_connect(
    server: &ServerRecord,
    key: &Arc<dyn StaticKey>,
    hello: &[u8],
) -> Option<LinkStream<TcpStream>> {
    for addr in &server.addresses {
        let stream = match tcp_connect(addr).await {
            Ok(stream) => stream,
            Err(e) => {
                log::debug!("connecting to {addr}: {e}");
                continue;
            }
        };
        let handshake = link::reconnect(stream, key, &server.public_key, Some(hello));
        match timeout(HANDSHAKE_TIMEOUT, handshake).await {
            Ok(Ok(stream)) => {
                log::info!("connected to {} at {addr}", server.name);
                return Some(stream);
            }
            Ok(Err(e)) => log::debug!("reconnecting to {addr}: {e}"),
            Err(_) => log::debug!("reconnecting to {addr}: the handshake timed out"),
        }
    }
    None
}

/// Connects to the daemon's socket.
async fn local_connect(socket: &std::path::Path) -> Option<UnixStream> {
    match UnixStream::connect(socket).await {
        Ok(stream) => {
            log::info!("connected to {}", socket.display());
            Some(stream)
        }
        Err(e) => {
            log::debug!("connecting to {}: {e}", socket.display());
            None
        }
    }
}

/// Opens a link to one of `addresses` with `IK` and `hello`, each address
/// bounded by [`HANDSHAKE_TIMEOUT`].
async fn probe_connect(
    addresses: &[String],
    key: &Arc<dyn StaticKey>,
    server_key: &[u8; 32],
    hello: &[u8],
) -> Option<LinkStream<TcpStream>> {
    for addr in addresses {
        let attempt = async {
            let stream = tcp_connect(addr).await.map_err(|e| e.to_string())?;
            link::reconnect(stream, key, server_key, Some(hello))
                .await
                .map_err(|e| e.to_string())
        };
        match timeout(HANDSHAKE_TIMEOUT, attempt).await {
            Ok(Ok(stream)) => return Some(stream),
            Ok(Err(e)) => log::debug!("probe: {addr}: {e}"),
            Err(_) => log::debug!("probe: {addr}: timed out"),
        }
    }
    None
}

/// Reads JSON-RPC messages from `stream` until the response to `id`, for
/// up to `wait`.
async fn response_to(
    stream: &mut LinkStream<TcpStream>,
    id: Option<&Id>,
    wait: Duration,
) -> Option<libflankr_proto::Response> {
    let read = async {
        loop {
            let Ok(message) = stream.recv().await else {
                return None;
            };
            let Message::Json(bytes) = message else {
                continue;
            };
            if let Ok(Incoming::Response(response)) = serde_json::from_slice::<Incoming>(&bytes)
                && (id.is_none() || response.id.as_ref() == id)
            {
                return Some(response);
            }
        }
    };
    timeout(wait, read).await.ok().flatten()
}

/// The pairing probe (decision 10): tries `IK` with `hello` every
/// [`PAIRING_RETRY`] for `window`, until a `hello` is answered: the daemon
/// knows this key, the code having been added. The connection is then
/// closed. Whether it was answered.
async fn probe(
    addresses: &[String],
    key: Arc<dyn StaticKey>,
    server_key: &[u8; 32],
    hello: &[u8],
    window: Duration,
) -> bool {
    let until = tokio::time::Instant::now() + window;
    loop {
        if let Some(mut stream) = probe_connect(addresses, &key, server_key, hello).await {
            let answered = response_to(&mut stream, None, HANDSHAKE_TIMEOUT).await;
            let _ = stream.shutdown().await;
            match answered {
                Some(response) if response.error.is_none() => return true,
                Some(response) => log::warn!("probe: hello refused: {:?}", response.error),
                None => log::debug!("probe: hello not answered"),
            }
        }
        let now = tokio::time::Instant::now();
        if now >= until {
            return false;
        }
        tokio::time::sleep(PAIRING_RETRY.min(until - now)).await;
    }
}

/// The pairing request (the 14:37 EDT amendment, item 1): `request` over
/// `NK` to each of `addresses` in order, each bounded by
/// [`HANDSHAKE_TIMEOUT`], until a daemon with `server_key` answers it. A
/// refusal ends it; an accepted request is reported as [`Done::Asked`] on
/// `tasks`, and the daemon's [`PairResult`] is awaited for the offer's time
/// left and [`RESULT_GRACE`]. Returns how it ended: [`Done::Paired`] once
/// the key is added, [`Done::Unreachable`] if no address answered.
/// Dropping the task (Cancel) closes the connection.
async fn pair_request(
    addresses: &[String],
    server_key: &[u8; 32],
    request: &PairRequest,
    token: u64,
    tasks: &UnboundedSender<Done>,
) -> Done {
    for addr in addresses {
        let attempt = async {
            let stream = tcp_connect(addr).await.map_err(|e| e.to_string())?;
            link::request_pairing(stream, server_key, request)
                .await
                .map_err(|e| e.to_string())
        };
        let mut response = match timeout(HANDSHAKE_TIMEOUT, attempt).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                log::debug!("pairing request: {addr}: {e}");
                continue;
            }
            Err(_) => {
                log::debug!("pairing request: {addr}: timed out");
                continue;
            }
        };
        let expires_in_secs = match response.ack {
            PairAck::Accepted { expires_in_secs } => expires_in_secs,
            PairAck::Refused(reason) => {
                return Done::Ended {
                    token,
                    ended: Ended::Refused(reason),
                };
            }
        };
        let _ = tasks.send(Done::Asked {
            token,
            address: addr.clone(),
            expires_in_secs,
        });
        let wait = Duration::from_secs(u64::from(expires_in_secs)) + RESULT_GRACE;
        let result = timeout(wait, link::recv_pair_result(&mut response.stream)).await;
        let _ = response.stream.shutdown().await;
        let ended = match result {
            Ok(Ok(PairResult::Added)) => return Done::Paired { token },
            Ok(Ok(result)) => Ended::Result(result),
            Ok(Err(e)) => {
                log::debug!("pairing request: {addr}: {e}");
                Ended::Dropped
            }
            Err(_) => Ended::TimedOut,
        };
        return Done::Ended { token, ended };
    }
    Done::Unreachable { token }
}

/// What the phone says when a pairing request with `name` ended without
/// adding it.
fn ended_note(name: &str, ended: &Ended) -> String {
    match ended {
        Ended::Refused(PairRefusal::WrongOffer) => format!(
            "{name} refused the request: the link is not from its current flankr pair run; \
             run flankr pair again"
        ),
        Ended::Refused(PairRefusal::Expired) | Ended::Result(PairResult::Expired) => {
            format!("the link from {name} expired; run flankr pair again")
        }
        Ended::Refused(PairRefusal::Busy) => {
            format!("{name} is already pairing another phone; run flankr pair again")
        }
        Ended::Refused(PairRefusal::Malformed) => {
            format!("{name} could not read the pairing request")
        }
        Ended::Refused(other) => format!("{name} refused the request ({other:?})"),
        Ended::Result(PairResult::Refused) => {
            format!("not confirmed on {name}: nothing was added")
        }
        Ended::Result(PairResult::Added) => format!("paired with {name}"),
        Ended::Result(other) => format!("{name} ended the pairing ({other:?})"),
        Ended::Dropped => {
            format!("{name} closed the connection before the pairing was confirmed")
        }
        Ended::TimedOut => format!("{name} did not answer in time; run flankr pair again"),
    }
}

/// The address check: one `IK` connection to `addresses` with `key` and
/// `hello`. Whether `hello` was answered without an error: the remote is
/// there and still knows this phone.
async fn check(
    addresses: &[String],
    key: Arc<dyn StaticKey>,
    server_key: &[u8; 32],
    hello: &[u8],
) -> bool {
    let Some(mut stream) = probe_connect(addresses, &key, server_key, hello).await else {
        return false;
    };
    let answered = response_to(&mut stream, None, HANDSHAKE_TIMEOUT).await;
    let _ = stream.shutdown().await;
    answered.is_some_and(|r| r.error.is_none())
}

/// The revoke probe (decision 22): one `IK` connection to the remote with
/// its key, `hello`, then `link.unpair`, all within [`REVOKE_TIMEOUT`].
/// Whether the daemon answered that it forgot this phone.
async fn revoke(record: &ServerRecord, key: Arc<dyn StaticKey>, hello: &[u8]) -> bool {
    let ask = async {
        let mut stream = probe_connect(&record.addresses, &key, &record.public_key, hello).await?;
        response_to(&mut stream, None, HANDSHAKE_TIMEOUT).await?;
        let id = Id::Number(2);
        let request = Request::new(
            id.clone(),
            methods::LINK_UNPAIR,
            common::LinkUnpairParams {},
        );
        let message = serde_json::to_vec(&request).ok()?;
        stream.send_message(&message).await.ok()?;
        let response = response_to(&mut stream, Some(&id), UNPAIR_WAIT).await;
        let _ = stream.shutdown().await;
        Some(response.is_some_and(|r| r.error.is_none()))
    };
    let daemon = timeout(REVOKE_TIMEOUT, ask)
        .await
        .ok()
        .flatten()
        .unwrap_or(false);
    log::info!(
        "remotes: {} {}",
        record.name,
        if daemon {
            "forgot this phone"
        } else {
            "was not reached; removed here only"
        }
    );
    daemon
}

/// Reads lines from the daemon's socket into `lines` until it closes,
/// adding each line's bytes, its newline included, to `bytes`. Each is a
/// JSON-RPC message: the socket has no other kind.
fn read_lines(
    read: tokio::net::unix::OwnedReadHalf,
    lines: UnboundedSender<Message>,
    bytes: Arc<AtomicU64>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut read = BufReader::new(read);
        loop {
            let mut line = Vec::new();
            let limit = (MAX_LINE + 1) as u64;
            match (&mut read).take(limit).read_until(b'\n', &mut line).await {
                Ok(0) => break,
                Ok(_) if line.len() > MAX_LINE => {
                    log::warn!("a line from the daemon is over {MAX_LINE} bytes");
                    break;
                }
                Ok(n) => {
                    bytes.fetch_add(n as u64, Ordering::Relaxed);
                    if lines.send(Message::Json(line)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    log::debug!("reading from the daemon: {e}");
                    break;
                }
            }
        }
    })
}

/// Connects to `host:port`, trying each address it resolves to, with
/// `SO_KEEPALIVE` set (plan 20261022_02, liveness).
async fn tcp_connect(addr: &str) -> std::io::Result<TcpStream> {
    let mut last = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("{addr} resolves to no address"),
    );
    for target in tokio::net::lookup_host(addr).await? {
        let socket = if target.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };
        socket.set_keepalive(true)?;
        match timeout(CONNECT_TIMEOUT, socket.connect(target)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(e)) => last = e,
            Err(_) => {
                last = std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("connecting to {target} timed out"),
                )
            }
        }
    }
    Err(last)
}

/// The phone's local time of day, as `flankr_speech` reads it.
fn local_now() -> flankr_speech::LocalTime {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let now = secs as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which is a valid, owned
    // value; `now` is a valid time_t.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        // UTC, then.
        let seconds = secs % 86_400;
        return flankr_speech::LocalTime {
            hour: (seconds / 3600) as u8,
            minute: (seconds / 60 % 60) as u8,
            second: (seconds % 60) as u8,
            utc_offset_minutes: 0,
        };
    }
    flankr_speech::LocalTime {
        hour: tm.tm_hour as u8,
        minute: tm.tm_min as u8,
        second: tm.tm_sec as u8,
        utc_offset_minutes: (tm.tm_gmtoff / 60) as i16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_to_30_s_and_resets() {
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(30));
        let secs: Vec<u64> = (0..7).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 16, 30, 30]);
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }
}
