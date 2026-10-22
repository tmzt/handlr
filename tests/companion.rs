//! `handlr.companion` and the turn service end to end (plan 20261022_02,
//! step 4, with the amendments of 12:06, 18:03 and 18:05 EDT), over the
//! herdr mock and a scripted mock engine. The mock's command and keystroke
//! logs stand in for the panes: nothing reaches a live herdr or Gemini.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use handlr::config::{AgentConfig, AppConfig};
use handlr::reasoning::{MockHandler, MockReasoningEngine, ReasoningEngine, ToolExecutor};
use handlr::server::companion::{TurnOptions, INTERRUPTED_ENTRY};
use handlr::server::link::{Link, LinkSettings};
use handlr::server::{self, CoreEvent, Daemon, ServeOptions};
use handlr::transport::{AgentStatus, HerdrClient, HerdrTransport};
use libhandlr_proto::link::{generate_keypair, pair, LinkStream, PairingPayload};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpStream, UnixStream};
use tokio_util::codec::{FramedRead, LinesCodec};

const WAIT: Duration = Duration::from_secs(5);

/// The engine's script, keyed on the user's text:
/// * `send <command> to <target>`: `run_command`, then a short reply;
/// * `... then slow`: the same, then a call that never ends in time;
/// * `watch <target> then slow`: `watch_pane`, then the slow call;
/// * `slow`: only the slow call;
/// * anything else: an echo.
struct Script;

#[async_trait]
impl MockHandler for Script {
    async fn handle(
        &self,
        text: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        let (text, slow) = match text.strip_suffix(" then slow") {
            Some(rest) => (rest, true),
            None => (text, text == "slow"),
        };
        let mut reply = format!("You said {text}.");
        if let Some(rest) = text.strip_prefix("send ") {
            let (command, target) = rest.split_once(" to ").expect("send <command> to <target>");
            let result = executor
                .execute_tool(
                    "run_command",
                    &json!({"pane_id": target, "command": command}),
                )
                .await?;
            reply = if result["status"] == "awaiting_user_approval" {
                "That needs your approval.".to_string()
            } else {
                "Sent.".to_string()
            };
        } else if let Some(target) = text.strip_prefix("watch ") {
            executor
                .execute_tool("watch_pane", &json!({"pane_id": target}))
                .await?;
            reply = "Watching.".to_string();
        }
        if slow {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        Ok(Some(reply))
    }
}

struct Env {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    sessions: PathBuf,
    daemon: Arc<Daemon>,
    /// The link's address and server, when it is on.
    link: Option<(String, Arc<Link>)>,
}

fn config(agents: Vec<AgentConfig>) -> AppConfig {
    AppConfig {
        agents,
        ..AppConfig::default()
    }
}

/// claude needs confirmation, as by default.
fn confirming() -> AppConfig {
    AppConfig::default()
}

/// claude takes `continue` directly, with interrupt key `key`.
fn direct(key: &str) -> AppConfig {
    config(vec![AgentConfig::new(
        "claude",
        vec!["continue".to_string()],
        false,
    )
    .with_interrupt_key(key)])
}

async fn start_with(config: AppConfig, engine: Arc<dyn ReasoningEngine>, with_link: bool) -> Env {
    let dir = tempfile::Builder::new()
        .prefix("hc")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("h.sock");
    let sessions = dir.path().join("sessions");
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
        &config,
        link.as_ref().map(|(_, l)| Arc::clone(l)),
        TurnOptions {
            engine: Some(engine),
            session_dir: Some(sessions.clone()),
        },
    );
    let listener = server::bind(&socket).await.unwrap();
    if with_link {
        server::link::spawn(Arc::clone(&daemon), listeners);
    }
    tokio::spawn(server::run(
        Arc::clone(&daemon),
        listener,
        ServeOptions {
            show_tokens: false,
            event_transport: None,
        },
    ));
    Env {
        _dir: dir,
        socket,
        sessions,
        daemon,
        link,
    }
}

async fn start(config: AppConfig) -> Env {
    start_with(
        config,
        Arc::new(MockReasoningEngine::with_handler(Script)),
        false,
    )
    .await
}

async fn mock_logs(daemon: &Daemon) -> (Vec<(String, String)>, Vec<(String, String)>) {
    match daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            let state = state.read().await;
            (state.command_log.clone(), state.keystroke_log.clone())
        }
        _ => unreachable!("tests use the mock"),
    }
}

async fn set_status(daemon: &Daemon, status: AgentStatus) {
    match daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            state.write().await.snapshot.workspaces[0].tabs[0].panes[0].agent_status = Some(status);
        }
        _ => unreachable!("tests use the mock"),
    }
}

/// The history the session `id` has written.
fn history(env: &Env, id: &str) -> Vec<Value> {
    let text = std::fs::read_to_string(env.sessions.join(format!("{id}.jsonl"))).unwrap();
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

enum Wire {
    Unix {
        write: OwnedWriteHalf,
        lines: FramedRead<OwnedReadHalf, LinesCodec>,
    },
    Link(LinkStream<TcpStream>),
}

/// A client: requests are sent without waiting, and every message that
/// arrives is kept in order.
struct Client {
    wire: Wire,
    next_id: i64,
    seen: Vec<Value>,
}

impl Client {
    async fn unix(socket: &Path, protocol: &str, client: &str) -> (Client, Value) {
        let (read, write) = UnixStream::connect(socket).await.unwrap().into_split();
        let mut c = Client {
            wire: Wire::Unix {
                write,
                lines: FramedRead::new(read, LinesCodec::new()),
            },
            next_id: 0,
            seen: Vec::new(),
        };
        let hello = c
            .call(
                "hello",
                json!({"protocol": protocol, "version": 1, "client": client}),
            )
            .await;
        assert_eq!(hello["result"]["server"], "handlr", "{hello}");
        (c, hello)
    }

    async fn companion(env: &Env) -> (Client, Value) {
        Client::unix(&env.socket, "handlr.companion", "t").await
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
        let value: Value = match &mut self.wire {
            Wire::Unix { lines, .. } => {
                let line = tokio::time::timeout(within, lines.next()).await.ok()??;
                serde_json::from_str(&line.unwrap()).unwrap()
            }
            Wire::Link(stream) => {
                let message = tokio::time::timeout(within, stream.recv_message())
                    .await
                    .ok()?
                    .ok()?;
                serde_json::from_slice(&message).unwrap()
            }
        };
        self.seen.push(value.clone());
        Some(value)
    }

    /// Send a request without waiting; returns its id.
    async fn request(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        id
    }

    /// The response to `id`, kept or yet to arrive.
    async fn response(&mut self, id: i64) -> Value {
        if let Some(r) = self
            .seen
            .iter()
            .find(|m| m["id"] == id && m.get("method").is_none())
        {
            return r.clone();
        }
        loop {
            let msg = self.recv(WAIT).await.expect("no response in time");
            if msg["id"] == id && msg.get("method").is_none() {
                return msg;
            }
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.request(method, params).await;
        self.response(id).await
    }

    /// The first notification named `method`, kept or yet to arrive.
    async fn note(&mut self, method: &str) -> Value {
        if let Some(n) = self.seen.iter().find(|m| m["method"] == method) {
            return n.clone();
        }
        loop {
            let msg = self.recv(WAIT).await.expect("no notification in time");
            if msg["method"] == method {
                return msg;
            }
        }
    }

    /// Where `pred` first matched in what has arrived.
    fn position(&self, pred: impl Fn(&Value) -> bool) -> Option<usize> {
        self.seen.iter().position(pred)
    }

    /// Read everything that arrives within `window`.
    async fn drain(&mut self, window: Duration) {
        let end = tokio::time::Instant::now() + window;
        while let Some(left) = end.checked_duration_since(tokio::time::Instant::now()) {
            if self.recv(left).await.is_none() {
                break;
            }
        }
    }
}

fn turn(id: u64, text: &str) -> Value {
    json!({"turn_id": id, "text": text})
}

#[tokio::test]
async fn hello_gives_the_session_and_the_vocabulary() {
    let env = start(confirming()).await;
    let (_c, hello) = Client::companion(&env).await;
    assert_eq!(
        hello["result"]["session_id"], "companion:local:t",
        "{hello}"
    );
    assert_eq!(hello["result"]["protocol_version"], 1);
    let vocabulary = hello["result"]["vocabulary"].as_array().unwrap();
    assert!(vocabulary.iter().any(|w| w == "handlr"), "{hello}");

    // Turns are a companion's: a harness connection does not have them.
    let (mut harness, _) = Client::unix(&env.socket, "handlr.harness", "h").await;
    let r = harness.call("turn.start", turn(1, "hello")).await;
    assert_eq!(r["error"]["code"], -32601, "{r}");
}

#[tokio::test]
async fn approve_sends_once_and_ran_lists_it() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let id = c
        .request("turn.start", turn(1, "send continue to claude"))
        .await;

    // The reply first, then the question, which names the command and the
    // agent's spoken name.
    let say = c.note("turn.say").await;
    assert_eq!(say["params"]["text"], "That needs your approval.");
    assert_eq!(say["params"]["seq"], 0);
    let asked = c.note("confirm.requested").await;
    assert_eq!(
        asked["params"]["text"], "Approve sending \"continue\" to Claude?",
        "{asked}"
    );
    assert_eq!(asked["params"]["spoken"], "Claude");
    assert!(
        c.position(|m| m["method"] == "turn.say")
            < c.position(|m| m["method"] == "confirm.requested")
    );
    // A local companion cannot approve others' requests, so it is not shown
    // the token.
    assert!(asked["params"].get("token").is_none(), "{asked}");
    assert!(
        mock_logs(&env.daemon).await.0.is_empty(),
        "nothing sent yet"
    );

    let confirm_id = asked["params"]["confirm_id"].as_u64().unwrap();
    let r = c
        .call(
            "confirm.reply",
            json!({"confirm_id": confirm_id, "approve": true}),
        )
        .await;
    assert_eq!(r["result"]["outcome"], "sent", "{r}");

    let done = c.response(id).await;
    assert_eq!(done["result"]["outcome"], "done", "{done}");
    assert_eq!(
        done["result"]["ran"],
        json!([{"pane_id": "p1", "command": "continue", "name": "Claude", "spoken": "Claude"}])
    );
    assert_eq!(
        mock_logs(&env.daemon).await.0,
        vec![("p1".to_string(), "continue".to_string())],
        "sent exactly once"
    );
    let resolved = c.note("confirm.resolved").await;
    assert_eq!(resolved["params"]["outcome"], "sent");

    // The history has the turn.
    let h = history(&env, "companion:local:t");
    assert_eq!(h[0]["content"], "send continue to claude");
    assert_eq!(h[1]["content"], "That needs your approval.");
}

#[tokio::test]
async fn anything_else_denies_and_sends_nothing() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let id = c
        .request("turn.start", turn(1, "send continue to claude"))
        .await;
    let asked = c.note("confirm.requested").await;
    let confirm_id = asked["params"]["confirm_id"].as_u64().unwrap();
    let r = c
        .call(
            "confirm.reply",
            json!({"confirm_id": confirm_id, "approve": false}),
        )
        .await;
    assert_eq!(r["result"]["outcome"], "denied", "{r}");
    let done = c.response(id).await;
    assert_eq!(done["result"]["outcome"], "done", "{done}");
    assert_eq!(done["result"]["ran"], json!([]));
    assert!(mock_logs(&env.daemon).await.0.is_empty());
}

#[tokio::test]
async fn the_next_turn_denies_a_pending_question() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let first = c
        .request("turn.start", turn(1, "send continue to claude"))
        .await;
    c.note("confirm.requested").await;
    // A line other than "approve" while it is pending: handled as a turn,
    // and the question ends as denied.
    let second = c.request("turn.start", turn(2, "what now")).await;
    let r = c.response(first).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
    assert_eq!(r["result"]["ran"], json!([]));
    let r = c.response(second).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
    assert_eq!(
        c.note("confirm.resolved").await["params"]["outcome"],
        "denied"
    );
    assert!(mock_logs(&env.daemon).await.0.is_empty());
}

#[tokio::test]
async fn an_interrupt_ends_the_turn_within_500_ms() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let id = c.request("turn.start", turn(7, "slow")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let asked_at = Instant::now();
    c.request("turn.interrupt", json!({"turn_id": 7})).await;
    let r = c.response(id).await;
    let took = asked_at.elapsed();
    assert_eq!(r["result"]["outcome"], "interrupted", "{r}");
    assert!(took < Duration::from_millis(500), "took {took:?}");
    assert_eq!(r["result"]["ran"], json!([]));
    // Nothing was addressed, so no keys.
    assert_eq!(r["result"]["interrupted"], json!([]));
    assert!(mock_logs(&env.daemon).await.1.is_empty());

    let h = history(&env, "companion:local:t");
    assert_eq!(h.first().unwrap()["content"], "slow");
    assert_eq!(h.last().unwrap()["content"], INTERRUPTED_ENTRY);
    assert_eq!(h.last().unwrap()["role"], "model");

    // `cancel` on the turn.start id does the same.
    let id = c.request("turn.start", turn(8, "slow")).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let asked_at = Instant::now();
    let r = c.call("cancel", json!({"id": id})).await;
    assert_eq!(r["result"]["cancelled"], true, "{r}");
    let r = c.response(id).await;
    assert_eq!(r["result"]["outcome"], "interrupted", "{r}");
    assert!(asked_at.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn a_second_turn_is_busy() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let first = c.request("turn.start", turn(1, "slow")).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let r = c.call("turn.start", turn(2, "hello")).await;
    assert_eq!(r["error"]["code"], -32020, "{r}");
    // Another connection of the same session is busy too.
    let (mut other, _) = Client::companion(&env).await;
    let r = other.call("turn.start", turn(3, "hello")).await;
    assert_eq!(r["error"]["code"], -32020, "{r}");
    c.request("turn.interrupt", json!({"turn_id": 1})).await;
    assert_eq!(c.response(first).await["result"]["outcome"], "interrupted");
    let r = c.call("turn.start", turn(4, "hello")).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
    assert_eq!(c.note("turn.say").await["params"]["turn_id"], 4);
}

#[tokio::test]
async fn an_interrupt_sends_the_key_to_the_pane_the_turn_sent_to() {
    let env = start(direct("esc")).await;
    let (mut c, _) = Client::companion(&env).await;
    // Idle: the key goes to a pane the turn sent to whatever its status.
    set_status(&env.daemon, AgentStatus::Idle).await;
    let id = c
        .request("turn.start", turn(1, "send continue to claude then slow"))
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.request("turn.interrupt", json!({"turn_id": 1})).await;
    let r = c.response(id).await;
    assert_eq!(r["result"]["outcome"], "interrupted", "{r}");
    assert_eq!(r["result"]["ran"][0]["command"], "continue", "{r}");
    assert_eq!(
        r["result"]["interrupted"],
        json!([{"pane_id": "p1", "name": "Claude", "spoken": "Claude"}])
    );
    let (commands, keys) = mock_logs(&env.daemon).await;
    assert_eq!(commands, vec![("p1".to_string(), "continue".to_string())]);
    assert_eq!(keys, vec![("p1".to_string(), "esc".to_string())]);
}

#[tokio::test]
async fn the_current_agent_is_interrupted_only_while_working() {
    let env = start(direct("esc")).await;
    let (mut c, _) = Client::companion(&env).await;

    // Working (the mock's default): the current agent gets the key.
    let id = c
        .request("turn.start", turn(1, "watch claude then slow"))
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.request("turn.interrupt", json!({"turn_id": 1})).await;
    let r = c.response(id).await;
    assert_eq!(r["result"]["interrupted"][0]["pane_id"], "p1", "{r}");
    assert_eq!(mock_logs(&env.daemon).await.1.len(), 1);

    // Idle: left alone, with nothing sent this turn.
    set_status(&env.daemon, AgentStatus::Idle).await;
    let id = c.request("turn.start", turn(2, "slow")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.request("turn.interrupt", json!({"turn_id": 2})).await;
    let r = c.response(id).await;
    assert_eq!(r["result"]["outcome"], "interrupted", "{r}");
    assert_eq!(r["result"]["interrupted"], json!([]));
    assert_eq!(mock_logs(&env.daemon).await.1.len(), 1, "no second key");
}

#[tokio::test]
async fn an_empty_interrupt_key_sends_nothing() {
    let env = start(direct("")).await;
    let (mut c, _) = Client::companion(&env).await;
    let id = c
        .request("turn.start", turn(1, "send continue to claude then slow"))
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.request("turn.interrupt", json!({"turn_id": 1})).await;
    let r = c.response(id).await;
    assert_eq!(r["result"]["outcome"], "interrupted", "{r}");
    assert_eq!(r["result"]["interrupted"], json!([]));
    assert!(mock_logs(&env.daemon).await.1.is_empty());

    let (mut harness, _) = Client::unix(&env.socket, "handlr.harness", "h").await;
    let r = harness
        .call("agents.interrupt", json!({"target": "claude"}))
        .await;
    assert_eq!(r["result"]["interrupted"], false, "{r}");
    assert!(mock_logs(&env.daemon).await.1.is_empty());
}

#[tokio::test]
async fn agents_interrupt_from_a_harness_sends_the_key() {
    let env = start(confirming()).await;
    let (mut harness, _) = Client::unix(&env.socket, "handlr.harness", "mcp").await;
    let r = harness
        .call("agents.interrupt", json!({"target": "claude"}))
        .await;
    assert_eq!(
        r["result"],
        json!({"interrupted": true, "name": "Claude", "spoken": "Claude"}),
        "{r}"
    );
    assert_eq!(
        mock_logs(&env.daemon).await.1,
        vec![("p1".to_string(), "esc".to_string())]
    );
    // Not gated by confirmation, and nothing typed.
    assert!(mock_logs(&env.daemon).await.0.is_empty());
    let r = harness
        .call("agents.interrupt", json!({"target": "nobody"}))
        .await;
    assert_eq!(r["error"]["code"], -32002, "{r}");

    // Over the companion protocol too.
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("agents.interrupt", json!({"target": "p1"})).await;
    assert_eq!(r["result"]["interrupted"], true, "{r}");
}

#[tokio::test]
async fn a_notice_waits_for_a_pending_confirmation() {
    let env = start(confirming()).await;
    env.daemon.watches.watch("p1");
    let (mut c, _) = Client::companion(&env).await;

    // With nothing pending, a notice comes at once.
    let _ = env.daemon.events.send(CoreEvent::AgentStatus {
        pane_id: "p1".to_string(),
        agent: "claude".to_string(),
        status: AgentStatus::Done,
    });
    let n = c.note("notice").await;
    assert_eq!(n["params"]["text"], "Claude finished.", "{n}");
    assert_eq!(n["params"]["kind"], "agent_status");
    assert_eq!(n["params"]["spoken"], "Claude");
    c.seen.clear();

    let id = c
        .request("turn.start", turn(1, "send continue to claude"))
        .await;
    let asked = c.note("confirm.requested").await;
    let _ = env.daemon.events.send(CoreEvent::AgentStatus {
        pane_id: "p1".to_string(),
        agent: "claude".to_string(),
        status: AgentStatus::Blocked,
    });
    c.drain(Duration::from_millis(300)).await;
    assert!(
        c.position(|m| m["method"] == "notice").is_none(),
        "held while the confirmation is pending: {:?}",
        c.seen
    );

    let confirm_id = asked["params"]["confirm_id"].as_u64().unwrap();
    c.call(
        "confirm.reply",
        json!({"confirm_id": confirm_id, "approve": true}),
    )
    .await;
    c.response(id).await;
    let n = c.note("notice").await;
    assert_eq!(n["params"]["text"], "Claude needs your input.", "{n}");
    let resolved = c.position(|m| m["method"] == "confirm.resolved").unwrap();
    let notice = c.position(|m| m["method"] == "notice").unwrap();
    assert!(resolved < notice, "{:?}", c.seen);
}

#[tokio::test]
async fn an_engine_error_is_a_spoken_error_outcome() {
    let env = start_with(
        confirming(),
        Arc::new(MockReasoningEngine::with_error("GEMINI_API_KEY not set")),
        false,
    )
    .await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("turn.start", turn(1, "hello")).await;
    assert_eq!(r["result"]["outcome"], "error", "{r}");
    assert!(r["result"]["detail"]
        .as_str()
        .unwrap()
        .contains("GEMINI_API_KEY"));
    let say = c.note("turn.say").await;
    assert_eq!(
        say["params"]["text"],
        "Sorry, I can't think right now: the server has no Gemini API key."
    );
    // The session is free again.
    let r = c.call("turn.start", turn(2, "hello")).await;
    assert_eq!(r["result"]["outcome"], "error", "{r}");
}

#[tokio::test]
async fn the_vocabulary_is_sent_again_when_it_changes() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    match env.daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            state.write().await.snapshot.workspaces[0].tabs[0].panes[0].cwd =
                Some("/src/libmorph".to_string());
        }
        _ => unreachable!(),
    }
    c.call("turn.start", turn(1, "hello")).await;
    let v = c.note("vocabulary.changed").await;
    let words = v["params"]["vocabulary"].as_array().unwrap();
    assert!(words.iter().any(|w| w == "libmorph"), "{v}");
}

/// Pair a companion over the link, as the phone would.
async fn pair_companion(env: &Env) -> Client {
    let (mut local, _) = Client::unix(&env.socket, "handlr.harness", "pair").await;
    let r = local
        .call("link.pair_begin", json!({"name": "phone"}))
        .await;
    let payload = PairingPayload::decode(r["result"]["pairing"].as_str().unwrap()).unwrap();
    let keypair = generate_keypair().unwrap();
    let (address, _) = env.link.as_ref().unwrap();
    let tcp = TcpStream::connect(address).await.unwrap();
    let hello = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": {"protocol": "handlr.companion", "version": 1, "client": "phone"}
    }))
    .unwrap();
    let (stream, _) = pair(
        tcp,
        &keypair.private,
        &payload.server_key,
        &payload.secret,
        Some(&hello),
    )
    .await
    .unwrap();
    let mut c = Client {
        wire: Wire::Link(stream),
        next_id: 0,
        seen: Vec::new(),
    };
    let hello = c.response(0).await;
    assert_eq!(hello["result"]["session_id"], "companion:phone", "{hello}");
    assert_eq!(hello["result"]["paired"], true, "{hello}");
    c
}

#[tokio::test]
async fn over_the_link_a_paired_phone_approves_its_own_turn() {
    let env = start_with(
        confirming(),
        Arc::new(MockReasoningEngine::with_handler(Script)),
        true,
    )
    .await;
    let mut phone = pair_companion(&env).await;
    let id = phone
        .request("turn.start", turn(1, "send continue to claude"))
        .await;
    let asked = phone.note("confirm.requested").await;
    assert_eq!(
        asked["params"]["text"],
        "Approve sending \"continue\" to Claude?"
    );
    // A paired phone may approve, so it is shown the token to display.
    assert!(asked["params"]["token"].is_string(), "{asked}");
    let confirm_id = asked["params"]["confirm_id"].as_u64().unwrap();
    phone
        .request(
            "confirm.reply",
            json!({"confirm_id": confirm_id, "approve": true}),
        )
        .await;
    let done = phone.response(id).await;
    assert_eq!(done["result"]["ran"][0]["command"], "continue", "{done}");
    assert_eq!(
        mock_logs(&env.daemon).await.0,
        vec![("p1".to_string(), "continue".to_string())]
    );

    // An interrupt over the link, within 500 ms.
    let id = phone.request("turn.start", turn(2, "slow")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let asked_at = Instant::now();
    phone.request("turn.interrupt", json!({"turn_id": 2})).await;
    let r = phone.response(id).await;
    assert_eq!(r["result"]["outcome"], "interrupted", "{r}");
    assert!(asked_at.elapsed() < Duration::from_millis(500));
    let h = history(&env, "companion:phone");
    assert_eq!(h.last().unwrap()["content"], INTERRUPTED_ENTRY);
}
