//! The session hub (plan 20261023_03, section 5.1, as amended at 14:54 and
//! 22:55 EDT).
//!
//! One [`Tailer`] for each session file that at least one subscription
//! needs, however many subscribers it has. It keeps the file's line index
//! (the start of every line, 8 bytes a line, built by one scan when it
//! starts), checks the file every 250 ms, reads only the bytes appended
//! since (at most 1 MiB a check), parses whole lines, builds, links and
//! redacts their sections once, runs the card rules, and broadcasts the
//! same [`Batch`] to every subscriber. A tailer with no subscribers stops
//! and is dropped.
//!
//! [`page`] and [`since`] read the sections of any span of the file from
//! the index, for backfills and `session.history`: a builder started
//! [`REACH`] lines before the span and read [`REACH`] lines past it groups
//! the span exactly as the tailer does (see [`super::sections`]).
//!
//! The [`Outline`] numbers the file's sections 0, 1, 2… with no gaps
//! (plan 20261024_01, decisions 1 and 2). It is built, the first time an
//! outline subscription or `session.content` needs it, by reading every
//! section of the file in serial order; from then on each check's new
//! sections take the next indexes, in serial order, a replaced section
//! keeps its index and bumps its `rev`, and a removed one's index stays
//! removed. A file read again (another tailer, or after a restart) is
//! numbered the same, but for a section that came and went before the
//! read, or a card that appeared behind sections already numbered.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime};

use libhandlr_proto::common::{
    CardKind, PendingQuestion, SERIAL_LINE_MAX, Section, SectionKind, Serial, Slot,
};
use libherdr::protocol::AgentStatus;
use tokio::sync::{Notify, broadcast, mpsc};

mod summaries;
use summaries::SummaryJob;
pub use summaries::{CARD_SUMMARIES_MAX, CardSummaries, CardSummarizer, CardSummary};

use super::journal::{Approval, Journal};
use super::links::Linker;
use super::sections::{self, ApprovalCard, Builder, Live, Placed, REACH};
use super::{Item, LineReader, Located, Matched, Sessions, TAIL_READ_MAX, pending};

/// How often a tailer checks its file.
pub const CHECK_INTERVAL: Duration = Duration::from_millis(250);
/// The most a tailer reads in one check; the rest waits for the next.
pub const READ_PER_CHECK: u64 = 1024 * 1024;
/// The items a tailer keeps, unredacted, for the live cards and the
/// pending question.
pub const KEEP_ITEMS: usize = 256;
/// How far a catch-up (`since`) reads; a client further behind is reset.
pub const SINCE_MAX_LINES: u32 = 50_000;
/// How much of a line's start is read to know it as bookkeeping.
const HEAD: usize = 256;
/// The lines one span of an outline's build reads.
const OUTLINE_SPAN: u32 = 4096;
/// Serials further apart than this, in lines, are read in separate spans
/// for `session.content`.
const CONTENT_GAP: u32 = 1024;

/// The first and last lines a scan finished, if any.
pub type NewLines = Option<(u32, u32)>;

/// The start of every whole line of a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineIndex {
    starts: Vec<u64>,
    /// The offset just after the last whole line.
    end: u64,
    /// How far the file has been scanned, past `end` while a line is
    /// unfinished.
    scanned: u64,
    /// The unfinished line holds a needle (see [`LineIndex::extend`]).
    line_flag: bool,
    /// The unfinished line's last bytes, so a needle split between two
    /// reads is found.
    line_tail: Vec<u8>,
}

impl LineIndex {
    /// Index the whole file at `path`; also the lines that may name a
    /// secret path (see [`LineIndex::extend`]).
    pub fn scan(path: &Path) -> std::io::Result<(Self, Vec<u32>)> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut index = LineIndex::default();
        let (_, flagged) = index.extend(&mut file, len, u64::MAX)?;
        Ok((index, flagged))
    }

    /// The number of whole lines.
    pub fn lines(&self) -> u32 {
        self.starts.len().min(u32::MAX as usize) as u32
    }

    /// The offset just after the last whole line.
    pub fn end(&self) -> u64 {
        self.end
    }

    pub fn scanned(&self) -> u64 {
        self.scanned
    }

    /// Line `line`'s bytes (1-based): its start, and its end without the
    /// line break.
    pub fn span(&self, line: u32) -> Option<(u64, u64)> {
        let i = (line as usize).checked_sub(1)?;
        let start = *self.starts.get(i)?;
        let next = self.starts.get(i + 1).copied().unwrap_or(self.end);
        Some((start, next.saturating_sub(1)))
    }

    /// Scan `file` from where the index stops, up to `len` and at most
    /// `max` bytes, adding the lines it finishes. Returns the first and
    /// last new line, or `None` when none was finished; and the new lines
    /// that hold `.env` or `.private`, whose tool calls may name a secret
    /// path (decision 7: a result is redacted by its call wherever in the
    /// file the call is).
    pub fn extend(
        &mut self,
        file: &mut File,
        len: u64,
        max: u64,
    ) -> std::io::Result<(NewLines, Vec<u32>)> {
        let from = self.scanned.max(self.end);
        let to = len.min(from.saturating_add(max));
        let mut flagged = Vec::new();
        if to <= from {
            return Ok((None, flagged));
        }
        let before = self.lines();
        file.seek(SeekFrom::Start(from))?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut at = from;
        while at < to {
            let want = ((to - at) as usize).min(buf.len());
            let got = file.read(&mut buf[..want])?;
            if got == 0 {
                break;
            }
            let data = &buf[..got];
            let mut seg = 0usize;
            while let Some(rel) = data[seg..].iter().position(|&b| b == b'\n') {
                let nl = seg + rel;
                self.note(&data[seg..nl]);
                if self.line_flag {
                    flagged.push(self.lines() + 1);
                }
                self.line_flag = false;
                self.line_tail.clear();
                self.starts.push(self.end);
                self.end = at + nl as u64 + 1;
                seg = nl + 1;
            }
            self.note(&data[seg..]);
            at += got as u64;
        }
        self.scanned = at;
        let after = self.lines();
        Ok(((after > before).then_some((before + 1, after)), flagged))
    }

    /// Note a piece of the line being scanned: whether it, or it joined to
    /// the line's last bytes before it, holds a needle.
    fn note(&mut self, piece: &[u8]) {
        const NEEDLES: [&[u8]; 2] = [b".env", b".private"];
        const TAIL: usize = 7;
        if piece.is_empty() || self.line_flag {
            return;
        }
        let has = |bytes: &[u8]| {
            NEEDLES
                .iter()
                .any(|n| bytes.windows(n.len()).any(|w| w == *n))
        };
        let mut joint = self.line_tail.clone();
        joint.extend_from_slice(&piece[..piece.len().min(TAIL)]);
        if has(&joint) || has(piece) {
            self.line_flag = true;
            return;
        }
        let mut tail = std::mem::take(&mut self.line_tail);
        tail.extend_from_slice(&piece[piece.len().saturating_sub(TAIL)..]);
        let cut = tail.len().saturating_sub(TAIL);
        tail.drain(..cut);
        self.line_tail = tail;
    }
}

/// The items of lines `from` to `to` (1-based, inclusive), each line's
/// with its number; lines with none are left out. Bookkeeping lines are
/// known from their first bytes and not read further; a line longer than
/// `TAIL_READ_MAX` is not read.
pub fn read_lines(
    reader: &dyn LineReader,
    path: &Path,
    index: &LineIndex,
    from: u32,
    to: u32,
) -> std::io::Result<Vec<(u32, Vec<Item>)>> {
    let mut out = Vec::new();
    if from > to {
        return Ok(out);
    }
    let mut file = File::open(path)?;
    let mut buf = Vec::new();
    for line in from..=to {
        let Some((start, end)) = index.span(line) else {
            break;
        };
        let len = end.saturating_sub(start);
        if len == 0 || len > TAIL_READ_MAX {
            continue;
        }
        file.seek(SeekFrom::Start(start))?;
        let head = (len as usize).min(HEAD);
        buf.resize(head, 0);
        file.read_exact(&mut buf)?;
        if reader.is_bookkeeping(buf.trim_ascii_start()) {
            continue;
        }
        if (len as usize) > head {
            buf.resize(len as usize, 0);
            file.read_exact(&mut buf[head..])?;
        }
        let text = buf.trim_ascii();
        if text.is_empty() {
            continue;
        }
        let items = reader.parse_line(&String::from_utf8_lossy(text));
        if !items.is_empty() {
            out.push((line, items));
        }
    }
    Ok(out)
}

/// The pane's live state, as a tailer keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveState {
    /// The agent's shown name.
    pub name: String,
    pub status: AgentStatus,
    pub quiet: bool,
    pub pending_question: Option<PendingQuestion>,
    pub blocked_at: Option<u32>,
}

impl Default for LiveState {
    fn default() -> Self {
        LiveState {
            name: "The agent".to_string(),
            status: AgentStatus::Unknown,
            quiet: false,
            pending_question: None,
            blocked_at: None,
        }
    }
}

impl LiveState {
    fn live(&self) -> Live<'_> {
        Live {
            name: &self.name,
            status: &self.status,
            quiet: self.quiet,
            pending_question: self.pending_question.as_ref(),
            blocked_at: self.blocked_at,
        }
    }
}

/// The source backend for reading items.
pub enum SourceBackend<'a> {
    Line {
        reader: &'a dyn LineReader,
        index: &'a LineIndex,
    },
    Agy {
        reader: &'a super::agy::AgyReader,
        max_idx: u32,
    },
}

/// What a span of the file is read from.
pub struct Source<'a> {
    pub backend: SourceBackend<'a>,
    pub path: &'a Path,
    /// The file name's hash, the serials' prefix.
    pub file: u32,
    pub linker: &'a Linker,
    pub approvals: &'a [ApprovalCard],
    /// The pane's state, for the cards at the file's end.
    pub live: &'a LiveState,
    /// The calls anywhere in the file that name a secret path.
    pub secrets: &'a HashSet<String>,
    /// The Finished cards' summaries, given to the cards found.
    pub summaries: Option<&'a CardSummaries>,
}

impl<'a> Source<'a> {
    pub fn first(&self) -> u32 {
        match &self.backend {
            SourceBackend::Line { .. } => 1,
            SourceBackend::Agy { .. } => 0,
        }
    }

    pub fn last(&self) -> u32 {
        match &self.backend {
            SourceBackend::Line { index, .. } => index.lines(),
            SourceBackend::Agy { max_idx, .. } => *max_idx,
        }
    }

    pub fn read_records(&self, from: u32, to: u32) -> std::io::Result<Vec<(u32, Vec<Item>)>> {
        match &self.backend {
            SourceBackend::Line { reader, index } => {
                read_lines(*reader, self.path, index, from, to)
            }
            SourceBackend::Agy { reader, .. } => reader.read_range(self.path, from, to),
        }
    }
}

/// The sections, cards among them, that start on lines `a` to `b`.
fn span(src: &Source, a: u32, b: u32) -> std::io::Result<Vec<Section>> {
    let last = src.last();
    let from = a.saturating_sub(REACH).max(src.first());
    let to = b.saturating_add(REACH).min(last);
    let mut builder = Builder::new(src.file, src.linker.clone());
    builder.add_secrets(src.secrets);
    let mut placed: Vec<Placed> = Vec::new();
    for (line, items) in src.read_records(from, to)? {
        placed.extend(builder.feed(line, items));
    }
    let at_end = to >= last;
    let approvals: Vec<ApprovalCard> = src
        .approvals
        .iter()
        .filter(|c| (a..=b).contains(&c.line))
        .cloned()
        .collect();
    let live = src.live.live();
    let mut cards = sections::cards(
        src.file,
        &placed,
        &approvals,
        at_end.then_some(&live),
        src.linker,
    );
    if let Some(summaries) = src.summaries {
        cards.iter_mut().for_each(|c| summaries.apply(c));
    }
    let within = |s: &Section| (a..=b).contains(&s.serial.line);
    Ok(sections::merged(
        builder.sections().values().filter(|s| within(s)).cloned(),
        cards.into_iter().filter(within).collect(),
    ))
}

/// A page of sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// Oldest first.
    pub sections: Vec<Section>,
    /// It reaches the file's first line.
    pub start: bool,
}

/// The newest `n` sections below `below` (all of the file's, without it):
/// read back from `below`'s line, a span at a time, until there are `n`
/// or the file's first line is reached.
pub fn page(src: &Source, below: Option<Serial>, n: usize) -> std::io::Result<Page> {
    let first = src.first();
    let last = src.last();
    let hi = below.map_or(last, |s| s.line.min(last));
    if (hi == 0 && first > 0) || n == 0 {
        return Ok(Page {
            sections: Vec::new(),
            start: true,
        });
    }
    let mut back: u32 = 64;
    loop {
        let a = hi.saturating_sub(back).max(first);
        let mut found: Vec<Section> = span(src, a, hi)?
            .into_iter()
            .filter(|s| below.is_none_or(|b| s.serial.to_u64() < b.to_u64()))
            .collect();
        if found.len() >= n || a == first {
            let start = a == first && found.len() <= n;
            let skip = found.len().saturating_sub(n);
            found.drain(..skip);
            return Ok(Page {
                sections: found,
                start,
            });
        }
        back = back.saturating_mul(4);
    }
}

/// What a client holding sections up to `after` is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Since {
    pub sections: Vec<Section>,
    /// The client cannot be caught up (another file, a shorter file, or
    /// more than `n` new sections): `sections` is a fresh backfill.
    pub reset: bool,
}

/// The sections after `after`, and those within `REACH` lines before it,
/// which may have changed; a reset with the newest `n` when that is more
/// than `n` new ones or `after` is not of this file.
pub fn since(src: &Source, after: Serial, n: usize) -> std::io::Result<Since> {
    let last = src.last();
    let reset = |src: &Source| -> std::io::Result<Since> {
        Ok(Since {
            sections: page(src, None, n)?.sections,
            reset: true,
        })
    };
    if after.file != src.file || after.line > last || last - after.line > SINCE_MAX_LINES {
        return reset(src);
    }
    let from = after.line.saturating_sub(REACH).max(src.first());
    let found = span(src, from, last)?;
    let newer = found
        .iter()
        .filter(|s| s.serial.to_u64() > after.to_u64())
        .count();
    if newer > n {
        return reset(src);
    }
    Ok(Since {
        sections: found,
        reset: false,
    })
}

/// A slot as the outline keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Kept {
    serial: Serial,
    kind: SectionKind,
    lines: u32,
    chars: u32,
    rev: u32,
    removed: bool,
    /// A Finished card's slot.
    bookend: bool,
}

/// A session file's sections by index (plan 20261024_01, decision 1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    slots: Vec<Kept>,
    by_serial: HashMap<u64, u64>,
}

/// What an index range of the outline holds (see [`Outline::range`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Range {
    /// The indexes of the sections in it, in order.
    pub indexes: Vec<u64>,
    /// The removed indexes in the part walked, in order.
    pub removed: Vec<u64>,
    /// It was cut at the limit with sections left.
    pub more: bool,
}

impl Outline {
    /// The outline of `sections`, in the order given, numbered from 0.
    pub fn of(sections: &[Section]) -> Self {
        let mut outline = Outline::default();
        for s in sections {
            outline.put(s);
        }
        outline
    }

    /// The indexes given so far, removed ones among them.
    pub fn len(&self) -> u64 {
        self.slots.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The highest index, if any.
    pub fn last(&self) -> Option<u64> {
        self.len().checked_sub(1)
    }

    /// The index of the section `serial`, removed or not.
    pub fn index_of(&self, serial: Serial) -> Option<u64> {
        self.by_serial.get(&serial.to_u64()).copied()
    }

    pub fn is_removed(&self, index: u64) -> bool {
        self.slots.get(index as usize).is_some_and(|k| k.removed)
    }

    /// The removed indexes.
    pub fn removed(&self) -> Vec<u64> {
        (0..self.len()).filter(|&i| self.is_removed(i)).collect()
    }

    /// The slot at `index`, unless it is removed.
    pub fn slot(&self, index: u64) -> Option<Slot> {
        let k = self.slots.get(index as usize).filter(|k| !k.removed)?;
        Some(Slot {
            index,
            serial: k.serial,
            kind: k.kind,
            lines: k.lines,
            chars: k.chars,
            rev: k.rev,
            bookend: k.bookend,
        })
    }

    /// The slots of indexes `from` to `from + n - 1`, removed ones left
    /// out.
    pub fn slots(&self, from: u64, n: usize) -> Vec<Slot> {
        let to = from.saturating_add(n as u64).min(self.len());
        (from..to).filter_map(|i| self.slot(i)).collect()
    }

    /// Number `section`, or replace the one of its serial: a new serial
    /// takes the next index; a known one keeps its index, bumps its `rev`
    /// and stands again if it was removed (the same section, so its index
    /// names it still; no other section ever takes it).
    pub fn put(&mut self, section: &Section) -> Slot {
        let (lines, chars) = sections::size(section);
        let bookend = is_bookend(section);
        let key = section.serial.to_u64();
        let index = match self.by_serial.get(&key) {
            Some(&i) => {
                let k = &mut self.slots[i as usize];
                k.kind = section.kind;
                k.lines = lines;
                k.chars = chars;
                k.rev = k.rev.saturating_add(1);
                k.removed = false;
                k.bookend = bookend;
                i
            }
            None => {
                let i = self.len();
                self.slots.push(Kept {
                    serial: section.serial,
                    kind: section.kind,
                    lines,
                    chars,
                    rev: 0,
                    removed: false,
                    bookend,
                });
                self.by_serial.insert(key, i);
                i
            }
        };
        self.slot(index).expect("just put")
    }

    /// Mark the section `serial` removed: its index, if it stood.
    pub fn remove(&mut self, serial: Serial) -> Option<u64> {
        let i = self.index_of(serial)?;
        let k = &mut self.slots[i as usize];
        if k.removed {
            return None;
        }
        k.removed = true;
        Some(i)
    }

    /// The sections of an inclusive index range, `None` open on its side
    /// (plan 20261024_01, the 00:03 EDT amendment): `from` only, the next
    /// `limit` from `from` on; `to` only, the `limit` ending at `to`,
    /// oldest first; both, the range cut at `limit` from `from`; neither,
    /// the newest `limit`. `more` when sections of the range were left
    /// past the cut (after it walking forward, before it walking back).
    pub fn range(&self, from: Option<u64>, to: Option<u64>, limit: usize) -> Range {
        let mut out = Range::default();
        let Some(last) = self.last() else {
            return out;
        };
        let hi = to.unwrap_or(last).min(last);
        match from {
            Some(lo) => {
                let mut i = lo;
                while i <= hi {
                    if self.is_removed(i) {
                        out.removed.push(i);
                    } else if out.indexes.len() == limit {
                        out.more = true;
                        break;
                    } else {
                        out.indexes.push(i);
                    }
                    i += 1;
                }
                // Removed indexes past the cut belong to the next request.
                if out.more {
                    let cut = out.indexes.last().copied().unwrap_or(lo);
                    out.removed.retain(|&r| r < cut);
                }
            }
            None => {
                let mut i = hi;
                loop {
                    if self.is_removed(i) {
                        out.removed.push(i);
                    } else if out.indexes.len() == limit {
                        out.more = true;
                        break;
                    } else {
                        out.indexes.push(i);
                    }
                    if i == 0 {
                        break;
                    }
                    i -= 1;
                }
                if out.more {
                    let cut = out.indexes.last().copied().unwrap_or(hi);
                    out.removed.retain(|&r| r > cut);
                }
                out.indexes.reverse();
                out.removed.reverse();
            }
        }
        out
    }
}

/// Whether `section` is a bookend: a Finished card, which closes a section
/// of the session (plan 20261022_03, the L3 amendment of 2026-10-24 12:27
/// EDT).
pub fn is_bookend(section: &Section) -> bool {
    section
        .card
        .as_ref()
        .is_some_and(|c| c.card == CardKind::Finished)
}

/// Every section of the file, in serial order, read a span at a time.
pub fn every_section(src: &Source) -> std::io::Result<Vec<Section>> {
    let first = src.first();
    let last = src.last();
    let mut out = Vec::new();
    let mut a = first;
    while a <= last {
        let b = a.saturating_add(OUTLINE_SPAN - 1).min(last);
        out.extend(span(src, a, b)?);
        if b == u32::MAX || b >= last {
            break;
        }
        a = b + 1;
    }
    Ok(out)
}

/// The sections with the serials `wanted`, in the order given; those no
/// longer found are left out. Serials close together are read in one
/// span.
pub fn sections_at(src: &Source, wanted: &[Serial]) -> std::io::Result<Vec<Section>> {
    let mut lines: Vec<u32> = wanted.iter().map(|s| s.line).collect();
    lines.sort_unstable();
    lines.dedup();
    let keys: HashSet<u64> = wanted.iter().map(|s| s.to_u64()).collect();
    let mut found: HashMap<u64, Section> = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let a = lines[i];
        let mut b = a;
        while i + 1 < lines.len() && lines[i + 1] - b <= CONTENT_GAP {
            i += 1;
            b = lines[i];
        }
        i += 1;
        for s in span(src, a, b)? {
            let key = s.serial.to_u64();
            if keys.contains(&key) {
                found.insert(key, s);
            }
        }
    }
    Ok(wanted
        .iter()
        .filter_map(|s| found.remove(&s.to_u64()))
        .collect())
}

/// What `session.content` answers from (see [`Tailer::content`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Content {
    /// Each with its index; empty when only slots were asked.
    pub sections: Vec<Section>,
    /// When only slots were asked.
    pub slots: Vec<Slot>,
    pub removed: Vec<u64>,
    pub last: Option<u64>,
    pub more: bool,
}

/// What a tailer broadcasts after a check that found something.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    /// New and changed sections, in serial order.
    pub sections: Vec<Section>,
    /// Card sections that no longer stand.
    pub removed: Vec<Serial>,
    /// The file shrank or went: subscribers reset.
    pub truncated: bool,
    /// The file has more lines than serials can number.
    pub too_long: bool,
    /// The pending question now.
    pub pending_question: Option<PendingQuestion>,
    /// The outline was kept when this was made: `sections` carry their
    /// indexes, and the three fields below are set (plan 20261024_01).
    pub outlined: bool,
    /// The slots of `sections`, in index order.
    pub slots: Vec<Slot>,
    /// The indexes of `removed` that stood, in order.
    pub removed_index: Vec<u64>,
    /// The outline's highest index after this batch.
    pub last: Option<u64>,
}

/// What a subscriber reads its backfill from: the tailer's state when it
/// subscribed.
#[derive(Debug, Clone)]
pub enum SnapshotBackend {
    Line(LineIndex),
    Agy { max_idx: u32 },
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub backend: SnapshotBackend,
    pub index: LineIndex,
    pub approvals: Vec<ApprovalCard>,
    pub live: LiveState,
    pub too_long: bool,
    /// The calls anywhere in the file that name a secret path.
    pub secrets: HashSet<String>,
}

#[derive(Debug)]
enum TailBackend {
    Line(LineIndex),
    Agy {
        max_idx: u32,
        last_modified: Option<SystemTime>,
    },
}

#[derive(Debug)]
struct TailState {
    backend: TailBackend,
    builder: Builder,
    window: VecDeque<Placed>,
    /// The cards as last broadcast, from the window's first line on.
    cards: BTreeMap<u64, Section>,
    approvals: Vec<Approval>,
    pending: HashSet<u64>,
    live: LiveState,
    /// The calls anywhere in the file that name a secret path.
    secrets: HashSet<String>,
    /// Something changed that the cards depend on.
    dirty: bool,
    too_long: bool,
    gone: bool,
    /// The sections by index, once something has asked for them.
    outline: Option<Outline>,
    /// Bumped each time the file is read again from its start, after it
    /// shrank or was replaced: indexes of another epoch do not apply.
    epoch: u64,
    /// The summaries' generation the cards were last given.
    summaries_seen: u64,
    /// The cards have been found once since the state was read: before
    /// that, only the newest Finished card is new.
    primed: bool,
}

/// One session file, tailed for its subscribers.
pub struct Tailer {
    pub path: PathBuf,
    /// The file name's hash, the serials' prefix.
    pub file: u32,
    /// The session's id, else the file's stem: the journal's name.
    pub session: String,
    agent: String,
    sessions: Arc<Sessions>,
    linker: Linker,
    state: Mutex<TailState>,
    tx: broadcast::Sender<Arc<Batch>>,
    wake: Arc<Notify>,
    /// The Finished cards' summaries, and where new cards are sent to be
    /// summarised (`[summary] cards`), if anywhere.
    summaries: Arc<CardSummaries>,
    jobs: Option<mpsc::UnboundedSender<SummaryJob>>,
    /// The project named to the summariser: the cwd's last part.
    project: String,
}

impl std::fmt::Debug for Tailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tailer").field("path", &self.path).finish()
    }
}

/// The journal's name for a located session.
pub fn session_key(located: &Located) -> String {
    located.session_id.clone().unwrap_or_else(|| {
        located
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

/// A file name's hash, the serials' prefix.
pub fn file_hash(path: &Path) -> u32 {
    Serial::file_hash(
        &path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

impl Tailer {
    /// A tailer for the session `located` of an `agent`, linked by
    /// `linker`, with the approvals of `journal`. Reads the file's index
    /// and its end.
    pub fn open(
        sessions: Arc<Sessions>,
        agent: &str,
        located: &Located,
        linker: Linker,
        journal: &Journal,
    ) -> std::io::Result<Tailer> {
        let session = session_key(located);
        let file = file_hash(&located.path);
        let state = if agent == "agy" {
            Self::fresh_state_agy(&sessions, &located.path, file, &linker)?
        } else {
            Self::fresh_state_line(&sessions, agent, &located.path, file, &linker)?
        };
        let mut state = state;
        state.approvals = journal.read(&session);
        let (tx, _) = broadcast::channel(64);
        Ok(Tailer {
            path: located.path.clone(),
            file,
            session,
            agent: agent.to_string(),
            sessions,
            linker,
            state: Mutex::new(state),
            tx,
            wake: Arc::new(Notify::new()),
            summaries: Arc::new(CardSummaries::default()),
            jobs: None,
            project: String::new(),
        })
    }

    /// This tailer with its cards summarised: given from `summaries`, and
    /// each new Finished card sent to `jobs`, as a reply of `project`.
    pub fn with_summaries(
        mut self,
        summaries: Arc<CardSummaries>,
        jobs: Option<mpsc::UnboundedSender<SummaryJob>>,
        project: &str,
    ) -> Self {
        self.summaries = summaries;
        self.jobs = jobs;
        self.project = project.to_string();
        self.state.lock().unwrap().dirty = true;
        self
    }

    fn fresh_state_line(
        sessions: &Sessions,
        agent: &str,
        path: &Path,
        file: u32,
        linker: &Linker,
    ) -> std::io::Result<TailState> {
        let (index, flagged) = LineIndex::scan(path)?;
        let reader = sessions
            .line_reader_for(agent)
            .ok_or_else(|| std::io::Error::other(format!("no line reader for {agent}")))?;
        let secrets = secret_calls(reader, path, &index, &flagged)?;
        let last = index.lines();
        // The end of the file, enough of it for the live cards: read back
        // until there are some items, as `read_tail` does. The builder
        // starts `REACH` lines earlier, so what it continues is grouped as
        // a read from the file's start groups it.
        let mut back = 2 * REACH;
        let (builder, window) = loop {
            let from = last.saturating_sub(back) + 1;
            let mut builder = Builder::new(file, linker.clone());
            builder.add_secrets(&secrets);
            let mut window: VecDeque<Placed> = VecDeque::new();
            let start = from.saturating_sub(REACH).max(1);
            for (line, items) in read_lines(reader, path, &index, start, last)? {
                let placed = builder.feed(line, items);
                if line >= from {
                    window.extend(placed);
                }
            }
            if window.len() >= KEEP_ITEMS / 4 || from <= 1 || back >= 64 * REACH {
                break (builder, window);
            }
            back *= 4;
        };
        let mut builder = builder;
        builder.take_changed();
        builder.prune();
        let mut window = window;
        while window.len() > KEEP_ITEMS {
            window.pop_front();
        }
        Ok(TailState {
            backend: TailBackend::Line(index),
            too_long: last > SERIAL_LINE_MAX,
            builder,
            window,
            cards: BTreeMap::new(),
            approvals: Vec::new(),
            pending: HashSet::new(),
            live: LiveState::default(),
            secrets,
            dirty: true,
            gone: false,
            outline: None,
            epoch: 0,
            summaries_seen: 0,
            primed: false,
        })
    }

    fn fresh_state_agy(
        sessions: &Sessions,
        path: &Path,
        file: u32,
        linker: &Linker,
    ) -> std::io::Result<TailState> {
        let max_idx = sessions.agy.max_idx(path)?.unwrap_or(0);
        let mut builder = Builder::new(file, linker.clone());
        let mut window: VecDeque<Placed> = VecDeque::new();
        let back = 2 * REACH;
        let from = max_idx.saturating_sub(back);
        let start = from.saturating_sub(REACH);
        for (pos, items) in sessions.agy.read_range(path, start, max_idx)? {
            let placed = builder.feed(pos, items);
            if pos >= from {
                window.extend(placed);
            }
        }
        let secrets = builder.secret_calls().cloned().collect();
        builder.take_changed();
        builder.prune();
        while window.len() > KEEP_ITEMS {
            window.pop_front();
        }
        let last_modified = Located {
            path: path.to_path_buf(),
            session_id: None,
            matched: Matched::Id,
        }
        .modified();
        Ok(TailState {
            backend: TailBackend::Agy {
                max_idx,
                last_modified,
            },
            too_long: max_idx > SERIAL_LINE_MAX,
            builder,
            window,
            cards: BTreeMap::new(),
            approvals: Vec::new(),
            pending: HashSet::new(),
            live: LiveState::default(),
            secrets,
            dirty: true,
            gone: false,
            outline: None,
            epoch: 0,
            summaries_seen: 0,
            primed: false,
        })
    }

    /// The reader of this tailer's agent.
    fn reader(&self) -> Option<&dyn LineReader> {
        self.sessions.line_reader_for(&self.agent)
    }

    /// Subscribe: the batches from now, and the state they follow from.
    pub fn subscribe(&self) -> (broadcast::Receiver<Arc<Batch>>, Snapshot) {
        let rx = self.tx.subscribe();
        // The cards as they stand now, so the first batch only changes them.
        self.check_cards_now();
        let snapshot = self.snapshot();
        (rx, snapshot)
    }

    /// Recompute the cards at once, so a new subscriber's snapshot and the
    /// cards broadcast agree; anything found goes to the subscribers.
    fn check_cards_now(&self) {
        let batch = {
            let mut st = self.state.lock().unwrap();
            if !st.dirty {
                return;
            }
            self.recompute(&mut st, Vec::new())
        };
        if let Some(batch) = batch {
            let _ = self.tx.send(Arc::new(batch));
        }
    }

    /// The pane's name and status, from a subscription's check. A blocked
    /// spell is anchored after the newest line when it began.
    pub fn set_live(&self, name: &str, status: &AgentStatus) {
        let mut st = self.state.lock().unwrap();
        let blocked = matches!(status, AgentStatus::Blocked);
        let blocked_at = match (blocked, st.live.blocked_at) {
            (true, Some(at)) => Some(at),
            (true, None) => Some(match &st.backend {
                TailBackend::Line(idx) => idx.lines(),
                TailBackend::Agy { max_idx, .. } => *max_idx,
            }),
            (false, _) => None,
        };
        if st.live.name != name || st.live.status != *status || st.live.blocked_at != blocked_at {
            st.live.name = name.to_string();
            st.live.status = status.clone();
            st.live.blocked_at = blocked_at;
            st.dirty = true;
            drop(st);
            self.wake.notify_one();
        }
    }

    /// A confirmation for this session: asked (with its journal record) or
    /// pending as before.
    pub fn approval(&self, approval: Approval, pending: bool) {
        let mut st = self.state.lock().unwrap();
        match st
            .approvals
            .iter_mut()
            .find(|a| a.confirm_id == approval.confirm_id)
        {
            Some(a) => *a = approval.clone(),
            None => st.approvals.push(approval.clone()),
        }
        if pending {
            st.pending.insert(approval.confirm_id);
        } else {
            st.pending.remove(&approval.confirm_id);
        }
        st.dirty = true;
        drop(st);
        self.wake.notify_one();
    }

    /// A confirmation of this session resolved.
    pub fn resolved(&self, confirm_id: u64, outcome: &str) {
        let mut st = self.state.lock().unwrap();
        st.pending.remove(&confirm_id);
        if let Some(a) = st.approvals.iter_mut().find(|a| a.confirm_id == confirm_id) {
            a.outcome = Some(outcome.to_string());
        }
        st.dirty = true;
        drop(st);
        self.wake.notify_one();
    }

    /// The journal's confirmations of this session without an outcome.
    pub fn unresolved(&self) -> Vec<Approval> {
        self.state
            .lock()
            .unwrap()
            .approvals
            .iter()
            .filter(|a| a.outcome.is_none())
            .cloned()
            .collect()
    }

    /// The newest whole line.
    pub fn newest_line(&self) -> u32 {
        match &self.state.lock().unwrap().backend {
            TailBackend::Line(idx) => idx.lines(),
            TailBackend::Agy { max_idx, .. } => *max_idx,
        }
    }

    /// Whether confirmation `confirm_id` is known to this tailer.
    pub fn knows(&self, confirm_id: u64) -> bool {
        self.state
            .lock()
            .unwrap()
            .approvals
            .iter()
            .any(|a| a.confirm_id == confirm_id)
    }

    /// The pending question as the tailer last found it.
    pub fn pending_question(&self) -> Option<PendingQuestion> {
        self.state.lock().unwrap().live.pending_question.clone()
    }

    /// The sections of a span of the file, as a backfill or a history page
    /// reads them, from `snapshot`.
    pub fn source<'a>(&'a self, snapshot: &'a Snapshot) -> Option<Source<'a>> {
        let backend = match &snapshot.backend {
            SnapshotBackend::Line(index) => SourceBackend::Line {
                reader: self.reader()?,
                index,
            },
            SnapshotBackend::Agy { max_idx } => SourceBackend::Agy {
                reader: &self.sessions.agy,
                max_idx: *max_idx,
            },
        };
        Some(Source {
            backend,
            path: &self.path,
            file: self.file,
            linker: &self.linker,
            approvals: &snapshot.approvals,
            live: &snapshot.live,
            secrets: &snapshot.secrets,
            summaries: Some(&self.summaries),
        })
    }

    /// A snapshot with the file read to its end and the cards and the
    /// pending question brought up to date first (what changed goes to the
    /// subscribers too). Blocks on the file.
    pub fn fresh_snapshot(&self) -> Snapshot {
        if let Some(batch) = self.check() {
            let _ = self.tx.send(Arc::new(batch));
        }
        self.check_cards_now();
        self.snapshot()
    }

    /// A snapshot for a history page: the state now.
    pub fn snapshot(&self) -> Snapshot {
        snapshot_of(&self.state.lock().unwrap())
    }

    /// The epoch of the file's reading (see `TailState::epoch`).
    pub fn epoch(&self) -> u64 {
        self.state.lock().unwrap().epoch
    }

    /// Build the outline if it is not kept yet: every section of the file
    /// read once, under the state's lock, so that each batch after it
    /// numbers what it adds. Blocks on the file.
    pub fn ensure_outline(&self) -> std::io::Result<()> {
        let mut st = self.state.lock().unwrap();
        if st.outline.is_some() {
            return Ok(());
        }
        let snap = snapshot_of(&st);
        let src = self
            .source(&snap)
            .ok_or_else(|| std::io::Error::other(format!("no reader for {}", self.agent)))?;
        let sections = every_section(&src)?;
        st.outline = Some(Outline::of(&sections));
        Ok(())
    }

    /// The outline as it stands, with its epoch; built first if needed.
    pub fn outline(&self) -> std::io::Result<(Outline, u64)> {
        self.ensure_outline()?;
        let st = self.state.lock().unwrap();
        Ok((st.outline.clone().unwrap_or_default(), st.epoch))
    }

    /// `session.content`'s answer for an index range (plan 20261024_01,
    /// decision 4, as amended at 00:03 EDT): the slots, or the redacted
    /// sections, each with its index. Blocks on the file.
    pub fn content(
        &self,
        from: Option<u64>,
        to: Option<u64>,
        limit: usize,
        slots_only: bool,
    ) -> std::io::Result<Content> {
        self.ensure_outline()?;
        let (range, slots, last, snap) = {
            let st = self.state.lock().unwrap();
            let outline = st.outline.as_ref().expect("just built");
            let range = outline.range(from, to, limit);
            let slots: Vec<Slot> = range
                .indexes
                .iter()
                .filter_map(|&i| outline.slot(i))
                .collect();
            (range, slots, outline.last(), snapshot_of(&st))
        };
        let mut content = Content {
            removed: range.removed,
            last,
            more: range.more,
            ..Content::default()
        };
        if slots_only {
            content.slots = slots;
        } else {
            content.sections = self.read_slots(&snap, &slots)?;
        }
        Ok(content)
    }

    /// The sections of `slots`, each given its slot's index; those no
    /// longer found are left out. Blocks on the file.
    pub fn read_slots(&self, snap: &Snapshot, slots: &[Slot]) -> std::io::Result<Vec<Section>> {
        let Some(src) = self.source(snap) else {
            return Ok(Vec::new());
        };
        let serials: Vec<Serial> = slots.iter().map(|s| s.serial).collect();
        let index: HashMap<u64, u64> = slots.iter().map(|s| (s.serial.to_u64(), s.index)).collect();
        let mut sections = sections_at(&src, &serials)?;
        for s in &mut sections {
            s.index = index.get(&s.serial.to_u64()).copied().unwrap_or(0);
        }
        Ok(sections)
    }

    /// Check the file once: what it found, if anything.
    pub fn check(&self) -> Option<Batch> {
        let mut st = self.state.lock().unwrap();
        if st.gone {
            return None;
        }
        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(_) => {
                st.gone = true;
                return Some(Batch {
                    truncated: true,
                    ..Batch::default()
                });
            }
        };
        let mut changed = Vec::new();
        let is_line = matches!(st.backend, TailBackend::Line(_));
        if is_line {
            let len = meta.len();
            let need_fresh = if let TailBackend::Line(index) = &st.backend {
                len < index.end()
            } else {
                false
            };
            if need_fresh {
                match Self::fresh_state_line(
                    &self.sessions,
                    &self.agent,
                    &self.path,
                    self.file,
                    &self.linker,
                ) {
                    Ok(mut fresh) => {
                        fresh.approvals = std::mem::take(&mut st.approvals);
                        fresh.pending = std::mem::take(&mut st.pending);
                        fresh.live = st.live.clone();
                        fresh.epoch = st.epoch + 1;
                        *st = fresh;
                    }
                    Err(_) => st.gone = true,
                }
                return Some(Batch {
                    truncated: true,
                    ..Batch::default()
                });
            }
            if !st.too_long {
                let TailBackend::Line(ref mut index) = st.backend else {
                    unreachable!()
                };
                let extended = if len > index.scanned() {
                    File::open(&self.path)
                        .and_then(|mut f| index.extend(&mut f, len, READ_PER_CHECK))
                } else {
                    Ok((None, Vec::new()))
                };
                match extended {
                    Ok((Some((first, last)), _)) => {
                        if last > SERIAL_LINE_MAX {
                            st.too_long = true;
                            return Some(Batch {
                                too_long: true,
                                ..Batch::default()
                            });
                        } else if let Some(reader) = self.reader() {
                            let TailBackend::Line(ref index) = st.backend else {
                                unreachable!()
                            };
                            match read_lines(reader, &self.path, index, first, last) {
                                Ok(lines) => {
                                    for (line, items) in lines {
                                        let placed = st.builder.feed(line, items);
                                        st.window.extend(placed);
                                    }
                                    while st.window.len() > KEEP_ITEMS {
                                        st.window.pop_front();
                                    }
                                    let found: Vec<String> =
                                        st.builder.secret_calls().cloned().collect();
                                    st.secrets.extend(found);
                                    changed = st.builder.take_changed();
                                    st.builder.prune();
                                    st.dirty = true;
                                }
                                Err(e) => log::warn!("reading {}: {e}", self.path.display()),
                            }
                        }
                    }
                    Ok((None, _)) => {}
                    Err(e) => log::warn!("reading {}: {e}", self.path.display()),
                }
            }
        } else {
            let (max_idx, last_modified) = match st.backend {
                TailBackend::Agy {
                    max_idx,
                    last_modified,
                } => (max_idx, last_modified),
                _ => unreachable!(),
            };
            let modified = Located {
                path: self.path.clone(),
                session_id: None,
                matched: Matched::Id,
            }
            .modified();
            if modified != last_modified {
                match self.sessions.agy.max_idx(&self.path) {
                    Ok(Some(new_max)) => {
                        if new_max < max_idx {
                            match Self::fresh_state_agy(
                                &self.sessions,
                                &self.path,
                                self.file,
                                &self.linker,
                            ) {
                                Ok(mut fresh) => {
                                    fresh.approvals = std::mem::take(&mut st.approvals);
                                    fresh.pending = std::mem::take(&mut st.pending);
                                    fresh.live = st.live.clone();
                                    fresh.epoch = st.epoch + 1;
                                    *st = fresh;
                                }
                                Err(_) => st.gone = true,
                            }
                            return Some(Batch {
                                truncated: true,
                                ..Batch::default()
                            });
                        } else if new_max > max_idx && !st.too_long {
                            if new_max > SERIAL_LINE_MAX {
                                st.too_long = true;
                                st.backend = TailBackend::Agy {
                                    max_idx: new_max,
                                    last_modified: modified,
                                };
                                return Some(Batch {
                                    too_long: true,
                                    ..Batch::default()
                                });
                            }
                            match self
                                .sessions
                                .agy
                                .read_range(&self.path, max_idx + 1, new_max)
                            {
                                Ok(steps) => {
                                    for (pos, items) in steps {
                                        let placed = st.builder.feed(pos, items);
                                        st.window.extend(placed);
                                    }
                                    while st.window.len() > KEEP_ITEMS {
                                        st.window.pop_front();
                                    }
                                    let found: Vec<String> =
                                        st.builder.secret_calls().cloned().collect();
                                    st.secrets.extend(found);
                                    changed = st.builder.take_changed();
                                    st.builder.prune();
                                    st.dirty = true;
                                    st.backend = TailBackend::Agy {
                                        max_idx: new_max,
                                        last_modified: modified,
                                    };
                                }
                                Err(e) => log::warn!("reading {}: {e}", self.path.display()),
                            }
                        } else {
                            st.backend = TailBackend::Agy {
                                max_idx,
                                last_modified: modified,
                            };
                        }
                    }
                    Ok(None) => {}
                    Err(e) => log::warn!("reading max_idx {}: {e}", self.path.display()),
                }
            }
        }
        let quiet = Located {
            path: self.path.clone(),
            session_id: None,
            matched: Matched::Id,
        }
        .quiet();
        if quiet != st.live.quiet {
            st.live.quiet = quiet;
            st.dirty = true;
        }
        let generation = self.summaries.generation();
        if generation != st.summaries_seen {
            st.summaries_seen = generation;
            st.dirty = true;
        }
        if !st.dirty {
            return None;
        }
        self.recompute(&mut st, changed)
    }

    /// The pending question and the cards again; `changed` are the
    /// sections the check built. `None` when nothing changed.
    fn recompute(&self, st: &mut TailState, changed: Vec<Section>) -> Option<Batch> {
        st.dirty = false;
        let items: Vec<Item> = st.window.iter().map(|p| p.item.clone()).collect();
        let mut question = pending::pending_question(&items, &st.live.status, st.live.quiet);
        if let Some(q) = &mut question {
            self.linker.link_question(q);
        }
        let question_changed = question != st.live.pending_question;
        st.live.pending_question = question.clone();
        let managed = st.window.front().map_or(1, |p| p.line);
        let approvals: Vec<ApprovalCard> = approval_cards(st)
            .into_iter()
            .filter(|a| a.line >= managed)
            .collect();
        let window = st.window.make_contiguous().to_vec();
        let live = st.live.live();
        let mut cards = sections::cards(self.file, &window, &approvals, Some(&live), &self.linker);
        cards.iter_mut().for_each(|c| self.summaries.apply(c));
        self.ask_summaries(st, &window, &cards);
        let mut out: BTreeMap<u64, Section> = changed
            .into_iter()
            .map(|s| (s.serial.to_u64(), s))
            .collect();
        let mut keep = HashSet::new();
        for c in cards {
            let key = c.serial.to_u64();
            keep.insert(key);
            if st.cards.get(&key) != Some(&c) {
                st.cards.insert(key, c.clone());
                out.insert(key, c);
            }
        }
        let mut removed = Vec::new();
        st.cards.retain(|key, c| {
            if c.serial.line < managed {
                return false;
            }
            if keep.contains(key) {
                return true;
            }
            removed.push(c.serial);
            false
        });
        if out.is_empty() && removed.is_empty() && !question_changed {
            return None;
        }
        let mut sections: Vec<Section> = out.into_values().collect();
        let mut slots = Vec::new();
        let mut removed_index = Vec::new();
        let outlined = st.outline.is_some();
        let mut last = None;
        if let Some(outline) = &mut st.outline {
            // New sections take the next indexes, in serial order.
            for s in &mut sections {
                let slot = outline.put(s);
                s.index = slot.index;
                slots.push(slot);
            }
            slots.sort_by_key(|s| s.index);
            removed_index = removed.iter().filter_map(|r| outline.remove(*r)).collect();
            removed_index.sort_unstable();
            last = outline.last();
        }
        Some(Batch {
            sections,
            removed,
            truncated: false,
            too_long: false,
            pending_question: question,
            outlined,
            slots,
            removed_index,
            last,
        })
    }

    /// Send the new Finished cards among `cards` to be summarised, once
    /// each: those not among the cards last broadcast, and, the first time
    /// the cards are found, only the newest.
    fn ask_summaries(&self, st: &mut TailState, window: &[Placed], cards: &[Section]) {
        let primed = std::mem::replace(&mut st.primed, true);
        let Some(jobs) = &self.jobs else {
            return;
        };
        // The pane's shown name, once a subscription has given it; the
        // agent's own name before.
        let agent = if st.live.name == LiveState::default().name {
            self.agent.clone()
        } else {
            st.live.name.clone()
        };
        let finished: Vec<&Section> = cards.iter().filter(|c| is_bookend(c)).collect();
        let new: Vec<&Section> = if primed {
            finished
                .into_iter()
                .filter(|c| !st.cards.contains_key(&c.serial.to_u64()))
                .collect()
        } else {
            finished.last().copied().into_iter().collect()
        };
        for card in new {
            let Some(key) = CardSummaries::key(card) else {
                continue;
            };
            let Some(reply) = reply_at(window, card.serial.line) else {
                continue;
            };
            if !self.summaries.ask(key) {
                continue;
            }
            let _ = jobs.send(SummaryJob {
                key,
                agent: agent.clone(),
                project: self.project.clone(),
                reply,
                wake: Arc::clone(&self.wake),
            });
        }
    }

    /// Check every `CHECK_INTERVAL`, or at once when woken, until nobody
    /// is subscribed; then leave `hub`.
    async fn run(self: Arc<Self>, hub: Weak<SessionHub>) {
        let mut tick = tokio::time::interval(CHECK_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = self.wake.notified() => {}
            }
            if let Some(hub) = hub.upgrade()
                && hub.leave_if_unused(&self)
            {
                return;
            }
            if hub.strong_count() == 0 {
                return;
            }
            let me = Arc::clone(&self);
            let batch = tokio::task::spawn_blocking(move || me.check())
                .await
                .ok()
                .flatten();
            if let Some(batch) = batch {
                let _ = self.tx.send(Arc::new(batch));
            }
        }
    }
}

/// The whole redacted reply that ends on `line`: the agent's text entries
/// before its last one there, back to the first entry that is not one.
fn reply_at(window: &[Placed], line: u32) -> Option<String> {
    let is_reply = |p: &Placed| {
        p.item.role == super::Role::Assistant
            && p.item.kind == super::EntryKind::Text
            && p.item.origin.is_none()
    };
    let end = window.iter().rposition(|p| p.line == line && is_reply(p))?;
    let start = window[..end]
        .iter()
        .rposition(|p| !is_reply(p))
        .map_or(0, |i| i + 1);
    let parts: Vec<String> = window[start..=end]
        .iter()
        .map(|p| super::redact::redact(&p.item.text))
        .collect();
    Some(parts.join("\n\n"))
}

/// The ids of the tool calls on `lines` that name a secret path.
fn secret_calls(
    reader: &dyn LineReader,
    path: &Path,
    index: &LineIndex,
    lines: &[u32],
) -> std::io::Result<HashSet<String>> {
    let mut ids = HashSet::new();
    for &line in lines {
        for (_, items) in read_lines(reader, path, index, line, line)? {
            ids.extend(
                items
                    .into_iter()
                    .filter(|i| i.kind == super::EntryKind::ToolUse)
                    .filter(|i| super::redact::mentions_secret_path(&i.text))
                    .filter_map(|i| i.call_id),
            );
        }
    }
    Ok(ids)
}

fn snapshot_of(st: &TailState) -> Snapshot {
    let (backend, index) = match &st.backend {
        TailBackend::Line(index) => (SnapshotBackend::Line(index.clone()), index.clone()),
        TailBackend::Agy { max_idx, .. } => (
            SnapshotBackend::Agy { max_idx: *max_idx },
            LineIndex::default(),
        ),
    };
    Snapshot {
        backend,
        index,
        approvals: approval_cards(st),
        live: st.live.clone(),
        too_long: st.too_long,
        secrets: st.secrets.clone(),
    }
}

fn approval_cards(st: &TailState) -> Vec<ApprovalCard> {
    st.approvals
        .iter()
        .map(|a| a.card(st.pending.contains(&a.confirm_id)))
        .collect()
}

/// The shared tailers (decision 3).
pub struct SessionHub {
    sessions: Arc<Sessions>,
    repowiki: Option<String>,
    journal: Journal,
    tailers: Mutex<HashMap<PathBuf, Arc<Tailer>>>,
    /// The Finished cards' summaries, kept across tailers.
    summaries: Arc<CardSummaries>,
    /// Where new Finished cards go to be summarised, once a summariser is
    /// set.
    jobs: Mutex<Option<mpsc::UnboundedSender<SummaryJob>>>,
}

impl SessionHub {
    /// A hub reading through `sessions`, linking `docs/…` paths to
    /// `repowiki`, with the journals in `sessions.state`.
    pub fn new(sessions: Arc<Sessions>, repowiki: Option<String>) -> Arc<Self> {
        let journal = Journal::new(sessions.state.clone());
        Arc::new(SessionHub {
            sessions,
            repowiki,
            journal,
            tailers: Mutex::new(HashMap::new()),
            summaries: Arc::new(CardSummaries::default()),
            jobs: Mutex::new(None),
        })
    }

    /// Summarise each new Finished card once with `summarizer`
    /// (`[summary] cards`), for the tailers started from now. Needs a
    /// tokio runtime; without one, nothing is summarised.
    pub fn summarize_cards_with(&self, summarizer: Arc<dyn CardSummarizer>) {
        let jobs = summaries::spawn_worker(summarizer, Arc::clone(&self.summaries));
        if jobs.is_none() {
            log::warn!("no runtime for the card summaries; cards are not summarised");
        }
        *self.jobs.lock().unwrap() = jobs;
    }

    /// The Finished cards' summaries.
    pub fn summaries(&self) -> &Arc<CardSummaries> {
        &self.summaries
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn sessions(&self) -> &Arc<Sessions> {
        &self.sessions
    }

    /// The tailer of `located`, the session of an `agent` in `cwd`, started
    /// if it is new, and a subscription to it.
    pub async fn subscribe(
        self: &Arc<Self>,
        agent: &str,
        cwd: Option<&str>,
        located: &Located,
    ) -> std::io::Result<(Arc<Tailer>, broadcast::Receiver<Arc<Batch>>, Snapshot)> {
        loop {
            // Subscribed under the hub's lock, so the tailer cannot leave
            // in between.
            {
                let tailers = self.tailers.lock().unwrap();
                if let Some(t) = tailers.get(&located.path) {
                    let (rx, snapshot) = t.subscribe();
                    return Ok((Arc::clone(t), rx, snapshot));
                }
            }
            let sessions = Arc::clone(&self.sessions);
            let linker = Linker::new(self.repowiki.as_deref(), cwd);
            let journal = self.journal.clone();
            let agent = agent.to_string();
            let at = located.clone();
            let summaries = Arc::clone(&self.summaries);
            let jobs = self.jobs.lock().unwrap().clone();
            let project = cwd
                .and_then(|c| Path::new(c).file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "this project".to_string());
            let opened = tokio::task::spawn_blocking(move || {
                Tailer::open(sessions, &agent, &at, linker, &journal)
                    .map(|t| t.with_summaries(summaries, jobs, &project))
            })
            .await
            .map_err(std::io::Error::other)??;
            let mut tailers = self.tailers.lock().unwrap();
            if tailers.contains_key(&located.path) {
                continue; // another subscriber started one meanwhile
            }
            let tailer = Arc::new(opened);
            tailers.insert(located.path.clone(), Arc::clone(&tailer));
            let (rx, snapshot) = tailer.subscribe();
            tokio::spawn(Arc::clone(&tailer).run(Arc::downgrade(self)));
            return Ok((tailer, rx, snapshot));
        }
    }

    /// The running tailer of the file at `path`, if any.
    pub fn tailer(&self, path: &Path) -> Option<Arc<Tailer>> {
        self.tailers.lock().unwrap().get(path).cloned()
    }

    /// The running tailers.
    pub fn tailers(&self) -> Vec<Arc<Tailer>> {
        self.tailers.lock().unwrap().values().cloned().collect()
    }

    /// Drop `tailer` if nobody is subscribed to it; whether it was.
    fn leave_if_unused(&self, tailer: &Arc<Tailer>) -> bool {
        let mut tailers = self.tailers.lock().unwrap();
        if tailer.tx.receiver_count() > 0 {
            return false;
        }
        if tailers
            .get(&tailer.path)
            .is_some_and(|t| Arc::ptr_eq(t, tailer))
        {
            tailers.remove(&tailer.path);
        }
        true
    }
}

#[cfg(test)]
mod tests;
