//! Sections, their serials, and the card rules (plan 20261023_03, the
//! 14:54 EDT amendment, A1–A3, and the 22:55 EDT amendment).
//!
//! A session file's items become **sections**, one unit of the agent view
//! each: a `turn` (the user's prompt), a `text` block (consecutive
//! assistant text with no tool call between), a `tool` (a call and, once
//! it arrives, its result), a `context` entry, or a `card`. Every section
//! has a [`Serial`] from the file: the file name's hash, the 1-based line
//! it starts on (every line counts, whether or not anything reads it) and
//! a slot on that line.
//!
//! Slots, so that what a line holds and the cards after it never collide
//! (CHANGES.md, plan 20261023_03, step 3):
//!
//! | Slots | What |
//! |---|---|
//! | 0–63 | the line's own entries, in order; a line with more puts the rest into slot 63's section |
//! | 64–127 | Approval cards, in the order they were asked (the journal) |
//! | 128 | the Question card at a reply |
//! | 129 | the Broke card of a blocked spell |
//! | 130–193 | a Broke card at a failing result: 130 plus the result's slot |
//! | 194 | the Finished card at a reply |
//!
//! [`Builder`] reads items forward, line by line. Grouping looks back at
//! most [`REACH`] lines: a text entry joins the open text block, and a
//! result its call's section, only within it. A builder started `REACH`
//! lines before a span therefore groups the span's sections as one
//! started at the file's first line does, which is what lets history
//! pages, backfills and the live tail agree on every serial.
//!
//! [`cards`] is the four card rules of the companion's agent view (plan
//! 20261022_03, L3-2), moved here and made a pure function of the placed
//! items, the journal's approvals and, at the file's end, the pane's live
//! state. The same function finds live cards at the newest end and
//! resolved ones in history.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use libhandlr_proto::common::{CardBody, CardKind, Section, SectionKind, Serial};
use libherdr::protocol::AgentStatus;

use super::links::Linker;
use super::{EntryKind, Item, PendingQuestion, Role, pending, redact};

/// The last slot of a line's own entries.
pub const SLOT_BLOCK_MAX: u8 = 63;
/// The slots of Approval cards.
pub const SLOT_APPROVAL_FIRST: u8 = 64;
pub const SLOT_APPROVAL_LAST: u8 = 127;
/// The Question card at a reply.
pub const SLOT_QUESTION: u8 = 128;
/// The Broke card of a blocked spell.
pub const SLOT_BLOCKED: u8 = 129;
/// A Broke card at a failing result: this plus the result's slot.
pub const SLOT_BROKE_FIRST: u8 = 130;
/// The Finished card at a reply.
pub const SLOT_FINISHED: u8 = 194;

/// How far back, in lines, a section may be continued: a text entry
/// joins the open text block, a result its call's section, and a reply's
/// turn is known to have ended by the next entry, only within it.
pub const REACH: u32 = 256;
/// Tool calls remembered for redaction and their results' tool names
/// (decision 7).
pub const KNOWN_CALLS: usize = 512;
/// The longest text a Broke or Finished card carries, in characters.
pub const CARD_TEXT_MAX: usize = 300;

/// An item at its place in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub line: u32,
    /// Its place among the line's items, at most [`SLOT_BLOCK_MAX`].
    pub slot: u8,
    /// Redacted as a result whose call names a secret path is.
    pub item: Item,
}

/// Builds sections from a file's items, read forward.
#[derive(Debug)]
pub struct Builder {
    file: u32,
    linker: Linker,
    sections: BTreeMap<u64, Section>,
    changed: BTreeSet<u64>,
    /// The text block more text may join: its key and its last line.
    open_text: Option<(u64, u32)>,
    /// Tool calls within reach: their section's key and line.
    calls: HashMap<String, (u64, u32)>,
    /// The last `KNOWN_CALLS` calls: their tool and whether they name a
    /// secret path.
    known: HashMap<String, (Option<String>, bool)>,
    known_order: VecDeque<String>,
    /// The ids of calls anywhere in the file that name a secret path.
    secrets: HashSet<String>,
    last_line: u32,
}

impl Builder {
    /// A builder for the file whose name hashes to `file`; entries are
    /// linked by `linker`.
    pub fn new(file: u32, linker: Linker) -> Self {
        Builder {
            file,
            linker,
            sections: BTreeMap::new(),
            changed: BTreeSet::new(),
            open_text: None,
            calls: HashMap::new(),
            known: HashMap::new(),
            known_order: VecDeque::new(),
            secrets: HashSet::new(),
            last_line: 0,
        }
    }

    pub fn file(&self) -> u32 {
        self.file
    }

    pub fn linker(&self) -> &Linker {
        &self.linker
    }

    /// Calls known, from a scan of the whole file, to name a secret path:
    /// their results are redacted though the builder never sees the call.
    pub fn add_secrets<'a>(&mut self, ids: impl IntoIterator<Item = &'a String>) {
        self.secrets.extend(ids.into_iter().cloned());
    }

    /// The calls this builder has seen that name a secret path.
    pub fn secret_calls(&self) -> impl Iterator<Item = &String> {
        self.known
            .iter()
            .filter(|(_, (_, secret))| *secret)
            .map(|(id, _)| id)
    }

    /// Add line `line`'s items. Lines come in increasing order. The items
    /// come back placed, a secret result's text redacted.
    pub fn feed(&mut self, line: u32, items: Vec<Item>) -> Vec<Placed> {
        self.last_line = line;
        self.calls
            .retain(|_, (_, at)| line.saturating_sub(*at) <= REACH);
        let mut placed = Vec::with_capacity(items.len());
        for (i, mut item) in items.into_iter().enumerate() {
            let slot = i.min(SLOT_BLOCK_MAX as usize) as u8;
            let key = Serial::new(self.file, line, slot).to_u64();
            match item.kind {
                EntryKind::ToolUse => {
                    self.open_text = None;
                    if let Some(id) = item.call_id.clone() {
                        let secret = redact::mentions_secret_path(&item.text);
                        self.remember(id.clone(), item.tool.clone(), secret);
                        self.calls.insert(id, (key, line));
                    }
                    self.add(key, SectionKind::Tool, &item);
                }
                EntryKind::ToolResult => {
                    self.open_text = None;
                    let known = item.call_id.as_ref().and_then(|id| self.known.get(id));
                    match known {
                        Some((tool, secret)) => {
                            if item.tool.is_none() {
                                item.tool = tool.clone();
                            }
                            if *secret {
                                item.text = redact::REDACTED.to_string();
                            }
                        }
                        // A call this builder never saw: redacted when the
                        // file's scan found it names a secret path, or when
                        // the result's own text does (decision 7).
                        None => {
                            let secret_call = item
                                .call_id
                                .as_ref()
                                .is_some_and(|id| self.secrets.contains(id));
                            if secret_call || redact::mentions_secret_path(&item.text) {
                                item.text = redact::REDACTED.to_string();
                            }
                        }
                    }
                    let call = item
                        .call_id
                        .as_ref()
                        .and_then(|id| self.calls.get(id))
                        .map(|(k, _)| *k);
                    match call {
                        Some(k) => self.append(k, &item),
                        None => self.add(key, SectionKind::Tool, &item),
                    }
                }
                EntryKind::Text if item.origin.is_some() => {
                    self.open_text = None;
                    self.add(key, SectionKind::Context, &item);
                }
                EntryKind::Text if item.role == Role::Assistant => {
                    match self
                        .open_text
                        .filter(|(_, last)| line.saturating_sub(*last) <= REACH)
                    {
                        Some((k, _)) => {
                            self.append(k, &item);
                            self.open_text = Some((k, line));
                        }
                        None => {
                            self.add(key, SectionKind::Text, &item);
                            self.open_text = Some((key, line));
                        }
                    }
                }
                EntryKind::Text if item.role == Role::User => {
                    self.open_text = None;
                    self.add(key, SectionKind::Turn, &item);
                }
                EntryKind::Text => {
                    self.open_text = None;
                    self.add(key, SectionKind::Text, &item);
                }
            }
            placed.push(Placed { line, slot, item });
        }
        placed
    }

    fn remember(&mut self, id: String, tool: Option<String>, secret: bool) {
        if self.known.insert(id.clone(), (tool, secret)).is_none() {
            self.known_order.push_back(id);
        }
        while self.known_order.len() > KNOWN_CALLS {
            if let Some(old) = self.known_order.pop_front() {
                self.known.remove(&old);
            }
        }
    }

    fn entry(&self, item: &Item) -> libhandlr_proto::common::Entry {
        let mut entry = item.entry();
        self.linker.link_entry(&mut entry);
        entry
    }

    fn add(&mut self, key: u64, kind: SectionKind, item: &Item) {
        if self.sections.contains_key(&key) {
            // A line with more entries than slots: the rest join slot 63.
            return self.append(key, item);
        }
        let entry = self.entry(item);
        self.sections.insert(
            key,
            Section {
                serial: Serial::from_u64(key),
                kind,
                entries: vec![entry],
                card: None,
            },
        );
        self.changed.insert(key);
    }

    fn append(&mut self, key: u64, item: &Item) {
        let entry = self.entry(item);
        if let Some(section) = self.sections.get_mut(&key) {
            section.entries.push(entry);
            self.changed.insert(key);
        }
    }

    /// Every section built so far, by packed serial.
    pub fn sections(&self) -> &BTreeMap<u64, Section> {
        &self.sections
    }

    /// The sections added or changed since the last call, in order.
    pub fn take_changed(&mut self) -> Vec<Section> {
        let keys = std::mem::take(&mut self.changed);
        keys.into_iter()
            .filter_map(|k| self.sections.get(&k).cloned())
            .collect()
    }

    /// Forget the sections that can no longer change: those starting more
    /// than `REACH` lines before the newest, but for the open text block
    /// and the sections of calls within reach.
    pub fn prune(&mut self) {
        let keep_from = self.last_line.saturating_sub(REACH);
        let mut keep: BTreeSet<u64> = self.calls.values().map(|(k, _)| *k).collect();
        keep.extend(self.open_text.map(|(k, _)| k));
        self.sections
            .retain(|k, s| s.serial.line >= keep_from || keep.contains(k));
    }
}

/// An Approval card, from the journal and the broker (A4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCard {
    /// The session's newest line when it was asked.
    pub line: u32,
    pub slot: u8,
    pub confirm_id: u64,
    /// The question, as the companion is asked it.
    pub text: String,
    /// Still pending.
    pub live: bool,
}

/// The pane's state now, for the cards at the file's end.
#[derive(Debug, Clone, Copy)]
pub struct Live<'a> {
    /// The agent's shown name.
    pub name: &'a str,
    pub status: &'a AgentStatus,
    /// The file has been unwritten for [`pending::TURN_QUIET`].
    pub quiet: bool,
    /// The pending question, as [`pending::pending_question`] finds it.
    pub pending_question: Option<&'a PendingQuestion>,
    /// The newest line when the pane's current blocked spell began, while
    /// it is blocked.
    pub blocked_at: Option<u32>,
}

/// The cards among `placed` (in file order) and `approvals`, highest
/// priority first within a line. `live` is the pane's state when
/// `placed` reaches the file's end; without it only resolved cards are
/// found.
///
/// 1. **Approval needed:** each approval, live while pending.
/// 2. **The agent is asking:** at the pending question's reply, live; and,
///    resolved, at each reply whose final paragraph asks and whose next
///    entry (but for context, within `REACH` lines) is the user's.
/// 3. **Something broke:** at each error or failing-tests result, live
///    while it is the newest result of the last turn; and, live only,
///    after the line where a blocked spell began.
/// 4. **Finished:** at each reply that does not ask and whose next entry
///    (but for context, within `REACH`) is the user's, resolved; and at
///    the newest entry when it is a reply and the pane is idle, done or
///    blocked, or the file is quiet, live.
pub fn cards(
    file: u32,
    placed: &[Placed],
    approvals: &[ApprovalCard],
    live: Option<&Live>,
    linker: &Linker,
) -> Vec<Section> {
    let mut out = Vec::new();
    for a in approvals {
        out.push(card(
            Serial::new(file, a.line, a.slot),
            CardBody {
                card: CardKind::Approval,
                live: a.live,
                title: "Approval needed".to_string(),
                text: a.text.clone(),
                confirm_id: Some(a.confirm_id),
                about_plan: false,
                links: Vec::new(),
            },
        ));
    }
    let name = live.map_or("The agent", |l| l.name);
    let is_reply = |p: &Placed| {
        p.item.role == Role::Assistant && p.item.kind == EntryKind::Text && p.item.origin.is_none()
    };
    let last_reply = placed.iter().rposition(is_reply);
    for (i, p) in placed.iter().enumerate() {
        if !is_reply(p) {
            continue;
        }
        let next = placed[i + 1..].iter().find(|n| n.item.origin.is_none());
        // The pending question's reply: a live Question card.
        if let Some(q) = live.and_then(|l| l.pending_question)
            && Some(i) == last_reply
            && q.ts == p.item.ts
        {
            out.push(question_card(file, p, q, true, name, linker));
            continue;
        }
        match next {
            Some(n) if n.line.saturating_sub(p.line) <= REACH && n.item.is_user_said() => {
                match pending::question_in(&p.item) {
                    Some(q) => out.push(question_card(file, p, &q, false, name, linker)),
                    None => out.push(finished_card(file, p, false, linker)),
                }
            }
            None if live.is_some_and(finished) => {
                if pending::question_in(&p.item).is_none() {
                    out.push(finished_card(file, p, true, linker));
                }
            }
            _ => {}
        }
    }
    // 3. Something broke.
    let newest_result = placed
        .iter()
        .rposition(|p| p.item.kind == EntryKind::ToolResult);
    let last_user = placed.iter().rposition(|p| p.item.is_user_said());
    for (i, p) in placed.iter().enumerate() {
        if p.item.kind != EntryKind::ToolResult {
            continue;
        }
        let summary = p.item.entry().summary;
        if !(is_error(&summary) || fails_tests(&summary)) {
            continue;
        }
        let this_turn = last_user.is_none_or(|u| u < i);
        let is_live = live.is_some() && Some(i) == newest_result && this_turn;
        let text = summary.strip_prefix("error: ").unwrap_or(&summary);
        let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        let what = match &p.item.tool {
            Some(tool) => format!("{tool}: {}", first.trim()),
            None => first.trim().to_string(),
        };
        let text = super::cut(&what, CARD_TEXT_MAX);
        out.push(card(
            Serial::new(file, p.line, SLOT_BROKE_FIRST + p.slot),
            CardBody {
                card: CardKind::Broke,
                live: is_live,
                title: "Something broke".to_string(),
                links: linker.links(&text),
                text,
                confirm_id: None,
                about_plan: false,
            },
        ));
    }
    if let Some(at) = live.and_then(|l| l.blocked_at) {
        out.push(card(
            Serial::new(file, at, SLOT_BLOCKED),
            CardBody {
                card: CardKind::Broke,
                live: true,
                title: "Something broke".to_string(),
                text: format!("{name} is blocked and needs your input."),
                confirm_id: None,
                about_plan: false,
                links: Vec::new(),
            },
        ));
    }
    out.sort_by_key(|s| s.serial.to_u64());
    out.dedup_by_key(|s| s.serial.to_u64());
    out
}

/// Whether the pane's state ends the turn on its newest reply: idle,
/// done or blocked, or the file quiet.
fn finished(live: &Live) -> bool {
    matches!(
        live.status,
        AgentStatus::Idle | AgentStatus::Done | AgentStatus::Blocked
    ) || live.quiet
}

fn card(serial: Serial, body: CardBody) -> Section {
    Section {
        serial,
        kind: SectionKind::Card,
        entries: Vec::new(),
        card: Some(body),
    }
}

fn question_card(
    file: u32,
    reply: &Placed,
    q: &PendingQuestion,
    live: bool,
    name: &str,
    linker: &Linker,
) -> Section {
    card(
        Serial::new(file, reply.line, SLOT_QUESTION),
        CardBody {
            card: CardKind::Question,
            live,
            title: format!("{name} is asking"),
            text: q.text.clone(),
            confirm_id: None,
            about_plan: q.about_plan,
            links: linker.links(&q.text),
        },
    )
}

fn finished_card(file: u32, reply: &Placed, live: bool, linker: &Linker) -> Section {
    let summary = reply.item.entry().summary;
    let text = super::cut(first_paragraph(&summary), CARD_TEXT_MAX);
    card(
        Serial::new(file, reply.line, SLOT_FINISHED),
        CardBody {
            card: CardKind::Finished,
            live,
            title: "Finished".to_string(),
            links: linker.links(&text),
            text,
            confirm_id: None,
            about_plan: false,
        },
    )
}

/// A tool result the reader marked as an error (plan 20261022_03,
/// decision 2).
pub fn is_error(summary: &str) -> bool {
    summary.starts_with("error: ")
}

/// Whether a tool result reports failing tests: cargo's `test result:
/// FAILED`, a count such as `2 failed` (pytest, jest, go), or a line
/// starting `FAIL` (jest, go).
pub fn fails_tests(summary: &str) -> bool {
    if summary.contains("test result: FAILED") {
        return true;
    }
    if summary.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("FAIL ") || l.starts_with("--- FAIL") || l == "FAIL"
    }) {
        return true;
    }
    let words: Vec<&str> = summary.split_whitespace().collect();
    words.windows(2).any(|w| {
        let count = w[0].trim_start_matches(['(', '[', ',']);
        let failed = w[1].trim_end_matches([',', ';', '.', ')', ']']);
        failed.eq_ignore_ascii_case("failed") && count.parse::<u64>().is_ok_and(|n| n > 0)
    })
}

/// The first paragraph of a reply: up to its first blank line.
pub fn first_paragraph(text: &str) -> &str {
    let text = text.trim();
    let mut end = text.len();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            end = offset;
            break;
        }
        offset += line.len();
    }
    text[..end].trim()
}

/// The sections of `built` and `cards` merged in serial order.
pub fn merged(built: impl IntoIterator<Item = Section>, cards: Vec<Section>) -> Vec<Section> {
    let mut all: BTreeMap<u64, Section> =
        built.into_iter().map(|s| (s.serial.to_u64(), s)).collect();
    for c in cards {
        all.insert(c.serial.to_u64(), c);
    }
    all.into_values().collect()
}
