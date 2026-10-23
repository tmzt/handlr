//! The pairing string and the device code (plan 02, amendment of
//! 23:07 EDT: pairing by public keys, with no shared secret).
//!
//! The pairing string's payload, as bytes: version (1 byte, `2`), the
//! server's public key (32), addresses (1-byte count, then each a 1-byte
//! length and `host:port` in UTF-8), and a checksum: the first 4 bytes of
//! BLAKE2s-256 over every field before it. The text form is `handlr1`
//! followed by the payload in base58 with the Bitcoin alphabet. It holds no
//! secret, so it may be shown, logged or kept freely.
//!
//! The pairing URL (plan 20261022_06, decision 1, as amended at 23:07 EDT)
//! is `handlr://<host>:<port>/pair#<pairing string>`: the address a phone
//! reaches the server at, and the string. A QR code carries it.
//!
//! The device code is what the device shows for the user to paste into
//! `handlr pair`: version (1 byte, `1`), the device's public key (32), its
//! name (1-byte length, then 1 to 32 bytes of ASCII letters, digits, `-`,
//! `_` or `.`), and a checksum: the first 4 bytes of BLAKE2s-256 over the
//! **server's** public key, then every field before it. The text form is
//! `handlrkey1` and base58. Because the checksum covers the server's key, a
//! code made for another server, or mistyped, fails it.

use std::fmt;

use blake2::{Blake2s256, Digest};

use super::KEY_LEN;

/// The pairing string's prefix.
pub const PAIRING_PREFIX: &str = "handlr1";
/// The pairing URL's scheme.
pub const PAIRING_URL_SCHEME: &str = "handlr";
/// The pairing URL's path.
const PAIRING_URL_PATH: &str = "/pair";

/// The pairing payload version this crate writes and reads.
pub const PAIRING_VERSION: u8 = 2;

/// The device code's prefix.
pub const DEVICE_CODE_PREFIX: &str = "handlrkey1";
/// The device code version this crate writes and reads.
pub const DEVICE_CODE_VERSION: u8 = 1;
/// The longest device name, in bytes.
pub const DEVICE_NAME_MAX: usize = 32;

const CHECKSUM_LEN: usize = 4;

/// What a pairing string carries: the server's key and where to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingPayload {
    /// The server's static X25519 public key.
    pub server_key: [u8; KEY_LEN],
    /// The server's addresses, `host:port`, in order of preference.
    pub addresses: Vec<String>,
}

/// What a device code carries: the device's key and the name it chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    /// The device's static X25519 public key.
    pub key: [u8; KEY_LEN],
    /// 1 to 32 bytes of ASCII letters, digits, `-`, `_` or `.`.
    pub name: String,
}

/// Why a pairing string, a pairing URL or a device code was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PairingStringError {
    /// The text does not start with `handlr1` (a pairing string) or
    /// `handlrkey1` (a device code).
    BadPrefix,
    /// A character outside the base58 alphabet.
    BadBase58,
    /// The checksum does not match. For a device code, it may also have
    /// been made for another server.
    BadChecksum,
    /// A version this crate does not know.
    UnknownVersion(u8),
    /// The payload is truncated, has trailing bytes or invalid UTF-8.
    Malformed,
    /// Encoding: more than 255 addresses.
    TooManyAddresses,
    /// Encoding: an address longer than 255 bytes.
    AddressTooLong,
    /// A pairing URL that is not `handlr://<host>:<port>/pair#<string>`
    /// with a host the payload carries, or a payload with no address.
    BadUrl,
    /// A device name that is not 1 to 32 bytes of ASCII letters, digits,
    /// `-`, `_` or `.`.
    BadName,
}

impl fmt::Display for PairingStringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self {
            PairingStringError::BadPrefix => {
                "it does not start with \"handlr1\" (or \"handlrkey1\" for a device code)"
            }
            PairingStringError::BadBase58 => "it has a character outside base58",
            PairingStringError::BadChecksum => "its checksum does not match",
            PairingStringError::UnknownVersion(v) => {
                return write!(f, "invalid pairing string: unknown version {v}")
            }
            PairingStringError::Malformed => "its payload is malformed",
            PairingStringError::TooManyAddresses => "there are more than 255 addresses",
            PairingStringError::AddressTooLong => "an address is longer than 255 bytes",
            PairingStringError::BadUrl => {
                "it is not a handlr://<host>:<port>/pair# URL for one of its addresses"
            }
            PairingStringError::BadName => {
                "the name is not 1 to 32 ASCII letters, digits, '-', '_' or '.'"
            }
        };
        write!(f, "invalid pairing string: {what}")
    }
}

impl std::error::Error for PairingStringError {}

impl PairingPayload {
    /// The binary payload, with its checksum.
    pub fn to_bytes(&self) -> Result<Vec<u8>, PairingStringError> {
        let mut out = Vec::with_capacity(1 + KEY_LEN + 64);
        out.push(PAIRING_VERSION);
        out.extend_from_slice(&self.server_key);
        let count =
            u8::try_from(self.addresses.len()).map_err(|_| PairingStringError::TooManyAddresses)?;
        out.push(count);
        for addr in &self.addresses {
            let len = u8::try_from(addr.len()).map_err(|_| PairingStringError::AddressTooLong)?;
            out.push(len);
            out.extend_from_slice(addr.as_bytes());
        }
        let sum = blake2s_prefix(&[&out]);
        out.extend_from_slice(&sum);
        Ok(out)
    }

    /// Reads a binary payload: checksum first, then version, then fields.
    /// A version 1 payload (with a secret) is [`PairingStringError::UnknownVersion`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PairingStringError> {
        let mut r = Reader(checked(bytes, &[])?);
        let version = r.u8()?;
        if version != PAIRING_VERSION {
            return Err(PairingStringError::UnknownVersion(version));
        }
        let server_key = r.array()?;
        let count = r.u8()?;
        let addresses = (0..count)
            .map(|_| r.string())
            .collect::<Result<Vec<_>, _>>()?;
        r.end()?;
        Ok(PairingPayload {
            server_key,
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

    /// The pairing URL: `handlr://<host>:<port>/pair#<string>`, where
    /// `<host>:<port>` is the first address that is not loopback (with
    /// `tailscale serve`, the Tailscale name), or the first address if all
    /// are.
    pub fn url(&self) -> Result<String, PairingStringError> {
        let address = self
            .addresses
            .iter()
            .find(|a| !is_loopback(a))
            .or_else(|| self.addresses.first())
            .ok_or(PairingStringError::BadUrl)?;
        Ok(format!(
            "{PAIRING_URL_SCHEME}://{address}{PAIRING_URL_PATH}#{}",
            self.encode()?
        ))
    }

    /// Whether any address can be reached from another machine.
    pub fn has_remote_address(&self) -> bool {
        self.addresses.iter().any(|a| !is_loopback(a))
    }

    /// Reads the text form or the pairing URL. Surrounding whitespace, as
    /// from a paste or a scan, is ignored. A URL's host must be one of the
    /// payload's addresses.
    pub fn decode(s: &str) -> Result<Self, PairingStringError> {
        let s = s.trim();
        let Some(rest) = s
            .strip_prefix(PAIRING_URL_SCHEME)
            .and_then(|r| r.strip_prefix("://"))
        else {
            return Self::decode_string(s);
        };
        let (before, string) = rest.split_once('#').ok_or(PairingStringError::BadUrl)?;
        let address = before
            .strip_suffix(PAIRING_URL_PATH)
            .ok_or(PairingStringError::BadUrl)?;
        let payload = Self::decode_string(string)?;
        if !payload.addresses.iter().any(|a| a == address) {
            return Err(PairingStringError::BadUrl);
        }
        Ok(payload)
    }

    fn decode_string(s: &str) -> Result<Self, PairingStringError> {
        Self::from_bytes(&from_base58(s, PAIRING_PREFIX)?)
    }
}

impl std::str::FromStr for PairingPayload {
    type Err = PairingStringError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::decode(s)
    }
}

impl DeviceCode {
    /// Whether `name` may be a device's name: 1 to 32 bytes of ASCII
    /// letters, digits, `-`, `_` or `.`.
    pub fn valid_name(name: &str) -> bool {
        (1..=DEVICE_NAME_MAX).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    }

    /// The text form for the server whose key is `server_key`:
    /// `handlrkey1` and the code in base58.
    pub fn encode(&self, server_key: &[u8; KEY_LEN]) -> Result<String, PairingStringError> {
        if !Self::valid_name(&self.name) {
            return Err(PairingStringError::BadName);
        }
        let mut out = Vec::with_capacity(1 + KEY_LEN + 1 + self.name.len() + CHECKSUM_LEN);
        out.push(DEVICE_CODE_VERSION);
        out.extend_from_slice(&self.key);
        out.push(self.name.len() as u8);
        out.extend_from_slice(self.name.as_bytes());
        let sum = blake2s_prefix(&[server_key, &out]);
        out.extend_from_slice(&sum);
        Ok(format!(
            "{DEVICE_CODE_PREFIX}{}",
            bs58::encode(out).into_string()
        ))
    }

    /// Reads the text form, checking it against `server_key`.
    /// Surrounding whitespace is ignored. A code made for another server
    /// fails as [`PairingStringError::BadChecksum`].
    pub fn decode(s: &str, server_key: &[u8; KEY_LEN]) -> Result<Self, PairingStringError> {
        let bytes = from_base58(s.trim(), DEVICE_CODE_PREFIX)?;
        let mut r = Reader(checked(&bytes, server_key)?);
        let version = r.u8()?;
        if version != DEVICE_CODE_VERSION {
            return Err(PairingStringError::UnknownVersion(version));
        }
        let key = r.array()?;
        let name = r.string()?;
        r.end()?;
        if !Self::valid_name(&name) {
            return Err(PairingStringError::BadName);
        }
        Ok(DeviceCode { key, name })
    }
}

/// The base58 after `prefix`.
fn from_base58(s: &str, prefix: &str) -> Result<Vec<u8>, PairingStringError> {
    let rest = s
        .strip_prefix(prefix)
        .ok_or(PairingStringError::BadPrefix)?;
    bs58::decode(rest)
        .into_vec()
        .map_err(|_| PairingStringError::BadBase58)
}

/// The body of `bytes`, once its trailing checksum matches BLAKE2s-256 of
/// `salt` then the body.
fn checked<'a>(bytes: &'a [u8], salt: &[u8]) -> Result<&'a [u8], PairingStringError> {
    if bytes.len() < 1 + CHECKSUM_LEN {
        return Err(PairingStringError::Malformed);
    }
    let (body, sum) = bytes.split_at(bytes.len() - CHECKSUM_LEN);
    if blake2s_prefix(&[salt, body]) != sum {
        return Err(PairingStringError::BadChecksum);
    }
    Ok(body)
}

/// Whether `host:port` names this machine only: `localhost`, `127.0.0.0/8`
/// or `[::1]`.
fn is_loopback(address: &str) -> bool {
    let host = address.rsplit_once(':').map_or(address, |(host, _)| host);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The first 4 bytes of BLAKE2s-256 over `parts`, in order.
fn blake2s_prefix(parts: &[&[u8]]) -> [u8; CHECKSUM_LEN] {
    let mut hasher = Blake2s256::new();
    for part in parts {
        hasher.update(part);
    }
    let digest = hasher.finalize();
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

    fn end(&self) -> Result<(), PairingStringError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(PairingStringError::Malformed)
        }
    }
}
