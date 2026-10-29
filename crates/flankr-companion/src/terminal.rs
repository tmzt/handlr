//! The terminal mode's client half (docs/design/20261027_07_TERMINAL_MODE_PLAN.md,
//! step 4, and step 6's client side).
//!
//! [`Terminals`] is shared between the `net` thread and the app: the `net`
//! thread writes it as [`State`](crate::client::State) handles `terminal.*`
//! answers and the link's terminal frames, and the app reads it from its
//! own thread through [`Companion::terminals`](crate::Companion::terminals).
//! Every read takes a short lock; nothing is held across calls.
//!
//! One terminal per session (the pane id or name the app opened it with).
//! While the app wants it open, it stays wanted across connections: a
//! reconnect opens it again, and the new terminal's first frame starts a
//! fresh [`Screen`] (a new daemon `Mirror` numbers from 1). Until that
//! frame the old screen stays shown.
//!
//! Each frame shown is acknowledged at once, on the `net` thread
//! (`terminal.ack` with [`Screen::seq`]): the daemon diffs from what this
//! client holds, and the app draws the newest screen whenever
//! [`Terminals::changes`] moves, so acknowledging the held frame rather than
//! the drawn one costs nothing and needs no round trip through the app.
//!
//! Keys ([`Input::TerminalKeys`](crate::Input::TerminalKeys)) are cut into
//! batches of at most [`TERMINAL_INPUT_MAX`] bytes ([`key_batches`]) and
//! sent one at a time, each after the last was answered; a refusal drops
//! the rest.
//!
//! Scrolls ([`Input::TerminalScroll`](crate::Input::TerminalScroll), a drag
//! on the terminal) are coalesced: one `terminal.scroll` is in flight per
//! terminal at most, and the lines asked for meanwhile are summed and sent
//! together when it is answered. The answer's offset is kept for the app
//! ([`TerminalStatus::scroll_offset`]).
//!
//! The arm has no expiry (the L3 amendment of 2026-10-28 10:42 EDT of
//! docs/design/20261022_03_HARNESS_PROTOCOL_PLAN.md): once
//! `terminal.input_arm` is answered the terminal stays armed, whatever
//! `expires_in_s` says (the daemon answers 0, no expiry), until it is
//! closed, the daemon ends it (`terminal.closed`) or the connection drops.
//!
//! Plan 20261027_08, step 9:
//!
//! - `terminal.open` asks for the cursor trailer (`cursor: true`); the
//!   answer's `cursor` and `source` land in [`TerminalStatus`], and each
//!   frame's cursor in the [`Screen`] ([`Screen::cursor`]).
//! - The phone's fit ([`Input::TerminalResize`](crate::Input::TerminalResize))
//!   is kept as the terminal's hints, so a reopen after a reconnect asks
//!   for the current fit, and is sent as `terminal.resize` to an open
//!   terminal whose source is not the poll (a polled pane cannot be
//!   sized): never the size last sent (the open's hints included), one in
//!   flight at most, and the newest fit asked for meanwhile goes when it is
//!   answered. A refusal is logged, not shown; `METHOD_NOT_FOUND` stops
//!   resizes on that terminal.
//! - Mouse events ([`Input::TerminalMouse`](crate::Input::TerminalMouse))
//!   need an open, armed terminal and a daemon that serves
//!   `terminal.mouse` ([`Terminals::mouse_allowed`]). They go in order, one
//!   in flight at a time; a drag queued behind a drag replaces it (only
//!   the newest position matters), and a refusal drops the rest.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use libflankr_proto::common::{
    TERMINAL_INPUT_MAX, TerminalClosedParams, TerminalClosedReason, TerminalMouseKind,
    TerminalOpenResult, TerminalScrollResult, TerminalSource,
};
use libflankr_proto::companion::TerminalInfo;
use libflankr_term::{Frame, HISTORY, Screen};

/// The terminals, shared by the `net` thread and the app.
#[derive(Debug, Default)]
pub struct Terminals {
    book: Mutex<Book>,
    changes: AtomicU64,
}

/// What the app can show of one session's terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalStatus {
    /// The daemon has given this connection a terminal for it: frames come.
    pub open: bool,
    /// Wanted but not open: `terminal.open` is on its way, or waits for a
    /// connection or for a daemon that offers the terminal mode.
    pub opening: bool,
    /// The pane's size as the daemon gave it in `terminal.open`'s answer;
    /// 0 before. The screen's own [`Screen::cols`]/[`Screen::rows`] follow
    /// the frames.
    pub cols: u16,
    pub rows: u16,
    /// The daemon ended the terminal (`terminal.closed`): it is no longer
    /// wanted and its screen is gone, until it is opened again.
    pub closed: Option<TerminalClosedReason>,
    /// The last refusal or failure for this terminal (an error code's
    /// message), until the next open.
    pub error: Option<String>,
    /// Keyboard input is armed (`terminal.input_arm`): `Some(Duration::MAX)`,
    /// since the arm has no expiry (the L3 amendment of 2026-10-28 10:42
    /// EDT); `None` when not armed.
    pub armed_for: Option<Duration>,
    /// Key batches not yet answered, the one in flight included.
    pub keys_pending: usize,
    /// The pane's scroll offset in rows above the bottom, as the last
    /// `terminal.scroll` answer gave it; `None` before one (this
    /// connection's terminal).
    pub scroll_offset: Option<u64>,
    /// The furthest the pane can scroll back, when the daemon said.
    pub scroll_max: Option<u64>,
    /// The daemon writes the cursor trailer on this terminal's frames
    /// (`terminal.open`'s answer; plan 20261027_08, decision 10). False
    /// before the answer.
    pub cursor: bool,
    /// Where the daemon reads the terminal from, as its `terminal.open`
    /// answer said; [`TerminalSource::Poll`] before it.
    pub source: TerminalSource,
}

impl Terminals {
    /// Whether the daemon offers the terminal mode on this connection: its
    /// `hello` or last `server.capabilities` had `terminal`, and no
    /// `terminal.*` call was answered `METHOD_NOT_FOUND`. False offline.
    pub fn supported(&self) -> bool {
        let book = self.lock();
        book.online && book.info.is_some()
    }

    /// Whether the daemon lets keys be sent on this connection (its
    /// `[terminal] input`). Each agent's own setting and the arm still
    /// apply, at the daemon.
    pub fn input_allowed(&self) -> bool {
        let book = self.lock();
        book.online && book.info.is_some_and(|i| i.input)
    }

    /// Whether the daemon serves `terminal.mouse` on this connection
    /// (`TerminalInfo::mouse`, plan 20261027_08 decision 10). Its own gates
    /// and the arm still apply.
    pub fn mouse_allowed(&self) -> bool {
        let book = self.lock();
        book.online && book.info.is_some_and(|i| i.mouse)
    }

    /// Counts every change the app may draw: a frame shown, a terminal
    /// opened, closed or refused, an arm given or run out, the mode
    /// offered or withdrawn. Starts at 0; compare with the last value read.
    pub fn changes(&self) -> u64 {
        self.changes.load(Ordering::Acquire)
    }

    /// The sessions with a terminal entry (wanted, or closed by the daemon
    /// with a reason to show).
    pub fn sessions(&self) -> Vec<String> {
        let mut sessions: Vec<String> = self.lock().sessions.keys().cloned().collect();
        sessions.sort();
        sessions
    }

    /// A copy of the session's screen while its terminal is wanted (empty
    /// before the first frame). Cheap: a screen's grids are shared.
    pub fn screen(&self, session: &str) -> Option<Screen> {
        self.with_screen(session, Screen::clone)
    }

    /// `f` on the session's screen, under the lock, while its terminal is
    /// wanted. Keep `f` short: the `net` thread waits on it.
    pub fn with_screen<R>(&self, session: &str, f: impl FnOnce(&Screen) -> R) -> Option<R> {
        let book = self.lock();
        book.sessions
            .get(session)
            .filter(|t| t.wanted)
            .map(|t| f(&t.screen))
    }

    /// The session's terminal as the app can show it; `None` when it was
    /// never opened or was closed by the app.
    pub fn status(&self, session: &str) -> Option<TerminalStatus> {
        let book = self.lock();
        let t = book.sessions.get(session)?;
        Some(TerminalStatus {
            open: t.terminal_id.is_some(),
            opening: t.wanted && t.terminal_id.is_none(),
            cols: t.size.0,
            rows: t.size.1,
            closed: t.closed,
            error: t.error.clone(),
            armed_for: t.is_armed.then_some(Duration::MAX),
            keys_pending: t.keys.len() + usize::from(t.keys_in_flight),
            scroll_offset: t.scroll_offset,
            scroll_max: t.scroll_max,
            cursor: t.cursor,
            source: t.source,
        })
    }

    /// Why the daemon closed the session's terminal, if it did.
    pub fn closed_reason(&self, session: &str) -> Option<TerminalClosedReason> {
        self.lock().sessions.get(session).and_then(|t| t.closed)
    }

    /// Whether keys may be sent into the session's terminal now: it is open
    /// and armed.
    pub fn armed(&self, session: &str) -> bool {
        self.status(session)
            .is_some_and(|s| s.open && s.armed_for.is_some())
    }

    /// The session's terminal id while it is open and armed, for
    /// `terminal.paste` (plan 20261028_05, the 23:43 EDT amendment).
    pub(crate) fn armed_id(&self, session: &str) -> Option<u32> {
        let book = self.lock();
        let t = book.sessions.get(session)?;
        t.terminal_id.filter(|_| t.is_armed && t.wanted)
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Swaps this book's contents with `other`'s, both counted as changed
    /// (a remote coming into use, plan 20261028_03).
    pub(crate) fn swap_with(&self, other: &Terminals) {
        if std::ptr::eq(self, other) {
            return;
        }
        std::mem::swap(&mut *self.lock(), &mut *other.lock());
        self.changes.fetch_add(1, Ordering::AcqRel);
        other.changes.fetch_add(1, Ordering::AcqRel);
    }

    /// Runs `f` on the book; counts a change when it says so.
    pub(crate) fn update<R>(&self, f: impl FnOnce(&mut Book) -> (R, bool)) -> R {
        let (r, changed) = f(&mut self.lock());
        if changed {
            self.changes.fetch_add(1, Ordering::AcqRel);
        }
        r
    }
}

/// What the state must do for the terminals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Act {
    Open {
        session: String,
        cols_hint: Option<u16>,
        rows_hint: Option<u16>,
    },
    Close {
        terminal_id: u32,
    },
    Ack {
        terminal_id: u32,
        seq: u32,
    },
    Arm {
        session: String,
        terminal_id: u32,
    },
    Keys {
        session: String,
        terminal_id: u32,
        keys: String,
    },
    Scroll {
        session: String,
        terminal_id: u32,
        lines: i32,
    },
    Resize {
        session: String,
        terminal_id: u32,
        cols: u16,
        rows: u16,
    },
    Mouse {
        session: String,
        terminal_id: u32,
        event: MouseEvent,
    },
    Error(String),
}

/// One mouse event, pane-local, as `terminal.mouse` carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MouseEvent {
    pub kind: TerminalMouseKind,
    pub col: u16,
    pub row: u16,
    pub modifiers: u8,
}

/// The most mouse events held behind the one in flight; older ones are
/// dropped first.
const MOUSE_QUEUE: usize = 64;

#[derive(Debug, Default)]
pub(crate) struct Book {
    /// `hello` was answered on this connection.
    online: bool,
    /// The daemon's terminal mode, as last told; `None` means none.
    info: Option<TerminalInfo>,
    sessions: HashMap<String, Term>,
    /// Frames for a terminal id not known yet, while an open is on its
    /// way: its answer may come after its first frame.
    early: VecDeque<Frame>,
}

#[derive(Debug, Default)]
struct Term {
    /// The app wants it open: opened on each connection.
    wanted: bool,
    cols_hint: Option<u16>,
    rows_hint: Option<u16>,
    /// `terminal.open` is on its way on this connection.
    opening: bool,
    /// The terminal on this connection.
    terminal_id: Option<u32>,
    size: (u16, u16),
    screen: Screen,
    /// The next frame starts a fresh screen: the terminal is a new one.
    fresh: bool,
    closed: Option<TerminalClosedReason>,
    error: Option<String>,
    /// Armed by `terminal.input_arm`, until disarmed: no expiry.
    is_armed: bool,
    arming: bool,
    keys: VecDeque<String>,
    keys_in_flight: bool,
    /// Lines asked for while a `terminal.scroll` was in flight, sent as
    /// one when it is answered.
    scroll_pending: i32,
    scroll_in_flight: bool,
    scroll_offset: Option<u64>,
    scroll_max: Option<u64>,
    /// From `terminal.open`'s answer.
    cursor: bool,
    source: TerminalSource,
    /// The size last sent for this terminal (the open's hints, then each
    /// `terminal.resize`): not sent again.
    fit_sent: Option<(u16, u16)>,
    resize_in_flight: bool,
    /// The daemon has no `terminal.resize`: none is sent on this terminal.
    resize_unsupported: bool,
    /// Mouse events not yet sent, and whether one is in flight.
    mouse: VecDeque<MouseEvent>,
    mouse_in_flight: bool,
}

impl Term {
    fn disarm(&mut self) {
        self.is_armed = false;
        self.arming = false;
        self.keys.clear();
        self.keys_in_flight = false;
        self.scroll_pending = 0;
        self.scroll_in_flight = false;
        self.mouse.clear();
        self.mouse_in_flight = false;
    }

    /// The fit, when the app gave both of its parts.
    fn fit(&self) -> Option<(u16, u16)> {
        self.cols_hint.zip(self.rows_hint)
    }
}

const NO_MODE: &str = "the daemon offers no terminal mode";

impl Book {
    /// `hello` was answered (`online`) with this terminal mode, or a
    /// `server.capabilities` changed it. Opens what is wanted.
    pub(crate) fn set_info(&mut self, info: Option<TerminalInfo>) -> (Vec<Act>, bool) {
        let changed = !self.online || self.info != info;
        self.online = true;
        self.info = info;
        for t in self.sessions.values_mut() {
            if !info.is_some_and(|i| i.input) {
                t.disarm();
            }
            // Still wanted, and opened once a daemon offers the mode.
            if info.is_none() && t.wanted {
                t.error = Some(NO_MODE.to_string());
            }
        }
        (self.open_wanted(), changed)
    }

    /// An `open` for every wanted terminal not open or opening.
    fn open_wanted(&mut self) -> Vec<Act> {
        if !self.online || self.info.is_none() {
            return Vec::new();
        }
        let mut acts = Vec::new();
        let mut sessions: Vec<&String> = self.sessions.keys().collect();
        sessions.sort();
        for session in sessions {
            let t = &self.sessions[session];
            if t.wanted && !t.opening && t.terminal_id.is_none() {
                acts.push(Act::Open {
                    session: session.clone(),
                    cols_hint: t.cols_hint,
                    rows_hint: t.rows_hint,
                });
            }
        }
        for act in &acts {
            if let Act::Open { session, .. } = act
                && let Some(t) = self.sessions.get_mut(session)
            {
                t.opening = true;
                t.error = None;
                t.fit_sent = t.fit();
                t.resize_in_flight = false;
                t.resize_unsupported = false;
            }
        }
        acts
    }

    /// The connection closed: nothing is open on the next one until it is
    /// opened again, and no arm carries over.
    pub(crate) fn disconnected(&mut self) -> bool {
        let changed = self.online;
        self.online = false;
        self.early.clear();
        for t in self.sessions.values_mut() {
            t.opening = false;
            t.terminal_id = None;
            t.resize_in_flight = false;
            t.disarm();
        }
        changed
    }

    /// The app opens the session's terminal.
    pub(crate) fn open(
        &mut self,
        session: String,
        cols_hint: Option<u16>,
        rows_hint: Option<u16>,
    ) -> (Vec<Act>, bool) {
        let refuse = self.online && self.info.is_none();
        let t = self.sessions.entry(session.clone()).or_default();
        if !t.wanted {
            t.screen = Screen::default();
            t.fresh = false;
        }
        t.wanted = true;
        t.cols_hint = cols_hint;
        t.rows_hint = rows_hint;
        t.closed = None;
        t.error = None;
        if refuse {
            t.wanted = false;
            t.error = Some(NO_MODE.to_string());
            return (vec![Act::Error(format!("terminal.open: {NO_MODE}"))], true);
        }
        (self.open_wanted(), true)
    }

    /// The app closes the session's terminal: it is forgotten.
    pub(crate) fn close(&mut self, session: &str) -> (Vec<Act>, bool) {
        let Some(t) = self.sessions.remove(session) else {
            return (Vec::new(), false);
        };
        // One still opening is closed when its answer comes.
        let acts = t
            .terminal_id
            .map(|terminal_id| Act::Close { terminal_id })
            .into_iter()
            .collect();
        (acts, true)
    }

    /// Every terminal is closed and forgotten (its remote goes out of
    /// use).
    pub(crate) fn close_all(&mut self) -> (Vec<Act>, bool) {
        let sessions: Vec<String> = self.sessions.keys().cloned().collect();
        let mut acts = Vec::new();
        let mut changed = false;
        for session in sessions {
            let (more, c) = self.close(&session);
            acts.extend(more);
            changed |= c;
        }
        (acts, changed)
    }

    /// `terminal.open` was answered.
    pub(crate) fn opened(&mut self, session: &str, result: TerminalOpenResult) -> (Vec<Act>, bool) {
        let terminal_id = result.terminal_id;
        let Some(t) = self.sessions.get_mut(session).filter(|t| t.opening) else {
            // Closed by the app meanwhile.
            return (vec![Act::Close { terminal_id }], false);
        };
        t.opening = false;
        t.terminal_id = Some(terminal_id);
        t.size = (result.cols, result.rows);
        t.fresh = true;
        t.scroll_offset = None;
        t.scroll_max = None;
        t.cursor = result.cursor;
        t.source = result.source;
        // A fit that changed while the open was on its way.
        let mut acts = self.next_resize(session);
        let early: Vec<Frame> = self.early.iter().cloned().collect();
        self.early.retain(|f| f.terminal_id != terminal_id);
        let mut early: Vec<Frame> = early
            .into_iter()
            .filter(|f| f.terminal_id == terminal_id)
            .collect();
        early.sort_by_key(|f| f.seq);
        for frame in early {
            acts.extend(self.frame(frame).0);
        }
        (acts, true)
    }

    /// `terminal.open` was refused. `no_method`: by an older daemon,
    /// which has no terminal mode.
    pub(crate) fn open_failed(
        &mut self,
        session: &str,
        message: String,
        no_method: bool,
    ) -> (Vec<Act>, bool) {
        if no_method {
            self.info = None;
        }
        let Some(t) = self.sessions.get_mut(session).filter(|t| t.opening) else {
            return (Vec::new(), no_method);
        };
        t.opening = false;
        t.wanted = false;
        let message = if no_method {
            NO_MODE.to_string()
        } else {
            message
        };
        t.error = Some(message.clone());
        (vec![Act::Error(format!("terminal.open: {message}"))], true)
    }

    /// `terminal.closed` came.
    pub(crate) fn closed(&mut self, note: TerminalClosedParams) -> bool {
        let Some(t) = self
            .sessions
            .values_mut()
            .find(|t| t.terminal_id == Some(note.terminal_id))
        else {
            log::debug!("terminal.closed for {}, not ours", note.terminal_id);
            return false;
        };
        t.terminal_id = None;
        t.wanted = false;
        t.opening = false;
        t.closed = Some(note.reason);
        t.screen = Screen::default();
        t.disarm();
        true
    }

    /// A frame came: shown, if it can be, and then acknowledged.
    pub(crate) fn frame(&mut self, frame: Frame) -> (Vec<Act>, bool) {
        let terminal_id = frame.terminal_id;
        let Some(t) = self
            .sessions
            .values_mut()
            .find(|t| t.terminal_id == Some(terminal_id))
        else {
            if self.sessions.values().any(|t| t.opening) {
                if self.early.len() == HISTORY {
                    self.early.pop_front();
                }
                self.early.push_back(frame);
            } else {
                log::debug!("a frame for terminal {terminal_id}, not ours");
            }
            return (Vec::new(), false);
        };
        if t.fresh {
            let mut screen = Screen::default();
            if !screen.apply(&frame) {
                // Not a frame a new screen can show (a diff): keep the old
                // screen until one comes.
                return (Vec::new(), false);
            }
            t.screen = screen;
            t.fresh = false;
        } else if !t.screen.apply(&frame) {
            return (Vec::new(), false);
        }
        let seq = t.screen.seq();
        (vec![Act::Ack { terminal_id, seq }], true)
    }

    /// The app arms keyboard input.
    pub(crate) fn arm(&mut self, session: &str) -> (Vec<Act>, bool) {
        let allowed = self.info.is_some_and(|i| i.input);
        let Some(t) = self.sessions.get_mut(session) else {
            return (
                vec![Act::Error(format!(
                    "terminal.input_arm: no terminal open for {session}"
                ))],
                false,
            );
        };
        let Some(terminal_id) = t.terminal_id else {
            let message = "the terminal is not open".to_string();
            t.error = Some(message.clone());
            return (
                vec![Act::Error(format!("terminal.input_arm: {message}"))],
                true,
            );
        };
        if !allowed {
            let message = "the daemon does not allow terminal input".to_string();
            t.error = Some(message.clone());
            return (
                vec![Act::Error(format!("terminal.input_arm: {message}"))],
                true,
            );
        }
        if t.arming {
            return (Vec::new(), false);
        }
        t.arming = true;
        (
            vec![Act::Arm {
                session: session.to_string(),
                terminal_id,
            }],
            false,
        )
    }

    /// `terminal.input_arm` was answered: armed until disarmed (its
    /// `expires_in_s` is not used: the arm has no expiry), or refused.
    pub(crate) fn armed(&mut self, session: &str, result: Result<u32, String>) -> (Vec<Act>, bool) {
        let Some(t) = self.sessions.get_mut(session).filter(|t| t.arming) else {
            return (Vec::new(), false);
        };
        t.arming = false;
        match result {
            Ok(_) => {
                t.is_armed = true;
                t.error = None;
                (Vec::new(), true)
            }
            Err(message) => {
                t.is_armed = false;
                t.error = Some(message.clone());
                (
                    vec![Act::Error(format!("terminal.input_arm: {message}"))],
                    true,
                )
            }
        }
    }

    /// The app sends keys: queued in batches, the first sent if none is
    /// in flight. Refused here unless open and armed.
    pub(crate) fn send_keys(&mut self, session: &str, keys: &str) -> (Vec<Act>, bool) {
        let Some(t) = self.sessions.get_mut(session) else {
            return (
                vec![Act::Error(format!(
                    "terminal.input: no terminal open for {session}"
                ))],
                false,
            );
        };
        if t.terminal_id.is_none() || !t.is_armed {
            let message = "terminal input is not armed".to_string();
            t.error = Some(message.clone());
            return (vec![Act::Error(format!("terminal.input: {message}"))], true);
        }
        t.keys.extend(key_batches(keys));
        (self.next_keys(session), true)
    }

    fn next_keys(&mut self, session: &str) -> Vec<Act> {
        let Some(t) = self.sessions.get_mut(session) else {
            return Vec::new();
        };
        let Some(terminal_id) = t.terminal_id else {
            return Vec::new();
        };
        if t.keys_in_flight {
            return Vec::new();
        }
        let Some(keys) = t.keys.pop_front() else {
            return Vec::new();
        };
        t.keys_in_flight = true;
        vec![Act::Keys {
            session: session.to_string(),
            terminal_id,
            keys,
        }]
    }

    /// A `terminal.input` was answered: the next batch goes, or, refused,
    /// the rest are dropped.
    pub(crate) fn keys_sent(
        &mut self,
        session: &str,
        result: Result<(), String>,
    ) -> (Vec<Act>, bool) {
        let Some(t) = self.sessions.get_mut(session).filter(|t| t.keys_in_flight) else {
            return (Vec::new(), false);
        };
        t.keys_in_flight = false;
        match result {
            Ok(()) => (self.next_keys(session), true),
            Err(message) => {
                t.keys.clear();
                t.error = Some(message.clone());
                (vec![Act::Error(format!("terminal.input: {message}"))], true)
            }
        }
    }

    /// The app scrolls the pane by `lines` (positive back into the
    /// scrollback): sent at once if no scroll is in flight, else added to
    /// what goes when it is answered. Refused here unless open and armed.
    pub(crate) fn scroll(&mut self, session: &str, lines: i32) -> (Vec<Act>, bool) {
        let Some(t) = self.sessions.get_mut(session) else {
            return (
                vec![Act::Error(format!(
                    "terminal.scroll: no terminal open for {session}"
                ))],
                false,
            );
        };
        if t.terminal_id.is_none() || !t.is_armed {
            let message = "terminal input is not armed".to_string();
            t.error = Some(message.clone());
            return (
                vec![Act::Error(format!("terminal.scroll: {message}"))],
                true,
            );
        }
        t.scroll_pending = t.scroll_pending.saturating_add(lines);
        (self.next_scroll(session), false)
    }

    fn next_scroll(&mut self, session: &str) -> Vec<Act> {
        let Some(t) = self.sessions.get_mut(session) else {
            return Vec::new();
        };
        let Some(terminal_id) = t.terminal_id else {
            return Vec::new();
        };
        if t.scroll_in_flight || t.scroll_pending == 0 {
            return Vec::new();
        }
        let lines = std::mem::take(&mut t.scroll_pending);
        t.scroll_in_flight = true;
        vec![Act::Scroll {
            session: session.to_string(),
            terminal_id,
            lines,
        }]
    }

    /// A `terminal.scroll` was answered: its offset is kept and what was
    /// asked meanwhile goes, or, refused, that is dropped.
    pub(crate) fn scrolled(
        &mut self,
        session: &str,
        result: Result<TerminalScrollResult, String>,
    ) -> (Vec<Act>, bool) {
        let Some(t) = self
            .sessions
            .get_mut(session)
            .filter(|t| t.scroll_in_flight)
        else {
            return (Vec::new(), false);
        };
        t.scroll_in_flight = false;
        match result {
            Ok(r) => {
                t.scroll_offset = Some(r.offset_from_bottom);
                t.scroll_max = r.max_offset_from_bottom;
                (self.next_scroll(session), true)
            }
            Err(message) => {
                t.scroll_pending = 0;
                t.error = Some(message.clone());
                (
                    vec![Act::Error(format!("terminal.scroll: {message}"))],
                    true,
                )
            }
        }
    }

    /// The app's fit for the session's terminal changed: kept as its hints
    /// (a reopen asks for it), and sent to an open terminal unless it is
    /// the size last sent or one is in flight (then it goes when that is
    /// answered, the newest fit winning).
    pub(crate) fn resize(&mut self, session: &str, cols: u16, rows: u16) -> (Vec<Act>, bool) {
        let Some(t) = self.sessions.get_mut(session).filter(|t| t.wanted) else {
            return (Vec::new(), false);
        };
        t.cols_hint = Some(cols);
        t.rows_hint = Some(rows);
        (self.next_resize(session), false)
    }

    fn next_resize(&mut self, session: &str) -> Vec<Act> {
        let Some(t) = self.sessions.get_mut(session) else {
            return Vec::new();
        };
        let (Some(terminal_id), Some(fit)) = (t.terminal_id, t.fit()) else {
            return Vec::new();
        };
        // A polled pane cannot be sized (the 23:24 EDT amendment, item 1).
        if t.source == TerminalSource::Poll
            || t.resize_unsupported
            || t.resize_in_flight
            || t.fit_sent == Some(fit)
        {
            return Vec::new();
        }
        t.fit_sent = Some(fit);
        t.resize_in_flight = true;
        vec![Act::Resize {
            session: session.to_string(),
            terminal_id,
            cols: fit.0,
            rows: fit.1,
        }]
    }

    /// A `terminal.resize` was answered: the newest fit goes if it is not
    /// the one sent. A refusal is only logged; `no_method` (an older
    /// daemon) stops resizes on this terminal.
    pub(crate) fn resized(
        &mut self,
        session: &str,
        result: Result<(), String>,
        no_method: bool,
    ) -> (Vec<Act>, bool) {
        let Some(t) = self
            .sessions
            .get_mut(session)
            .filter(|t| t.resize_in_flight)
        else {
            return (Vec::new(), false);
        };
        t.resize_in_flight = false;
        if let Err(message) = result {
            log::info!("terminal.resize of {session}: {message}");
            if no_method {
                t.resize_unsupported = true;
                return (Vec::new(), false);
            }
        }
        (self.next_resize(session), false)
    }

    /// The app sends a mouse event: queued, and sent if none is in flight.
    /// Refused here unless the daemon serves `terminal.mouse` and the
    /// terminal is open and armed.
    pub(crate) fn mouse(&mut self, session: &str, event: MouseEvent) -> (Vec<Act>, bool) {
        let allowed = self.online && self.info.is_some_and(|i| i.mouse);
        let Some(t) = self.sessions.get_mut(session) else {
            return (
                vec![Act::Error(format!(
                    "terminal.mouse: no terminal open for {session}"
                ))],
                false,
            );
        };
        if !allowed {
            let message = "the daemon does not serve the mouse".to_string();
            t.error = Some(message.clone());
            return (vec![Act::Error(format!("terminal.mouse: {message}"))], true);
        }
        if t.terminal_id.is_none() || !t.is_armed {
            let message = "terminal input is not armed".to_string();
            t.error = Some(message.clone());
            return (vec![Act::Error(format!("terminal.mouse: {message}"))], true);
        }
        let drag = |e: &MouseEvent| matches!(e.kind, TerminalMouseKind::Drag { .. });
        match t.mouse.back_mut() {
            // Only the newest position of a drag matters.
            Some(last) if drag(last) && drag(&event) && last.kind == event.kind => *last = event,
            _ => {
                if t.mouse.len() == MOUSE_QUEUE {
                    t.mouse.pop_front();
                }
                t.mouse.push_back(event);
            }
        }
        (self.next_mouse(session), false)
    }

    fn next_mouse(&mut self, session: &str) -> Vec<Act> {
        let Some(t) = self.sessions.get_mut(session) else {
            return Vec::new();
        };
        let Some(terminal_id) = t.terminal_id else {
            return Vec::new();
        };
        if t.mouse_in_flight {
            return Vec::new();
        }
        let Some(event) = t.mouse.pop_front() else {
            return Vec::new();
        };
        t.mouse_in_flight = true;
        vec![Act::Mouse {
            session: session.to_string(),
            terminal_id,
            event,
        }]
    }

    /// A `terminal.mouse` was answered: the next event goes and the arm is
    /// extended, or, refused, the rest are dropped.
    pub(crate) fn mouse_sent(
        &mut self,
        session: &str,
        result: Result<(), String>,
    ) -> (Vec<Act>, bool) {
        let Some(t) = self.sessions.get_mut(session).filter(|t| t.mouse_in_flight) else {
            return (Vec::new(), false);
        };
        t.mouse_in_flight = false;
        match result {
            Ok(()) => (self.next_mouse(session), true),
            Err(message) => {
                t.mouse.clear();
                t.error = Some(message.clone());
                (vec![Act::Error(format!("terminal.mouse: {message}"))], true)
            }
        }
    }

    /// The session whose terminal is `terminal_id` on this connection.
    #[cfg(test)]
    fn session_of(&self, terminal_id: u32) -> Option<&str> {
        self.sessions
            .iter()
            .find(|(_, t)| t.terminal_id == Some(terminal_id))
            .map(|(s, _)| s.as_str())
    }
}

/// `keys` cut into batches of at most [`TERMINAL_INPUT_MAX`] bytes, which
/// joined give `keys` back. A batch ends after whitespace where it can, so
/// a token (`enter`, `ctrl+a`, a word) is not split; a token longer than a
/// batch is cut at a character boundary, never inside a UTF-8 sequence.
/// Empty keys give no batch.
pub fn key_batches(keys: &str) -> Vec<String> {
    let mut batches = Vec::new();
    let mut rest = keys;
    while !rest.is_empty() {
        if rest.len() <= TERMINAL_INPUT_MAX {
            batches.push(rest.to_string());
            break;
        }
        let mut limit = TERMINAL_INPUT_MAX;
        while !rest.is_char_boundary(limit) {
            limit -= 1;
        }
        let head = &rest[..limit];
        // After the last whitespace in the batch, if a token follows it;
        // else the whole batch.
        let cut = head
            .char_indices()
            .filter(|(_, c)| c.is_whitespace())
            .map(|(i, c)| i + c.len_utf8())
            .next_back()
            .unwrap_or(limit);
        batches.push(rest[..cut].to_string());
        rest = &rest[cut..];
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use libflankr_term::{Mirror, parse_ansi};

    fn online(input: bool) -> Book {
        let mut b = Book::default();
        b.set_info(Some(TerminalInfo {
            input,
            mouse: false,
        }));
        b
    }

    fn opened(b: &mut Book, session: &str, id: u32) {
        let (acts, _) = b.open(session.to_string(), Some(60), None);
        assert_eq!(
            acts,
            vec![Act::Open {
                session: session.to_string(),
                cols_hint: Some(60),
                rows_hint: None
            }]
        );
        b.opened(
            session,
            TerminalOpenResult {
                terminal_id: id,
                cols: 80,
                rows: 24,
                cursor: false,
                source: Default::default(),
            },
        );
    }

    #[test]
    fn batches_are_at_most_64_bytes_on_token_boundaries_and_join_back() {
        assert!(key_batches("").is_empty());
        assert_eq!(key_batches("ls enter"), vec!["ls enter"]);
        let words: String = (0..30).map(|i| format!("w{i} ")).collect();
        let batches = key_batches(&words);
        assert!(batches.len() > 1);
        assert_eq!(batches.concat(), words);
        for b in &batches {
            assert!(b.len() <= TERMINAL_INPUT_MAX, "{b:?}");
            assert!(b.ends_with(' '), "cut after whitespace: {b:?}");
        }
        // One long token of 3-byte characters: cut at a character boundary.
        let long = "\u{2500}".repeat(40);
        let batches = key_batches(&long);
        assert_eq!(batches.concat(), long);
        assert!(batches.iter().all(|b| b.len() <= TERMINAL_INPUT_MAX));
        assert_eq!(batches[0].len(), 63);
    }

    #[test]
    fn frames_show_and_are_acked_and_a_reopen_starts_a_fresh_screen() {
        let mut b = online(false);
        opened(&mut b, "claude", 7);
        let mut m = Mirror::new(7);
        let (acts, changed) = b.frame(m.update(parse_ansi("one\ntwo")).unwrap());
        assert!(changed);
        assert_eq!(
            acts,
            vec![Act::Ack {
                terminal_id: 7,
                seq: 1
            }]
        );
        m.ack(1);
        let diff = m.update(parse_ansi("one\nsix")).unwrap();
        assert!(!diff.is_full());
        b.frame(diff);
        assert_eq!(b.sessions["claude"].screen.grid(), &parse_ansi("one\nsix"));

        // Reconnect: reopened, and the new terminal's seq 1 shows.
        assert!(b.disconnected());
        assert_eq!(b.sessions["claude"].terminal_id, None);
        let (acts, _) = b.set_info(Some(TerminalInfo {
            input: false,
            mouse: false,
        }));
        assert_eq!(acts.len(), 1);
        b.opened(
            "claude",
            TerminalOpenResult {
                terminal_id: 9,
                cols: 80,
                rows: 24,
                cursor: false,
                source: Default::default(),
            },
        );
        // The old screen until the first frame.
        assert_eq!(b.sessions["claude"].screen.seq(), 2);
        let mut m = Mirror::new(9);
        b.frame(m.update(parse_ansi("fresh")).unwrap());
        assert_eq!(b.sessions["claude"].screen.seq(), 1);
        assert_eq!(b.sessions["claude"].screen.grid(), &parse_ansi("fresh"));
        assert_eq!(b.session_of(9), Some("claude"));
    }

    #[test]
    fn a_frame_before_its_open_answer_is_kept_for_it() {
        let mut b = online(false);
        b.open("claude".into(), None, None);
        let mut m = Mirror::new(3);
        assert_eq!(b.frame(m.update(parse_ansi("early")).unwrap()).0, vec![]);
        let (acts, _) = b.opened(
            "claude",
            TerminalOpenResult {
                terminal_id: 3,
                cols: 5,
                rows: 1,
                cursor: false,
                source: Default::default(),
            },
        );
        assert_eq!(
            acts,
            vec![Act::Ack {
                terminal_id: 3,
                seq: 1
            }]
        );
        assert_eq!(b.sessions["claude"].screen.grid(), &parse_ansi("early"));
    }

    #[test]
    fn method_not_found_is_no_terminal_mode() {
        let mut b = online(true);
        b.open("claude".into(), None, None);
        let (acts, _) = b.open_failed("claude", "x".into(), true);
        assert_eq!(acts, vec![Act::Error(format!("terminal.open: {NO_MODE}"))]);
        assert_eq!(b.info, None);
        assert!(!b.sessions["claude"].wanted);
        // And a later open is refused here.
        let (acts, _) = b.open("claude".into(), None, None);
        assert!(matches!(&acts[..], [Act::Error(_)]));
    }

    #[test]
    fn keys_need_an_arm_and_go_one_batch_at_a_time() {
        let mut b = online(true);
        opened(&mut b, "claude", 7);
        assert!(matches!(
            &b.send_keys("claude", "ls").0[..],
            [Act::Error(_)]
        ));
        assert_eq!(
            b.arm("claude").0,
            vec![Act::Arm {
                session: "claude".into(),
                terminal_id: 7
            }]
        );
        // 0: no expiry, as the daemon answers since the L3 amendment of
        // 2026-10-28 10:42 EDT.
        b.armed("claude", Ok(0));
        let keys = "a ".repeat(50);
        let (acts, _) = b.send_keys("claude", &keys);
        assert_eq!(acts.len(), 1);
        let (acts, _) = b.keys_sent("claude", Ok(()));
        assert!(matches!(&acts[..], [Act::Keys { keys, .. }] if keys.len() == 36));
        assert_eq!(b.keys_sent("claude", Ok(())).0, vec![]);
        // No expiry: still armed, for as long as the terminal is open.
        assert_eq!(b.send_keys("claude", "ls").0.len(), 1);
        b.keys_sent("claude", Ok(()));
        let st = Terminals::default();
        *st.lock() = std::mem::take(&mut b);
        assert_eq!(st.status("claude").unwrap().armed_for, Some(Duration::MAX));
        assert!(st.armed("claude"));
        // The connection drops: the arm ends with it.
        st.lock().disconnected();
        assert_eq!(st.status("claude").unwrap().armed_for, None);
    }

    #[test]
    fn scrolls_coalesce_and_keep_the_arm() {
        let mut b = online(true);
        opened(&mut b, "claude", 7);
        assert!(matches!(&b.scroll("claude", 3).0[..], [Act::Error(_)]));
        b.arm("claude");
        b.armed("claude", Ok(0));
        assert_eq!(
            b.scroll("claude", 3).0,
            vec![Act::Scroll {
                session: "claude".into(),
                terminal_id: 7,
                lines: 3
            }]
        );
        assert_eq!(b.scroll("claude", -1).0, vec![]);
        assert_eq!(b.scroll("claude", 5).0, vec![]);
        let (acts, changed) = b.scrolled(
            "claude",
            Ok(TerminalScrollResult {
                offset_from_bottom: 3,
                max_offset_from_bottom: None,
            }),
        );
        assert!(changed);
        assert!(matches!(&acts[..], [Act::Scroll { lines: 4, .. }]));
        assert_eq!(b.sessions["claude"].scroll_offset, Some(3));
        assert!(b.sessions["claude"].is_armed);
        // A refusal drops what was asked meanwhile.
        b.scroll("claude", 2);
        let (acts, _) = b.scrolled("claude", Err("no".into()));
        assert!(matches!(&acts[..], [Act::Error(_)]));
        assert_eq!(b.sessions["claude"].scroll_pending, 0);
        assert!(!b.sessions["claude"].scroll_in_flight);
    }

    #[test]
    fn arming_is_refused_here_without_the_daemons_input() {
        let mut b = online(false);
        opened(&mut b, "claude", 7);
        assert!(matches!(&b.arm("claude").0[..], [Act::Error(_)]));
    }

    /// Opened with hints 60x30 as an attached terminal `id`.
    fn attached(b: &mut Book, session: &str, id: u32) {
        let (acts, _) = b.open(session.to_string(), Some(60), Some(30));
        assert!(matches!(&acts[..], [Act::Open { .. }]));
        let (acts, _) = b.opened(
            session,
            TerminalOpenResult {
                terminal_id: id,
                cols: 60,
                rows: 30,
                cursor: true,
                source: TerminalSource::Attached,
            },
        );
        assert_eq!(acts, vec![], "the open's own hints are not resent");
    }

    fn resize_act(cols: u16, rows: u16) -> Act {
        Act::Resize {
            session: "claude".into(),
            terminal_id: 7,
            cols,
            rows,
        }
    }

    #[test]
    fn resizes_are_deduplicated_one_in_flight_and_the_newest_wins() {
        let mut b = online(true);
        attached(&mut b, "claude", 7);
        let st = b.sessions["claude"].source;
        assert_eq!(st, TerminalSource::Attached);
        assert_eq!(b.resize("claude", 60, 30).0, vec![], "the size sent");
        assert_eq!(b.resize("claude", 60, 20).0, vec![resize_act(60, 20)]);
        // While it is in flight: held, the newest kept.
        assert_eq!(b.resize("claude", 60, 18).0, vec![]);
        assert_eq!(b.resize("claude", 50, 18).0, vec![]);
        assert_eq!(
            b.resized("claude", Ok(()), false).0,
            vec![resize_act(50, 18)]
        );
        // Back to what was in flight, then answered: nothing new.
        assert_eq!(b.resize("claude", 50, 18).0, vec![]);
        assert_eq!(b.resized("claude", Ok(()), false).0, vec![]);
        // Held, then the fit returns to the size sent: nothing goes.
        assert_eq!(b.resize("claude", 40, 18).0, vec![resize_act(40, 18)]);
        b.resize("claude", 30, 10);
        b.resize("claude", 40, 18);
        assert_eq!(b.resized("claude", Ok(()), false).0, vec![]);
        // A refusal is only logged; the next change still goes.
        assert_eq!(b.resize("claude", 41, 18).0, vec![resize_act(41, 18)]);
        assert_eq!(
            b.resized("claude", Err("busy".into()), false).0,
            vec![],
            "no error shown"
        );
        assert_eq!(b.sessions["claude"].error, None);
        assert_eq!(b.resize("claude", 42, 18).0, vec![resize_act(42, 18)]);
        // An older daemon: no more resizes on this terminal.
        b.resized("claude", Err("no".into()), true);
        assert_eq!(b.resize("claude", 43, 18).0, vec![]);
    }

    #[test]
    fn the_fit_is_the_reopens_hints_and_a_polled_pane_is_not_resized() {
        let mut b = online(true);
        opened(&mut b, "claude", 7);
        // The poll: kept, not sent.
        assert_eq!(b.resize("claude", 70, 25).0, vec![]);
        b.disconnected();
        let (acts, _) = b.set_info(Some(TerminalInfo {
            input: true,
            mouse: true,
        }));
        assert_eq!(
            acts,
            vec![Act::Open {
                session: "claude".into(),
                cols_hint: Some(70),
                rows_hint: Some(25)
            }]
        );
        // A fit that changed while the open was on its way goes with its
        // answer.
        b.resize("claude", 72, 25);
        let (acts, _) = b.opened(
            "claude",
            TerminalOpenResult {
                terminal_id: 9,
                cols: 70,
                rows: 25,
                cursor: true,
                source: TerminalSource::Attached,
            },
        );
        assert_eq!(
            acts,
            vec![Act::Resize {
                session: "claude".into(),
                terminal_id: 9,
                cols: 72,
                rows: 25
            }]
        );
        let st = Terminals::default();
        *st.lock() = std::mem::take(&mut b);
        let status = st.status("claude").unwrap();
        assert!(status.cursor);
        assert_eq!(status.source, TerminalSource::Attached);
    }

    fn click(kind: TerminalMouseKind, col: u16) -> MouseEvent {
        MouseEvent {
            kind,
            col,
            row: 2,
            modifiers: 0,
        }
    }

    #[test]
    fn mouse_events_need_the_arm_and_go_in_order() {
        use libflankr_proto::common::MouseButton::Left;
        let mut b = Book::default();
        b.set_info(Some(TerminalInfo {
            input: true,
            mouse: true,
        }));
        attached(&mut b, "claude", 7);
        let down = click(TerminalMouseKind::Down { button: Left }, 1);
        assert!(matches!(&b.mouse("claude", down).0[..], [Act::Error(_)]));
        b.arm("claude");
        b.armed("claude", Ok(0));
        let sent = |e| Act::Mouse {
            session: "claude".into(),
            terminal_id: 7,
            event: e,
        };
        assert_eq!(b.mouse("claude", down).0, vec![sent(down)]);
        let drag = |c| click(TerminalMouseKind::Drag { button: Left }, c);
        let up = click(TerminalMouseKind::Up { button: Left }, 5);
        assert_eq!(b.mouse("claude", drag(2)).0, vec![]);
        assert_eq!(b.mouse("claude", drag(3)).0, vec![]);
        assert_eq!(b.mouse("claude", drag(4)).0, vec![]);
        assert_eq!(b.mouse("claude", up).0, vec![]);
        // The drags behind the down became the newest one.
        assert_eq!(b.mouse_sent("claude", Ok(())).0, vec![sent(drag(4))]);
        assert!(b.sessions["claude"].is_armed);
        assert_eq!(b.mouse_sent("claude", Ok(())).0, vec![sent(up)]);
        assert_eq!(b.mouse_sent("claude", Ok(())).0, vec![]);
        // A refusal drops what is queued.
        b.mouse("claude", down);
        b.mouse("claude", up);
        let (acts, _) = b.mouse_sent("claude", Err("no".into()));
        assert!(matches!(&acts[..], [Act::Error(_)]));
        assert!(b.sessions["claude"].mouse.is_empty());
    }

    #[test]
    fn mouse_events_need_a_daemon_that_serves_them() {
        let mut b = online(true);
        attached(&mut b, "claude", 7);
        b.arm("claude");
        b.armed("claude", Ok(0));
        let down = click(
            TerminalMouseKind::Down {
                button: libflankr_proto::common::MouseButton::Left,
            },
            1,
        );
        assert!(matches!(&b.mouse("claude", down).0[..], [Act::Error(_)]));
    }

    #[test]
    fn closed_drops_the_screen_and_keeps_the_reason() {
        let mut b = online(false);
        opened(&mut b, "claude", 7);
        assert!(b.closed(TerminalClosedParams {
            terminal_id: 7,
            reason: TerminalClosedReason::PaneClosed
        }));
        let t = &b.sessions["claude"];
        assert!(!t.wanted);
        assert_eq!(t.closed, Some(TerminalClosedReason::PaneClosed));
        // Not reopened on reconnect.
        b.disconnected();
        assert_eq!(
            b.set_info(Some(TerminalInfo {
                input: false,
                mouse: false
            }))
            .0,
            vec![]
        );
    }
}
