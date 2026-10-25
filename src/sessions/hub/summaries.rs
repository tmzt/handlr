//! Finished cards' summaries (plan 20261022_03, the L3 amendment of
//! 2026-10-24 12:06 EDT, and plan 20261023_05's CHANGES.md departure of
//! the same time: `[summary] cards`).
//!
//! With a [`CardSummarizer`] set on the hub, a tailer asks for each new
//! Finished card's summary once. The hub's worker asks the summariser, one
//! card at a time, and keeps the answer here by the card's serial and text.
//! Every card the hub makes, live or read from a span, then carries its
//! `points` and `spoken`; a tailer that sees a new answer recomputes its
//! cards, so the card's section is replaced (its `rev` bumped) for every
//! subscriber. A card with no answer (no Gemini, or an error) keeps both
//! fields empty, and is not asked again.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use libflankr_proto::common::{CardKind, Section};
use tokio::sync::{Notify, mpsc};

/// The most cards whose summaries are kept, the oldest asked dropped
/// first.
pub const CARD_SUMMARIES_MAX: usize = 1024;

/// A Finished card's summary: `CardBody::points` and `CardBody::spoken`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardSummary {
    pub points: Vec<String>,
    pub spoken: Option<String>,
}

/// What summarises a Finished card's reply: the daemon's Gemini, or a mock.
#[async_trait]
pub trait CardSummarizer: Send + Sync {
    /// The summary of `reply`, the whole redacted reply of the `agent` (its
    /// shown name) in `project`; `None` when there is none (no Gemini, or
    /// an error).
    async fn summarize_card(&self, agent: &str, project: &str, reply: &str) -> Option<CardSummary>;
}

/// A card's key: its serial and its text's hash, so a file read again with
/// other text at the same serial is not given another reply's summary.
pub type CardKey = (u64, u64);

/// One card to summarise.
pub struct SummaryJob {
    pub key: CardKey,
    pub agent: String,
    pub project: String,
    pub reply: String,
    /// The tailer to wake when the answer is kept.
    pub wake: Arc<Notify>,
}

/// The summaries asked for and answered.
#[derive(Debug, Default)]
pub struct CardSummaries {
    inner: Mutex<Store>,
}

#[derive(Debug, Default)]
struct Store {
    /// `None` while asked, or when there was no answer.
    by_key: HashMap<CardKey, Option<CardSummary>>,
    order: VecDeque<CardKey>,
    /// Bumped with each answer kept.
    generation: u64,
}

impl CardSummaries {
    /// A Finished card's key; `None` for any other section.
    pub fn key(section: &Section) -> Option<CardKey> {
        let card = section.card.as_ref()?;
        if card.card != CardKind::Finished {
            return None;
        }
        let mut h = DefaultHasher::new();
        card.text.hash(&mut h);
        Some((section.serial.to_u64(), h.finish()))
    }

    /// Give a Finished card its summary, when there is one.
    pub fn apply(&self, section: &mut Section) {
        let Some(key) = Self::key(section) else {
            return;
        };
        let store = self.inner.lock().unwrap();
        if let (Some(Some(summary)), Some(card)) = (store.by_key.get(&key), section.card.as_mut()) {
            card.points = summary.points.clone();
            card.spoken = summary.spoken.clone();
        }
    }

    /// Note `key` as asked: whether it had not been asked before.
    pub fn ask(&self, key: CardKey) -> bool {
        let mut store = self.inner.lock().unwrap();
        if store.by_key.contains_key(&key) {
            return false;
        }
        store.by_key.insert(key, None);
        store.order.push_back(key);
        while store.order.len() > CARD_SUMMARIES_MAX {
            if let Some(old) = store.order.pop_front() {
                store.by_key.remove(&old);
            }
        }
        true
    }

    /// Keep the answer for `key`, if it is still kept as asked.
    pub fn put(&self, key: CardKey, summary: CardSummary) {
        let mut store = self.inner.lock().unwrap();
        if let Some(slot) = store.by_key.get_mut(&key) {
            *slot = Some(summary);
            store.generation += 1;
        }
    }

    /// Bumped with each answer kept: a tailer that has seen another
    /// recomputes its cards.
    pub fn generation(&self) -> u64 {
        self.inner.lock().unwrap().generation
    }
}

/// The worker: each job's summary asked of `summarizer` in turn, kept in
/// `store`, and its tailer woken. Ends when every sender is gone.
pub fn spawn_worker(
    summarizer: Arc<dyn CardSummarizer>,
    store: Arc<CardSummaries>,
) -> Option<mpsc::UnboundedSender<SummaryJob>> {
    let handle = tokio::runtime::Handle::try_current().ok()?;
    let (tx, mut rx) = mpsc::unbounded_channel::<SummaryJob>();
    handle.spawn(async move {
        while let Some(job) = rx.recv().await {
            let answer = summarizer
                .summarize_card(&job.agent, &job.project, &job.reply)
                .await;
            if let Some(summary) = answer {
                store.put(job.key, summary);
                job.wake.notify_one();
            }
        }
    });
    Some(tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_proto::common::{CardBody, SectionKind, Serial};

    fn finished(line: u32, text: &str) -> Section {
        Section {
            serial: Serial::new(1, line, 194),
            kind: SectionKind::Card,
            entries: Vec::new(),
            card: Some(CardBody {
                card: CardKind::Finished,
                live: true,
                title: "Finished".into(),
                text: text.into(),
                confirm_id: None,
                about_plan: false,
                links: Vec::new(),
                points: Vec::new(),
                spoken: None,
            }),
            index: 0,
        }
    }

    #[test]
    fn a_card_is_asked_once_and_given_its_answer() {
        let store = CardSummaries::default();
        let mut card = finished(3, "Done.");
        let key = CardSummaries::key(&card).unwrap();
        assert!(store.ask(key));
        assert!(!store.ask(key), "asked once");
        store.apply(&mut card);
        assert!(card.card.as_ref().unwrap().points.is_empty());
        let before = store.generation();
        store.put(
            key,
            CardSummary {
                points: vec!["Did it".into()],
                spoken: Some("It is done.".into()),
            },
        );
        assert_eq!(store.generation(), before + 1);
        store.apply(&mut card);
        let body = card.card.as_ref().unwrap();
        assert_eq!(body.points, ["Did it"]);
        assert_eq!(body.spoken.as_deref(), Some("It is done."));
        // Another text at the same serial is another card.
        let mut other = finished(3, "Something else.");
        store.apply(&mut other);
        assert!(other.card.as_ref().unwrap().spoken.is_none());
        // Only Finished cards have keys.
        let mut question = finished(3, "Done.");
        question.card.as_mut().unwrap().card = CardKind::Question;
        assert_eq!(CardSummaries::key(&question), None);
    }

    #[test]
    fn the_oldest_asked_are_dropped_past_the_cap() {
        let store = CardSummaries::default();
        for line in 1..=(CARD_SUMMARIES_MAX as u32 + 1) {
            assert!(store.ask(CardSummaries::key(&finished(line, "x")).unwrap()));
        }
        assert!(store.ask(CardSummaries::key(&finished(1, "x")).unwrap()));
        assert!(!store.ask(CardSummaries::key(&finished(3, "x")).unwrap()));
    }
}
