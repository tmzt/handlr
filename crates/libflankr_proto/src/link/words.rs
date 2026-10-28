//! Six BIP-39 words: the pairing words and the fingerprint words
//! (docs/design/20261028_01_MULTIPLE_REMOTES_PLAN.md, the 14:37 EDT
//! amendment's items 1.4 and 3).
//!
//! Both take the first 66 bits of a BLAKE2s-256 digest, big-endian (the
//! first 8 bytes and the top 2 bits of the 9th), as six 11-bit indexes into
//! the BIP-39 English word list of 2048 words, first index first.
//!
//! * The **pairing words** ([`pairing_words`]) hash
//!   [`PAIRING_WORDS_CONTEXT`], the daemon's static key, the `flankr pair`
//!   run's offer and the device's static key, in that order. `flankr pair`
//!   and the phone both show them; the owner types **yes** on the Mac only
//!   when they match. They bind the request to both keys and the offer, so
//!   a substituted link or device key shows different words.
//! * The **fingerprint words** ([`fingerprint_words`]) are the same
//!   66 bits of [`fingerprint`](super::fingerprint) (BLAKE2s-256 of the
//!   daemon's key alone): how a daemon's identity is compared by eye. The
//!   hex [`fingerprint_text`](super::fingerprint_text) stays a label.
//!
//! The two are shown labelled "pairing words" and "daemon fingerprint".
//!
//! The word list is the `bip39` crate's English list (CC0-1.0, the list of
//! the BIP-39 specification, SHA-256 of its text file
//! `2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda`). It
//! is used only for these words, never for keys.

use blake2::{Blake2s256, Digest};

use super::{KEY_LEN, OFFER_LEN, fingerprint};

/// The number of words in a pairing or fingerprint phrase.
pub const WORD_COUNT: usize = 6;

/// What the pairing words' hash starts with, before the keys and the offer.
pub const PAIRING_WORDS_CONTEXT: &[u8] = b"flankr.pair.words/1";

/// The six pairing words: the first 66 bits of BLAKE2s-256 over
/// [`PAIRING_WORDS_CONTEXT`], `server_key`, `offer` and `device_key`, as
/// six BIP-39 English words.
pub fn pairing_words(
    server_key: &[u8; KEY_LEN],
    offer: &[u8; OFFER_LEN],
    device_key: &[u8; KEY_LEN],
) -> [&'static str; WORD_COUNT] {
    let mut hasher = Blake2s256::new();
    hasher.update(PAIRING_WORDS_CONTEXT);
    hasher.update(server_key);
    hasher.update(offer);
    hasher.update(device_key);
    words_of(&hasher.finalize().into())
}

/// [`pairing_words`] joined by single spaces.
pub fn pairing_words_text(
    server_key: &[u8; KEY_LEN],
    offer: &[u8; OFFER_LEN],
    device_key: &[u8; KEY_LEN],
) -> String {
    pairing_words(server_key, offer, device_key).join(" ")
}

/// The daemon's fingerprint as six words: the first 66 bits of
/// [`fingerprint`] as six BIP-39 English words.
pub fn fingerprint_words(key: &[u8; KEY_LEN]) -> [&'static str; WORD_COUNT] {
    words_of(&fingerprint(key))
}

/// [`fingerprint_words`] joined by single spaces.
pub fn fingerprint_words_text(key: &[u8; KEY_LEN]) -> String {
    fingerprint_words(key).join(" ")
}

/// The digest's first 66 bits as six words.
fn words_of(digest: &[u8; 32]) -> [&'static str; WORD_COUNT] {
    let list = bip39::Language::English.word_list();
    let mut head = [0u8; 16];
    head[..9].copy_from_slice(&digest[..9]);
    // The first 72 bits, at the top of a u128; the top 66 are the indexes.
    let bits = u128::from_be_bytes(head);
    std::array::from_fn(|i| {
        let index = (bits >> (128 - 11 * (i + 1))) & 0x7FF;
        list[index as usize]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counting<const N: usize>() -> [u8; N] {
        std::array::from_fn(|i| i as u8)
    }

    /// The vectors were computed outside this crate, with Python's
    /// `hashlib.blake2s(..., digest_size=32)` and the BIP-39 English list
    /// from `bitcoin/bips` (`bip-0039/english.txt`, SHA-256
    /// `2f5eed53…24dbda`): `n = int.from_bytes(d[:9], "big") >> 6`, word
    /// `i` is `list[(n >> 11 * (5 - i)) & 0x7ff]`.
    #[test]
    fn fingerprint_word_vectors() {
        assert_eq!(
            fingerprint_words(&[0; KEY_LEN]),
            ["craft", "foot", "price", "devote", "father", "throw"]
        );
        assert_eq!(
            fingerprint_words_text(&counting()),
            "airport bargain limit quiz tennis hire"
        );
        assert_eq!(
            fingerprint_words_text(&[8; KEY_LEN]),
            "bottom you jealous burger open quality"
        );
    }

    #[test]
    fn pairing_word_vectors() {
        assert_eq!(
            pairing_words(&[0; KEY_LEN], &[0; OFFER_LEN], &[0; KEY_LEN]),
            ["east", "alcohol", "team", "sell", "length", "dose"]
        );
        assert_eq!(
            pairing_words_text(&counting(), &counting(), &[8; KEY_LEN]),
            "brisk finish enemy kiss spend select"
        );
        // The keys swapped: different words.
        assert_eq!(
            pairing_words_text(&[8; KEY_LEN], &counting(), &counting()),
            "wall select mention direct close cradle"
        );
    }

    #[test]
    fn the_words_take_the_digest_s_first_66_bits() {
        // 0x00 0x20 0x00 0x01 0x00 ...: index 0 is 0b0000_0000_001 = 1;
        // index 1 is the next 11 bits, 0; index 2 is bits 22-32, of which
        // byte 3's low bit is bit 31, so 0b10 = 2.
        let mut d = [0u8; 32];
        d[1] = 0x20;
        d[3] = 0x01;
        let list = bip39::Language::English.word_list();
        assert_eq!(
            words_of(&d),
            [list[1], list[0], list[2], list[0], list[0], list[0]]
        );
        // All 66 bits set; the 67th and later do not matter.
        let mut d = [0u8; 32];
        d[..8].fill(0xFF);
        d[8] = 0xC0;
        assert_eq!(words_of(&d), ["zoo"; WORD_COUNT]);
        d[8] = 0xFF;
        d[9..].fill(0xFF);
        assert_eq!(words_of(&d), ["zoo"; WORD_COUNT]);
        // The list itself.
        assert_eq!(list.len(), 2048);
        assert_eq!((list[0], list[2047]), ("abandon", "zoo"));
    }

    #[test]
    fn each_input_changes_the_pairing_words() {
        let base = pairing_words(&[1; KEY_LEN], &[2; OFFER_LEN], &[3; KEY_LEN]);
        assert_ne!(
            base,
            pairing_words(&[9; KEY_LEN], &[2; OFFER_LEN], &[3; KEY_LEN])
        );
        assert_ne!(
            base,
            pairing_words(&[1; KEY_LEN], &[9; OFFER_LEN], &[3; KEY_LEN])
        );
        assert_ne!(
            base,
            pairing_words(&[1; KEY_LEN], &[2; OFFER_LEN], &[9; KEY_LEN])
        );
    }
}
