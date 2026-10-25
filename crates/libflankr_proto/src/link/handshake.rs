//! The handshake, `IK`, client and server (plan 02, amendment of 23:07
//! EDT: one handshake).

use snow::{Builder, HandshakeState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::keys::to_key;
use super::resolver::{KeyResolver, STATIC_MARKER};
use super::{
    KEY_LEN, LinkError, LinkStream, MAX_FIRST_MESSAGE, MAX_FRAME, MODE_RECONNECT, PROLOGUE_PREFIX,
    RECONNECT_PATTERN, StaticKey,
};

/// A connection the server accepted.
pub struct Accepted<S> {
    pub stream: LinkStream<S>,
    /// The client's static X25519 public key, which `authorize` allowed.
    pub peer_key: [u8; KEY_LEN],
    /// The message the client carried in its first handshake message, if
    /// any (`hello`).
    pub first_message: Option<Vec<u8>>,
}

/// Client: connects to a server whose static key it knows, over `IK`.
/// The first connection after pairing is one of these too: the server's
/// key comes from the pairing string.
///
/// Sends the mode byte `0x02` and the first handshake message, carrying
/// `first_message` (encrypted) if given. A server that does not authorize
/// the client's key sends nothing and closes the connection, which this
/// returns as [`LinkError::Closed`].
pub async fn reconnect<S: AsyncRead + AsyncWrite + Unpin, K: StaticKey + Clone>(
    mut io: S,
    local: &K,
    server_key: &[u8; KEY_LEN],
    first_message: Option<&[u8]>,
) -> Result<LinkStream<S>, LinkError> {
    let first = check_first(first_message)?;
    let prologue = prologue(MODE_RECONNECT);
    let mut hs = builder(RECONNECT_PATTERN, local)?
        .remote_public_key(server_key)?
        .prologue(&prologue)?
        .build_initiator()?;
    let mut buf = vec![0u8; MAX_FRAME];

    // -> mode, e, es, s, ss (+ first message)
    let n = hs.write_message(first, &mut buf)?;
    let mut head = vec![MODE_RECONNECT];
    head.extend_from_slice(&(n as u16).to_be_bytes());
    head.extend_from_slice(&buf[..n]);
    io.write_all(&head).await?;
    io.flush().await?;

    // <- e, ee, se
    let msg = read_frame(&mut io).await?;
    hs.read_message(&msg, &mut buf)?;

    let transport = hs.into_stateless_transport_mode()?;
    Ok(LinkStream::new(io, transport, *server_key))
}

/// Server: accepts one connection.
///
/// Reads the mode byte, which must be `0x02` (`IK`); any other byte
/// (`0x01`, the pairing handshake that was removed, included) is
/// [`LinkError::UnknownMode`] and closes the connection with no reply.
/// Then reads the client's first message, learns its static key and asks
/// `authorize`; an unauthorized key gets no reply, and the connection is
/// dropped.
///
/// On error, `io` is dropped, which closes the connection.
pub async fn accept<S, K, A>(mut io: S, local: &K, authorize: A) -> Result<Accepted<S>, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    K: StaticKey + Clone,
    A: FnOnce(&[u8; KEY_LEN]) -> bool,
{
    let mode = io.read_u8().await?;
    if mode != MODE_RECONNECT {
        return Err(LinkError::UnknownMode(mode));
    }
    let mut buf = vec![0u8; MAX_FRAME];
    let prologue = prologue(MODE_RECONNECT);
    let mut hs = builder(RECONNECT_PATTERN, local)?
        .prologue(&prologue)?
        .build_responder()?;

    // -> e, es, s, ss (+ first message)
    let msg = read_frame(&mut io).await?;
    let n = hs.read_message(&msg, &mut buf)?;
    let first = (n > 0).then(|| buf[..n].to_vec());
    let peer_key = remote_static(&hs)?;
    if !authorize(&peer_key) {
        return Err(LinkError::Unauthorized);
    }
    // <- e, ee, se
    let n = hs.write_message(&[], &mut buf)?;
    write_frame(&mut io, &buf[..n]).await?;

    let transport = hs.into_stateless_transport_mode()?;
    Ok(Accepted {
        stream: LinkStream::new(io, transport, peer_key),
        peer_key,
        first_message: first,
    })
}

/// A builder for `pattern` whose local static key is `local`, through
/// [`KeyResolver`]: snow is given a marker, not the private key.
fn builder<'a, K: StaticKey + Clone>(pattern: &str, local: &K) -> Result<Builder<'a>, LinkError> {
    let resolver = Box::new(KeyResolver::new(local));
    Ok(Builder::with_resolver(pattern.parse()?, resolver).local_private_key(STATIC_MARKER)?)
}

fn prologue(mode: u8) -> Vec<u8> {
    let mut p = PROLOGUE_PREFIX.to_vec();
    p.push(mode);
    p
}

fn check_first(first: Option<&[u8]>) -> Result<&[u8], LinkError> {
    let first = first.unwrap_or(&[]);
    if first.len() > MAX_FIRST_MESSAGE {
        return Err(LinkError::FirstMessageTooLarge);
    }
    Ok(first)
}

fn remote_static(hs: &HandshakeState) -> Result<[u8; KEY_LEN], LinkError> {
    to_key(hs.get_remote_static().ok_or(snow::Error::State(
        snow::error::StateProblem::MissingKeyMaterial,
    ))?)
}

async fn read_frame<R: AsyncRead + Unpin>(io: &mut R) -> Result<Vec<u8>, LinkError> {
    let len = usize::from(io.read_u16().await?);
    let mut msg = vec![0u8; len];
    io.read_exact(&mut msg).await?;
    Ok(msg)
}

async fn write_frame<W: AsyncWrite + Unpin>(io: &mut W, msg: &[u8]) -> Result<(), LinkError> {
    let mut frame = Vec::with_capacity(2 + msg.len());
    frame.extend_from_slice(&(msg.len() as u16).to_be_bytes());
    frame.extend_from_slice(msg);
    io.write_all(&frame).await?;
    io.flush().await?;
    Ok(())
}
