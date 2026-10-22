//! `handlr serve` end to end, on a temporary socket, over the herdr mock
//! (plan 20261022_01, step 5). The mock's command log stands in for the
//! panes, so nothing is typed into a real session.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use handlr::config::AppConfig;
use handlr::core::confirm::{nonce, ConfirmEvent};
use handlr::core::principal::Principal;
use handlr::server::{self, Daemon, ServeOptions};
use handlr::transport::{HerdrClient, HerdrTransport};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::UnixStream;
use tokio_util::codec::{FramedRead, LinesCodec};

struct Client {
    write: OwnedWriteHalf,
    lines: FramedRead<tokio::net::unix::OwnedReadHalf, LinesCodec>,
    next_id: i64,
    /// Notifications that arrived while waiting for a response.
    notes: Vec<Value>,
}

impl Client {
    async fn connect(socket: &Path) -> Client {
        let (read, write) = UnixStream::connect(socket).await.unwrap().into_split();
        Client {
            write,
            lines: FramedRead::new(read, LinesCodec::new()),
            next_id: 0,
            notes: Vec::new(),
        }
    }

    async fn hello(socket: &Path, client: &str) -> Client {
        let mut c = Client::connect(socket).await;
        let r = c
            .call(
                "hello",
                json!({"protocol": "handlr.harness", "version": 1, "client": client}),
            )
            .await;
        assert_eq!(r["result"]["server"], "handlr", "{r}");
        c
    }

    async fn send_line(&mut self, value: Value) {
        let mut line = value.to_string();
        line.push('\n');
        self.write.write_all(line.as_bytes()).await.unwrap();
    }

    async fn next(&mut self) -> Option<Value> {
        let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next())
            .await
            .expect("no reply within 5 s")?;
        Some(serde_json::from_str(&line.unwrap()).unwrap())
    }

    /// Send a request and return its response, keeping notifications.
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send_line(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        loop {
            let msg = self.next().await.expect("connection closed");
            if msg.get("method").is_some() {
                self.notes.push(msg);
            } else if msg["id"] == id {
                return msg;
            }
        }
    }

    /// The next notification named `method`, kept or yet to arrive.
    async fn note(&mut self, method: &str) -> Value {
        if let Some(i) = self.notes.iter().position(|n| n["method"] == method) {
            return self.notes.remove(i);
        }
        loop {
            let msg = self.next().await.expect("connection closed");
            if msg["method"] == method {
                return msg;
            }
            self.notes.push(msg);
        }
    }
}

async fn start() -> (tempfile::TempDir, std::path::PathBuf, Arc<Daemon>) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("h.sock");
    let daemon = Daemon::new(HerdrClient::mock(), &AppConfig::default());
    let listener = server::bind(&socket).await.unwrap();
    let options = ServeOptions {
        show_tokens: false,
        event_transport: None,
    };
    tokio::spawn(server::run(Arc::clone(&daemon), listener, options));
    (dir, socket, daemon)
}

async fn sent(daemon: &Daemon) -> Vec<(String, String)> {
    match daemon.client.transport() {
        HerdrTransport::Mock { state } => state.read().await.command_log.clone(),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn hello_must_come_first() {
    let (_dir, socket, _daemon) = start().await;
    let mut c = Client::connect(&socket).await;
    let r = c.call("ping", json!({})).await;
    assert_eq!(r["error"]["code"], -32010);
    assert!(c.next().await.is_none(), "the daemon closes the connection");

    let mut c = Client::connect(&socket).await;
    let r = c
        .call(
            "hello",
            json!({"protocol": "handlr.nope", "version": 1, "client": "t"}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32011);
}

#[tokio::test]
async fn a_second_daemon_cannot_take_the_socket() {
    let (_dir, socket, _daemon) = start().await;
    let err = server::bind(&socket).await.unwrap_err();
    assert!(err.contains("already listening"), "{err}");
}

#[tokio::test]
async fn agents_are_listed_read_and_watched() {
    let (_dir, socket, _daemon) = start().await;
    let mut c = Client::hello(&socket, "t").await;
    let r = c.call("agents.list", json!({})).await;
    let agents = r["result"]["agents"].as_array().unwrap();
    assert!(
        agents
            .iter()
            .any(|a| a["pane_id"] == "p1" && a["agent"] == "claude"),
        "{r}"
    );

    let r = c
        .call("agents.read", json!({"pane_id": "p1", "lines": 5}))
        .await;
    assert!(r["result"]["text"].is_string(), "{r}");
    let r = c
        .call("agents.read", json!({"pane_id": "p1", "lines": 0}))
        .await;
    assert_eq!(r["error"]["code"], -32602);

    let r = c.call("watch.add", json!({"pane_id": "p1"})).await;
    assert_eq!(r["result"]["watched"], json!(["p1"]));
    let r = c.call("watch.add", json!({"pane_id": "p999"})).await;
    assert_eq!(r["error"]["code"], -32002);
    let r = c.call("no.such.method", json!({})).await;
    assert_eq!(r["error"]["code"], -32601);
}

#[tokio::test]
async fn a_confirmed_send_runs_once() {
    let (_dir, socket, daemon) = start().await;
    // The daemon's terminal: the only place the token appears.
    let mut terminal = daemon.broker.subscribe();

    let mut mcp = Client::hello(&socket, "mcp").await;
    let mut watcher = Client::hello(&socket, "watcher").await;
    let r = watcher
        .call("events.subscribe", json!({"topics": ["confirm"]}))
        .await;
    assert!(r.get("error").is_none(), "{r}");

    let r = mcp
        .call(
            "agents.send",
            json!({"pane_id": "p1", "command": "continue", "handle": "keep-going"}),
        )
        .await;
    assert_eq!(r["result"]["status"], "pending", "{r}");
    let id = r["result"]["confirm_id"].as_u64().unwrap();
    assert!(sent(&daemon).await.is_empty());

    // A subscriber sees the request, without the token.
    let requested = watcher.note("confirm.requested").await;
    assert_eq!(requested["params"]["confirm_id"], id);
    assert_eq!(requested["params"]["handle"], "keep-going");
    assert!(requested["params"].get("token").is_none(), "{requested}");

    let token = match terminal.recv().await.unwrap() {
        ConfirmEvent::Requested { token, .. } => token.expose_for_terminal().to_string(),
        other => panic!("{other:?}"),
    };

    // The requester cannot approve its own command, even with the nonce.
    let good = nonce(&token, id, Some("keep-going"));
    let r = mcp
        .call(
            "confirm.reply",
            json!({"confirm_id": id, "approve": true, "nonce": good}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32007);

    // A local client approves with the nonce; the token itself is never sent.
    let mut ctl = Client::hello(&socket, "ctl").await;
    let r = ctl
        .call("confirm.reply", json!({"confirm_id": id, "approve": true}))
        .await;
    assert_eq!(r["error"]["code"], -32007, "no nonce, no approval: {r}");
    let r = ctl
        .call(
            "confirm.reply",
            json!({"confirm_id": id, "approve": true, "nonce": "00".repeat(32)}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32008);
    let r = ctl
        .call(
            "confirm.reply",
            json!({"confirm_id": id, "approve": true, "nonce": good}),
        )
        .await;
    assert_eq!(r["result"]["outcome"], "sent", "{r}");
    assert_eq!(
        sent(&daemon).await,
        vec![("p1".to_string(), "continue".to_string())]
    );

    // Replaying the reply finds nothing pending, and sends nothing.
    let r = ctl
        .call(
            "confirm.reply",
            json!({"confirm_id": id, "approve": true, "nonce": good}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32006);
    assert_eq!(sent(&daemon).await.len(), 1);

    // The requester hears how it ended, and finds it by its handle.
    let resolved = mcp.note("confirm.resolved").await;
    assert_eq!(resolved["params"]["outcome"], "sent");
    let r = mcp
        .call("confirm.status", json!({"handle": "keep-going"}))
        .await;
    assert_eq!(r["result"]["outcome"], "sent", "{r}");
    assert_eq!(r["result"]["confirm_id"], id);
}

#[tokio::test]
async fn unconfirmed_agents_are_sent_to_directly_and_refusals_say_why() {
    let (_dir, socket, daemon) = start().await;
    let mut c = Client::hello(&socket, "t").await;
    // claude needs confirmation by default; a command off its list is refused.
    let r = c
        .call(
            "agents.send",
            json!({"pane_id": "p1", "command": "rm -rf /"}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32001);
    assert!(r["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not permitted for agent 'claude'"));
    let r = c
        .call(
            "agents.send",
            json!({"pane_id": "p1", "command": "continue", "handle": "Bad Handle"}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32602);
    assert!(sent(&daemon).await.is_empty());
}

#[tokio::test]
async fn closing_the_connection_cancels_its_confirmations() {
    let (_dir, socket, daemon) = start().await;
    let mut mcp = Client::hello(&socket, "mcp").await;
    let r = mcp
        .call(
            "agents.send",
            json!({"pane_id": "p1", "command": "continue", "handle": "keep-going"}),
        )
        .await;
    let id = r["result"]["confirm_id"].as_u64().unwrap();
    drop(mcp);

    let principal = Principal::local("mcp");
    let mut outcome = None;
    for _ in 0..50 {
        let status = daemon
            .broker
            .status(&principal, &handlr::core::confirm::StatusKey::Id(id))
            .map(|(_, o)| o);
        if status != Some(handlr::core::confirm::Outcome::Pending) {
            outcome = status;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(outcome, Some(handlr::core::confirm::Outcome::Cancelled));
    assert!(sent(&daemon).await.is_empty());
}
