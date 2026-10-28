//! The terminal mode, the daemon's half
//! (docs/design/20261027_07_TERMINAL_MODE_PLAN.md, decisions 2, 5, 6 and
//! 8, steps 3 and 6, and the amendments of 14:23 and 20:44 EDT).
//!
//! A companion connected over the link opens a terminal on a session's
//! pane with `terminal.open`. Each open terminal has a poller, which reads
//! the pane's visible frame from herdr (`pane.read { source: "visible",
//! format: "ansi" }`) when herdr says the pane scrolled and every 2 s
//! regardless, at most 8 times a second, and sends what changed as a
//! `libflankr_term` frame of the link's terminal kind, diffed from the last
//! frame the phone acknowledged (`terminal.ack`).
//!
//! * **Where:** only to companions whose connection came over the Noise
//!   link, since frames need the link's terminal kind. A companion on the
//!   daemon socket is refused with `NOT_PERMITTED`; a harness client does
//!   not have the methods (`METHOD_NOT_FOUND`). Only for a herdr reached
//!   over its local socket: with a remote (SSH) herdr, `hello` reports no
//!   terminal and `terminal.open` is `HERDR_UNAVAILABLE`.
//! * **Who:** opening is gated as reaching an agent is: the pane's agent
//!   has an `[[agents]]` entry and the principal may reach it
//!   (`NOT_PERMITTED`); an unknown pane is `UNKNOWN_AGENT`.
//! * **Typing** (`terminal.input_arm`, `terminal.input`) needs `[terminal]
//!   input`, the agent's `terminal_input`, a principal that may approve,
//!   and an arm within the last 60 s (each accepted batch extends it to 60
//!   s from that batch). Keys are herdr's `send_keys` syntax (see
//!   [`parse_keys`]); `ctrl+c`, `ctrl+d`, `ctrl+z` and `ctrl+\` also need
//!   the agent's `terminal_signals`. Each accepted batch is a `notice` of
//!   kind `terminal_input` naming the agent and the key count, never the
//!   keys. The confirmation gate is not used for keys.
//! * **Scrolling** (`terminal.scroll`, the 2026-10-27 21:58 EDT amendment
//!   of docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md) is input: the
//!   same gates and arm as typing, and an accepted scroll extends the arm.
//!   It sets herdr's `pane.scroll` offset, so the pane scrolls on the Mac
//!   too, and the poller reads a frame at once. No notice is sent.
//! * **Ending:** `terminal.closed` is sent when the pane closes
//!   (`pane_closed`, or a read that fails on a pane no longer in herdr's
//!   snapshot), when the pane's agent is no longer the one it was opened
//!   on (`agent_changed`), and after `terminal.close` (`closed`). All of a
//!   connection's terminals end with it, without a notification.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libflankr_proto::common::{
    self, TERMINAL_INPUT_ARM_SECS, TERMINAL_INPUT_MAX, TerminalClosedReason, methods,
};
use libflankr_proto::companion;
use libflankr_proto::envelope::codes;
use libflankr_proto::{ErrorObject, Id, Notification};
use libflankr_term::{Frame, Mirror, parse_ansi};
use libherdr::protocol::{HerdrRequest, ReadFormat, ReadSource};
use libherdr::transport::HerdrTransport;
use serde_json::{Value, json};
use tokio::sync::{Notify, broadcast, oneshot, watch};
use tokio::task::AbortHandle;
use tokio::time::Instant;

use super::conn::{Conn, ok, resolve_target, snapshot};
use super::{CoreEvent, Daemon};
use crate::core::agents::{self, extract_agent_name_for_pane};
use crate::core::policy;
use crate::watch::pane_ids_match;

/// How often an open terminal is read without a `pane.scroll_changed`.
pub const IDLE_TICK: Duration = Duration::from_secs(2);
/// The least time between two reads of one terminal: 8 frames a second.
pub const MIN_GAP: Duration = Duration::from_millis(125);
/// The lines asked of herdr's visible frame: herdr's cap. The visible frame
/// is never taller than the pane, and the phone pans, so the rows hint does
/// not cut it.
const READ_LINES: usize = libflankr_term::MAX_ROWS;
/// The `notice.kind` of an accepted input batch (decision 6).
pub const NOTICE_TERMINAL_INPUT: &str = "terminal_input";
/// The most rows one `terminal.scroll` moves, either way.
pub const SCROLL_MAX_LINES: i32 = 1000;

/// The panes of every open terminal on every connection, counted, for the
/// herdr event subscription's `pane.scroll_changed` (decision 8). As with
/// the watch set, a change bumps a version the event supervisor reconnects
/// on, since herdr takes one `events.subscribe` per connection.
#[derive(Debug)]
pub struct TerminalPanes {
    panes: Mutex<BTreeMap<String, usize>>,
    version: watch::Sender<u64>,
}

impl Default for TerminalPanes {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalPanes {
    pub fn new() -> Self {
        TerminalPanes {
            panes: Mutex::new(BTreeMap::new()),
            version: watch::channel(0).0,
        }
    }

    /// One more terminal on `pane_id`.
    pub fn add(&self, pane_id: &str) {
        let first = {
            let mut panes = self.panes.lock().unwrap();
            let n = panes.entry(pane_id.to_string()).or_insert(0);
            *n += 1;
            *n == 1
        };
        if first {
            self.version.send_modify(|v| *v = v.wrapping_add(1));
        }
    }

    /// One terminal fewer on `pane_id`.
    pub fn remove(&self, pane_id: &str) {
        let last = {
            let mut panes = self.panes.lock().unwrap();
            match panes.get_mut(pane_id) {
                Some(n) if *n > 1 => {
                    *n -= 1;
                    false
                }
                Some(_) => {
                    panes.remove(pane_id);
                    true
                }
                None => false,
            }
        };
        if last {
            self.version.send_modify(|v| *v = v.wrapping_add(1));
        }
    }

    /// The panes with at least one open terminal, in order.
    pub fn panes(&self) -> Vec<String> {
        self.panes.lock().unwrap().keys().cloned().collect()
    }

    /// Fires whenever [`TerminalPanes::panes`] changes.
    pub fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.version.subscribe()
    }
}

/// Whether `transport` reaches herdr over its local socket (or is the
/// mock): the terminal mode's precondition (20:44 EDT amendment, item 2).
pub fn herdr_is_local(transport: &HerdrTransport) -> bool {
    !matches!(transport, HerdrTransport::RemoteSsh { .. })
}

/// What `hello` and `server.capabilities` say of the terminal mode on a
/// connection: offered on a link connection to a local herdr, with
/// `input` as `[terminal] input` has it.
pub fn offered(
    transport: &HerdrTransport,
    over_link: bool,
    input: bool,
) -> Option<companion::TerminalInfo> {
    (over_link && herdr_is_local(transport)).then_some(companion::TerminalInfo {
        input,
        mouse: false,
    })
}

/// [`offered`] for `daemon`.
pub fn info(daemon: &Daemon, over_link: bool) -> Option<companion::TerminalInfo> {
    offered(daemon.client.transport(), over_link, daemon.terminal_input)
}

/// One open terminal.
struct Terminal {
    id: u32,
    pane_id: String,
    /// The pane's agent when it was opened (lowercased, as the policy has
    /// it).
    agent: String,
    state: Mutex<State>,
    poller: Mutex<Option<AbortHandle>>,
    /// Wakes the poller for a read now (after a scroll), within
    /// [`MIN_GAP`].
    poke: Notify,
}

struct State {
    mirror: Mirror,
    /// The ANSI text last parsed: an unchanged read is not parsed again.
    last_text: Option<String>,
    /// Input is let through until then (decision 6).
    armed_until: Option<Instant>,
    /// The pane's scroll offset, in rows above the bottom, as last known.
    scroll_offset: u64,
    /// The furthest back it can scroll, when herdr has said.
    scroll_max: Option<u64>,
}

impl State {
    fn armed(&self) -> bool {
        self.armed_until.is_some_and(|until| Instant::now() < until)
    }
}

impl Terminal {
    fn abort_poller(&self) {
        if let Some(task) = self.poller.lock().unwrap().take() {
            task.abort();
        }
    }
}

/// A connection's terminals, by id. Ids start at 1 on each connection.
#[derive(Default)]
pub(super) struct Terminals {
    next: AtomicU32,
    open: Mutex<HashMap<u32, Arc<Terminal>>>,
}

impl Terminals {
    fn get(&self, id: u32) -> Option<Arc<Terminal>> {
        self.open.lock().unwrap().get(&id).cloned()
    }

    fn remove(&self, id: u32) -> Option<Arc<Terminal>> {
        self.open.lock().unwrap().remove(&id)
    }

    /// The terminal on `pane_id`, or a new one; `true` when it is new.
    fn get_or_insert(&self, pane_id: &str, agent: &str) -> (Arc<Terminal>, bool) {
        let mut open = self.open.lock().unwrap();
        if let Some(t) = open.values().find(|t| t.pane_id == pane_id) {
            return (Arc::clone(t), false);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let t = Arc::new(Terminal {
            id,
            pane_id: pane_id.to_string(),
            agent: agent.to_string(),
            state: Mutex::new(State {
                mirror: Mirror::new(id),
                last_text: None,
                armed_until: None,
                scroll_offset: 0,
                scroll_max: None,
            }),
            poller: Mutex::new(None),
            poke: Notify::new(),
        });
        open.insert(id, Arc::clone(&t));
        (t, true)
    }

    /// The connection ended: stop every poller, without notifications.
    pub(super) fn close_all(&self, daemon: &Daemon) {
        let all: Vec<Arc<Terminal>> = self.open.lock().unwrap().drain().map(|(_, t)| t).collect();
        for t in all {
            t.abort_poller();
            daemon.terminal_panes.remove(&t.pane_id);
        }
    }
}

fn not_permitted(message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(codes::NOT_PERMITTED, message.into())
}

fn herdr_unavailable(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(codes::HERDR_UNAVAILABLE, format!("herdr: {e}"))
}

/// Where `terminal.*` is served: a companion over the link, to a local
/// herdr. `METHOD_NOT_FOUND` handles the harness before this.
fn served(conn: &Conn) -> Result<(), ErrorObject> {
    if !conn.is_link() {
        return Err(not_permitted(
            "terminal.* is served to companions over the link only",
        ));
    }
    if !herdr_is_local(conn.daemon.client.transport()) {
        return Err(ErrorObject::new(
            codes::HERDR_UNAVAILABLE,
            "the terminal mode needs herdr's local socket; this daemon reaches herdr over SSH",
        ));
    }
    Ok(())
}

/// Read `pane_id`'s visible frame as ANSI text.
async fn read_text(daemon: &Daemon, pane_id: &str) -> Result<String, libherdr::HerdrError> {
    let read = daemon
        .client
        .inspect_pane_as(pane_id, READ_LINES, ReadSource::Visible, ReadFormat::Ansi)
        .await?;
    Ok(agents::pane_text(&read).unwrap_or_default().to_string())
}

/// `terminal.open` (decision 2).
pub(super) async fn open(
    conn: &Arc<Conn>,
    request_id: &Id,
    raw: Value,
) -> Result<Value, ErrorObject> {
    served(conn)?;
    let p: common::TerminalOpenParams = super::conn::params(raw)?;
    let daemon = &conn.daemon;
    let snap = snapshot(conn).await?;
    let pane_id = resolve_target(daemon, &snap, &p.pane_id)?;
    if daemon.names(&snap).get(&pane_id).is_none() {
        return Err(ErrorObject::new(
            codes::UNKNOWN_AGENT,
            format!("no pane '{pane_id}'"),
        ));
    }
    let agent = extract_agent_name_for_pane(&snap, &pane_id);
    policy::reach(conn.principal(), &daemon.agents, &agent, &pane_id)
        .map_err(|denied| not_permitted(denied.to_string()))?;

    let (term, new) = conn.terminals.get_or_insert(&pane_id, &agent);
    let text = match read_text(daemon, &pane_id).await {
        Ok(text) => text,
        Err(e) => {
            if new {
                conn.terminals.remove(term.id);
            }
            return Err(herdr_unavailable(e));
        }
    };
    let grid = parse_ansi(&text);
    let scroll = snapshot_scroll(&snap, &pane_id);
    let first = {
        let mut state = term.state.lock().unwrap();
        (state.scroll_offset, state.scroll_max) = scroll.unwrap_or((0, None));
        // A second open: the phone starts again, so a full frame.
        state.mirror.reset();
        state.last_text = Some(text);
        state.mirror.update(grid)
    };
    let Some(first) = first else {
        // A new or reset mirror always sends.
        return Err(ErrorObject::new(codes::INTERNAL_ERROR, "no first frame"));
    };
    if new {
        daemon.terminal_panes.add(&pane_id);
    }
    // The first frame follows the answer, so the phone knows the id.
    let (ready, after) = oneshot::channel();
    conn.on_answered(request_id.clone(), ready);
    term.abort_poller();
    let task = tokio::spawn(poll(
        Arc::clone(conn),
        Arc::clone(&term),
        after,
        first.clone(),
    ));
    *term.poller.lock().unwrap() = Some(task.abort_handle());
    log::info!(
        "Terminal {} open on pane '{pane_id}' ({agent}) for {}",
        term.id,
        conn.principal().id
    );
    ok(common::TerminalOpenResult {
        terminal_id: term.id,
        cols: first.cols,
        rows: first.rows,
        cursor: false,
        source: Default::default(),
    })
}

/// `terminal.close`: `{}`, also for a terminal already gone.
pub(super) fn close(conn: &Conn, raw: Value) -> Result<Value, ErrorObject> {
    served(conn)?;
    let p: common::TerminalCloseParams = super::conn::params(raw)?;
    if let Some(term) = conn.terminals.remove(p.terminal_id) {
        term.abort_poller();
        conn.daemon.terminal_panes.remove(&term.pane_id);
        conn.send(&Notification::new(
            methods::TERMINAL_CLOSED,
            common::TerminalClosedParams {
                terminal_id: term.id,
                reason: TerminalClosedReason::Closed,
            },
        ));
    }
    ok(common::Empty {})
}

/// `terminal.ack`, a notification: later frames are diffed from `seq`.
pub(super) fn ack(conn: &Conn, raw: Value) {
    let Ok(p) = super::conn::params::<common::TerminalAckParams>(raw) else {
        log::debug!("Ignoring a terminal.ack without valid params");
        return;
    };
    if let Some(term) = conn.terminals.get(p.terminal_id) {
        term.state.lock().unwrap().mirror.ack(p.seq);
    }
}

/// The checks every keystroke passes before the arm (decision 6), giving
/// the terminal and its agent's `terminal_signals`.
fn input_allowed(conn: &Conn, terminal_id: u32) -> Result<(Arc<Terminal>, bool), ErrorObject> {
    served(conn)?;
    let Some(term) = conn.terminals.get(terminal_id) else {
        return Err(not_permitted(format!(
            "no terminal {terminal_id} is open on this connection"
        )));
    };
    let daemon = &conn.daemon;
    if !daemon.terminal_input {
        return Err(not_permitted(
            "typing into a terminal is off ([terminal] input)",
        ));
    }
    let principal = conn.principal();
    let cfg = policy::reach(principal, &daemon.agents, &term.agent, &term.pane_id)
        .map_err(|denied| not_permitted(denied.to_string()))?;
    if !cfg.terminal_input {
        return Err(not_permitted(format!(
            "typing into {}'s terminal is off ([[agents]] terminal_input)",
            term.agent
        )));
    }
    if !principal.can_approve {
        return Err(not_permitted(format!(
            "{} may not type into a terminal: it may not approve",
            principal.id
        )));
    }
    Ok((term, cfg.terminal_signals))
}

fn arm_window() -> Duration {
    Duration::from_secs(TERMINAL_INPUT_ARM_SECS as u64)
}

/// `terminal.input_arm`.
pub(super) fn input_arm(conn: &Conn, raw: Value) -> Result<Value, ErrorObject> {
    let p: common::TerminalInputArmParams = super::conn::params(raw)?;
    let (term, _) = input_allowed(conn, p.terminal_id)?;
    term.state.lock().unwrap().armed_until = Some(Instant::now() + arm_window());
    log::info!(
        "Terminal {} on pane '{}' armed for input by {}",
        term.id,
        term.pane_id,
        conn.principal().id
    );
    ok(common::TerminalInputArmResult {
        expires_in_s: TERMINAL_INPUT_ARM_SECS,
    })
}

fn require_armed(term: &Terminal) -> Result<(), ErrorObject> {
    if term.state.lock().unwrap().armed() {
        Ok(())
    } else {
        Err(not_permitted(format!(
            "terminal {} is not armed for input (terminal.input_arm)",
            term.id
        )))
    }
}

/// herdr's `pane.scroll` for `pane_id`: libherdr has no request of its
/// own for it, so a custom one.
pub fn scroll_request(pane_id: &str, offset_from_bottom: u64) -> HerdrRequest {
    HerdrRequest::Custom(json!({
        "method": "pane.scroll",
        "params": {"pane_id": pane_id, "offset_from_bottom": offset_from_bottom},
    }))
}

/// The offset `lines` rows from `current` (positive back into the
/// scrollback), `lines` held to [`SCROLL_MAX_LINES`] either way, kept
/// between 0 and `max` when it is known.
pub fn scroll_target(current: u64, max: Option<u64>, lines: i32) -> u64 {
    let lines = lines.clamp(-SCROLL_MAX_LINES, SCROLL_MAX_LINES);
    let target = if lines >= 0 {
        current.saturating_add(lines as u64)
    } else {
        current.saturating_sub(lines.unsigned_abs() as u64)
    };
    max.map_or(target, |max| target.min(max))
}

/// A `scroll` object's offset and its max.
fn scroll_info(scroll: &Value) -> Option<(u64, Option<u64>)> {
    let offset = scroll.get("offset_from_bottom")?.as_u64()?;
    let max = scroll.get("max_offset_from_bottom").and_then(Value::as_u64);
    Some((offset, max))
}

/// The offset and max in herdr's answer to `pane.scroll`, read leniently:
/// `pane.scroll` (herdr's `{ "pane": PaneInfo }`) or a bare `scroll`.
pub fn scroll_from_response(result: &Value) -> Option<(u64, Option<u64>)> {
    result
        .get("pane")
        .and_then(|pane| pane.get("scroll"))
        .or_else(|| result.get("scroll"))
        .and_then(scroll_info)
}

/// `pane_id`'s scroll offset and max in a `list_panes` snapshot, where
/// herdr gives them.
fn snapshot_scroll(snap: &Value, pane_id: &str) -> Option<(u64, Option<u64>)> {
    agents::collect_panes(snap)
        .into_iter()
        .find(|pane| {
            pane.get("pane_id")
                .and_then(Value::as_str)
                .is_some_and(|id| pane_ids_match(id, pane_id))
        })
        .and_then(|pane| pane.get("scroll"))
        .and_then(scroll_info)
}

/// `terminal.scroll`: gated and armed as `terminal.input` is; `lines == 0`
/// answers the offset without asking herdr.
pub(super) async fn scroll(conn: &Arc<Conn>, raw: Value) -> Result<Value, ErrorObject> {
    let p: common::TerminalScrollParams = super::conn::params(raw)?;
    let (term, _) = input_allowed(conn, p.terminal_id)?;
    require_armed(&term)?;
    let (current, max) = {
        let state = term.state.lock().unwrap();
        (state.scroll_offset, state.scroll_max)
    };
    if p.lines == 0 {
        return ok(common::TerminalScrollResult {
            offset_from_bottom: current,
            max_offset_from_bottom: max,
        });
    }
    let target = scroll_target(current, max, p.lines);
    let answer = conn
        .daemon
        .client
        .send_request(&scroll_request(&term.pane_id, target))
        .await
        .map_err(herdr_unavailable)?;
    let (offset, max) = match scroll_from_response(&answer) {
        Some((offset, reported)) => (offset, reported.or(max)),
        None => (target, max),
    };
    {
        let mut state = term.state.lock().unwrap();
        state.scroll_offset = offset;
        state.scroll_max = max;
        state.armed_until = Some(Instant::now() + arm_window());
    }
    term.poke.notify_one();
    log::debug!(
        "Terminal {} on pane '{}' scrolled {} row(s) to {offset} above the bottom",
        term.id,
        term.pane_id,
        p.lines
    );
    ok(common::TerminalScrollResult {
        offset_from_bottom: offset,
        max_offset_from_bottom: max,
    })
}

/// `terminal.input`.
pub(super) async fn input(conn: &Arc<Conn>, raw: Value) -> Result<Value, ErrorObject> {
    let p: common::TerminalInputParams = super::conn::params(raw)?;
    let (term, signals) = input_allowed(conn, p.terminal_id)?;
    require_armed(&term)?;
    let keys = parse_keys(&p.keys, signals).map_err(|e| match e {
        KeyError::Signal(_) => not_permitted(e.to_string()),
        _ => ErrorObject::new(codes::INVALID_PARAMS, e.to_string()),
    })?;
    let daemon = &conn.daemon;
    daemon
        .client
        .send_keys(&term.pane_id, &keys.join(" "))
        .await
        .map_err(herdr_unavailable)?;
    term.state.lock().unwrap().armed_until = Some(Instant::now() + arm_window());
    let count = keys.len();
    log::info!(
        "{} typed {count} key(s) into terminal {} on pane '{}' ({})",
        conn.principal().id,
        term.id,
        term.pane_id,
        term.agent
    );
    let names = daemon.names_for(&term.pane_id).await;
    let spoken = names
        .as_ref()
        .map(|n| n.spoken.clone())
        .unwrap_or_else(|| crate::core::naming::capitalise(&term.agent));
    let plural = if count == 1 { "key" } else { "keys" };
    let notice = companion::Notice {
        id: conn.next_notice.fetch_add(1, Ordering::Relaxed) + 1,
        kind: NOTICE_TERMINAL_INPUT.to_string(),
        pane_id: term.pane_id.clone(),
        name: names.map(|n| n.name),
        text: format!(
            "Typed {count} {plural} into {}'s terminal.",
            crate::core::naming::capitalise(&spoken)
        ),
        spoken: Some(spoken),
    };
    conn.notices.lock().unwrap().queue(notice);
    super::companion::flush_notices(conn);
    ok(common::Empty {})
}

/// End `term` for `reason`: the poller's own exit.
fn closed(conn: &Conn, term: &Terminal, reason: TerminalClosedReason) {
    if conn.terminals.remove(term.id).is_none() {
        return;
    }
    conn.daemon.terminal_panes.remove(&term.pane_id);
    log::info!(
        "Terminal {} on pane '{}' ended: {reason:?}",
        term.id,
        term.pane_id
    );
    conn.send(&Notification::new(
        methods::TERMINAL_CLOSED,
        common::TerminalClosedParams {
            terminal_id: term.id,
            reason,
        },
    ));
}

/// Why a pane's terminal must end, from a fresh snapshot: the pane is gone,
/// or its agent is not the one the terminal was opened on.
async fn ended(daemon: &Daemon, term: &Terminal) -> Option<TerminalClosedReason> {
    let snap = daemon.client.list_panes().await.ok()?;
    let present = agents::agents(&snap)
        .iter()
        .any(|a| pane_ids_match(&a.pane_id, &term.pane_id));
    if !present {
        return Some(TerminalClosedReason::PaneClosed);
    }
    (extract_agent_name_for_pane(&snap, &term.pane_id) != term.agent)
        .then_some(TerminalClosedReason::AgentChanged)
}

/// What an event means for `term`'s poller.
enum Seen {
    Nothing,
    Changed,
    /// A `pane_updated` for the pane: its agent may have changed.
    Updated,
    Closed,
    /// The event bus is gone.
    Gone,
}

fn seen(term: &Terminal, event: Result<CoreEvent, broadcast::error::RecvError>) -> Seen {
    match event {
        Ok(CoreEvent::Output { pane_id }) if pane_ids_match(&pane_id, &term.pane_id) => {
            Seen::Changed
        }
        Ok(CoreEvent::Pane { pane_id, closed }) if pane_ids_match(&pane_id, &term.pane_id) => {
            if closed {
                Seen::Closed
            } else {
                Seen::Updated
            }
        }
        Ok(_) => Seen::Nothing,
        Err(broadcast::error::RecvError::Lagged(_)) => Seen::Changed,
        Err(broadcast::error::RecvError::Closed) => Seen::Gone,
    }
}

/// One terminal's poller: the first frame once the open is answered, then
/// a read on each `pane.scroll_changed` (coalesced, at most one per
/// [`MIN_GAP`]) and each [`IDLE_TICK`], which also resends the newest
/// frame while the phone has not acknowledged it.
async fn poll(conn: Arc<Conn>, term: Arc<Terminal>, ready: oneshot::Receiver<()>, first: Frame) {
    let mut events = conn.daemon.events.subscribe();
    let _ = ready.await;
    conn.send_terminal(first.encode());
    let mut tick = tokio::time::interval_at(Instant::now() + IDLE_TICK, IDLE_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_read = Instant::now();
    loop {
        let idle = tokio::select! {
            _ = tick.tick() => true,
            _ = term.poke.notified() => {
                // A scroll: the visible frame follows the offset.
                tokio::time::sleep_until(last_read + MIN_GAP).await;
                false
            }
            event = events.recv() => {
                let mut what = seen(&term, event);
                if matches!(what, Seen::Changed | Seen::Updated) {
                    // Coalesce what arrives until the next read is due.
                    tokio::time::sleep_until(last_read + MIN_GAP).await;
                    loop {
                        match events.try_recv() {
                            Ok(e) => match seen(&term, Ok(e)) {
                                Seen::Nothing | Seen::Changed => {}
                                other => {
                                    if !matches!(what, Seen::Updated) || matches!(other, Seen::Closed) {
                                        what = other;
                                    }
                                }
                            },
                            Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                            Err(_) => break,
                        }
                    }
                }
                match what {
                    Seen::Nothing => continue,
                    Seen::Changed => false,
                    Seen::Updated => {
                        if let Some(reason) = ended(&conn.daemon, &term).await {
                            return closed(&conn, &term, reason);
                        }
                        false
                    }
                    Seen::Closed => return closed(&conn, &term, TerminalClosedReason::PaneClosed),
                    Seen::Gone => return,
                }
            }
        };
        last_read = Instant::now();
        let text = match read_text(&conn.daemon, &term.pane_id).await {
            Ok(text) => text,
            Err(e) => {
                if let Some(reason) = ended(&conn.daemon, &term).await {
                    return closed(&conn, &term, reason);
                }
                log::debug!("Terminal {} could not read its pane: {e}", term.id);
                continue;
            }
        };
        let frame = {
            let mut state = term.state.lock().unwrap();
            let frame = if state.last_text.as_deref() == Some(text.as_str()) {
                None
            } else {
                let grid = parse_ansi(&text);
                state.last_text = Some(text);
                state.mirror.update(grid)
            };
            match frame {
                Some(frame) => Some(frame),
                None if idle => state.mirror.retransmit(),
                None => None,
            }
        };
        if let Some(frame) = frame {
            conn.send_terminal(frame.encode());
        }
    }
}

/// Why [`parse_keys`] refused a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    Empty,
    TooLong(usize),
    /// A control character, or a key or modifier the terminal does not
    /// send.
    Invalid(String),
    /// A signal key without the agent's `terminal_signals`.
    Signal(String),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::Empty => f.write_str("no keys"),
            KeyError::TooLong(n) => write!(
                f,
                "{n} bytes of keys; at most {TERMINAL_INPUT_MAX} in one terminal.input"
            ),
            KeyError::Invalid(key) => write!(f, "the key {key:?} is not sent to a terminal"),
            KeyError::Signal(key) => write!(
                f,
                "{key} is a signal key, and this agent's terminal_signals is off"
            ),
        }
    }
}

/// The named keys a terminal sends (decision 6), as herdr names them.
const NAMED_KEYS: [&str; 10] = [
    "enter",
    "escape",
    "esc",
    "tab",
    "backspace",
    "space",
    "up",
    "down",
    "left",
    "right",
];
/// The `ctrl+` keys that signal the pane's process (decision 6).
const SIGNAL_KEYS: [&str; 4] = ["ctrl+c", "ctrl+d", "ctrl+z", "ctrl+\\"];
/// Modifier prefixes refused: only `ctrl+` combos are sent.
const OTHER_MODIFIERS: [&str; 6] = ["alt+", "shift+", "meta+", "super+", "cmd+", "option+"];

/// `keys` as herdr's `send_keys` keys, one per element, joined with spaces
/// when sent (libherdr splits them again on whitespace). The syntax is
/// herdr's: whitespace-separated tokens, each a named key ([`NAMED_KEYS`],
/// any case), `ctrl+<a-z>`, or printable text. herdr takes one character
/// per text token, so a longer text token is sent as one key per character
/// (`ls` is `l s`); a space is the `space` key. Control characters, other
/// modifiers (`alt+`, `shift+` …), other named keys (`f1`, `home` …) and
/// other `ctrl+` combos are [`KeyError::Invalid`]; [`SIGNAL_KEYS`] are
/// [`KeyError::Signal`] unless `signals`.
pub fn parse_keys(keys: &str, signals: bool) -> Result<Vec<String>, KeyError> {
    if keys.len() > TERMINAL_INPUT_MAX {
        return Err(KeyError::TooLong(keys.len()));
    }
    let mut out = Vec::new();
    for token in keys.split_whitespace() {
        let lower = token.to_lowercase();
        if NAMED_KEYS.contains(&lower.as_str()) {
            out.push(lower);
        } else if SIGNAL_KEYS.contains(&lower.as_str()) {
            if !signals {
                return Err(KeyError::Signal(lower));
            }
            out.push(lower);
        } else if let Some(rest) = lower.strip_prefix("ctrl+") {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_lowercase() => out.push(lower),
                _ => return Err(KeyError::Invalid(token.to_string())),
            }
        } else if OTHER_MODIFIERS.iter().any(|m| lower.starts_with(m)) {
            return Err(KeyError::Invalid(token.to_string()));
        } else if let Some(c) = token.chars().find(|c| c.is_control()) {
            return Err(KeyError::Invalid(format!("{c:?}")));
        } else {
            out.extend(token.chars().map(String::from));
        }
    }
    if out.is_empty() {
        return Err(KeyError::Empty);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(s: &str) -> Vec<String> {
        parse_keys(s, false).unwrap()
    }

    #[test]
    fn keys_are_herdr_tokens_and_text_is_one_key_per_character() {
        assert_eq!(keys("l s enter"), ["l", "s", "enter"]);
        assert_eq!(keys("ls Enter"), ["l", "s", "enter"]);
        assert_eq!(
            keys("git space status"),
            ["g", "i", "t", "space", "s", "t", "a", "t", "u", "s"]
        );
        assert_eq!(
            keys("ESC up Down tab backspace"),
            ["esc", "up", "down", "tab", "backspace"]
        );
        assert_eq!(keys("ctrl+a ctrl+R"), ["ctrl+a", "ctrl+r"]);
        assert_eq!(keys("A"), ["A"]);
        assert_eq!(keys("+ é"), ["+", "é"]);
    }

    #[test]
    fn control_characters_other_keys_and_modifiers_are_refused() {
        for bad in [
            "\u{1b}[A",
            "a\u{7f}",
            "ctrl+1",
            "ctrl+enter",
            "alt+x",
            "shift+tab",
            "ctrl+",
        ] {
            assert!(
                matches!(parse_keys(bad, true), Err(KeyError::Invalid(_))),
                "{bad:?}"
            );
        }
        assert_eq!(parse_keys("  ", false), Err(KeyError::Empty));
        let long = "a".repeat(TERMINAL_INPUT_MAX + 1);
        assert_eq!(
            parse_keys(&long, true),
            Err(KeyError::TooLong(TERMINAL_INPUT_MAX + 1))
        );
        assert!(parse_keys(&"a".repeat(TERMINAL_INPUT_MAX), false).is_ok());
    }

    #[test]
    fn signal_keys_need_terminal_signals() {
        for signal in ["ctrl+c", "CTRL+D", "ctrl+z", "ctrl+\\"] {
            assert!(
                matches!(parse_keys(signal, false), Err(KeyError::Signal(_))),
                "{signal}"
            );
            assert_eq!(parse_keys(signal, true).unwrap(), [signal.to_lowercase()]);
        }
    }

    #[test]
    fn a_scroll_is_herdrs_pane_scroll_with_the_offset_clamped() {
        let rpc = scroll_request("w1:p1", 12).to_jsonrpc();
        assert_eq!(rpc.method, "pane.scroll");
        assert_eq!(
            rpc.params,
            json!({"pane_id": "w1:p1", "offset_from_bottom": 12})
        );

        assert_eq!(scroll_target(0, None, 3), 3);
        assert_eq!(scroll_target(5, None, -3), 2);
        assert_eq!(scroll_target(2, None, -10), 0);
        assert_eq!(scroll_target(0, Some(4), 10), 4);
        assert_eq!(scroll_target(0, None, i32::MAX), 1000);
        assert_eq!(scroll_target(5000, None, i32::MIN), 4000);
    }

    #[test]
    fn herdrs_scroll_answer_is_read_leniently() {
        let pane = json!({"type": "pane_info", "pane": {"pane_id": "w1:p1",
            "scroll": {"offset_from_bottom": 7, "max_offset_from_bottom": 300, "viewport_rows": 40}}});
        assert_eq!(scroll_from_response(&pane), Some((7, Some(300))));
        let bare = json!({"scroll": {"offset_from_bottom": 2}});
        assert_eq!(scroll_from_response(&bare), Some((2, None)));
        // The mock's echo of the request: nothing to read.
        let echo =
            json!({"method": "pane.scroll", "params": {"pane_id": "p1", "offset_from_bottom": 3}});
        assert_eq!(scroll_from_response(&echo), None);
    }

    #[test]
    fn terminal_panes_count_their_terminals() {
        let panes = TerminalPanes::new();
        let changes = panes.subscribe_changes();
        panes.add("w1:p1");
        panes.add("w1:p1");
        assert_eq!(*changes.borrow(), 1);
        panes.remove("w1:p1");
        assert_eq!(panes.panes(), ["w1:p1"]);
        panes.remove("w1:p1");
        assert!(panes.panes().is_empty());
        assert_eq!(*changes.borrow(), 2);
        panes.remove("w1:p1");
        assert_eq!(*changes.borrow(), 2);
    }

    #[test]
    fn a_remote_herdr_has_no_terminal() {
        // Constructed only: nothing is run or connected.
        let remote = HerdrTransport::remote_ssh("nobody@nowhere.invalid");
        assert!(!herdr_is_local(&remote));
        assert_eq!(offered(&remote, true, true), None);
        let local = HerdrTransport::local("/nonexistent/herdr.sock");
        assert!(herdr_is_local(&local));
        assert_eq!(
            offered(&local, true, true),
            Some(companion::TerminalInfo {
                input: true,
                mouse: false
            })
        );
        assert_eq!(
            offered(&HerdrTransport::mock(), true, false),
            Some(companion::TerminalInfo {
                input: false,
                mouse: false
            })
        );
        // Not over the link: no terminal.
        assert_eq!(offered(&local, false, true), None);
    }
}
