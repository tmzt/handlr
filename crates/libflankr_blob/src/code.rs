//! Refusal and cancel codes (decision 4.6).

use crate::NS_IMAGES;

/// A REFUSE or CANCEL code: `u16`, 0 reserved, 1-999 this crate's,
/// 1000-65535 a namespace's own. Open: a code this crate does not name is
/// still a code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Code(pub u16);

impl Code {
    /// The namespace or subcommand is not served.
    pub const UNSUPPORTED: Code = Code(1);
    /// The session's pane is gone.
    pub const NO_SESSION: Code = Code(2);
    /// No `[[agents]]` entry, out of reach, or the path's rules refuse it.
    pub const NOT_PERMITTED: Code = Code(3);
    /// Over `IMAGE_MAX`, `FILE_MAX` or REQUEST's `max_size`.
    pub const TOO_LARGE: Code = Code(4);
    /// MIME not allowed, magic bytes disagree, or text not UTF-8.
    pub const BAD_TYPE: Code = Code(5);
    /// Over a concurrency limit.
    pub const BUSY: Code = Code(6);
    /// The store cannot make room.
    pub const NO_SPACE: Code = Code(7);
    /// No such file.
    pub const NOT_FOUND: Code = Code(8);
    /// `read` is empty.
    pub const FILES_OFF: Code = Code(9);
    /// The SHA-256 differs from OFFER's.
    pub const HASH_MISMATCH: Code = Code(10);
    /// END or the bytes differ from OFFER's size.
    pub const SIZE_MISMATCH: Code = Code(11);
    /// A timeout of decision 4.4.
    pub const TIMEOUT: Code = Code(12);
    /// A write or read failed.
    pub const IO: Code = Code(13);
    /// Out of order, a bad body, the wrong namespace or id.
    pub const PROTOCOL: Code = Code(14);
    /// The user cancelled.
    pub const CANCELLED: Code = Code(15);
    /// The receiver already holds this SHA-256.
    pub const HAVE_IT: Code = Code(16);
    /// No such live transfer.
    pub const UNKNOWN_TRANSFER: Code = Code(17);
    /// A `git` request that is not one of decision 6.5's forms, or a rev
    /// that 6.6 refuses.
    pub const BAD_REQUEST: Code = Code(18);
    /// The session's folder is not in a git work tree.
    pub const NOT_A_REPOSITORY: Code = Code(19);
    /// `git` exited non-zero; the reason is its stderr's first line.
    pub const FAILED: Code = Code(20);

    /// The code's name, as decision 4.6 writes it, if this crate names it.
    pub fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            1 => "unsupported",
            2 => "no_session",
            3 => "not_permitted",
            4 => "too_large",
            5 => "bad_type",
            6 => "busy",
            7 => "no_space",
            8 => "not_found",
            9 => "files_off",
            10 => "hash_mismatch",
            11 => "size_mismatch",
            12 => "timeout",
            13 => "io",
            14 => "protocol",
            15 => "cancelled",
            16 => "have_it",
            17 => "unknown_transfer",
            18 => "bad_request",
            19 => "not_a_repository",
            20 => "failed",
            _ => return None,
        })
    }

    /// The app's line for this code in namespace `ns` (decision 4.6),
    /// `reason` being the REFUSE or CANCEL's reason; `None` for the codes
    /// that show no line (`cancelled`, `have_it`, `unknown_transfer`). A
    /// code this crate does not name shows the protocol line.
    pub fn line(self, ns: &str, reason: &str) -> Option<String> {
        let images = ns == NS_IMAGES;
        Some(
            match self {
                Code::UNSUPPORTED => "This daemon can't take that.",
                Code::NO_SESSION => "That session has closed.",
                Code::NOT_PERMITTED => "Not allowed for this session.",
                Code::TOO_LARGE if images => "Too large (over 20 MB).",
                Code::TOO_LARGE => "Too large (over 8 MB).",
                Code::BAD_TYPE if images => "Not a PNG, JPEG, WebP or GIF image.",
                Code::BAD_TYPE => "No preview for this kind of file.",
                Code::BUSY => "Busy, try again in a moment.",
                Code::NO_SPACE => "No room on the computer for images.",
                Code::NOT_FOUND => "File not found.",
                Code::FILES_OFF => "File access is off on this daemon.",
                Code::HASH_MISMATCH | Code::SIZE_MISMATCH => "The upload was damaged; try again.",
                Code::TIMEOUT => "Timed out.",
                Code::IO => "The computer couldn't save it.",
                Code::CANCELLED | Code::HAVE_IT | Code::UNKNOWN_TRANSFER => return None,
                Code::BAD_REQUEST => "That isn't a commit flankr can show.",
                Code::NOT_A_REPOSITORY => "This session's folder isn't a git repository.",
                Code::FAILED => return Some(format!("git couldn't show that: {reason}")),
                _ => "Something went wrong (protocol).",
            }
            .to_string(),
        )
    }
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "code {}", self.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_have_their_names_and_lines() {
        for n in 1..=20u16 {
            assert!(Code(n).name().is_some(), "{n}");
        }
        assert_eq!(Code(0).name(), None);
        assert_eq!(Code(1000).to_string(), "code 1000");
        assert_eq!(Code::NOT_PERMITTED.to_string(), "not_permitted");
        assert_eq!(
            Code::TOO_LARGE.line("images", "").unwrap(),
            "Too large (over 20 MB)."
        );
        assert_eq!(
            Code::TOO_LARGE.line("files", "").unwrap(),
            "Too large (over 8 MB)."
        );
        assert_eq!(
            Code::FAILED.line("git", "unknown revision").unwrap(),
            "git couldn't show that: unknown revision"
        );
        assert_eq!(Code::HAVE_IT.line("files", ""), None);
        assert_eq!(
            Code(1234).line("dev.x.y", "").unwrap(),
            "Something went wrong (protocol)."
        );
    }
}
