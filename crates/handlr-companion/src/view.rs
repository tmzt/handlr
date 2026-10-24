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
//! For the app, the map is flattened into the entries of its non-card
//! sections and the cards placed among them ([`Sections::flatten`]); the
//! pinned card is the highest live one, the newest of its kind.

use std::collections::BTreeMap;

pub use libhandlr_proto::common::CardKind;
use libhandlr_proto::common::{Entry, EntryKind, Link, Role, Section, SectionKind, Serial};

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
}

/// The user's own words: a user text entry with no `origin`. An entry
/// with one (a task notification, a peer's message, a recap) is context,
/// never the user speaking.
pub fn is_user_said(e: &Entry) -> bool {
    e.role == Role::User && e.kind == EntryKind::Text && e.origin.is_none()
}

/// The sections of one agent view, by packed serial.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sections {
    map: BTreeMap<u64, Section>,
    /// The session file the sections are of (the serials' prefix), once
    /// known.
    file: Option<u32>,
}

impl Sections {
    /// Drop everything: a reset, or another session.
    pub fn clear(&mut self) {
        self.map.clear();
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
        let file = section.serial.file;
        match self.file {
            Some(held) if held != file => return false,
            Some(_) => {}
            None => self.file = Some(file),
        }
        self.map.insert(section.serial.to_u64(), section);
        true
    }

    /// Drop the section at `serial`, if held.
    pub fn remove(&mut self, serial: Serial) {
        if self.file == Some(serial.file) {
            self.map.remove(&serial.to_u64());
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
                    let card = AgentCard {
                        kind: body.card,
                        live: body.live,
                        at: entries.len(),
                        title: body.title.clone(),
                        text: body.text.clone(),
                        confirm_id: body.confirm_id,
                        about_plan: body.about_plan,
                        links: body.links.clone(),
                    };
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use libhandlr_proto::common::CardBody;

    pub fn entry(ts: &str, role: Role, kind: EntryKind, summary: &str) -> Entry {
        Entry {
            ts: ts.to_string(),
            role,
            kind,
            tool: (kind != EntryKind::Text).then(|| "Bash".to_string()),
            summary: summary.to_string(),
            links: Vec::new(),
            origin: None,
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
            }),
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
}
