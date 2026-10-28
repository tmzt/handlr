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
//!
//! **Attached terminals** (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md,
//! steps 7 and 8, and its amendments of 23:24, 23:32 and 23:42 EDT). With
//! the daemon's herdr attachment ([`super::herdr_attach`]) a terminal may
//! read from it instead of the poll, unless the phone's `tab_follows_focus`
//! is off and the pane is not on herdr's active tab in the JSON snapshot
//! (decision 5): that terminal polls, and `pane.focus` is never sent for
//! it. While the attachment is up and its surface shows the pane, the
//! pane's cells, cursor and modes go through the same `Mirror` and the
//! same 8 frames a second (the 2 s re-read and the scroll wake-ups are not
//! used); otherwise the terminal is read by the poll as above, without
//! closing. Frames carry the cursor trailer only when the phone's
//! `terminal.open` asked (`cursor: true`); while the desktop has the tab
//! they carry `Cursor.stale`. On an attached terminal keys (after
//! [`parse_keys`]' refusals), `terminal.scroll` (a wheel event) and
//! `terminal.mouse` go over the attachment, gated as `terminal.input` is,
//! and each is a touch that takes the tab back from the desktop. On a
//! polled terminal `terminal.mouse` is `NOT_PERMITTED` and
//! `terminal.resize` is answered `{}` and ignored, since the poll cannot
//! size the pane.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libflankr_proto::common::{
    self, TERMINAL_INPUT_ARM_SECS, TERMINAL_INPUT_MAX, TerminalClosedReason, TerminalSource,
    methods,
};
use libflankr_proto::companion;
use libflankr_proto::envelope::codes;
use libflankr_proto::{ErrorObject, Id, Notification};
use libflankr_term::{Cursor, Frame, Grid, Mirror, parse_ansi};
use libherdr::protocol::{HerdrRequest, ReadFormat, ReadSource};
use libherdr::transport::HerdrTransport;
use libherdr_term::herdr::ClientPaneInputEvent;
use libherdr_term::{AttachError, SurfaceView, key_events};
use serde_json::{Value, json};
use tokio::sync::{Notify, broadcast, oneshot, watch};
use tokio::task::AbortHandle;
use tokio::time::Instant;

use super::conn::{Conn, ok, resolve_target, snapshot};
use super::herdr_attach::{HerdrAttach, Published, ViewerId};
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
/// How long `terminal.open` waits for the attachment's first surface of
/// the pane at the phone's size before it reads the pane by the poll.
const FIRST_VIEW_WAIT: Duration = Duration::from_millis(1500);
/// How long an attached `terminal.scroll` waits for the next surface's
/// offsets.
const SCROLL_ANSWER_WAIT: Duration = Duration::from_millis(500);
/// The phone's size when `terminal.open` gives no hints (decision 6).
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

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
    mouse: bool,
) -> Option<companion::TerminalInfo> {
    (over_link && herdr_is_local(transport)).then_some(companion::TerminalInfo { input, mouse })
}

/// [`offered`] for `daemon`: `mouse` while its herdr attachment is up.
pub fn info(daemon: &Daemon, over_link: bool) -> Option<companion::TerminalInfo> {
    let mouse = daemon.herdr_attach.as_ref().is_some_and(|a| a.usable());
    offered(
        daemon.client.transport(),
        over_link,
        daemon.terminal_input,
        mouse,
    )
}

/// Whether `pane_id` is on herdr's active tab in the JSON snapshot `snap`:
/// it is the focused pane, or shares the focused pane's tab. Unknown tabs
/// count as another tab.
pub fn on_active_tab(snap: &Value, pane_id: &str) -> bool {
    let Some(focused) = agents::focused_pane_id(snap) else {
        return false;
    };
    if pane_ids_match(&focused, pane_id) {
        return true;
    }
    let refs = agents::agents(snap);
    let tab_of = |id: &str| {
        refs.iter()
            .find(|r| pane_ids_match(&r.pane_id, id))
            .and_then(|r| r.tab_id.clone())
    };
    matches!((tab_of(pane_id), tab_of(&focused)), (Some(a), Some(b)) if a == b)
}

/// One open terminal.
struct Terminal {
    id: u32,
    /// Its viewer id on the daemon's herdr attachment.
    viewer: ViewerId,
    pane_id: String,
    /// The pane's agent when it was opened (lowercased, as the policy has
    /// it).
    agent: String,
    /// The phone reads the cursor trailer (`terminal.open`'s `cursor`).
    cursor: AtomicBool,
    /// The herdr attachment this terminal may read from, set at open.
    attach: Mutex<Option<Arc<HerdrAttach>>>,
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
    /// The last polled frame was marked stale (see [`stale_mark`]).
    last_marked: bool,
    /// A mouse event's notice was sent since the last arm (decision 12).
    mouse_noticed: bool,
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

    fn cursor(&self) -> bool {
        self.cursor.load(Ordering::Relaxed)
    }

    /// The attachment this terminal may use.
    fn attach(&self) -> Option<Arc<HerdrAttach>> {
        self.attach.lock().unwrap().clone()
    }

    /// The attachment, while it is up: input goes over it.
    fn attached(&self) -> Option<Arc<HerdrAttach>> {
        self.attach().filter(|a| a.usable())
    }

    /// Stop the poller and stop using the attachment; the terminal is out
    /// of the table already.
    fn release(&self, daemon: &Daemon) {
        self.abort_poller();
        daemon.terminal_panes.remove(&self.pane_id);
        if let Some(attach) = self.attach.lock().unwrap().take() {
            attach.leave(self.viewer);
        }
    }

    /// One frame's bytes, with the cursor trailer when the phone asked.
    fn encode(&self, frame: &Frame) -> Vec<u8> {
        frame.encode_with(self.cursor())
    }
}

/// The pane `pane_id` of `view` as a grid with its cursor, `stale` set: a
/// pane whose cursor is outside it gets a hidden one, so the modes and the
/// stale flag still reach the phone. `None` when the view does not show
/// the pane.
fn attached_grid(view: &SurfaceView, pane_id: &str, stale: bool) -> Option<Grid> {
    let pane = view.pane(pane_id)?;
    let hidden = Cursor {
        visible: false,
        mouse_reporting: pane.pane.mouse_reporting,
        alternate_screen: pane.pane.alternate_screen_active,
        ..Cursor::default()
    };
    let mut grid = Grid::from(pane);
    grid.cursor.get_or_insert(hidden).stale = stale;
    Some(grid)
}

/// `published`'s grid of `pane_id`, if it shows it.
fn published_grid(published: &Published, pane_id: &str) -> Option<Grid> {
    published
        .view
        .as_deref()
        .and_then(|view| attached_grid(view, pane_id, published.stale))
}

/// Whether a polled frame of `term` carries a hidden, stale cursor: its
/// attachment is up but the desktop has the tab (another tab is shown).
fn stale_mark(term: &Terminal) -> bool {
    term.attached().is_some_and(|a| a.stale())
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
    fn get_or_insert(&self, conn_id: u64, pane_id: &str, agent: &str) -> (Arc<Terminal>, bool) {
        let mut open = self.open.lock().unwrap();
        if let Some(t) = open.values().find(|t| t.pane_id == pane_id) {
            return (Arc::clone(t), false);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let t = Arc::new(Terminal {
            id,
            viewer: (conn_id, id),
            pane_id: pane_id.to_string(),
            agent: agent.to_string(),
            cursor: AtomicBool::new(false),
            attach: Mutex::new(None),
            state: Mutex::new(State {
                mirror: Mirror::new(id),
                last_text: None,
                armed_until: None,
                scroll_offset: 0,
                scroll_max: None,
                last_marked: false,
                mouse_noticed: false,
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
            t.release(daemon);
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

    let (term, new) = conn.terminals.get_or_insert(conn.id(), &pane_id, &agent);
    term.cursor.store(p.cursor, Ordering::Relaxed);

    // The attachment, unless the phone's tab_follows_focus is off and the
    // pane is on another tab than herdr's active one (decision 5).
    let attach = daemon.herdr_attach.clone().filter(|_| {
        let allowed = conn.tab_follows_focus() || on_active_tab(&snap, &pane_id);
        if !allowed {
            log::debug!(
                "Terminal {} on pane '{pane_id}' is polled: it is not on herdr's active tab \
                 and tab_follows_focus is off",
                term.id
            );
        }
        allowed
    });
    let previous = std::mem::replace(&mut *term.attach.lock().unwrap(), attach.clone());
    if let Some(previous) = previous
        && attach.is_none()
    {
        previous.leave(term.viewer);
    }
    let attached = match &attach {
        Some(a) => {
            a.open(
                term.viewer,
                &pane_id,
                p.cols_hint.unwrap_or(DEFAULT_COLS),
                p.rows_hint.unwrap_or(DEFAULT_ROWS),
            );
            a.usable()
        }
        None => false,
    };
    let view = match &attach {
        Some(a) if attached => a.first_view(&pane_id, FIRST_VIEW_WAIT).await,
        _ => None,
    };
    let stale = attach.as_ref().is_some_and(|a| a.stale());
    let (grid, text) = match view
        .as_deref()
        .and_then(|v| attached_grid(v, &pane_id, stale))
    {
        Some(grid) => (grid, None),
        None => match read_text(daemon, &pane_id).await {
            Ok(text) => (parse_ansi(&text), Some(text)),
            Err(e) => {
                if new {
                    conn.terminals.remove(term.id);
                    if let Some(a) = term.attach.lock().unwrap().take() {
                        a.leave(term.viewer);
                    }
                }
                return Err(herdr_unavailable(e));
            }
        },
    };
    let scroll = snapshot_scroll(&snap, &pane_id);
    let first = {
        let mut state = term.state.lock().unwrap();
        (state.scroll_offset, state.scroll_max) = scroll.unwrap_or((0, None));
        // A second open: the phone starts again, so a full frame.
        state.mirror.reset();
        state.last_text = text;
        state.last_marked = false;
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
    let source = if attached {
        TerminalSource::Attached
    } else {
        TerminalSource::Poll
    };
    log::info!(
        "Terminal {} open on pane '{pane_id}' ({agent}) for {}, {source:?}",
        term.id,
        conn.principal().id
    );
    ok(common::TerminalOpenResult {
        terminal_id: term.id,
        cols: first.cols,
        rows: first.rows,
        cursor: p.cursor && attached,
        source,
    })
}

/// `terminal.resize`: the phone's terminal area, sent to herdr for an
/// attached terminal (the 23:24 EDT amendment, item 1). A polled terminal
/// cannot be sized: answered `{}` and ignored.
pub(super) fn resize(conn: &Conn, raw: Value) -> Result<Value, ErrorObject> {
    served(conn)?;
    let p: common::TerminalResizeParams = super::conn::params(raw)?;
    let Some(term) = conn.terminals.get(p.terminal_id) else {
        return Err(not_permitted(format!(
            "no terminal {} is open on this connection",
            p.terminal_id
        )));
    };
    if let Some(attach) = term.attach() {
        attach.resize(term.viewer, p.cols, p.rows);
    }
    ok(common::Empty {})
}

/// `terminal.close`: `{}`, also for a terminal already gone.
pub(super) fn close(conn: &Conn, raw: Value) -> Result<Value, ErrorObject> {
    served(conn)?;
    let p: common::TerminalCloseParams = super::conn::params(raw)?;
    if let Some(term) = conn.terminals.remove(p.terminal_id) {
        term.release(&conn.daemon);
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
    {
        let mut state = term.state.lock().unwrap();
        state.armed_until = Some(Instant::now() + arm_window());
        // A new arm window: its first mouse event is noticed.
        state.mouse_noticed = false;
    }
    // The tap that armed is a touch: it takes herdr's tab back from the
    // desktop (the 23:32 EDT amendment, item 3).
    if let Some(attach) = term.attached()
        && let Err(e) = attach.claim(term.viewer)
    {
        log::debug!("Terminal {} armed without the attachment: {e}", term.id);
    }
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
    if let Some(attach) = term.attached() {
        let pane_scroll = |published: &Published| {
            published
                .view
                .as_ref()
                .and_then(|v| v.pane(&term.pane_id))
                .and_then(|v| v.pane.scroll)
                .map(|s| (s.offset_from_bottom, Some(s.max_offset_from_bottom)))
        };
        if p.lines == 0 {
            let (offset, max) = pane_scroll(&attach.current()).unwrap_or((current, max));
            return ok(common::TerminalScrollResult {
                offset_from_bottom: offset,
                max_offset_from_bottom: max,
            });
        }
        // A wheel event (decision 10): herdr routes it by the pane's modes.
        let mut surfaces = attach.subscribe();
        let before = pane_scroll(&surfaces.borrow_and_update()).map(|(o, _)| o);
        let sent = match attach.touch(term.viewer).await {
            Ok(()) => attach.input(term.viewer, vec![ClientPaneInputEvent::from(p)]),
            Err(e) => Err(e),
        };
        match sent {
            Ok(()) => {
                // The offsets from the next surface that shows the pane
                // scrolled; surfaces drawn before herdr took the wheel
                // still show the old offset, so without a change within
                // the wait, the latest surface's.
                let mut latest = None;
                let until = Instant::now() + SCROLL_ANSWER_WAIT;
                while let Ok(Ok(())) = tokio::time::timeout_at(until, surfaces.changed()).await {
                    if let Some(s) = pane_scroll(&surfaces.borrow_and_update()) {
                        latest = Some(s);
                        if Some(s.0) != before {
                            break;
                        }
                    }
                }
                let (offset, max) = latest.unwrap_or((current, max));
                {
                    let mut state = term.state.lock().unwrap();
                    state.scroll_offset = offset;
                    state.scroll_max = max;
                    state.armed_until = Some(Instant::now() + arm_window());
                }
                log::debug!(
                    "Terminal {} on pane '{}' got a wheel of {} row(s)",
                    term.id,
                    term.pane_id,
                    p.lines
                );
                return ok(common::TerminalScrollResult {
                    offset_from_bottom: offset,
                    max_offset_from_bottom: max,
                });
            }
            // The attachment went away: herdr's pane.scroll, as polled.
            Err(AttachError::Closed) => {}
            Err(e) => return Err(herdr_unavailable(e)),
        }
    }
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
    let mut sent = false;
    if let Some(attach) = term.attached() {
        // herdr's own key events (decision 8), after parse_keys' refusals.
        let events = key_events(&keys).map_err(|key| {
            ErrorObject::new(
                codes::INVALID_PARAMS,
                format!("the key {key:?} is not sent to a terminal"),
            )
        })?;
        match send_attached(&attach, &term, events).await {
            Ok(()) => sent = true,
            Err(AttachError::Closed) => {}
            Err(e) => return Err(herdr_unavailable(e)),
        }
    }
    if !sent {
        daemon
            .client
            .send_keys(&term.pane_id, &keys.join(" "))
            .await
            .map_err(herdr_unavailable)?;
    }
    term.state.lock().unwrap().armed_until = Some(Instant::now() + arm_window());
    let count = keys.len();
    log::info!(
        "{} typed {count} key(s) into terminal {} on pane '{}' ({})",
        conn.principal().id,
        term.id,
        term.pane_id,
        term.agent
    );
    let plural = if count == 1 { "key" } else { "keys" };
    notice(conn, &term, |name| {
        format!("Typed {count} {plural} into {name}'s terminal.")
    })
    .await;
    ok(common::Empty {})
}

/// A touch, then `events`, over the attachment.
async fn send_attached(
    attach: &Arc<HerdrAttach>,
    term: &Terminal,
    events: Vec<ClientPaneInputEvent>,
) -> Result<(), AttachError> {
    attach.touch(term.viewer).await?;
    attach.input(term.viewer, events)
}

/// A `terminal_input` notice on `term`, its text from the agent's spoken
/// name, capitalised.
async fn notice(conn: &Arc<Conn>, term: &Terminal, text: impl FnOnce(&str) -> String) {
    let names = conn.daemon.names_for(&term.pane_id).await;
    let spoken = names
        .as_ref()
        .map(|n| n.spoken.clone())
        .unwrap_or_else(|| crate::core::naming::capitalise(&term.agent));
    let notice = companion::Notice {
        id: conn.next_notice.fetch_add(1, Ordering::Relaxed) + 1,
        kind: NOTICE_TERMINAL_INPUT.to_string(),
        pane_id: term.pane_id.clone(),
        name: names.map(|n| n.name),
        text: text(&crate::core::naming::capitalise(&spoken)),
        spoken: Some(spoken),
    };
    conn.notices.lock().unwrap().queue(notice);
    super::companion::flush_notices(conn);
}

fn mouse_needs_attachment() -> ErrorObject {
    not_permitted("mouse needs the attached terminal")
}

/// `terminal.mouse` (decisions 10 and 12): gated and armed as
/// `terminal.input` is, over the attachment only. A kind herdr has no event
/// for is answered `{}` and dropped. One notice per arm window.
pub(super) async fn mouse(conn: &Arc<Conn>, raw: Value) -> Result<Value, ErrorObject> {
    let p: common::TerminalMouseParams = super::conn::params(raw)?;
    let (term, _) = input_allowed(conn, p.terminal_id)?;
    require_armed(&term)?;
    let Some(attach) = term.attached() else {
        return Err(mouse_needs_attachment());
    };
    let Ok(event) = ClientPaneInputEvent::try_from(&p) else {
        log::debug!("Ignoring a terminal.mouse of a kind herdr has no event for");
        return ok(common::Empty {});
    };
    send_attached(&attach, &term, vec![event])
        .await
        .map_err(|e| match e {
            AttachError::Closed => mouse_needs_attachment(),
            e => herdr_unavailable(e),
        })?;
    let first = {
        let mut state = term.state.lock().unwrap();
        state.armed_until = Some(Instant::now() + arm_window());
        !std::mem::replace(&mut state.mouse_noticed, true)
    };
    if first {
        log::info!(
            "{} used the mouse in terminal {} on pane '{}' ({})",
            conn.principal().id,
            term.id,
            term.pane_id,
            term.agent
        );
        notice(conn, &term, |name| format!("Clicked in {name}'s terminal.")).await;
    }
    ok(common::Empty {})
}

/// End `term` for `reason`: the poller's own exit.
fn closed(conn: &Conn, term: &Terminal, reason: TerminalClosedReason) {
    if conn.terminals.remove(term.id).is_none() {
        return;
    }
    // The poller's own task: not aborted, it returns after this.
    conn.daemon.terminal_panes.remove(&term.pane_id);
    if let Some(attach) = term.attach.lock().unwrap().take() {
        attach.leave(term.viewer);
    }
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

/// What a poller does next.
enum Step {
    /// Read the pane by `pane.read`; `idle` on the tick, which also resends
    /// an unacknowledged frame.
    Read { idle: bool },
    /// The attachment published a surface.
    Surface,
    /// The tick while attached: resend an unacknowledged frame.
    Retransmit,
}

/// The next surface, or `false` when the attachment's channel is gone;
/// never without one.
async fn next_surface(surfaces: &mut Option<watch::Receiver<Published>>) -> bool {
    match surfaces {
        Some(rx) => rx.changed().await.is_ok(),
        None => std::future::pending().await,
    }
}

/// One terminal's poller: the first frame once the open is answered, then
/// a read on each `pane.scroll_changed` (coalesced, at most one per
/// [`MIN_GAP`]) and each [`IDLE_TICK`], which also resends the newest
/// frame while the phone has not acknowledged it. While the attachment's
/// surface shows the pane, its surfaces are the frames instead (at most
/// one per [`MIN_GAP`]), and the tick only resends.
async fn poll(conn: Arc<Conn>, term: Arc<Terminal>, ready: oneshot::Receiver<()>, first: Frame) {
    let mut events = conn.daemon.events.subscribe();
    let mut surfaces = term.attach().map(|a| a.subscribe());
    let mut shown = surfaces
        .as_mut()
        .is_some_and(|rx| rx.borrow_and_update().shows(&term.pane_id));
    let _ = ready.await;
    conn.send_terminal(term.encode(&first));
    let mut tick = tokio::time::interval_at(Instant::now() + IDLE_TICK, IDLE_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_read = Instant::now();
    loop {
        let step = tokio::select! {
            _ = tick.tick() => if shown { Step::Retransmit } else { Step::Read { idle: true } },
            _ = term.poke.notified() => {
                // A scroll: the visible frame follows the offset.
                tokio::time::sleep_until(last_read + MIN_GAP).await;
                if shown {
                    continue;
                }
                Step::Read { idle: false }
            }
            changed = next_surface(&mut surfaces) => {
                if !changed {
                    surfaces = None;
                    shown = false;
                    continue;
                }
                tokio::time::sleep_until(last_read + MIN_GAP).await;
                Step::Surface
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
                    Seen::Changed => {
                        if shown {
                            continue;
                        }
                        Step::Read { idle: false }
                    }
                    Seen::Updated => {
                        if let Some(reason) = ended(&conn.daemon, &term).await {
                            return closed(&conn, &term, reason);
                        }
                        if shown {
                            continue;
                        }
                        Step::Read { idle: false }
                    }
                    Seen::Closed => return closed(&conn, &term, TerminalClosedReason::PaneClosed),
                    Seen::Gone => return,
                }
            }
        };
        let idle = match step {
            Step::Retransmit => {
                let frame = term.state.lock().unwrap().mirror.retransmit();
                if let Some(frame) = frame {
                    conn.send_terminal(term.encode(&frame));
                }
                continue;
            }
            Step::Surface => {
                let grid = surfaces
                    .as_mut()
                    .and_then(|rx| published_grid(&rx.borrow_and_update(), &term.pane_id));
                match grid {
                    Some(grid) => {
                        shown = true;
                        last_read = Instant::now();
                        let frame = term.state.lock().unwrap().mirror.update(grid);
                        if let Some(frame) = frame {
                            conn.send_terminal(term.encode(&frame));
                        }
                        continue;
                    }
                    // Still not shown: the poll goes on as it was.
                    None if !shown => continue,
                    // No longer shown (the attachment closed, or the
                    // desktop moved to another tab): read it now.
                    None => {
                        shown = false;
                        let mut state = term.state.lock().unwrap();
                        state.last_text = None;
                        false
                    }
                }
            }
            Step::Read { idle } => idle,
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
        let marked = stale_mark(&term);
        let frame = {
            let mut state = term.state.lock().unwrap();
            let frame = if state.last_text.as_deref() == Some(text.as_str())
                && state.last_marked == marked
            {
                None
            } else {
                let mut grid = parse_ansi(&text);
                if marked {
                    grid.cursor = Some(Cursor {
                        stale: true,
                        ..Cursor::default()
                    });
                }
                state.last_text = Some(text);
                state.last_marked = marked;
                state.mirror.update(grid)
            };
            match frame {
                Some(frame) => Some(frame),
                None if idle => state.mirror.retransmit(),
                None => None,
            }
        };
        if let Some(frame) = frame {
            conn.send_terminal(term.encode(&frame));
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
        assert_eq!(offered(&remote, true, true, false), None);
        let local = HerdrTransport::local("/nonexistent/herdr.sock");
        assert!(herdr_is_local(&local));
        assert_eq!(
            offered(&local, true, true, false),
            Some(companion::TerminalInfo {
                input: true,
                mouse: false
            })
        );
        assert_eq!(
            offered(&HerdrTransport::mock(), true, false, false),
            Some(companion::TerminalInfo {
                input: false,
                mouse: false
            })
        );
        // Not over the link: no terminal.
        assert_eq!(offered(&local, false, true, true), None);
    }
}
