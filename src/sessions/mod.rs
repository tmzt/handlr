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

pub mod agy;
pub mod children;
pub mod choices;
pub mod claude;
pub mod codex;
pub mod context;
pub mod hub;
pub mod journal;
pub mod links;
pub mod pending;
pub mod proto_wire;
pub mod redact;
pub mod sections;
pub mod turn;

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read as _, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

pub use libflankr_proto::common::{
    ChoiceSet, ENTRY_SUMMARY_MAX, Entry, EntryKind, PENDING_QUESTION_MAX, PendingQuestion, Role,
};

pub use children::Child;

use crate::core::agents::AgentRef;
use crate::suggestions::{SuggestionStore, Who};

/// How much of a file `tail` reads at a time, from the end.
pub const TAIL_CHUNK: u64 = 256 * 1024;
/// The most conversation `tail` reads: the bytes of the lines that are
/// not bookkeeping ([`SessionReader::is_bookkeeping`]).
pub const TAIL_MAX: u64 = 2 * 1024 * 1024;
/// The most of a file `tail` reads, bookkeeping and all.
pub const TAIL_READ_MAX: u64 = 32 * 1024 * 1024;
/// The most one line counts toward `TAIL_MAX`. A tool result holding an
/// image is most of a megabyte, of which the reader keeps a few thousand
/// characters; counted whole, five of them filled the budget.
pub const LINE_BUDGET_MAX: u64 = 32 * 1024;
/// A tool call's or result's text is kept to this many characters as it is
/// read; summaries are cut further after redaction.
pub(crate) const TOOL_TEXT_MAX: usize = 4000;
/// A text entry is kept to this many characters as it is read.
pub(crate) const TEXT_MAX: usize = 32 * 1024;

/// One entry as it was read, before it is redacted and cut for a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The record's position in its session: a JSONL line's number
    /// (1-based), or agy's `steps.idx` (0-based). Stable across reads
    /// (plan 20261023_06, decision 2).
    pub pos: u64,
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
    /// A tool call that asks a choice question (plan 20261029_01,
    /// decision 4): its questions and options, not redacted. `None` from
    /// every reader but Claude Code's and on every other line.
    pub(crate) choices: Option<ChoiceSet>,
    /// A tool result that answers a choice question, as the reader read
    /// it. Only meaningful for a result whose `call_id` is a call with
    /// `choices`.
    pub(crate) choice_result: Option<ChoiceOutcome>,
}

/// How a choice question was answered, as its tool result recorded it
/// (plan 20261029_01, decision 4). Not redacted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChoiceOutcome {
    /// Each question's text and its answer, from a structured result
    /// (Claude Code: `toolUseResult.answers`), in the order asked.
    pub answers: Vec<(String, String)>,
    /// The result's text, when there was no structured result: its
    /// `"<question>"="<answer>"` pairs are found once the questions are
    /// known ([`ChoiceOutcome::answers_for`]).
    pub content: Option<String>,
    /// The call was rejected (Esc, Cancel on the review, or "Chat about
    /// this"): no answers.
    pub rejected: bool,
}

impl ChoiceOutcome {
    /// One answer per question of `set`, in order: by its text in
    /// `answers`, else from `content`'s pairs. `None` where none was
    /// recorded, and for every question when rejected.
    pub fn answers_for(&self, set: &ChoiceSet) -> Vec<Option<String>> {
        if self.rejected {
            return vec![None; set.questions.len()];
        }
        if !self.answers.is_empty() {
            return set
                .questions
                .iter()
                .map(|q| {
                    self.answers
                        .iter()
                        .find(|(text, _)| *text == q.text)
                        .map(|(_, a)| a.clone())
                })
                .collect();
        }
        let Some(content) = &self.content else {
            return vec![None; set.questions.len()];
        };
        let texts: Vec<&str> = set.questions.iter().map(|q| q.text.as_str()).collect();
        content_pairs(content, &texts)
    }
}

/// The answer to each of `questions` in a result text of the form
/// `… "<q1>"="<a1>", "<q2>"="<a2>". …`, a preview answer followed by
/// ` selected preview:\n<preview>` (plan 20261029_01, finding 1). Each
/// answer runs to the next question's pair, or, for the last found, to
/// the closing `"` before `. ` or the preview note.
fn content_pairs(content: &str, questions: &[&str]) -> Vec<Option<String>> {
    let starts: Vec<Option<(usize, usize)>> = questions
        .iter()
        .map(|q| {
            let key = format!("\"{q}\"=\"");
            content.find(&key).map(|at| (at, at + key.len()))
        })
        .collect();
    starts
        .iter()
        .map(|found| {
            let (_, from) = (*found)?;
            // The end: the nearest start of another found pair after this
            // one, else the content's end.
            let next = starts
                .iter()
                .flatten()
                .map(|(at, _)| *at)
                .filter(|at| *at > from)
                .min()
                .unwrap_or(content.len());
            let mut body = &content[from..next];
            if let Some(at) = body.find("\" selected preview:") {
                body = &body[..at + 1];
            }
            if next == content.len() {
                // The last pair: `"<a>". You can now continue …`, or
                // `"<a>". Read the answers carefully …`.
                let end = ["\". You can now", "\". Read the answers"]
                    .iter()
                    .find_map(|m| body.find(m))
                    .or_else(|| body.rfind("\"."));
                if let Some(at) = end {
                    body = &body[..at + 1];
                }
            }
            let body = body.trim_end();
            let body = body.strip_suffix(',').unwrap_or(body);
            let body = body.strip_suffix('"').unwrap_or(body);
            Some(body.to_string())
        })
        .collect()
}

impl Item {
    /// The entry a client sees: redacted, its `summary` cut to
    /// `ENTRY_SUMMARY_MAX`, and its `text` the whole redacted text (up to
    /// the reader's `TEXT_MAX` or `TOOL_TEXT_MAX`) when the summary is cut
    /// (plan 20261022_03, the L3 amendment of 2026-10-24 11:54 EDT). Left
    /// out when the summary is the whole text already.
    pub fn entry(&self) -> Entry {
        let full = redact::redact(&self.text);
        let summary = cut(&full, ENTRY_SUMMARY_MAX);
        let text = (summary != full).then_some(full);
        Entry {
            ts: self.ts.clone(),
            role: self.role,
            kind: self.kind,
            tool: self.tool.clone(),
            summary,
            links: Vec::new(),
            origin: self.origin.clone(),
            text,
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

/// Parse an exit code from command execution output or task summaries, such as
/// `Command exited with code <N>` or `(exit code <N>)`.
pub fn parse_command_exit_code(text: &str) -> Option<i32> {
    if let Some(pos) = text.find("Command exited with code ") {
        let after = &text[pos + "Command exited with code ".len()..];
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num_str.parse::<i32>() {
            return Some(code);
        }
    }
    if let Some(pos) = text.find("(exit code ") {
        let after = &text[pos + "(exit code ".len()..];
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num_str.parse::<i32>() {
            return Some(code);
        }
    }
    None
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
        self.modified()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= pending::TURN_QUIET)
    }

    /// When the session was last modified: a file's modification time for
    /// JSONL, the newer of the database's and its `-wal`'s for SQLite
    /// (plan 20261023_06, decision 1).
    pub fn modified(&self) -> Option<SystemTime> {
        let db = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        let mut wal_path = self.path.clone().into_os_string();
        wal_path.push("-wal");
        let wal = std::fs::metadata(wal_path).and_then(|m| m.modified()).ok();
        match (db, wal) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }
}

/// How long after its last write a subagent not marked finished counts as
/// running (plan 20261028_03, 3.6: `SUBAGENT_LIVE`).
pub const SUBAGENT_LIVE: Duration = Duration::from_secs(60);

/// A session's subagents, as [`SessionReader::subagents`] reads them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Subagents {
    /// Those not marked finished whose transcripts were written within
    /// [`SUBAGENT_LIVE`].
    pub running: u32,
    /// When any subagent's transcript was last written.
    pub newest: Option<SystemTime>,
}

/// A session's activity as `agents.list` reports it (plan 20261028_03,
/// 3.12): `AgentInfo::idle_ms` and `AgentInfo::subagents_running`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    /// Milliseconds from the newest write of the session or of any of its
    /// subagents to `now`; 0 for a write after `now`.
    pub idle_ms: Option<u64>,
    pub subagents_running: Option<u32>,
}

/// The activity of the session `at`, read by `reader`, as of `now`.
pub fn activity(reader: &dyn SessionReader, at: &Located, now: SystemTime) -> Activity {
    let subagents = reader.subagents(at, now);
    let newest = reader
        .modified(at)
        .into_iter()
        .chain(subagents.and_then(|s| s.newest))
        .max();
    Activity {
        idle_ms: newest.map(|t| {
            let age = now.duration_since(t).unwrap_or_default();
            u64::try_from(age.as_millis()).unwrap_or(u64::MAX)
        }),
        subagents_running: subagents.map(|s| s.running),
    }
}

/// One agent's session format (decision 3; plan 20261023_06, decision 1).
pub trait SessionReader: Send + Sync {
    /// `claude_code`, `codex`, `agy`.
    fn kind(&self) -> &'static str;

    /// The session for a pane: by `session_id` when herdr gives one, else
    /// the newest session for `cwd`.
    fn locate(&self, cwd: Option<&Path>, session_id: Option<&str>) -> Option<Located>;

    /// Records from the newest back, oldest first in the result, until
    /// `enough` says so or the reader's limits are reached.
    fn read_back(&self, at: &Located, enough: &dyn Fn(&[Item]) -> bool) -> Result<Read, String>;

    /// When the session last changed, for [`Located::quiet`].
    fn modified(&self, at: &Located) -> Option<SystemTime> {
        at.modified()
    }

    /// The session's subagents as of `now`, for the drawer's activity
    /// (plan 20261028_03, 3.12 item 47). `None` when the reader does not
    /// read subagents; only Claude Code's does.
    fn subagents(&self, _at: &Located, _now: SystemTime) -> Option<Subagents> {
        None
    }

    /// The sessions started from `at`, newest first (the subagents plan,
    /// decision 1): nested ones listed flat with the rest (the 17:49 EDT
    /// amendment), finished ones for the session's life. None by default;
    /// only Claude Code's reader finds them.
    fn children(&self, _at: &Located) -> Vec<Child> {
        Vec::new()
    }

    /// The last `n` entries of the file at `path`.
    fn tail(&self, path: &Path, n: usize) -> Result<Vec<Item>, String> {
        let at = Located {
            path: path.to_path_buf(),
            session_id: None,
            matched: Matched::Id,
        };
        let mut items = self.read_back(&at, &|items| items.len() >= n)?.items;
        let skip = items.len().saturating_sub(n);
        items.drain(..skip);
        Ok(items)
    }
}

/// Helper trait for line-based JSONL session readers (plan 20261023_06,
/// decision 1).
pub trait LineReader: Send + Sync {
    /// The entries one line holds. Unknown and malformed lines give none.
    fn parse_line(&self, line: &str) -> Vec<Item>;

    /// Whether a line is bookkeeping that holds no entry, known cheaply
    /// from its first bytes.
    fn is_bookkeeping(&self, _line: &[u8]) -> bool {
        false
    }
}

/// What [`SessionReader::read_back`] read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    /// Every entry in the whole lines or records read, oldest first.
    pub items: Vec<Item>,
    /// How many bytes of the file were read.
    pub bytes_read: u64,
}

pub type Tail = Read;

/// Read entries from the end of the file at `path`: its last `TAIL_CHUNK`
/// bytes, then `TAIL_CHUNK` more at a time until `enough` holds, the file's
/// start is reached, `TAIL_MAX` bytes of conversation have been read, or
/// `TAIL_READ_MAX` bytes in all. Lines the reader knows as bookkeeping are
/// neither parsed nor counted as conversation. Only whole lines are
/// parsed, each once. Sets each item's `pos` to its 1-based line number.
pub fn read_tail<R: LineReader + ?Sized>(
    reader: &R,
    path: &Path,
    enough: impl Fn(&[Item]) -> bool,
) -> Result<Read, String> {
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
    let mut earliest_start;
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
        earliest_start = start + whole as u64;
        let (mut fresh, bytes, line_count) = parse_lines(reader, &chunk[whole..]);
        chunk.truncate(whole);
        carry = chunk;
        conversation += bytes;
        for item in &mut items {
            item.pos += line_count;
        }
        if !fresh.is_empty() {
            fresh.append(&mut items);
            items = fresh;
            link_results(&mut items);
        }
        let at_limit = start == 0 || conversation >= TAIL_MAX || len - start >= TAIL_READ_MAX;
        if at_limit || enough(&items) {
            break;
        }
    }

    let base_lines = if earliest_start == 0 {
        0
    } else {
        count_newlines(&mut file, earliest_start)
            .map_err(|e| format!("count lines {}: {e}", path.display()))?
    };

    if base_lines > 0 {
        for item in &mut items {
            item.pos += base_lines;
        }
    }

    Ok(Read {
        items,
        bytes_read: len - start,
    })
}

fn count_newlines(file: &mut File, up_to: u64) -> std::io::Result<u64> {
    file.seek(SeekFrom::Start(0))?;
    let mut count = 0u64;
    let mut buf = [0u8; 64 * 1024];
    let mut remaining = up_to;
    while remaining > 0 {
        let to_read = (remaining as usize).min(buf.len());
        let n = file.read(&mut buf[..to_read])?;
        if n == 0 {
            break;
        }
        count += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64;
        remaining -= n as u64;
    }
    Ok(count)
}

/// The entries in `bytes`' lines, how many bytes of them are
/// conversation (not bookkeeping), and how many total lines.
fn parse_lines<R: LineReader + ?Sized>(reader: &R, bytes: &[u8]) -> (Vec<Item>, u64, u64) {
    let mut conversation = 0u64;
    let mut line_no = 0u64;
    let mut items = Vec::new();
    let slice = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if slice.is_empty() {
        return (items, conversation, 0);
    }
    for l in slice.split(|&b| b == b'\n') {
        line_no += 1;
        let trimmed = l.trim_ascii();
        if trimmed.is_empty() || reader.is_bookkeeping(trimmed) {
            continue;
        }
        conversation += (trimmed.len() as u64 + 1).min(LINE_BUDGET_MAX);
        let s = String::from_utf8_lossy(trimmed);
        for mut item in reader.parse_line(&s) {
            item.pos = line_no;
            items.push(item);
        }
    }
    (items, conversation, line_no)
}

/// Give each tool result its call's tool name, and redact a result whose
/// call names a path under `.private/` or a `.env` file whole: its text is
/// that file's contents.
pub(crate) fn link_results(items: &mut [Item]) {
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

/// The session readers, by agent name (decision 3; plan 20261023_06, decision 4).
pub struct Sessions {
    pub claude: claude::ClaudeCode,
    pub codex: codex::Codex,
    pub agy: agy::AgyReader,
    /// Where the approval journals are kept (plan 20261023_03, A4):
    /// `.local/state/flankr/sessions` under the home.
    pub state: PathBuf,
    /// The rules that give a pending question its answers (plan
    /// 20261027_06): the shipped ones until the daemon names its files.
    pub suggestions: Arc<SuggestionStore>,
    /// Subagents' turns as last read, by transcript, with the transcript's
    /// modification time and length and whether it was finished then: a
    /// list reads only the transcripts that changed.
    child_turns: Mutex<HashMap<PathBuf, ChildTurn>>,
}

/// A subagent's turn as read from its transcript at a modification time
/// and length, finished or not.
type ChildTurn = (Option<SystemTime>, u64, bool, turn::Turn);

/// The most subagents' turns [`Sessions`] keeps.
const CHILD_TURNS_MAX: usize = 512;

/// A subagent named that the pane's session does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownChild;

/// The status a subagent's session is read with (the subagents plan,
/// decision 1): herdr's status is its parent's, so `done` once it is
/// finished, else `working`.
pub fn child_status(child: &Child) -> libherdr::protocol::AgentStatus {
    if child.finished {
        libherdr::protocol::AgentStatus::Done
    } else {
        libherdr::protocol::AgentStatus::Working
    }
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
    /// `.codex/sessions`, `.gemini/antigravity-cli/conversations`), with the
    /// approval journals in `.local/state/flankr/sessions` under it.
    pub fn with_home(home: &Path) -> Self {
        Sessions {
            claude: claude::ClaudeCode::new(home.join(".claude").join("projects")),
            codex: codex::Codex::new(home.join(".codex").join("sessions")),
            agy: agy::AgyReader::new(
                home.join(".gemini")
                    .join("antigravity-cli")
                    .join("conversations"),
            ),
            state: home
                .join(".local")
                .join("state")
                .join("flankr")
                .join("sessions"),
            suggestions: Arc::new(SuggestionStore::shipped()),
            child_turns: Mutex::new(HashMap::new()),
        }
    }

    /// The reader for an agent: `claude` is Claude Code, `codex` is Codex,
    /// `agy` is Antigravity CLI; any other agent has none.
    pub fn reader_for(&self, agent: &str) -> Option<&dyn SessionReader> {
        match agent {
            "claude" => Some(&self.claude),
            "codex" => Some(&self.codex),
            "agy" => Some(&self.agy),
            _ => None,
        }
    }

    /// The line-based reader for an agent, if any.
    pub fn line_reader_for(&self, agent: &str) -> Option<&dyn LineReader> {
        match agent {
            "claude" => Some(&self.claude),
            "codex" => Some(&self.codex),
            _ => None,
        }
    }

    /// The line-based reader for the file at `path` of an agent: a Claude
    /// Code subagent's transcript has its own (the subagents plan,
    /// decision 1), every other file its agent's.
    pub fn line_reader_at(&self, agent: &str, path: &Path) -> Option<&dyn LineReader> {
        if agent == "claude" && claude::is_child_path(path) {
            return Some(&claude::CHILD_READER);
        }
        self.line_reader_for(agent)
    }

    /// The subagent `id` of the session of the agent in `pane`, if its
    /// session has one by that id (the subagents plan, decision 2).
    pub fn child(&self, pane: &AgentRef, id: &str) -> Option<(&dyn SessionReader, Child)> {
        let (reader, located) = self.locate(pane)?;
        let child = reader.children(&located).into_iter().find(|c| c.id == id)?;
        Some((reader, child))
    }

    /// The session to read for `pane`, or for its subagent `child`: its
    /// reader, where it is, and the pane as it is read, a subagent's with
    /// its own status ([`child_status`]). `Ok(None)` when the pane has no
    /// session here; [`UnknownChild`] when it has no such subagent.
    #[allow(clippy::type_complexity)]
    pub fn locate_in(
        &self,
        pane: &AgentRef,
        child: Option<&str>,
    ) -> Result<Option<(&dyn SessionReader, Located, AgentRef)>, UnknownChild> {
        let Some(id) = child else {
            return Ok(self
                .locate(pane)
                .map(|(reader, located)| (reader, located, pane.clone())));
        };
        let (reader, child) = self.child(pane, id).ok_or(UnknownChild)?;
        let as_read = AgentRef {
            status: child_status(&child),
            ..pane.clone()
        };
        Ok(Some((reader, child.located, as_read)))
    }

    /// The subagents of the session `at`, as `agents.list` gives them (the
    /// subagents plan, decision 2): at most [`CHILDREN_MAX`], the running
    /// first, then the finished, newest first within each; each with its
    /// short name and its turn, its description redacted and cut.
    ///
    /// [`CHILDREN_MAX`]: libflankr_proto::common::CHILDREN_MAX
    pub fn children_info(
        &self,
        reader: &dyn SessionReader,
        at: &Located,
    ) -> Vec<libflankr_proto::common::ChildInfo> {
        use libflankr_proto::common::{CHILDREN_MAX, ChildInfo, TURN_TEXT_MAX};
        let mut children = reader.children(at);
        // Stable: newest first within each.
        children.sort_by_key(|c| c.finished);
        children.truncate(CHILDREN_MAX);
        children
            .into_iter()
            .map(|c| {
                let description = cut(&redact::redact(&c.description), TURN_TEXT_MAX);
                ChildInfo {
                    id: c.id.clone(),
                    short: children::short_name(&description, &c.kind),
                    turn: self.child_turn(reader, &c),
                    description,
                    kind: c.kind,
                    background: c.background,
                    finished: c.finished,
                    started: c.started.unwrap_or(0),
                    started_by: c.started_by,
                }
            })
            .collect()
    }

    /// The turn of subagent `child`, read again only when its transcript
    /// or its finishing changed since last read.
    fn child_turn(&self, reader: &dyn SessionReader, child: &Child) -> Option<turn::Turn> {
        let modified = reader.modified(&child.located);
        let len = std::fs::metadata(&child.located.path).ok()?.len();
        let key = (modified, len, child.finished);
        if let Some((m, l, f, turn)) = self.child_turns.lock().unwrap().get(&child.located.path)
            && (*m, *l, *f) == key
        {
            return Some(turn.clone());
        }
        let read = reader
            .read_back(&child.located, &|items: &[Item]| {
                items
                    .iter()
                    .any(|i| i.is_text() && i.role == Role::Assistant)
            })
            .ok()?;
        let turn = turn::turn(&read.items, &child_status(child), child.located.quiet());
        let mut turns = self.child_turns.lock().unwrap();
        if turns.len() >= CHILD_TURNS_MAX {
            turns.clear();
        }
        turns.insert(
            child.located.path.clone(),
            (modified, len, child.finished, turn.clone()),
        );
        Some(turn)
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
    /// holds (within the reader's limits).
    pub fn read(&self, pane: &AgentRef, enough: impl Fn(&[Item]) -> bool) -> Option<SessionRead> {
        let (reader, located) = self.locate(pane)?;
        Self::read_at(reader, located, enough)
    }

    /// Read the session at `located` with `reader` from the end until
    /// `enough` holds (within the reader's limits).
    pub fn read_at(
        reader: &dyn SessionReader,
        located: Located,
        enough: impl Fn(&[Item]) -> bool,
    ) -> Option<SessionRead> {
        match reader.read_back(&located, &enough) {
            Ok(read) => Some(SessionRead {
                kind: reader.kind(),
                located,
                items: read.items,
            }),
            Err(e) => {
                log::warn!("Could not read the {} session: {e}", reader.kind());
                None
            }
        }
    }
}

/// A choice question open in a pane's session (plan 20261029_01): the
/// call as its reader read it, not redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenChoice {
    /// The reader's kind (`claude_code`).
    pub kind: &'static str,
    pub located: Located,
    /// The questions as the agent asked them.
    pub set: ChoiceSet,
    /// The call's timestamp.
    pub ts: String,
}

impl Sessions {
    /// The choice question open in `pane`'s session, whatever herdr's
    /// status: the newest call with choices that has no result and only
    /// context after it (decision 4's rule, the wait not required: the
    /// question's screen is up from the moment the call is written).
    pub fn open_choice(&self, pane: &AgentRef) -> Option<OpenChoice> {
        let read = self.read(pane, |items| {
            items
                .iter()
                .any(|i| i.choices.is_some() || (i.is_text() && i.role == Role::Assistant))
        })?;
        let at = pending::open_choice(&read.items, &pane.status, true)?;
        let call = &read.items[at];
        Some(OpenChoice {
            kind: read.kind,
            located: read.located,
            set: call.choices.clone()?,
            ts: call.ts.clone(),
        })
    }
}

impl Sessions {
    /// How choice call `id` in `pane`'s session was answered, once its
    /// result is written (plan 20261029_01, decision 5). A result that does
    /// not read as an answer gives an outcome with no answers.
    pub fn choice_result(&self, pane: &AgentRef, id: &str) -> Option<ChoiceOutcome> {
        let read = self.read(pane, |items| {
            items
                .iter()
                .any(|i| i.kind == EntryKind::ToolUse && i.call_id.as_deref() == Some(id))
        })?;
        let result = read
            .items
            .iter()
            .rev()
            .find(|i| i.kind == EntryKind::ToolResult && i.call_id.as_deref() == Some(id))?;
        Some(
            result
                .choice_result
                .clone()
                .unwrap_or_else(|| ChoiceOutcome {
                    content: Some(result.text.clone()),
                    ..ChoiceOutcome::default()
                }),
        )
    }
}

/// Why a message or command to `pane_id` is refused (plan 20261029_01,
/// decision 6): its agent is showing a choice question, which typed text
/// would land in as keystrokes. `None` when it is not, or the pane is not
/// in `snapshot`. Session files are only read on this machine, so the
/// caller passes `local: false` for a remote herdr and nothing is refused.
pub fn choice_refusal(
    sessions: &Sessions,
    local: bool,
    snapshot: &serde_json::Value,
    pane_id: &str,
) -> Option<String> {
    if !local {
        return None;
    }
    let pane = crate::core::agents::agents(snapshot)
        .into_iter()
        .find(|a| crate::watch::pane_ids_match(&a.pane_id, pane_id))?;
    sessions.reader_for(&pane.agent)?;
    sessions.open_choice(&pane)?;
    Some(format!(
        "{} is showing a question; answer it with its options",
        crate::core::naming::capitalise(&pane.agent)
    ))
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
    /// The position ([`Item::pos`]) of the newest entry of the agent's own
    /// reply text read, where the latest Finished card would be.
    pub last_reply: Option<u64>,
    /// The status the session was read with: herdr's for the pane, a
    /// subagent's own ([`child_status`]).
    pub status: libherdr::protocol::AgentStatus,
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
        self.peek_in(pane, None, n).ok().flatten()
    }

    /// As [`Sessions::peek`], of `pane`'s session or of its subagent
    /// `child` (the subagents plan, decision 2), read with the subagent's
    /// own status. [`UnknownChild`] when the session has no such subagent.
    pub fn peek_in(
        &self,
        pane: &AgentRef,
        child: Option<&str>,
        n: usize,
    ) -> Result<Option<Peeked>, UnknownChild> {
        let Some((reader, located, pane)) = self.locate_in(pane, child)? else {
            return Ok(None);
        };
        Ok(self.peek_at(reader, located, &pane, n))
    }

    fn peek_at(
        &self,
        reader: &dyn SessionReader,
        located: Located,
        pane: &AgentRef,
        n: usize,
    ) -> Option<Peeked> {
        let read = Self::read_at(reader, located, |items| {
            items.len() >= n
                && items
                    .iter()
                    .any(|i| i.is_text() && i.role == Role::Assistant)
        })?;
        let quiet = read.located.quiet();
        let suggestions = self.suggestions.current();
        let agent = crate::core::naming::capitalise(&pane.agent);
        let asker = pending::Asker {
            suggestions: &suggestions,
            who: Who {
                agent: &agent,
                project: pane.project.as_deref().unwrap_or_default(),
            },
        };
        let pending_question =
            pending::pending_question(&read.items, &pane.status, quiet, Some(&asker));
        let turn = turn::turn(&read.items, &pane.status, quiet);
        let last_reply = read
            .items
            .iter()
            .rfind(|i| i.is_text() && i.role == Role::Assistant && i.origin.is_none())
            .map(|i| i.pos);
        let skip = read.items.len().saturating_sub(n);
        Some(Peeked {
            status: pane.status.clone(),
            located: read.located,
            entries: read.items[skip..].iter().map(Item::entry).collect(),
            pending_question,
            turn,
            last_reply,
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
        self.reply_text_in(pane, None, ts).ok().flatten()
    }

    /// As [`Sessions::reply_text`], in `pane`'s session or in its subagent
    /// `child`'s (the subagents plan, decision 2). [`UnknownChild`] when
    /// the session has no such subagent.
    pub fn reply_text_in(
        &self,
        pane: &AgentRef,
        child: Option<&str>,
        ts: &str,
    ) -> Result<Option<String>, UnknownChild> {
        let Some((reader, located, _)) = self.locate_in(pane, child)? else {
            return Ok(None);
        };
        Ok(Self::reply_at(reader, located, ts))
    }

    fn reply_at(reader: &dyn SessionReader, located: Located, ts: &str) -> Option<String> {
        let is_reply = |i: &Item| i.is_text() && i.role == Role::Assistant && i.ts == ts;
        if ts.is_empty() {
            return None;
        }
        let read = Self::read_at(reader, located, |items| items.iter().any(is_reply))?;
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

impl From<Matched> for libflankr_proto::common::SessionMatched {
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

    /// A home with one Claude Code session in `/work/example` whose last
    /// reply ends on `question`, and its pane, idle.
    fn asking(question: &str) -> (tempfile::TempDir, Sessions, AgentRef) {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".claude/projects/-work-example");
        std::fs::create_dir_all(&dir).unwrap();
        let lines = [
            serde_json::json!({"type": "user", "timestamp": "2026-01-02T00:00:01.000Z",
                "isSidechain": false, "message": {"role": "user", "content": "go"},
                "origin": {"kind": "human"}}),
            serde_json::json!({"type": "assistant", "timestamp": "2026-01-02T00:00:02.000Z",
                "isSidechain": false, "message": {"role": "assistant", "type": "message",
                "content": [{"type": "text", "text": format!("Wrote it.\n\n{question}")}]}}),
        ];
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(dir.join("s1.jsonl"), text).unwrap();
        let sessions = Sessions::with_home(home.path());
        let pane = AgentRef {
            pane_id: "w1:p1".into(),
            agent: "claude".into(),
            project: Some("example".into()),
            cwd: Some("/work/example".into()),
            workspace_id: None,
            tab_id: None,
            tab_label: None,
            tab_number: None,
            status: libherdr::protocol::AgentStatus::Idle,
            focused: false,
            session: None,
        };
        (home, sessions, pane)
    }

    fn answers(sessions: &Sessions, pane: &AgentRef) -> Vec<(String, Option<String>)> {
        let q = sessions.peek(pane, 10).unwrap().pending_question.unwrap();
        q.answers
            .into_iter()
            .map(|a| (a.label, a.sends.or(a.starts)))
            .collect()
    }

    fn pair(label: &str, text: &str) -> (String, Option<String>) {
        (label.to_string(), Some(text.to_string()))
    }

    #[test]
    fn a_peeked_plan_question_has_its_answers() {
        let (_home, sessions, pane) = asking("Approve plan 04 and build it?");
        let q = sessions.peek(&pane, 10).unwrap().pending_question.unwrap();
        assert!(q.about_plan);
        assert_eq!(q.plan.as_deref(), Some("plan 04"));
        assert_eq!(
            answers(&sessions, &pane),
            [
                pair("approve", "Approve plan 04"),
                pair("discuss", "Discuss plan 04:"),
                pair("implement", "Approve plan 04 and start building it"),
            ]
        );
        assert_eq!(q.answers[0].style, "green");
        assert!(q.answers[0].answers && !q.answers[1].answers);
        let (_home, sessions, pane) = asking("Install it on the S23?");
        let q = sessions.peek(&pane, 10).unwrap().pending_question.unwrap();
        assert!(!q.about_plan && q.plan.is_none());
        assert_eq!(
            answers(&sessions, &pane),
            [pair("Yes", "yes"), pair("No", "no")]
        );
    }

    const USER_FILE: &str = "\
# [Answer] approve\n\n- label: approve\n- style: green\n- sends: LGTM {{ [abc] detected_plan_id }}, {{ [abc] agent }}\n- answers: true\n\n\
# [Rule] install\n\n- when: install\n- offers: install, no\n\n\
# [Answer] install\n\n- label: install\n- style: green\n- sends: Install it on {{ [abc] project }}\n- answers: true\n\n\
# [Answer] discuss\n\n- label: discuss\n- style: purple\n- starts: Talk:\n";

    #[test]
    fn a_user_file_and_an_edit_change_the_next_peeks_answers() {
        let (home, sessions, pane) = asking("Approve plan 04?");
        let file = home.path().join("rules.md");
        let user = home.path().join("user/SUGGESTIONS.md");
        std::fs::write(&file, crate::suggestions::SUGGESTIONS_RAW).unwrap();
        sessions
            .suggestions
            .set_files(Some(file.clone()), Some(user.clone()));
        let before = sessions.suggestions.generation();
        assert_eq!(
            answers(&sessions, &pane)[0],
            pair("approve", "Approve plan 04")
        );
        // The user file: approve replaced, a rule tried first, and a broken
        // discuss that leaves the shipped one.
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        std::fs::write(&user, USER_FILE).unwrap();
        assert!(sessions.suggestions.reload());
        assert_eq!(
            answers(&sessions, &pane),
            [
                pair("approve", "LGTM plan 04, Claude"),
                pair("discuss", "Discuss plan 04:"),
                pair("implement", "Approve plan 04 and start building it"),
            ]
        );
        let (_h2, other, install) = asking("Install it now?");
        other
            .suggestions
            .set_files(Some(file.clone()), Some(user.clone()));
        assert_eq!(
            answers(&other, &install),
            [pair("install", "Install it on example"), pair("No", "no")]
        );
        // An edit to the given file changes the next peek's answers.
        std::fs::write(
            &file,
            crate::suggestions::SUGGESTIONS_RAW.replace("label: approve", "label: ok"),
        )
        .unwrap();
        // The user's approve still replaces the shipped one: no change.
        assert!(!sessions.suggestions.reload());
        assert_eq!(sessions.suggestions.generation(), before + 1);
        assert_eq!(answers(&sessions, &pane)[0].0, "approve");
        std::fs::remove_file(&user).unwrap();
        assert!(sessions.suggestions.reload());
        assert_eq!(answers(&sessions, &pane)[0], pair("ok", "Approve plan 04"));
        // Unchanged files: nothing to do.
        assert!(!sessions.suggestions.reload());
    }

    #[tokio::test]
    async fn the_file_watcher_reloads_the_rules() {
        let Ok(watcher) = crate::watch_files::FileWatcher::start() else {
            return;
        };
        let (home, sessions, pane) = asking("Approve plan 04?");
        let file = home.path().join("rules.md");
        std::fs::write(&file, crate::suggestions::SUGGESTIONS_RAW).unwrap();
        sessions.suggestions.set_files(Some(file.clone()), None);
        sessions.suggestions.spawn_watch(Some(&watcher));
        let before = sessions.suggestions.generation();
        // Let the watch start before the edit.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        std::fs::write(
            &file,
            crate::suggestions::SUGGESTIONS_RAW.replace("label: approve", "label: ok"),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while sessions.suggestions.generation() == before {
            assert!(std::time::Instant::now() < deadline, "no reload");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(answers(&sessions, &pane)[0].0, "ok");
    }

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
    fn parse_command_exit_codes() {
        assert_eq!(
            parse_command_exit_code("Command exited with code 0.\n\noutput"),
            Some(0)
        );
        assert_eq!(
            parse_command_exit_code("Command exited with code 1.\n\nerror"),
            Some(1)
        );
        assert_eq!(
            parse_command_exit_code("Command exited with code 101."),
            Some(101)
        );
        assert_eq!(
            parse_command_exit_code("Background command completed (exit code 0)"),
            Some(0)
        );
        assert_eq!(
            parse_command_exit_code("Background command completed (exit code 2)"),
            Some(2)
        );
        assert_eq!(parse_command_exit_code("Just normal output"), None);
    }

    #[test]
    fn an_entry_carries_its_full_redacted_text_and_a_500_character_summary() {
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let long = format!("{} {secret} the end", "word ".repeat(400));
        let item = Item {
            pos: 1,
            ts: "t".into(),
            role: Role::Assistant,
            kind: EntryKind::Text,
            tool: None,
            text: long.clone(),
            call_id: None,
            origin: None,
            choices: None,
            choice_result: None,
        };
        let entry = item.entry();
        assert_eq!(entry.summary.chars().count(), ENTRY_SUMMARY_MAX);
        assert!(entry.summary.ends_with('…'));
        let text = entry.text.expect("the full text");
        assert_eq!(text, redact::redact(&long));
        assert!(
            text.ends_with("the end") && !text.contains(secret),
            "{text}"
        );
        assert!(text.chars().count() > 2000);
        // A short entry's summary is all of it: no `text`.
        let short = Item {
            text: "Done.".into(),
            ..item
        };
        assert_eq!(short.entry().summary, "Done.");
        assert_eq!(short.entry().text, None);
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

    #[test]
    fn huge_lines_do_not_use_up_the_read_budget() {
        // Ten conversation lines, each followed by a line as big as an
        // image result: all ten come back, as without the padding.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let mut text = String::new();
        let huge = "x".repeat(800 * 1024);
        for i in 0..10 {
            text.push_str(&format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"t{i}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"reply {i}\"}}]}}}}\n"
            ));
            text.push_str(&format!(
                "{{\"type\":\"user\",\"timestamp\":\"u{i}\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"c{i}\",\"content\":\"{huge}\"}}]}}}}\n"
            ));
        }
        std::fs::write(&path, text).unwrap();
        let reader = claude::ClaudeCode::new(dir.path().to_path_buf());
        let items = read_tail(&reader, &path, |items| items.len() >= 100)
            .unwrap()
            .items;
        let replies = items.iter().filter(|i| i.kind == EntryKind::Text).count();
        assert_eq!(replies, 10);
    }
}
