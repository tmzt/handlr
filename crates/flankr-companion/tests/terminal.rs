//! The terminal mode over flankr link (plan 20261027_07, step 4 and step
//! 6's client half), against an in-test daemon: a `link::accept` server
//! whose `terminal.open` starts a `libflankr_term::Mirror` of a mock
//! buffer and sends its frames with `send_terminal`. The client's
//! `Screen` must match the buffer after a full frame and a diff, its acks
//! must reach the mirror, a `terminal.closed` drops the terminal, a
//! reconnect opens it again, bytes that are not a frame are not fatal, and
//! keys over 64 bytes go in batches. Scrolls need the arm, coalesce while
//! one is in flight, and their offset lands in the status.
//!
//! Plan 20261027_08, step 9: with an "attached" daemon (`source: attached`,
//! `mouse` in hello), `terminal.open` asks for the cursor and a frame's
//! cursor trailer reaches `Screen::cursor`; `terminal.resize` is
//! deduplicated, one in flight and the newest wins, and the fit is the
//! reopen's hints; `terminal.mouse` needs the arm.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use flankr_companion::{
    Companion, CompanionConfig, FileKeyStore, KeyStore, LEGACY_KEY_NAME, Remotes, ServerRecord,
    TerminalClosedReason,
};
use flankr_companion::{Cursor, MouseButton, TerminalMouseKind, TerminalSource};
use libflankr_proto::link::{self, StaticKeypair};
use libflankr_term::{Mirror, parse_ansi};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const TIMEOUT: Duration = Duration::from_secs(5);
const SESSION: &str = "w1:p1";

/// What the test tells the daemon's connection to do.
enum Cmd {
    /// The pane now shows this (ANSI): a frame from the mirror.
    Buffer(String),
    /// These bytes as a terminal message.
    Raw(Vec<u8>),
    /// `terminal.closed` for the open terminal.
    Closed(&'static str),
    /// Answer the `terminal.scroll`s held while `hold_scroll` was set.
    ReleaseScrolls,
    /// Answer the `terminal.resize`s held while `hold_resize` was set.
    ReleaseResizes,
    /// The pane's cursor is now this: a frame from the mirror.
    Cursor(Cursor),
    /// End the connection.
    Hangup,
}

/// How much of the terminal mode the daemon has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Old {
    /// All of it.
    No,
    /// None: no `terminal` in `hello`, and `terminal.*` is
    /// METHOD_NOT_FOUND.
    Daemon,
    /// `terminal` in `hello`, but `terminal.*` is METHOD_NOT_FOUND.
    Methods,
}

struct Daemon {
    addr: String,
    key: StaticKeypair,
    /// Every message the client sent after `hello`.
    seen: mpsc::Receiver<Value>,
    cmds: UnboundedSender<Cmd>,
    /// While set, `terminal.scroll` is not answered until
    /// [`Cmd::ReleaseScrolls`].
    hold_scroll: Arc<AtomicBool>,
    /// While set, `terminal.resize` is not answered until
    /// [`Cmd::ReleaseResizes`].
    hold_resize: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Ctx {
    key: StaticKeypair,
    client: [u8; 32],
    seen: mpsc::Sender<Value>,
    cmds: Arc<tokio::sync::Mutex<UnboundedReceiver<Cmd>>>,
    /// The pane's buffer, as last set.
    buffer: Arc<Mutex<String>>,
    next_id: Arc<AtomicU32>,
    old: Old,
    hold_scroll: Arc<AtomicBool>,
    hold_resize: Arc<AtomicBool>,
    /// Answers `terminal.open` as an attached terminal, and offers the
    /// mouse in hello.
    attached: bool,
}

impl Daemon {
    fn start(client: [u8; 32], buffer: &str, old: Old, attached: bool) -> Daemon {
        let key = link::generate_keypair().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (seen_tx, seen) = mpsc::channel();
        let (cmds, cmds_rx) = unbounded_channel();
        let hold_scroll = Arc::new(AtomicBool::new(false));
        let hold_resize = Arc::new(AtomicBool::new(false));
        let ctx = Ctx {
            key: key.clone(),
            client,
            seen: seen_tx,
            cmds: Arc::new(tokio::sync::Mutex::new(cmds_rx)),
            buffer: Arc::new(Mutex::new(buffer.to_string())),
            next_id: Arc::new(AtomicU32::new(7)),
            old,
            hold_scroll: hold_scroll.clone(),
            hold_resize: hold_resize.clone(),
            attached,
        };
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    let ctx = ctx.clone();
                    tokio::spawn(async move { ctx.serve(stream).await });
                }
            });
        });
        Daemon {
            addr,
            key,
            seen,
            cmds,
            hold_scroll,
            hold_resize,
        }
    }

    fn send(&self, cmd: Cmd) {
        self.cmds.send(cmd).unwrap();
    }

    /// Whether a message with this method comes within `wait`; the
    /// others are dropped.
    fn comes(&self, method: &str, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.seen.recv_timeout(left) {
                Ok(m) if m["method"] == method => return true,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
    }

    /// The next message from the client with this method.
    fn next(&self, method: &str) -> Value {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let m = self
                .seen
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("a {method} within 5 s"));
            if m["method"] == method {
                return m;
            }
        }
    }
}

impl Ctx {
    async fn serve(self, stream: tokio::net::TcpStream) {
        let client = self.client;
        let Ok(accepted) = link::accept(stream, &self.key, |key| *key == client).await else {
            return;
        };
        let first: Value =
            serde_json::from_slice(&accepted.first_message.unwrap_or_default()).unwrap();
        let mut result = json!({
            "server": "flankr", "server_version": "0.1.0", "protocol_version": 1,
            "session_id": "companion:phone", "vocabulary": []
        });
        if self.old != Old::Daemon {
            result["terminal"] = json!({"input": true, "mouse": self.attached});
        }
        let (mut read, mut write) = accepted.stream.into_split();
        let hello = json!({"jsonrpc": "2.0", "id": first["id"], "result": result});
        if write
            .send_message(&serde_json::to_vec(&hello).unwrap())
            .await
            .is_err()
        {
            return;
        }
        let (from_tx, mut from) = unbounded_channel::<Vec<u8>>();
        let reader = tokio::spawn(async move {
            while let Ok(m) = read.recv_message().await {
                if from_tx.send(m).is_err() {
                    break;
                }
            }
        });
        // One connection takes the commands at a time.
        let mut cmds = self.cmds.lock().await;
        let mut mirror: Option<Mirror> = None;
        // The pane's scroll offset, and the scrolls not answered yet.
        let mut offset: i64 = 0;
        let mut held: Vec<Value> = Vec::new();
        let mut held_resizes: Vec<Value> = Vec::new();
        // The open asked for the cursor trailer; the cursor last set.
        let mut trailer = false;
        let mut cursor: Option<Cursor> = None;
        loop {
            tokio::select! {
                m = from.recv() => {
                    let Some(m) = m else { break };
                    let m: Value = serde_json::from_slice(&m).unwrap();
                    let method = m["method"].as_str().unwrap_or("").to_string();
                    let mut frame = None;
                    if method == "terminal.scroll" && self.hold_scroll.load(Ordering::SeqCst) {
                        held.push(m.clone());
                        let _ = self.seen.send(m);
                        continue;
                    }
                    if method == "terminal.resize" && self.hold_resize.load(Ordering::SeqCst) {
                        held_resizes.push(m.clone());
                        let _ = self.seen.send(m);
                        continue;
                    }
                    let result = if self.old != Old::No && method.starts_with("terminal.") {
                        Err((-32601, "method not found"))
                    } else {
                        match method.as_str() {
                            "terminal.open" => {
                                let id = self.next_id.fetch_add(1, Ordering::SeqCst);
                                let mut mi = Mirror::new(id);
                                let mut grid = parse_ansi(&self.buffer.lock().unwrap());
                                trailer = self.attached && m["params"]["cursor"] == true;
                                grid.cursor = cursor.filter(|_| trailer);
                                let (cols, rows) = (grid.cols, grid.rows);
                                frame = mi.update(grid);
                                mirror = Some(mi);
                                if self.attached {
                                    Ok(json!({"terminal_id": id, "cols": cols, "rows": rows,
                                        "cursor": trailer, "source": "attached"}))
                                } else {
                                    Ok(json!({"terminal_id": id, "cols": cols, "rows": rows}))
                                }
                            }
                            "terminal.ack" => {
                                if let Some(mi) = mirror.as_mut() {
                                    mi.ack(m["params"]["seq"].as_u64().unwrap() as u32);
                                }
                                Ok(Value::Null)
                            }
                            // 0: no expiry (the L3 amendment of
                            // 2026-10-28 10:42 EDT).
                            "terminal.input_arm" => Ok(json!({"expires_in_s": 0})),
                            "terminal.scroll" => {
                                offset = (offset + m["params"]["lines"].as_i64().unwrap()).max(0);
                                Ok(json!({"offset_from_bottom": offset, "max_offset_from_bottom": 500}))
                            }
                            _ => Ok(json!({})),
                        }
                    };
                    if m.get("id").is_some() {
                        let response = match result {
                            Ok(result) => json!({"jsonrpc": "2.0", "id": m["id"], "result": result}),
                            Err((code, message)) => json!({"jsonrpc": "2.0", "id": m["id"],
                                "error": {"code": code, "message": message}}),
                        };
                        if write.send_message(&serde_json::to_vec(&response).unwrap()).await.is_err() {
                            break;
                        }
                    }
                    // The first frame after the answer.
                    if let Some(frame) = frame
                        && write.send_terminal(&frame.encode_with(trailer)).await.is_err()
                    {
                        break;
                    }
                    let _ = self.seen.send(m);
                }
                cmd = cmds.recv() => {
                    let Some(cmd) = cmd else { break };
                    let bytes = match cmd {
                        Cmd::Buffer(text) => {
                            *self.buffer.lock().unwrap() = text.clone();
                            let mut grid = parse_ansi(&text);
                            grid.cursor = cursor.filter(|_| trailer);
                            let Some(frame) = mirror.as_mut().and_then(|mi| mi.update(grid)) else {
                                continue;
                            };
                            frame.encode_with(trailer)
                        }
                        Cmd::Cursor(c) => {
                            cursor = Some(c);
                            let mut grid = parse_ansi(&self.buffer.lock().unwrap());
                            grid.cursor = cursor.filter(|_| trailer);
                            let Some(frame) = mirror.as_mut().and_then(|mi| mi.update(grid)) else {
                                continue;
                            };
                            frame.encode_with(trailer)
                        }
                        Cmd::ReleaseResizes => {
                            self.hold_resize.store(false, Ordering::SeqCst);
                            let mut failed = false;
                            for m in held_resizes.drain(..) {
                                let response = json!({"jsonrpc": "2.0", "id": m["id"], "result": {}});
                                if write.send_message(&serde_json::to_vec(&response).unwrap()).await.is_err() {
                                    failed = true;
                                    break;
                                }
                            }
                            if failed {
                                break;
                            }
                            continue;
                        }
                        Cmd::Raw(bytes) => bytes,
                        Cmd::ReleaseScrolls => {
                            self.hold_scroll.store(false, Ordering::SeqCst);
                            let mut failed = false;
                            for m in held.drain(..) {
                                offset = (offset + m["params"]["lines"].as_i64().unwrap()).max(0);
                                let response = json!({"jsonrpc": "2.0", "id": m["id"],
                                    "result": {"offset_from_bottom": offset}});
                                if write.send_message(&serde_json::to_vec(&response).unwrap()).await.is_err() {
                                    failed = true;
                                    break;
                                }
                            }
                            if failed {
                                break;
                            }
                            continue;
                        }
                        Cmd::Closed(reason) => {
                            let id = mirror.take().map(|mi| mi.terminal_id()).unwrap();
                            let note = json!({"jsonrpc": "2.0", "method": "terminal.closed",
                                "params": {"terminal_id": id, "reason": reason}});
                            if write.send_message(&serde_json::to_vec(&note).unwrap()).await.is_err() {
                                break;
                            }
                            continue;
                        }
                        Cmd::Hangup => break,
                    };
                    if write.send_terminal(&bytes).await.is_err() {
                        break;
                    }
                }
            }
        }
        reader.abort();
        let _ = write.shutdown().await;
    }
}

/// A companion paired with a daemon started on `buffer`.
fn paired(buffer: &str, old: Old) -> (Daemon, Companion, tempfile::TempDir) {
    paired_with(buffer, old, false)
}

/// As [`paired`], the daemon attached (`source: attached`, the mouse
/// offered) when `attached`.
fn paired_with(buffer: &str, old: Old, attached: bool) -> (Daemon, Companion, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(dir.path().join("keys"));
    let client = keys.generate_static_key(LEGACY_KEY_NAME).unwrap().public();
    let daemon = Daemon::start(client, buffer, old, attached);
    let mut record = ServerRecord::new("phone", daemon.key.public, vec![daemon.addr.clone()]);
    record.key_name = LEGACY_KEY_NAME.into();
    keys.set_remotes(&Remotes {
        active: Some(record.id),
        list: vec![record],
    })
    .unwrap();
    let mut config = CompanionConfig::new("test");
    config.socket = dir.path().join("no-socket");
    config.backoff_initial = Duration::from_millis(50);
    config.backoff_max = Duration::from_millis(200);
    let c = Companion::spawn(config, Box::new(FileKeyStore::new(dir.path().join("keys"))));
    (daemon, c, dir)
}

/// Waits until `f` holds of the companion.
fn until(c: &Companion, what: &str, f: impl Fn(&Companion) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !f(c) {
        assert!(Instant::now() < deadline, "{what} within 5 s");
        // Drain the events, as the app's loop would.
        let _ = c.poll().count();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn shows(c: &Companion, text: &str) -> bool {
    c.terminals()
        // The rows only: an attached frame carries the cursor beside them.
        .with_screen(SESSION, |s| {
            let want = parse_ansi(text);
            (s.grid().cols, s.grid().rows, &s.grid().lines) == (want.cols, want.rows, &want.lines)
        })
        .unwrap_or(false)
}

const ONE: &str = "\x1b[1mclaude\x1b[0m ready\n> \nstatus: idle";
/// As wide as [`ONE`], so the mirror sends it as a diff.
const TWO: &str = "\x1b[1mclaude\x1b[0m ready\n> ls\nstatus: \x1b[32mbusy\x1b[0m";

#[test]
fn two_is_a_diff_from_one() {
    let mut m = Mirror::new(7);
    assert!(m.update(parse_ansi(ONE)).unwrap().is_full());
    m.ack(1);
    let diff = m.update(parse_ansi(TWO)).unwrap();
    assert!(!diff.is_full());
    assert_eq!(diff.diffs.len(), 2);
}

#[test]
fn a_full_frame_then_a_diff_match_the_buffer_and_are_acked() {
    let (daemon, c, _dir) = paired(ONE, Old::No);
    until(&c, "the terminal mode", |c| c.terminals().supported());
    assert!(c.terminals().input_allowed());
    let changes = c.terminals().changes();
    c.terminal_open(SESSION, Some(60), Some(30));
    let open = daemon.next("terminal.open");
    assert_eq!(
        open["params"],
        json!({"pane_id": SESSION, "cols_hint": 60, "rows_hint": 30, "cursor": true})
    );
    let ack = daemon.next("terminal.ack");
    assert_eq!(ack["params"], json!({"terminal_id": 7, "seq": 1}));
    assert!(ack.get("id").is_none(), "an ack is a notification");
    assert!(shows(&c, ONE));
    assert!(c.terminals().changes() > changes);
    let status = c.terminals().status(SESSION).unwrap();
    assert!(status.open && !status.opening);
    let grid = parse_ansi(ONE);
    assert_eq!((status.cols, status.rows), (grid.cols, 3));

    // Bytes that are not a frame are dropped; the link stays.
    daemon.send(Cmd::Raw(vec![1, 2, 3]));
    daemon.send(Cmd::Buffer(TWO.into()));
    let ack = daemon.next("terminal.ack");
    assert_eq!(ack["params"], json!({"terminal_id": 7, "seq": 2}));
    until(&c, "the diff shown", |c| shows(c, TWO));
    let screen = c.terminals().screen(SESSION).unwrap();
    assert_eq!(screen.seq(), 2);
    assert_eq!(screen.grid().text_lines()[2], "status: busy");

    c.terminal_close(SESSION);
    assert_eq!(
        daemon.next("terminal.close")["params"],
        json!({"terminal_id": 7})
    );
    until(&c, "the terminal forgotten", |c| {
        c.terminals().status(SESSION).is_none()
    });
}

#[test]
fn terminal_closed_drops_it_with_its_reason() {
    let (daemon, c, _dir) = paired(ONE, Old::No);
    until(&c, "the terminal mode", |c| c.terminals().supported());
    c.terminal_open(SESSION, None, None);
    daemon.next("terminal.ack");
    daemon.send(Cmd::Closed("pane_closed"));
    until(&c, "the reason", |c| {
        c.terminals().closed_reason(SESSION) == Some(TerminalClosedReason::PaneClosed)
    });
    assert!(c.terminals().screen(SESSION).is_none());
    let status = c.terminals().status(SESSION).unwrap();
    assert!(!status.open && !status.opening);
}

#[test]
fn a_reconnect_opens_the_terminal_again_on_a_fresh_screen() {
    let (daemon, c, _dir) = paired(ONE, Old::No);
    until(&c, "the terminal mode", |c| c.terminals().supported());
    c.terminal_open(SESSION, None, None);
    daemon.next("terminal.ack");
    daemon.send(Cmd::Buffer(TWO.into()));
    daemon.next("terminal.ack");
    until(&c, "seq 2", |c| {
        c.terminals().screen(SESSION).is_some_and(|s| s.seq() == 2)
    });

    daemon.send(Cmd::Hangup);
    // The buffer the new terminal starts from is TWO; a new mirror numbers
    // from 1, which a fresh screen shows.
    let open = daemon.next("terminal.open");
    assert_eq!(open["params"]["pane_id"], SESSION);
    let ack = daemon.next("terminal.ack");
    assert_eq!(ack["params"], json!({"terminal_id": 8, "seq": 1}));
    until(&c, "the new terminal's frame", |c| {
        c.terminals().screen(SESSION).is_some_and(|s| s.seq() == 1)
    });
    assert!(shows(&c, TWO));
    daemon.send(Cmd::Buffer(ONE.into()));
    assert_eq!(
        daemon.next("terminal.ack")["params"],
        json!({"terminal_id": 8, "seq": 2})
    );
    until(&c, "the diff on the new terminal", |c| shows(c, ONE));
}

#[test]
fn keys_over_64_bytes_go_in_batches_after_the_arm() {
    let (daemon, c, _dir) = paired(ONE, Old::No);
    until(&c, "the terminal mode", |c| c.terminals().supported());
    c.terminal_open(SESSION, None, None);
    daemon.next("terminal.ack");
    c.terminal_arm_input(SESSION);
    assert_eq!(
        daemon.next("terminal.input_arm")["params"],
        json!({"terminal_id": 7})
    );
    until(&c, "armed", |c| c.terminals().armed(SESSION));
    // No expiry.
    assert_eq!(
        c.terminals().status(SESSION).unwrap().armed_for,
        Some(Duration::MAX)
    );

    let keys = "echo the quick brown fox jumps over the lazy dog and keeps on running enter";
    assert!(keys.len() > 64);
    c.terminal_send_keys(SESSION, keys);
    let first = daemon.next("terminal.input");
    let second = daemon.next("terminal.input");
    let a = first["params"]["keys"].as_str().unwrap();
    let b = second["params"]["keys"].as_str().unwrap();
    assert!(a.len() <= 64 && b.len() <= 64);
    assert!(a.ends_with(' '), "cut on a token boundary: {a:?}");
    assert_eq!(format!("{a}{b}"), keys);
    assert_eq!(first["params"]["terminal_id"], 7);
}

#[test]
fn an_older_daemon_has_no_terminal_mode() {
    let (_daemon, c, _dir) = paired(ONE, Old::Daemon);
    // Connected, but no `terminal` in hello.
    std::thread::sleep(Duration::from_millis(300));
    let _ = c.poll().count();
    assert!(!c.terminals().supported());
    c.terminal_open(SESSION, None, None);
    until(&c, "the refusal", |c| {
        c.terminals()
            .status(SESSION)
            .is_some_and(|s| s.error.is_some() && !s.opening)
    });
}

#[test]
fn method_not_found_is_no_terminal_mode() {
    let (daemon, c, _dir) = paired(ONE, Old::Methods);
    until(&c, "the terminal mode", |c| c.terminals().supported());
    c.terminal_open(SESSION, None, None);
    daemon.next("terminal.open");
    until(&c, "no terminal mode", |c| !c.terminals().supported());
    let status = c.terminals().status(SESSION).unwrap();
    assert!(!status.open && !status.opening);
    assert_eq!(
        status.error.as_deref(),
        Some("the daemon offers no terminal mode")
    );
}

/// A companion with the terminal open (terminal 7) and, if `arm`, armed.
fn open_terminal(arm: bool) -> (Daemon, Companion, tempfile::TempDir) {
    let (daemon, c, dir) = paired(ONE, Old::No);
    until(&c, "the terminal mode", |c| c.terminals().supported());
    c.terminal_open(SESSION, None, None);
    daemon.next("terminal.ack");
    until(&c, "open", |c| {
        c.terminals().status(SESSION).is_some_and(|s| s.open)
    });
    if arm {
        c.terminal_arm_input(SESSION);
        daemon.next("terminal.input_arm");
        until(&c, "armed", |c| c.terminals().armed(SESSION));
    }
    (daemon, c, dir)
}

#[test]
fn a_scroll_needs_the_arm() {
    let (daemon, c, _dir) = open_terminal(false);
    c.terminal_scroll(SESSION, 5);
    until(&c, "the refusal", |c| {
        c.terminals()
            .status(SESSION)
            .is_some_and(|s| s.error.as_deref() == Some("terminal input is not armed"))
    });
    assert!(!daemon.comes("terminal.scroll", Duration::from_millis(200)));

    c.terminal_arm_input(SESSION);
    daemon.next("terminal.input_arm");
    until(&c, "armed", |c| c.terminals().armed(SESSION));
    c.terminal_scroll(SESSION, -2);
    // The refused 5 is not carried over.
    assert_eq!(
        daemon.next("terminal.scroll")["params"],
        json!({"terminal_id": 7, "lines": -2})
    );
}

#[test]
fn scrolls_coalesce_while_one_is_in_flight_and_the_offset_lands_in_the_status() {
    let (daemon, c, _dir) = open_terminal(true);
    assert_eq!(c.terminals().status(SESSION).unwrap().scroll_offset, None);
    daemon.hold_scroll.store(true, Ordering::SeqCst);
    c.terminal_scroll(SESSION, 3);
    assert_eq!(daemon.next("terminal.scroll")["params"]["lines"], 3);
    c.terminal_scroll(SESSION, 2);
    c.terminal_scroll(SESSION, 4);
    c.terminal_scroll(SESSION, 0);
    // Nothing more while the first is unanswered.
    assert!(!daemon.comes("terminal.scroll", Duration::from_millis(300)));

    let changes = c.terminals().changes();
    daemon.send(Cmd::ReleaseScrolls);
    // The sum goes once the first is answered.
    assert_eq!(
        daemon.next("terminal.scroll")["params"],
        json!({"terminal_id": 7, "lines": 6})
    );
    until(&c, "the offset", |c| {
        c.terminals()
            .status(SESSION)
            .is_some_and(|s| s.scroll_offset == Some(9))
    });
    let status = c.terminals().status(SESSION).unwrap();
    assert_eq!(status.scroll_max, Some(500));
    assert!(c.terminals().changes() > changes);
    assert!(!daemon.comes("terminal.scroll", Duration::from_millis(200)));
}

/// An attached terminal open (terminal 7) at a fit of 60x30.
fn attached_terminal() -> (Daemon, Companion, tempfile::TempDir) {
    let (daemon, c, dir) = paired_with(ONE, Old::No, true);
    until(&c, "the terminal mode", |c| c.terminals().supported());
    assert!(c.terminals().mouse_allowed());
    c.terminal_open(SESSION, Some(60), Some(30));
    assert_eq!(daemon.next("terminal.open")["params"]["cursor"], true);
    daemon.next("terminal.ack");
    until(&c, "open", |c| {
        c.terminals().status(SESSION).is_some_and(|s| s.open)
    });
    (daemon, c, dir)
}

#[test]
fn the_open_asks_for_the_cursor_and_a_trailer_reaches_the_screen() {
    let (daemon, c, _dir) = attached_terminal();
    let status = c.terminals().status(SESSION).unwrap();
    assert!(status.cursor);
    assert_eq!(status.source, TerminalSource::Attached);
    assert_eq!(c.terminals().screen(SESSION).unwrap().cursor(), None);
    let cursor = Cursor {
        col: 2,
        row: 1,
        visible: true,
        shape: 6,
        mouse_reporting: true,
        alternate_screen: false,
        stale: false,
    };
    daemon.send(Cmd::Cursor(cursor));
    // A cursor-only change is a frame of its own, acknowledged.
    assert_eq!(
        daemon.next("terminal.ack")["params"],
        json!({"terminal_id": 7, "seq": 2})
    );
    until(&c, "the cursor", |c| {
        c.terminals()
            .with_screen(SESSION, |s| s.cursor() == Some(cursor))
            .unwrap_or(false)
    });
    assert!(shows(&c, ONE));
    // Stale, then a new buffer: the cursor rides along.
    let stale = Cursor {
        stale: true,
        ..cursor
    };
    daemon.send(Cmd::Cursor(stale));
    daemon.send(Cmd::Buffer(TWO.into()));
    until(&c, "the diff with the stale cursor", |c| {
        shows(c, TWO)
            && c.terminals()
                .with_screen(SESSION, |s| s.cursor() == Some(stale))
                .unwrap_or(false)
    });
}

#[test]
fn a_polled_daemon_answers_the_poll_and_sends_no_cursor() {
    let (daemon, c, _dir) = open_terminal(false);
    let status = c.terminals().status(SESSION).unwrap();
    assert!(!status.cursor);
    assert_eq!(status.source, TerminalSource::Poll);
    assert!(!c.terminals().mouse_allowed());
    // A polled pane is not resized.
    c.terminal_resize(SESSION, 50, 20);
    assert!(!daemon.comes("terminal.resize", Duration::from_millis(300)));
}

#[test]
fn resizes_are_deduplicated_and_the_newest_wins() {
    let (daemon, c, _dir) = attached_terminal();
    // The open's own size: nothing.
    c.terminal_resize(SESSION, 60, 30);
    assert!(!daemon.comes("terminal.resize", Duration::from_millis(200)));
    daemon.hold_resize.store(true, Ordering::SeqCst);
    c.terminal_resize(SESSION, 60, 18);
    assert_eq!(
        daemon.next("terminal.resize")["params"],
        json!({"terminal_id": 7, "cols": 60, "rows": 18})
    );
    // While it is in flight, three more: only the last goes.
    c.terminal_resize(SESSION, 60, 17);
    c.terminal_resize(SESSION, 58, 17);
    c.terminal_resize(SESSION, 44, 17);
    assert!(!daemon.comes("terminal.resize", Duration::from_millis(300)));
    daemon.send(Cmd::ReleaseResizes);
    assert_eq!(
        daemon.next("terminal.resize")["params"],
        json!({"terminal_id": 7, "cols": 44, "rows": 17})
    );
    c.terminal_resize(SESSION, 44, 17);
    assert!(!daemon.comes("terminal.resize", Duration::from_millis(200)));

    // A reconnect reopens at the current fit.
    daemon.send(Cmd::Hangup);
    let open = daemon.next("terminal.open");
    assert_eq!(
        open["params"],
        json!({"pane_id": SESSION, "cols_hint": 44, "rows_hint": 17, "cursor": true})
    );
}

#[test]
fn a_mouse_event_needs_the_arm_and_keeps_it() {
    let (daemon, c, _dir) = attached_terminal();
    let down = TerminalMouseKind::Down {
        button: MouseButton::Left,
    };
    c.terminal_mouse(SESSION, down, 4, 2, 0);
    until(&c, "the refusal", |c| {
        c.terminals()
            .status(SESSION)
            .is_some_and(|s| s.error.as_deref() == Some("terminal input is not armed"))
    });
    assert!(!daemon.comes("terminal.mouse", Duration::from_millis(200)));
    c.terminal_arm_input(SESSION);
    daemon.next("terminal.input_arm");
    until(&c, "armed", |c| c.terminals().armed(SESSION));
    c.terminal_mouse(SESSION, down, 4, 2, 0);
    c.terminal_mouse(
        SESSION,
        TerminalMouseKind::Up {
            button: MouseButton::Left,
        },
        4,
        2,
        0,
    );
    assert_eq!(
        daemon.next("terminal.mouse")["params"],
        json!({"terminal_id": 7, "kind": "down", "button": "left", "col": 4, "row": 2})
    );
    assert_eq!(
        daemon.next("terminal.mouse")["params"],
        json!({"terminal_id": 7, "kind": "up", "button": "left", "col": 4, "row": 2})
    );
    // Accepted: still armed, with no expiry.
    assert!(c.terminals().armed(SESSION));
    assert_eq!(
        c.terminals().status(SESSION).unwrap().armed_for,
        Some(Duration::MAX)
    );
}
