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
            .any(|a| a["pane_id"] == "p1" && a["agent"] == "claude" && a["name"] == "Claude"),
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

    // A handle returned slightly wrong finds it while pending; one that
    // matches nothing is told which handles are pending.
    let r = mcp
        .call("confirm.status", json!({"handle": "Kep Going"}))
        .await;
    assert_eq!(r["result"]["handle"], "keep-going", "{r}");
    assert_eq!(r["result"]["outcome"], "pending");
    let r = mcp
        .call("confirm.status", json!({"handle": "compact-codex"}))
        .await;
    assert_eq!(r["error"]["code"], -32006, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .ends_with("pending: keep-going"),
        "{r}"
    );

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

// ── Names (plan 20261022_05, step 3) ────────────────────────────────────
//
// The mock's typed snapshot has no top-level `tabs`, labels or numbers, so
// these tests run the daemon against a fake herdr on a temporary socket
// that answers with a snapshot shaped like herdr's own.

/// One workspace, `cc-highbay`, so every project is a cwd basename. Two
/// panes share `/src/libmorph`, which names them by tab; two tabs share the
/// label `dev`.
fn tabbed_snapshot() -> Value {
    let tabs = [
        ("w1:p1", "w1:t1", "17-hr", 17, "/src/handlr"),
        ("w1:p2", "w1:t2", "-mb", 4, "/src/libmorph"),
        ("w1:p3", "w1:t3", "-ad", 3, "/src/libmorph"),
        ("w1:p4", "w1:t4", "dev", 5, "/src/alpha"),
        ("w1:p5", "w1:t5", "dev", 6, "/src/beta"),
    ];
    let tab_list: Vec<Value> = tabs
        .iter()
        .map(|(_, tab, label, number, _)| {
            json!({"tab_id": tab, "workspace_id": "w1", "number": number, "label": label})
        })
        .collect();
    let panes: Vec<Value> = tabs
        .iter()
        .map(|(pane, tab, _, _, cwd)| {
            json!({
                "pane_id": pane, "tab_id": tab, "workspace_id": "w1", "cwd": cwd,
                "agent": "claude", "agent_status": "idle", "focused": false
            })
        })
        .collect();
    json!({"snapshot": {
        "workspaces": [{"workspace_id": "w1", "label": "cc-highbay", "number": 1}],
        "tabs": tab_list,
        "panes": panes
    }})
}

/// A fake herdr: answers `session.snapshot` with `snapshot`, `pane.read`
/// with "text of <pane>", anything else with success, and logs every
/// request as `(method, params)`.
struct FakeHerdr {
    log: Arc<std::sync::Mutex<Vec<(String, Value)>>>,
}

impl FakeHerdr {
    async fn start(socket: &Path, snapshot: Value) -> FakeHerdr {
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let task_log = Arc::clone(&log);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let log = Arc::clone(&task_log);
                let snapshot = snapshot.clone();
                tokio::spawn(async move {
                    let (read, mut write) = stream.into_split();
                    let mut lines = FramedRead::new(read, LinesCodec::new());
                    let Some(Ok(line)) = lines.next().await else {
                        return;
                    };
                    let request: Value = serde_json::from_str(&line).unwrap();
                    let method = request["method"].as_str().unwrap_or_default().to_string();
                    let params = request["params"].clone();
                    let result = match method.as_str() {
                        "session.snapshot" => snapshot,
                        "pane.read" => json!({"read": {
                            "text": format!("text of {}", params["pane_id"].as_str().unwrap_or("?"))
                        }}),
                        _ => json!({"ok": true}),
                    };
                    log.lock().unwrap().push((method, params));
                    let mut reply =
                        json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
                            .to_string();
                    reply.push('\n');
                    let _ = write.write_all(reply.as_bytes()).await;
                });
            }
        });
        FakeHerdr { log }
    }

    /// The `pane_id`s (or other target) of every logged `method`.
    fn targets(&self, method: &str) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, p)| {
                p.get("pane_id")
                    .or_else(|| p.get("workspace_id"))
                    .or_else(|| p.get("tab_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect()
    }
}

async fn start_tabbed() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Arc<Daemon>,
    FakeHerdr,
) {
    let dir = tempfile::tempdir().unwrap();
    let herdr = FakeHerdr::start(&dir.path().join("herdr.sock"), tabbed_snapshot()).await;
    let socket = dir.path().join("h.sock");
    let daemon = Daemon::new(
        HerdrClient::local(dir.path().join("herdr.sock")),
        &AppConfig::default(),
    );
    let listener = server::bind(&socket).await.unwrap();
    let options = ServeOptions {
        show_tokens: false,
        event_transport: None,
    };
    tokio::spawn(server::run(Arc::clone(&daemon), listener, options));
    (dir, socket, daemon, herdr)
}

#[tokio::test]
async fn agents_are_listed_with_names_and_tabs() {
    let (_dir, socket, _daemon, _herdr) = start_tabbed().await;
    let mut c = Client::hello(&socket, "t").await;
    let r = c.call("agents.list", json!({})).await;
    let agents = r["result"]["agents"].as_array().unwrap();
    let by_pane = |id: &str| agents.iter().find(|a| a["pane_id"] == id).unwrap();

    let hr = by_pane("w1:p1");
    assert_eq!(hr["name"], "the handlr agent", "{r}");
    assert_eq!(hr["spoken"], "the handlr agent");
    assert_eq!(hr["tab_label"], "17-hr");
    assert_eq!(hr["tab_number"], 17);

    let mb = by_pane("w1:p2");
    assert_eq!(mb["name"], "Claude in libmorph, tab -mb");
    assert_eq!(mb["spoken"], "Claude in libmorph, tab em bee");
    assert_eq!(mb["tab_label"], "-mb");
    assert_eq!(mb["tab_number"], 4);
    assert_eq!(by_pane("w1:p3")["spoken"], "Claude in libmorph, tab ad");

    // No name is a pane id.
    for a in agents {
        let id = a["pane_id"].as_str().unwrap();
        let short = id.rsplit(':').next().unwrap();
        for key in ["name", "spoken"] {
            let name = a[key].as_str().unwrap();
            assert!(
                !name.contains(id) && !name.split([' ', ',']).any(|w| w == short),
                "{name}"
            );
        }
    }
}

#[tokio::test]
async fn targets_resolve_by_label_spoken_form_and_tab_number() {
    let (_dir, socket, _daemon, herdr) = start_tabbed().await;
    let mut c = Client::hello(&socket, "t").await;
    for target in ["hr", "HR", "17-hr", "aitch ar", "w1:p1", "p1"] {
        let r = c
            .call("agents.read", json!({"pane_id": target, "lines": 5}))
            .await;
        assert_eq!(r["result"]["text"], "text of w1:p1", "{target}: {r}");
    }
    // A tab number, and a spelled form.
    let r = c
        .call("agents.read", json!({"pane_id": "4", "lines": 5}))
        .await;
    assert_eq!(r["result"]["text"], "text of w1:p2", "{r}");
    let r = c
        .call("agents.read", json!({"pane_id": "em bee", "lines": 5}))
        .await;
    assert_eq!(r["result"]["text"], "text of w1:p2", "{r}");
    assert_eq!(herdr.targets("pane.read").len(), 8);

    // watch.add and watch.remove resolve the same way.
    let r = c.call("watch.add", json!({"pane_id": "-ad"})).await;
    assert_eq!(r["result"]["watched"], json!(["w1:p3"]), "{r}");
    let r = c.call("watch.remove", json!({"pane_id": "AD"})).await;
    assert_eq!(r["result"]["watched"], json!([]), "{r}");

    // agents.focus resolves names, and passes workspace and tab ids through.
    for target in ["hr", "w1", "w1:t2"] {
        let r = c.call("agents.focus", json!({"target_id": target})).await;
        assert!(r.get("error").is_none(), "{target}: {r}");
    }
    assert_eq!(herdr.targets("pane.focus"), vec!["w1:p1"]);
    assert_eq!(herdr.targets("workspace.focus"), vec!["w1"]);
    assert_eq!(herdr.targets("tab.focus"), vec!["w1:t2"]);
}

#[tokio::test]
async fn an_ambiguous_target_lists_the_candidates_by_name() {
    let (_dir, socket, _daemon, herdr) = start_tabbed().await;
    let mut c = Client::hello(&socket, "t").await;
    for (method, params) in [
        ("agents.read", json!({"pane_id": "dev", "lines": 5})),
        (
            "agents.send",
            json!({"pane_id": "dev", "command": "continue", "handle": "go-on"}),
        ),
        ("agents.focus", json!({"target_id": "dev"})),
        ("watch.add", json!({"pane_id": "dev"})),
    ] {
        let r = c.call(method, params).await;
        assert_eq!(r["error"]["code"], -32002, "{method}: {r}");
        let message = r["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("the alpha agent") && message.contains("the beta agent"),
            "{message}"
        );
        assert!(
            !message.contains("w1:p4") && !message.contains("p5"),
            "{message}"
        );
        assert_eq!(
            r["error"]["data"]["candidates"],
            json!(["the alpha agent", "the beta agent"])
        );
    }
    let r = c
        .call("agents.read", json!({"pane_id": "nothing", "lines": 5}))
        .await;
    assert_eq!(r["error"]["code"], -32002);
    assert!(herdr.targets("pane.read").is_empty());
    assert!(herdr.targets("pane.send_input").is_empty());
    assert!(herdr.targets("pane.focus").is_empty());
}

#[tokio::test]
async fn confirm_requested_carries_names() {
    let (_dir, socket, daemon, herdr) = start_tabbed().await;
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
            json!({"pane_id": "em bee", "command": "continue", "handle": "keep-going"}),
        )
        .await;
    assert_eq!(r["result"]["status"], "pending", "{r}");
    let id = r["result"]["confirm_id"].as_u64().unwrap();

    let requested = watcher.note("confirm.requested").await;
    let p = &requested["params"];
    assert_eq!(p["confirm_id"], id);
    assert_eq!(p["pane_id"], "w1:p2");
    assert_eq!(p["name"], "Claude in libmorph, tab -mb", "{requested}");
    assert_eq!(p["spoken"], "Claude in libmorph, tab em bee");
    assert_eq!(p["project"], "cc-highbay");

    // Approving sends to the pane the name resolved to.
    let token = match terminal.recv().await.unwrap() {
        ConfirmEvent::Requested { token, .. } => token.expose_for_terminal().to_string(),
        other => panic!("{other:?}"),
    };
    let mut ctl = Client::hello(&socket, "ctl").await;
    let r = ctl
        .call(
            "confirm.reply",
            json!({"confirm_id": id, "approve": true, "nonce": nonce(&token, id, Some("keep-going"))}),
        )
        .await;
    assert_eq!(r["result"]["outcome"], "sent", "{r}");
    assert_eq!(herdr.targets("pane.send_input"), vec!["w1:p2"]);
}

#[tokio::test]
async fn status_by_handle_lists_pending_when_ambiguous() {
    // Plan 20261023_01, section 3.3 and step 3.
    let (_dir, socket, _daemon) = start().await;
    let mut mcp = Client::hello(&socket, "mcp").await;
    for (command, handle) in [
        ("continue", "send-continue-claude-highbay"),
        ("/compact", "send-compact-claude-highbay"),
    ] {
        let r = mcp
            .call(
                "agents.send",
                json!({"pane_id": "p1", "command": command, "handle": handle}),
            )
            .await;
        assert_eq!(r["result"]["status"], "pending", "{r}");
    }

    // 3 bits for the first, 1 for the second: neither is taken, and the
    // pending handles are listed, newest first.
    let r = mcp
        .call("confirm.status", json!({"handle": "continue claude"}))
        .await;
    assert_eq!(r["error"]["code"], -32009, "{r}");
    assert_eq!(
        r["error"]["data"]["pending"],
        json!([
            "send-compact-claude-highbay",
            "send-continue-claude-highbay"
        ]),
        "{r}"
    );
    assert_eq!(
        r["error"]["data"]["candidates"],
        json!(["send-continue-claude-highbay"])
    );
    // A miss lists them too.
    let r = mcp
        .call("confirm.status", json!({"handle": "send claude highbay"}))
        .await;
    assert_eq!(r["error"]["code"], -32006, "{r}");
    assert_eq!(r["error"]["data"]["pending"].as_array().unwrap().len(), 2);

    // Every word of one: coverage; equal once normalised: exact.
    let r = mcp
        .call(
            "confirm.status",
            json!({"handle": "compact claude high bay"}),
        )
        .await;
    assert_eq!(r["result"]["handle"], "send-compact-claude-highbay", "{r}");
    assert_eq!(r["result"]["matched"], "coverage");
    let r = mcp
        .call(
            "confirm.status",
            json!({"handle": "Send Continue Claude Highbay"}),
        )
        .await;
    assert_eq!(r["result"]["matched"], "exact", "{r}");
    assert!(r["result"].get("bits").is_none());

    // The distinguished one, by its distinctive word alone.
    let r = mcp
        .call(
            "agents.send",
            json!({"pane_id": "p1", "command": "continue", "handle": "continue-claude-highbay-ripple"}),
        )
        .await;
    assert_eq!(r["result"]["status"], "pending", "{r}");
    let r = mcp
        .call("confirm.status", json!({"handle": "the ripple one"}))
        .await;
    assert_eq!(
        r["result"]["handle"], "continue-claude-highbay-ripple",
        "{r}"
    );
    assert_eq!(r["result"]["matched"], "entropy");
    assert!(r["result"]["bits"].as_f64().unwrap() >= 6.0, "{r}");
}
