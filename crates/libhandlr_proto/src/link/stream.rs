//! The established link: framing, chunking and the 1 MiB limit.

use std::sync::Arc;

use snow::StatelessTransportState;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

use super::{FLAG_LAST, FLAG_MORE, KEY_LEN, LinkError, MAX_CHUNK, MAX_FRAME, MAX_MESSAGE};

/// An established link connection. Messages are whole: one
/// [`send_message`](Self::send_message) is one
/// [`recv_message`](Self::recv_message) on the other side.
///
/// `recv_message` is not cancel-safe: a message half read when its future
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
        if msg.len() > MAX_MESSAGE {
            return Err(LinkError::MessageTooLarge);
        }
        self.send_unchecked(msg).await
    }

    pub(super) async fn send_unchecked(&mut self, msg: &[u8]) -> Result<(), LinkError> {
        if self.closed {
            return Err(LinkError::Closed);
        }
        let r = write_message(&mut self.io, &self.transport, &mut self.send_nonce, msg).await;
        if r.is_err() {
            self.close().await;
        }
        r
    }

    /// Receives one whole message. On any error, including a message
    /// reassembled past [`MAX_MESSAGE`], the connection is closed.
    pub async fn recv_message(&mut self) -> Result<Vec<u8>, LinkError> {
        if self.closed {
            return Err(LinkError::Closed);
        }
        let r = read_message(&mut self.io, &self.transport, &mut self.recv_nonce).await;
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
        if self.closed {
            return Err(LinkError::Closed);
        }
        let r = read_message(&mut self.io, &self.transport, &mut self.nonce).await;
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
        if msg.len() > MAX_MESSAGE {
            return Err(LinkError::MessageTooLarge);
        }
        if self.closed {
            return Err(LinkError::Closed);
        }
        let r = write_message(&mut self.io, &self.transport, &mut self.nonce, msg).await;
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
    msg: &[u8],
) -> Result<(), LinkError> {
    let mut plain = Vec::with_capacity(1 + MAX_CHUNK.min(msg.len()));
    let mut frame = vec![0u8; 2 + MAX_FRAME];
    let mut rest = msg;
    loop {
        let (chunk, tail) = rest.split_at(rest.len().min(MAX_CHUNK));
        rest = tail;
        plain.clear();
        plain.push(if rest.is_empty() {
            FLAG_LAST
        } else {
            FLAG_MORE
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

async fn read_message<R: AsyncRead + Unpin>(
    io: &mut R,
    transport: &StatelessTransportState,
    nonce: &mut u64,
) -> Result<Vec<u8>, LinkError> {
    let mut frame = vec![0u8; MAX_FRAME];
    let mut plain = vec![0u8; MAX_FRAME];
    let mut out = Vec::new();
    loop {
        let len = usize::from(io.read_u16().await?);
        io.read_exact(&mut frame[..len]).await?;
        let n = transport.read_message(*nonce, &frame[..len], &mut plain)?;
        *nonce += 1;
        let (&flag, chunk) = plain[..n].split_first().ok_or(LinkError::BadFrame)?;
        if out.len() + chunk.len() > MAX_MESSAGE {
            return Err(LinkError::MessageTooLarge);
        }
        out.extend_from_slice(chunk);
        match flag {
            FLAG_LAST => return Ok(out),
            FLAG_MORE => continue,
            _ => return Err(LinkError::BadFrame),
        }
    }
}
