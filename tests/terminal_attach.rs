//! The terminal read from the daemon's herdr attachment, end to end over
//! the link (docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md, steps 7
//! and 8, and its amendments of 23:24, 23:32 and 23:42 EDT): a companion
//! over TCP on 127.0.0.1, the JSON API on the herdr mock, and the
//! attachment pointed by the test hook at an in-process fake herdr
//! (`libherdr_term::fake`) on a socket in a temporary directory. Nothing
//! touches `~/.config/flankr`, `~/.config/herdr` or a live herdr.

use std::collections::VecDeque;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flankr::config::{AgentConfig, AppConfig, TerminalConfig};
use flankr::server::companion::TurnOptions;
use flankr::server::herdr_attach::AttachSettings;
use flankr::server::link::auth::{self, AuthEntry};
use flankr::server::link::{Link, LinkSettings};
use flankr::server::{self, Daemon, ServeOptions};
use flankr::sessions::Sessions;
use flankr::transport::HerdrClient;
use libflankr_proto::envelope::codes;
use libflankr_proto::link::{LinkStream, Message, StaticKeypair, generate_keypair, reconnect};
use libflankr_term::{Frame, Screen};
use libherdr::protocol::{PaneInfo, TabInfo};
use libherdr::transport::{HerdrTransport, MockState};
use libherdr_term::fake::{self, Fake};
use libherdr_term::herdr::{
    self, ClientKeyCode, ClientKeyKind, ClientMessage, ClientMouseButton, ClientMouseKind,
    ClientMousePosition, ClientPaneInputEvent, ServerMessage,
};
use serde_json::{Value, json};
use tokio::net::TcpStream;

const WAIT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------
// The fake herdr.

/// How the fake answers the hello.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Generation 1, serving `pane.focus` and `client_shell.surface.set`.
    Full,
    /// Generation 1 without `client_shell.surface.set` (older herdr).
    NoSurfaceSet,
    /// Refuses generation 1.
    Incompatible,
}

#[derive(Default)]
struct FakeState {
    cols: u16,
    rows: u16,
    active: bool,
    focused: String,
    revision: u64,
    /// The desktop holds the tab's geometry: the pane's PTY is not the
    /// surface's rectangle (cleared by flankr's next `pane.focus`).
    desktop: bool,
    offset: u64,
    /// The current connection's writer.
    writer: Option<UnixStream>,
    /// What each connection's client sent, after its hello: (connection,
    /// message).
    log: Vec<(usize, ClientMessage)>,
    /// Each hello's `surface_active`, by connection.
    hellos: Vec<bool>,
}

struct FakeHerdr {
    socket: PathBuf,
    state: Arc<Mutex<FakeState>>,
    accepted: Arc<AtomicUsize>,
}

/// Row 0 of the fake's pane.
fn fake_text(pane: &str) -> String {
    format!("hi from {pane}")
}

fn draw(state: &mut FakeState) {
    if !state.active {
        return;
    }
    state.revision += 1;
    let (cols, rows) = (state.cols, state.rows);
    let mut pane = fake::pane(&state.focused, fake::rect(0, 0, cols, rows), true);
    if let Some(scroll) = pane.scroll.as_mut() {
        scroll.offset_from_bottom = state.offset;
        scroll.max_offset_from_bottom = 500;
        if state.desktop {
            scroll.viewport_rows += 10;
        }
    }
    let mut surface = fake::surface(
        state.revision,
        cols,
        rows,
        " ",
        vec![pane],
        Some(herdr::CursorState {
            x: 3,
            y: 0,
            visible: true,
            shape: 2,
        }),
    );
    for (i, ch) in fake_text(&state.focused).chars().enumerate() {
        if i < usize::from(cols) {
            surface.frame.cells[i] = fake::cell(&ch.to_string(), 0, 0, 0);
        }
    }
    if let Some(writer) = state.writer.as_ref() {
        let _ = herdr::write_message(&mut &*writer, &ServerMessage::PaneSurface(surface));
    }
}

impl FakeHerdr {
    fn start(dir: &std::path::Path, kind: Kind) -> FakeHerdr {
        let socket = dir.join("herdr-client.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let state = Arc::new(Mutex::new(FakeState {
            focused: "p1".into(),
            ..FakeState::default()
        }));
        let accepted = Arc::new(AtomicUsize::new(0));
        let (st, acc) = (Arc::clone(&state), Arc::clone(&accepted));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let n = acc.fetch_add(1, Ordering::SeqCst) + 1;
                let st = Arc::clone(&st);
                std::thread::spawn(move || serve(n, stream, kind, st));
            }
        });
        FakeHerdr {
            socket,
            state,
            accepted,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut FakeState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    fn log(&self) -> Vec<(usize, ClientMessage)> {
        self.with(|s| s.log.clone())
    }

    /// The endpoint requests sent, as (method, params).
    fn requests(&self) -> Vec<(String, Value)> {
        self.log()
            .into_iter()
            .filter_map(|(_, m)| match m {
                ClientMessage::ClientShellEndpointRequest { request, .. } => {
                    let v: Value = serde_json::from_str(&request).unwrap();
                    Some((
                        v["method"].as_str().unwrap().to_string(),
                        v["params"].clone(),
                    ))
                }
                _ => None,
            })
            .collect()
    }

    fn focuses(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|(m, _)| m == "pane.focus")
            .map(|(_, p)| p["pane_id"].as_str().unwrap().to_string())
            .collect()
    }

    fn actives(&self) -> Vec<bool> {
        self.requests()
            .into_iter()
            .filter(|(m, _)| m == "client_shell.surface.set")
            .map(|(_, p)| p["active"].as_bool().unwrap())
            .collect()
    }

    fn resizes(&self) -> Vec<(u16, u16)> {
        self.log()
            .into_iter()
            .filter_map(|(_, m)| match m {
                ClientMessage::ClientShellResize { surface_size, .. } => {
                    Some((surface_size.cols, surface_size.rows))
                }
                _ => None,
            })
            .collect()
    }

    fn inputs(&self) -> Vec<(String, Vec<ClientPaneInputEvent>)> {
        self.log()
            .into_iter()
            .filter_map(|(_, m)| match m {
                ClientMessage::ClientShellPaneInput { pane_id, events } => Some((pane_id, events)),
                _ => None,
            })
            .collect()
    }

    fn detaches(&self) -> usize {
        self.log()
            .iter()
            .filter(|(_, m)| matches!(m, ClientMessage::Detach))
            .count()
    }

    /// The desktop takes the tab: the pane's PTY at the desktop's size.
    fn desktop_takes_the_tab(&self) {
        self.with(|s| {
            s.desktop = true;
            draw(s);
        });
    }

    /// Redraw, as herdr does when the pane's output changes.
    fn redraw(&self) {
        self.with(draw);
    }

    /// herdr shuts down: it says so, and the connection ends.
    fn drop_connection(&self) {
        self.with(|s| {
            if let Some(w) = s.writer.take() {
                let _ = herdr::write_message(
                    &mut &w,
                    &ServerMessage::ServerShutdown {
                        reason: Some("restarting".into()),
                    },
                );
                let _ = w.shutdown(std::net::Shutdown::Both);
            }
        });
    }
}

fn serve(n: usize, stream: UnixStream, kind: Kind, state: Arc<Mutex<FakeState>>) {
    let mut fake = Fake {
        stream: stream.try_clone().unwrap(),
    };
    let hello = fake.read_hello();
    if kind == Kind::Incompatible {
        let welcome = herdr::EndpointServerWelcome {
            generation: 1,
            server_version: "0.0.1-old".into(),
            snapshot_codec: String::new(),
            surface_codec: String::new(),
            input_codec: String::new(),
            blob_codec: String::new(),
            methods: Vec::new(),
            capabilities: Vec::new(),
            error: Some(herdr::EndpointHandshakeError {
                code: "unsupported_generation".into(),
                message: "not generation 1".into(),
            }),
        };
        fake.send(&ServerMessage::EndpointControl {
            kind: herdr::ENDPOINT_WELCOME_KIND.into(),
            data: serde_json::to_string(&welcome).unwrap(),
        });
        return;
    }
    let methods: &[&str] = match kind {
        Kind::NoSurfaceSet => &["pane.focus"],
        _ => &["pane.focus", "client_shell.surface.set"],
    };
    let focused = {
        let mut s = state.lock().unwrap();
        s.hellos.push(hello.surface_active);
        s.cols = hello.surface_size.cols;
        s.rows = hello.surface_size.rows;
        // A herdr without surface.set ignores surface_active.
        s.active = hello.surface_active || kind == Kind::NoSurfaceSet;
        s.writer = Some(stream.try_clone().unwrap());
        s.focused.clone()
    };
    fake.welcome(methods);
    fake.snapshot("boot-1", Some(&focused));
    {
        let mut s = state.lock().unwrap();
        draw(&mut s);
    }
    loop {
        let Ok(msg) = fake.try_read() else {
            return;
        };
        let mut s = state.lock().unwrap();
        s.log.push((n, msg.clone()));
        match msg {
            ClientMessage::ClientShellEndpointRequest { request, .. } => {
                let v: Value = serde_json::from_str(&request).unwrap();
                let id = v["id"].as_str().unwrap().to_string();
                match v["method"].as_str().unwrap() {
                    "pane.focus" => {
                        s.focused = v["params"]["pane_id"].as_str().unwrap().to_string();
                        s.desktop = false;
                    }
                    "client_shell.surface.set" => {
                        s.active = v["params"]["active"].as_bool().unwrap();
                    }
                    _ => {}
                }
                let answer = json!({"id": id, "result": {}}).to_string();
                if let Some(w) = s.writer.as_ref() {
                    let mut f = Fake {
                        stream: w.try_clone().unwrap(),
                    };
                    f.answer("boot-1", &id, &answer);
                }
                draw(&mut s);
            }
            ClientMessage::ClientShellResize { surface_size, .. } => {
                s.cols = surface_size.cols;
                s.rows = surface_size.rows;
                draw(&mut s);
            }
            ClientMessage::ClientShellPaneInput { events, .. } => {
                for e in events {
                    if let ClientPaneInputEvent::Mouse { kind, lines, .. } = e {
                        match kind {
                            ClientMouseKind::ScrollUp => s.offset += u64::from(lines),
                            ClientMouseKind::ScrollDown => {
                                s.offset = s.offset.saturating_sub(u64::from(lines))
                            }
                            _ => {}
                        }
                    }
                }
                draw(&mut s);
            }
            ClientMessage::Detach => {
                s.writer = None;
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------
// The daemon and the phone (as tests/terminal.rs has them).

struct Env {
    dir: tempfile::TempDir,
    address: String,
    link: Arc<Link>,
    daemon: Arc<Daemon>,
    mock: HerdrClient,
    herdr: FakeHerdr,
}

fn config() -> AppConfig {
    AppConfig {
        agents: vec![
            AgentConfig::new("claude", vec!["continue".to_string()], true)
                .with_terminal_input(true, false),
        ],
        summary: Some(flankr::config::SummaryConfig { cards: false }),
        terminal: TerminalConfig {
            input: true,
            ..TerminalConfig::default()
        },
        ..AppConfig::default()
    }
}

fn pane_info(id: &str, focused: bool) -> PaneInfo {
    let mut extra = serde_json::Map::new();
    extra.insert("agent".to_string(), json!("claude"));
    PaneInfo {
        pane_id: id.to_string(),
        terminal_title: None,
        cwd: Some("/tmp".to_string()),
        agent_status: None,
        focused,
        command: None,
        agent_session: None,
        extra,
    }
}

/// The mock with `p1` focused; `p2` on the same tab, `p3` on another.
fn mock_state() -> MockState {
    let mut state = MockState::default();
    state
        .pane_buffers
        .insert("p1".to_string(), vec!["hello".to_string()]);
    state
        .pane_buffers
        .insert("p2".to_string(), vec!["two".to_string()]);
    state
        .pane_buffers
        .insert("p3".to_string(), vec!["three".to_string()]);
    let ws = &mut state.snapshot.workspaces[0];
    ws.tabs[0].panes.push(pane_info("p2", false));
    ws.tabs.push(TabInfo {
        tab_id: "t2".to_string(),
        title: None,
        panes: vec![pane_info("p3", false)],
        focused: false,
    });
    state
}

async fn start(kind: Kind) -> Env {
    start_with(kind, config(), Duration::from_millis(50)).await
}

async fn start_with(kind: Kind, config: AppConfig, backoff: Duration) -> Env {
    let dir = tempfile::Builder::new()
        .prefix("hta")
        .tempdir_in("/tmp")
        .unwrap();
    let herdr = FakeHerdr::start(dir.path(), kind);
    let link = Link::open(LinkSettings {
        key_path: dir.path().join("link_key"),
        auth_path: dir.path().join("auth.toml"),
        listen: Some(vec!["127.0.0.1:0".to_string()]),
        revoke_interval: Duration::from_secs(60),
    })
    .unwrap();
    let listeners = link.bind().await.unwrap();
    let address = listeners[0].local_addr().unwrap().to_string();
    let mock = HerdrClient::mock_with_state(mock_state());
    let daemon = Daemon::with_turns(
        mock.clone(),
        &config,
        Some(Arc::clone(&link)),
        TurnOptions {
            session_dir: Some(dir.path().join("history")),
            agent_sessions: Some(Arc::new(Sessions::with_home(&dir.path().join("home")))),
            herdr_attach: Some(AttachSettings {
                backoff_initial: backoff,
                backoff_max: backoff * 4,
                handshake_timeout: Duration::from_secs(2),
                ..AttachSettings::new(&herdr.socket)
            }),
            ..TurnOptions::default()
        },
    );
    let unix = server::bind(&dir.path().join("h.sock")).await.unwrap();
    server::link::spawn(Arc::clone(&daemon), listeners);
    tokio::spawn(server::run(
        Arc::clone(&daemon),
        unix,
        ServeOptions {
            show_tokens: false,
            event_transport: None,
        },
    ));
    Env {
        dir,
        address,
        link,
        daemon,
        mock,
        herdr,
    }
}

/// Wait until `f` holds, or fail with `what`.
async fn until(what: &str, mut f: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

impl Env {
    async fn attached(&self) {
        let attach = self.daemon.herdr_attach.clone().expect("the attachment");
        until("the attachment", || attach.usable()).await;
    }

    async fn mock<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        let HerdrTransport::Mock { state } = self.mock.transport() else {
            unreachable!()
        };
        f(&mut *state.write().await)
    }

    async fn device(&self, name: &str, approve: bool) -> Client {
        let keypair = generate_keypair().unwrap();
        let mut entry = AuthEntry::new(name, &keypair.public, vec!["companion".to_string()]);
        entry.approve = Some(approve);
        auth::add(&self.dir.path().join("auth.toml"), entry, false).unwrap();
        let hello = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 0, "method": "hello",
            "params": {"protocol": "flankr.companion", "version": 1, "client": "test"}
        }))
        .unwrap();
        let key = StaticKeypair::from_private(keypair.private).unwrap();
        let tcp = TcpStream::connect(&self.address).await.unwrap();
        let stream = reconnect(tcp, &key, &self.link.public_key(), Some(&hello))
            .await
            .unwrap();
        let mut client = Client {
            stream,
            next_id: 0,
            notes: Vec::new(),
            frames: VecDeque::new(),
            hello: Value::Null,
        };
        client.hello = client.reply(0).await;
        client
    }
}

struct Client {
    stream: LinkStream<TcpStream>,
    next_id: i64,
    notes: Vec<Value>,
    frames: VecDeque<Frame>,
    hello: Value,
}

impl Client {
    async fn pump(&mut self) -> Option<Value> {
        let message = tokio::time::timeout(WAIT, self.stream.recv())
            .await
            .expect("no message in time")
            .expect("the link closed");
        match message {
            Message::Json(bytes) => {
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                if value.get("method").is_some() {
                    self.notes.push(value);
                    None
                } else {
                    Some(value)
                }
            }
            Message::Terminal(bytes) => {
                self.frames
                    .push_back(Frame::decode(&bytes).expect("a frame that decodes"));
                None
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn reply(&mut self, id: i64) -> Value {
        loop {
            if let Some(r) = self.pump().await
                && r["id"] == id
            {
                return r;
            }
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.stream
            .send_message(&serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        self.reply(id).await
    }

    async fn notify(&mut self, method: &str, params: Value) {
        let note = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.stream
            .send_message(&serde_json::to_vec(&note).unwrap())
            .await
            .unwrap();
    }

    async fn frame(&mut self) -> Frame {
        loop {
            if let Some(frame) = self.frames.pop_front() {
                return frame;
            }
            self.pump().await;
        }
    }

    /// Frames until one satisfies `f`.
    async fn frame_where(&mut self, screen: &mut Screen, f: impl Fn(&Frame, &Screen) -> bool) {
        let until = tokio::time::Instant::now() + WAIT;
        loop {
            let frame = self.frame().await;
            screen.apply(&frame);
            if f(&frame, screen) {
                return;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "no such frame; the screen shows {:?}, cursor {:?}",
                screen.grid().text_lines(),
                screen.cursor()
            );
        }
    }

    async fn note(&mut self, method: &str) -> Value {
        loop {
            if let Some(i) = self.notes.iter().position(|n| n["method"] == method) {
                return self.notes.remove(i);
            }
            self.pump().await;
        }
    }

    async fn open(&mut self, params: Value) -> Value {
        let r = self.call("terminal.open", params).await;
        assert!(r["result"]["terminal_id"].is_u64(), "{r}");
        r["result"].clone()
    }

    async fn arm(&mut self, terminal_id: u64) {
        let r = self
            .call("terminal.input_arm", json!({"terminal_id": terminal_id}))
            .await;
        assert_eq!(r["result"]["expires_in_s"], 60, "{r}");
    }
}

fn code(r: &Value) -> i64 {
    r["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("not an error: {r}"))
}

fn first_row(screen: &Screen) -> String {
    screen
        .grid()
        .text_lines()
        .first()
        .cloned()
        .unwrap_or_default()
}

// ---------------------------------------------------------------------
// Step 7: the read side.

/// Opening a terminal makes the surface active, focuses the pane and sizes
/// the surface to the phone; frames carry the cursor; a surface change is
/// a diff with the cursor; closing the last terminal makes the surface
/// inactive, without detaching.
#[tokio::test]
async fn opening_attaches_and_focuses_and_frames_carry_the_cursor() {
    let env = start(Kind::Full).await;
    env.attached().await;
    // Inactive while no terminal is open.
    assert_eq!(env.herdr.with(|s| s.hellos.clone()), [false]);
    let mut phone = env.device("s23", true).await;
    assert_eq!(
        phone.hello["result"]["terminal"],
        json!({"input": true, "mouse": true})
    );

    let opened = phone
        .open(json!({"pane_id": "p1", "cols_hint": 60, "rows_hint": 20, "cursor": true}))
        .await;
    assert_eq!(opened["source"], "attached", "{opened}");
    assert_eq!(opened["cursor"], true);
    assert_eq!(
        (opened["cols"].as_u64(), opened["rows"].as_u64()),
        (Some(60), Some(20))
    );
    assert_eq!(env.herdr.actives(), [true]);
    assert_eq!(env.herdr.focuses(), ["p1"]);
    assert_eq!(env.herdr.resizes(), [(60, 20)]);

    let first = phone.frame().await;
    let mut screen = Screen::default();
    assert!(screen.apply(&first));
    assert_eq!(first_row(&screen), "hi from p1");
    let cursor = first.cursor.expect("the cursor");
    assert_eq!(
        (cursor.col, cursor.row, cursor.visible, cursor.stale),
        (3, 0, true, false)
    );
    // No pane.read for an attached terminal.
    assert!(env.mock(|m| m.read_log.is_empty()).await);

    phone
        .notify("terminal.ack", json!({"terminal_id": 1, "seq": first.seq}))
        .await;
    assert!(phone.call("ping", json!({})).await["result"].is_object());
    // herdr draws a change: a diff from the acknowledged frame.
    env.herdr.with(|s| {
        if let Some(w) = s.writer.as_ref() {
            let mut surface = fake::surface(
                s.revision + 1,
                60,
                20,
                " ",
                vec![fake::pane("p1", fake::rect(0, 0, 60, 20), true)],
                Some(herdr::CursorState {
                    x: 5,
                    y: 1,
                    visible: true,
                    shape: 0,
                }),
            );
            s.revision += 1;
            for (i, ch) in "hi from p1".chars().enumerate() {
                surface.frame.cells[i] = fake::cell(&ch.to_string(), 0, 0, 0);
            }
            surface.frame.cells[60] = fake::cell("$", 0, 0, 0);
            let _ = herdr::write_message(&mut &*w, &ServerMessage::PaneSurface(surface));
        }
    });
    let diff = phone.frame().await;
    assert_eq!(diff.base_seq, first.seq, "{diff:?}");
    assert_eq!(diff.diffs.len(), 1);
    assert_eq!(diff.diffs[0].row, 1);
    assert_eq!(diff.cursor.map(|c| (c.col, c.row)), Some((5, 1)));

    let r = phone
        .call("terminal.close", json!({"terminal_id": 1}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    until("the surface inactive", || {
        env.herdr.actives() == [true, false]
    })
    .await;
    assert_eq!(env.herdr.detaches(), 0);
    assert_eq!(env.herdr.accepted.load(Ordering::SeqCst), 1);
}

/// A phone that does not ask for the cursor gets 0.1.0 frames.
#[tokio::test]
async fn frames_carry_no_cursor_unless_asked() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    let opened = phone.open(json!({"pane_id": "p1"})).await;
    assert_eq!(opened["source"], "attached");
    assert!(opened.get("cursor").is_none(), "{opened}");
    let first = phone.frame().await;
    assert_eq!(first.cursor, None);
    // Without hints: 80 by 24.
    assert_eq!((first.cols, first.rows), (80, 24));
}

/// A second session on the same connection retargets the one attachment.
#[tokio::test]
async fn a_second_session_retargets_the_same_attachment() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    phone.open(json!({"pane_id": "p1", "cursor": true})).await;
    let mut screen = Screen::default();
    phone
        .frame_where(&mut screen, |_, s| first_row(s) == "hi from p1")
        .await;
    let second = phone.open(json!({"pane_id": "p2", "cursor": true})).await;
    assert_eq!(second["terminal_id"], 2);
    assert_eq!(second["source"], "attached");
    assert_eq!(env.herdr.focuses(), ["p1", "p2"]);
    assert_eq!(env.herdr.accepted.load(Ordering::SeqCst), 1);
    let mut screen = Screen::default();
    loop {
        let f = phone.frame().await;
        if f.terminal_id == 2 {
            screen.apply(&f);
            if first_row(&screen) == "hi from p2" {
                break;
            }
        }
    }
    // Closing one keeps the surface active; the last makes it inactive.
    phone
        .call("terminal.close", json!({"terminal_id": 2}))
        .await;
    assert_eq!(env.herdr.actives(), [true]);
    phone
        .call("terminal.close", json!({"terminal_id": 1}))
        .await;
    until("the surface inactive", || {
        env.herdr.actives() == [true, false]
    })
    .await;
}

/// An incompatible herdr: the poll, `source: poll`, no mouse.
#[tokio::test]
async fn an_incompatible_herdr_falls_back_to_the_poll() {
    let env = start(Kind::Incompatible).await;
    until("a refused hello", || {
        env.herdr.accepted.load(Ordering::SeqCst) >= 1
    })
    .await;
    let mut phone = env.device("s23", true).await;
    assert_eq!(phone.hello["result"]["terminal"], json!({"input": true}));
    let opened = phone.open(json!({"pane_id": "p1", "cursor": true})).await;
    assert_eq!(opened["source"], "poll", "{opened}");
    assert!(opened.get("cursor").is_none(), "{opened}");
    let first = phone.frame().await;
    let mut screen = Screen::default();
    screen.apply(&first);
    assert_eq!(first_row(&screen), "hello");
    assert_eq!(first.cursor, None);

    // Input on a polled terminal: keys by send_keys, the scroll by
    // pane.scroll, the mouse refused, a resize ignored.
    phone.arm(1).await;
    let r = phone
        .call(
            "terminal.mouse",
            json!({"terminal_id": 1, "kind": "down", "button": "left", "col": 1, "row": 1}),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    assert!(r["error"]["message"].as_str().unwrap().contains("attached"));
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 3}))
        .await;
    assert_eq!(r["result"], json!({"offset_from_bottom": 3}), "{r}");
    let r = phone
        .call("terminal.input", json!({"terminal_id": 1, "keys": "l s"}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    assert_eq!(
        env.mock(|m| m.keystroke_log.clone()).await,
        [("p1".to_string(), "l s".to_string())]
    );
    let r = phone
        .call(
            "terminal.resize",
            json!({"terminal_id": 1, "cols": 50, "rows": 20}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    assert!(env.herdr.log().is_empty());
}

/// `tab_follows_focus` off and the pane on another tab: polled, and no
/// `pane.focus`; on herdr's active tab it is attached.
#[tokio::test]
async fn without_tab_follows_focus_a_pane_on_another_tab_is_polled() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    let opened = phone.open(json!({"pane_id": "p3", "cursor": true})).await;
    assert_eq!(opened["source"], "poll", "{opened}");
    let mut screen = Screen::default();
    screen.apply(&phone.frame().await);
    assert_eq!(first_row(&screen), "three");
    assert!(env.herdr.focuses().is_empty());
    assert!(env.herdr.actives().is_empty());
    assert_eq!(env.daemon.herdr_attach.as_ref().unwrap().viewers(), 0);

    // With tab_follows_focus on, the same pane is attached.
    let r = phone
        .call("client.options", json!({"tab_follows_focus": true}))
        .await;
    assert!(r["result"].is_object(), "{r}");
    let again = phone.open(json!({"pane_id": "p3", "cursor": true})).await;
    assert_eq!(again["source"], "attached", "{again}");
    assert_eq!(env.herdr.focuses(), ["p3"]);
}

/// The desktop takes the tab: frames are stale, flankr sends no focus or
/// resize of its own (a phone resize waits), and a touch takes it back
/// (focus, then the waiting resize, then the input).
#[tokio::test]
async fn a_desktop_change_is_not_forced_back_until_a_touch() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    phone
        .open(json!({"pane_id": "p1", "cols_hint": 60, "rows_hint": 20, "cursor": true}))
        .await;
    let mut screen = Screen::default();
    phone
        .frame_where(&mut screen, |_, s| first_row(s) == "hi from p1")
        .await;
    phone.arm(1).await;
    let attach = env.daemon.herdr_attach.clone().unwrap();
    until("settled", || attach.settled()).await;

    env.herdr.desktop_takes_the_tab();
    phone
        .frame_where(&mut screen, |_, s| s.cursor().is_some_and(|c| c.stale))
        .await;
    let (focuses, resizes) = (env.herdr.focuses(), env.herdr.resizes());
    // Redraws stay stale and send nothing.
    env.herdr.redraw();
    let r = phone
        .call(
            "terminal.resize",
            json!({"terminal_id": 1, "cols": 50, "rows": 18}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(env.herdr.focuses(), focuses);
    assert_eq!(env.herdr.resizes(), resizes, "a resize while stale waits");
    assert!(attach.stale());

    // A key: focus and the waited resize, then the input.
    let r = phone
        .call("terminal.input", json!({"terminal_id": 1, "keys": "x"}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    until("the resize", || {
        env.herdr.resizes().last() == Some(&(50, 18))
    })
    .await;
    assert_eq!(env.herdr.focuses().len(), focuses.len() + 1);
    let log = env.herdr.log();
    let focus_at = log
        .iter()
        .rposition(|(_, m)| matches!(m, ClientMessage::ClientShellEndpointRequest { .. }))
        .unwrap();
    let input_at = log
        .iter()
        .position(|(_, m)| matches!(m, ClientMessage::ClientShellPaneInput { .. }))
        .unwrap();
    assert!(focus_at < input_at, "the focus goes before the input");
    assert!(!attach.stale());
    phone
        .frame_where(&mut screen, |f, s| {
            f.cursor.is_some_and(|c| !c.stale) && s.cols() == 50
        })
        .await;
}

/// The tap that arms typing is a touch too: it takes the tab back.
#[tokio::test]
async fn arming_a_stale_terminal_takes_the_tab_back() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    phone
        .open(json!({"pane_id": "p1", "cols_hint": 60, "rows_hint": 20, "cursor": true}))
        .await;
    let attach = env.daemon.herdr_attach.clone().unwrap();
    until("settled", || attach.settled()).await;
    let mut screen = Screen::default();
    env.herdr.desktop_takes_the_tab();
    phone
        .frame_where(&mut screen, |_, s| s.cursor().is_some_and(|c| c.stale))
        .await;
    let focuses = env.herdr.focuses().len();
    phone.arm(1).await;
    assert!(!attach.stale());
    until("the focus again", || {
        env.herdr.focuses().len() == focuses + 1
    })
    .await;
    assert!(env.herdr.inputs().is_empty(), "arming sends no input");
    phone
        .frame_where(&mut screen, |f, _| f.cursor.is_some_and(|c| !c.stale))
        .await;
}

/// herdr goes away and comes back: the terminal falls back to the poll
/// without closing, the daemon reconnects with backoff, focuses again, and
/// the frames come from the attachment again.
#[tokio::test]
async fn the_attachment_reconnects_with_backoff_and_the_terminal_stays_open() {
    let env = start_with(Kind::Full, config(), Duration::from_millis(500)).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    phone.open(json!({"pane_id": "p1", "cursor": true})).await;
    let mut screen = Screen::default();
    phone
        .frame_where(&mut screen, |_, s| first_row(s) == "hi from p1")
        .await;

    env.herdr.drop_connection();
    // The poll: the mock's text, with no cursor.
    phone
        .frame_where(&mut screen, |_, s| first_row(s) == "hello")
        .await;
    until("a reconnect", || {
        env.herdr.accepted.load(Ordering::SeqCst) == 2
    })
    .await;
    env.attached().await;
    until("the focus again", || {
        env.herdr
            .log()
            .iter()
            .any(|(n, m)| *n == 2 && matches!(m, ClientMessage::ClientShellEndpointRequest { .. }))
    })
    .await;
    // The reconnect is active at once: a terminal is open.
    assert_eq!(env.herdr.with(|s| s.hellos.clone()), [false, true]);
    phone
        .frame_where(&mut screen, |f, s| {
            first_row(s) == "hi from p1" && f.cursor.is_some()
        })
        .await;
    assert!(phone.notes.iter().all(|n| n["method"] != "terminal.closed"));
}

/// A herdr without `client_shell.surface.set`: detached while no terminal
/// is open, attached on open, detached on the last close.
#[tokio::test]
async fn without_surface_set_the_last_close_detaches() {
    let env = start(Kind::NoSurfaceSet).await;
    // At start, with no terminal open, it detaches at once.
    until("the first detach", || env.herdr.detaches() == 1).await;
    let attach = env.daemon.herdr_attach.clone().unwrap();
    assert!(!attach.usable());
    let mut phone = env.device("s23", true).await;
    let opened = phone.open(json!({"pane_id": "p1", "cursor": true})).await;
    // Not attached at the answer; the reconnect follows the open.
    assert_eq!(opened["source"], "poll");
    until("a reconnect", || {
        env.herdr.accepted.load(Ordering::SeqCst) == 2
    })
    .await;
    env.attached().await;
    let mut screen = Screen::default();
    phone
        .frame_where(&mut screen, |_, s| first_row(s) == "hi from p1")
        .await;
    assert!(env.herdr.actives().is_empty());
    phone
        .call("terminal.close", json!({"terminal_id": 1}))
        .await;
    until("the detach", || env.herdr.detaches() == 2).await;
    until("not usable", || !attach.usable()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.herdr.accepted.load(Ordering::SeqCst), 2, "no reconnect");
}

/// `[terminal] attach = false`: no attachment at all.
#[tokio::test]
async fn attach_off_keeps_the_poll() {
    let mut cfg = config();
    cfg.terminal.attach = false;
    let env = start_with(Kind::Full, cfg, Duration::from_millis(50)).await;
    assert!(env.daemon.herdr_attach.is_none());
    let mut phone = env.device("s23", true).await;
    let opened = phone.open(json!({"pane_id": "p1", "cursor": true})).await;
    assert_eq!(opened["source"], "poll");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(env.herdr.accepted.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------
// Step 8: the input side.

/// Keys go as herdr's key events, after parse_keys' refusals, which still
/// refuse; the notice counts them.
#[tokio::test]
async fn attached_keys_reach_herdr_as_key_events() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    phone.open(json!({"pane_id": "p1", "cursor": true})).await;
    let r = phone
        .call("terminal.input", json!({"terminal_id": 1, "keys": "l s"}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "not armed: {r}");
    phone.arm(1).await;
    let r = phone
        .call(
            "terminal.input",
            json!({"terminal_id": 1, "keys": "l s enter"}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    until("the keys", || env.herdr.inputs().len() == 1).await;
    let inputs = env.herdr.inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].0, "p1");
    let events = &inputs[0].1;
    // Every key a press and a release, typed characters included (the
    // owner: "ime keyboard input should be translated to keys").
    let keys: Vec<(ClientKeyCode, ClientKeyKind, u8)> = events
        .iter()
        .map(|e| match e {
            ClientPaneInputEvent::Key {
                code,
                kind,
                modifiers,
                ..
            } => (code.clone(), *kind, *modifiers),
            other => panic!("not a key: {other:?}"),
        })
        .collect();
    let mut want = Vec::new();
    for code in [
        ClientKeyCode::Char('l'),
        ClientKeyCode::Char('s'),
        ClientKeyCode::Enter,
    ] {
        want.push((code.clone(), ClientKeyKind::Press, 0));
        want.push((code, ClientKeyKind::Release, 0));
    }
    assert_eq!(keys, want);
    assert!(env.mock(|m| m.keystroke_log.is_empty()).await);
    let notice = phone.note("notice").await;
    assert!(
        notice["params"]["text"]
            .as_str()
            .unwrap()
            .contains("3 keys")
    );

    // The refusals.
    for (keys, want) in [
        ("ctrl+c", codes::NOT_PERMITTED),
        ("\u{1b}[A", codes::INVALID_PARAMS),
        (&*"a ".repeat(40), codes::INVALID_PARAMS),
        ("alt+x", codes::INVALID_PARAMS),
    ] {
        let r = phone
            .call("terminal.input", json!({"terminal_id": 1, "keys": keys}))
            .await;
        assert_eq!(code(&r), want, "{keys:?}: {r}");
    }
    assert_eq!(env.herdr.inputs().len(), 1);
}

/// An armed `terminal.mouse` reaches herdr with pane-local cells, once
/// noticed per arm window; unarmed it is refused.
#[tokio::test]
async fn armed_mouse_reaches_herdr_with_pane_local_cells() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    phone.open(json!({"pane_id": "p1", "cursor": true})).await;
    let down = json!({"terminal_id": 1, "kind": "down", "button": "left", "col": 4, "row": 2});
    let r = phone.call("terminal.mouse", down.clone()).await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    assert!(env.herdr.inputs().is_empty());

    phone.arm(1).await;
    let r = phone.call("terminal.mouse", down.clone()).await;
    assert_eq!(r["result"], json!({}), "{r}");
    let r = phone
        .call(
            "terminal.mouse",
            json!({"terminal_id": 1, "kind": "up", "button": "left", "col": 4, "row": 2, "modifiers": 2}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    // A kind herdr has no event for is dropped.
    let r = phone
        .call(
            "terminal.mouse",
            json!({"terminal_id": 1, "kind": "pinch", "col": 0, "row": 0}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    until("the mouse", || env.herdr.inputs().len() == 2).await;
    let inputs = env.herdr.inputs();
    assert_eq!(inputs.len(), 2, "{inputs:?}");
    match &inputs[0].1[..] {
        [
            ClientPaneInputEvent::Mouse {
                kind: ClientMouseKind::Down(ClientMouseButton::Left),
                position: ClientMousePosition::Cell { column: 4, row: 2 },
                modifiers: 0,
                ..
            },
        ] => {}
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        &inputs[1].1[..],
        [ClientPaneInputEvent::Mouse {
            kind: ClientMouseKind::Up(ClientMouseButton::Left),
            modifiers: 2,
            ..
        }]
    ));
    // One notice for the arm window.
    phone.call("ping", json!({})).await;
    let notices: Vec<&Value> = phone
        .notes
        .iter()
        .filter(|n| n["method"] == "notice")
        .collect();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0]["params"]["kind"], "terminal_input");
    // A new arm: a new notice.
    phone.arm(1).await;
    phone.call("terminal.mouse", down).await;
    phone.call("ping", json!({})).await;
    assert_eq!(
        phone
            .notes
            .iter()
            .filter(|n| n["method"] == "notice")
            .count(),
        2
    );
}

/// An attached `terminal.scroll` is a wheel event, its offsets from the
/// next surface; `pane.scroll` is not used.
#[tokio::test]
async fn attached_scroll_is_a_wheel_event() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    phone.open(json!({"pane_id": "p1", "cursor": true})).await;
    phone.arm(1).await;
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 3}))
        .await;
    assert_eq!(
        r["result"],
        json!({"offset_from_bottom": 3, "max_offset_from_bottom": 500}),
        "{r}"
    );
    let inputs = env.herdr.inputs();
    assert!(matches!(
        &inputs.last().unwrap().1[..],
        [ClientPaneInputEvent::Mouse {
            kind: ClientMouseKind::ScrollUp,
            lines: 3,
            ..
        }]
    ));
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": -2}))
        .await;
    assert_eq!(r["result"]["offset_from_bottom"], 1, "{r}");
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 0}))
        .await;
    assert_eq!(r["result"]["offset_from_bottom"], 1, "{r}");
    // Each switch still gates it: a device that may not approve.
    let mut viewer = env.device("viewer", false).await;
    viewer.open(json!({"pane_id": "p1"})).await;
    let r = viewer
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 3}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
}

/// `terminal.resize` on an attached terminal is a `ClientShellResize`, at
/// most twice a second, coalesced to the latest size.
#[tokio::test]
async fn resizes_are_sent_at_most_twice_a_second() {
    let env = start(Kind::Full).await;
    env.attached().await;
    let mut phone = env.device("s23", true).await;
    phone
        .open(json!({"pane_id": "p1", "cols_hint": 60, "rows_hint": 20, "cursor": true}))
        .await;
    for (cols, rows) in [(61, 20), (62, 20), (63, 21)] {
        let r = phone
            .call(
                "terminal.resize",
                json!({"terminal_id": 1, "cols": cols, "rows": rows}),
            )
            .await;
        assert_eq!(r["result"], json!({}), "{r}");
    }
    // The open's resize went now; the rest wait and go as the latest.
    assert_eq!(env.herdr.resizes(), [(60, 20)]);
    until("the coalesced resize", || {
        env.herdr.resizes() == [(60, 20), (63, 21)]
    })
    .await;
    let mut screen = Screen::default();
    phone.frame_where(&mut screen, |_, s| s.cols() == 63).await;
    // A size past the clamp is clamped.
    tokio::time::sleep(Duration::from_millis(550)).await;
    phone
        .call(
            "terminal.resize",
            json!({"terminal_id": 1, "cols": 5, "rows": 1000}),
        )
        .await;
    until("the clamped resize", || {
        env.herdr.resizes().last() == Some(&(20, 200))
    })
    .await;
    // Not a terminal of this connection.
    let r = phone
        .call(
            "terminal.resize",
            json!({"terminal_id": 9, "cols": 50, "rows": 20}),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
}
