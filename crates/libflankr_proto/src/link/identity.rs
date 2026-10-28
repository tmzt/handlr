//! A daemon's identity, from its link static public key
//! (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md, decisions 1-2 and
//! the 13:03 EDT amendment's item 7).
//!
//! * [`fingerprint`] is BLAKE2s-256 over the 32-byte key, nothing else.
//! * The remote's **id** ([`remote_id`]) is its first 8 bytes as 16
//!   lower-case hex digits, `320b5ea99e653bc2`; the phone names a remote's
//!   directory and key-store entry with it.
//! * The **fingerprint** label ([`fingerprint_text`]) is the id's first 8
//!   hex digits in two groups of four, `320b 5ea9`. Since the 14:37 EDT
//!   amendment it is only a label that tells remotes apart (32 bits can be
//!   matched by a targeted attacker); a daemon's identity is compared by
//!   its fingerprint words.
//! * The **fingerprint words** ([`fingerprint_words`](super::fingerprint_words))
//!   are the same hash's first 66 bits as six BIP-39 words (the 14:37 EDT
//!   amendment's item 3); see the `words` module.
//!
//! None of these is a secret: each is computed from a public key.

use blake2::{Blake2s256, Digest};

use super::KEY_LEN;

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

/// The fingerprint label: the id's first 8 hex digits in two groups of
/// four, `3f9a 12c4`. A label for telling remotes apart, not for checking a
/// daemon's identity: that is [`fingerprint_words`](super::fingerprint_words).
pub fn fingerprint_text(key: &[u8; KEY_LEN]) -> String {
    let digits = hex(&fingerprint(key)[..4]);
    format!("{} {}", &digits[..4], &digits[4..])
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
    /// `hashlib.blake2s(key, digest_size=32)`.
    #[test]
    fn fixed_vectors() {
        let zero = [0u8; KEY_LEN];
        assert_eq!(
            hex(&fingerprint(&zero)),
            "320b5ea99e653bc2b593db4130d10a4efd3a0b4cc2e1a6672b678d71dfbd33ad"
        );
        assert_eq!(remote_id(&zero), "320b5ea99e653bc2");
        assert_eq!(fingerprint_text(&zero), "320b 5ea9");

        let counting = counting();
        assert_eq!(
            hex(&fingerprint(&counting)),
            "05825607d7fdf2d82ef4c3c8c2aea961ad98d60edff7d018983e21204c0d93d1"
        );
        assert_eq!(remote_id(&counting), "05825607d7fdf2d8");
        assert_eq!(fingerprint_text(&counting), "0582 5607");

        let eights = [8u8; KEY_LEN];
        assert_eq!(remote_id(&eights), "1a5fe5de0f59b35e");
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
    fn the_id_is_sixteen_hex_digits_for_any_key() {
        for b in 0..=255u8 {
            let id = remote_id(&[b; KEY_LEN]);
            assert_eq!(id.len(), 16);
            assert!(id.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')));
        }
    }
}
