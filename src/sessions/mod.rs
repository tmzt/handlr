//! Agents' own session transcripts (docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md,
//! decisions 2, 3, 7 and 8, with the 16:23 and 02:28 EDT amendments).
//!
//! herdr holds no scrollback for a full-screen agent such as Claude Code,
//! so what an agent said and is waiting on is read from its own transcript
//! file. A session is found by the id herdr reports for the pane
//! (`agent_session`), else by the pane's cwd. Files are read from the end,
//! never whole, and only on the machine the daemon runs on.
//!
//! Everything that leaves this module for a client or for Gemini goes
//! through [`redact::redact`] first: [`Item::entry`], [`pending`],
//! [`turn`] and [`context`] all redact.

pub mod claude;
pub mod codex;
pub mod context;
pub mod links;
pub mod pending;
pub mod redact;
pub mod turn;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub use libhandlr_proto::common::{
    ENTRY_SUMMARY_MAX, Entry, EntryKind, PENDING_QUESTION_MAX, PendingQuestion, Role,
};

use crate::core::agents::AgentRef;

/// How much of a file `tail` reads at a time, from the end.
pub const TAIL_CHUNK: u64 = 256 * 1024;
/// The most conversation `tail` reads: the bytes of the lines that are
/// not bookkeeping ([`SessionReader::is_bookkeeping`]).
pub const TAIL_MAX: u64 = 2 * 1024 * 1024;
/// The most of a file `tail` reads, bookkeeping and all.
pub const TAIL_READ_MAX: u64 = 32 * 1024 * 1024;
/// A tool call's or result's text is kept to this many characters as it is
/// read; summaries are cut further after redaction.
pub(crate) const TOOL_TEXT_MAX: usize = 4000;
/// A text entry is kept to this many characters as it is read.
pub(crate) const TEXT_MAX: usize = 32 * 1024;

/// One entry as it was read, before it is redacted and cut for a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// RFC 3339, from the file; empty when the line had none.
    pub ts: String,
    pub role: Role,
    pub kind: EntryKind,
    pub tool: Option<String>,
    /// The whole text of a `Text` entry (up to 32 KiB); a tool call's name
    /// and input, or a result's text, up to 4000 characters. Not redacted.
    pub text: String,
    /// The id linking a tool call and its result.
    pub call_id: Option<String>,
    /// Where an entry that is context, not the user's own words nor the
    /// agent's, comes from (`Entry::origin`): `task`, `peer:<name>`,
    /// `agent:<id>`, `recap` or `compact`. Such an entry is never the
    /// user speaking.
    pub origin: Option<String>,
}

impl Item {
    /// The entry a client sees: redacted, then cut to `ENTRY_SUMMARY_MAX`.
    pub fn entry(&self) -> Entry {
        Entry {
            ts: self.ts.clone(),
            role: self.role,
            kind: self.kind,
            tool: self.tool.clone(),
            summary: cut(&redact::redact(&self.text), ENTRY_SUMMARY_MAX),
            links: Vec::new(),
            origin: self.origin.clone(),
        }
    }

    /// Whether this is the user's own words: a user text entry with no
    /// origin.
    pub fn is_user_said(&self) -> bool {
        self.role == Role::User && self.kind == EntryKind::Text && self.origin.is_none()
    }

    pub fn is_text(&self) -> bool {
        self.kind == EntryKind::Text
    }
}

/// `s` cut to at most `max` characters, at a character boundary, ending in
/// "…" when it was cut.
pub fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some(_) => {
            let keep = max.saturating_sub(1);
            let end = s.char_indices().nth(keep).map_or(s.len(), |(i, _)| i);
            format!("{}…", &s[..end])
        }
    }
}

/// How a session was found (16:23 EDT amendment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Matched {
    /// By the session id herdr reports for the pane.
    Id,
    /// By the pane's cwd: the newest session there, which may belong to
    /// another agent in the same directory.
    Cwd,
}

/// A session file found for a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub path: PathBuf,
    /// The session's id, where it is known.
    pub session_id: Option<String>,
    pub matched: Matched,
}

impl Located {
    /// Whether the session file has been unwritten for
    /// [`pending::TURN_QUIET`], so a final reply ended the turn whatever
    /// herdr's status.
    pub fn quiet(&self) -> bool {
        std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= pending::TURN_QUIET)
    }
}

/// One agent's session format (decision 3).
pub trait SessionReader: Send + Sync {
    /// `claude_code` or `codex`.
    fn kind(&self) -> &'static str;

    /// The session for a pane: by `session_id` when herdr gives one, else
    /// the newest session for `cwd`.
    fn locate(&self, cwd: Option<&Path>, session_id: Option<&str>) -> Option<Located>;

    /// The entries one line holds. Unknown and malformed lines give none.
    fn parse_line(&self, line: &str) -> Vec<Item>;

    /// Whether a line is bookkeeping that holds no entry, known cheaply
    /// from its first bytes: such a line is neither parsed nor counted
    /// toward `TAIL_MAX`. `false` unless the reader knows its format's.
    fn is_bookkeeping(&self, _line: &[u8]) -> bool {
        false
    }

    /// The last `n` entries of the file at `path`.
    fn tail(&self, path: &Path, n: usize) -> Result<Vec<Item>, String> {
        let mut items = read_tail(self, path, |items| items.len() >= n)?.items;
        let skip = items.len().saturating_sub(n);
        items.drain(..skip);
        Ok(items)
    }
}

/// What [`read_tail`] read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tail {
    /// Every entry in the whole lines read, oldest first.
    pub items: Vec<Item>,
    /// How many bytes of the file were read.
    pub bytes_read: u64,
}

/// Read entries from the end of the file at `path`: its last `TAIL_CHUNK`
/// bytes, then `TAIL_CHUNK` more at a time until `enough` holds, the file's
/// start is reached, `TAIL_MAX` bytes of conversation have been read, or
/// `TAIL_READ_MAX` bytes in all. Lines the reader knows as bookkeeping are
/// neither parsed nor counted as conversation. Only whole lines are
/// parsed, each once.
pub fn read_tail<R: SessionReader + ?Sized>(
    reader: &R,
    path: &Path,
    enough: impl Fn(&[Item]) -> bool,
) -> Result<Tail, String> {
    let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let len = file
        .metadata()
        .map_err(|e| format!("stat {}: {e}", path.display()))?
        .len();
    let mut start = len;
    // The start of a line cut at `start`, not yet parsed.
    let mut carry: Vec<u8> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    let mut conversation: u64 = 0;
    loop {
        let next = start
            .saturating_sub(TAIL_CHUNK)
            .max(len.saturating_sub(TAIL_READ_MAX));
        let mut chunk = vec![0u8; (start - next) as usize];
        file.seek(SeekFrom::Start(next))
            .and_then(|_| file.read_exact(&mut chunk))
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        chunk.extend_from_slice(&carry);
        start = next;
        // Whole lines: all of it at the file's start, else what follows
        // its first line break.
        let whole = if start == 0 {
            0
        } else {
            chunk
                .iter()
                .position(|&b| b == b'\n')
                .map_or(chunk.len(), |i| i + 1)
        };
        let (mut fresh, bytes) = parse_lines(reader, &chunk[whole..]);
        chunk.truncate(whole);
        carry = chunk;
        conversation += bytes;
        if !fresh.is_empty() {
            fresh.append(&mut items);
            items = fresh;
            link_results(&mut items);
        }
        let at_limit = start == 0 || conversation >= TAIL_MAX || len - start >= TAIL_READ_MAX;
        if at_limit || enough(&items) {
            return Ok(Tail {
                items,
                bytes_read: len - start,
            });
        }
    }
}

/// The entries in `bytes`' lines, and how many bytes of them are
/// conversation (not bookkeeping).
fn parse_lines<R: SessionReader + ?Sized>(reader: &R, bytes: &[u8]) -> (Vec<Item>, u64) {
    let mut conversation = 0u64;
    let items = bytes
        .split(|&b| b == b'\n')
        .map(<[u8]>::trim_ascii)
        .filter(|l| !l.is_empty() && !reader.is_bookkeeping(l))
        .flat_map(|l| {
            conversation += l.len() as u64 + 1;
            reader.parse_line(&String::from_utf8_lossy(l))
        })
        .collect();
    (items, conversation)
}

/// Give each tool result its call's tool name, and redact a result whose
/// call names a path under `.private/` or a `.env` file whole: its text is
/// that file's contents.
fn link_results(items: &mut [Item]) {
    let calls: HashMap<String, (Option<String>, bool)> = items
        .iter()
        .filter(|i| i.kind == EntryKind::ToolUse)
        .filter_map(|i| {
            let id = i.call_id.clone()?;
            Some((id, (i.tool.clone(), redact::mentions_secret_path(&i.text))))
        })
        .collect();
    for item in items.iter_mut().filter(|i| i.kind == EntryKind::ToolResult) {
        let Some((tool, secret)) = item.call_id.as_ref().and_then(|id| calls.get(id)) else {
            continue;
        };
        if item.tool.is_none() {
            item.tool = tool.clone();
        }
        if *secret {
            item.text = redact::REDACTED.to_string();
        }
    }
}

/// A pane's session, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRead {
    /// The reader's kind (`claude_code`, `codex`).
    pub kind: &'static str,
    pub located: Located,
    pub items: Vec<Item>,
}

/// The session readers, by agent name (decision 3).
pub struct Sessions {
    pub claude: claude::ClaudeCode,
    pub codex: codex::Codex,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Sessions {
    /// The readers for the files under `$HOME`.
    pub fn new() -> Self {
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
        Self::with_home(&home)
    }

    /// The readers for the files under `home` (`.claude/projects`,
    /// `.codex/sessions`).
    pub fn with_home(home: &Path) -> Self {
        Sessions {
            claude: claude::ClaudeCode::new(home.join(".claude").join("projects")),
            codex: codex::Codex::new(home.join(".codex").join("sessions")),
        }
    }

    /// The reader for an agent: `claude` is Claude Code, `codex` is Codex;
    /// any other agent has none.
    pub fn reader_for(&self, agent: &str) -> Option<&dyn SessionReader> {
        match agent {
            "claude" => Some(&self.claude),
            "codex" => Some(&self.codex),
            _ => None,
        }
    }

    /// The session of the agent in `pane`, if it has a reader and a file.
    pub fn locate(&self, pane: &AgentRef) -> Option<(&dyn SessionReader, Located)> {
        let reader = self.reader_for(&pane.agent)?;
        let id = pane.session.as_ref().and_then(|s| s.id());
        let cwd = pane.cwd.as_deref().map(Path::new);
        let located = reader.locate(cwd, id)?;
        Some((reader, located))
    }

    /// Read the session of the agent in `pane` from the end until `enough`
    /// holds (within `read_tail`'s limits).
    pub fn read(&self, pane: &AgentRef, enough: impl Fn(&[Item]) -> bool) -> Option<SessionRead> {
        let (reader, located) = self.locate(pane)?;
        match read_tail(reader, &located.path, enough) {
            Ok(tail) => Some(SessionRead {
                kind: reader.kind(),
                located,
                items: tail.items,
            }),
            Err(e) => {
                log::warn!("Could not read the {} session: {e}", reader.kind());
                None
            }
        }
    }
}

/// A pane's session as `session.peek` and `agents.list` report it (plan
/// 20261022_03, step 4): redacted entries, the newest last, and the
/// pending question for the pane's status, and where its turn stands
/// (plan 20261023_04, A4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peeked {
    pub located: Located,
    pub entries: Vec<Entry>,
    pub pending_question: Option<PendingQuestion>,
    pub turn: turn::Turn,
}

impl Peeked {
    /// Give the entries and the pending question their links (the 21:57
    /// EDT amendment): web links always, and repowiki links when
    /// `[urls] repowiki_url` is set and the agent's directory has a site.
    pub fn link(&mut self, repowiki: Option<&str>, cwd: Option<&str>) {
        let linker = links::Linker::new(repowiki, cwd);
        for entry in &mut self.entries {
            linker.link_entry(entry);
        }
        if let Some(q) = &mut self.pending_question {
            linker.link_question(q);
        }
    }
}

impl Sessions {
    /// The last `n` entries of the agent in `pane`, its pending question
    /// and its turn. Reads back until there are `n` entries and an assistant
    /// reply among them, within `read_tail`'s limits.
    pub fn peek(&self, pane: &AgentRef, n: usize) -> Option<Peeked> {
        let read = self.read(pane, |items| {
            items.len() >= n
                && items
                    .iter()
                    .any(|i| i.is_text() && i.role == Role::Assistant)
        })?;
        let quiet = read.located.quiet();
        let pending_question = pending::pending_question(&read.items, &pane.status, quiet);
        let turn = turn::turn(&read.items, &pane.status, quiet);
        let skip = read.items.len().saturating_sub(n);
        Some(Peeked {
            located: read.located,
            entries: read.items[skip..].iter().map(Item::entry).collect(),
            pending_question,
            turn,
        })
    }
}

impl Sessions {
    /// The whole text of the agent's reply at `ts` in `pane`'s session,
    /// redacted, for `speech.summarize` (plan 20261023_05, decision 3).
    /// The text entries of one reply share its time and are joined. Reads
    /// back until the reply is found, within `read_tail`'s limits; `None`
    /// when it is not there.
    pub fn reply_text(&self, pane: &AgentRef, ts: &str) -> Option<String> {
        let is_reply = |i: &Item| i.is_text() && i.role == Role::Assistant && i.ts == ts;
        if ts.is_empty() {
            return None;
        }
        let read = self.read(pane, |items| items.iter().any(is_reply))?;
        let parts: Vec<&str> = read
            .items
            .iter()
            .filter(|i| is_reply(i))
            .map(|i| i.text.as_str())
            .collect();
        if parts.is_empty() {
            return None;
        }
        Some(cut(&redact::redact(&parts.join("\n\n")), TAIL_MAX as usize))
    }
}

impl From<Matched> for libhandlr_proto::common::SessionMatched {
    fn from(m: Matched) -> Self {
        match m {
            Matched::Id => Self::Id,
            Matched::Cwd => Self::Cwd,
        }
    }
}

/// Whether a session id can be used in a file name as given.
pub(crate) fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The first line of the file at `path`, read up to `max` bytes.
pub(crate) fn first_line(path: &Path, max: usize) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let mut filled = 0;
    while filled < max {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => {
                filled += n;
                if buf[..filled].contains(&b'\n') {
                    break;
                }
            }
            Err(_) => return None,
        }
    }
    let text = String::from_utf8_lossy(&buf[..filled]);
    text.split('\n').next().map(str::to_string)
}

/// The `*.jsonl` files directly in `dir`, newest modification first.
pub(crate) fn jsonl_newest_first(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl") && p.is_file())
        .map(|p| {
            let modified = p
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (modified, p)
        })
        .collect();
    files.sort_by(|a, b| b.cmp(a));
    files.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    /// `tests/fixtures/sessions/<name>`.
    pub fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("sessions")
            .join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn cut_keeps_short_text_and_cuts_at_a_char_boundary() {
        assert_eq!(cut("hello", 5), "hello");
        assert_eq!(cut("hello!", 5), "hell…");
        assert_eq!(cut("ééééé", 3), "éé…");
        assert_eq!(cut("", 3), "");
        let long = "x".repeat(600);
        assert_eq!(cut(&long, ENTRY_SUMMARY_MAX).chars().count(), 500);
    }

    #[test]
    fn a_large_file_is_read_from_the_end_only() {
        // 50 MiB of user lines, then 10 assistant replies at the end.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.jsonl");
        let mut file = std::io::BufWriter::new(File::create(&path).unwrap());
        let filler = format!(
            "{{\"type\":\"user\",\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"message\":{{\"role\":\"user\",\"content\":\"{}\"}}}}\n",
            "filler ".repeat(140)
        );
        let mut written = 0usize;
        while written < 50 * 1024 * 1024 {
            file.write_all(filler.as_bytes()).unwrap();
            written += filler.len();
        }
        for i in 0..10 {
            writeln!(
                file,
                "{{\"type\":\"assistant\",\"timestamp\":\"2026-01-01T00:00:{i:02}.000Z\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"reply {i}\"}}]}}}}"
            )
            .unwrap();
        }
        drop(file);
        assert!(std::fs::metadata(&path).unwrap().len() >= 50 * 1024 * 1024);

        let reader = claude::ClaudeCode::new(dir.path().to_path_buf());
        let tail = read_tail(&reader, &path, |items| items.len() >= 10).unwrap();
        assert!(tail.bytes_read <= 512 * 1024, "read {}", tail.bytes_read);
        let last: Vec<String> = reader
            .tail(&path, 10)
            .unwrap()
            .into_iter()
            .map(|i| i.text)
            .collect();
        let want: Vec<String> = (0..10).map(|i| format!("reply {i}")).collect();
        assert_eq!(last, want);
    }

    #[test]
    fn reading_back_stops_at_two_mebibytes_of_conversation() {
        // Conversation lines that give no entry: nothing is ever enough.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noise.jsonl");
        let line = format!(
            "{{\"parentUuid\":\"u\",\"isSidechain\":false,\"type\":\"user\",\"isMeta\":true,\"message\":{{\"content\":\"{}\"}}}}\n",
            "z".repeat(1000)
        );
        let body = line.repeat(5 * 1024);
        std::fs::write(&path, body).unwrap();
        let reader = claude::ClaudeCode::new(dir.path().to_path_buf());
        let tail = read_tail(&reader, &path, |_| false).unwrap();
        assert!(tail.items.is_empty());
        assert!(
            (TAIL_MAX..=TAIL_MAX + TAIL_CHUNK).contains(&tail.bytes_read),
            "read {}",
            tail.bytes_read
        );
    }

    #[test]
    fn bookkeeping_does_not_count_but_reading_stops_at_thirty_two_mebibytes() {
        // Two replies, then 40 MiB of blob lines.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blobs.jsonl");
        let mut file = std::io::BufWriter::new(File::create(&path).unwrap());
        for i in 0..2 {
            writeln!(
                file,
                "{{\"parentUuid\":\"u\",\"isSidechain\":false,\"message\":{{\"content\":\"reply {i}\"}},\"type\":\"assistant\",\"timestamp\":\"t{i}\"}}"
            )
            .unwrap();
        }
        let blob = format!(
            "{{\"type\":\"api-request-blob\",\"hash\":\"h\",\"message\":\"{}\"}}\n",
            "b".repeat(64 * 1024)
        );
        let mut written = 0usize;
        while written < 40 * 1024 * 1024 {
            file.write_all(blob.as_bytes()).unwrap();
            written += blob.len();
        }
        drop(file);
        let reader = claude::ClaudeCode::new(dir.path().to_path_buf());
        let tail = read_tail(&reader, &path, |_| false).unwrap();
        assert!(tail.items.is_empty(), "the replies are past the cap");
        assert_eq!(tail.bytes_read, TAIL_READ_MAX);
        // 20 MiB of blobs: read through to the replies.
        let short = dir.path().join("short.jsonl");
        let body = std::fs::read(&path).unwrap();
        std::fs::write(&short, &body[..20 * 1024 * 1024 + 300]).unwrap();
        let texts: Vec<String> = reader
            .tail(&short, 5)
            .unwrap()
            .into_iter()
            .map(|i| i.text)
            .collect();
        assert_eq!(texts, ["reply 0", "reply 1"]);
    }
}
