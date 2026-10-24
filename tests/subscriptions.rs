//! Session subscriptions end to end (plan 20261023_03, sections 10.3 and
//! 10.4, and A8.7): over the daemon socket and over the Noise link, with
//! the herdr mock, a stand-in herdr event socket, and synthetic session
//! files under a temporary home. Nothing reaches a live herdr, and no
//! real session file is read.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use handlr::config::AppConfig;
use handlr::server::companion::TurnOptions;
use handlr::server::link::auth::{self, AuthEntry};
use handlr::server::link::{Link, LinkSettings};
use handlr::server::{self, Daemon, ServeOptions};
use handlr::sessions::Sessions;
use handlr::transport::{AgentSession, HerdrClient, HerdrTransport, PaneInfo};
use libhandlr_proto::link::{LinkStream, StaticKeypair, generate_keypair, reconnect};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpStream, UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio_util::codec::{FramedRead, LinesCodec};

const WAIT: Duration = Duration::from_secs(5);
const SESSION_A: &str = "00000000-0000-4000-8000-0000000000a1";
const SESSION_B: &str = "00000000-0000-4000-8000-0000000000b2";

fn ts(n: u32) -> String {
    format!(
        "2026-01-02T{:02}:{:02}:{:02}.000Z",
        n / 3600 % 24,
        n / 60 % 60,
        n % 60
    )
}

fn user(n: u32, text: &str) -> String {
    json!({"type": "user", "timestamp": ts(n), "isSidechain": false,
        "message": {"role": "user", "content": text}, "origin": {"kind": "human"}})
    .to_string()
}

fn reply(n: u32, text: &str) -> String {
    json!({"type": "assistant", "timestamp": ts(n), "isSidechain": false,
        "message": {"role": "assistant", "type": "message",
                    "content": [{"type": "text", "text": text}]}})
    .to_string()
}

fn blob() -> String {
    json!({"type": "api-request-blob", "hash": "h", "message": "b"}).to_string()
}

fn write_lines(path: &Path, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(path, text).unwrap();
}

fn append_lines(path: &Path, lines: &[String]) {
    use std::io::Write;
    let mut text = lines.join("\n");
    text.push('\n');
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

struct Env {
    dir: tempfile::TempDir,
    socket: PathBuf,
    daemon: Arc<Daemon>,
    /// Lines the stand-in herdr pushes down its event connections.
    herdr: broadcast::Sender<String>,
    link: Option<(String, Arc<Link>)>,
}

impl Env {
    fn session(&self, id: &str) -> PathBuf {
        self.dir
            .path()
            .join("home/.claude/projects/-work-example")
            .join(format!("{id}.jsonl"))
    }

    /// Push a herdr event, as herdr would.
    fn herdr_event(&self, event: Value) {
        let _ = self.herdr.send(event.to_string());
    }

    async fn pane(&self, edit: impl FnOnce(&mut Vec<PaneInfo>)) {
        match self.daemon.client.transport() {
            HerdrTransport::Mock { state } => {
                let mut state = state.write().await;
                edit(&mut state.snapshot.workspaces[0].tabs[0].panes);
            }
            _ => unreachable!("tests use the mock"),
        }
    }

    async fn set_focus(&self, pane: &str) {
        match self.daemon.client.transport() {
            HerdrTransport::Mock { state } => {
                let mut state = state.write().await;
                state.snapshot.active_pane_id = Some(pane.to_string());
                for p in &mut state.snapshot.workspaces[0].tabs[0].panes {
                    p.focused = p.pane_id == pane;
                }
            }
            _ => unreachable!("tests use the mock"),
        }
    }
}

fn session_of(id: &str) -> AgentSession {
    AgentSession {
        source: "herdr:claude".to_string(),
        agent: "claude".to_string(),
        kind: "id".to_string(),
        value: id.to_string(),
    }
}

/// A stand-in herdr event socket: each connection's `events.subscribe` is
/// acknowledged, then every line sent on `lines` is pushed down it.
async fn herdr_events(path: &Path, lines: broadcast::Sender<String>) {
    let listener = UnixListener::bind(path).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let mut rx = lines.subscribe();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut read = BufReader::new(read);
                let mut first = String::new();
                if read.read_line(&mut first).await.is_err() {
                    return;
                }
                let req: Value = serde_json::from_str(first.trim()).unwrap_or(Value::Null);
                let ack = json!({"id": req["id"], "result": {"type": "subscription_started"}});
                if write
                    .write_all(format!("{ack}\n").as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
                while let Ok(line) = rx.recv().await {
                    if write
                        .write_all(format!("{line}\n").as_bytes())
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
}

async fn start(with_link: bool) -> Env {
    let dir = tempfile::Builder::new()
        .prefix("hs")
        .tempdir_in("/tmp")
        .unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".claude/projects/-work-example")).unwrap();
    let socket = dir.path().join("h.sock");
    let (link, listeners) = if with_link {
        let link = Link::open(LinkSettings {
            key_path: dir.path().join("link_key"),
            auth_path: dir.path().join("auth.toml"),
            listen: Some(vec!["127.0.0.1:0".to_string()]),
            revoke_interval: Duration::from_secs(60),
        })
        .unwrap();
        let listeners = link.bind().await.unwrap();
        let address = listeners[0].local_addr().unwrap().to_string();
        (Some((address, link)), listeners)
    } else {
        (None, Vec::new())
    };
    let daemon = Daemon::with_turns(
        HerdrClient::mock(),
        &AppConfig::default(),
        link.as_ref().map(|(_, l)| Arc::clone(l)),
        TurnOptions {
            session_dir: Some(dir.path().join("history")),
            agent_sessions: Some(Arc::new(Sessions::with_home(&home))),
            ..TurnOptions::default()
        },
    );
    let (herdr, _) = broadcast::channel(64);
    let events = dir.path().join("herdr.sock");
    herdr_events(&events, herdr.clone()).await;
    let listener = server::bind(&socket).await.unwrap();
    if with_link {
        server::link::spawn(Arc::clone(&daemon), listeners);
    }
    tokio::spawn(server::run(
        Arc::clone(&daemon),
        listener,
        ServeOptions {
            show_tokens: false,
            event_transport: Some(HerdrTransport::local(&events)),
        },
    ));
    let env = Env {
        dir,
        socket,
        daemon,
        herdr,
        link,
    };
    // The mock's p1: Claude Code in /work/example, on session A.
    env.pane(|panes| {
        panes[0].cwd = Some("/work/example".to_string());
        panes[0].agent_session = Some(session_of(SESSION_A));
    })
    .await;
    let mut lines = Vec::new();
    for i in 0..3 {
        lines.push(user(10 * i, &format!("prompt {i}")));
        lines.push(reply(10 * i + 1, &format!("Reply {i}.")));
    }
    write_lines(&env.session(SESSION_A), &lines);
    // Let the watcher subscribe.
    tokio::time::sleep(Duration::from_millis(200)).await;
    env
}

enum Wire {
    Unix {
        write: OwnedWriteHalf,
        lines: FramedRead<OwnedReadHalf, LinesCodec>,
    },
    Link(Box<LinkStream<TcpStream>>),
}

struct Client {
    wire: Wire,
    next_id: i64,
    seen: Vec<Value>,
    /// The bytes of every message received.
    bytes: usize,
}

fn hello(protocol: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 0, "method": "hello",
           "params": {"protocol": protocol, "version": 1, "client": "t"}})
}

impl Client {
    async fn unix(socket: &Path, protocol: &str) -> Client {
        let (read, write) = UnixStream::connect(socket).await.unwrap().into_split();
        let mut c = Client {
            wire: Wire::Unix {
                write,
                lines: FramedRead::new(read, LinesCodec::new_with_max_length(1 << 20)),
            },
            next_id: 0,
            seen: Vec::new(),
            bytes: 0,
        };
        let r = c.call("hello", hello(protocol)["params"].clone()).await;
        assert_eq!(r["result"]["server"], "handlr", "{r}");
        c
    }

    /// A paired companion over the link, with `agents` narrowing its reach.
    async fn link(env: &Env, agents: Option<&[&str]>) -> Client {
        let (address, link) = env.link.as_ref().expect("the link is on");
        let keypair = generate_keypair().unwrap();
        let name = format!(
            "phone{}",
            agents.map_or(0, |a| a.len()) + agents.map_or(0, |a| a[0].len())
        );
        let mut entry = AuthEntry::new(&name, &keypair.public, vec!["companion".to_string()]);
        entry.agents = agents.map(|a| a.iter().map(|s| s.to_string()).collect());
        auth::add(&env.dir.path().join("auth.toml"), entry, true).unwrap();
        Self::link_with(address, link, &keypair).await
    }

    async fn link_with(address: &str, link: &Link, keypair: &StaticKeypair) -> Client {
        let tcp = TcpStream::connect(address).await.unwrap();
        let first = serde_json::to_vec(&hello("handlr.companion")).unwrap();
        let stream = reconnect(tcp, keypair, &link.public_key(), Some(&first))
            .await
            .unwrap();
        let mut c = Client {
            wire: Wire::Link(Box::new(stream)),
            next_id: 0,
            seen: Vec::new(),
            bytes: 0,
        };
        let r = c.recv(WAIT).await.expect("hello answered");
        assert_eq!(r["result"]["server"], "handlr", "{r}");
        c
    }

    async fn send(&mut self, value: Value) {
        match &mut self.wire {
            Wire::Unix { write, .. } => {
                let mut line = value.to_string();
                line.push('\n');
                write.write_all(line.as_bytes()).await.unwrap();
            }
            Wire::Link(stream) => stream
                .send_message(&serde_json::to_vec(&value).unwrap())
                .await
                .unwrap(),
        }
    }

    async fn recv(&mut self, within: Duration) -> Option<Value> {
        let (value, len): (Value, usize) = match &mut self.wire {
            Wire::Unix { lines, .. } => {
                let line = tokio::time::timeout(within, lines.next())
                    .await
                    .ok()??
                    .unwrap();
                (serde_json::from_str(&line).unwrap(), line.len())
            }
            Wire::Link(stream) => {
                let message = tokio::time::timeout(within, stream.recv_message())
                    .await
                    .ok()?
                    .ok()?;
                (serde_json::from_slice(&message).unwrap(), message.len())
            }
        };
        self.bytes += len;
        self.seen.push(value.clone());
        Some(value)
    }

    async fn request(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        id
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.request(method, params).await;
        loop {
            let msg = self.recv(WAIT).await.expect("no response in time");
            if msg["id"] == id && msg.get("method").is_none() {
                return msg;
            }
        }
    }

    /// The next notification named `method` satisfying `pred` that has not
    /// been taken, within `within`.
    async fn next_note(
        &mut self,
        method: &str,
        within: Duration,
        pred: impl Fn(&Value) -> bool,
    ) -> Option<Value> {
        let end = Instant::now() + within;
        loop {
            if let Some(i) = self
                .seen
                .iter()
                .position(|m| m["method"] == method && pred(&m["params"]))
            {
                return Some(self.seen.remove(i));
            }
            let left = end.checked_duration_since(Instant::now())?;
            self.recv(left).await?;
        }
    }

    async fn delivery(&mut self, within: Duration, pred: impl Fn(&Value) -> bool) -> Option<Value> {
        self.next_note("session.delivery", within, pred)
            .await
            .map(|n| n["params"].clone())
    }

    /// Read whatever arrives within `window`.
    async fn drain(&mut self, window: Duration) {
        let end = Instant::now() + window;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if self.recv(left).await.is_none() {
                break;
            }
        }
    }

    async fn subscribe(&mut self, params: Value) -> Value {
        let r = self.call("session.subscribe", params).await;
        assert!(r.get("error").is_none(), "{r}");
        r["result"].clone()
    }
}

/// The lines of a delivery's sections, cards left out.
fn lines_of(delivery: &Value) -> Vec<u64> {
    delivery["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] != "card")
        .map(|s| {
            let serial = s["serial"].as_str().unwrap();
            serial
                .split(':')
                .nth(1)
                .unwrap()
                .split('.')
                .next()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect()
}

fn newest(delivery: &Value) -> String {
    delivery["sections"].as_array().unwrap().last().unwrap()["serial"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_subscription_backfills_then_pushes_within_a_second() {
    let env = start(false).await;
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    let id = c
        .request(
            "session.subscribe",
            json!({"pane_id": "p1", "sections": 50}),
        )
        .await;
    let answer = loop {
        let m = c.recv(WAIT).await.expect("the answer");
        assert_ne!(
            m["method"], "session.delivery",
            "a delivery before the answer"
        );
        if m["id"] == id {
            break m;
        }
    };
    let result = &answer["result"];
    let sub = result["subscription_id"].as_u64().unwrap();
    let first = &result["delivery"];
    assert_eq!(first["reset"], true);
    assert_eq!(first["pane_id"], "p1");
    assert_eq!(first["agent"], "claude");
    assert_eq!(first["session"]["session_id"], SESSION_A);
    assert_eq!(first["session"]["matched"], "id");
    assert!(first["status"].is_string(), "{first}");
    assert_eq!(lines_of(first)[..6], [1, 2, 3, 4, 5, 6]);
    // Appended: pushed within a second.
    append_lines(&env.session(SESSION_A), &[user(100, "and now?")]);
    let at = Instant::now();
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && lines_of(d).contains(&7)
        })
        .await
        .expect("a delivery");
    assert!(at.elapsed() < Duration::from_secs(1), "{:?}", at.elapsed());
    assert_eq!(d["reset"], false);
    assert!(lines_of(&d).contains(&7), "{d}");
    // Unsubscribed: nothing more.
    let r = c
        .call("session.unsubscribe", json!({"subscription_id": sub}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    // What was on its way before the answer may still land; then nothing.
    c.drain(Duration::from_millis(300)).await;
    c.seen.clear();
    append_lines(&env.session(SESSION_A), &[user(101, "anyone?")]);
    assert!(
        c.delivery(Duration::from_millis(1200), |d| d["subscription_id"] == sub)
            .await
            .is_none()
    );
    let r = c
        .call("session.unsubscribe", json!({"subscription_id": sub}))
        .await;
    assert_eq!(r["error"]["code"], -32013, "{r}");
}

#[tokio::test]
async fn after_gives_only_what_was_missed() {
    let env = start(false).await;
    // Far more than the 256 lines a catch-up looks back, then one turn.
    let mut lines: Vec<String> = (0..400).map(|_| blob()).collect();
    lines.push(user(500, "the last one before"));
    append_lines(&env.session(SESSION_A), &lines);
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    let r = c.subscribe(json!({"pane_id": "p1"})).await;
    let held = newest(&r["delivery"]);
    let sub = r["subscription_id"].as_u64().unwrap();
    c.call("session.unsubscribe", json!({"subscription_id": sub}))
        .await;
    append_lines(
        &env.session(SESSION_A),
        &[user(501, "missed one"), user(502, "missed two")],
    );
    let r = c.subscribe(json!({"pane_id": "p1", "after": held})).await;
    let d = &r["delivery"];
    assert_eq!(d["reset"], false, "{d}");
    // The held turn (within reach, resent in case it changed) and the two
    // missed; none of the six from before the blobs.
    assert_eq!(lines_of(d), [407, 408, 409]);
    // A serial of another file resets.
    let r = c
        .subscribe(json!({"pane_id": "p1", "after": "00000001:3.0"}))
        .await;
    assert_eq!(r["delivery"]["reset"], true);
    // A serial that does not parse is refused.
    let r = c
        .call("session.subscribe", json!({"pane_id": "p1", "after": "x"}))
        .await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
}

#[tokio::test]
async fn a_new_session_in_the_pane_resets_onto_it() {
    let env = start(false).await;
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    let r = c.subscribe(json!({"pane_id": "p1"})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    // `/clear`: a new file, and herdr reports its id.
    write_lines(&env.session(SESSION_B), &[user(1, "fresh start")]);
    env.pane(|panes| panes[0].agent_session = Some(session_of(SESSION_B)))
        .await;
    env.herdr_event(
        json!({"event": "pane_updated", "data": {"type": "pane_updated",
        "pane": {"pane_id": "p1", "terminal_id": "t", "workspace_id": "w1", "tab_id": "t1",
                 "focused": true, "agent_status": "working", "revision": 2}}}),
    );
    let at = Instant::now();
    let d = c
        .delivery(WAIT, |d| d["subscription_id"] == sub && d["reset"] == true)
        .await
        .expect("a reset");
    assert!(
        at.elapsed() < Duration::from_secs(1),
        "at once on pane_updated"
    );
    assert_eq!(d["session"]["session_id"], SESSION_B);
    assert_eq!(lines_of(&d), [1]);
    assert_eq!(d["sections"][0]["entries"][0]["summary"], "fresh start");
}

#[tokio::test]
async fn herdr_focus_is_pushed_at_once_and_the_subscription_stays() {
    let env = start(false).await;
    env.pane(|panes| {
        let mut p2 = panes[0].clone();
        p2.pane_id = "p2".to_string();
        p2.agent_session = None;
        p2.cwd = Some("/work/other".to_string());
        panes.push(p2);
    })
    .await;
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    let r = c.subscribe(json!({"pane_id": "p1"})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    env.set_focus("p2").await;
    env.herdr_event(json!({"event": "pane_focused",
        "data": {"type": "pane_focused", "pane_id": "p2", "workspace_id": "w1"}}));
    let at = Instant::now();
    let moved = c
        .next_note("focus.changed", WAIT, |p| p["focus"]["pane_id"] == "p2")
        .await
        .expect("focus.changed");
    assert!(at.elapsed() < Duration::from_secs(2), "{:?}", at.elapsed());
    assert_eq!(moved["params"]["focus"]["pane_id"], "p2");
    // Still on p1.
    append_lines(&env.session(SESSION_A), &[user(200, "still here")]);
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && lines_of(d).contains(&7)
        })
        .await
        .expect("p1's delivery");
    assert_eq!(d["pane_id"], "p1");
    assert!(lines_of(&d).contains(&7));
}

#[tokio::test]
async fn a_closed_pane_ends_its_subscription() {
    let env = start(false).await;
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    let r = c.subscribe(json!({"pane_id": "p1"})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    env.pane(|panes| panes.clear()).await;
    env.herdr_event(json!({"event": "pane_closed",
        "data": {"type": "pane_closed", "pane_id": "p1", "workspace_id": "w1"}}));
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && d["reason"] == "pane_closed"
        })
        .await
        .expect("pane_closed");
    assert!(d["session"].is_null());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let r = c
        .call("session.unsubscribe", json!({"subscription_id": sub}))
        .await;
    assert_eq!(r["error"]["code"], -32013, "it ended by itself: {r}");
}

#[tokio::test]
async fn a_connection_holds_at_most_eight() {
    let env = start(false).await;
    let mut c = Client::unix(&env.socket, "handlr.harness").await;
    for bad in [0, 201] {
        let r = c
            .call(
                "session.subscribe",
                json!({"pane_id": "p1", "sections": bad}),
            )
            .await;
        assert_eq!(r["error"]["code"], -32602, "{r}");
    }
    let r = c.call("session.subscribe", json!({"pane_id": "p9"})).await;
    assert_eq!(r["error"]["code"], -32002, "{r}");
    for _ in 0..8 {
        c.subscribe(json!({"pane_id": "p1", "sections": 1})).await;
    }
    let r = c.call("session.subscribe", json!({"pane_id": "p1"})).await;
    assert_eq!(r["error"]["code"], -32012, "{r}");
}

#[tokio::test]
async fn a_principal_without_reach_is_refused_and_told() {
    let env = start(true).await;
    // Reach only codex: the claude pane is refused at once.
    let mut codex_only = Client::link(&env, Some(&["codex"])).await;
    let r = codex_only
        .call("session.subscribe", json!({"pane_id": "p1"}))
        .await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    // Reach only claude: the pane's agent changes to codex, and the
    // subscription says not_permitted, with nothing after.
    let mut claude_only = Client::link(&env, Some(&["claude"])).await;
    let r = claude_only.subscribe(json!({"pane_id": "p1"})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    env.pane(|panes| {
        panes[0]
            .extra
            .insert("agent".to_string(), Value::String("codex".to_string()));
    })
    .await;
    env.herdr_event(
        json!({"event": "pane_updated", "data": {"type": "pane_updated",
        "pane": {"pane_id": "p1", "terminal_id": "t", "workspace_id": "w1", "tab_id": "t1",
                 "focused": true, "agent_status": "working", "revision": 3}}}),
    );
    let d = claude_only
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && d["reason"] == "not_permitted"
        })
        .await
        .expect("not_permitted");
    assert!(d["session"].is_null() && d["sections"].as_array().unwrap().is_empty());
    // A harness over the link is not served.
    let r = claude_only
        .call(
            "session.history",
            json!({"subscription_id": 99, "before": "00000001:1.0"}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32013);
}

#[tokio::test]
async fn over_the_link_a_backfill_spans_frames_and_pages_back() {
    let env = start(true).await;
    let mut lines = Vec::new();
    for i in 0..450u32 {
        lines.push(user(i, &format!("{i:03} {}", "w".repeat(600))));
    }
    write_lines(&env.session(SESSION_A), &lines);
    let mut c = Client::link(&env, None).await;
    let r = c.subscribe(json!({"pane_id": "p1", "sections": 200})).await;
    let d = &r["delivery"];
    assert_eq!(d["sections"].as_array().unwrap().len(), 200);
    assert!(c.bytes > 65535, "{} bytes: more than one frame", c.bytes);
    let sub = r["subscription_id"].as_u64();
    for s in d["sections"].as_array().unwrap() {
        assert_eq!(
            s["entries"][0]["summary"].as_str().unwrap().chars().count(),
            500
        );
    }
    // A 200-section history page, also spanning frames (A8.7).
    let oldest = d["sections"][0]["serial"].clone();
    let before = c.bytes;
    let h = c
        .call(
            "session.history",
            json!({"subscription_id": sub, "before": oldest, "sections": 200}),
        )
        .await;
    let page = h["result"]["sections"].as_array().unwrap();
    assert_eq!(page.len(), 200);
    assert!(c.bytes - before > 65535);
    assert_eq!(h["result"]["start"], false);
    let lines: Vec<u64> = page
        .iter()
        .map(|s| {
            s["serial"]
                .as_str()
                .unwrap()
                .split(':')
                .nth(1)
                .unwrap()
                .split('.')
                .next()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(lines, (51..=250).collect::<Vec<u64>>());
    // The rest, to the start.
    let h = c
        .call(
            "session.history",
            json!({"subscription_id": sub, "before": page[0]["serial"], "sections": 200}),
        )
        .await;
    assert_eq!(h["result"]["sections"].as_array().unwrap().len(), 50);
    assert_eq!(h["result"]["start"], true);
    // A serial of another file: the session changed.
    let h = c
        .call(
            "session.history",
            json!({"subscription_id": sub, "before": "00000001:3.0"}),
        )
        .await;
    assert_eq!(h["error"]["code"], -32014, "{h}");
}

#[tokio::test]
async fn over_the_link_appends_arrive_and_a_reconnect_catches_up() {
    let env = start(true).await;
    let (address, link) = env.link.clone().unwrap();
    let keypair = generate_keypair().unwrap();
    auth::add(
        &env.dir.path().join("auth.toml"),
        AuthEntry::new("phone", &keypair.public, vec!["companion".to_string()]),
        true,
    )
    .unwrap();
    let mut c = Client::link_with(&address, &link, &keypair).await;
    let r = c.subscribe(json!({"pane_id": "p1"})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    append_lines(&env.session(SESSION_A), &[user(300, "over the link")]);
    let at = Instant::now();
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && lines_of(d).contains(&7)
        })
        .await
        .expect("a delivery");
    assert!(at.elapsed() < Duration::from_secs(1));
    let held = newest(&d);
    // The phone drops off; two more turns are written; it comes back with
    // what it holds.
    drop(c);
    append_lines(
        &env.session(SESSION_A),
        &[user(301, "while away"), user(302, "still away")],
    );
    let mut c = Client::link_with(&address, &link, &keypair).await;
    let r = c.subscribe(json!({"pane_id": "p1", "after": held})).await;
    assert_eq!(r["delivery"]["reset"], false);
    let lines = lines_of(&r["delivery"]);
    assert!(lines.ends_with(&[7, 8, 9]), "{lines:?}");
}

#[tokio::test]
async fn a_client_that_stops_reading_gets_a_reset_not_everything() {
    let env = start(true).await;
    let mut c = Client::link(&env, None).await;
    let r = c.subscribe(json!({"pane_id": "p1"})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    let before = c.bytes;
    // 5 MiB written while the phone reads nothing.
    let body = "q".repeat(1000);
    let mut n = 1000u32;
    for _ in 0..5 {
        let lines: Vec<String> = (0..1000)
            .map(|_| {
                n += 1;
                user(n, &format!("{n} {body}"))
            })
            .collect();
        append_lines(&env.session(SESSION_A), &lines);
    }
    tokio::time::sleep(Duration::from_secs(3)).await;
    // Now it reads: much less than 5 MiB, and a reset that ends on the
    // newest line.
    c.drain(Duration::from_secs(2)).await;
    let got = c.bytes - before;
    assert!(got < 2 * 1024 * 1024, "{got} bytes");
    let deliveries: Vec<&Value> = c
        .seen
        .iter()
        .filter(|m| m["method"] == "session.delivery" && m["params"]["subscription_id"] == sub)
        .map(|m| &m["params"])
        .collect();
    let last_reset = deliveries
        .iter()
        .rev()
        .find(|d| d["reset"] == true)
        .expect("a reset");
    assert!(lines_of(last_reset).len() <= 200);
    let newest_line = deliveries.iter().flat_map(|d| lines_of(d)).max().unwrap();
    assert_eq!(newest_line, 6 + 5000);
}

#[tokio::test]
async fn a_confirmation_for_the_pane_is_an_approval_card_and_journalled() {
    let env = start(false).await;
    let mut phone = Client::unix(&env.socket, "handlr.companion").await;
    let r = phone.subscribe(json!({"pane_id": "p1"})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    let mut mcp = Client::unix(&env.socket, "handlr.harness").await;
    let r = mcp
        .call(
            "agents.send",
            json!({"pane_id": "p1", "command": "continue", "handle": "keep-going"}),
        )
        .await;
    assert_eq!(r["result"]["status"], "pending", "{r}");
    let id = r["result"]["confirm_id"].as_u64().unwrap();
    let card = |d: &Value, live: bool| {
        d["sections"].as_array().unwrap().iter().any(|s| {
            s["card"]["card"] == "approval"
                && s["card"]["confirm_id"] == id
                && s["card"]["live"] == live
        })
    };
    let d = phone
        .delivery(WAIT, |d| d["subscription_id"] == sub && card(d, true))
        .await
        .expect("a live approval card");
    let approval = d["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["card"]["card"] == "approval")
        .unwrap();
    // After the newest line when it was asked: line 6.
    assert!(
        approval["serial"].as_str().unwrap().ends_with(":6.64"),
        "{approval}"
    );
    let r = mcp.call("confirm.cancel", json!({"confirm_id": id})).await;
    assert_eq!(r["result"]["outcome"], "cancelled", "{r}");
    phone
        .delivery(WAIT, |d| d["subscription_id"] == sub && card(d, false))
        .await
        .expect("the card resolved");
    // The journal: asked and resolved, private.
    let journal = env
        .dir
        .path()
        .join("home/.local/state/handlr/sessions")
        .join(format!("{SESSION_A}.jsonl"));
    let text = std::fs::read_to_string(&journal).unwrap();
    let lines: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert_eq!(lines[0]["line"], 6);
    assert_eq!(lines[0]["slot"], 64);
    assert_eq!(lines[0]["command"], "continue");
    assert_eq!(lines[1]["outcome"], "cancelled");
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&journal).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    // A new subscription's backfill rebuilds the resolved card from the
    // journal.
    let r = phone.subscribe(json!({"pane_id": "p1"})).await;
    assert!(card(&r["delivery"], false), "{}", r["delivery"]);
}

/// What `handlr session tail` printed, shared with the test.
#[derive(Clone, Default)]
struct Printed(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Printed {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Printed {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }

    /// Wait until the output holds `want`.
    async fn wait_for(&self, want: &str) -> String {
        let end = Instant::now() + WAIT;
        loop {
            let text = self.text();
            if text.contains(want) {
                return text;
            }
            assert!(Instant::now() < end, "no {want:?} in:\n{text}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// `handlr session tail` against the daemon, printing into `Printed`.
fn tail(env: &Env, options: handlr::tail::TailOptions) -> (Printed, tokio::task::JoinHandle<()>) {
    let socket = env.socket.clone();
    let printed = Printed::default();
    let mut out = printed.clone();
    let job = tokio::spawn(async move {
        let code = handlr::tail::run(&socket, &options, &mut out)
            .await
            .unwrap();
        assert_eq!(code, 0);
    });
    (printed, job)
}

#[tokio::test]
async fn session_tail_backfills_pages_back_and_prints_what_comes() {
    let env = start(false).await;
    let mut lines = Vec::new();
    for i in 0..30u32 {
        lines.push(user(1000 + i, &format!("older {i}")));
    }
    // 30 older turns, then the six of `start`.
    let path = env.session(SESSION_A);
    let six = std::fs::read_to_string(&path).unwrap();
    write_lines(&path, &lines);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(&six);
    std::fs::write(&path, text).unwrap();
    // The newest six sections (a Finished card among them), then 20 more
    // back: lines 13 to 32.
    let (printed, job) = tail(
        &env,
        handlr::tail::TailOptions {
            pane: Some("p1".to_string()),
            sections: 6,
            history: 20,
            ..handlr::tail::TailOptions::default()
        },
    );
    printed.wait_for("-- history: 20 section(s) before ").await;
    append_lines(&path, &[user(2000, "a new prompt")]);
    let out = printed.wait_for(":37.0 user text: a new prompt").await;
    job.abort();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "-- subscribed to p1 --", "{out}");
    assert_eq!(lines[1], "-- reset (backfill) --", "{out}");
    assert!(out.contains(":33.0 user text: prompt 1"), "{out}");
    assert!(out.contains(":13.0 user text: older 12"), "{out}");
    assert!(!out.contains(":12.0 user text"), "only 20 back: {out}");
}

#[tokio::test]
async fn session_tail_follows_the_focus_only_when_asked() {
    let env = start(false).await;
    write_lines(&env.session(SESSION_B), &[user(1, "the other agent")]);
    env.pane(|panes| {
        let mut p2 = panes[0].clone();
        p2.pane_id = "p2".to_string();
        p2.agent_session = Some(session_of(SESSION_B));
        panes.push(p2);
    })
    .await;
    // Without --pane: herdr's focus, p1. One tail follows, one does not.
    let follow = handlr::tail::TailOptions {
        follow: true,
        ..handlr::tail::TailOptions::default()
    };
    let (following, a) = tail(&env, follow);
    let (staying, b) = tail(&env, handlr::tail::TailOptions::default());
    following.wait_for("-- subscribed to p1 --").await;
    staying.wait_for("-- subscribed to p1 --").await;
    env.set_focus("p2").await;
    env.herdr_event(json!({"event": "pane_focused",
        "data": {"type": "pane_focused", "pane_id": "p2", "workspace_id": "w1"}}));
    following.wait_for("-- subscribed to p2 --").await;
    append_lines(&env.session(SESSION_B), &[user(2, "on p2 now")]);
    append_lines(&env.session(SESSION_A), &[user(3, "still on p1")]);
    let out = following.wait_for(":2.0 user text: on p2 now").await;
    assert!(out.contains("-- following "), "{out}");
    let out = staying.wait_for(":7.0 user text: still on p1").await;
    assert!(!out.contains("p2"), "{out}");
    a.abort();
    b.abort();
}

fn calls(n: u32, id: &str, command: &str) -> String {
    json!({"type": "assistant", "timestamp": ts(n), "isSidechain": false,
        "message": {"role": "assistant", "type": "message",
                    "content": [{"type": "tool_use", "id": id, "name": "Bash",
                                 "input": {"command": command}}]}})
    .to_string()
}

/// The indexes of a delivery's or a result's slots.
fn slot_indexes(v: &Value) -> Vec<u64> {
    v["slots"]
        .as_array()
        .map(|a| a.iter().map(|s| s["index"].as_u64().unwrap()).collect())
        .unwrap_or_default()
}

fn section_indexes(v: &Value) -> Vec<u64> {
    v["sections"]
        .as_array()
        .map(|a| a.iter().map(|s| s["index"].as_u64().unwrap_or(0)).collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn an_outline_covers_every_index_in_chunks_with_only_the_newest_contents() {
    let env = start(false).await;
    let lines: Vec<String> = (0..4500u32)
        .map(|i| user(i, &format!("prompt {i}")))
        .collect();
    write_lines(&env.session(SESSION_A), &lines);
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    let r = c.subscribe(json!({"pane_id": "p1", "outline": true})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    let first = &r["delivery"];
    assert_eq!(first["reset"], true);
    assert_eq!(first["last"], 4499);
    assert_eq!(slot_indexes(first), (0..2000).collect::<Vec<u64>>());
    assert!(
        first["sections"].as_array().unwrap().is_empty(),
        "no contents from the start"
    );
    let slot = &first["slots"][7];
    assert_eq!(slot["kind"], "turn");
    assert_eq!(slot["lines"], 1);
    assert_eq!(slot["chars"], "prompt 7".len());
    assert_eq!(slot["rev"], 0);
    assert!(slot["serial"].as_str().unwrap().ends_with(":8.0"), "{slot}");
    // The rest follows by itself, at most SLOTS_MAX a delivery.
    let mut indexes = slot_indexes(first);
    let mut contents = Vec::new();
    while indexes.len() < 4500 {
        let d = c
            .delivery(WAIT, |d| d["subscription_id"] == sub)
            .await
            .expect("the outline continued");
        assert_eq!(d["reset"], false);
        assert!(slot_indexes(&d).len() <= 2000);
        indexes.extend(slot_indexes(&d));
        contents.extend(section_indexes(&d));
    }
    assert_eq!(indexes, (0..4500).collect::<Vec<u64>>());
    // Only the newest LIVE_INLINE (8) indexes' contents rode along.
    assert_eq!(contents, (4492..4500).collect::<Vec<u64>>());
    // A new section: its slot and its contents.
    append_lines(&env.session(SESSION_A), &[user(9000, "one more")]);
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && slot_indexes(d).contains(&4500)
        })
        .await
        .expect("the new slot");
    assert_eq!(d["last"], 4500);
    assert_eq!(section_indexes(&d), [4500]);
    assert_eq!(d["sections"][0]["entries"][0]["summary"], "one more");
}

#[tokio::test]
async fn an_outline_resumes_from_an_index_and_content_answers_ranges() {
    let env = start(false).await;
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    // Three turns and replies: two resolved Finished cards, and a live one
    // with the pane idle.
    env.pane(|panes| panes[0].agent_status = Some(handlr::transport::AgentStatus::Idle))
        .await;
    let r = c.subscribe(json!({"pane_id": "p1", "outline": true})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    let d = &r["delivery"];
    assert_eq!(slot_indexes(d), (0..9).collect::<Vec<u64>>());
    assert_eq!(d["last"], 8);
    let finished = d["slots"][8]["serial"].as_str().unwrap().to_string();
    assert!(finished.ends_with(":6.194"), "{d}");
    assert_eq!(section_indexes(d), (1..9).collect::<Vec<u64>>());
    // The agent carries on: the live card goes, and a tool comes.
    append_lines(&env.session(SESSION_A), &[calls(60, "c1", "ls")]);
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && slot_indexes(d).contains(&9)
        })
        .await
        .expect("the tool's slot");
    assert_eq!(d["removed_index"], json!([8]), "{d}");
    assert_eq!(d["removed"], json!([finished]));
    assert_eq!(d["last"], 9);
    async fn content(c: &mut Client, sub: u64, mut params: Value) -> Value {
        params["subscription_id"] = json!(sub);
        let r = c.call("session.content", params).await;
        assert!(r.get("error").is_none(), "{r}");
        r["result"].clone()
    }
    // From only.
    let r = content(&mut c, sub, json!({"from": 6, "limit": 2})).await;
    assert_eq!(section_indexes(&r), [6, 7]);
    assert_eq!(r["more"], true);
    assert_eq!(r["last"], 9);
    let r = content(&mut c, sub, json!({"from": 7, "limit": 5})).await;
    assert_eq!(section_indexes(&r), [7, 9]);
    assert_eq!(r["removed_index"], json!([8]));
    assert_eq!(r["more"], false);
    assert_eq!(r["sections"][1]["kind"], "tool");
    // To only: the newest ending at `to`, oldest first.
    let r = content(&mut c, sub, json!({"to": 9, "limit": 2})).await;
    assert_eq!(section_indexes(&r), [7, 9]);
    assert_eq!(r["removed_index"], json!([8]));
    assert_eq!(r["more"], true);
    // Both: cut at the limit from `from`.
    let r = content(&mut c, sub, json!({"from": 0, "to": 4, "limit": 3})).await;
    assert_eq!(section_indexes(&r), [0, 1, 2]);
    assert_eq!(r["more"], true);
    assert_eq!(r["sections"][0]["entries"][0]["summary"], "prompt 0");
    let r = content(&mut c, sub, json!({"from": 3, "to": 4})).await;
    assert_eq!(section_indexes(&r), [3, 4]);
    assert_eq!(r["more"], false);
    // Neither: the newest.
    let r = content(&mut c, sub, json!({"limit": 3})).await;
    assert_eq!(section_indexes(&r), [6, 7, 9]);
    assert_eq!(r["more"], true);
    // Slots only.
    let r = content(
        &mut c,
        sub,
        json!({"from": 0, "limit": 200, "slots_only": true}),
    )
    .await;
    assert_eq!(slot_indexes(&r), [0, 1, 2, 3, 4, 5, 6, 7, 9]);
    assert!(r["sections"].as_array().unwrap().is_empty());
    assert_eq!(r["slots"][8]["kind"], "tool");
    // Bad ranges and limits are refused.
    for mut bad in [
        json!({"limit": 0}),
        json!({"limit": 201}),
        json!({"from": 5, "to": 4}),
    ] {
        bad["subscription_id"] = json!(sub);
        let r = c.call("session.content", bad).await;
        assert_eq!(r["error"]["code"], -32602, "{r}");
    }
    // Far past the start, one more turn, held; then the client goes.
    let blobs: Vec<String> = (0..400).map(|_| blob()).collect();
    append_lines(&env.session(SESSION_A), &blobs);
    append_lines(&env.session(SESSION_A), &[user(65, "held")]);
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && slot_indexes(d).contains(&10)
        })
        .await
        .expect("the held turn");
    let held = d["slots"][0]["serial"].clone();
    assert!(held.as_str().unwrap().ends_with(":408.0"), "{d}");
    c.call("session.unsubscribe", json!({"subscription_id": sub}))
        .await;
    append_lines(
        &env.session(SESSION_A),
        &[user(70, "missed one"), user(71, "missed two")],
    );
    // Resumed from the next index: only what follows, and the held one,
    // within reach of it, in case it changed.
    let r = c
        .subscribe(json!({"pane_id": "p1", "outline": true, "after": held, "from": 11}))
        .await;
    let d = &r["delivery"];
    assert_eq!(d["reset"], false, "{d}");
    assert_eq!(slot_indexes(d), [10, 11, 12]);
    assert_eq!(d["last"], 12);
    assert_eq!(section_indexes(d), [10, 11, 12]);
    // Another file's serial: the outline from 0, as a reset.
    let r = c
        .subscribe(json!({"pane_id": "p1", "outline": true, "after": "00000001:3.0", "from": 10}))
        .await;
    assert_eq!(r["delivery"]["reset"], true);
    assert_eq!(slot_indexes(&r["delivery"]).first(), Some(&0));
}

#[tokio::test]
async fn content_of_a_file_the_subscription_moved_off_is_session_changed() {
    let env = start(false).await;
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    let r = c.subscribe(json!({"pane_id": "p1", "outline": true})).await;
    let sub = r["subscription_id"].as_u64().unwrap();
    let r = c
        .call(
            "session.content",
            json!({"subscription_id": sub, "from": 0}),
        )
        .await;
    assert_eq!(section_indexes(&r["result"]).len(), 8, "{r}");
    // The phone goes to the background, so the reset waits; meanwhile
    // `/clear` starts a new file.
    let r = c.call("ping", json!({"active": false})).await;
    assert!(r.get("error").is_none(), "{r}");
    write_lines(&env.session(SESSION_B), &[user(1, "fresh start")]);
    env.pane(|panes| panes[0].agent_session = Some(session_of(SESSION_B)))
        .await;
    env.herdr_event(
        json!({"event": "pane_updated", "data": {"type": "pane_updated",
        "pane": {"pane_id": "p1", "terminal_id": "t", "workspace_id": "w1", "tab_id": "t1",
                 "focused": true, "agent_status": "working", "revision": 2}}}),
    );
    let end = Instant::now() + WAIT;
    loop {
        let r = c
            .call(
                "session.content",
                json!({"subscription_id": sub, "from": 0}),
            )
            .await;
        if r["error"]["code"] == -32014 {
            break;
        }
        assert!(Instant::now() < end, "never changed: {r}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Awake again: the reset onto the new file, and content from it.
    c.call("ping", json!({"active": true})).await;
    let d = c
        .delivery(WAIT, |d| d["subscription_id"] == sub && d["reset"] == true)
        .await
        .expect("the reset");
    assert_eq!(d["session"]["session_id"], SESSION_B);
    assert_eq!(slot_indexes(&d), [0]);
    let r = c
        .call(
            "session.content",
            json!({"subscription_id": sub, "from": 0}),
        )
        .await;
    assert_eq!(
        r["result"]["sections"][0]["entries"][0]["summary"], "fresh start",
        "{r}"
    );
}

#[tokio::test]
async fn a_subscription_without_outline_is_as_before_beside_one_with_it() {
    let env = start(false).await;
    let mut c = Client::unix(&env.socket, "handlr.companion").await;
    let outlined = c.subscribe(json!({"pane_id": "p1", "outline": true})).await;
    let r = c.subscribe(json!({"pane_id": "p1"})).await;
    let plain = r["subscription_id"].as_u64().unwrap();
    let no_outline = |d: &Value| {
        ["slots", "removed_index", "first", "last"]
            .iter()
            .all(|k| d.get(*k).is_none())
            && d["sections"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s.get("index").is_none())
    };
    assert!(no_outline(&r["delivery"]), "{}", r["delivery"]);
    assert_eq!(lines_of(&r["delivery"]), [1, 2, 3, 4, 5, 6]);
    // The tailer they share numbers what comes; the plain one is not told.
    append_lines(&env.session(SESSION_A), &[user(100, "and now?")]);
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == plain && lines_of(d).contains(&7)
        })
        .await
        .expect("the plain delivery");
    assert!(no_outline(&d), "{d}");
    let sub = outlined["subscription_id"].as_u64().unwrap();
    let d = c
        .delivery(WAIT, |d| {
            d["subscription_id"] == sub && !slot_indexes(d).is_empty()
        })
        .await
        .expect("the outline delivery");
    assert!(section_indexes(&d).iter().all(|&i| i >= 8), "{d}");
}

#[tokio::test]
async fn session_tail_prints_the_outline_and_a_content_range() {
    let env = start(false).await;
    let (printed, job) = tail(
        &env,
        handlr::tail::TailOptions {
            pane: Some("p1".to_string()),
            content: Some((Some(0), Some(2))),
            ..handlr::tail::TailOptions::default()
        },
    );
    let marker = "-- content #0 to #2: 3 section(s) --";
    let out = printed.wait_for(marker).await;
    job.abort();
    assert!(out.contains("-- outline: 8 slot(s), last #7 --"), "{out}");
    assert!(out.contains(":1.0 turn 1x8 rev 0"), "{out}");
    let after: Vec<&str> = out
        .split(marker)
        .nth(1)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert!(
        after[0].starts_with("#0 ") && after[0].ends_with("user text: prompt 0"),
        "{out}"
    );
    assert!(
        after[2].starts_with("#2 ") && after[2].contains(" card finished resolved"),
        "{out}"
    );
}
