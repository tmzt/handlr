//! The `net` thread: a Tokio current-thread runtime that keeps a connection
//! to the daemon, reconnects with backoff, and drives
//! [`State`](crate::client::State) (plan 20261022_04, decision 2, and the
//! 18:10 EDT amendment's steps 1 and 2).
//!
//! With a paired server in the [`KeyStore`] (a server record and this
//! client's static key), it connects over handlr link: TCP to the record's
//! addresses in order, `IK`, and `hello` carried in the handshake. Without
//! one, it connects to the daemon's local socket.
//!
//! [`Input::Pair`] takes the pairing URL, the bare string or the QR code's
//! data URL (plan 20261022_06, step 5). The client stores the server's key
//! and addresses, makes its own key if it has none, and gives its device
//! code as [`Event::PairingCode`]. It then connects with `IK` as usual,
//! but retries every [`PAIRING_RETRY`] for [`PAIRING_WINDOW`] while the
//! server drops it, until the user pastes the code into `handlr pair`;
//! after that it falls back to the usual backoff. The `hello` answered with
//! `paired: true` gives [`Event::Paired`] (plan 20261022_02, amendment of
//! 23:07 EDT).
//!
//! [`Input::Unpair`] asks the daemon to forget this device, and the state
//! answers with [`Output::Forget`]: the key and the server record are
//! deleted, the connection ends, and [`Event::Unpaired`] is given (plan
//! 20261024_02, decision 6). The next connection is the local socket's.
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
//! not counted.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use libflankr_proto::Incoming;
use libflankr_proto::link::{self, DeviceCode, LinkStream, LinkWriter, StaticKey};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{TcpSocket, TcpStream, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::CompanionConfig;
use crate::client::{Event, Input, Output, State, TICK};
use crate::keys::{KeyStore, ServerRecord};
use crate::scan::{self, Pairing};

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
/// How long to retry at [`PAIRING_RETRY`] after a pairing input.
pub const PAIRING_WINDOW: Duration = Duration::from_secs(10 * 60);

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
    let mut state = State::new(config.client.clone());
    state.set_dictation_silence(config.dictation_silence);
    let net = Net {
        state,
        start: tokio::time::Instant::now(),
        backoff: Backoff::new(config.backoff_initial, config.backoff_max),
        config,
        keys,
        server: None,
        pairing_until: None,
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
    /// The paired server of the current link connection, as last stored.
    /// `None` on the local socket.
    server: Option<ServerRecord>,
    state: State,
    /// The clock [`Input::Tick`] carries: the time since the thread began.
    start: tokio::time::Instant,
    backoff: Backoff,
    /// While pairing: until when to retry at [`PAIRING_RETRY`].
    pairing_until: Option<tokio::time::Instant>,
    inputs: UnboundedReceiver<Input>,
    events: mpsc::Sender<Event>,
    /// The bytes sent and received since the thread began.
    bytes: Arc<AtomicU64>,
    /// Sees each event, on this thread, before the app is given it.
    tap: Tap,
}

/// A function that sees each event on the `net` thread, with the bytes
/// over the link so far ([`Companion::spawn_tapped`](crate::Companion::spawn_tapped)).
pub(crate) type Tap = Box<dyn Fn(&Event, u64) + Send>;

/// How a session ended.
enum End {
    /// The connection closed; reconnect.
    Closed,
    /// The app is gone; stop.
    AppGone,
    /// The app gave a valid pairing input; pair with it now.
    Pair(Pairing),
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

impl Net {
    async fn run(mut self) {
        let mut pairing: Option<Pairing> = None;
        loop {
            if let Some(p) = pairing.take() {
                self.begin_pairing(p);
            }
            let conn = self.connect().await;
            if let Some(conn) = conn {
                self.pairing_until = None;
                match self.session(conn).await {
                    End::AppGone => return,
                    End::Pair(p) => {
                        // Leave this connection for the new server at once.
                        let outputs = self.state.disconnected(0);
                        self.emit_offline(outputs);
                        self.backoff.reset();
                        pairing = Some(p);
                        continue;
                    }
                    End::Closed => {}
                }
            }
            if self.state.online() {
                self.backoff.reset();
            }
            let delay = match self.pairing_until {
                Some(until) if tokio::time::Instant::now() < until => PAIRING_RETRY,
                _ => {
                    self.pairing_until = None;
                    self.backoff.next_delay()
                }
            };
            let retry_in_ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
            let outputs = self.state.disconnected(retry_in_ms);
            self.emit_offline(outputs);
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    input = self.inputs.recv() => match input {
                        Some(Input::Pair(text)) => {
                            if let Some(p) = self.check_pairing(&text) {
                                pairing = Some(p);
                                break;
                            }
                        }
                        Some(input) => {
                            let tick = self.tick();
                            let mut outputs = self.state.input(tick);
                            outputs.extend(self.state.input(input));
                            if self.emit_offline(outputs) {
                                // A refresh: connect now, the backoff
                                // started over.
                                self.backoff.reset();
                                break;
                            }
                        }
                        None => return,
                    },
                }
            }
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

    /// Gives events while there is no connection, and forgets the pairing
    /// when told to. The state sends nothing while offline, so anything
    /// else is only logged. Whether the state asked to connect now
    /// ([`Output::Reconnect`]).
    fn emit_offline(&mut self, outputs: Vec<Output>) -> bool {
        let mut reconnect = false;
        for output in outputs {
            match output {
                Output::Event(e) => self.emit(e),
                Output::Forget { daemon } => self.forget(daemon),
                Output::Reconnect => reconnect = true,
                other => log::warn!("nothing to send it on: {other:?}"),
            }
        }
        reconnect
    }

    /// Deletes the key and the server record, so the next connection is
    /// the local socket's, and gives [`Event::Unpaired`].
    fn forget(&mut self, daemon: bool) {
        if let Err(e) = self.keys.forget() {
            log::error!("unpairing: {e}");
            self.emit(Event::Error(format!("forgetting the pairing: {e}")));
            return;
        }
        log::info!("unpairing: the pairing is forgotten");
        self.server = None;
        self.pairing_until = None;
        self.state.set_session_id(None);
        self.emit(Event::Unpaired { daemon });
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

    /// Connects to the paired server over the link, or, with none, to the
    /// daemon's socket.
    async fn connect(&mut self) -> Option<Conn> {
        self.server = None;
        let server = match self.keys.server() {
            Ok(server) => server,
            Err(e) => {
                self.emit(Event::Error(format!("reading the paired server: {e}")));
                return None;
            }
        };
        let Some(server) = server else {
            return self.connect_local().await;
        };
        let key = match self.keys.static_key() {
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

    /// Stores the server, makes this client's key if it has none, gives the
    /// device code, and opens the window of quick retries. The connection
    /// itself is the usual `IK`, which the server drops until the code is
    /// added. Nothing is stored if the key or the code cannot be made.
    fn begin_pairing(&mut self, p: Pairing) {
        let key = match self.static_key() {
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
        .encode(&p.payload.server_key);
        let code = match code {
            Ok(code) => code,
            Err(e) => {
                self.emit(Event::PairFailed(format!(
                    "the device name '{}': {e}",
                    self.config.device_name
                )));
                return;
            }
        };
        let record = ServerRecord {
            name: p.server.clone(),
            public_key: p.payload.server_key,
            addresses: p.payload.addresses,
            // A new server: no session to continue.
            session_id: None,
        };
        if let Err(e) = self.keys.set_server(record) {
            self.emit(Event::PairFailed(format!("storing the server: {e}")));
            return;
        }
        log::info!(
            "pairing with {}; waiting for the code to be added",
            p.server
        );
        self.pairing_until = Some(tokio::time::Instant::now() + PAIRING_WINDOW);
        self.emit(Event::PairingCode {
            code,
            server: p.server,
        });
    }

    /// This client's static key, made and stored on first use.
    fn static_key(&mut self) -> Result<Arc<dyn StaticKey>, String> {
        if let Some(key) = self.keys.static_key()? {
            return Ok(key);
        }
        self.keys.generate_static_key()
    }

    /// Stores a `session_id` that `hello` changed in the server record.
    fn persist_session(&mut self) {
        let Some(server) = self.server.as_mut() else {
            return;
        };
        let current = self.state.session_id();
        if server.session_id.as_deref() == current {
            return;
        }
        server.session_id = current.map(str::to_string);
        if let Err(e) = self.keys.set_server(server.clone()) {
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
                        match read.recv_message().await {
                            Ok(message) => {
                                bytes.fetch_add(message.len() as u64, Ordering::Relaxed);
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
                    Some(message) => match serde_json::from_slice::<Incoming>(&message) {
                        Ok(message) => {
                            let tick = self.tick();
                            let mut outputs = self.state.input(tick);
                            if is_paired_hello(&message) {
                                self.emit(Event::Paired {
                                    name: self.config.device_name.clone(),
                                });
                            }
                            outputs.extend(self.state.incoming(message));
                            self.persist_session();
                            end = self.carry_out(outputs, &mut write).await;
                        }
                        Err(e) => log::warn!("a message from the daemon is not JSON-RPC: {e}"),
                    },
                    None => end = Some(End::Closed),
                },
                input = self.inputs.recv() => match input {
                    Some(Input::Pair(text)) => {
                        if let Some(p) = self.check_pairing(&text) {
                            end = Some(End::Pair(p));
                        }
                    }
                    Some(input) => {
                        let tick = self.tick();
                        let mut outputs = self.state.input(tick);
                        outputs.extend(self.state.input(input));
                        end = self.carry_out(outputs, &mut write).await;
                    }
                    None => end = Some(End::AppGone),
                },
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
                Output::Close(reason) => {
                    log::warn!("closing the connection: {reason}");
                    return Some(End::Closed);
                }
                // Unpaired: the connection was the pairing's.
                Output::Forget { daemon } => {
                    self.forget(daemon);
                    return Some(End::Closed);
                }
                // Already connected.
                Output::Reconnect => {}
            }
        }
        None
    }
}

/// Whether `message` is a response saying `paired: true`: the `hello` of
/// the first connection after the code was added. Only `hello` results
/// carry `paired`.
fn is_paired_hello(message: &Incoming) -> bool {
    matches!(
        message,
        Incoming::Response(response)
            if response.result.as_ref().and_then(|r| r.get("paired")) == Some(&serde_json::Value::Bool(true))
    )
}

/// Reads lines from the daemon's socket into `lines` until it closes,
/// adding each line's bytes, its newline included, to `bytes`.
fn read_lines(
    read: tokio::net::unix::OwnedReadHalf,
    lines: UnboundedSender<Vec<u8>>,
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
                    if lines.send(line).is_err() {
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
