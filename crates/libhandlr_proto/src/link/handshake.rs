//! The two handshakes, client and server.

use snow::{Builder, HandshakeState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::keys::to_key;
use super::{
    LinkError, LinkStream, PairingId, KEY_LEN, MAX_FIRST_MESSAGE, MAX_FRAME, MODE_PAIRING,
    MODE_RECONNECT, PAIRING_PATTERN, PROLOGUE_PREFIX, RECONNECT_PATTERN,
};

/// The PSK's position in `XXpsk3`.
const PSK_LOCATION: u8 = 3;

/// A connection the server accepted.
pub struct Accepted<S> {
    pub stream: LinkStream<S>,
    /// The client's static X25519 public key.
    pub peer_key: [u8; KEY_LEN],
    pub mode: AcceptedMode,
    /// The message the client carried in its last handshake message, if
    /// any (`hello`, when reconnecting).
    pub first_message: Option<Vec<u8>>,
}

/// How the client connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptedMode {
    /// Paired with the pending pairing of this id. The caller records the
    /// client's key and burns the secret.
    Pairing(PairingId),
    /// Reconnected with a key the caller authorized.
    Reconnect,
}

/// Client: pairs with a server, over `XXpsk3` with `secret` as the PSK.
///
/// Sends the mode byte `0x01` and the [`PairingId`] of `secret`, then runs
/// the handshake. If the server's static key is not `server_key`, the
/// client aborts with [`LinkError::ServerKeyMismatch`] before sending its
/// own static key. `first_message`, if any, travels in the last handshake
/// message.
///
/// A wrong secret is detected by the server, which closes the connection;
/// the client sees that as an error on its first receive.
///
/// Returns the stream and the server's static key.
pub async fn pair<S: AsyncRead + AsyncWrite + Unpin>(
    mut io: S,
    local_private: &[u8; KEY_LEN],
    server_key: &[u8; KEY_LEN],
    secret: &[u8; KEY_LEN],
    first_message: Option<&[u8]>,
) -> Result<(LinkStream<S>, [u8; KEY_LEN]), LinkError> {
    let first = check_first(first_message)?;
    let prologue = prologue(MODE_PAIRING);
    let mut hs = Builder::new(PAIRING_PATTERN.parse()?)
        .local_private_key(local_private)?
        .psk(PSK_LOCATION, secret)?
        .prologue(&prologue)?
        .build_initiator()?;
    let mut buf = vec![0u8; MAX_FRAME];

    // -> mode, pairing id, e
    let n = hs.write_message(&[], &mut buf)?;
    let mut head = vec![MODE_PAIRING];
    head.extend_from_slice(&PairingId::from_secret(secret).0);
    head.extend_from_slice(&(n as u16).to_be_bytes());
    head.extend_from_slice(&buf[..n]);
    io.write_all(&head).await?;
    io.flush().await?;

    // <- e, ee, s, es
    let msg = read_frame(&mut io).await?;
    hs.read_message(&msg, &mut buf)?;
    let remote = remote_static(&hs)?;
    if &remote != server_key {
        return Err(LinkError::ServerKeyMismatch);
    }

    // -> s, se, psk
    let n = hs.write_message(first, &mut buf)?;
    write_frame(&mut io, &buf[..n]).await?;

    let transport = hs.into_stateless_transport_mode()?;
    Ok((LinkStream::new(io, transport, remote), remote))
}

/// Client: reconnects to a server whose static key it knows, over `IK`.
///
/// Sends the mode byte `0x02` and the first handshake message, carrying
/// `first_message` (encrypted) if given. A server that does not authorize
/// the client's key sends nothing and closes the connection, which this
/// returns as [`LinkError::Closed`].
pub async fn reconnect<S: AsyncRead + AsyncWrite + Unpin>(
    mut io: S,
    local_private: &[u8; KEY_LEN],
    server_key: &[u8; KEY_LEN],
    first_message: Option<&[u8]>,
) -> Result<LinkStream<S>, LinkError> {
    let first = check_first(first_message)?;
    let prologue = prologue(MODE_RECONNECT);
    let mut hs = Builder::new(RECONNECT_PATTERN.parse()?)
        .local_private_key(local_private)?
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
/// Reads the mode byte. For pairing (`0x01`), reads the [`PairingId`] and
/// asks `pairing_secret` for the pending secret it names; with none, or
/// with a handshake that fails (a wrong secret), the connection is
/// dropped. For reconnecting (`0x02`), reads the client's first message,
/// learns its static key and asks `authorize`; an unauthorized key gets no
/// reply, and the connection is dropped. Any other mode byte closes the
/// connection.
///
/// On error, `io` is dropped, which closes the connection.
pub async fn accept<S, P, A>(
    mut io: S,
    local_private: &[u8; KEY_LEN],
    pairing_secret: P,
    authorize: A,
) -> Result<Accepted<S>, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: FnOnce(PairingId) -> Option<[u8; KEY_LEN]>,
    A: FnOnce(&[u8; KEY_LEN]) -> bool,
{
    let mode = io.read_u8().await?;
    let mut buf = vec![0u8; MAX_FRAME];
    match mode {
        MODE_PAIRING => {
            let mut id = [0u8; 4];
            io.read_exact(&mut id).await?;
            let id = PairingId(id);
            let secret = pairing_secret(id).ok_or(LinkError::UnknownPairing)?;
            let prologue = prologue(MODE_PAIRING);
            let mut hs = Builder::new(PAIRING_PATTERN.parse()?)
                .local_private_key(local_private)?
                .psk(PSK_LOCATION, &secret)?
                .prologue(&prologue)?
                .build_responder()?;

            // -> e
            let msg = read_frame(&mut io).await?;
            hs.read_message(&msg, &mut buf)?;
            // <- e, ee, s, es
            let n = hs.write_message(&[], &mut buf)?;
            write_frame(&mut io, &buf[..n]).await?;
            // -> s, se, psk (+ first message)
            let msg = read_frame(&mut io).await?;
            let n = hs.read_message(&msg, &mut buf)?;
            let first = (n > 0).then(|| buf[..n].to_vec());

            let peer_key = remote_static(&hs)?;
            let transport = hs.into_stateless_transport_mode()?;
            Ok(Accepted {
                stream: LinkStream::new(io, transport, peer_key),
                peer_key,
                mode: AcceptedMode::Pairing(id),
                first_message: first,
            })
        }
        MODE_RECONNECT => {
            let prologue = prologue(MODE_RECONNECT);
            let mut hs = Builder::new(RECONNECT_PATTERN.parse()?)
                .local_private_key(local_private)?
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
                mode: AcceptedMode::Reconnect,
                first_message: first,
            })
        }
        other => Err(LinkError::UnknownMode(other)),
    }
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
