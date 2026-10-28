//! flankr link: Noise over any Tokio byte stream
//! (docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md, decisions 1–2 and
//! the amendments of 11:58, 16:23 and 23:07 EDT).
//!
//! * Every connection, the first one included, is
//!   `Noise_IK_25519_ChaChaPoly_BLAKE2s`: the client knows the server's
//!   static key (from the pairing string), opens with the mode byte `0x02`,
//!   and may carry its first message in the handshake. The server learns
//!   the client's key from the device code pasted into `flankr pair`, and
//!   drops a key it does not know with no reply. Any other mode byte but
//!   the pairing request's `0x03` (below), `0x01` (the pairing handshake
//!   that was removed) included, closes the connection.
//! * The prologue is `b"flankr.link/1"` followed by the mode byte.
//! * Every Noise message is a frame: a 2-byte big-endian length, then the
//!   message (at most 65535 bytes). After the handshake each frame's
//!   plaintext is a flag byte (`0x00` last, `0x01` more) and a chunk of one
//!   message. A message reassembled past 1 MiB closes the connection.
//! * The flag byte is two nibbles (plan 20261027_07, the 14:23 EDT
//!   amendment). The low nibble is the flags: bit 0 is *more* (0 last,
//!   1 more); bits 1-3 are reserved, must be zero, and a frame with any of
//!   them set is [`LinkError::BadFrame`]. The high nibble is the message's
//!   kind: 0 ([`KIND_JSON`]) a JSON-RPC message, 1 ([`KIND_TERMINAL`]) a
//!   terminal frame, 2-14 reserved, 15 ([`KIND_EXTENDED`]) extended (the
//!   kind would continue in the next byte). A reserved or extended kind is
//!   `BadFrame` for now. Every frame of one message carries the same kind;
//!   a change mid-message is `BadFrame`. So `0x00`/`0x01` are kind 0, as
//!   before, and `0x10`/`0x11` a terminal frame, last/more.
//!   [`LinkStream::recv_message`] takes kind 0 only and is `BadFrame` on
//!   any other, as before this layout; [`LinkStream::recv`] takes both
//!   known kinds and returns a [`Message`]. A terminal frame is sent only
//!   to a peer that has shown it reads them (by calling `terminal.open`).
//! * [`PairingPayload`] is the pairing string, `flankr1…`, and its URL;
//!   [`DeviceCode`] is the device's code, `flankrkey1…`. Neither holds a
//!   secret.
//! * [`fingerprint`], [`remote_id`], [`fingerprint_text`] and
//!   [`fingerprint_words`] name a daemon by its static public key (plan
//!   20261028_01, decisions 1-2, the 13:03 EDT amendment's item 7 and the
//!   14:37 EDT amendment's item 3).
//! * A pairing request (plan 20261028_01, the 14:37 EDT amendment's
//!   item 1) is `Noise_NK_25519_ChaChaPoly_BLAKE2s` with the mode byte
//!   [`MODE_PAIR`] (`0x03`): [`request_pairing`] on the phone,
//!   [`accept_pair`] or [`accept_incoming`] on the daemon, and the six
//!   [`pairing_words`] both sides show. See the `pair_mode` module.
//!
//! The local static key is a [`StaticKey`], which does its own
//! Diffie-Hellman, so a client's private key can stay in a key store and
//! out of snow. [`StaticKeypair`] is one held in memory.
//!
//! The handshake functions do not time out; callers wrap them in
//! `tokio::time::timeout` as they see fit.

mod handshake;
mod identity;
mod keys;
mod pair_mode;
mod pairing;
mod resolver;
mod stream;
mod words;

#[cfg(test)]
mod tests;

pub use handshake::{Accepted, accept, reconnect};
pub use identity::{REMOTE_ID_LEN, fingerprint, fingerprint_text, remote_id};
pub use keys::{StaticKey, StaticKeypair, generate_keypair, public_key};
pub use pair_mode::{
    Incoming, MODE_PAIR, PAIR_MESSAGE_VERSION, PAIR_PATTERN, PAIR_TIMEOUT_DEFAULT_SECS,
    PAIR_TIMEOUT_MAX_SECS, PairAck, PairIncoming, PairMessageError, PairRefusal, PairRequest,
    PairResponse, PairResult, accept_incoming, accept_pair, generate_offer, recv_pair_result,
    request_pairing, send_pair_result,
};
pub use pairing::{
    DEVICE_CODE_PREFIX, DEVICE_CODE_VERSION, DEVICE_NAME_MAX, DeviceCode, OFFER_LEN,
    PAIRING_PREFIX, PAIRING_URL_SCHEME, PAIRING_VERSION, PAIRING_VERSION_NO_OFFER, PairingPayload,
    PairingStringError,
};
pub use stream::{Kind, LinkReader, LinkStream, LinkWriter, Message};
pub use words::{
    PAIRING_WORDS_CONTEXT, WORD_COUNT, fingerprint_words, fingerprint_words_text, pairing_words,
    pairing_words_text,
};

use std::fmt;

/// An X25519 public or private key.
pub const KEY_LEN: usize = 32;

/// The handshake.
pub const RECONNECT_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
/// The prologue, before the mode byte.
pub const PROLOGUE_PREFIX: &[u8] = b"flankr.link/1";

/// The client's first byte: `IK`. The pairing request's is
/// [`MODE_PAIR`].
pub const MODE_RECONNECT: u8 = 0x02;

/// The largest Noise message, and so the largest frame body.
pub const MAX_FRAME: usize = 65535;
/// The largest chunk of a message in one frame: the frame less the AEAD tag
/// and the flag byte.
pub const MAX_CHUNK: usize = MAX_FRAME - TAG_LEN - 1;
/// The largest message, reassembled.
pub const MAX_MESSAGE: usize = 1 << 20;
/// The largest first message carried inside a handshake: the frame less the
/// largest handshake overhead (`IK`'s first message: ephemeral key,
/// encrypted static key and the payload's tag).
pub const MAX_FIRST_MESSAGE: usize = MAX_FRAME - (KEY_LEN + KEY_LEN + TAG_LEN + TAG_LEN);

/// The kind nibble of a JSON-RPC message: the high nibble of the flag
/// byte.
pub const KIND_JSON: u8 = 0x0;
/// The kind nibble of a terminal frame (plan 20261027_07, decision 3).
pub const KIND_TERMINAL: u8 = 0x1;
/// The kind nibble that says the kind continues in the next byte.
/// Reserved: a frame with it is [`LinkError::BadFrame`] today.
pub const KIND_EXTENDED: u8 = 0xF;

const TAG_LEN: usize = 16;
/// The flag nibble's *more* bit.
const FLAG_MORE: u8 = 0x01;
/// The flag nibble's reserved bits, which must be zero.
const FLAG_RESERVED: u8 = 0x0E;

/// Why a link operation failed. After any error but
/// [`LinkError::MessageTooLarge`] from a send, the connection is finished
/// and should be dropped.
#[derive(Debug)]
#[non_exhaustive]
pub enum LinkError {
    /// The underlying stream failed.
    Io(std::io::Error),
    /// The peer closed the connection, or the stream was already closed by
    /// an earlier error.
    Closed,
    /// The Noise handshake or a decryption failed (for example, a client
    /// that has the wrong server key).
    Noise(NoiseError),
    /// The client's first byte was not a known mode.
    UnknownMode(u8),
    /// The client's static key was not authorized. The
    /// server sent nothing back.
    Unauthorized,
    /// A message is larger than [`MAX_MESSAGE`] (sending), or was
    /// reassembled past it (receiving, which closes the connection).
    MessageTooLarge,
    /// A first message longer than [`MAX_FIRST_MESSAGE`].
    FirstMessageTooLarge,
    /// A frame's plaintext was empty, or its flag byte was not one the
    /// receive accepts: a reserved flag bit set, a reserved or extended
    /// kind, a kind other than JSON-RPC to [`LinkStream::recv_message`],
    /// or a kind that changed mid-message. For `recv_message` that is any
    /// flag byte but `0x00`/`0x01`, as before.
    BadFrame,
    /// A pairing request's message could not be read or written.
    Pairing(PairMessageError),
}

/// A Noise failure, kept opaque so `snow`'s types are not part of this
/// crate's API.
#[derive(Debug)]
pub struct NoiseError(snow::Error);

impl fmt::Display for NoiseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for NoiseError {}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::Io(e) => write!(f, "link i/o: {e}"),
            LinkError::Closed => f.write_str("link closed"),
            LinkError::Noise(e) => write!(f, "noise: {e}"),
            LinkError::UnknownMode(m) => write!(f, "unknown link mode byte 0x{m:02x}"),
            LinkError::Unauthorized => f.write_str("client key not authorized"),
            LinkError::MessageTooLarge => {
                write!(f, "message larger than {MAX_MESSAGE} bytes")
            }
            LinkError::FirstMessageTooLarge => {
                write!(f, "first message larger than {MAX_FIRST_MESSAGE} bytes")
            }
            LinkError::BadFrame => f.write_str("malformed link frame"),
            LinkError::Pairing(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for LinkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LinkError::Io(e) => Some(e),
            LinkError::Noise(e) => Some(e),
            LinkError::Pairing(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for LinkError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            LinkError::Closed
        } else {
            LinkError::Io(e)
        }
    }
}

impl From<PairMessageError> for LinkError {
    fn from(e: PairMessageError) -> Self {
        LinkError::Pairing(e)
    }
}

impl From<snow::Error> for LinkError {
    fn from(e: snow::Error) -> Self {
        LinkError::Noise(NoiseError(e))
    }
}
