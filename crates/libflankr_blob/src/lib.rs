//! `libflankr_blob` — binary transfers over flankr's link
//! (docs/design/20261028_05_BINARY_FILES_PLAN.md, decision 4).
//!
//! Every message of the link's kind 2 (`KIND_BINARY` in libflankr_proto)
//! is one command type with a subcommand, a transfer id and a namespace,
//! in big-endian:
//!
//! ```text
//! u8   subcommand
//! u32  transfer            (0 is invalid)
//! u8   ns_len              (1..=128)
//! ns_len bytes namespace   (ASCII [a-z0-9_.], not starting or ending with '.')
//! ...  body                (by subcommand)
//! ```
//!
//! The subcommands are REQUEST, OFFER, ACCEPT, REFUSE, CHUNK, END, DONE and
//! CANCEL ([`Body`]). All transfer control is here; none is in JSON-RPC.
//!
//! The crate is pure state: no I/O, no async. [`BlobMessage`] encodes and
//! decodes; [`Receiving`] and [`Sending`] are one transfer's two sides,
//! fed messages and the clock; [`Ids`] hands out a side's transfer ids;
//! [`ImageType`] sniffs images by their magic bytes; [`GitRequest`] parses
//! a `git` namespace request; [`ReadPattern`] and [`always_refused`] are
//! the `read` rules the daemon and the app share.

mod code;
mod git;
mod pattern;
mod sniff;
mod state;
mod wire;

pub use code::Code;
pub use git::{GitRequest, Rev};
pub use pattern::{ReadPattern, always_refused};
pub use sniff::ImageType;
pub use state::{Receiving, Sending, Step};
pub use wire::{BlobMessage, Body, DecodeError, Namespace, NamespaceError, Offer, Subcommand};

use std::time::Duration;

/// The built-in namespace of files, daemon to phone (decision 6.1-6.4).
pub const NS_FILES: &str = "files";
/// The built-in namespace of images, phone to daemon (decision 5).
pub const NS_IMAGES: &str = "images";
/// The built-in namespace of `git` output, daemon to phone (decision
/// 6.5-6.10).
pub const NS_GIT: &str = "git";

/// The binary protocol's version, as `BlobInfo::version` carries it.
pub const PROTOCOL_VERSION: u32 = 1;

/// The largest CHUNK data a receiver accepts: 960 KiB, so a CHUNK's header
/// and body stay under the link's 1 MiB message.
pub const CHUNK_MAX: u32 = 983_040;
/// The CHUNK data a sender sends by default: 256 KiB.
pub const CHUNK_DEFAULT: u32 = 262_144;
/// The largest image accepted: 20 MiB.
pub const IMAGE_MAX: u64 = 20_971_520;
/// The smallest image accepted.
pub const IMAGE_MIN: u64 = 16;
/// The largest file served: 8 MiB.
pub const FILE_MAX: u64 = 8_388_608;
/// The largest `git` output served, its cut marker included: 8 MiB.
pub const GIT_MAX: u64 = 8_388_608;
/// What ends a `git` output cut at [`GIT_MAX`] (decision 6.8).
pub const GIT_CUT_MARKER: &str = "\n[flankr: cut at 8 MiB]\n";
/// `git` processes the daemon runs at once, across connections.
pub const DAEMON_GIT_RUNNING: usize = 2;
/// Live transfers each side may start, per connection.
pub const ACTIVE_PER_SIDE: usize = 4;
/// Incoming images the daemon takes at once, across connections.
pub const DAEMON_IMAGES_ACTIVE: usize = 8;
/// Most images in one `agents.send` (decision 7.1).
pub const IMAGES_PER_SEND: usize = 4;
/// Most parts in one `agents.send` (decision 7.1).
pub const PARTS_PER_SEND: usize = 16;
/// From REQUEST or OFFER to its answer.
pub const OFFER_TIMEOUT: Duration = Duration::from_secs(30);
/// Without a CHUNK or END after ACCEPT.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// From END to DONE or CANCEL.
pub const DONE_TIMEOUT: Duration = Duration::from_secs(30);
/// From OFFER to END.
pub const TRANSFER_MAX_TIME: Duration = Duration::from_secs(600);

/// The longest namespace, in bytes.
pub const NS_MAX: usize = 128;
/// The longest session (a pane id), in bytes.
pub const SESSION_MAX: usize = 128;
/// The longest REQUEST request, in bytes.
pub const REQUEST_MAX: usize = 1024;
/// The longest OFFER name, in bytes.
pub const NAME_MAX: usize = 1024;
/// The longest OFFER MIME type, in bytes.
pub const MIME_MAX: usize = 127;
/// The longest REFUSE or CANCEL reason, in bytes.
pub const REASON_MAX: usize = 512;
/// The longest header: subcommand, transfer, namespace length and the
/// longest namespace.
pub const HEADER_MAX: usize = 1 + 4 + 1 + NS_MAX;
/// The length of an `images` DONE's reference.
pub const REFERENCE_LEN: usize = 8;

/// One transfer's id (decision 4.3). Phone-picked ids have the top bit
/// clear, daemon-picked ids have it set; 0 is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransferId(pub u32);

impl TransferId {
    const DAEMON_BIT: u32 = 0x8000_0000;

    /// Whether the daemon picked this id (its top bit is set).
    pub fn started_by_daemon(self) -> bool {
        self.0 & Self::DAEMON_BIT != 0
    }

    /// Whether the id is valid: not 0 and not `0x8000_0000`, the daemon
    /// half's 0.
    pub fn is_valid(self) -> bool {
        self.0 & !Self::DAEMON_BIT != 0
    }
}

impl std::fmt::Display for TransferId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#010x}", self.0)
    }
}

/// Hands out one side's transfer ids on one connection (decision 4.3):
/// counting up from 1 in its half, never repeating.
#[derive(Debug, Clone)]
pub struct Ids {
    next: u32,
    daemon: bool,
}

impl Ids {
    /// The phone's ids: `0x0000_0001..=0x7FFF_FFFF`.
    pub fn phone() -> Ids {
        Ids {
            next: 1,
            daemon: false,
        }
    }

    /// The daemon's ids: `0x8000_0001..=0xFFFF_FFFF`.
    pub fn daemon() -> Ids {
        Ids {
            next: 1,
            daemon: true,
        }
    }

    /// The next id, or `None` once the half's 2³¹ - 1 ids are used; a
    /// connection that gets `None` must start no more transfers.
    pub fn try_next(&mut self) -> Option<TransferId> {
        if self.next > 0x7FFF_FFFF {
            return None;
        }
        let n = self.next;
        self.next += 1;
        Some(TransferId(if self.daemon {
            n | TransferId::DAEMON_BIT
        } else {
            n
        }))
    }

    /// The next id.
    ///
    /// # Panics
    ///
    /// After 2³¹ - 1 ids on one connection; see [`try_next`](Self::try_next).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> TransferId {
        self.try_next()
            .expect("2^31 - 1 transfer ids used on one connection")
    }

    /// Whether `id` is in this side's half.
    pub fn owns(&self, id: TransferId) -> bool {
        id.started_by_daemon() == self.daemon
    }
}

/// The SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

/// `bytes` as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// An `images` DONE's reference as `agents.send`'s `parts` writes it: 16
/// lowercase hex digits. `None` for anything else.
pub fn parse_reference(text: &str) -> Option<[u8; REFERENCE_LEN]> {
    if text.len() != REFERENCE_LEN * 2
        || !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    let mut out = [0u8; REFERENCE_LEN];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_never_repeat_and_keep_to_their_half() {
        let mut phone = Ids::phone();
        let mut daemon = Ids::daemon();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let p = phone.next();
            let d = daemon.next();
            assert!(p.is_valid() && d.is_valid());
            assert!(!p.started_by_daemon() && d.started_by_daemon());
            assert!(phone.owns(p) && !phone.owns(d) && daemon.owns(d));
            assert!(seen.insert(p) && seen.insert(d));
        }
        assert_eq!(Ids::phone().next(), TransferId(1));
        assert_eq!(Ids::daemon().next(), TransferId(0x8000_0001));
        assert!(!TransferId(0).is_valid() && !TransferId(0x8000_0000).is_valid());
        let mut last = Ids {
            next: 0x7FFF_FFFF,
            daemon: true,
        };
        assert_eq!(last.try_next(), Some(TransferId(0xFFFF_FFFF)));
        assert_eq!(last.try_next(), None);
    }

    #[test]
    fn references_are_16_lowercase_hex_digits() {
        let r = [0x9f, 0x3c, 0x1a, 0x2b, 0x00, 0x11, 0x22, 0xff];
        assert_eq!(hex(&r), "9f3c1a2b001122ff");
        assert_eq!(parse_reference("9f3c1a2b001122ff"), Some(r));
        for bad in [
            "9F3C1A2B001122FF",
            "9f3c1a2b001122f",
            "9f3c1a2b001122ffa",
            "",
            "9f3c1a2b0011 2ff",
        ] {
            assert_eq!(parse_reference(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_chunk_at_the_limits_fits_in_a_link_message() {
        // libflankr_proto's MAX_MESSAGE is 1 MiB.
        assert!(HEADER_MAX + 1 + 8 + CHUNK_MAX as usize <= 1 << 20);
        assert_eq!(HEADER_MAX, 134);
        assert_eq!(GIT_CUT_MARKER.len(), 24);
    }
}
