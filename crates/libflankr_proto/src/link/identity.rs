//! A daemon's identity, from its link static public key
//! (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md, decisions 1-2 and
//! the 13:03 EDT amendment's item 7).
//!
//! * [`fingerprint`] is BLAKE2s-256 over the 32-byte key, nothing else.
//! * The remote's **id** ([`remote_id`]) is its first 8 bytes as 16
//!   lower-case hex digits, `320b5ea99e653bc2`; the phone names a remote's
//!   directory and key-store entry with it.
//! * The **fingerprint** shown to people ([`fingerprint_text`]) is the
//!   id's first 8 hex digits in two groups of four, `320b 5ea9`.
//! * The **confirmation code** ([`confirmation_code`]) is six decimal
//!   digits that `flankr pair` prints and the phone shows while pairing,
//!   so the owner can check that the phone was given this daemon's key. It
//!   is a separate hash, BLAKE2s-256 over [`CONFIRMATION_CONTEXT`] followed
//!   by the key, so it says nothing about the fingerprint's digits. Its
//!   first 20 bits (big-endian: byte 0, byte 1, and the high nibble of
//!   byte 2) are a number from 0 to 1,048,575; that number **modulo
//!   1,000,000**, zero-padded to six digits, is the code. The values below
//!   48,576 are so twice as likely as the rest, which is of no matter for
//!   a code read by eye.
//!
//! None of these is a secret: each is computed from a public key.

use blake2::{Blake2s256, Digest};

use super::KEY_LEN;

/// What the confirmation code's hash starts with, before the key.
pub const CONFIRMATION_CONTEXT: &[u8] = b"flankr.pair.confirm/1";

/// The number of bytes of [`fingerprint`] in a remote's id.
pub const REMOTE_ID_LEN: usize = 8;

/// BLAKE2s-256 over a link static public key.
pub fn fingerprint(key: &[u8; KEY_LEN]) -> [u8; 32] {
    Blake2s256::digest(key).into()
}

/// The remote's id: [`fingerprint`]'s first 8 bytes as 16 lower-case hex
/// digits.
pub fn remote_id(key: &[u8; KEY_LEN]) -> String {
    hex(&fingerprint(key)[..REMOTE_ID_LEN])
}

/// The fingerprint as shown: the id's first 8 hex digits in two groups of
/// four, `3f9a 12c4`.
pub fn fingerprint_text(key: &[u8; KEY_LEN]) -> String {
    let digits = hex(&fingerprint(key)[..4]);
    format!("{} {}", &digits[..4], &digits[4..])
}

/// The six-digit confirmation code: the first 20 bits of BLAKE2s-256 over
/// [`CONFIRMATION_CONTEXT`] and the key, modulo 1,000,000, zero-padded.
pub fn confirmation_code(key: &[u8; KEY_LEN]) -> String {
    let mut hasher = Blake2s256::new();
    hasher.update(CONFIRMATION_CONTEXT);
    hasher.update(key);
    let digest = hasher.finalize();
    let bits =
        (u32::from(digest[0]) << 12) | (u32::from(digest[1]) << 4) | (u32::from(digest[2]) >> 4);
    format!("{:06}", bits % 1_000_000)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counting() -> [u8; KEY_LEN] {
        std::array::from_fn(|i| i as u8)
    }

    /// The vectors were computed outside this crate, with Python's
    /// `hashlib.blake2s(key, digest_size=32)` and
    /// `hashlib.blake2s(b"flankr.pair.confirm/1" + key, digest_size=32)`.
    #[test]
    fn fixed_vectors() {
        let zero = [0u8; KEY_LEN];
        assert_eq!(
            hex(&fingerprint(&zero)),
            "320b5ea99e653bc2b593db4130d10a4efd3a0b4cc2e1a6672b678d71dfbd33ad"
        );
        assert_eq!(remote_id(&zero), "320b5ea99e653bc2");
        assert_eq!(fingerprint_text(&zero), "320b 5ea9");
        assert_eq!(confirmation_code(&zero), "543268");

        let counting = counting();
        assert_eq!(
            hex(&fingerprint(&counting)),
            "05825607d7fdf2d82ef4c3c8c2aea961ad98d60edff7d018983e21204c0d93d1"
        );
        assert_eq!(remote_id(&counting), "05825607d7fdf2d8");
        assert_eq!(fingerprint_text(&counting), "0582 5607");
        // 21,485: zero-padded.
        assert_eq!(confirmation_code(&counting), "021485");

        // The first 20 bits are 1,000,249: past six digits, so taken
        // modulo 1,000,000.
        let eights = [8u8; KEY_LEN];
        assert_eq!(remote_id(&eights), "1a5fe5de0f59b35e");
        assert_eq!(confirmation_code(&eights), "000249");
    }

    #[test]
    fn fingerprint_is_blake2s_256_over_the_key_alone() {
        let key = counting();
        let mut hasher = Blake2s256::new();
        hasher.update(key);
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(fingerprint(&key), expected);
        assert!(remote_id(&key).starts_with(&fingerprint_text(&key).replace(' ', "")));
    }

    #[test]
    fn the_code_is_six_digits_for_any_key() {
        for b in 0..=255u8 {
            let code = confirmation_code(&[b; KEY_LEN]);
            assert_eq!(code.len(), 6, "{code}");
            assert!(code.bytes().all(|c| c.is_ascii_digit()), "{code}");
            let id = remote_id(&[b; KEY_LEN]);
            assert_eq!(id.len(), 16);
            assert!(id.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')));
        }
    }
}
