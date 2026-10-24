//! Session subscriptions (plan 20261023_03, sections 4 and 5.2, as amended
//! at 14:54 and 22:55 EDT).
//!
//! A subscription names one pane and stays on it. Its task holds the
//! delivery waiting to be written and adds the tailer's batches to it; it
//! writes it when the last one has left the connection, 250 ms after the
//! last, and while the companion is awake. Past 200 sections or 256 KiB
//! of JSON the waiting delivery becomes a reset carrying the newest 50
//! (decision 9). Every 2 s, and at once on a herdr event for its pane or
//! a status change, it reads the pane again: a closed pane ends it, a new
//! session file resets it, and a different agent is checked against the
//! principal again (decision 10).
//!
//! In outline mode (plan 20261024_01, decision 3, as amended at 00:03
//! EDT) a delivery carries slots in place of sections: the first one and
//! each reset carry the outline from its start, at most `SLOTS_MAX` slots
//! a delivery, continued as soon as each has left until it is complete;
//! later ones carry new and replaced slots and removed indexes. Only the
//! newest `LIVE_INLINE` indexes' contents ride along; the rest are asked
//! for with `session.content`.
//!
//! The journal task records every confirmation that reaches a session's
//! pane in that session's approval journal (A4).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libhandlr_proto::common::{
    self, LIVE_INLINE, PendingQuestion, SECTIONS_MAX, SLOTS_MAX, SUBSCRIPTIONS_MAX, Section,
    SessionContentParams, SessionContentResult, SessionDelivery, SessionHistoryParams,
    SessionHistoryResult, SessionRef, SessionSubscribeParams, SessionSubscribeResult,
    SessionUnsubscribeParams, Slot, delivery_reasons, methods,
};
use libhandlr_proto::envelope::codes;
use libhandlr_proto::{ErrorObject, Id, Notification};
use serde_json::Value;
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

use super::conn::{self, Conn};
use super::{CoreEvent, Daemon};
use crate::core::agents::AgentRef;
use crate::core::confirm::{ConfirmEvent, Lifetime};
use crate::sessions::hub::{self, Batch, LineIndex, Snapshot, Tailer};
use crate::sessions::sections::REACH;
use crate::sessions::{Located, redact};

/// The least time between two deliveries of a subscription.
pub const GAP: Duration = Duration::from_millis(250);
/// How often a subscription reads its pane again.
pub const RECHECK: Duration = Duration::from_secs(2);
/// The most JSON one catch-up delivery carries.
pub const DELIVERY_BYTES_MAX: usize = 256 * 1024;
/// The sections a reset carries when a delivery outgrew its limits.
pub const RESET_SECTIONS: usize = 50;
/// How soon an outline's next chunk is tried once the last has left.
pub const OUTLINE_PACE: Duration = Duration::from_millis(20);

/// One subscription, as its connection holds it.
pub(super) struct Subscription {
    pub id: u64,
    cancel: CancellationToken,
    /// The tailer of the session it is on, if any.
    tailer: Mutex<Option<Arc<Tailer>>>,
    /// One history or content request at a time.
    history: tokio::sync::Mutex<()>,
    /// The file and epoch of the session the client was last given a
    /// reset or a change of session for: what `session.content` answers
    /// from.
    delivered: Mutex<Option<(u32, u64)>>,
}

impl Subscription {
    pub(super) fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// What a client has been given, or will be with the waiting delivery.
#[derive(Debug, Default)]
struct Waiting {
    sections: BTreeMap<u64, Section>,
    /// Each section's JSON length.
    sizes: BTreeMap<u64, usize>,
    removed: BTreeSet<u64>,
    /// Outline mode: slots and removed indexes by index, and the
    /// outline's highest index.
    slots: BTreeMap<u64, Slot>,
    removed_index: BTreeSet<u64>,
    last: Option<u64>,
    reset: bool,
    /// A change of session or reason to carry.
    session_changed: bool,
    status: Option<String>,
    question: Option<Option<PendingQuestion>>,
}

impl Waiting {
    fn is_empty(&self) -> bool {
        self.sections.is_empty()
            && self.removed.is_empty()
            && self.slots.is_empty()
            && self.removed_index.is_empty()
            && !self.reset
            && !self.session_changed
            && self.status.is_none()
            && self.question.is_none()
    }

    fn bytes(&self) -> usize {
        self.sizes.values().sum()
    }

    fn add(&mut self, section: Section) {
        let key = section.serial.to_u64();
        self.removed.remove(&key);
        self.sizes
            .insert(key, serde_json::to_string(&section).map_or(0, |s| s.len()));
        self.sections.insert(key, section);
    }

    fn remove(&mut self, key: u64) {
        self.sections.remove(&key);
        self.sizes.remove(&key);
        self.removed.insert(key);
    }

    fn add_slot(&mut self, slot: Slot) {
        self.removed_index.remove(&slot.index);
        self.slots.insert(slot.index, slot);
    }

    fn remove_index(&mut self, index: u64) {
        self.slots.remove(&index);
        self.removed_index.insert(index);
    }
}

/// Whether `index` is among the newest `LIVE_INLINE` of an outline whose
/// highest index is `last`.
fn inline(index: u64, last: Option<u64>) -> bool {
    last.is_some_and(|last| index.saturating_add(LIVE_INLINE) > last)
}

/// A subscription's task and its state.
struct Task {
    conn: Arc<Conn>,
    sub: Arc<Subscription>,
    pane: AgentRef,
    name: String,
    permitted: bool,
    located: Option<Located>,
    rx: Option<broadcast::Receiver<Arc<Batch>>>,
    session: Option<SessionRef>,
    reason: Option<String>,
    waiting: Waiting,
    /// What the client has, or will with the waiting delivery.
    status: String,
    question: Option<PendingQuestion>,
    /// The queue position of the last delivery written to the connection.
    last_seq: u64,
    last_sent: Option<Instant>,
    /// The backfill size asked for.
    sections: usize,
    /// Outline mode (plan 20261024_01).
    outline: bool,
    /// The next index of an outline still being sent, if one is.
    outline_next: Option<u64>,
    /// The file and epoch the subscription is on, once open.
    opened: Option<(u32, u64)>,
    /// Outline mode: the highest index the client has been told of.
    outline_last: Option<u64>,
}

/// `session.subscribe`: resolve the pane, check the principal, and answer
/// with the first delivery; the subscription's task starts once the
/// answer is written (4.4).
pub(super) async fn subscribe(
    conn: &Arc<Conn>,
    request_id: &Id,
    p: SessionSubscribeParams,
) -> Result<Value, ErrorObject> {
    let daemon = &conn.daemon;
    served(conn, "session.subscribe")?;
    if !(1..=SECTIONS_MAX).contains(&p.sections) {
        return Err(ErrorObject::new(
            codes::INVALID_PARAMS,
            format!("sections must be 1 to {SECTIONS_MAX}"),
        ));
    }
    if conn.subscription_count() >= SUBSCRIPTIONS_MAX {
        return Err(ErrorObject::new(
            codes::TOO_MANY_SUBSCRIPTIONS,
            format!("a connection may hold {SUBSCRIPTIONS_MAX} subscriptions"),
        ));
    }
    let snap = conn::snapshot(conn).await?;
    let pane_id = conn::resolve_target(daemon, &snap, &p.pane_id)?;
    let names = daemon.names(&snap);
    let Some(named) = names.get(&pane_id) else {
        return Err(ErrorObject::new(
            codes::UNKNOWN_AGENT,
            format!("no pane '{pane_id}'"),
        ));
    };
    if !conn.principal().may_reach(&named.agent.agent) {
        return Err(ErrorObject::new(
            codes::NOT_PERMITTED,
            format!("{} may not reach {}", conn.principal().id, named.shown),
        ));
    }
    let sub = Arc::new(Subscription {
        id: conn.next_subscription_id(),
        cancel: CancellationToken::new(),
        tailer: Mutex::new(None),
        history: tokio::sync::Mutex::new(()),
        delivered: Mutex::new(None),
    });
    let mut task = Task {
        conn: Arc::clone(conn),
        sub: Arc::clone(&sub),
        pane: named.agent.clone(),
        name: named.shown.clone(),
        permitted: true,
        located: None,
        rx: None,
        session: None,
        reason: None,
        waiting: Waiting::default(),
        status: conn::wire_status(&named.agent.status),
        question: None,
        last_seq: 0,
        last_sent: None,
        sections: p.sections,
        outline: p.outline,
        outline_next: None,
        opened: None,
        outline_last: None,
    };
    task.open(p.after, p.from).await;
    let delivery = task.take_delivery();
    let (ready, ready_rx) = oneshot::channel();
    conn.add_subscription(Arc::clone(&sub), request_id.clone(), ready);
    tokio::spawn(task.run(ready_rx));
    conn::ok(SessionSubscribeResult {
        subscription_id: sub.id,
        delivery,
    })
}

/// `session.unsubscribe`.
pub(super) fn unsubscribe(conn: &Conn, p: SessionUnsubscribeParams) -> Result<Value, ErrorObject> {
    match conn.remove_subscription(p.subscription_id) {
        Some(sub) => {
            sub.cancel();
            conn::ok(common::Empty {})
        }
        None => Err(not_found(p.subscription_id)),
    }
}

/// `session.history`: the page below `before` in the subscription's
/// current session file.
pub(super) async fn history(conn: &Conn, p: SessionHistoryParams) -> Result<Value, ErrorObject> {
    if !(1..=SECTIONS_MAX).contains(&p.sections) {
        return Err(ErrorObject::new(
            codes::INVALID_PARAMS,
            format!("sections must be 1 to {SECTIONS_MAX}"),
        ));
    }
    let Some(sub) = conn.subscription(p.subscription_id) else {
        return Err(not_found(p.subscription_id));
    };
    let _one = sub.history.lock().await;
    let tailer = sub.tailer.lock().unwrap().clone();
    let Some(tailer) = tailer.filter(|t| t.file == p.before.file) else {
        return Err(ErrorObject::new(
            codes::SESSION_CHANGED,
            "that serial is not of the subscription's session file; a reset follows",
        ));
    };
    let page = tokio::task::spawn_blocking(move || {
        let snap = tailer.snapshot();
        let src = tailer.source(&snap)?;
        hub::page(&src, Some(p.before), p.sections).ok()
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| ErrorObject::new(codes::INTERNAL_ERROR, "the session file could not be read"))?;
    conn::ok(SessionHistoryResult {
        sections: page.sections,
        start: page.start,
    })
}

/// `session.content`: an index range of the subscription's current
/// session file, as slots or sections (plan 20261024_01, decision 4, as
/// amended at 00:03 EDT). It counts against the same one-at-a-time limit
/// as `session.history`.
pub(super) async fn content(conn: &Conn, p: SessionContentParams) -> Result<Value, ErrorObject> {
    if !(1..=SECTIONS_MAX).contains(&p.limit) {
        return Err(ErrorObject::new(
            codes::INVALID_PARAMS,
            format!("limit must be 1 to {SECTIONS_MAX}"),
        ));
    }
    if let (Some(from), Some(to)) = (p.from, p.to)
        && from > to
    {
        return Err(ErrorObject::new(
            codes::INVALID_PARAMS,
            format!("from ({from}) is past to ({to})"),
        ));
    }
    let Some(sub) = conn.subscription(p.subscription_id) else {
        return Err(not_found(p.subscription_id));
    };
    let _one = sub.history.lock().await;
    let tailer = sub.tailer.lock().unwrap().clone();
    let delivered = *sub.delivered.lock().unwrap();
    let Some(tailer) = tailer.filter(|t| delivered == Some((t.file, t.epoch()))) else {
        return Err(ErrorObject::new(
            codes::SESSION_CHANGED,
            "the subscription's session file has changed; a reset follows",
        ));
    };
    let content = tokio::task::spawn_blocking(move || {
        tailer.content(p.from, p.to, p.limit, p.slots_only).ok()
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| ErrorObject::new(codes::INTERNAL_ERROR, "the session file could not be read"))?;
    conn::ok(SessionContentResult {
        sections: content.sections,
        slots: content.slots,
        removed_index: content.removed,
        first: 0,
        last: content.last,
        more: content.more,
    })
}

fn not_found(id: u64) -> ErrorObject {
    ErrorObject::new(
        codes::SUBSCRIPTION_NOT_FOUND,
        format!("no subscription {id} on this connection"),
    )
}

/// Subscriptions are served to companions and on the daemon socket, as
/// `session.peek` is (decision 10).
fn served(conn: &Conn, method: &str) -> Result<(), ErrorObject> {
    if conn.is_companion() || conn.is_local() {
        Ok(())
    } else {
        Err(ErrorObject::new(
            codes::NOT_PERMITTED,
            format!("{method} is served to companions and on the daemon socket"),
        ))
    }
}

impl Task {
    fn daemon(&self) -> &Arc<Daemon> {
        &self.conn.daemon
    }

    /// Locate the pane's session and make the first delivery: a backfill,
    /// only what follows `after` when that is still of this file, or a
    /// reason. In outline mode, the outline from `from` when `after` is
    /// of this file, else from 0.
    async fn open(&mut self, after: Option<common::Serial>, from: Option<u64>) {
        self.waiting = Waiting {
            reset: true,
            session_changed: true,
            status: Some(self.status.clone()),
            ..Waiting::default()
        };
        self.rx = None;
        self.set_tailer(None);
        self.located = None;
        self.session = None;
        self.outline_next = None;
        self.opened = None;
        self.outline_last = None;
        let daemon = Arc::clone(self.daemon());
        if !self.permitted {
            return self.no_session(delivery_reasons::NOT_PERMITTED);
        }
        if daemon.sessions.reader_for(&self.pane.agent).is_none() {
            return self.no_session(delivery_reasons::NO_READER);
        }
        let located = conn::sessions_here(&daemon)
            .then(|| daemon.sessions.locate(&self.pane).map(|(_, l)| l))
            .flatten();
        let Some(located) = located else {
            return self.no_session(delivery_reasons::NO_SESSION);
        };
        let subscribed = daemon
            .hub
            .subscribe(&self.pane.agent, self.pane.cwd.as_deref(), &located)
            .await;
        let (tailer, rx, _) = match subscribed {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Could not tail {}: {e}", located.path.display());
                return self.no_session(delivery_reasons::NO_SESSION);
            }
        };
        tailer.set_live(&self.name, &self.pane.status);
        // The journal's confirmations still waiting are live cards.
        for a in tailer.unresolved() {
            if daemon.broker.pending(a.confirm_id).is_some() {
                tailer.approval(a, true);
            }
        }
        let reading = Arc::clone(&tailer);
        let snap = match tokio::task::spawn_blocking(move || reading.fresh_snapshot()).await {
            Ok(snap) => snap,
            Err(_) => tailer.snapshot(),
        };
        self.session = Some(SessionRef {
            session_id: located.session_id.clone(),
            matched: located.matched.into(),
        });
        self.located = Some(located);
        self.rx = Some(rx);
        self.set_tailer(Some(Arc::clone(&tailer)));
        if snap.too_long {
            self.session = None;
            return self.no_session(delivery_reasons::TOO_LONG);
        }
        self.reason = None;
        self.question = snap.live.pending_question.clone();
        self.waiting.question = Some(self.question.clone());
        if self.outline {
            return self.open_outline(&tailer, after, from).await;
        }
        self.opened = Some((tailer.file, tailer.epoch()));
        let n = self.sections;
        let backfill = tokio::task::spawn_blocking(move || backfill(&tailer, &snap, after, n))
            .await
            .ok()
            .flatten();
        match backfill {
            Some((sections, reset)) => {
                self.waiting.reset = reset;
                for s in sections {
                    self.waiting.add(s);
                }
            }
            None => self.no_session(delivery_reasons::NO_SESSION),
        }
    }

    /// No session to deliver from, for `reason`: a reset with no sections.
    fn no_session(&mut self, reason: &str) {
        self.session = None;
        self.outline_next = None;
        self.reason = Some(reason.to_string());
        self.question = None;
        self.waiting = Waiting {
            reset: true,
            session_changed: true,
            status: Some(self.status.clone()),
            question: Some(None),
            ..Waiting::default()
        };
    }

    /// The first outline delivery: from `from` when `after` is of the
    /// tailer's file and `from` within its outline, else a reset from 0.
    /// A resumed outline also resends the slots within `REACH` lines
    /// before `after`, which may have changed, as a catch-up does.
    async fn open_outline(
        &mut self,
        tailer: &Arc<Tailer>,
        after: Option<common::Serial>,
        from: Option<u64>,
    ) {
        let reading = Arc::clone(tailer);
        let got = tokio::task::spawn_blocking(move || reading.outline().ok())
            .await
            .ok()
            .flatten();
        let Some((outline, epoch)) = got else {
            return self.no_session(delivery_reasons::NO_SESSION);
        };
        self.opened = Some((tailer.file, epoch));
        let resume = after
            .zip(from)
            .filter(|(a, f)| a.file == tailer.file && *f <= outline.len());
        self.waiting.reset = resume.is_none();
        if let Some((after, from)) = resume {
            let reach = after.line.saturating_sub(REACH);
            for i in (0..from).rev() {
                match outline.slot(i) {
                    Some(slot) if slot.serial.line >= reach => self.waiting.add_slot(slot),
                    Some(_) => break,
                    None => {}
                }
            }
        }
        self.waiting.last = outline.last();
        self.outline_next = Some(resume.map_or(0, |(_, f)| f));
        self.fill_chunk().await;
    }

    /// Add the outline's next chunk to the waiting delivery: slots up to
    /// `SLOTS_MAX` in all, the removed indexes among them, and, with the
    /// last chunk, the contents of the newest `LIVE_INLINE` indexes.
    async fn fill_chunk(&mut self) {
        let Some(next) = self.outline_next else {
            return;
        };
        let Some(tailer) = self.sub.tailer.lock().unwrap().clone() else {
            self.outline_next = None;
            return;
        };
        let reading = Arc::clone(&tailer);
        let got = tokio::task::spawn_blocking(move || reading.outline().ok())
            .await
            .ok()
            .flatten();
        let Some((outline, _)) = got.filter(|(_, e)| Some((tailer.file, *e)) == self.opened) else {
            self.outline_next = None;
            return; // a reset is on its way
        };
        let room = SLOTS_MAX.saturating_sub(self.waiting.slots.len()) as u64;
        if room == 0 {
            return;
        }
        let end = next.saturating_add(room).min(outline.len());
        for i in next..end {
            match outline.slot(i) {
                Some(slot) => self.waiting.add_slot(slot),
                None => self.waiting.remove_index(i),
            }
        }
        self.waiting.last = outline.last();
        if end < outline.len() {
            self.outline_next = Some(end);
            return;
        }
        self.outline_next = None;
        // The contents of every waiting slot at the end: this chunk's, and
        // those a resumed outline resends.
        let live: Vec<Slot> = self
            .waiting
            .slots
            .values()
            .filter(|s| inline(s.index, outline.last()))
            .copied()
            .collect();
        if live.is_empty() {
            return;
        }
        let reading = Arc::clone(&tailer);
        let sections = tokio::task::spawn_blocking(move || {
            let snap = reading.snapshot();
            reading.read_slots(&snap, &live).ok()
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        for s in sections {
            self.waiting.add(s);
        }
    }

    /// Send the outline's next chunk at once, once the last delivery has
    /// left.
    async fn continue_outline(&mut self) {
        if self.outline_next.is_none() || self.conn.asleep() {
            return;
        }
        if self.conn.written() < self.last_seq {
            return;
        }
        self.fill_chunk().await;
        if self.waiting.is_empty() {
            return;
        }
        let delivery = self.take_delivery();
        self.last_seq = self
            .conn
            .send_counted(&Notification::new(methods::SESSION_DELIVERY, delivery));
        self.last_sent = Some(Instant::now());
    }

    fn set_tailer(&self, tailer: Option<Arc<Tailer>>) {
        *self.sub.tailer.lock().unwrap() = tailer;
    }

    /// The waiting delivery, taken.
    fn take_delivery(&mut self) -> SessionDelivery {
        let w = std::mem::take(&mut self.waiting);
        if w.reset || w.session_changed {
            *self.sub.delivered.lock().unwrap() = self.opened.filter(|_| self.session.is_some());
        }
        let outlined = self.outline && self.session.is_some();
        if w.last.is_some() {
            self.outline_last = w.last;
        }
        SessionDelivery {
            subscription_id: self.sub.id,
            pane_id: self.pane.pane_id.clone(),
            agent: self.pane.agent.clone(),
            reset: w.reset,
            session: self.session.clone(),
            reason: self.reason.clone(),
            sections: w.sections.into_values().collect(),
            status: w.status,
            pending_question: w.question,
            removed: w
                .removed
                .into_iter()
                .map(common::Serial::from_u64)
                .collect(),
            slots: w.slots.into_values().collect(),
            removed_index: w.removed_index.into_iter().collect(),
            first: 0,
            last: self.outline_last.filter(|_| outlined),
        }
    }

    /// Add a batch to the waiting delivery.
    async fn take_batch(&mut self, batch: &Batch) {
        if batch.truncated {
            return self.open(None, None).await;
        }
        if batch.too_long {
            self.rx = None;
            self.set_tailer(None);
            return self.no_session(delivery_reasons::TOO_LONG);
        }
        if batch.pending_question != self.question {
            self.question = batch.pending_question.clone();
            self.waiting.question = Some(self.question.clone());
        }
        if self.outline {
            return self.take_outline_batch(batch).await;
        }
        for s in &batch.sections {
            let mut s = s.clone();
            s.index = 0; // indexes are for outline mode
            self.waiting.add(s);
        }
        for r in &batch.removed {
            self.waiting.remove(r.to_u64());
        }
        if self.waiting.sections.len() > SECTIONS_MAX || self.waiting.bytes() > DELIVERY_BYTES_MAX {
            self.reset_newest().await;
        }
    }

    /// Add an outline batch to the waiting delivery: its slots and
    /// removed indexes, and the contents of those among the newest
    /// `LIVE_INLINE`. A batch made before the outline was kept is already
    /// in it. Past `SLOTS_MAX` waiting slots, the outline is sent again
    /// from 0 as a reset.
    async fn take_outline_batch(&mut self, batch: &Batch) {
        if !batch.outlined || self.opened.is_none() {
            return;
        }
        for slot in &batch.slots {
            self.waiting.add_slot(*slot);
        }
        for s in &batch.sections {
            if inline(s.index, batch.last) {
                self.waiting.add(s.clone());
            }
        }
        for r in &batch.removed {
            self.waiting.remove(r.to_u64());
        }
        for &i in &batch.removed_index {
            self.waiting.remove_index(i);
        }
        self.waiting.last = batch.last;
        if self.waiting.slots.len() > SLOTS_MAX {
            let question = self.waiting.question.take();
            self.waiting = Waiting {
                reset: true,
                status: Some(self.status.clone()),
                question,
                ..Waiting::default()
            };
            self.outline_next = Some(0);
            self.fill_chunk().await;
        }
    }

    /// The waiting delivery outgrew one catch-up: a reset with the newest
    /// sections instead.
    async fn reset_newest(&mut self) {
        let Some(tailer) = self.sub.tailer.lock().unwrap().clone() else {
            return;
        };
        let page = tokio::task::spawn_blocking(move || {
            let snap = tailer.snapshot();
            let src = tailer.source(&snap)?;
            hub::page(&src, None, RESET_SECTIONS).ok()
        })
        .await
        .ok()
        .flatten();
        let question = self.waiting.question.take();
        self.waiting = Waiting {
            reset: true,
            status: Some(self.status.clone()),
            question,
            ..Waiting::default()
        };
        for s in page.map(|p| p.sections).unwrap_or_default() {
            self.waiting.add(s);
        }
    }

    /// Read the pane again. `false` when it is gone and the subscription
    /// ends.
    async fn recheck(&mut self) -> bool {
        let daemon = Arc::clone(self.daemon());
        let Ok(Ok(snap)) =
            tokio::time::timeout(super::NAME_LOOKUP_TIMEOUT, daemon.client.list_panes()).await
        else {
            return true; // herdr unreadable: carry on as before
        };
        let names = daemon.names(&snap);
        let Some(named) = names.get(&self.pane.pane_id) else {
            self.reason = Some(delivery_reasons::PANE_CLOSED.to_string());
            self.session = None;
            self.waiting = Waiting {
                reset: true,
                session_changed: true,
                ..Waiting::default()
            };
            return false;
        };
        let pane = named.agent.clone();
        self.name = named.shown.clone();
        let status = conn::wire_status(&pane.status);
        if status != self.status {
            self.status = status.clone();
            self.waiting.status = Some(status);
        }
        let agent_changed = pane.agent != self.pane.agent;
        let session_changed = pane.session != self.pane.session || pane.cwd != self.pane.cwd;
        self.pane = pane;
        if agent_changed {
            self.permitted = self.conn.principal().may_reach(&self.pane.agent);
            self.open(None, None).await;
            return true;
        }
        let tailer = self.sub.tailer.lock().unwrap().clone();
        if let Some(t) = &tailer {
            t.set_live(&self.name, &self.pane.status);
        }
        // A different file, as after `/clear`, or one found at last.
        let now = conn::sessions_here(&daemon)
            .then(|| daemon.sessions.locate(&self.pane).map(|(_, l)| l))
            .flatten();
        let moved = match (&self.located, &now) {
            (Some(a), Some(b)) => a.path != b.path,
            (None, Some(_)) => {
                self.permitted && self.reason.as_deref() != Some(delivery_reasons::TOO_LONG)
            }
            (Some(_), None) => true,
            (None, None) => false,
        };
        if moved || (session_changed && tailer.is_none() && now.is_some()) {
            self.open(None, None).await;
        }
        true
    }

    /// Write the waiting delivery if it may go now.
    fn flush_if_due(&mut self) {
        if self.waiting.is_empty() || self.conn.asleep() {
            return;
        }
        if self.conn.written() < self.last_seq {
            return; // the last one has not left yet: keep coalescing
        }
        if self.last_sent.is_some_and(|at| at.elapsed() < GAP) {
            return;
        }
        let delivery = self.take_delivery();
        self.last_seq = self
            .conn
            .send_counted(&Notification::new(methods::SESSION_DELIVERY, delivery));
        self.last_sent = Some(Instant::now());
    }

    /// The subscription's loop, from its answer being written to its end.
    async fn run(mut self, ready: oneshot::Receiver<()>) {
        let cancel = self.sub.cancel.clone();
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = ready => {}
        }
        self.last_sent = Some(Instant::now());
        let mut core = self.daemon().events.subscribe();
        let mut gate = tokio::time::interval(GAP);
        gate.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut recheck = tokio::time::interval_at(tokio::time::Instant::now() + RECHECK, RECHECK);
        recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let mut again = false;
            tokio::select! {
                _ = cancel.cancelled() => break,
                batch = recv(&mut self.rx) => match batch {
                    Ok(batch) => self.take_batch(&batch).await,
                    Err(broadcast::error::RecvError::Lagged(_)) => self.open(None, None).await,
                    Err(broadcast::error::RecvError::Closed) => self.rx = None,
                },
                _ = recheck.tick() => again = true,
                event = core.recv() => match event {
                    Ok(CoreEvent::Pane { pane_id, .. }) | Ok(CoreEvent::AgentStatus { pane_id, .. })
                        if pane_id == self.pane.pane_id => again = true,
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => again = true,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = gate.tick() => {}
                _ = tokio::time::sleep(OUTLINE_PACE), if self.outline_next.is_some() => {}
            }
            if again && !self.recheck().await {
                // The pane is gone: say so, and end.
                let delivery = self.take_delivery();
                self.conn
                    .send_counted(&Notification::new(methods::SESSION_DELIVERY, delivery));
                break;
            }
            self.continue_outline().await;
            self.flush_if_due();
        }
        self.conn.remove_subscription(self.sub.id);
    }
}

/// The next batch, or never when there is no tailer.
async fn recv(
    rx: &mut Option<broadcast::Receiver<Arc<Batch>>>,
) -> Result<Arc<Batch>, broadcast::error::RecvError> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// The first delivery's sections from `snap`: what follows `after` when
/// it can be continued, else the newest `n`. `true` with a reset.
fn backfill(
    tailer: &Tailer,
    snap: &Snapshot,
    after: Option<common::Serial>,
    n: usize,
) -> Option<(Vec<Section>, bool)> {
    let src = tailer.source(snap)?;
    match after {
        Some(after) => {
            let since = hub::since(&src, after, n).ok()?;
            Some((since.sections, since.reset))
        }
        None => Some((hub::page(&src, None, n).ok()?.sections, true)),
    }
}

/// Record each confirmation that reaches a session's pane in its approval
/// journal, and give it to the session's tailer (A4).
pub(super) async fn journal_confirmations(daemon: Arc<Daemon>) {
    let mut events = daemon.broker.subscribe();
    // Where each journalled confirmation went: its journal and its file.
    let mut journalled: HashMap<u64, (String, std::path::PathBuf)> = HashMap::new();
    loop {
        match events.recv().await {
            Ok(ConfirmEvent::Requested {
                view,
                reminder: false,
                ..
            }) => {
                if let Some(at) = journal_asked(&daemon, &view).await {
                    journalled.insert(view.confirm_id, at);
                }
            }
            Ok(ConfirmEvent::Resolved { view, outcome, .. }) => {
                let Some((session, path)) = journalled.remove(&view.confirm_id) else {
                    continue;
                };
                let outcome = conn::outcome_word(outcome);
                if let Err(e) = daemon
                    .hub
                    .journal()
                    .resolved(&session, view.confirm_id, &outcome)
                {
                    log::warn!("The approval journal of {session}: {e}");
                }
                if let Some(t) = daemon.hub.tailer(&path) {
                    t.resolved(view.confirm_id, &outcome);
                }
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => {
                log::warn!("The approval journal missed {n} confirmation event(s)");
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Journal a confirmation asked of `view`'s pane, when that pane has a
/// session here: the journal's name and the session file.
async fn journal_asked(
    daemon: &Arc<Daemon>,
    view: &crate::core::confirm::ConfirmView,
) -> Option<(String, std::path::PathBuf)> {
    if !conn::sessions_here(daemon) {
        return None;
    }
    let snap = tokio::time::timeout(super::NAME_LOOKUP_TIMEOUT, daemon.client.list_panes())
        .await
        .ok()?
        .ok()?;
    let names = daemon.names(&snap);
    let named = names.get(&view.pane_id)?;
    let (_, located) = daemon.sessions.locate(&named.agent)?;
    let session = hub::session_key(&located);
    let tailer = daemon.hub.tailer(&located.path);
    let line = match &tailer {
        Some(t) => t.newest_line(),
        None => {
            let path = located.path.clone();
            tokio::task::spawn_blocking(move || LineIndex::scan(&path).map(|(i, _)| i.lines()))
                .await
                .ok()?
                .ok()?
        }
    };
    let spoken = named.spoken.clone();
    let text = if view.message {
        super::companion::message_confirm_text(&spoken)
    } else {
        super::companion::confirm_text(
            &view.command,
            &spoken,
            view.handle
                .as_deref()
                .filter(|_| view.lifetime != Lifetime::VoiceTurn),
        )
    };
    let recorded = daemon.hub.journal().asked(
        &session,
        line,
        view.confirm_id,
        &redact::redact(&view.command),
        &redact::redact(&text),
    );
    match recorded {
        Ok(Some(approval)) => {
            if let Some(t) = tailer {
                t.approval(approval, true);
            }
            Some((session, located.path))
        }
        Ok(None) => None,
        Err(e) => {
            log::warn!("The approval journal of {session}: {e}");
            None
        }
    }
}
