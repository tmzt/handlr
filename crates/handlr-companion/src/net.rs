//! The `net` thread: a Tokio current-thread runtime that keeps a connection
//! to the daemon, reconnects with backoff, and drives
//! [`State`](crate::client::State) (plan 20261022_04, decision 2, and the
//! 18:10 EDT amendment's steps 1 and 2).
//!
//! With a paired server in the [`KeyStore`] (a server record and this
//! client's static key), it connects over handlr link: TCP to the record's
//! addresses in order, `IK`, and `hello` carried in the handshake. Without
//! one, it connects to the daemon's local socket. [`Input::Pair`] pairs
//! over `XXpsk3` from a `handlr1…` string, stores the server and continues
//! on that connection (plan 20261022_02, the 11:58 EDT amendment).
//!
//! Liveness is the operating system's: link sockets have `SO_KEEPALIVE`,
//! and a connection ends only on an error or end of stream. The client does
//! not `ping`.

use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libhandlr_proto::link::{self, LinkError, LinkStream, LinkWriter, PairingPayload};
use libhandlr_proto::Incoming;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{TcpSocket, TcpStream, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::client::{Event, Input, Output, State};
use crate::keys::{KeyStore, ServerRecord};
use crate::CompanionConfig;

/// The longest line read from the daemon's socket (the daemon's own limit).
/// The link has the same limit on its messages.
const MAX_LINE: usize = 1 << 20;

/// How long a TCP connection to one address may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a link handshake may take (as the daemon's, CHANGES.md 18:24).
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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
    let net = Net {
        state: State::new(config.client.clone()),
        backoff: Backoff::new(config.backoff_initial, config.backoff_max),
        config,
        keys,
        server: None,
        inputs,
        events,
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
    backoff: Backoff,
    inputs: UnboundedReceiver<Input>,
    events: mpsc::Sender<Event>,
}

/// How a session ended.
enum End {
    /// The connection closed; reconnect.
    Closed,
    /// The app is gone; stop.
    AppGone,
    /// The app gave a valid pairing string; pair with it now.
    Pair(PairingPayload),
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
    /// link.
    async fn send(&mut self, message: &[u8]) -> Result<(), String> {
        match self {
            Writer::Unix(write) => {
                let mut line = Vec::with_capacity(message.len() + 1);
                line.extend_from_slice(message);
                line.push(b'\n');
                write.write_all(&line).await.map_err(|e| e.to_string())
            }
            Writer::Link(write) => write.send_message(message).await.map_err(|e| e.to_string()),
        }
    }
}

impl Net {
    async fn run(mut self) {
        let mut pairing: Option<PairingPayload> = None;
        loop {
            let conn = match pairing.take() {
                Some(payload) => self.pair(payload).await,
                None => self.connect().await,
            };
            if let Some(conn) = conn {
                match self.session(conn).await {
                    End::AppGone => return,
                    End::Pair(payload) => {
                        // Leave this connection for the new server at once.
                        let outputs = self.state.disconnected(0);
                        self.emit_offline(outputs);
                        self.backoff.reset();
                        pairing = Some(payload);
                        continue;
                    }
                    End::Closed => {}
                }
            }
            if self.state.online() {
                self.backoff.reset();
            }
            let delay = self.backoff.next_delay();
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
                            if let Some(payload) = self.check_pairing(&text) {
                                pairing = Some(payload);
                                break;
                            }
                        }
                        Some(input) => {
                            let outputs = self.state.input(input);
                            self.emit_offline(outputs);
                        }
                        None => return,
                    },
                }
            }
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Gives events while there is no connection. The state sends nothing
    /// while offline, so anything else is only logged.
    fn emit_offline(&self, outputs: Vec<Output>) {
        for output in outputs {
            match output {
                Output::Event(e) => self.emit(e),
                other => log::warn!("nothing to send it on: {other:?}"),
            }
        }
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
        let secret = match self.keys.secret() {
            Ok(Some(secret)) => secret,
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
            let handshake = link::reconnect(stream, &secret, &server.public_key, Some(&hello));
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

    /// Decodes a pairing string and refuses an expired one, before any
    /// connection is made. A refusal is given as [`Event::PairFailed`].
    fn check_pairing(&self, text: &str) -> Option<PairingPayload> {
        let checked = PairingPayload::decode(text)
            .map_err(|e| e.to_string())
            .and_then(|payload| {
                if payload.is_expired(unix_now()) {
                    Err("the pairing string has expired; run handlr pair again".to_string())
                } else if payload.addresses.is_empty() {
                    Err("the pairing string has no addresses".to_string())
                } else {
                    Ok(payload)
                }
            });
        match checked {
            Ok(payload) => Some(payload),
            Err(why) => {
                self.emit(Event::PairFailed(why));
                None
            }
        }
    }

    /// Pairs over `XXpsk3`, trying the payload's addresses in order until
    /// one accepts a TCP connection. A handshake that fails there ends the
    /// attempt: the secret is single use. On success, the server is stored
    /// and the connection, `hello` already sent, is returned.
    async fn pair(&mut self, payload: PairingPayload) -> Option<Conn> {
        let private = match self.static_key() {
            Ok(private) => private,
            Err(e) => {
                self.emit(Event::PairFailed(format!("this client's key: {e}")));
                return None;
            }
        };
        // A new server: no session to continue.
        let previous_session = self.state.session_id().map(str::to_string);
        self.state.set_session_id(None);
        let result = self.pair_with(&payload, &private).await;
        match result {
            Ok(stream) => {
                self.emit(Event::Paired {
                    name: payload.name.clone(),
                });
                Some(Conn::Link(stream))
            }
            Err(why) => {
                self.state.set_session_id(previous_session);
                self.emit(Event::PairFailed(why));
                None
            }
        }
    }

    async fn pair_with(
        &mut self,
        payload: &PairingPayload,
        private: &[u8; 32],
    ) -> Result<LinkStream<TcpStream>, String> {
        let hello = self.hello()?;
        let mut failures = Vec::new();
        for addr in &payload.addresses {
            let stream = match tcp_connect(addr).await {
                Ok(stream) => stream,
                Err(e) => {
                    failures.push(format!("{addr}: {e}"));
                    continue;
                }
            };
            let handshake = link::pair(
                stream,
                private,
                &payload.server_key,
                &payload.secret,
                Some(&hello),
            );
            let stream = match timeout(HANDSHAKE_TIMEOUT, handshake).await {
                Ok(Ok((stream, _))) => stream,
                Ok(Err(LinkError::Closed)) => {
                    return Err(format!(
                        "{addr} closed the connection: the pairing string may be used, \
                         expired or for another server"
                    ))
                }
                Ok(Err(e)) => return Err(format!("pairing with {addr}: {e}")),
                Err(_) => return Err(format!("pairing with {addr}: the handshake timed out")),
            };
            let record = ServerRecord {
                name: payload.name.clone(),
                public_key: payload.server_key,
                addresses: payload.addresses.clone(),
                session_id: None,
            };
            self.keys
                .set_server(record.clone())
                .map_err(|e| format!("storing the paired server: {e}"))?;
            log::info!("paired as {} at {addr}", payload.name);
            self.server = Some(record);
            return Ok(stream);
        }
        Err(format!("could not connect: {}", failures.join("; ")))
    }

    /// This client's static key, made and stored on first use.
    fn static_key(&mut self) -> Result<[u8; 32], String> {
        if let Some(private) = self.keys.secret()? {
            return Ok(private);
        }
        let keypair = link::generate_keypair().map_err(|e| e.to_string())?;
        self.keys.set_secret(keypair.private)?;
        Ok(keypair.private)
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
                    read_lines(read, messages_tx),
                    Writer::Unix(write),
                    Some(hello),
                )
            }
            Conn::Link(stream) => {
                let (read, write) = stream.into_split();
                let reader = tokio::spawn(async move {
                    let mut read = read;
                    loop {
                        match read.recv_message().await {
                            Ok(message) => {
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
        if let Some(hello) = first {
            if let Err(e) = write.send(&hello).await {
                log::debug!("writing to the daemon: {e}");
                end = Some(End::Closed);
            }
        }
        while end.is_none() {
            tokio::select! {
                message = messages.recv() => match message {
                    Some(message) => match serde_json::from_slice::<Incoming>(&message) {
                        Ok(message) => {
                            let outputs = self.state.incoming(message);
                            self.persist_session();
                            end = self.carry_out(outputs, &mut write).await;
                        }
                        Err(e) => log::warn!("a message from the daemon is not JSON-RPC: {e}"),
                    },
                    None => end = Some(End::Closed),
                },
                input = self.inputs.recv() => match input {
                    Some(Input::Pair(text)) => {
                        if let Some(payload) = self.check_pairing(&text) {
                            end = Some(End::Pair(payload));
                        }
                    }
                    Some(input) => {
                        let outputs = self.state.input(input);
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
    async fn carry_out(&self, outputs: Vec<Output>, write: &mut Writer) -> Option<End> {
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
                    if let Err(e) = write.send(&message).await {
                        log::debug!("writing to the daemon: {e}");
                        return Some(End::Closed);
                    }
                }
                Output::Close(reason) => {
                    log::warn!("closing the connection: {reason}");
                    return Some(End::Closed);
                }
            }
        }
        None
    }
}

/// Reads lines from the daemon's socket into `lines` until it closes.
fn read_lines(
    read: tokio::net::unix::OwnedReadHalf,
    lines: UnboundedSender<Vec<u8>>,
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
                Ok(_) => {
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

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
