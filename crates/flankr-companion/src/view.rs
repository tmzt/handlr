//! The agent view's sections (plan 20261023_03, the 14:54 EDT amendment,
//! A6 and A7, and the 22:55 EDT amendment, items 1–3).
//!
//! The daemon numbers every unit of the agent view (a turn's prompt, a
//! block of text, a tool call and its result, a context line, a priority
//! card) with a [`Serial`] derived from the session file, and runs the
//! four card rules itself. The client keeps every section it has been
//! given, for the life of the view, in a map sorted by serial
//! ([`Sections`]): no cap and no trimming, so scrolling back can page in
//! the whole session. Whatever order sections arrive in (a history page
//! after live deliveries, a resent section, a reconnect's backfill), the
//! view shows them in serial order, and a serial already held is replaced
//! in place, never added twice.
//!
//! For the app, the view is the sections in serial order, each its entries
//! or its card ([`Sections::view`]), with the pinned card's serial: the
//! highest live card, the newest of its kind (plan 20261022_03, the
//! 23:54 EDT amendment: the scrollback is a ribbon of sections). The map can
//! also be flattened into the entries of its non-card sections and the
//! cards placed among them ([`Sections::flatten`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use libflankr_proto::common::{CardBody, Entry, EntryKind, Link, Role, Section, Serial};
pub use libflankr_proto::common::{CardKind, SectionKind, Slot};

/// A card as the view shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCard {
    pub kind: CardKind,
    /// Live cards stand out; a resolved one fades to scrollback styling.
    pub live: bool,
    /// Where it sits: how many of the view's entries come before it.
    pub at: usize,
    /// "Approval needed", "<name> is asking", "Something broke",
    /// "Finished".
    pub title: String,
    pub text: String,
    /// The confirmation an Approval card answers.
    pub confirm_id: Option<u64>,
    /// A Question card's question is about a plan: it offers approve,
    /// discuss and implement in place of Yes and No (plan 20261022_03, the
    /// 21:57 EDT amendment).
    pub about_plan: bool,
    /// The repowiki links for the `docs/…` paths in `text`.
    pub links: Vec<Link>,
    /// A summary card's point-by-point summary (`CardBody::points`, plan
    /// 20261022_03, the L3 amendment of 2026-10-24 12:06 EDT): empty
    /// without the daemon's `[summary] cards`.
    pub points: Vec<String>,
    /// The text read aloud for the card (`CardBody::spoken`), when the
    /// daemon gave one: a summary card's short form, and its spoken form.
    pub spoken: Option<String>,
}

impl AgentCard {
    /// A summary card (the 12:03 EDT amendment): a Finished card, or any
    /// card the daemon summarised. It folds into its short form.
    pub fn is_summary(&self) -> bool {
        self.kind == CardKind::Finished || !self.points.is_empty() || self.spoken.is_some()
    }
}

/// A bookend: a Finished card's slot, which closes a section of the
/// session, for the scrollbar's section labels (plan 20261022_03, the L3
/// amendment of 2026-10-24 12:27 EDT). Placed from the outline before any
/// contents load; its text comes with the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookend {
    pub serial: Serial,
    /// The card's short summary (`CardBody::spoken`'s first line), once
    /// the card is held and summarised.
    pub summary: Option<String>,
    /// The phone's clock time of the reply the card closes, as "12:41",
    /// once that reply is held.
    pub time: Option<String>,
}

/// The phone's clock time, "H:MM", of an RFC 3339 time `ts`, for a phone
/// `local_offset` minutes ahead of UTC. `None` when `ts` is not one.
pub fn clock_time(ts: &str, local_offset: i16) -> Option<String> {
    let b = ts.as_bytes();
    let num = |i: usize, n: usize| -> Option<i32> {
        let part = ts.get(i..i + n)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    if b.get(10).is_none_or(|&c| c != b'T' && c != b' ') || b.get(13) != Some(&b':') {
        return None;
    }
    let (hour, minute) = (num(11, 2)?, num(14, 2)?);
    // The zone, after the seconds and any fraction.
    let zone = ts[16..].find(['Z', 'z', '+', '-']).map(|i| 16 + i)?;
    let offset = match b[zone] {
        b'Z' | b'z' => 0,
        sign => {
            let (h, m) = (num(zone + 1, 2)?, num(zone + 4, 2)?);
            let o = h * 60 + m;
            if sign == b'-' { -o } else { o }
        }
    };
    let local = (hour * 60 + minute - offset + i32::from(local_offset)).rem_euclid(24 * 60);
    Some(format!("{}:{:02}", local / 60, local % 60))
}

/// The user's own words: a user text entry with no `origin`. An entry
/// with one (a task notification, a peer's message, a recap) is context,
/// never the user speaking.
pub fn is_user_said(e: &Entry) -> bool {
    e.role == Role::User && e.kind == EntryKind::Text && e.origin.is_none()
}

/// What one section of the view shows: its entries, or its card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shows {
    Entries(Vec<Entry>),
    /// Its `at` is 0: [`flatten_view`] places it.
    Card(AgentCard),
    /// Outline mode (plan 20261024_01, decision 7): a slot whose contents
    /// are not held, with its kind and size for the height estimate.
    Pending {
        kind: SectionKind,
        lines: u32,
        chars: u32,
    },
}

/// One section of the view, as the app lays it out: one block of the
/// ribbon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewSection {
    /// Its place in the ribbon: the daemon's index in outline mode (plan
    /// 20261024_01, [`outline_view`]); from a daemon without outlines, its
    /// place among those held ([`section_index`]).
    pub index: u64,
    pub serial: Serial,
    pub shows: Shows,
}

/// The ribbon index of `section`, held `position`th among the view's
/// sections (0 the oldest held), for a daemon that sends no outline: that
/// position. In outline mode the daemon's index is used instead.
pub fn section_index(position: usize, section: &Section) -> u64 {
    let _ = section;
    position as u64
}

/// The view's sections flattened as [`Sections::flatten`] gives them: the
/// entries, the cards placed among them, and the pinned card's index.
pub fn flatten_view(
    sections: &[ViewSection],
    pinned: Option<Serial>,
) -> (Vec<Entry>, Vec<AgentCard>, Option<usize>) {
    let mut entries = Vec::new();
    let mut cards = Vec::new();
    let mut pin = None;
    for section in sections {
        match &section.shows {
            Shows::Entries(e) => entries.extend(e.iter().cloned()),
            Shows::Card(card) => {
                if pinned == Some(section.serial) {
                    pin = Some(cards.len());
                }
                cards.push(AgentCard {
                    at: entries.len(),
                    ..card.clone()
                });
            }
            Shows::Pending { .. } => {}
        }
    }
    (entries, cards, pin)
}

/// Where the app's window is on the ribbon, by index (plan 20261024_01,
/// decision 6): the indexes it shows, those within `KEEP_SCREENS` screens
/// of it (whose contents are fetched), and those within 20 screens (whose
/// contents are kept; the rest are forgotten). Each range is inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewWindow {
    pub shown: (u64, u64),
    /// Within 1 screen of the window: what the open view keeps under
    /// memory pressure's `Background` grade (plan 20261022_03, the L3
    /// amendment of 2026-10-24 12:21 EDT).
    pub close: (u64, u64),
    pub near: (u64, u64),
    pub keep: (u64, u64),
    /// The view is scrolled back, not following the bottom: a new outline
    /// is held aside until the jump line is tapped (CHANGES.md, 2026-10-24
    /// 00:36 EDT, items 3 and 4).
    pub scrolled_back: bool,
}

/// A session's outline (plan 20261024_01, decision 6): its slots by
/// index, the indexes known removed, and how far from 0 it is held with
/// no hole.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outline {
    slots: BTreeMap<u64, Slot>,
    /// Packed serial to index.
    index_of: HashMap<u64, u64>,
    removed: BTreeSet<u64>,
    last: Option<u64>,
    epoch: Option<u64>,
    /// Every index below it is held or known removed.
    contiguous: u64,
}

impl Outline {
    pub fn new(epoch: Option<u64>) -> Self {
        Outline {
            epoch,
            ..Outline::default()
        }
    }

    /// Insert or replace the slot at its index. A serial held at another
    /// index leaves it.
    pub fn put(&mut self, slot: Slot) {
        let key = slot.serial.to_u64();
        if let Some(old) = self.slots.insert(slot.index, slot)
            && old.serial != slot.serial
        {
            self.index_of.remove(&old.serial.to_u64());
        }
        if let Some(was) = self.index_of.insert(key, slot.index)
            && was != slot.index
        {
            self.slots.remove(&was);
        }
        self.removed.remove(&slot.index);
        self.advance();
    }

    /// The index no longer stands: its slot, if held, is returned.
    pub fn remove(&mut self, index: u64) -> Option<Slot> {
        let slot = self.slots.remove(&index);
        if let Some(s) = &slot {
            self.index_of.remove(&s.serial.to_u64());
        }
        self.removed.insert(index);
        self.advance();
        slot
    }

    fn advance(&mut self) {
        while self.slots.contains_key(&self.contiguous) || self.removed.contains(&self.contiguous) {
            self.contiguous += 1;
        }
    }

    pub fn set_last(&mut self, last: Option<u64>) {
        self.last = last;
    }

    /// The epoch, when one is given.
    pub fn set_epoch(&mut self, epoch: Option<u64>) {
        if epoch.is_some() {
            self.epoch = epoch;
        }
    }

    /// The session's highest index, as the daemon last said.
    pub fn last(&self) -> Option<u64> {
        self.last
    }

    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    /// The first index neither held nor known removed.
    pub fn contiguous(&self) -> u64 {
        self.contiguous
    }

    /// Every index up to `last` is held or known removed.
    pub fn complete(&self) -> bool {
        self.last.is_none_or(|l| self.contiguous > l)
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn get(&self, index: u64) -> Option<&Slot> {
        self.slots.get(&index)
    }

    /// The slot of the section `serial`, if held.
    pub fn slot_of(&self, serial: Serial) -> Option<&Slot> {
        self.index_of
            .get(&serial.to_u64())
            .and_then(|i| self.slots.get(i))
    }

    /// The slots, in index order.
    pub fn slots(&self) -> impl DoubleEndedIterator<Item = &Slot> {
        self.slots.values()
    }

    /// The slots from `from` to `to`, inclusive.
    pub fn range(&self, from: u64, to: u64) -> impl DoubleEndedIterator<Item = &Slot> {
        self.slots.range(from..=to.max(from)).map(|(_, s)| s)
    }

    /// The session file the slots are of, if any is held.
    pub fn file(&self) -> Option<u32> {
        self.slots.values().next().map(|s| s.serial.file)
    }

    /// Where a subscription resumes: the serial of the newest slot below
    /// the first hole (`after`), and that hole's index (`from`).
    pub fn resume(&self) -> Option<(Serial, u64)> {
        let (_, below) = self.slots.range(..self.contiguous).next_back()?;
        Some((below.serial, self.contiguous))
    }
}

/// The view in outline mode: every slot of `outline` in index order, each
/// its contents held in `sections` or [`Shows::Pending`], and the pinned
/// card's serial, as [`Sections::view`] gives them. `hide` drops a card
/// the user has just answered, and its slot.
pub fn outline_view(
    outline: &Outline,
    sections: &Sections,
    hide: impl Fn(&AgentCard) -> bool,
) -> (Vec<ViewSection>, Option<Serial>) {
    outline_view_appending(outline, sections, None, hide)
}

/// [`outline_view`], with the newer sections of an outline held aside
/// added below the end (plan 20261022_03, the 01:06 EDT amendment, item
/// 5): the slots of `newer`'s outline whose serials come after the
/// newest of `outline` (all of them when they are of another session
/// file), with their contents from `newer`'s sections, else from
/// `sections`, else pending. They keep their own indexes when those all
/// come after `outline`'s last; otherwise each is placed at `outline`'s
/// last index + 1 + its own, which keeps their order below the end.
pub fn outline_view_appending(
    outline: &Outline,
    sections: &Sections,
    newer: Option<(&Outline, &Sections)>,
    hide: impl Fn(&AgentCard) -> bool,
) -> (Vec<ViewSection>, Option<Serial>) {
    let mut out = Vec::with_capacity(outline.len());
    let mut pinned: Option<(CardKind, Serial)> = None;
    let mut push = |slot: &Slot, index: u64, held: Option<&Section>| {
        let shows = match held {
            Some(section) => match shows_of(section, &hide, &mut pinned) {
                Some(shows) => shows,
                None => return,
            },
            None => Shows::Pending {
                kind: slot.kind,
                lines: slot.lines,
                chars: slot.chars,
            },
        };
        out.push(ViewSection {
            index,
            serial: slot.serial,
            shows,
        });
    };
    for slot in outline.slots() {
        push(slot, slot.index, sections.get(slot.serial));
    }
    if let Some((new, new_sections)) = newer {
        let added = newer_slots(outline, new);
        let last = outline.slots().next_back().map(|s| s.index);
        let keep_own = match (last, added.first()) {
            (Some(last), Some(first)) => first.index > last,
            _ => true,
        };
        for slot in added {
            let index = match last {
                Some(last) if !keep_own => last + 1 + slot.index,
                _ => slot.index,
            };
            let held = new_sections
                .get(slot.serial)
                .or_else(|| sections.get(slot.serial));
            push(slot, index, held);
        }
    }
    (out, pinned.map(|(_, serial)| serial))
}

/// The slots of `new`, an outline held aside, that come after `outline`'s
/// newest (all of them when they are of another session file), in index
/// order: those [`outline_view_appending`] adds below the end, and whose
/// estimated sizes count in the scrollbar's length while held aside (plan
/// 20261022_03, the 11:09 EDT amendment, item 2).
pub fn newer_slots<'a>(outline: &Outline, new: &'a Outline) -> Vec<&'a Slot> {
    let newest = outline.slots().map(|s| s.serial.to_u64()).max();
    let other_file = matches!(
        (new.file(), outline.file()),
        (Some(a), Some(b)) if a != b
    );
    new.slots()
        .filter(|s| other_file || newest.is_none_or(|n| s.serial.to_u64() > n))
        .collect()
}

/// The end of an outline held aside, for the jump line's state (plan
/// 20261022_03, the 01:07 EDT amendment): its live cards and its newest
/// held section, in index order, at the outline's indexes. Only the
/// contents held in `sections` count; `hide` drops a card as in
/// [`outline_view`].
pub fn held_end(
    outline: &Outline,
    sections: &Sections,
    hide: impl Fn(&AgentCard) -> bool,
) -> Vec<ViewSection> {
    let mut held: Vec<(u64, &Section)> = sections
        .iter()
        .filter_map(|s| outline.slot_of(s.serial).map(|slot| (slot.index, s)))
        .collect();
    held.sort_by_key(|&(index, _)| index);
    let newest = held.last().map(|&(index, _)| index);
    let mut pinned = None;
    held.into_iter()
        .filter(|&(index, s)| Some(index) == newest || s.card.as_ref().is_some_and(|c| c.live))
        .filter_map(|(index, s)| {
            shows_of(s, &hide, &mut pinned).map(|shows| ViewSection {
                index,
                serial: s.serial,
                shows,
            })
        })
        .collect()
}

/// What `section` shows, or `None` for a card `hide` drops. A live card
/// ranks for the pin in `pinned`: the highest, the newest of its kind
/// (sections given oldest first).
fn shows_of(
    section: &Section,
    hide: &impl Fn(&AgentCard) -> bool,
    pinned: &mut Option<(CardKind, Serial)>,
) -> Option<Shows> {
    match (&section.kind, &section.card) {
        (SectionKind::Card, Some(body)) => {
            // `at` is a flattened view's: here it is 0, so a card stays as
            // it was when sections come before it.
            let card = card_of(body, 0);
            if hide(&card) {
                return None;
            }
            if card.live && pinned.is_none_or(|(kind, _)| card.kind <= kind) {
                *pinned = Some((card.kind, section.serial));
            }
            Some(Shows::Card(card))
        }
        // A card without a body, or a kind from a newer daemon: its
        // entries, if any, are shown as they are.
        _ => Some(Shows::Entries(section.entries.clone())),
    }
}

/// The sections of one agent view, by packed serial.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    map: BTreeMap<u64, Section>,
    /// The slot `rev` each section's contents were fetched at (outline
    /// mode, plan 20261024_01; 0 otherwise).
    revs: HashMap<u64, u32>,
    /// The session file the sections are of (the serials' prefix), once
    /// known.
    file: Option<u32>,
}

impl Sections {
    /// Drop everything: a reset, or another session.
    pub fn clear(&mut self) {
        self.map.clear();
        self.revs.clear();
        self.file = None;
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// The session file held, if known.
    pub fn file(&self) -> Option<u32> {
        self.file
    }

    /// Insert `section`, or replace the one with its serial. A section of
    /// another session file than the one held is ignored (`false`).
    pub fn put(&mut self, section: Section) -> bool {
        self.put_rev(section, 0)
    }

    /// As [`Sections::put`], its contents being of the slot's `rev`.
    pub fn put_rev(&mut self, section: Section, rev: u32) -> bool {
        let file = section.serial.file;
        match self.file {
            Some(held) if held != file => return false,
            Some(_) => {}
            None => self.file = Some(file),
        }
        let key = section.serial.to_u64();
        self.map.insert(key, section);
        self.revs.insert(key, rev);
        true
    }

    /// Drop the section at `serial`, if held.
    pub fn remove(&mut self, serial: Serial) {
        if self.file == Some(serial.file) {
            self.map.remove(&serial.to_u64());
            self.revs.remove(&serial.to_u64());
        }
    }

    /// The section at `serial`, if held.
    pub fn get(&self, serial: Serial) -> Option<&Section> {
        self.map.get(&serial.to_u64())
    }

    /// The slot `rev` the contents at `serial` were fetched at, if held.
    pub fn rev(&self, serial: Serial) -> Option<u32> {
        self.revs.get(&serial.to_u64()).copied()
    }

    /// Keeps only the sections `keep` says to.
    pub fn retain(&mut self, mut keep: impl FnMut(&Section) -> bool) {
        let revs = &mut self.revs;
        self.map.retain(|k, s| {
            let kept = keep(s);
            if !kept {
                revs.remove(k);
            }
            kept
        });
    }

    /// Takes in `other`'s sections with their revs, replacing those held
    /// at the same serials; those of another file are left out.
    pub fn merge(&mut self, other: Sections) {
        let Sections { map, revs, .. } = other;
        for (k, section) in map {
            let rev = revs.get(&k).copied().unwrap_or(0);
            self.put_rev(section, rev);
        }
    }

    /// The oldest serial held.
    pub fn oldest(&self) -> Option<Serial> {
        self.map.keys().next().map(|k| Serial::from_u64(*k))
    }

    /// The newest serial held.
    pub fn newest(&self) -> Option<Serial> {
        self.map.keys().next_back().map(|k| Serial::from_u64(*k))
    }

    /// The sections, in serial order.
    pub fn iter(&self) -> impl Iterator<Item = &Section> {
        self.map.values()
    }

    /// The sections on the session file's line of `serial`, in slot order:
    /// a card's reply shares its line.
    pub fn on_line(&self, serial: Serial) -> impl DoubleEndedIterator<Item = &Section> {
        let lo = Serial::new(serial.file, serial.line, 0).to_u64();
        let hi = Serial::new(serial.file, serial.line, u8::MAX).to_u64();
        self.map.range(lo..=hi).map(|(_, s)| s)
    }

    /// The view's sections in serial order, each its entries or its card,
    /// and the pinned
    /// card's serial: the highest live card, the newest of its kind.
    /// `hide` drops a card the user has just answered, and its section.
    pub fn view(&self, hide: impl Fn(&AgentCard) -> bool) -> (Vec<ViewSection>, Option<Serial>) {
        let mut out = Vec::with_capacity(self.map.len());
        // The pinned card's rank, newest first among equals.
        let mut pinned: Option<(CardKind, Serial)> = None;
        for (position, section) in self.map.values().enumerate() {
            let Some(shows) = shows_of(section, &hide, &mut pinned) else {
                continue;
            };
            out.push(ViewSection {
                index: section_index(position, section),
                serial: section.serial,
                shows,
            });
        }
        (out, pinned.map(|(_, serial)| serial))
    }

    /// The entries of the non-card sections, in serial order, and the
    /// cards placed among them (each `at` the number of entries before
    /// it), with the index of the pinned one: the highest live card, the
    /// newest of its kind. `hide` drops a card the user has just answered.
    pub fn flatten(
        &self,
        hide: impl Fn(&AgentCard) -> bool,
    ) -> (Vec<Entry>, Vec<AgentCard>, Option<usize>) {
        let mut entries = Vec::new();
        let mut cards = Vec::new();
        for section in self.map.values() {
            match (&section.kind, &section.card) {
                (SectionKind::Card, Some(body)) => {
                    let card = card_of(body, entries.len());
                    if !hide(&card) {
                        cards.push(card);
                    }
                }
                // A card without a body, or a kind from a newer daemon:
                // its entries, if any, are shown as they are.
                _ => entries.extend(section.entries.iter().cloned()),
            }
        }
        let pinned = cards
            .iter()
            .enumerate()
            .filter(|(_, c)| c.live)
            .min_by_key(|(i, c)| (c.kind, std::cmp::Reverse(*i)))
            .map(|(i, _)| i);
        (entries, cards, pinned)
    }
}

/// The card a card section's body shows, `at` its place among the entries.
fn card_of(body: &CardBody, at: usize) -> AgentCard {
    AgentCard {
        kind: body.card,
        live: body.live,
        at,
        title: body.title.clone(),
        text: body.text.clone(),
        confirm_id: body.confirm_id,
        about_plan: body.about_plan,
        links: body.links.clone(),
        points: body.points.clone(),
        spoken: body.spoken.clone(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn entry(ts: &str, role: Role, kind: EntryKind, summary: &str) -> Entry {
        Entry {
            ts: ts.to_string(),
            role,
            kind,
            tool: (kind != EntryKind::Text).then(|| "Bash".to_string()),
            summary: summary.to_string(),
            links: Vec::new(),
            origin: None,
            text: None,
        }
    }

    pub const FILE: u32 = 0x3fa9_c21e;

    pub fn text(line: u32, slot: u8, summary: &str) -> Section {
        Section {
            serial: Serial::new(FILE, line, slot),
            kind: SectionKind::Text,
            entries: vec![entry(
                &line.to_string(),
                Role::Assistant,
                EntryKind::Text,
                summary,
            )],
            card: None,
            index: 0,
        }
    }

    pub fn card(line: u32, slot: u8, kind: CardKind, live: bool) -> Section {
        Section {
            serial: Serial::new(FILE, line, slot),
            kind: SectionKind::Card,
            entries: Vec::new(),
            card: Some(CardBody {
                card: kind,
                live,
                title: format!("{kind:?}"),
                text: "t".to_string(),
                confirm_id: (kind == CardKind::Approval).then_some(u64::from(line)),
                about_plan: false,
                links: Vec::new(),
                points: Vec::new(),
                spoken: None,
            }),
            index: 0,
        }
    }

    fn summaries(s: &Sections) -> Vec<String> {
        s.flatten(|_| false)
            .0
            .into_iter()
            .map(|e| e.summary)
            .collect()
    }

    #[test]
    fn sections_are_held_in_serial_order_and_replaced_in_place() {
        let mut s = Sections::default();
        assert!(s.put(text(5, 0, "five")));
        assert!(s.put(text(2, 0, "two")));
        assert!(s.put(text(5, 1, "five b")));
        assert_eq!(summaries(&s), ["two", "five", "five b"]);
        // Resent: replaced, not added.
        assert!(s.put(text(2, 0, "two, grown")));
        assert_eq!(summaries(&s), ["two, grown", "five", "five b"]);
        assert_eq!(s.len(), 3);
        assert_eq!(s.oldest(), Some(Serial::new(FILE, 2, 0)));
        assert_eq!(s.newest(), Some(Serial::new(FILE, 5, 1)));
        s.remove(Serial::new(FILE, 5, 0));
        assert_eq!(summaries(&s), ["two, grown", "five b"]);
    }

    #[test]
    fn clock_times_are_the_phones() {
        assert_eq!(
            clock_time("2026-10-24T16:41:05.120Z", -240).as_deref(),
            Some("12:41")
        );
        assert_eq!(
            clock_time("2026-10-24T16:41:05+02:00", 0).as_deref(),
            Some("14:41")
        );
        assert_eq!(
            clock_time("2026-10-24T01:05:00-01:30", 60).as_deref(),
            Some("3:35")
        );
        assert_eq!(
            clock_time("2026-10-24T00:10:00Z", -60).as_deref(),
            Some("23:10")
        );
        assert_eq!(clock_time("12", 0), None);
        assert_eq!(clock_time("t5", 0), None);
    }

    #[test]
    fn a_lines_sections_are_found_by_serial() {
        let mut s = Sections::default();
        s.put(text(4, 0, "four"));
        s.put(text(5, 0, "five"));
        s.put(card(5, 194, CardKind::Finished, false));
        s.put(text(6, 0, "six"));
        let on: Vec<u8> = s
            .on_line(Serial::new(FILE, 5, 194))
            .map(|x| x.serial.slot)
            .collect();
        assert_eq!(on, [0, 194]);
    }

    #[test]
    fn another_files_sections_are_ignored() {
        let mut s = Sections::default();
        s.put(text(1, 0, "one"));
        let mut other = text(9, 0, "elsewhere");
        other.serial.file = FILE + 1;
        assert!(!s.put(other));
        assert_eq!(summaries(&s), ["one"]);
        s.clear();
        assert_eq!(s.file(), None);
    }

    #[test]
    fn cards_are_placed_among_the_entries_and_the_highest_live_is_pinned() {
        let mut s = Sections::default();
        s.put(text(1, 0, "a"));
        s.put(card(1, 130, CardKind::Broke, true));
        s.put(text(2, 0, "b"));
        s.put(card(2, 128, CardKind::Question, true));
        s.put(card(3, 64, CardKind::Approval, false));
        let (entries, cards, pinned) = s.flatten(|_| false);
        assert_eq!(entries.len(), 2);
        let placed: Vec<(CardKind, usize, bool)> =
            cards.iter().map(|c| (c.kind, c.at, c.live)).collect();
        assert_eq!(
            placed,
            [
                (CardKind::Broke, 1, true),
                (CardKind::Question, 2, true),
                (CardKind::Approval, 2, false)
            ]
        );
        assert_eq!(pinned, Some(1), "the question outranks the error");
        // Two live of a kind: the newest.
        s.put(card(4, 64, CardKind::Approval, true));
        s.put(card(5, 64, CardKind::Approval, true));
        let (_, cards, pinned) = s.flatten(|_| false);
        assert_eq!(cards[pinned.unwrap()].confirm_id, Some(5));
        // A hidden card is not placed.
        let (_, cards, _) = s.flatten(|c| c.kind == CardKind::Approval);
        assert!(cards.iter().all(|c| c.kind != CardKind::Approval));
    }

    #[test]
    fn the_view_is_the_sections_in_order_with_the_pinned_cards_serial() {
        let mut s = Sections::default();
        s.put(text(1, 0, "a"));
        s.put(card(1, 130, CardKind::Broke, true));
        s.put(text(2, 0, "b"));
        s.put(card(2, 128, CardKind::Question, true));
        s.put(card(3, 64, CardKind::Approval, false));
        let (sections, pinned) = s.view(|_| false);
        let serials: Vec<(u32, u8)> = sections
            .iter()
            .map(|v| (v.serial.line, v.serial.slot))
            .collect();
        assert_eq!(serials, [(1, 0), (1, 130), (2, 0), (2, 128), (3, 64)]);
        assert_eq!(pinned, Some(Serial::new(FILE, 2, 128)), "the question");
        let indices: Vec<u64> = sections.iter().map(|v| v.index).collect();
        assert_eq!(indices, [0, 1, 2, 3, 4], "the place among those held");
        // Flattened, the same as the map flattened.
        assert_eq!(flatten_view(&sections, pinned), s.flatten(|_| false));
        // A hidden card's section is not in the view; the pin moves on.
        let (sections, pinned) = s.view(|c| c.kind == CardKind::Question);
        assert_eq!(sections.len(), 4);
        assert_eq!(pinned, Some(Serial::new(FILE, 1, 130)));
        let indices: Vec<u64> = sections.iter().map(|v| v.index).collect();
        assert_eq!(indices, [0, 1, 2, 4], "a hidden card keeps its index");
        assert!(matches!(&sections[1].shows, Shows::Card(c) if c.kind == CardKind::Broke));
        assert!(matches!(&sections[0].shows, Shows::Entries(e) if e[0].summary == "a"));
    }
}
