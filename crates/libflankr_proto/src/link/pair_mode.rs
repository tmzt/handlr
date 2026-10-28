//! The pairing request: a phone asks a daemon in pair mode to add its key
//! (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md, the 14:37 EDT
//! amendment's item 1, the 14:40 EDT amendment and the 14:44 EDT amendment).
//!
//! The phone has scanned a `flankr pair` run's link: it knows the daemon's
//! static key and the run's offer. It makes a new static key `D` for this
//! daemon and connects with **`Noise_NK_25519_ChaChaPoly_BLAKE2s`**: the
//! daemon's key is pinned from the link, so only that daemon can read the
//! request; the phone has no static key in this handshake.
//!
//! On the wire, the client opens with the mode byte [`MODE_PAIR`] (`0x03`);
//! the prologue is `b"flankr.link/1"` followed by it. Every Noise message
//! is a frame (a 2-byte big-endian length, then the message), as in `IK`.
//!
//! 1. `-> e, es` carries a [`PairRequest`]: `D`'s public key, the device's
//!    name and the offer.
//! 2. `<- e, ee` carries a [`PairAck`]: accepted (with the seconds left on
//!    the offer), or refused with a [`PairRefusal`] (a wrong offer, an
//!    expired one, a request already held, a malformed request).
//! 3. After an accepted ack both sides show the six
//!    [`pairing_words`](super::pairing_words) of the daemon's key, the offer
//!    and `D`. The connection stays open as a [`LinkStream`]; the daemon
//!    sends one [`PairResult`] when the owner has answered: added, refused
//!    or expired. The phone cancels by closing the connection.
//!
//! The NK handshake does not prove that the phone holds `D`'s private key;
//! the owner's check of the words binds `D` to this request, and the
//! phone's first `IK` connection afterwards proves the key.
//!
//! Each message is versioned by its first byte, [`PAIR_MESSAGE_VERSION`]:
//!
//! * request: version, `D` (32), name (1-byte length, then the name, as
//!   [`DeviceCode::valid_name`] allows), offer (16);
//! * ack: version, `0x00` accepted and the seconds left (2 bytes,
//!   big-endian), or `0x01` refused and the reason (1 byte:
//!   [`PairRefusal::code`]);
//! * result: version, and `0x00` added, `0x01` refused or `0x02` expired.
//!
//! The result travels as one message on the stream (the kind nibble is
//! JSON-RPC's, `0`, though the bytes are this encoding: a pairing session
//! carries nothing else).

use snow::Builder;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::handshake::{Accepted, accept_after_mode, builder, prologue, read_frame, write_frame};
use super::{
    DeviceCode, KEY_LEN, LinkError, LinkStream, MAX_FRAME, MODE_RECONNECT, OFFER_LEN, StaticKey,
};

/// The pairing request's handshake.
pub const PAIR_PATTERN: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";
/// The client's first byte for a pairing request: `NK`.
pub const MODE_PAIR: u8 = 0x03;
/// The first byte of every pairing message.
pub const PAIR_MESSAGE_VERSION: u8 = 1;
/// How long a `flankr pair` run's offer lasts by default, in seconds.
pub const PAIR_TIMEOUT_DEFAULT_SECS: u16 = 60;
/// The longest a `flankr pair` run's offer may last, in seconds.
pub const PAIR_TIMEOUT_MAX_SECS: u16 = 300;

const ACK_ACCEPTED: u8 = 0x00;
const ACK_REFUSED: u8 = 0x01;

/// What the phone asks the daemon in the first handshake message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRequest {
    /// The device's new static X25519 public key, `D`.
    pub device_key: [u8; KEY_LEN],
    /// The device's name, as [`DeviceCode::valid_name`] allows.
    pub name: String,
    /// The offer from the link the phone was given.
    pub offer: [u8; OFFER_LEN],
}

/// The daemon's answer to a [`PairRequest`], in the second handshake
/// message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairAck {
    /// The request is held; both sides now show the pairing words. The
    /// offer has this many seconds left; the owner's **yes** must come
    /// within them.
    Accepted { expires_in_secs: u16 },
    /// The request was refused at once and does not count as the run's
    /// attempt; the daemon closes the connection.
    Refused(PairRefusal),
}

/// Why the daemon refused a [`PairRequest`] at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairRefusal {
    /// The offer is not the current `flankr pair` run's (or no run is in
    /// pair mode).
    WrongOffer,
    /// The run's offer has expired.
    Expired,
    /// The run already holds a request.
    Busy,
    /// The request could not be read (a bad name, a short message).
    Malformed,
}

impl PairRefusal {
    /// The reason's byte in an ack.
    pub fn code(self) -> u8 {
        match self {
            PairRefusal::WrongOffer => 1,
            PairRefusal::Expired => 2,
            PairRefusal::Busy => 3,
            PairRefusal::Malformed => 4,
        }
    }

    /// The reason a byte names.
    pub fn from_code(code: u8) -> Option<PairRefusal> {
        match code {
            1 => Some(PairRefusal::WrongOffer),
            2 => Some(PairRefusal::Expired),
            3 => Some(PairRefusal::Busy),
            4 => Some(PairRefusal::Malformed),
            _ => None,
        }
    }
}

/// How an accepted request ended, sent by the daemon once over the open
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairResult {
    /// The owner typed **yes**: `D` is in `auth.toml`. The phone stores the
    /// remote and connects with `IK`.
    Added,
    /// The owner answered anything but **yes**: nothing was added.
    Refused,
    /// The offer expired before **yes**: nothing was added.
    Expired,
}

impl PairResult {
    /// The result's byte.
    pub fn code(self) -> u8 {
        match self {
            PairResult::Added => 0,
            PairResult::Refused => 1,
            PairResult::Expired => 2,
        }
    }

    /// The result a byte names.
    pub fn from_code(code: u8) -> Option<PairResult> {
        match code {
            0 => Some(PairResult::Added),
            1 => Some(PairResult::Refused),
            2 => Some(PairResult::Expired),
            _ => None,
        }
    }

    /// The message's bytes.
    pub fn to_bytes(self) -> Vec<u8> {
        vec![PAIR_MESSAGE_VERSION, self.code()]
    }

    /// Reads a result message.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PairMessageError> {
        match bytes {
            [PAIR_MESSAGE_VERSION, code] => {
                PairResult::from_code(*code).ok_or(PairMessageError::UnknownCode(*code))
            }
            [v, ..] if *v != PAIR_MESSAGE_VERSION => Err(PairMessageError::UnknownVersion(*v)),
            _ => Err(PairMessageError::Malformed),
        }
    }
}

/// Why a pairing message could not be read or written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairMessageError {
    /// Truncated, or with trailing bytes.
    Malformed,
    /// A first byte other than [`PAIR_MESSAGE_VERSION`].
    UnknownVersion(u8),
    /// An ack, refusal reason or result byte this crate does not know.
    UnknownCode(u8),
    /// A device name that [`DeviceCode::valid_name`] refuses.
    BadName,
}

impl std::fmt::Display for PairMessageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairMessageError::Malformed => f.write_str("malformed pairing message"),
            PairMessageError::UnknownVersion(v) => {
                write!(f, "pairing message of unknown version {v}")
            }
            PairMessageError::UnknownCode(c) => write!(f, "pairing message with unknown code {c}"),
            PairMessageError::BadName => {
                f.write_str("the device name is not 1 to 32 ASCII letters, digits, '-', '_' or '.'")
            }
        }
    }
}

impl std::error::Error for PairMessageError {}

impl PairRequest {
    /// The message's bytes. A name [`DeviceCode::valid_name`] refuses is
    /// [`PairMessageError::BadName`].
    pub fn to_bytes(&self) -> Result<Vec<u8>, PairMessageError> {
        if !DeviceCode::valid_name(&self.name) {
            return Err(PairMessageError::BadName);
        }
        let mut out = Vec::with_capacity(1 + KEY_LEN + 1 + self.name.len() + OFFER_LEN);
        out.push(PAIR_MESSAGE_VERSION);
        out.extend_from_slice(&self.device_key);
        out.push(self.name.len() as u8);
        out.extend_from_slice(self.name.as_bytes());
        out.extend_from_slice(&self.offer);
        Ok(out)
    }

    /// Reads a request message.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PairMessageError> {
        let (&version, rest) = bytes.split_first().ok_or(PairMessageError::Malformed)?;
        if version != PAIR_MESSAGE_VERSION {
            return Err(PairMessageError::UnknownVersion(version));
        }
        let (device_key, rest) = split_array::<KEY_LEN>(rest)?;
        let (&len, rest) = rest.split_first().ok_or(PairMessageError::Malformed)?;
        let len = usize::from(len);
        if rest.len() != len + OFFER_LEN {
            return Err(PairMessageError::Malformed);
        }
        let (name, offer) = rest.split_at(len);
        let name = std::str::from_utf8(name).map_err(|_| PairMessageError::BadName)?;
        if !DeviceCode::valid_name(name) {
            return Err(PairMessageError::BadName);
        }
        Ok(PairRequest {
            device_key,
            name: name.to_string(),
            offer: split_array::<OFFER_LEN>(offer)?.0,
        })
    }
}

impl PairAck {
    /// The message's bytes.
    pub fn to_bytes(self) -> Vec<u8> {
        match self {
            PairAck::Accepted { expires_in_secs } => {
                let [hi, lo] = expires_in_secs.to_be_bytes();
                vec![PAIR_MESSAGE_VERSION, ACK_ACCEPTED, hi, lo]
            }
            PairAck::Refused(reason) => vec![PAIR_MESSAGE_VERSION, ACK_REFUSED, reason.code()],
        }
    }

    /// Reads an ack message.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PairMessageError> {
        match bytes {
            [PAIR_MESSAGE_VERSION, ACK_ACCEPTED, hi, lo] => Ok(PairAck::Accepted {
                expires_in_secs: u16::from_be_bytes([*hi, *lo]),
            }),
            [PAIR_MESSAGE_VERSION, ACK_REFUSED, code] => PairRefusal::from_code(*code)
                .map(PairAck::Refused)
                .ok_or(PairMessageError::UnknownCode(*code)),
            [PAIR_MESSAGE_VERSION, ACK_ACCEPTED | ACK_REFUSED, ..] => {
                Err(PairMessageError::Malformed)
            }
            [PAIR_MESSAGE_VERSION, code, ..] => Err(PairMessageError::UnknownCode(*code)),
            [v, ..] if *v != PAIR_MESSAGE_VERSION => Err(PairMessageError::UnknownVersion(*v)),
            _ => Err(PairMessageError::Malformed),
        }
    }
}

fn split_array<const N: usize>(bytes: &[u8]) -> Result<([u8; N], &[u8]), PairMessageError> {
    if bytes.len() < N {
        return Err(PairMessageError::Malformed);
    }
    let (head, tail) = bytes.split_at(N);
    Ok((
        head.try_into().map_err(|_| PairMessageError::Malformed)?,
        tail,
    ))
}

/// A new random offer for a `flankr pair` run, from snow's random source
/// (the operating system's).
pub fn generate_offer() -> Result<[u8; OFFER_LEN], LinkError> {
    use snow::resolvers::{CryptoResolver, DefaultResolver};
    let mut rng = DefaultResolver
        .resolve_rng()
        .ok_or(snow::Error::Init(snow::error::InitStage::GetRngImpl))?;
    let mut offer = [0u8; OFFER_LEN];
    rng.try_fill_bytes(&mut offer)?;
    Ok(offer)
}

/// What the phone has after [`request_pairing`]: the daemon's ack, and the
/// session on which an accepted request's [`PairResult`] will come.
pub struct PairResponse<S> {
    /// The daemon's answer.
    pub ack: PairAck,
    /// The session. Its [`peer_key`](LinkStream::peer_key) is the daemon's
    /// key, which NK authenticated. After a refusal, drop it.
    pub stream: LinkStream<S>,
}

/// Phone: sends a pairing request to the daemon whose static key is
/// `server_key` (from the link), over `NK`, and reads its ack.
///
/// A daemon with another key cannot read the request, and its reply fails
/// to decrypt as [`LinkError::Noise`]; a daemon that is not in pair mode
/// closes the connection with no reply ([`LinkError::Closed`]). A message
/// that does not read is [`LinkError::Pairing`].
pub async fn request_pairing<S: AsyncRead + AsyncWrite + Unpin>(
    mut io: S,
    server_key: &[u8; KEY_LEN],
    request: &PairRequest,
) -> Result<PairResponse<S>, LinkError> {
    let body = request.to_bytes()?;
    let prologue = prologue(MODE_PAIR);
    let mut hs = Builder::new(PAIR_PATTERN.parse()?)
        .remote_public_key(server_key)?
        .prologue(&prologue)?
        .build_initiator()?;
    let mut buf = vec![0u8; MAX_FRAME];

    // -> mode, e, es (+ request)
    let n = hs.write_message(&body, &mut buf)?;
    let mut head = vec![MODE_PAIR];
    head.extend_from_slice(&(n as u16).to_be_bytes());
    head.extend_from_slice(&buf[..n]);
    io.write_all(&head).await?;
    io.flush().await?;

    // <- e, ee (+ ack)
    let msg = read_frame(&mut io).await?;
    let n = hs.read_message(&msg, &mut buf)?;
    let ack = PairAck::from_bytes(&buf[..n])?;

    let transport = hs.into_stateless_transport_mode()?;
    Ok(PairResponse {
        ack,
        stream: LinkStream::new(io, transport, *server_key),
    })
}

/// Phone: waits for the daemon's [`PairResult`] on an accepted session.
pub async fn recv_pair_result<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut LinkStream<S>,
) -> Result<PairResult, LinkError> {
    let msg = stream.recv_message().await?;
    Ok(PairResult::from_bytes(&msg)?)
}

/// Daemon: tells the phone how its accepted request ended.
pub async fn send_pair_result<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut LinkStream<S>,
    result: PairResult,
) -> Result<(), LinkError> {
    stream.send_message(&result.to_bytes()).await
}

/// A pairing request the daemon has read but not yet answered. Answer it
/// with [`respond`](Self::respond), or drop it to close the connection with
/// no reply (as when no `flankr pair` run is in pair mode).
pub struct PairIncoming<S> {
    io: S,
    hs: snow::HandshakeState,
    /// The request, or why it could not be read: answer that with
    /// [`PairRefusal::Malformed`].
    pub request: Result<PairRequest, PairMessageError>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> PairIncoming<S> {
    /// Sends `ack` in the second handshake message and returns the
    /// session. After [`PairAck::Accepted`], keep it to
    /// [`send_pair_result`] and to notice the phone closing (its Cancel);
    /// after a refusal, drop it.
    ///
    /// The session's [`peer_key`](LinkStream::peer_key) is all zeros: NK
    /// does not authenticate the phone. Its claimed key is the request's
    /// `device_key`.
    pub async fn respond(mut self, ack: PairAck) -> Result<LinkStream<S>, LinkError> {
        let mut buf = vec![0u8; MAX_FRAME];
        let n = self.hs.write_message(&ack.to_bytes(), &mut buf)?;
        write_frame(&mut self.io, &buf[..n]).await?;
        let transport = self.hs.into_stateless_transport_mode()?;
        Ok(LinkStream::new(self.io, transport, [0u8; KEY_LEN]))
    }
}

/// Daemon: accepts one pairing request. Reads the mode byte, which must be
/// [`MODE_PAIR`] (any other is [`LinkError::UnknownMode`]), then the NK
/// first message with the request.
pub async fn accept_pair<S, K>(mut io: S, local: &K) -> Result<PairIncoming<S>, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    K: StaticKey + Clone,
{
    let mode = io.read_u8().await?;
    if mode != MODE_PAIR {
        return Err(LinkError::UnknownMode(mode));
    }
    accept_pair_after_mode(io, local).await
}

async fn accept_pair_after_mode<S, K>(mut io: S, local: &K) -> Result<PairIncoming<S>, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    K: StaticKey + Clone,
{
    let prologue = prologue(MODE_PAIR);
    let mut hs = builder(PAIR_PATTERN, local)?
        .prologue(&prologue)?
        .build_responder()?;
    let mut buf = vec![0u8; MAX_FRAME];
    // -> e, es (+ request)
    let msg = read_frame(&mut io).await?;
    let n = hs.read_message(&msg, &mut buf)?;
    Ok(PairIncoming {
        io,
        hs,
        request: PairRequest::from_bytes(&buf[..n]),
    })
}

/// A connection the daemon accepted on a listener that takes both kinds.
pub enum Incoming<S> {
    /// An `IK` connection from an authorized device, as [`accept`](super::accept) returns.
    Link(Accepted<S>),
    /// A pairing request, as [`accept_pair`] returns.
    Pair(Box<PairIncoming<S>>),
}

/// Daemon: accepts one connection of either kind, by its mode byte:
/// [`MODE_RECONNECT`] as [`accept`](super::accept) (with `authorize`),
/// [`MODE_PAIR`] as [`accept_pair`]. Any other byte is
/// [`LinkError::UnknownMode`]. A daemon not in pair mode drops a
/// [`Incoming::Pair`] unanswered.
pub async fn accept_incoming<S, K, A>(
    mut io: S,
    local: &K,
    authorize: A,
) -> Result<Incoming<S>, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    K: StaticKey + Clone,
    A: FnOnce(&[u8; KEY_LEN]) -> bool,
{
    match io.read_u8().await? {
        MODE_RECONNECT => Ok(Incoming::Link(
            accept_after_mode(io, local, authorize).await?,
        )),
        MODE_PAIR => Ok(Incoming::Pair(Box::new(
            accept_pair_after_mode(io, local).await?,
        ))),
        mode => Err(LinkError::UnknownMode(mode)),
    }
}
