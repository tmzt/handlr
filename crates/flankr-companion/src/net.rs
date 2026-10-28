//! The `net` thread: a Tokio current-thread runtime that keeps a connection
//! to the daemon, reconnects with backoff, and drives
//! [`State`](crate::client::State) (plan 20261022_04, decision 2, and the
//! 18:10 EDT amendment's steps 1 and 2).
//!
//! The phone keeps a list of paired remotes ([`Remotes`], plan 20261028_01)
//! in the [`KeyStore`]. With an active remote, the thread connects to it
//! over flankr link: TCP to the record's addresses in order, `IK` with that
//! remote's own phone key, and `hello` carried in the handshake. Without
//! one, it connects to the daemon's local socket. The active remote's
//! `state.redb` ([`RemoteState`]) is open while it is active.
//!
//! [`Input::Pair`] takes the pairing URL, the bare string or the QR code's
//! data URL (plan 20261022_06, step 5). The thread makes the new remote's
//! key (`noise.static.<id>`), gives its device code as
//! [`Event::PairingCode`] with the confirmation code, and starts a
//! **pairing probe** beside the open connection: a task that tries `IK`
//! with `hello` every [`PAIRING_RETRY`] for the pairing window while the
//! server drops it, until the user pastes the code into `flankr pair`.
//! Once `hello` is answered, the remote is added to the list (and made
//! active only if none was) and [`Event::Paired`] and [`Event::Remotes`]
//! are given. Nothing is stored before then; a probe that gives up or is
//! cancelled ([`Input::CancelPairing`]) deletes its key (plan 20261028_01,
//! decisions 10-12). A link for the remote already being paired keeps the
//! probe, its key and its code, and gives the same code again.
//!
//! A link for a remote already in the list (and whose key this phone has)
//! starts an **address check** instead: one `IK` connection, with that
//! remote's key, to the link's addresses, within [`CHECK_TIMEOUT`]. Only
//! once its `hello` is answered are the link's name and addresses saved and
//! [`Event::RemoteUpdated`] given; until then, and if it fails
//! ([`Event::PairFailed`]), the old ones stay in use (plan 20261028_01, the
//! 14:37 EDT amendment, item 4). A listed remote whose key was lost is
//! paired again with a probe, its code given with `replace`: the daemon
//! still lists this phone's old key under its name, so the code is pasted
//! with `flankr pair --replace`.
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
//! acknowledges it. Terminal bytes count like any other message's.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use libflankr_proto::link::{self, DeviceCode, LinkStream, LinkWriter, Message, StaticKey};
use libflankr_proto::{Id, Incoming, Request, common, common::methods};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{TcpSocket, TcpStream, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::CompanionConfig;
use crate::client::{Event, Input, Output, State, TICK, UNPAIR_WAIT};
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

/// While pairing, how often to try again while the server drops the
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
    let mut state = State::with_terminals(config.client.clone(), terminals);
    state.set_dictation_silence(config.dictation_silence);
    let (tasks_tx, tasks) = tokio::sync::mpsc::unbounded_channel();
    let net = Net {
        state,
        start: tokio::time::Instant::now(),
        backoff: Backoff::new(config.backoff_initial, config.backoff_max),
        config,
        keys,
        remotes: Remotes::default(),
        server: None,
        remote_state: None,
        probe: None,
        checks: Vec::new(),
        next_probe: 0,
        removing: Vec::new(),
        switch: false,
        shown: None,
        tasks_tx,
        tasks,
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
    /// The remote of the current link connection. `None` on the local
    /// socket.
    server: Option<ServerRecord>,
    /// The active remote's `state.redb`, open while it is active.
    remote_state: Option<RemoteState>,
    /// The pairing in progress.
    probe: Option<Probe>,
    /// The address checks in progress, at most one per remote.
    checks: Vec<Check>,
    /// The token of the next probe or check.
    next_probe: u64,
    /// Remotes whose removal has begun.
    removing: Vec<RemoteId>,
    /// The active remote changed: leave the current connection.
    switch: bool,
    /// The rows last given as [`Event::Remotes`].
    shown: Option<Vec<RemoteInfo>>,
    tasks_tx: UnboundedSender<Done>,
    tasks: UnboundedReceiver<Done>,
    state: State,
    /// The clock [`Input::Tick`] carries: the time since the thread began.
    start: tokio::time::Instant,
    backoff: Backoff,
    inputs: UnboundedReceiver<Input>,
    events: mpsc::Sender<Event>,
    /// The bytes sent and received since the thread began.
    bytes: Arc<AtomicU64>,
    /// Sees each event, on this thread, before the app is given it.
    tap: Tap,
}

/// A pairing in progress beside the open connection (decision 10).
struct Probe {
    token: u64,
    /// The remote it pairs: a new record, or the listed one whose key was
    /// missing.
    record: ServerRecord,
    /// Whether the record is already in the list.
    listed: bool,
    /// The [`Event::PairingCode`] it gave, given again for a link to the
    /// same remote.
    shown: Event,
    task: JoinHandle<()>,
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

/// How a session ended.
enum End {
    /// The connection closed, or the active remote changed; reconnect.
    Closed,
    /// The app is gone; stop.
    AppGone,
}

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

/// Unix seconds now.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Net {
    async fn run(mut self) {
        self.load_remotes();
        loop {
            self.switch = false;
            if let Some(conn) = self.connect().await {
                match self.session(conn).await {
                    End::AppGone => return,
                    End::Closed => {}
                }
            }
            if self.state.online() || self.switch {
                self.backoff.reset();
            }
            let delay = if self.switch {
                Duration::ZERO
            } else {
                self.backoff.next_delay()
            };
            let retry_in_ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
            let outputs = self.state.disconnected(retry_in_ms);
            if self.emit_offline(outputs) {
                self.backoff.reset();
                continue;
            }
            self.show_remotes();
            if self.switch {
                continue;
            }
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    done = self.tasks.recv() => {
                        if let Some(done) = done {
                            self.task_done(done);
                        }
                        if self.switch {
                            break;
                        }
                    }
                    input = self.inputs.recv() => {
                        let Some(input) = input else { return };
                        let Some(input) = self.net_input(input) else {
                            if self.switch {
                                break;
                            }
                            continue;
                        };
                        let tick = self.tick();
                        let mut outputs = self.state.input(tick);
                        outputs.extend(self.state.input(input));
                        if self.emit_offline(outputs) {
                            // A refresh: connect now, the backoff
                            // started over.
                            self.backoff.reset();
                            break;
                        }
                        if self.switch {
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Reads the list (migrating the old single record), moves the data
    /// plan's app-wide store in, sweeps state directories no remote lists,
    /// opens the active remote's state, and gives [`Event::Remotes`].
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
        self.open_state();
        self.show_remotes();
    }

    /// Opens the active remote's `state.redb`, closing any other.
    fn open_state(&mut self) {
        let active = self.remotes.active_record().cloned();
        if self.remote_state.as_ref().map(RemoteState::id) == active.as_ref().map(|r| r.id) {
            return;
        }
        self.remote_state = None;
        let (Some(dir), Some(remote)) = (self.config.data_dir.as_deref(), active) else {
            return;
        };
        match RemoteState::open(dir, &remote) {
            Ok(state) => self.remote_state = Some(state),
            Err(e) => log::warn!("state: opening {}'s: {e}", remote.name),
        }
    }

    /// Gives [`Event::Remotes`] if the rows changed since last given.
    fn show_remotes(&mut self) {
        let online = self.state.online() && self.server.is_some();
        let rows = self.remotes.info(online);
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

    /// The inputs the thread handles itself: pairing and removing remotes.
    /// Others are returned for the state; so is [`Input::RemoveRemote`] of
    /// the active remote, which goes over the open link.
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
            input => Some(input),
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
                let hello = serde_json::to_vec(&self.state.hello_request()).unwrap_or_default();
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

    /// Removes `id` here: its state closed and its directory deleted, its
    /// key deleted, the list written without it (last), then the events
    /// (decisions 22 and 23). Removing the active remote makes the most
    /// recently paired of the rest active.
    fn finish_remove(&mut self, id: RemoteId, daemon: bool) {
        self.removing.retain(|r| *r != id);
        let Some(record) = self.remotes.get(id).cloned() else {
            return;
        };
        let was_active = self.remotes.active == Some(id);
        if was_active {
            self.remote_state = None;
            self.server = None;
            self.state.set_session_id(None);
        }
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
        self.remotes.list.retain(|r| r.id != id);
        if was_active {
            self.remotes.active = self.remotes.most_recent(&self.removing);
            self.switch = true;
            // Another daemon's sessions are not to be shown from the cache.
            let outputs = self.state.input(Input::ClearCache);
            for output in outputs {
                if let Output::Event(e) = output {
                    self.emit(e);
                }
            }
        }
        self.write_remotes();
        log::info!(
            "remotes: removed {}{}",
            record.name,
            if daemon { "" } else { " here only" }
        );
        self.open_state();
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
        if let Some(server) = self.server.as_mut().filter(|s| s.id == check.id) {
            server.name = check.name.clone();
            server.addresses = check.addresses;
        }
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
        if first {
            self.switch = true;
            let outputs = self.state.input(Input::ClearCache);
            for output in outputs {
                if let Output::Event(e) = output {
                    self.emit(e);
                }
            }
            self.open_state();
        }
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

    /// The clock now, as an input. Given before every input and message,
    /// so the state's times are those of what it is told. The phone's
    /// local time of day goes with it, for reading clock times as relative
    /// when the daemon cannot say how to read a reply.
    fn tick(&mut self) -> Input {
        self.state.set_local_time(local_now());
        Input::Tick(self.start.elapsed())
    }

    fn emit(&self, event: Event) {
        (self.tap)(&event, self.bytes.load(Ordering::Relaxed));
        let _ = self.events.send(event);
    }

    /// Gives events while there is no connection, and removes the active
    /// remote when told to. The state sends nothing while offline, so
    /// anything else is only logged. Whether the state asked to connect
    /// now ([`Output::Reconnect`]).
    fn emit_offline(&mut self, outputs: Vec<Output>) -> bool {
        let mut reconnect = false;
        for output in outputs {
            match output {
                Output::Event(e) => self.emit(e),
                Output::Remove { id, daemon } => self.finish_remove(id, daemon),
                Output::Reconnect => reconnect = true,
                other => log::warn!("nothing to send it on: {other:?}"),
            }
        }
        reconnect
    }

    /// Starts a connection in the state and returns its `hello`, serialized.
    fn hello(&mut self) -> Result<Vec<u8>, String> {
        let mut hello = None;
        for output in self.state.connected() {
            match output {
                Output::Send(request) if hello.is_none() => {
                    hello = Some(serde_json::to_vec(&request).map_err(|e| e.to_string())?);
                }
                Output::Event(e) => self.emit(e),
                other => log::warn!("not sent before hello: {other:?}"),
            }
        }
        hello.ok_or_else(|| "the state gave no hello".to_string())
    }

    /// Connects to the active remote over the link, or, with none, to the
    /// daemon's socket.
    async fn connect(&mut self) -> Option<Conn> {
        self.server = None;
        let Some(server) = self.remotes.active_record().cloned() else {
            return self.connect_local().await;
        };
        let key = match self.keys.static_key(&server.key_name) {
            Ok(Some(key)) => key,
            Ok(None) => {
                self.emit(Event::Error(format!(
                    "paired with {}, but this client has no key; pair again",
                    server.name
                )));
                return None;
            }
            Err(e) => {
                self.emit(Event::Error(format!("reading this client's key: {e}")));
                return None;
            }
        };
        self.state.set_session_id(server.session_id.clone());
        let hello = match self.hello() {
            Ok(hello) => hello,
            Err(e) => {
                log::error!("{e}");
                return None;
            }
        };
        for addr in &server.addresses {
            let stream = match tcp_connect(addr).await {
                Ok(stream) => stream,
                Err(e) => {
                    log::debug!("connecting to {addr}: {e}");
                    continue;
                }
            };
            let handshake = link::reconnect(stream, &key, &server.public_key, Some(&hello));
            match timeout(HANDSHAKE_TIMEOUT, handshake).await {
                Ok(Ok(stream)) => {
                    log::info!("connected to {} at {addr}", server.name);
                    self.server = Some(server);
                    return Some(Conn::Link(stream));
                }
                Ok(Err(e)) => log::debug!("reconnecting to {addr}: {e}"),
                Err(_) => log::debug!("reconnecting to {addr}: the handshake timed out"),
            }
        }
        None
    }

    async fn connect_local(&mut self) -> Option<Conn> {
        let socket = &self.config.socket;
        match UnixStream::connect(socket).await {
            Ok(stream) => {
                log::info!("connected to {}", socket.display());
                match self.hello() {
                    Ok(hello) => Some(Conn::Unix(stream, hello)),
                    Err(e) => {
                        log::error!("{e}");
                        None
                    }
                }
            }
            Err(e) => {
                log::debug!("connecting to {}: {e}", socket.display());
                None
            }
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

    /// Pairs with `p` beside the open connection (decisions 10-12). A
    /// link for the remote being paired gives its code again, keeping the
    /// probe and its key. A listed remote whose key this phone has gets an
    /// address check ([`Net::check_addresses`]); a new one (or a listed one
    /// whose key is missing) gets its key, its device code and a pairing
    /// probe. Nothing new is stored until the probe's `hello` is answered.
    fn begin_pairing(&mut self, p: Pairing) {
        let server_key = p.payload.server_key;
        let id = RemoteId::of(&server_key);
        if let Some(probe) = self
            .probe
            .as_ref()
            .filter(|probe| probe.record.public_key == server_key)
        {
            log::info!(
                "pairing: already pairing with {}; its code stays",
                probe.record.name
            );
            let shown = probe.shown.clone();
            self.emit(shown);
            return;
        }
        self.end_probe("replaced by a new pairing");
        let offered = ServerRecord::new(p.server.clone(), server_key, p.payload.addresses);
        let listed = match self.remotes.get(id) {
            Some(listed) if listed.public_key != server_key => {
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
                match self.keys.static_key(&listed.key_name) {
                    Ok(Some(key)) => {
                        self.check_addresses(&listed, offered.name, offered.addresses, key);
                        return;
                    }
                    Ok(None) => {
                        log::info!("pairing: {}'s key is missing; pairing again", listed.name)
                    }
                    Err(e) => log::warn!("pairing: {}'s key: {e}; pairing again", listed.name),
                }
                // Saved with the link's name and addresses only once the
                // probe reaches them.
                self.end_check(id);
                ServerRecord {
                    name: offered.name,
                    addresses: offered.addresses,
                    ..listed
                }
            }
            None if self.remotes.list.len() >= MAX_REMOTES => {
                self.emit(Event::PairFailed(full_note()));
                return;
            }
            None => offered,
        };
        let listed = self.remotes.get(id).is_some();
        let key = match self.keys.generate_static_key(&record.key_name) {
            Ok(key) => key,
            Err(e) => {
                self.emit(Event::PairFailed(format!("this client's key: {e}")));
                return;
            }
        };
        let code = DeviceCode {
            key: key.public(),
            name: self.config.device_name.clone(),
        }
        .encode(&server_key);
        let code = match code {
            Ok(code) => code,
            Err(e) => {
                let _ = self.keys.delete_static_key(&record.key_name);
                self.emit(Event::PairFailed(format!(
                    "the device name '{}': {e}",
                    self.config.device_name
                )));
                return;
            }
        };
        log::info!(
            "pairing with {} ({}); waiting for the code to be added",
            record.name,
            id.fingerprint()
        );
        let token = self.next_probe;
        self.next_probe += 1;
        let hello = serde_json::to_vec(&self.state.hello_request()).unwrap_or_default();
        let tasks = self.tasks_tx.clone();
        let window = self.config.pairing_window;
        let addresses = record.addresses.clone();
        let task = tokio::spawn(async move {
            let done = if probe(&addresses, key, &server_key, &hello, window).await {
                Done::Paired { token }
            } else {
                Done::GaveUp { token }
            };
            let _ = tasks.send(done);
        });
        let shown = Event::PairingCode {
            code,
            server: p.server,
            confirmation: link::confirmation_code(&server_key),
            replace: listed,
        };
        self.probe = Some(Probe {
            token,
            record,
            listed,
            shown: shown.clone(),
            task,
        });
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
        let hello = serde_json::to_vec(&self.state.hello_request()).unwrap_or_default();
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

    /// Stores a `session_id` that `hello` changed in the active record.
    fn persist_session(&mut self) {
        let Some(server) = self.server.as_mut() else {
            return;
        };
        let current = self.state.session_id();
        if server.session_id.as_deref() == current {
            return;
        }
        server.session_id = current.map(str::to_string);
        let (id, session_id) = (server.id, server.session_id.clone());
        let Some(listed) = self.remotes.get_mut(id) else {
            return;
        };
        listed.session_id = session_id;
        if let Err(e) = self.keys.set_remotes(&self.remotes) {
            log::warn!("storing the session id: {e}");
        }
    }

    async fn session(&mut self, conn: Conn) -> End {
        let (messages_tx, mut messages) = tokio::sync::mpsc::unbounded_channel();
        let (reader, mut write, first) = match conn {
            Conn::Unix(stream, hello) => {
                let (read, write) = stream.into_split();
                (
                    read_lines(read, messages_tx, Arc::clone(&self.bytes)),
                    Writer::Unix(write),
                    Some(hello),
                )
            }
            Conn::Link(stream) => {
                let (read, write) = stream.into_split();
                let bytes = Arc::clone(&self.bytes);
                let reader = tokio::spawn(async move {
                    let mut read = read;
                    loop {
                        match read.recv().await {
                            Ok(message) => {
                                bytes.fetch_add(message.bytes().len() as u64, Ordering::Relaxed);
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
        let mut end = None;
        if let Some(hello) = first
            && let Err(e) = write.send(&hello, &self.bytes).await
        {
            log::debug!("writing to the daemon: {e}");
            end = Some(End::Closed);
        }
        let mut ticks = tokio::time::interval(TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        while end.is_none() {
            tokio::select! {
                _ = ticks.tick() => {
                    let tick = self.tick();
                    let outputs = self.state.input(tick);
                    end = self.carry_out(outputs, &mut write).await;
                }
                message = messages.recv() => match message {
                    Some(Message::Terminal(frame)) => {
                        let outputs = self.state.terminal_frame(&frame);
                        end = self.carry_out(outputs, &mut write).await;
                    }
                    Some(Message::Json(message)) => match serde_json::from_slice::<Incoming>(&message) {
                        Ok(message) => {
                            let tick = self.tick();
                            let mut outputs = self.state.input(tick);
                            outputs.extend(self.state.incoming(message));
                            self.persist_session();
                            end = self.carry_out(outputs, &mut write).await;
                            self.show_remotes();
                        }
                        Err(e) => log::warn!("a message from the daemon is not JSON-RPC: {e}"),
                    },
                    Some(other) => log::debug!("ignoring a link message of kind {:?}", other.kind()),
                    None => end = Some(End::Closed),
                },
                done = self.tasks.recv() => {
                    if let Some(done) = done {
                        self.task_done(done);
                    }
                }
                input = self.inputs.recv() => match input {
                    Some(input) => {
                        if let Some(input) = self.net_input(input) {
                            let tick = self.tick();
                            let mut outputs = self.state.input(tick);
                            outputs.extend(self.state.input(input));
                            end = self.carry_out(outputs, &mut write).await;
                        }
                    }
                    None => end = Some(End::AppGone),
                },
            }
            if end.is_none() && self.switch {
                end = Some(End::Closed);
            }
        }
        reader.abort();
        end.unwrap_or(End::Closed)
    }

    /// Carries out the state's outputs; `Some` when the session must end.
    async fn carry_out(&mut self, outputs: Vec<Output>, write: &mut Writer) -> Option<End> {
        for output in outputs {
            match output {
                Output::Event(e) => self.emit(e),
                Output::Send(request) => {
                    let message = match serde_json::to_vec(&request) {
                        Ok(message) => message,
                        Err(e) => {
                            log::error!("serializing {}: {e}", request.method);
                            continue;
                        }
                    };
                    if let Err(e) = write.send(&message, &self.bytes).await {
                        log::debug!("writing to the daemon: {e}");
                        return Some(End::Closed);
                    }
                }
                Output::Notify(note) => {
                    let message = match serde_json::to_vec(&note) {
                        Ok(message) => message,
                        Err(e) => {
                            log::error!("serializing {}: {e}", note.method);
                            continue;
                        }
                    };
                    if let Err(e) = write.send(&message, &self.bytes).await {
                        log::debug!("writing to the daemon: {e}");
                        return Some(End::Closed);
                    }
                }
                Output::Close(reason) => {
                    log::warn!("closing the connection: {reason}");
                    return Some(End::Closed);
                }
                // The active remote is removed: the connection was its.
                Output::Remove { id, daemon } => {
                    self.finish_remove(id, daemon);
                    return Some(End::Closed);
                }
                // Already connected.
                Output::Reconnect => {}
            }
        }
        None
    }
}

/// The note when the list is full (decision 12).
fn full_note() -> String {
    format!("{MAX_REMOTES} remotes is the most this phone keeps: remove one to pair another")
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
