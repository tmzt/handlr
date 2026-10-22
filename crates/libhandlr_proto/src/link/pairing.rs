//! The pairing string (plan 02, amendment of 11:58 EDT).
//!
//! The payload, as bytes: version (1), server public key (32), secret (32),
//! expiry (4, unix seconds, big-endian), name (1-byte length, UTF-8),
//! addresses (1-byte count, then each a 1-byte length and `host:port` in
//! UTF-8), and a checksum: the first 4 bytes of BLAKE2s-256 over every
//! field before it. The text form is `handlr1` followed by the payload in
//! base58 with the Bitcoin alphabet.

use std::fmt;

use blake2::{Blake2s256, Digest};

use super::KEY_LEN;

/// The text form's prefix.
pub const PAIRING_PREFIX: &str = "handlr1";
/// The payload version this crate writes and reads.
pub const PAIRING_VERSION: u8 = 1;

const CHECKSUM_LEN: usize = 4;

/// What a pairing string carries. `Debug` does not print the secret.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingPayload {
    /// The server's static X25519 public key.
    pub server_key: [u8; KEY_LEN],
    /// The single-use pairing secret, the PSK of the pairing handshake.
    pub secret: [u8; KEY_LEN],
    /// When the secret expires, in unix seconds.
    pub expiry: u32,
    /// The name the client is paired as.
    pub name: String,
    /// The server's addresses, `host:port`, in order of preference.
    pub addresses: Vec<String>,
}

impl fmt::Debug for PairingPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingPayload")
            .field("server_key", &self.server_key)
            .field("secret", &"<redacted>")
            .field("expiry", &self.expiry)
            .field("name", &self.name)
            .field("addresses", &self.addresses)
            .finish()
    }
}

/// Which pending pairing a client means: the first 4 bytes of
/// BLAKE2s-256 of the secret. The client sends it in the clear after the
/// mode byte `0x01`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PairingId(pub [u8; 4]);

impl PairingId {
    pub fn from_secret(secret: &[u8; KEY_LEN]) -> Self {
        PairingId(blake2s_prefix(secret))
    }
}

/// Why a pairing string or payload was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairingStringError {
    /// The string does not start with `handlr1`.
    BadPrefix,
    /// A character outside the base58 alphabet.
    BadBase58,
    /// The checksum does not match the payload.
    BadChecksum,
    /// A payload version this crate does not know.
    UnknownVersion(u8),
    /// The payload is truncated, has trailing bytes or invalid UTF-8.
    Malformed,
    /// Encoding: the name is longer than 255 bytes.
    NameTooLong,
    /// Encoding: more than 255 addresses.
    TooManyAddresses,
    /// Encoding: an address longer than 255 bytes.
    AddressTooLong,
}

impl fmt::Display for PairingStringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self {
            PairingStringError::BadPrefix => "it does not start with \"handlr1\"",
            PairingStringError::BadBase58 => "it has a character outside base58",
            PairingStringError::BadChecksum => "its checksum does not match",
            PairingStringError::UnknownVersion(v) => {
                return write!(f, "invalid pairing string: unknown version {v}")
            }
            PairingStringError::Malformed => "its payload is malformed",
            PairingStringError::NameTooLong => "the name is longer than 255 bytes",
            PairingStringError::TooManyAddresses => "there are more than 255 addresses",
            PairingStringError::AddressTooLong => "an address is longer than 255 bytes",
        };
        write!(f, "invalid pairing string: {what}")
    }
}

impl std::error::Error for PairingStringError {}

impl PairingPayload {
    /// The pairing id the client sends for this payload's secret.
    pub fn pairing_id(&self) -> PairingId {
        PairingId::from_secret(&self.secret)
    }

    /// Whether the secret has expired at `now`, in unix seconds.
    pub fn is_expired(&self, now: u64) -> bool {
        now >= u64::from(self.expiry)
    }

    /// The binary payload, with its checksum.
    pub fn to_bytes(&self) -> Result<Vec<u8>, PairingStringError> {
        let mut out = Vec::with_capacity(1 + 2 * KEY_LEN + 4 + 64);
        out.push(PAIRING_VERSION);
        out.extend_from_slice(&self.server_key);
        out.extend_from_slice(&self.secret);
        out.extend_from_slice(&self.expiry.to_be_bytes());
        let name = u8::try_from(self.name.len()).map_err(|_| PairingStringError::NameTooLong)?;
        out.push(name);
        out.extend_from_slice(self.name.as_bytes());
        let count =
            u8::try_from(self.addresses.len()).map_err(|_| PairingStringError::TooManyAddresses)?;
        out.push(count);
        for addr in &self.addresses {
            let len = u8::try_from(addr.len()).map_err(|_| PairingStringError::AddressTooLong)?;
            out.push(len);
            out.extend_from_slice(addr.as_bytes());
        }
        let sum = blake2s_prefix(&out);
        out.extend_from_slice(&sum);
        Ok(out)
    }

    /// Reads a binary payload: checksum first, then version, then fields.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PairingStringError> {
        if bytes.len() < 1 + CHECKSUM_LEN {
            return Err(PairingStringError::Malformed);
        }
        let (body, sum) = bytes.split_at(bytes.len() - CHECKSUM_LEN);
        if blake2s_prefix(body) != sum {
            return Err(PairingStringError::BadChecksum);
        }
        let mut r = Reader(body);
        let version = r.u8()?;
        if version != PAIRING_VERSION {
            return Err(PairingStringError::UnknownVersion(version));
        }
        let server_key = r.array()?;
        let secret = r.array()?;
        let expiry = u32::from_be_bytes(r.array()?);
        let name = r.string()?;
        let count = r.u8()?;
        let addresses = (0..count)
            .map(|_| r.string())
            .collect::<Result<Vec<_>, _>>()?;
        if !r.0.is_empty() {
            return Err(PairingStringError::Malformed);
        }
        Ok(PairingPayload {
            server_key,
            secret,
            expiry,
            name,
            addresses,
        })
    }

    /// The text form: `handlr1` and the payload in base58.
    pub fn encode(&self) -> Result<String, PairingStringError> {
        let bytes = self.to_bytes()?;
        Ok(format!(
            "{PAIRING_PREFIX}{}",
            bs58::encode(bytes).into_string()
        ))
    }

    /// Reads the text form. Surrounding whitespace, as from a paste, is
    /// ignored.
    pub fn decode(s: &str) -> Result<Self, PairingStringError> {
        let rest = s
            .trim()
            .strip_prefix(PAIRING_PREFIX)
            .ok_or(PairingStringError::BadPrefix)?;
        let bytes = bs58::decode(rest)
            .into_vec()
            .map_err(|_| PairingStringError::BadBase58)?;
        Self::from_bytes(&bytes)
    }
}

impl std::str::FromStr for PairingPayload {
    type Err = PairingStringError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::decode(s)
    }
}

fn blake2s_prefix(data: &[u8]) -> [u8; 4] {
    let digest = Blake2s256::digest(data);
    [digest[0], digest[1], digest[2], digest[3]]
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], PairingStringError> {
        if self.0.len() < n {
            return Err(PairingStringError::Malformed);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, PairingStringError> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PairingStringError> {
        self.take(N)?
            .try_into()
            .map_err(|_| PairingStringError::Malformed)
    }

    fn string(&mut self) -> Result<String, PairingStringError> {
        let len = usize::from(self.u8()?);
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| PairingStringError::Malformed)
    }
}
