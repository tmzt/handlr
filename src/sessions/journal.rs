//! The approval journal (plan 20261023_03, the 14:54 EDT amendment, A4).
//!
//! Approval cards come from handlr's confirmations, not the session file.
//! So that history and a restarted daemon can rebuild them, each
//! confirmation that reaches a session's pane is appended to
//! `~/.local/state/handlr/sessions/<session id>.jsonl` (directory 0700,
//! file 0600), one line when it is asked and one when it resolves:
//!
//! ```json
//! {"ts":"2026-10-23T18:40:02Z","line":412,"slot":64,"confirm_id":17,"command":"/compact","text":"…"}
//! {"ts":"2026-10-23T18:40:09Z","confirm_id":17,"outcome":"approved"}
//! ```
//!
//! `line` is the session file's newest line when it was asked and `slot`
//! the card's (64 to 127, the first free on that line). `command` and
//! `text` (the question the companion is asked) are redacted. A missing
//! or deleted journal loses only the Approval cards; no serial depends on
//! it.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::sections::{ApprovalCard, SLOT_APPROVAL_FIRST, SLOT_APPROVAL_LAST};

/// One journal line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Line {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slot: Option<u8>,
    confirm_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
}

/// A confirmation as the journal has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub line: u32,
    pub slot: u8,
    pub confirm_id: u64,
    pub command: String,
    pub text: String,
    /// `approved`, `declined`, `expired` …; `None` while pending, or when
    /// the daemon stopped before it resolved.
    pub outcome: Option<String>,
}

impl Approval {
    /// Its card: live when it has no outcome and `pending` says it is
    /// still waiting.
    pub fn card(&self, pending: bool) -> ApprovalCard {
        ApprovalCard {
            line: self.line,
            slot: self.slot,
            confirm_id: self.confirm_id,
            text: self.text.clone(),
            live: pending && self.outcome.is_none(),
        }
    }
}

/// The journals' directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    dir: PathBuf,
}

impl Journal {
    pub fn new(dir: PathBuf) -> Self {
        Journal { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The journal of the session `session`: its id, or its file's stem.
    /// `None` for a name that is not safe as a file name.
    pub fn path(&self, session: &str) -> Option<PathBuf> {
        super::safe_id(session).then(|| self.dir.join(format!("{session}.jsonl")))
    }

    /// Every confirmation in `session`'s journal, in the order asked, with
    /// its outcome. A missing or unreadable journal has none.
    pub fn read(&self, session: &str) -> Vec<Approval> {
        let Some(path) = self.path(session) else {
            return Vec::new();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        let mut out: Vec<Approval> = Vec::new();
        for line in text.lines() {
            let Ok(l) = serde_json::from_str::<Line>(line) else {
                continue;
            };
            match (l.line, l.slot, l.outcome) {
                (Some(line), Some(slot), outcome) => {
                    if !out.iter().any(|a| a.confirm_id == l.confirm_id) {
                        out.push(Approval {
                            line,
                            slot,
                            confirm_id: l.confirm_id,
                            command: l.command.unwrap_or_default(),
                            text: l.text.unwrap_or_default(),
                            outcome,
                        });
                    }
                }
                (_, _, Some(outcome)) => {
                    if let Some(a) = out.iter_mut().find(|a| a.confirm_id == l.confirm_id) {
                        a.outcome = Some(outcome);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// The first free Approval slot on `line` among `known`, if any is.
    pub fn free_slot(known: &[Approval], line: u32) -> Option<u8> {
        (SLOT_APPROVAL_FIRST..=SLOT_APPROVAL_LAST)
            .find(|s| !known.iter().any(|a| a.line == line && a.slot == *s))
    }

    /// Record a confirmation asked at `line` of `session`. Returns what was
    /// recorded, or `None` when it is already there or the line has no free
    /// slot.
    pub fn asked(
        &self,
        session: &str,
        line: u32,
        confirm_id: u64,
        command: &str,
        text: &str,
    ) -> std::io::Result<Option<Approval>> {
        let known = self.read(session);
        if known.iter().any(|a| a.confirm_id == confirm_id) {
            return Ok(None);
        }
        let Some(slot) = Self::free_slot(&known, line) else {
            return Ok(None);
        };
        let approval = Approval {
            line,
            slot,
            confirm_id,
            command: command.to_string(),
            text: text.to_string(),
            outcome: None,
        };
        self.append(
            session,
            &Line {
                ts: now_rfc3339(),
                line: Some(line),
                slot: Some(slot),
                confirm_id,
                command: Some(approval.command.clone()),
                text: Some(approval.text.clone()),
                outcome: None,
            },
        )?;
        Ok(Some(approval))
    }

    /// Record how confirmation `confirm_id` of `session` resolved.
    pub fn resolved(&self, session: &str, confirm_id: u64, outcome: &str) -> std::io::Result<()> {
        self.append(
            session,
            &Line {
                ts: now_rfc3339(),
                line: None,
                slot: None,
                confirm_id,
                command: None,
                text: None,
                outcome: Some(outcome.to_string()),
            },
        )
    }

    fn append(&self, session: &str, line: &Line) -> std::io::Result<()> {
        let path = self.path(session).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "an unsafe session id")
        })?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)?;
        std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)?;
        let mut text = serde_json::to_string(line).map_err(std::io::Error::other)?;
        text.push('\n');
        file.write_all(text.as_bytes())
    }
}

/// The time now, RFC 3339 in UTC, to the second.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_journal_records_asks_and_outcomes_privately() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::new(dir.path().join("state").join("sessions"));
        let a = journal
            .asked("s1", 12, 7, "/compact", "Approve /compact?")
            .unwrap()
            .unwrap();
        assert_eq!((a.line, a.slot), (12, SLOT_APPROVAL_FIRST));
        // A second on the same line takes the next slot; the same id is
        // not recorded twice.
        let b = journal
            .asked("s1", 12, 8, "continue", "Go?")
            .unwrap()
            .unwrap();
        assert_eq!(b.slot, SLOT_APPROVAL_FIRST + 1);
        assert_eq!(journal.asked("s1", 13, 7, "x", "y").unwrap(), None);
        journal.resolved("s1", 7, "approved").unwrap();
        let read = journal.read("s1");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].outcome.as_deref(), Some("approved"));
        assert_eq!(read[1].outcome, None);
        assert!(read[1].card(true).live && !read[0].card(true).live);
        assert!(!read[1].card(false).live);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(journal.dir()), 0o700);
        assert_eq!(mode(&journal.path("s1").unwrap()), 0o600);
        // Unsafe ids are refused; a missing journal reads empty.
        assert_eq!(journal.path("../x"), None);
        assert!(journal.read("nope").is_empty());
        assert!(journal.asked("../x", 1, 1, "c", "t").is_err());
    }

    #[test]
    fn the_time_is_rfc_3339() {
        let t = now_rfc3339();
        assert_eq!(t.len(), 20, "{t}");
        assert!(t.starts_with("20") && t.ends_with('Z') && t.as_bytes()[10] == b'T');
    }
}
