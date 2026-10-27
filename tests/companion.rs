//! `flankr.companion` and the turn service end to end (plan 20261022_02,
//! step 4, with the amendments of 12:06, 18:03 and 18:05 EDT), over the
//! herdr mock and a scripted mock engine. The mock's command and keystroke
//! logs stand in for the panes: nothing reaches a live herdr or Gemini.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use flankr::config::{AgentConfig, AppConfig};
use flankr::llm::{ClientFactory, ClientSpec, KeyEnv, LlmOptions, Profile};
use flankr::reasoning::{MockHandler, MockReasoningEngine, ReasoningEngine, ToolExecutor};
use flankr::server::companion::{INTERRUPTED_ENTRY, TurnOptions};
use flankr::server::link::{Link, LinkSettings};
use flankr::server::speech::SpeechOptions;
use flankr::server::{self, CoreEvent, Daemon, ServeOptions};
use flankr::sessions::Sessions;
use flankr::transport::{AgentSession, AgentStatus, HerdrClient, HerdrTransport};
use futures_util::StreamExt;
use libflankr_proto::link::{DeviceCode, LinkStream, PairingPayload, generate_keypair, reconnect};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpStream, UnixStream};
use tokio_util::codec::{FramedRead, LinesCodec};

const WAIT: Duration = Duration::from_secs(5);

/// The engine's script, keyed on the user's text:
/// * `send <command> to <target>`: `run_command`, then a short reply;
/// * `send <command> to <target> as <handle>`: the same, with that handle;
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
            let mut args = json!({"pane_id": target, "command": command});
            if let Some((target, handle)) = target.split_once(" as ") {
                args = json!({"pane_id": target, "command": command, "handle": handle});
            }
            let result = executor.execute_tool("run_command", &args).await?;
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

/// No card summaries (`[summary] cards`), so none reaches Gemini.
fn config(agents: Vec<AgentConfig>) -> AppConfig {
    AppConfig {
        agents,
        summary: Some(flankr::config::SummaryConfig { cards: false }),
        ..AppConfig::default()
    }
}

/// claude needs confirmation, as by default.
fn confirming() -> AppConfig {
    AppConfig {
        summary: Some(flankr::config::SummaryConfig { cards: false }),
        ..AppConfig::default()
    }
}

/// claude takes `continue` directly, with interrupt key `key`.
fn direct(key: &str) -> AppConfig {
    config(vec![
        AgentConfig::new("claude", vec!["continue".to_string()], false).with_interrupt_key(key),
    ])
}

async fn start_with(config: AppConfig, engine: Arc<dyn ReasoningEngine>, with_link: bool) -> Env {
    start_home(config, engine, with_link, &fixture_home()).await
}

/// As `start_with`, with the agents' sessions read under `home`.
async fn start_home(
    config: AppConfig,
    engine: Arc<dyn ReasoningEngine>,
    with_link: bool,
    home: &Path,
) -> Env {
    start_speech(config, engine, with_link, home, SpeechOptions::default()).await
}

/// As `start_home`, with `speech.summarize` set up by `speech`.
async fn start_speech(
    config: AppConfig,
    engine: Arc<dyn ReasoningEngine>,
    with_link: bool,
    home: &Path,
    speech: SpeechOptions,
) -> Env {
    let turns = TurnOptions {
        engine: Some(engine),
        speech,
        ..TurnOptions::default()
    };
    start_turns(config, with_link, home, turns).await
}

/// A daemon with `turns`, its histories under a temporary folder and the
/// agents' sessions under `home`.
async fn start_turns(config: AppConfig, with_link: bool, home: &Path, turns: TurnOptions) -> Env {
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
            session_dir: Some(sessions.clone()),
            // The synthetic fixtures, never this machine's own sessions.
            agent_sessions: Some(Arc::new(Sessions::with_home(home))),
            ..turns
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

/// The synthetic home of `tests/fixtures/sessions`.
fn fixture_home() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions/home")
}

/// Make the mock's pane p1 Claude Code in `/work/example`, with the
/// fixture session whose last reply asks a question.
async fn give_session(daemon: &Daemon) {
    match daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            let mut state = state.write().await;
            let pane = &mut state.snapshot.workspaces[0].tabs[0].panes[0];
            pane.cwd = Some("/work/example".to_string());
            pane.agent_session = Some(AgentSession {
                source: "herdr:claude".to_string(),
                agent: "claude".to_string(),
                kind: "id".to_string(),
                value: "00000000-0000-4000-8000-0000000000a1".to_string(),
            });
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
        assert_eq!(hello["result"]["server"], "flankr", "{hello}");
        (c, hello)
    }

    async fn companion(env: &Env) -> (Client, Value) {
        Client::unix(&env.socket, "flankr.companion", "t").await
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
    assert!(vocabulary.iter().any(|w| w == "flankr"), "{hello}");

    // Turns are a companion's: a harness connection does not have them.
    let (mut harness, _) = Client::unix(&env.socket, "flankr.harness", "h").await;
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
    // With no handle from the model, one is made from the command and the
    // agent, padded to three words with a BIP-39 word; it is not spoken.
    let handle = asked["params"]["handle"].as_str().unwrap();
    assert!(handle.starts_with("continue-claude-"), "{asked}");
    assert_eq!(handle.split('-').count(), 3, "{asked}");
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
async fn the_models_handle_reaches_the_companion_but_is_not_spoken() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let id = c
        .request(
            "turn.start",
            turn(1, "send continue to claude as Claude_Continue Send"),
        )
        .await;
    let asked = c.note("confirm.requested").await;
    assert_eq!(asked["params"]["handle"], "claude-continue-send", "{asked}");
    let text = asked["params"]["text"].as_str().unwrap();
    assert_eq!(text, "Approve sending \"continue\" to Claude?");
    assert!(!text.contains("For"), "{text}");
    let confirm_id = asked["params"]["confirm_id"].as_u64().unwrap();
    let r = c
        .call(
            "confirm.reply",
            json!({"confirm_id": confirm_id, "approve": true}),
        )
        .await;
    assert_eq!(r["result"]["outcome"], "sent", "{r}");
    assert_eq!(c.response(id).await["result"]["outcome"], "done");
    assert_eq!(
        mock_logs(&env.daemon).await.0,
        vec![("p1".to_string(), "continue".to_string())]
    );
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

    let (mut harness, _) = Client::unix(&env.socket, "flankr.harness", "h").await;
    let r = harness
        .call("agents.interrupt", json!({"target": "claude"}))
        .await;
    assert_eq!(r["result"]["interrupted"], false, "{r}");
    assert!(mock_logs(&env.daemon).await.1.is_empty());
}

#[tokio::test]
async fn agents_interrupt_from_a_harness_sends_the_key() {
    let env = start(confirming()).await;
    let (mut harness, _) = Client::unix(&env.socket, "flankr.harness", "mcp").await;
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
async fn a_sleeping_phone_is_told_only_what_needs_it() {
    let env = start(confirming()).await;
    env.daemon.watches.watch("p1");
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("client.options", json!({"background": true})).await;
    assert_eq!(r["result"], json!({}), "{r}");
    // Asleep: herdr's focus moving (to a second pane) is held back...
    match env.daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            let mut state = state.write().await;
            let mut shell = state.snapshot.workspaces[0].tabs[0].panes[0].clone();
            shell.pane_id = "p2".to_string();
            shell.extra.clear();
            shell.terminal_title = None;
            shell.focused = false;
            state.snapshot.workspaces[0].tabs[0].panes.push(shell);
        }
        _ => unreachable!("tests use the mock"),
    }
    c.seen.clear();
    let r = c.call("agents.focus", json!({"target_id": "p2"})).await;
    assert!(r.get("error").is_none(), "{r}");
    c.drain(Duration::from_millis(300)).await;
    assert!(
        c.position(|m| m["method"] == "focus.changed").is_none(),
        "focus is held while asleep: {:?}",
        c.seen
    );
    // ...but an agent that is done or blocked is told.
    let _ = env.daemon.events.send(CoreEvent::AgentStatus {
        pane_id: "p1".to_string(),
        agent: "claude".to_string(),
        status: AgentStatus::Done,
    });
    let n = c.note("notice").await;
    assert_eq!(n["params"]["text"], "Claude finished.", "{n}");
    // Awake again: the next focus check tells it where the focus went.
    c.call("client.options", json!({"background": false})).await;
    c.call("agents.focus", json!({"target_id": "p2"})).await;
    let moved = c.note("focus.changed").await;
    assert_eq!(moved["params"]["focus"]["pane_id"], "p2", "{moved}");
}

#[tokio::test]
async fn a_ping_says_whether_the_app_is_in_front() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    match env.daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            let mut state = state.write().await;
            let mut shell = state.snapshot.workspaces[0].tabs[0].panes[0].clone();
            shell.pane_id = "p2".to_string();
            shell.extra.clear();
            shell.terminal_title = None;
            shell.focused = false;
            state.snapshot.workspaces[0].tabs[0].panes.push(shell);
        }
        _ => unreachable!("tests use the mock"),
    }
    // Not active: asleep, so the focus moving is held.
    let r = c.call("ping", json!({"active": false})).await;
    assert_eq!(r["result"], json!({}), "{r}");
    c.seen.clear();
    c.call("agents.focus", json!({"target_id": "p2"})).await;
    c.drain(Duration::from_millis(300)).await;
    assert!(
        c.position(|m| m["method"] == "focus.changed").is_none(),
        "{:?}",
        c.seen
    );
    // Resumed: active, and told.
    c.call("ping", json!({"active": true})).await;
    c.call("agents.focus", json!({"target_id": "p2"})).await;
    let moved = c.note("focus.changed").await;
    assert_eq!(moved["params"]["focus"]["pane_id"], "p2", "{moved}");
    // A plain ping says nothing either way.
    let r = c.call("ping", json!({})).await;
    assert_eq!(r["result"], json!({}), "{r}");
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
async fn a_new_pending_question_gives_one_notice() {
    let env = start(confirming()).await;
    give_session(&env.daemon).await;
    env.daemon.watches.watch("p1");
    let (mut c, _) = Client::companion(&env).await;
    let status = |status: AgentStatus| {
        let _ = env.daemon.events.send(CoreEvent::AgentStatus {
            pane_id: "p1".to_string(),
            agent: "claude".to_string(),
            status,
        });
    };

    // Working, then idle on the question: one notice that asks it.
    status(AgentStatus::Working);
    status(AgentStatus::Idle);
    let n = c.note("notice").await;
    assert_eq!(n["params"]["kind"], "pending_question", "{n}");
    assert_eq!(
        n["params"]["text"],
        "Claude is asking: Should the notices wait for the phone to finish speaking, or interrupt it?"
    );
    assert_eq!(n["params"]["spoken"], "Claude");
    c.drain(Duration::from_millis(300)).await;
    let count = |c: &Client| c.seen.iter().filter(|m| m["method"] == "notice").count();
    assert_eq!(count(&c), 1, "{:?}", c.seen);

    // The same question again: no second notice.
    status(AgentStatus::Working);
    status(AgentStatus::Idle);
    c.drain(Duration::from_millis(300)).await;
    assert_eq!(count(&c), 1, "{:?}", c.seen);

    // Blocked on it again: only the plain status notice.
    status(AgentStatus::Working);
    status(AgentStatus::Blocked);
    c.drain(Duration::from_millis(300)).await;
    let notices: Vec<&Value> = c.seen.iter().filter(|m| m["method"] == "notice").collect();
    assert_eq!(notices.len(), 2, "{:?}", c.seen);
    assert_eq!(notices[1]["params"]["kind"], "agent_status");
    assert_eq!(notices[1]["params"]["text"], "Claude needs your input.");
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
    assert!(
        r["result"]["detail"]
            .as_str()
            .unwrap()
            .contains("GEMINI_API_KEY")
    );
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
async fn never_mind_is_answered_without_the_engine_and_kept_in_the_history() {
    // flankr's Gemini engine is wrapped the same way (plan 20261022_07,
    // decision 4); the mock stands in for Gemini.
    let inner = Arc::new(MockReasoningEngine::with_handler(Script));
    let engine = flankr::reasoning::with_flankr_routes(inner.clone());
    let env = start_with(confirming(), engine, false).await;
    let (mut c, _) = Client::companion(&env).await;

    let r = c.call("turn.start", turn(1, "Never mind.")).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
    assert_eq!(r["result"]["ran"], json!([]), "{r}");
    let say = c.note("turn.say").await;
    assert_eq!(say["params"]["text"], "Okay.");
    assert!(inner.recorded_transcripts().is_empty());
    assert!(mock_logs(&env.daemon).await.0.is_empty());

    // The routed reply is persisted like any other.
    let h = history(&env, "companion:local:t");
    assert_eq!(h.len(), 2, "{h:?}");
    assert_eq!(
        (&h[0]["role"], &h[0]["content"]),
        (&json!("user"), &json!("Never mind."))
    );
    assert_eq!(
        (&h[1]["role"], &h[1]["content"]),
        (&json!("model"), &json!("Okay."))
    );

    // Anything longer still reaches the engine.
    let r = c.call("turn.start", turn(2, "never mind the build")).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
    assert_eq!(inner.recorded_transcripts(), vec!["never mind the build"]);
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
    pair_as(env, "companion").await
}

/// Pair a device over the link for `protocol` (`companion` or `harness`),
/// and say hello with it.
async fn pair_as(env: &Env, protocol: &str) -> Client {
    let (mut local, _) = Client::unix(&env.socket, "flankr.harness", "pair").await;
    let r = local.call("link.pair_begin", json!({})).await;
    let payload = PairingPayload::decode(r["result"]["pairing"].as_str().unwrap()).unwrap();
    let keypair = generate_keypair().unwrap();
    let code = DeviceCode {
        key: keypair.public,
        name: "phone".to_string(),
    }
    .encode(&payload.server_key)
    .unwrap();
    let r = local
        .call(
            "link.pair_add",
            json!({"code": code, "protocols": [protocol]}),
        )
        .await;
    assert_eq!(r["result"]["name"], "phone", "{r}");
    let (address, _) = env.link.as_ref().unwrap();
    let tcp = TcpStream::connect(address).await.unwrap();
    let hello = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": {"protocol": format!("flankr.{protocol}"), "version": 1, "client": "phone"}
    }))
    .unwrap();
    let stream = reconnect(tcp, &keypair, &payload.server_key, Some(&hello))
        .await
        .unwrap();
    let mut c = Client {
        wire: Wire::Link(stream),
        next_id: 0,
        seen: Vec::new(),
    };
    let hello = c.response(0).await;
    if protocol == "companion" {
        assert_eq!(hello["result"]["session_id"], "companion:phone", "{hello}");
    }
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

// ── Dictated messages (plan 20261023_02, step 2) ─────────────────────────

const MESSAGE: &str = "Please run the tests, then tell me what failed? It's in the parser: yes";

fn message(pane: &str, text: &str) -> Value {
    json!({"pane_id": pane, "text": text, "kind": "message",
           "handle": "message-claude-flankr-please-run"})
}

#[tokio::test]
async fn a_message_is_confirmed_once_and_sent_as_dictated() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("agents.send", message("claude", MESSAGE)).await;
    assert_eq!(r["result"]["status"], "pending", "{r}");
    assert_eq!(r["result"]["handle"], "message-claude-flankr-please-run");
    let confirm_id = r["result"]["confirm_id"].as_u64().unwrap();

    // One question, naming the agent, with the full text for the card.
    let asked = c.note("confirm.requested").await;
    assert_eq!(asked["params"]["confirm_id"], confirm_id);
    assert_eq!(
        asked["params"]["text"], "Approve sending your message to Claude?",
        "{asked}"
    );
    assert_eq!(asked["params"]["command"], MESSAGE);
    assert_eq!(
        asked["params"]["handle"],
        "message-claude-flankr-please-run"
    );
    assert!(
        mock_logs(&env.daemon).await.0.is_empty(),
        "nothing sent yet"
    );
    c.drain(Duration::from_millis(200)).await;
    let questions = c
        .seen
        .iter()
        .filter(|m| m["method"] == "confirm.requested")
        .count();
    assert_eq!(questions, 1, "asked once");

    // The companion approves its own message, as the user confirming it.
    let r = c
        .call(
            "confirm.reply",
            json!({"confirm_id": confirm_id, "approve": true}),
        )
        .await;
    assert_eq!(r["result"]["outcome"], "sent", "{r}");
    assert_eq!(
        c.note("confirm.resolved").await["params"]["outcome"],
        "sent"
    );
    // The exact text, once, by herdr's text send, which ends with Enter;
    // no separate keystroke.
    let (commands, keys) = mock_logs(&env.daemon).await;
    assert_eq!(commands, vec![("p1".to_string(), MESSAGE.to_string())]);
    assert!(keys.is_empty(), "{keys:?}");
    let rpc = flankr::transport::HerdrRequest::run_command("p1", MESSAGE).to_jsonrpc();
    assert_eq!(rpc.method, "pane.send_input");
    assert_eq!(rpc.params["text"], MESSAGE);
    assert_eq!(rpc.params["keys"], json!(["Enter"]));

    // It cannot be approved twice.
    let r = c
        .call(
            "confirm.reply",
            json!({"confirm_id": confirm_id, "approve": true}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32006, "{r}");
    assert_eq!(mock_logs(&env.daemon).await.0.len(), 1);
}

#[tokio::test]
async fn a_denied_message_sends_nothing() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("agents.send", message("claude", MESSAGE)).await;
    let confirm_id = r["result"]["confirm_id"].as_u64().unwrap();
    let r = c
        .call(
            "confirm.reply",
            json!({"confirm_id": confirm_id, "approve": false}),
        )
        .await;
    assert_eq!(r["result"]["outcome"], "denied", "{r}");
    assert_eq!(
        c.note("confirm.resolved").await["params"]["outcome"],
        "denied"
    );
    assert!(mock_logs(&env.daemon).await.0.is_empty());
}

#[tokio::test]
async fn messages_are_always_confirmed_and_strictly_filtered() {
    // claude takes commands without confirmation here; a message still asks.
    let env = start(direct("esc")).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("agents.send", message("claude", "keep going")).await;
    assert_eq!(r["result"]["status"], "pending", "{r}");

    // A character outside the filter is refused, and named.
    let r = c
        .call("agents.send", message("claude", "look in src/main.rs"))
        .await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    let why = r["error"]["message"].as_str().unwrap();
    assert!(why.contains("'/'"), "{why}");
    let r = c.call("agents.send", message("claude", "one\ntwo")).await;
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("control characters"),
        "{r}"
    );
    let r = c
        .call("agents.send", message("claude", &"a".repeat(4001)))
        .await;
    assert!(
        r["error"]["message"].as_str().unwrap().contains("4000"),
        "{r}"
    );
    assert!(mock_logs(&env.daemon).await.0.is_empty());

    // `allow_messages = false` refuses them.
    let off = config(vec![
        AgentConfig::new("claude", vec!["continue".to_string()], true).with_messages(false),
    ]);
    let env = start(off).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("agents.send", message("claude", "keep going")).await;
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("allow_messages"),
        "{r}"
    );
}

#[tokio::test]
async fn agents_list_carries_the_focus_and_focus_changes_are_pushed() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("agents.list", json!({})).await;
    let focus = &r["result"]["focus"];
    assert_eq!(focus["pane_id"], "p1", "{r}");
    assert_eq!(focus["agent"], "Claude");
    assert_eq!(focus["name"], r["result"]["agents"][0]["name"]);
    assert_eq!(focus["spoken"], r["result"]["agents"][0]["spoken"]);

    // Focus moves to a pane with no agent: pushed after agents.focus.
    match env.daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            let mut state = state.write().await;
            let mut shell = state.snapshot.workspaces[0].tabs[0].panes[0].clone();
            shell.pane_id = "p2".to_string();
            shell.extra.clear();
            shell.terminal_title = None;
            shell.focused = false;
            state.snapshot.workspaces[0].tabs[0].panes.push(shell);
        }
        _ => unreachable!("tests use the mock"),
    }
    let r = c.call("agents.focus", json!({"target_id": "p2"})).await;
    assert!(r.get("error").is_none(), "{r}");
    let moved = c.note("focus.changed").await;
    assert_eq!(moved["params"]["focus"]["pane_id"], "p2", "{moved}");
    assert!(moved["params"]["focus"].get("agent").is_none(), "{moved}");
}

#[tokio::test]
async fn session_peek_gives_the_agent_view_its_entries_and_question() {
    let env = start(confirming()).await;
    give_session(&env.daemon).await;
    set_status(&env.daemon, AgentStatus::Idle).await;
    let (mut c, _) = Client::companion(&env).await;

    let r = c.call("session.peek", json!({"pane_id": "p1"})).await;
    let result = &r["result"];
    assert_eq!(result["status"], "idle", "{r}");
    let session = &result["session"];
    assert_eq!(
        session["session_id"],
        "00000000-0000-4000-8000-0000000000a1"
    );
    assert_eq!(session["matched"], "id");
    let entries = session["entries"].as_array().unwrap();
    // The fixture's whole conversation: the question, a tool call and its
    // result, and the reply ending in a question.
    let kinds: Vec<(&str, &str)> = entries
        .iter()
        .map(|e| (e["role"].as_str().unwrap(), e["kind"].as_str().unwrap()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("user", "text"),
            ("assistant", "tool_use"),
            ("tool", "tool_result"),
            ("assistant", "text"),
        ],
        "{r}"
    );
    assert_eq!(entries[1]["tool"], "Grep");
    assert_eq!(entries[2]["summary"], "src/notify.rs");
    assert_eq!(
        session["pending_question"]["text"],
        "Should the notices wait for the phone to finish speaking, or interrupt it?"
    );

    // Fewer entries: the newest. herdr says working, but the fixture has
    // not been written for long and ends on the reply: the turn visibly
    // ended, so the question is still pending (the not-quiet case is
    // `pending.rs`'s test).
    set_status(&env.daemon, AgentStatus::Working).await;
    let r = c
        .call("session.peek", json!({"pane_id": "p1", "entries": 2}))
        .await;
    let entries = r["result"]["session"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "{r}");
    assert_eq!(entries[0]["kind"], "tool_result");
    assert!(
        r["result"]["session"].get("pending_question").is_some(),
        "{r}"
    );
    assert_eq!(r["result"]["status"], "working");

    // The bounds of `entries`.
    for bad in [0, 201] {
        let r = c
            .call("session.peek", json!({"pane_id": "p1", "entries": bad}))
            .await;
        assert_eq!(r["error"]["code"], -32602, "{r}");
    }
    let r = c
        .call("session.peek", json!({"pane_id": "p1", "entries": 200}))
        .await;
    assert!(r.get("error").is_none(), "{r}");
}

/// Give the mock's pane the plan question fixture's session, in
/// `/work/plans`.
async fn give_plan_session(daemon: &Daemon) {
    match daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            let mut state = state.write().await;
            let pane = &mut state.snapshot.workspaces[0].tabs[0].panes[0];
            pane.cwd = Some("/work/plans".to_string());
            pane.agent_session = Some(AgentSession {
                source: "herdr:claude".to_string(),
                agent: "claude".to_string(),
                kind: "id".to_string(),
                value: "00000000-0000-4000-8000-0000000000e5".to_string(),
            });
        }
        _ => unreachable!("tests use the mock"),
    }
}

#[tokio::test]
async fn a_plan_question_is_marked_and_its_paths_and_urls_link() {
    // Plan 20261022_03, the 21:57 EDT amendment.
    let mut cfg = confirming();
    cfg.urls.repowiki_url = Some("http://wiki:8088/".to_string());
    let env = start(cfg).await;
    give_plan_session(&env.daemon).await;
    set_status(&env.daemon, AgentStatus::Idle).await;
    let (mut c, _) = Client::companion(&env).await;

    let r = c.call("session.peek", json!({"pane_id": "p1"})).await;
    let session = &r["result"]["session"];
    let q = &session["pending_question"];
    assert_eq!(q["about_plan"], true, "{r}");
    assert_eq!(q["plan"], "the overview screen plan");
    let plan = "docs/design/20261023_04_OVERVIEW_SCREEN_PLAN.md";
    let plan_url = "http://wiki:8088/#/work--plans/docs/design/20261023_04_OVERVIEW_SCREEN_PLAN";
    let web = json!({"text": "https://example.com/spec", "url": "https://example.com/spec"});
    assert_eq!(
        q["links"],
        json!([{"text": plan, "url": plan_url}, web.clone()]),
        "{r}"
    );
    let reply = session["entries"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(
        reply["links"],
        json!([
            {"text": "docs/FONT_X.md", "url": "http://wiki:8088/#/work--plans/docs/FONT_X"},
            {"text": plan, "url": plan_url},
            web.clone(),
        ]),
        "{reply}"
    );
    // The user's entry names no path: no links on the wire.
    assert!(session["entries"][0].get("links").is_none(), "{r}");

    // agents.list's question carries them too.
    let r = c.call("agents.list", json!({})).await;
    let q = &r["result"]["agents"][0]["session"]["pending_question"];
    assert_eq!(q["links"][0]["url"], plan_url, "{r}");

    // Without `[urls] repowiki_url`, the question is still about the plan, and
    // only its web link links.
    let env = start(confirming()).await;
    give_plan_session(&env.daemon).await;
    set_status(&env.daemon, AgentStatus::Idle).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("session.peek", json!({"pane_id": "p1"})).await;
    let session = &r["result"]["session"];
    assert_eq!(session["pending_question"]["about_plan"], true, "{r}");
    assert_eq!(session["pending_question"]["links"], json!([web]), "{r}");
    let entries = session["entries"].as_array().unwrap();
    assert!(entries[0].get("links").is_none(), "{r}");
    assert_eq!(entries.last().unwrap()["links"], json!([web]), "{r}");
}

#[tokio::test]
async fn a_pane_without_a_session_peeks_as_null() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    // p1 has no cwd and no session id: no file is found.
    let r = c.call("session.peek", json!({"pane_id": "p1"})).await;
    assert_eq!(r["result"]["session"], Value::Null, "{r}");
    assert_eq!(r["result"]["status"], "working");
    let r = c.call("session.peek", json!({"pane_id": "nowhere"})).await;
    assert_eq!(r["error"]["code"], -32002, "{r}");
}

#[tokio::test]
async fn agents_list_carries_each_agents_session() {
    let env = start(confirming()).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("agents.list", json!({})).await;
    assert!(r["result"]["agents"][0].get("session").is_none(), "{r}");

    give_session(&env.daemon).await;
    set_status(&env.daemon, AgentStatus::Blocked).await;
    let r = c.call("agents.list", json!({})).await;
    let session = &r["result"]["agents"][0]["session"];
    assert_eq!(session["matched"], "id", "{r}");
    assert_eq!(
        session["session_id"],
        "00000000-0000-4000-8000-0000000000a1"
    );
    assert_eq!(
        session["pending_question"]["text"],
        "Should the notices wait for the phone to finish speaking, or interrupt it?"
    );
    assert!(session.get("entries").is_none(), "{r}");
    // The project is the directory, as named, not herdr's workspace label.
    assert_eq!(r["result"]["agents"][0]["project"], "example", "{r}");
}

/// Write p1's session (see `give_session`) under `home`: a user line, then
/// `reply` from the agent, then `more` if given; written `age` ago.
fn write_turn(home: &Path, reply: &str, more: Option<Value>, age: Duration) {
    let project = home.join(".claude/projects/-work-example");
    std::fs::create_dir_all(&project).unwrap();
    let user = json!({
        "type": "user", "cwd": "/work/example", "timestamp": "2026-01-02T12:00:00.000Z",
        "message": {"role": "user", "content": "Go on."}
    });
    let reply = json!({
        "type": "assistant", "cwd": "/work/example", "timestamp": "2026-01-02T12:00:05.000Z",
        "message": {"role": "assistant", "content": [{"type": "text", "text": reply}]}
    });
    let mut body = format!("{user}\n{reply}\n");
    if let Some(more) = more {
        body.push_str(&format!("{more}\n"));
    }
    let path = project.join("00000000-0000-4000-8000-0000000000a1.jsonl");
    std::fs::write(&path, body).unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(std::time::SystemTime::now() - age)
        .unwrap();
}

#[tokio::test]
async fn agents_list_gives_each_agents_turn_from_its_session() {
    // Plan 20261023_04, step 1 as amended (A4).
    let home = tempfile::tempdir().unwrap();
    let quiet = Duration::from_secs(60);
    write_turn(
        home.path(),
        "Built it.\n\nInstall it on the S23?",
        None,
        quiet,
    );
    let env = start_home(
        confirming(),
        Arc::new(MockReasoningEngine::with_handler(Script)),
        false,
        home.path(),
    )
    .await;
    give_session(&env.daemon).await;
    // p2: a shell beside it.
    match env.daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            let mut state = state.write().await;
            let mut shell = state.snapshot.workspaces[0].tabs[0].panes[0].clone();
            shell.pane_id = "p2".to_string();
            shell.extra.clear();
            shell.terminal_title = None;
            shell.agent_session = None;
            shell.focused = false;
            state.snapshot.workspaces[0].tabs[0].panes.push(shell);
        }
        _ => unreachable!("tests use the mock"),
    }
    set_status(&env.daemon, AgentStatus::Working).await;
    let (mut c, _) = Client::companion(&env).await;
    let turn_of = |r: &Value, pane: &str| {
        r["result"]["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["pane_id"] == pane)
            .unwrap_or_else(|| panic!("{pane} in {r}"))
            .get("turn")
            .cloned()
    };

    // Asking: the question, though herdr says working.
    let r = c.call("agents.list", json!({})).await;
    assert_eq!(
        turn_of(&r, "p1"),
        Some(json!({"state": "asking", "question": "Install it on the S23?"})),
        "{r}"
    );
    // No session: a shell has no turn.
    let shell = r["result"]["agents"][1].clone();
    assert_eq!(shell["agent"], "shell", "{r}");
    assert_eq!(turn_of(&r, "p2"), None, "{r}");

    // Stopped: a quiet reply with no question, herdr still saying working.
    write_turn(home.path(), "All tests pass.\n\nCommitted.", None, quiet);
    let r = c.call("agents.list", json!({})).await;
    assert_eq!(
        turn_of(&r, "p1"),
        Some(json!({"state": "stopped", "line": "All tests pass."})),
        "{r}"
    );
    assert_eq!(r["result"]["agents"][0]["status"], "working");

    // Working: the file was just written, with a tool call after the
    // reply.
    let call = json!({
        "type": "assistant", "cwd": "/work/example", "timestamp": "2026-01-02T12:00:06.000Z",
        "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_fake_09", "name": "Bash",
             "input": {"command": "cargo test"}}
        ]}
    });
    write_turn(
        home.path(),
        "Running the tests.",
        Some(call),
        Duration::ZERO,
    );
    let r = c.call("agents.list", json!({})).await;
    let turn = turn_of(&r, "p1").unwrap();
    assert_eq!(turn["state"], "working", "{r}");
    assert!(turn["line"].as_str().unwrap().contains("cargo test"), "{r}");
}

#[tokio::test]
async fn session_peek_is_for_companions_and_local_clients_only() {
    let env = start_with(
        confirming(),
        Arc::new(MockReasoningEngine::with_handler(Script)),
        true,
    )
    .await;
    give_session(&env.daemon).await;
    // A local harness client may peek.
    let (mut local, _) = Client::unix(&env.socket, "flankr.harness", "ctl").await;
    let r = local.call("session.peek", json!({"pane_id": "p1"})).await;
    assert_eq!(r["result"]["session"]["matched"], "id", "{r}");
    // A paired companion may.
    let mut phone = pair_companion(&env).await;
    let r = phone.call("session.peek", json!({"pane_id": "p1"})).await;
    assert_eq!(r["result"]["session"]["matched"], "id", "{r}");
}

#[tokio::test]
async fn a_paired_harness_may_not_peek_yet() {
    let env = start_with(
        confirming(),
        Arc::new(MockReasoningEngine::with_handler(Script)),
        true,
    )
    .await;
    give_session(&env.daemon).await;
    let mut harness = pair_as(&env, "harness").await;
    let r = harness.call("session.peek", json!({"pane_id": "p1"})).await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
}

#[tokio::test]
async fn peeked_entries_are_redacted() {
    // A home whose session reads a `.private/` file and a `.env` file.
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join(".claude/projects/-work-example");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sessions/claude_secret.jsonl"),
        project.join("00000000-0000-4000-8000-0000000000a1.jsonl"),
    )
    .unwrap();
    let env = start_home(
        confirming(),
        Arc::new(MockReasoningEngine::with_handler(Script)),
        false,
        home.path(),
    )
    .await;
    give_session(&env.daemon).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c
        .call("session.peek", json!({"pane_id": "p1", "entries": 200}))
        .await;
    let text = r["result"]["session"]["entries"].to_string();
    assert!(!text.is_empty() && text != "null", "{r}");
    assert!(!text.contains("not-a-real-secret-value"), "{text}");
    assert!(!text.contains("not-a-real-token"), "{text}");
    assert!(text.contains("[redacted]"), "{text}");
}

// speech.summarize (plan 20261023_05, step 3, as amended at 15:45 EDT, and
// step 8, as amended at 16:28 EDT: Gemini only with spoken output on).

/// The time of the reply `speech_home` writes.
const REPLY_TS: &str = "2026-01-02T12:00:05.000Z";

/// A reply that is complex after the reading rules: a list of changes.
const COMPLEX: &str = "I changed three files:\n- src/alpha.rs\n- src/beta.rs\n- src/gamma.rs";
/// `COMPLEX` under the reading rules.
const COMPLEX_READ: &str = "I changed three files: the alpha the beta the gamma";

/// A home whose session for p1 (see `give_session`) ends in an assistant
/// reply of `text`, at `REPLY_TS`.
fn speech_home(text: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join(".claude/projects/-work-example");
    std::fs::create_dir_all(&project).unwrap();
    let user = json!({
        "type": "user", "cwd": "/work/example", "timestamp": "2026-01-02T12:00:00.000Z",
        "message": {"role": "user", "content": "What changed?"}
    });
    let reply = json!({
        "type": "assistant", "cwd": "/work/example", "timestamp": REPLY_TS,
        "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}
    });
    std::fs::write(
        project.join("00000000-0000-4000-8000-0000000000a1.jsonl"),
        format!("{user}\n{reply}\n"),
    )
    .unwrap();
    home
}

/// A daemon reading `home`, whose summaries come from `engine` within
/// `timeout`.
async fn start_summaries(home: &Path, engine: Arc<dyn ReasoningEngine>, timeout: Duration) -> Env {
    let env = start_speech(
        confirming(),
        Arc::new(MockReasoningEngine::with_handler(Script)),
        false,
        home,
        SpeechOptions {
            engine: Some(engine),
            timeout: Some(timeout),
            ..SpeechOptions::default()
        },
    )
    .await;
    give_session(&env.daemon).await;
    env
}

/// A companion with spoken output on (`client.options`).
async fn speaking_companion(env: &Env) -> Client {
    let (mut c, _) = Client::companion(env).await;
    let r = c
        .call("client.options", json!({"spoken_output": true}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    c
}

/// Synthetic: the right shape for a Google key, made-up characters.
const FAKE_KEY: &str = "AIzaSyD0nTuSeTh1sFakeKeyAbCdEfGhIjKlMn";

#[tokio::test]
async fn the_whole_reply_reaches_the_engine_redacted_and_is_summarised_once() {
    // Past 500 characters, with a list, a key and a closing sentence that
    // the client's 500-character summary would not have.
    let reply = format!(
        "I changed `src/server/speech.rs` and set the key to {FAKE_KEY}.\n\
         - one\n- two\n- three\n{} The very last sentence.",
        "The rest of the change keeps every reply in order. ".repeat(12)
    );
    assert!(reply.chars().count() > 600);
    let home = speech_home(&reply);
    let mock = Arc::new(MockReasoningEngine::with_response(
        "The speech module changed.",
    ));
    let env = start_summaries(home.path(), mock.clone(), WAIT).await;
    let mut c = speaking_companion(&env).await;

    let params = json!({"pane_id": "p1", "ts": REPLY_TS, "text": "cut off…"});
    let r = c.call("speech.summarize", params.clone()).await;
    assert_eq!(r["result"]["complex"], true, "{r}");
    assert_eq!(r["result"]["summary"], "The speech module changed.");
    assert!(
        r["result"]["read"].as_str().is_some_and(|t| !t.is_empty()),
        "{r}"
    );
    let sent = mock.recorded_transcripts();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].starts_with("The Claude agent in "), "{}", sent[0]);
    assert!(sent[0].ends_with("The very last sentence."), "{}", sent[0]);
    assert!(!sent[0].contains(FAKE_KEY), "{}", sent[0]);
    assert!(sent[0].contains("[redacted]"));
    assert!(!sent[0].contains("cut off"));

    // The same reply again: the kept answer, and no second call.
    let again = c.call("speech.summarize", params).await;
    assert_eq!(again["result"], r["result"]);
    assert_eq!(mock.recorded_transcripts().len(), 1);
}

#[tokio::test]
async fn a_plain_reply_is_read_through_the_rules() {
    let home = speech_home("I ran `cargo test` and they all pass. Shall I go on?");
    let mock = Arc::new(MockReasoningEngine::with_response("unused"));
    let env = start_summaries(home.path(), mock.clone(), WAIT).await;
    let mut c = speaking_companion(&env).await;
    let r = c
        .call(
            "speech.summarize",
            json!({"pane_id": "p1", "ts": REPLY_TS, "text": "I ran the tests"}),
        )
        .await;
    assert_eq!(
        r["result"],
        json!({"read": "I ran cargo test and they all pass. Shall I go on?", "complex": false}),
        "{r}"
    );
    assert!(mock.recorded_transcripts().is_empty());
}

#[tokio::test]
async fn a_complex_reply_calls_the_engine_only_with_spoken_output_on() {
    let home = speech_home(COMPLEX);
    let mock = Arc::new(MockReasoningEngine::with_response("Three files changed."));
    let env = start_summaries(home.path(), mock.clone(), WAIT).await;
    let params = json!({"pane_id": "p1", "ts": REPLY_TS});

    // A new connection starts with spoken output off: the rules, no call.
    let (mut c, _) = Client::companion(&env).await;
    let r = c.call("speech.summarize", params.clone()).await;
    assert_eq!(
        r["result"],
        json!({"read": COMPLEX_READ, "complex": true}),
        "{r}"
    );
    assert!(mock.recorded_transcripts().is_empty());

    // On: one call.
    let r = c
        .call("client.options", json!({"spoken_output": true, "later": 1}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let r = c.call("speech.summarize", params.clone()).await;
    assert_eq!(
        r["result"],
        json!({"read": COMPLEX_READ, "summary": "Three files changed.", "complex": true}),
        "{r}"
    );
    assert_eq!(mock.recorded_transcripts().len(), 1);

    // Off again: the rules, though a summary is kept.
    c.call("client.options", json!({"spoken_output": false}))
        .await;
    let r = c.call("speech.summarize", params.clone()).await;
    assert_eq!(
        r["result"],
        json!({"read": COMPLEX_READ, "complex": true}),
        "{r}"
    );

    // Another connection of the same daemon starts off too.
    let (mut other, _) = Client::companion(&env).await;
    let r = other.call("speech.summarize", params).await;
    assert_eq!(r["result"]["read"], COMPLEX_READ, "{r}");
    assert!(r["result"].get("summary").is_none(), "{r}");
    assert_eq!(mock.recorded_transcripts().len(), 1);

    // client.options is for companions.
    let (mut local, _) = Client::unix(&env.socket, "flankr.harness", "ctl").await;
    let r = local
        .call("client.options", json!({"spoken_output": true}))
        .await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
}

#[tokio::test]
async fn a_reply_not_found_is_measured_from_the_clients_text() {
    let home = speech_home("Plain.");
    let mock = Arc::new(MockReasoningEngine::with_response("Summary."));
    let env = start_summaries(home.path(), mock.clone(), WAIT).await;
    let mut c = speaking_companion(&env).await;
    let r = c
        .call(
            "speech.summarize",
            json!({"pane_id": "p1", "ts": "2026-01-02T13:00:00.000Z", "text": COMPLEX}),
        )
        .await;
    assert_eq!(r["result"]["summary"], "Summary.", "{r}");
    assert!(mock.recorded_transcripts()[0].ends_with("- src/gamma.rs"));
}

/// A summary that never comes in time.
struct Never;

#[async_trait]
impl MockHandler for Never {
    async fn handle(&self, _: &str, _: &dyn ToolExecutor) -> Result<Option<String>, String> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(Some("too late".to_string()))
    }
}

#[tokio::test]
async fn a_summary_that_times_out_leaves_the_rules_version() {
    let home = speech_home(COMPLEX);
    let env = start_summaries(
        home.path(),
        Arc::new(MockReasoningEngine::with_handler(Never)),
        Duration::from_millis(200),
    )
    .await;
    let mut c = speaking_companion(&env).await;
    let started = Instant::now();
    let r = c
        .call("speech.summarize", json!({"pane_id": "p1", "ts": REPLY_TS}))
        .await;
    assert_eq!(r["result"]["read"], COMPLEX_READ, "{r}");
    assert!(r["result"].get("summary").is_none(), "{r}");
    assert!(started.elapsed() < Duration::from_secs(5));
    // An engine error leaves it too.
    let env = start_summaries(
        home.path(),
        Arc::new(MockReasoningEngine::with_error("no key")),
        WAIT,
    )
    .await;
    let mut c = speaking_companion(&env).await;
    let r = c
        .call("speech.summarize", json!({"pane_id": "p1", "ts": REPLY_TS}))
        .await;
    assert_eq!(r["result"]["read"], COMPLEX_READ, "{r}");
    assert!(r["result"].get("summary").is_none(), "{r}");
}

#[tokio::test]
async fn summaries_are_for_companions_and_agent_panes_only() {
    let home = speech_home("Changed `x`.");
    let env = start_summaries(
        home.path(),
        Arc::new(MockReasoningEngine::with_response("Summary.")),
        WAIT,
    )
    .await;
    // A shell pane: no agent.
    match env.daemon.client.transport() {
        HerdrTransport::Mock { state } => {
            let mut state = state.write().await;
            let mut shell = state.snapshot.workspaces[0].tabs[0].panes[0].clone();
            shell.pane_id = "p2".to_string();
            shell.extra.clear();
            shell.terminal_title = None;
            shell.agent_session = None;
            shell.focused = false;
            state.snapshot.workspaces[0].tabs[0].panes.push(shell);
        }
        _ => unreachable!("tests use the mock"),
    }
    let (mut c, _) = Client::companion(&env).await;
    let r = c
        .call("speech.summarize", json!({"pane_id": "p2", "ts": REPLY_TS}))
        .await;
    assert_eq!(r["error"]["code"], -32002, "{r}");
    let r = c
        .call(
            "speech.summarize",
            json!({"pane_id": "nowhere", "ts": REPLY_TS}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32002, "{r}");
    // A harness client may not.
    let (mut local, _) = Client::unix(&env.socket, "flankr.harness", "ctl").await;
    let r = local
        .call("speech.summarize", json!({"pane_id": "p1", "ts": REPLY_TS}))
        .await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
}

/// A summary that takes a moment.
struct Slow;

#[async_trait]
impl MockHandler for Slow {
    async fn handle(&self, _: &str, _: &dyn ToolExecutor) -> Result<Option<String>, String> {
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ok(Some("Summary.".to_string()))
    }
}

#[tokio::test]
async fn a_second_summary_on_a_connection_waits_for_the_first() {
    let home = speech_home(COMPLEX);
    let mock = Arc::new(MockReasoningEngine::with_handler(Slow));
    let env = start_summaries(home.path(), mock.clone(), WAIT).await;
    let mut c = speaking_companion(&env).await;
    // Both at once, for the same reply: the second waits, then finds the
    // first's answer kept.
    let params = json!({"pane_id": "p1", "ts": REPLY_TS});
    let first = c.request("speech.summarize", params.clone()).await;
    let second = c.request("speech.summarize", params).await;
    let a = c.response(first).await;
    let b = c.response(second).await;
    assert_eq!(a["result"]["summary"], "Summary.", "{a}");
    assert_eq!(b["result"], a["result"]);
    assert_eq!(mock.recorded_transcripts().len(), 1);
}

// client.options general_agent (plan 20261024_02, decision 7 and step 3,
// as amended at 11:37 EDT: only the turns the user starts stop).

#[tokio::test]
async fn with_the_general_agent_off_no_gemini_turn_starts() {
    let mock = Arc::new(MockReasoningEngine::with_response("Hello."));
    let env = start_with(confirming(), mock.clone(), false).await;
    let (mut c, _) = Client::companion(&env).await;

    let r = c
        .call("client.options", json!({"general_agent": false}))
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let r = c.call("turn.start", turn(1, "hello")).await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    assert!(mock.recorded_transcripts().is_empty());

    // Options without the field leave it on, as a new connection has it.
    c.call("client.options", json!({"spoken_output": false}))
        .await;
    let r = c.call("turn.start", turn(2, "hello")).await;
    assert!(r["result"]["outcome"].is_string(), "{r}");
    assert_eq!(mock.recorded_transcripts().len(), 1);

    // Off on one connection is off for that one only.
    c.call("client.options", json!({"general_agent": false}))
        .await;
    let (mut other, _) = Client::companion(&env).await;
    let r = other.call("turn.start", turn(3, "hello")).await;
    assert!(r["result"]["outcome"].is_string(), "{r}");
}

#[tokio::test]
async fn with_the_general_agent_off_spoken_summaries_go_on() {
    let home = speech_home(COMPLEX);
    let mock = Arc::new(MockReasoningEngine::with_response("Three files changed."));
    let env = start_summaries(home.path(), mock.clone(), WAIT).await;
    let (mut c, _) = Client::companion(&env).await;
    let r = c
        .call(
            "client.options",
            json!({"spoken_output": true, "general_agent": false}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let r = c
        .call("speech.summarize", json!({"pane_id": "p1", "ts": REPLY_TS}))
        .await;
    assert_eq!(r["result"]["summary"], "Three files changed.", "{r}");
    assert_eq!(mock.recorded_transcripts().len(), 1);
}

// Plan 20261027_02, step 2: the daemon on `[llm]`, with Gemini mocked by a
// factory that answers with the backend and model it was built for.

/// Builds mocks: the general agent's runs `Script` (and `wait`, which
/// takes 800 ms), a summary's answers with its backend and model.
#[derive(Default)]
struct Factory {
    built: std::sync::Mutex<Vec<(String, String, String)>>,
}

/// `wait`: a turn that takes 800 ms, then says which client ran it.
struct Labelled(String);

#[async_trait]
impl MockHandler for Labelled {
    async fn handle(
        &self,
        text: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        if text == "wait" {
            tokio::time::sleep(Duration::from_millis(800)).await;
            return Ok(Some(format!("Waited on {}.", self.0)));
        }
        let said = Script.handle(text, executor).await?;
        Ok(said.map(|s| format!("{s} ({})", self.0)))
    }
}

impl ClientFactory for Factory {
    fn build(&self, spec: &ClientSpec<'_>) -> Result<Arc<dyn ReasoningEngine>, String> {
        let label = format!("{}/{}", spec.target.backend, spec.target.model);
        self.built.lock().unwrap().push((
            spec.use_.name().to_string(),
            spec.target.backend.clone(),
            spec.target.model.clone(),
        ));
        Ok(match spec.profile {
            Profile::Agent => Arc::new(MockReasoningEngine::with_handler(Labelled(label))),
            Profile::Summary => Arc::new(MockReasoningEngine::with_response(label)),
        })
    }
}

struct LlmEnv {
    env: Env,
    /// The daemon's working folder, with the config and the key.
    folder: tempfile::TempDir,
    config: PathBuf,
    factory: Arc<Factory>,
    _home: tempfile::TempDir,
}

fn write_key(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "not-a-real-key").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

/// Rewrites the config so its stamp changes.
fn rewrite(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(std::time::SystemTime::now() + Duration::from_secs(2))
        .unwrap();
}

/// A daemon whose `[llm]` is `text`, watched every 100 ms, with a key when
/// `key`, the factory's clients and no engine given.
async fn start_llm(text: &str, key: bool) -> LlmEnv {
    let folder = tempfile::tempdir().unwrap();
    if key {
        write_key(&folder.path().join(".private/gemini_api_key"));
    }
    let config_path = folder.path().join("config.toml");
    std::fs::write(&config_path, text).unwrap();
    let config: AppConfig = toml::from_str(text).unwrap();
    let factory = Arc::new(Factory::default());
    let home = speech_home(COMPLEX);
    let turns = TurnOptions {
        llm: LlmOptions {
            config_path: Some(config_path.clone()),
            key_env: Some(KeyEnv::in_dir(folder.path())),
            factory: Some(factory.clone()),
            interval: Some(Duration::from_millis(100)),
            prompts_file: None,
        },
        ..TurnOptions::default()
    };
    let env = start_turns(config, false, home.path(), turns).await;
    give_session(&env.daemon).await;
    LlmEnv {
        env,
        folder,
        config: config_path,
        factory,
        _home: home,
    }
}

const TWO_BACKENDS: &str = r#"
[llm]
enabled = true
default = "main"
[llm.backends.main]
kind = "gemini"
[llm.backends.cheap]
kind = "gemini"
model = "gemini-3.8-flash-lite"
[llm.uses.spoken_summaries]
backend = "cheap"
[llm.uses.card_summaries]
enabled = false
"#;

#[tokio::test]
async fn with_llm_off_hello_says_so_and_turns_are_refused() {
    let e = start_llm("", true).await;
    let (mut c, hello) = Client::companion(&e.env).await;
    let llm = &hello["result"]["llm"];
    assert_eq!(llm["general_agent"], false, "{hello}");
    assert_eq!(llm["summaries"], false, "{hello}");
    assert!(
        llm["reason"].as_str().unwrap().contains("[llm] enabled"),
        "{hello}"
    );
    let r = c.call("turn.start", turn(1, "hello")).await;
    assert_eq!(r["error"]["code"], -32016, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("[llm] enabled"),
        "{r}"
    );
    // Read aloud goes by the rules, and nothing was built.
    c.call("client.options", json!({"spoken_output": true}))
        .await;
    let r = c
        .call("speech.summarize", json!({"pane_id": "p1", "ts": REPLY_TS}))
        .await;
    assert_eq!(
        r["result"],
        json!({"read": COMPLEX_READ, "complex": true}),
        "{r}"
    );
    assert!(e.factory.built.lock().unwrap().is_empty());
}

#[tokio::test]
async fn each_use_runs_on_its_own_backend_and_model() {
    let e = start_llm(TWO_BACKENDS, true).await;
    let (mut c, hello) = Client::companion(&e.env).await;
    assert_eq!(
        hello["result"]["llm"],
        json!({"general_agent": true, "summaries": true}),
        "{hello}"
    );
    let r = c.call("turn.start", turn(1, "hello")).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
    let say = c.note("turn.say").await;
    assert_eq!(
        say["params"]["text"], "You said hello. (main/gemini-3.8-flash)",
        "{say}"
    );
    c.call("client.options", json!({"spoken_output": true}))
        .await;
    let r = c
        .call("speech.summarize", json!({"pane_id": "p1", "ts": REPLY_TS}))
        .await;
    assert_eq!(r["result"]["summary"], "cheap/gemini-3.8-flash-lite", "{r}");
    let built = e.factory.built.lock().unwrap().clone();
    assert_eq!(
        built,
        [
            (
                "general_agent".to_string(),
                "main".to_string(),
                "gemini-3.8-flash".to_string()
            ),
            (
                "spoken_summaries".to_string(),
                "cheap".to_string(),
                "gemini-3.8-flash-lite".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn a_reload_turns_the_general_agent_off_for_the_next_turn_while_a_running_one_finishes() {
    let e = start_llm("[llm]\nenabled = true\n", true).await;
    let (mut c, _) = Client::companion(&e.env).await;
    let running = c.request("turn.start", turn(1, "wait")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    rewrite(
        &e.config,
        "[llm]\nenabled = true\n[llm.uses.general_agent]\nenabled = false\n",
    );
    // The phone is told, without reconnecting.
    let caps = c.note("server.capabilities").await;
    assert_eq!(caps["params"]["llm"]["general_agent"], false, "{caps}");
    assert_eq!(caps["params"]["llm"]["summaries"], true, "{caps}");
    assert!(
        caps["params"]["llm"]["reason"]
            .as_str()
            .unwrap()
            .contains("[llm.uses.general_agent] enabled = false"),
        "{caps}"
    );
    // The running turn finishes on its client.
    let r = c.response(running).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
    let say = c.note("turn.say").await;
    assert_eq!(say["params"]["text"], "Waited on gemini/gemini-3.8-flash.");
    // The next one is refused.
    let r = c.call("turn.start", turn(2, "hello")).await;
    assert_eq!(r["error"]["code"], -32016, "{r}");
    // A new connection's hello says so too.
    let (_other, hello) = Client::companion(&e.env).await;
    assert_eq!(hello["result"]["llm"]["general_agent"], false, "{hello}");
}

#[tokio::test]
async fn a_key_file_appearing_makes_the_uses_available() {
    let e = start_llm("[llm]\nenabled = true\n", false).await;
    let (mut c, hello) = Client::companion(&e.env).await;
    assert_eq!(hello["result"]["llm"]["general_agent"], false, "{hello}");
    assert!(
        hello["result"]["llm"]["reason"]
            .as_str()
            .unwrap()
            .contains("no Gemini key"),
        "{hello}"
    );
    let r = c.call("turn.start", turn(1, "hello")).await;
    assert_eq!(r["error"]["code"], -32016, "{r}");
    write_key(&e.folder.path().join(".private/gemini_api_key"));
    let caps = c.note("server.capabilities").await;
    assert_eq!(
        caps["params"]["llm"],
        json!({"general_agent": true, "summaries": true}),
        "{caps}"
    );
    let r = c.call("turn.start", turn(2, "hello")).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
}

#[tokio::test]
async fn a_broken_llm_section_keeps_the_last_good_settings() {
    let e = start_llm("[llm]\nenabled = true\n", true).await;
    let (mut c, _) = Client::companion(&e.env).await;
    rewrite(&e.config, "[llm]\nenabled = maybe\n");
    // Several checks pass; nothing changes.
    c.drain(Duration::from_millis(400)).await;
    assert!(
        c.position(|m| m["method"] == "server.capabilities")
            .is_none()
    );
    let r = c.call("turn.start", turn(1, "hello")).await;
    assert_eq!(r["result"]["outcome"], "done", "{r}");
}
