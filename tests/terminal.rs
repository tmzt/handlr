//! The terminal mode, the daemon's half, end to end over the link
//! (docs/design/20261027_07_TERMINAL_MODE_PLAN.md, steps 3 and 6): a
//! companion over TCP on 127.0.0.1 with an ephemeral port, its key written
//! to a temporary `auth.toml`, over the herdr mock. Nothing touches
//! `~/.config/flankr` or a live herdr; no key reaches a real pane.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use flankr::config::{AgentConfig, AppConfig, TerminalConfig};
use flankr::server::companion::TurnOptions;
use flankr::server::link::auth::{self, AuthEntry};
use flankr::server::link::{Link, LinkSettings};
use flankr::server::{self, CoreEvent, Daemon, ServeOptions};
use flankr::sessions::Sessions;
use flankr::transport::HerdrClient;
use futures_util::StreamExt;
use libflankr_proto::envelope::codes;
use libflankr_proto::link::{LinkStream, Message, StaticKeypair, generate_keypair, reconnect};
use libflankr_term::{Frame, Screen, parse_ansi};
use libherdr::transport::{HerdrTransport, MockState};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpStream, UnixStream};
use tokio_util::codec::{FramedRead, LinesCodec};

const WAIT: Duration = Duration::from_secs(5);

/// The mock pane's screen: a plain row and a styled one.
const BUFFER: [&str; 2] = ["hello", "\u{1b}[1mbold\u{1b}[0m world"];

struct Env {
    dir: tempfile::TempDir,
    socket: PathBuf,
    address: String,
    link: Arc<Link>,
    daemon: Arc<Daemon>,
    mock: HerdrClient,
}

/// claude with terminal input as given, and `[terminal] input` as given.
fn config(input: bool, agent_input: bool, signals: bool) -> AppConfig {
    AppConfig {
        agents: vec![
            AgentConfig::new("claude", vec!["continue".to_string()], true)
                .with_terminal_input(agent_input, signals),
            AgentConfig::new("omp", vec!["continue".to_string()], true),
        ],
        summary: Some(flankr::config::SummaryConfig { cards: false }),
        terminal: TerminalConfig {
            input,
            ..TerminalConfig::default()
        },
        ..AppConfig::default()
    }
}

fn mock_state() -> MockState {
    let mut state = MockState::default();
    state.pane_buffers.insert(
        "p1".to_string(),
        BUFFER.iter().map(|s| s.to_string()).collect(),
    );
    state
}

async fn start(config: AppConfig) -> Env {
    let dir = tempfile::Builder::new()
        .prefix("ht")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("h.sock");
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
            ..TurnOptions::default()
        },
    );
    let unix = server::bind(&socket).await.unwrap();
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
        socket,
        address,
        link,
        daemon,
        mock,
    }
}

impl Env {
    async fn mock<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        let HerdrTransport::Mock { state } = self.mock.transport() else {
            unreachable!()
        };
        f(&mut *state.write().await)
    }

    /// A companion device in `auth.toml`, reaching `agents` (all when
    /// `None`), approving as given; connected, after its `hello`.
    async fn device(&self, name: &str, agents: Option<&[&str]>, approve: bool) -> Client {
        let keypair = generate_keypair().unwrap();
        let mut entry = AuthEntry::new(name, &keypair.public, vec!["companion".to_string()]);
        entry.agents = agents.map(|a| a.iter().map(|s| s.to_string()).collect());
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
        assert_eq!(
            client.hello["result"]["server"], "flankr",
            "{}",
            client.hello
        );
        client
    }

    fn output(&self, pane_id: &str) {
        let _ = self.daemon.events.send(CoreEvent::Output {
            pane_id: pane_id.to_string(),
        });
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
    /// One message: notes and frames are kept, a response is returned.
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

    async fn note(&mut self, method: &str) -> Value {
        loop {
            if let Some(i) = self.notes.iter().position(|n| n["method"] == method) {
                return self.notes.remove(i);
            }
            self.pump().await;
        }
    }

    /// `terminal.open` on `pane`, its result.
    async fn open(&mut self, pane: &str) -> Value {
        let r = self.call("terminal.open", json!({"pane_id": pane})).await;
        assert!(r["result"]["terminal_id"].is_u64(), "{r}");
        r["result"].clone()
    }
}

fn code(r: &Value) -> i64 {
    r["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("not an error: {r}"))
}

/// Plan 20261027_07, step 3: an open sends a full frame that reassembles
/// to the pane; a change is a diff from the acknowledged frame; a second
/// open is the same terminal with a full frame.
#[tokio::test]
async fn an_open_terminal_mirrors_the_pane_in_frames() {
    let env = start(config(false, false, false)).await;
    let mut phone = env.device("s23", None, true).await;
    assert_eq!(phone.hello["result"]["terminal"], json!({"input": false}));

    let opened = phone.open("p1").await;
    assert_eq!(opened["terminal_id"], 1);
    let expected = parse_ansi(&BUFFER.join("\n"));
    assert_eq!(opened["cols"], expected.cols);
    assert_eq!(opened["rows"], 2);
    // The read is the visible frame.
    let reads = env.mock(|m| m.read_log.clone()).await;
    assert_eq!(
        reads.last().map(|r| (r.0.as_str(), r.1)),
        Some(("p1", libherdr::protocol::ReadSource::Visible))
    );

    let first = phone.frame().await;
    assert_eq!((first.terminal_id, first.base_seq), (1, 0));
    let mut screen = Screen::default();
    assert!(screen.apply(&first));
    assert_eq!(screen.grid(), &expected);
    assert_eq!(screen.grid().text_lines(), ["hello", "bold world"]);

    phone
        .notify("terminal.ack", json!({"terminal_id": 1, "seq": first.seq}))
        .await;
    // The ack is read before this answer.
    assert!(phone.call("ping", json!({})).await["result"].is_object());

    // A row changes within the frame's size (a new size is a full frame),
    // and herdr says the pane scrolled.
    env.mock(|m| {
        m.pane_buffers.get_mut("p1").unwrap()[0] = "howdy".to_string();
    })
    .await;
    env.output("p1");
    let diff = phone.frame().await;
    assert_eq!(diff.base_seq, first.seq, "{diff:?}");
    assert!(diff.seq > first.seq);
    assert_eq!(diff.diffs.len(), 1);
    assert_eq!(diff.diffs[0].row, 0);
    assert!(screen.apply(&diff));
    assert_eq!(screen.grid().text_lines(), ["howdy", "bold world"]);

    // Opened again: the same id, and a full frame the screen takes.
    let again = phone.open("p1").await;
    assert_eq!(again["terminal_id"], 1);
    let full = phone.frame().await;
    assert_eq!(full.base_seq, 0);
    assert!(full.seq > diff.seq);
    assert!(screen.apply(&full));
    assert_eq!(screen.grid().text_lines(), ["howdy", "bold world"]);

    // Closed: `closed`, and the panes no longer subscribed.
    let r = phone
        .call("terminal.close", json!({"terminal_id": 1}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let closed = phone.note("terminal.closed").await;
    assert_eq!(
        closed["params"],
        json!({"terminal_id": 1, "reason": "closed"})
    );
    assert!(env.daemon.terminal_panes.panes().is_empty());
}

#[tokio::test]
async fn unknown_panes_and_agents_out_of_reach_are_refused() {
    let env = start(config(false, false, false)).await;
    let mut phone = env.device("s23", None, true).await;
    let r = phone
        .call("terminal.open", json!({"pane_id": "p404"}))
        .await;
    assert_eq!(code(&r), codes::UNKNOWN_AGENT, "{r}");

    let mut narrow = env.device("tablet", Some(&["omp"]), true).await;
    let r = narrow.call("terminal.open", json!({"pane_id": "p1"})).await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");

    // A pane whose agent has no [[agents]] entry.
    env.mock(|m| {
        m.snapshot.workspaces[0].tabs[0].panes[0]
            .extra
            .insert("agent".to_string(), json!("codex"));
    })
    .await;
    let r = phone.call("terminal.open", json!({"pane_id": "p1"})).await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    assert!(phone.frames.is_empty());
}

/// A companion on the daemon socket has no terminal: frames need the link.
#[tokio::test]
async fn the_daemon_socket_has_no_terminal() {
    let env = start(config(true, true, false)).await;
    let (read, mut write) = UnixStream::connect(&env.socket).await.unwrap().into_split();
    let mut lines = FramedRead::new(read, LinesCodec::new());
    let mut ask = async |id: i64, method: &str, params: Value| -> Value {
        let mut line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        write.write_all(line.as_bytes()).await.unwrap();
        loop {
            let line = tokio::time::timeout(WAIT, lines.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == id {
                return v;
            }
        }
    };
    let hello = ask(
        1,
        "hello",
        json!({"protocol": "flankr.companion", "version": 1, "client": "local"}),
    )
    .await;
    assert!(hello["result"].get("terminal").is_none(), "{hello}");
    let r = ask(2, "terminal.open", json!({"pane_id": "p1"})).await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
}

#[tokio::test]
async fn the_pane_closing_or_its_agent_changing_ends_the_terminal() {
    let env = start(config(false, false, false)).await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    phone.frame().await;
    assert_eq!(env.daemon.terminal_panes.panes(), ["p1"]);
    let _ = env.daemon.events.send(CoreEvent::Pane {
        pane_id: "p1".to_string(),
        closed: true,
    });
    let closed = phone.note("terminal.closed").await;
    assert_eq!(
        closed["params"],
        json!({"terminal_id": 1, "reason": "pane_closed"})
    );
    assert!(env.daemon.terminal_panes.panes().is_empty());

    // Ids go on counting on the connection.
    assert_eq!(phone.open("p1").await["terminal_id"], 2);
    phone.frame().await;
    env.mock(|m| {
        m.snapshot.workspaces[0].tabs[0].panes[0]
            .extra
            .insert("agent".to_string(), json!("omp"));
    })
    .await;
    let _ = env.daemon.events.send(CoreEvent::Pane {
        pane_id: "p1".to_string(),
        closed: false,
    });
    let closed = phone.note("terminal.closed").await;
    assert_eq!(
        closed["params"],
        json!({"terminal_id": 2, "reason": "agent_changed"})
    );
}

/// Plan 20261027_07, step 6: an armed terminal takes keys, which reach
/// herdr's send_keys as sent; the notice counts them and does not say them.
#[tokio::test]
async fn armed_input_reaches_send_keys_and_is_noticed_without_the_keys() {
    let env = start(config(true, true, false)).await;
    let mut phone = env.device("s23", None, true).await;
    assert_eq!(phone.hello["result"]["terminal"], json!({"input": true}));
    phone.open("p1").await;

    // Not armed.
    let r = phone
        .call(
            "terminal.input",
            json!({"terminal_id": 1, "keys": "l s enter"}),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    // Not a terminal of this connection.
    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": 9}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");

    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    assert_eq!(r["result"], json!({"expires_in_s": 0}), "{r}");
    let r = phone
        .call(
            "terminal.input",
            json!({"terminal_id": 1, "keys": "l s enter"}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let keys = env.mock(|m| m.keystroke_log.clone()).await;
    assert_eq!(keys, [("p1".to_string(), "l s enter".to_string())]);

    let notice = phone.note("notice").await;
    assert_eq!(notice["params"]["kind"], "terminal_input", "{notice}");
    assert_eq!(notice["params"]["pane_id"], "p1");
    let text = notice["params"]["text"].as_str().unwrap();
    assert!(text.contains("Typing into"), "{text}");
    assert!(!notice.to_string().contains("enter"), "{notice}");

    // Over 64 bytes, and a signal key without terminal_signals.
    let long = "a ".repeat(40);
    let r = phone
        .call("terminal.input", json!({"terminal_id": 1, "keys": long}))
        .await;
    assert_eq!(code(&r), codes::INVALID_PARAMS, "{r}");
    let r = phone
        .call(
            "terminal.input",
            json!({"terminal_id": 1, "keys": "ctrl+c"}),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    let r = phone
        .call(
            "terminal.input",
            json!({"terminal_id": 1, "keys": "\u{1b}[A"}),
        )
        .await;
    assert_eq!(code(&r), codes::INVALID_PARAMS, "{r}");
    assert_eq!(env.mock(|m| m.keystroke_log.len()).await, 1);
}

/// The L3 amendment of 2026-10-28 10:42 EDT: ctrl/alt combos reach herdr's
/// send_keys in its own spelling (`ctrl+alt+<key>`), with no arm timer.
#[tokio::test]
async fn polled_ctrl_and_alt_combos_reach_send_keys() {
    let env = start(config(true, true, false)).await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    assert_eq!(r["result"], json!({"expires_in_s": 0}), "{r}");
    let r = phone
        .call(
            "terminal.input",
            json!({"terminal_id": 1, "keys": "alt+X alt+ctrl+b ctrl+left alt+enter alt++"}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let keys = env.mock(|m| m.keystroke_log.clone()).await;
    assert_eq!(
        keys,
        [(
            "p1".to_string(),
            "alt+X ctrl+alt+b ctrl+left alt+enter alt++".to_string()
        )]
    );
    for (keys, want) in [
        ("ctrl+alt+d", codes::NOT_PERMITTED),
        ("alt+space", codes::INVALID_PARAMS),
        ("ctrl+f1", codes::INVALID_PARAMS),
        ("shift+left", codes::INVALID_PARAMS),
    ] {
        let r = phone
            .call("terminal.input", json!({"terminal_id": 1, "keys": keys}))
            .await;
        assert_eq!(code(&r), want, "{keys:?}: {r}");
    }
    assert_eq!(env.mock(|m| m.keystroke_log.len()).await, 1);
}

#[tokio::test]
async fn signal_keys_pass_with_terminal_signals() {
    let env = start(config(true, true, true)).await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    let r = phone
        .call(
            "terminal.input",
            json!({"terminal_id": 1, "keys": "ctrl+c"}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let keys = env.mock(|m| m.keystroke_log.clone()).await;
    assert_eq!(keys, [("p1".to_string(), "ctrl+c".to_string())]);
}

/// Each switch of decision 6 on its own refuses the arm.
#[tokio::test]
async fn input_needs_every_switch() {
    // [terminal] input off.
    let env = start(config(false, true, false)).await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");

    // The agent's terminal_input off.
    let env = start(config(true, false, false)).await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");

    // A device that may not approve.
    let env = start(config(true, true, false)).await;
    let mut phone = env.device("viewer", None, false).await;
    phone.open("p1").await;
    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    let r = phone
        .call("terminal.input", json!({"terminal_id": 1, "keys": "l"}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    assert!(env.mock(|m| m.keystroke_log.is_empty()).await);
}

/// The idle tick resends the newest frame while it is not acknowledged.
#[tokio::test]
async fn an_unacknowledged_frame_is_resent_on_the_idle_tick() {
    let env = start(config(false, false, false)).await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    let first = phone.frame().await;
    let again = phone.frame().await;
    assert!(again.seq > first.seq);
    assert_eq!(again.base_seq, 0);
    let (mut a, mut b) = (Screen::default(), Screen::default());
    a.apply(&first);
    b.apply(&again);
    assert_eq!(a.grid(), b.grid());
}

/// Only the daemon sends terminal frames: one from the phone closes the
/// link.
#[tokio::test]
async fn a_terminal_frame_from_the_phone_closes_the_link() {
    let env = start(config(false, false, false)).await;
    let mut phone = env.device("s23", None, true).await;
    phone.stream.send_terminal(&[0u8; 18]).await.unwrap();
    let r = loop {
        if let Some(r) = phone.pump().await {
            break r;
        }
    };
    assert_eq!(code(&r), codes::INVALID_REQUEST, "{r}");
    let end = tokio::time::timeout(WAIT, phone.stream.recv())
        .await
        .unwrap();
    assert!(end.is_err(), "{end:?}");
}

/// The 2026-10-27 21:58 EDT amendment of plan 20261022_03: a drag scrolls
/// the pane, gated as typing is. The mock answers `pane.scroll` with an
/// echo, so the offset is the daemon's clamped one.
#[tokio::test]
async fn an_armed_scroll_moves_the_pane_and_clamps_at_the_bottom() {
    let env = start(config(true, true, false)).await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    phone.frame().await;

    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 3}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");

    phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    let reads = env.mock(|m| m.read_log.len()).await;
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 3}))
        .await;
    assert_eq!(r["result"], json!({"offset_from_bottom": 3}), "{r}");
    // The poller reads the pane again at once, not on the 2 s tick.
    tokio::time::timeout(Duration::from_secs(1), async {
        while env.mock(|m| m.read_log.len()).await == reads {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a read after the scroll");

    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": -10}))
        .await;
    assert_eq!(r["result"], json!({"offset_from_bottom": 0}), "{r}");
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 5000}))
        .await;
    assert_eq!(r["result"], json!({"offset_from_bottom": 1000}), "{r}");
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 0}))
        .await;
    assert_eq!(r["result"], json!({"offset_from_bottom": 1000}), "{r}");
    // No notice for a scroll.
    assert!(
        phone.notes.iter().all(|n| n["method"] != "notice"),
        "{:?}",
        phone.notes
    );
}

/// The pane's offset and max from herdr's snapshot are where a scroll
/// starts and stops.
#[tokio::test]
async fn a_scroll_starts_from_the_snapshots_offset_and_stops_at_its_max() {
    let env = start(config(true, true, false)).await;
    env.mock(|m| {
        m.snapshot.workspaces[0].tabs[0].panes[0].extra.insert(
            "scroll".to_string(),
            json!({"offset_from_bottom": 5, "max_offset_from_bottom": 8, "viewport_rows": 40}),
        );
    })
    .await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 0}))
        .await;
    assert_eq!(
        r["result"],
        json!({"offset_from_bottom": 5, "max_offset_from_bottom": 8}),
        "{r}"
    );
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 10}))
        .await;
    assert_eq!(
        r["result"],
        json!({"offset_from_bottom": 8, "max_offset_from_bottom": 8}),
        "{r}"
    );
}

/// Each switch that gates typing gates scrolling too.
#[tokio::test]
async fn scrolling_needs_every_switch() {
    let scroll = json!({"terminal_id": 1, "lines": 3});
    // [terminal] input off; the agent's terminal_input off.
    for (input, agent_input) in [(false, true), (true, false)] {
        let env = start(config(input, agent_input, false)).await;
        let mut phone = env.device("s23", None, true).await;
        phone.open("p1").await;
        let r = phone.call("terminal.scroll", scroll.clone()).await;
        assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    }
    // A device that may not approve.
    let env = start(config(true, true, false)).await;
    let mut phone = env.device("viewer", None, false).await;
    phone.open("p1").await;
    let r = phone.call("terminal.scroll", scroll.clone()).await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    // Not a terminal of this connection.
    let mut other = env.device("s23", None, true).await;
    let r = other
        .call("terminal.scroll", json!({"terminal_id": 9, "lines": 3}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
}

/// Makes the mock's pane `p1` a shell: no agent named, a title naming none.
async fn make_shell(env: &Env) {
    env.mock(|m| {
        let pane = &mut m.snapshot.workspaces[0].tabs[0].panes[0];
        pane.extra.remove("agent");
        pane.terminal_title = Some("htop".to_string());
        pane.agent_status = None;
    })
    .await;
}

/// The shipped agents, `[terminal] input` on.
fn shipped(shell_input: bool) -> AppConfig {
    let mut config = AppConfig {
        summary: Some(flankr::config::SummaryConfig { cards: false }),
        terminal: TerminalConfig {
            input: true,
            ..TerminalConfig::default()
        },
        ..AppConfig::default()
    };
    config
        .agents
        .iter_mut()
        .filter(|a| a.name == "shell")
        .for_each(|a| a.terminal_input = shell_input);
    config
}

/// Plan 20261022_03, the 2026-10-28 00:47 EDT amendment: a shell pane is
/// listed, its terminal opens with the shipped `shell` entry, read-only,
/// and nothing is sent to it as a command, a message or an interrupt.
#[tokio::test]
async fn a_shell_pane_is_listed_and_only_its_terminal_opens() {
    let env = start(shipped(false)).await;
    make_shell(&env).await;
    let mut phone = env.device("s23", None, true).await;

    let list = phone.call("agents.list", json!({})).await;
    let agents = list["result"]["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1, "{list}");
    assert_eq!(agents[0]["pane_id"], "p1");
    assert_eq!(agents[0]["agent"], "shell");
    assert!(agents[0]["name"].is_string(), "{list}");
    assert!(agents[0].get("session").is_none(), "{list}");
    assert!(agents[0].get("turn").is_none(), "{list}");

    // Its session: none, for want of a reader, without an error.
    let r = phone
        .call("session.subscribe", json!({"pane_id": "p1", "sections": 4}))
        .await;
    assert_eq!(r["result"]["delivery"]["reason"], "no_reader", "{r}");

    // The terminal opens and mirrors the pane.
    phone.open("p1").await;
    let first = phone.frame().await;
    let mut screen = Screen::default();
    assert!(screen.apply(&first));
    assert_eq!(screen.grid().text_lines(), ["hello", "bold world"]);
    // Read-only: the shell entry's terminal_input is off.
    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    let r = phone
        .call("terminal.scroll", json!({"terminal_id": 1, "lines": 3}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");

    // No command, tapped or not: an agent's allow-listed word, a command
    // line, a slash command.
    for command in ["continue", "ls", "/compact"] {
        for confirmed in [false, true] {
            let r = phone
                .call(
                    "agents.send",
                    json!({"pane_id": "p1", "command": command, "confirmed": confirmed}),
                )
                .await;
            assert_eq!(code(&r), codes::NOT_PERMITTED, "{command}: {r}");
        }
    }
    // No message.
    let r = phone
        .call(
            "agents.send",
            json!({"pane_id": "p1", "text": "list the files", "kind": "message", "confirmed": true}),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    // No interrupt key.
    let r = phone
        .call("agents.interrupt", json!({"target": "p1"}))
        .await;
    assert_eq!(r["result"]["interrupted"], false, "{r}");
    let (commands, keys) = env
        .mock(|m| (m.command_log.clone(), m.keystroke_log.clone()))
        .await;
    assert!(commands.is_empty(), "{commands:?}");
    assert!(keys.is_empty(), "{keys:?}");
}

/// With `terminal_input` set on the shell entry, an armed shell terminal
/// takes keys.
#[tokio::test]
async fn a_shell_types_only_with_its_entrys_terminal_input() {
    let env = start(shipped(true)).await;
    make_shell(&env).await;
    let mut phone = env.device("s23", None, true).await;
    phone.open("p1").await;
    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": 1}))
        .await;
    assert_eq!(r["result"], json!({"expires_in_s": 0}), "{r}");
    let r = phone
        .call("terminal.input", json!({"terminal_id": 1, "keys": "q"}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let keys = env.mock(|m| m.keystroke_log.clone()).await;
    assert_eq!(keys, [("p1".to_string(), "q".to_string())]);
}

/// Without a `shell` entry the pane is still listed, and its terminal is
/// refused with the daemon's message.
#[tokio::test]
async fn without_a_shell_entry_a_shell_is_listed_but_its_terminal_refused() {
    let env = start(config(false, false, false)).await;
    make_shell(&env).await;
    let mut phone = env.device("s23", None, true).await;
    let list = phone.call("agents.list", json!({})).await;
    assert_eq!(list["result"]["agents"][0]["agent"], "shell", "{list}");
    let r = phone.call("terminal.open", json!({"pane_id": "p1"})).await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    let message = r["error"]["message"].as_str().unwrap();
    assert!(message.contains("shell"), "{r}");
}
