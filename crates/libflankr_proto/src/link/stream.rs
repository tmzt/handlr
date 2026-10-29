//! The established link: framing, chunking and the 1 MiB limit.

use std::sync::Arc;

use snow::StatelessTransportState;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

use super::{
    FLAG_MORE, FLAG_RESERVED, KEY_LEN, KIND_BINARY, KIND_JSON, KIND_TERMINAL, LinkError, MAX_CHUNK,
    MAX_FRAME, MAX_MESSAGE,
};

/// A message's kind, as the high nibble of its frames' flag byte carries
/// it (see the [module](super) header). Open: later kinds are additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// A JSON-RPC message ([`KIND_JSON`](super::KIND_JSON)).
    Json,
    /// A terminal frame ([`KIND_TERMINAL`](super::KIND_TERMINAL); plan
    /// 20261027_07, decision 3): its bytes are `libflankr_term`'s encoding.
    Terminal,
    /// A binary transfer message ([`KIND_BINARY`](super::KIND_BINARY); plan
    /// 20261028_05, decision 3.1): its bytes are `libflankr_blob`'s
    /// encoding.
    Binary,
}

impl Kind {
    /// The kind's nibble, before it is shifted into the flag byte's high
    /// half.
    pub fn nibble(self) -> u8 {
        match self {
            Kind::Json => KIND_JSON,
            Kind::Terminal => KIND_TERMINAL,
            Kind::Binary => KIND_BINARY,
        }
    }

    /// The kind a nibble names, if it is one this crate reads.
    pub fn from_nibble(nibble: u8) -> Option<Kind> {
        match nibble {
            KIND_JSON => Some(Kind::Json),
            KIND_TERMINAL => Some(Kind::Terminal),
            KIND_BINARY => Some(Kind::Binary),
            _ => None,
        }
    }
}

/// One whole message as [`LinkStream::recv`] returns it, by its kind.
/// Open: later kinds are additive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Message {
    /// A JSON-RPC message: what [`LinkStream::recv_message`] returns.
    Json(Vec<u8>),
    /// A terminal frame.
    Terminal(Vec<u8>),
    /// A binary transfer message (plan 20261028_05).
    Binary(Vec<u8>),
}

impl Message {
    pub fn kind(&self) -> Kind {
        match self {
            Message::Json(_) => Kind::Json,
            Message::Terminal(_) => Kind::Terminal,
            Message::Binary(_) => Kind::Binary,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Message::Json(b) | Message::Terminal(b) | Message::Binary(b) => b,
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        match self {
            Message::Json(b) | Message::Terminal(b) | Message::Binary(b) => b,
        }
    }

    fn new(kind: Kind, bytes: Vec<u8>) -> Message {
        match kind {
            Kind::Json => Message::Json(bytes),
            Kind::Terminal => Message::Terminal(bytes),
            Kind::Binary => Message::Binary(bytes),
        }
    }
}

/// An established link connection. Messages are whole: one
/// [`send_message`](Self::send_message) is one
/// [`recv_message`](Self::recv_message) on the other side, and one
/// `send_message`, [`send_terminal`](Self::send_terminal) or
/// [`send_binary`](Self::send_binary) is one [`recv`](Self::recv).
///
/// `recv_message` and `recv` are not cancel-safe: a message half read when its future
/// is dropped is lost and the stream is out of step. To read and write at
/// once, [`into_split`](Self::into_split) the stream.
pub struct LinkStream<S> {
    io: S,
    transport: Arc<StatelessTransportState>,
    send_nonce: u64,
    recv_nonce: u64,
    peer_key: [u8; KEY_LEN],
    closed: bool,
}

impl<S: AsyncRead + AsyncWrite + Unpin> LinkStream<S> {
    pub(super) fn new(io: S, transport: StatelessTransportState, peer_key: [u8; KEY_LEN]) -> Self {
        LinkStream {
            io,
            transport: Arc::new(transport),
            send_nonce: 0,
            recv_nonce: 0,
            peer_key,
            closed: false,
        }
    }

    /// The peer's static X25519 public key, authenticated by the handshake.
    pub fn peer_key(&self) -> &[u8; KEY_LEN] {
        &self.peer_key
    }

    /// Sends one message, in as many frames as it needs. A message over
    /// [`MAX_MESSAGE`] is refused without sending anything.
    pub async fn send_message(&mut self, msg: &[u8]) -> Result<(), LinkError> {
        self.send_kind(Kind::Json, msg).await
    }

    /// Sends one terminal frame, as [`send_message`](Self::send_message)
    /// sends a JSON-RPC message. Send it only to a peer that reads with
    /// [`recv`](Self::recv): `recv_message` is `BadFrame` on it.
    pub async fn send_terminal(&mut self, msg: &[u8]) -> Result<(), LinkError> {
        self.send_kind(Kind::Terminal, msg).await
    }

    /// Sends one binary transfer message (plan 20261028_05), as
    /// [`send_message`](Self::send_message) sends a JSON-RPC message. Send
    /// it only to a peer that reads kind 2 (decision 3.2).
    pub async fn send_binary(&mut self, msg: &[u8]) -> Result<(), LinkError> {
        self.send_kind(Kind::Binary, msg).await
    }

    /// Sends one message of `kind`. A message over [`MAX_MESSAGE`] is
    /// refused without sending anything.
    pub async fn send_kind(&mut self, kind: Kind, msg: &[u8]) -> Result<(), LinkError> {
        if msg.len() > MAX_MESSAGE {
            return Err(LinkError::MessageTooLarge);
        }
        self.send_unchecked_kind(kind, msg).await
    }

    #[cfg(test)]
    pub(super) async fn send_unchecked(&mut self, msg: &[u8]) -> Result<(), LinkError> {
        self.send_unchecked_kind(Kind::Json, msg).await
    }

    async fn send_unchecked_kind(&mut self, kind: Kind, msg: &[u8]) -> Result<(), LinkError> {
        if self.closed {
            return Err(LinkError::Closed);
        }
        let r = write_message(
            &mut self.io,
            &self.transport,
            &mut self.send_nonce,
            kind,
            msg,
        )
        .await;
        if r.is_err() {
            self.close().await;
        }
        r
    }

    /// Sends frames with the given flag bytes and chunks as they are, for
    /// tests of the receiver's checks.
    #[cfg(test)]
    pub(super) async fn send_raw_frames(
        &mut self,
        frames: &[(u8, &[u8])],
    ) -> Result<(), LinkError> {
        let mut out = vec![0u8; 2 + MAX_FRAME];
        for &(flag, chunk) in frames {
            let mut plain = vec![flag];
            plain.extend_from_slice(chunk);
            let n = self
                .transport
                .write_message(self.send_nonce, &plain, &mut out[2..])?;
            self.send_nonce += 1;
            out[..2].copy_from_slice(&(n as u16).to_be_bytes());
            self.io.write_all(&out[..2 + n]).await?;
        }
        self.io.flush().await?;
        Ok(())
    }

    /// Receives one whole JSON-RPC message. A frame of any other kind is
    /// [`LinkError::BadFrame`]. On any error, including a message
    /// reassembled past [`MAX_MESSAGE`], the connection is closed.
    pub async fn recv_message(&mut self) -> Result<Vec<u8>, LinkError> {
        self.recv_accepting(false).await.map(Message::into_bytes)
    }

    /// Receives one whole message of any known kind. Errors as
    /// [`recv_message`](Self::recv_message) does, but a terminal frame is
    /// [`Message::Terminal`] and a binary message [`Message::Binary`].
    pub async fn recv(&mut self) -> Result<Message, LinkError> {
        self.recv_accepting(true).await
    }

    async fn recv_accepting(&mut self, all_kinds: bool) -> Result<Message, LinkError> {
        if self.closed {
            return Err(LinkError::Closed);
        }
        let r = read_message(
            &mut self.io,
            &self.transport,
            &mut self.recv_nonce,
            all_kinds,
        )
        .await;
        if r.is_err() {
            self.close().await;
        }
        r
    }

    /// Shuts the stream down; the peer then reads end of stream.
    pub async fn shutdown(&mut self) -> Result<(), LinkError> {
        self.closed = true;
        self.io.shutdown().await?;
        Ok(())
    }

    /// Splits the stream into halves that can be used from different tasks.
    pub fn into_split(self) -> (LinkReader<ReadHalf<S>>, LinkWriter<WriteHalf<S>>) {
        let (r, w) = tokio::io::split(self.io);
        (
            LinkReader {
                io: r,
                transport: self.transport.clone(),
                nonce: self.recv_nonce,
                closed: self.closed,
            },
            LinkWriter {
                io: w,
                transport: self.transport,
                nonce: self.send_nonce,
                closed: self.closed,
            },
        )
    }

    async fn close(&mut self) {
        self.closed = true;
        let _ = self.io.shutdown().await;
    }
}

/// The receiving half of a [`LinkStream`]. After an error, drop it and the
/// writer: the reader cannot shut the stream down by itself.
pub struct LinkReader<R> {
    io: R,
    transport: Arc<StatelessTransportState>,
    nonce: u64,
    closed: bool,
}

impl<R: AsyncRead + Unpin> LinkReader<R> {
    /// As [`LinkStream::recv_message`]. Not cancel-safe.
    pub async fn recv_message(&mut self) -> Result<Vec<u8>, LinkError> {
        self.recv_accepting(false).await.map(Message::into_bytes)
    }

    /// As [`LinkStream::recv`]. Not cancel-safe.
    pub async fn recv(&mut self) -> Result<Message, LinkError> {
        self.recv_accepting(true).await
    }

    async fn recv_accepting(&mut self, all_kinds: bool) -> Result<Message, LinkError> {
        if self.closed {
            return Err(LinkError::Closed);
        }
        let r = read_message(&mut self.io, &self.transport, &mut self.nonce, all_kinds).await;
        self.closed = r.is_err();
        r
    }
}

/// The sending half of a [`LinkStream`].
pub struct LinkWriter<W> {
    io: W,
    transport: Arc<StatelessTransportState>,
    nonce: u64,
    closed: bool,
}

impl<W: AsyncWrite + Unpin> LinkWriter<W> {
    /// As [`LinkStream::send_message`].
    pub async fn send_message(&mut self, msg: &[u8]) -> Result<(), LinkError> {
        self.send_kind(Kind::Json, msg).await
    }

    /// As [`LinkStream::send_terminal`].
    pub async fn send_terminal(&mut self, msg: &[u8]) -> Result<(), LinkError> {
        self.send_kind(Kind::Terminal, msg).await
    }

    /// As [`LinkStream::send_binary`].
    pub async fn send_binary(&mut self, msg: &[u8]) -> Result<(), LinkError> {
        self.send_kind(Kind::Binary, msg).await
    }

    /// As [`LinkStream::send_kind`].
    pub async fn send_kind(&mut self, kind: Kind, msg: &[u8]) -> Result<(), LinkError> {
        if msg.len() > MAX_MESSAGE {
            return Err(LinkError::MessageTooLarge);
        }
        if self.closed {
            return Err(LinkError::Closed);
        }
        let r = write_message(&mut self.io, &self.transport, &mut self.nonce, kind, msg).await;
        if r.is_err() {
            self.closed = true;
            let _ = self.io.shutdown().await;
        }
        r
    }

    /// Shuts the stream down; the peer then reads end of stream.
    pub async fn shutdown(&mut self) -> Result<(), LinkError> {
        self.closed = true;
        self.io.shutdown().await?;
        Ok(())
    }
}

async fn write_message<W: AsyncWrite + Unpin>(
    io: &mut W,
    transport: &StatelessTransportState,
    nonce: &mut u64,
    kind: Kind,
    msg: &[u8],
) -> Result<(), LinkError> {
    let kind = kind.nibble() << 4;
    let mut plain = Vec::with_capacity(1 + MAX_CHUNK.min(msg.len()));
    let mut frame = vec![0u8; 2 + MAX_FRAME];
    let mut rest = msg;
    loop {
        let (chunk, tail) = rest.split_at(rest.len().min(MAX_CHUNK));
        rest = tail;
        plain.clear();
        plain.push(if rest.is_empty() {
            kind
        } else {
            kind | FLAG_MORE
        });
        plain.extend_from_slice(chunk);
        let n = transport.write_message(*nonce, &plain, &mut frame[2..])?;
        *nonce += 1;
        frame[..2].copy_from_slice(&(n as u16).to_be_bytes());
        io.write_all(&frame[..2 + n]).await?;
        if rest.is_empty() {
            break;
        }
    }
    io.flush().await?;
    Ok(())
}

/// Reads one message. With `all_kinds` false only kind 0 is read, so any
/// flag byte but `0x00`/`0x01` is `BadFrame`, exactly as before the kind
/// nibble (plan 20261027_07, the 14:23 EDT amendment).
async fn read_message<R: AsyncRead + Unpin>(
    io: &mut R,
    transport: &StatelessTransportState,
    nonce: &mut u64,
    all_kinds: bool,
) -> Result<Message, LinkError> {
    let mut frame = vec![0u8; MAX_FRAME];
    let mut plain = vec![0u8; MAX_FRAME];
    let mut out = Vec::new();
    let mut kind: Option<Kind> = None;
    loop {
        let len = usize::from(io.read_u16().await?);
        io.read_exact(&mut frame[..len]).await?;
        let n = transport.read_message(*nonce, &frame[..len], &mut plain)?;
        *nonce += 1;
        let (&flag, chunk) = plain[..n].split_first().ok_or(LinkError::BadFrame)?;
        if out.len() + chunk.len() > MAX_MESSAGE {
            return Err(LinkError::MessageTooLarge);
        }
        if flag & FLAG_RESERVED != 0 {
            return Err(LinkError::BadFrame);
        }
        let this = match Kind::from_nibble(flag >> 4) {
            Some(Kind::Json) => Kind::Json,
            Some(_) if !all_kinds => return Err(LinkError::BadFrame),
            Some(k) => k,
            None => return Err(LinkError::BadFrame),
        };
        if *kind.get_or_insert(this) != this {
            return Err(LinkError::BadFrame);
        }
        out.extend_from_slice(chunk);
        if flag & FLAG_MORE == 0 {
            return Ok(Message::new(this, out));
        }
    }
}
