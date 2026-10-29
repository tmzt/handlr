//! The plugins, the daemon's half, end to end over the link
//! (docs/design/20261028_04_PLUGINS_PLAN.md, steps 2 and 3): companions
//! over TCP on 127.0.0.1 with an ephemeral port, their keys in a
//! temporary `auth.toml`, a temporary config file, over the herdr mock
//! whose one pane's folder is this checkout. Nothing touches
//! `~/.config/flankr` or a live herdr; the checkout is only read.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use flankr::config::AppConfig;
use flankr::llm::LlmOptions;
use flankr::server::companion::TurnOptions;
use flankr::server::link::auth::{self, AuthEntry};
use flankr::server::link::{Link, LinkSettings};
use flankr::server::{self, Daemon, ServeOptions};
use flankr::sessions::Sessions;
use flankr::transport::HerdrClient;
use libflankr_proto::envelope::codes;
use libflankr_proto::link::{LinkStream, Message, StaticKeypair, generate_keypair, reconnect};
use libherdr::transport::MockState;
use serde_json::{Value, json};
use tokio::net::TcpStream;

const WAIT: Duration = Duration::from_secs(10);
const PLANNING: &str = "dev.flankr.plugins.planning";
/// The template's `read` (config.default.toml).
const READ: &str = "[llm.uses.planning]\nread = [\"*.md\", \"docs/**/*.md\"]\n";

struct Env {
    dir: tempfile::TempDir,
    config: PathBuf,
    address: String,
    link: Arc<Link>,
    _daemon: Arc<Daemon>,
}

/// This checkout: the pane's folder.
fn checkout() -> PathBuf {
    std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap()
}

async fn start(config_text: &str) -> Env {
    let dir = tempfile::Builder::new()
        .prefix("hp")
        .tempdir_in("/tmp")
        .unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(&config, config_text).unwrap();
    let app: AppConfig = toml::from_str(config_text).unwrap();
    let link = Link::open(LinkSettings {
        key_path: dir.path().join("link_key"),
        auth_path: dir.path().join("auth.toml"),
        listen: Some(vec!["127.0.0.1:0".to_string()]),
        revoke_interval: Duration::from_secs(60),
    })
    .unwrap();
    let listeners = link.bind().await.unwrap();
    let address = listeners[0].local_addr().unwrap().to_string();
    let mut state = MockState::default();
    state.snapshot.workspaces[0].tabs[0].panes[0].cwd = Some(checkout().display().to_string());
    let mock = HerdrClient::mock_with_state(state);
    let daemon = Daemon::with_turns(
        mock,
        &app,
        Some(Arc::clone(&link)),
        TurnOptions {
            session_dir: Some(dir.path().join("history")),
            agent_sessions: Some(Arc::new(Sessions::with_home(&dir.path().join("home")))),
            llm: LlmOptions {
                config_path: Some(config.clone()),
                interval: Some(Duration::from_secs(3600)),
                ..Default::default()
            },
            // No platform watcher: the set_enabled write reloads at once.
            file_watcher: Some(Err("not in tests".to_string())),
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
        config,
        address,
        link,
        _daemon: daemon,
    }
}

impl Env {
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
    hello: Value,
}

impl Client {
    async fn pump(&mut self) -> Option<Value> {
        let message = tokio::time::timeout(WAIT, self.stream.recv())
            .await
            .expect("no message in time")
            .expect("the link closed");
        let Message::Json(bytes) = message else {
            return None;
        };
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        if value.get("method").is_some() {
            self.notes.push(value);
            None
        } else {
            Some(value)
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

    async fn note(&mut self, method: &str) -> Value {
        loop {
            if let Some(i) = self.notes.iter().position(|n| n["method"] == method) {
                return self.notes.remove(i);
            }
            self.pump().await;
        }
    }

    /// `plugin.call` with inner id `id`: accepted at once, then its
    /// `plugin.result`.
    async fn plugin(&mut self, id: u64, method: &str, params: Value) -> Value {
        let r = self
            .call(
                "plugin.call",
                json!({"ns": PLANNING, "id": id, "method": method, "params": params, "session": "p1"}),
            )
            .await;
        assert_eq!(r["result"], json!({}), "{r}");
        loop {
            let note = self.note("plugin.result").await;
            if note["params"]["id"] == id {
                assert_eq!(note["params"]["ns"], PLANNING);
                return note["params"].clone();
            }
        }
    }
}

fn code(r: &Value) -> i64 {
    r["error"]["code"]
        .as_i64()
        .unwrap_or_else(|| panic!("not an error: {r}"))
}

/// Step 2: the plugins are listed, off, in hello; a call to one that is
/// off, to one that does not exist, or with a bad method is refused at
/// once; only a device that can approve may switch one on.
#[tokio::test]
async fn hello_lists_the_plugins_and_only_an_approver_switches_one_on() {
    let env = start("# kept\n[naming]\nspell_max_len = 6\n").await;
    let mut viewer = env.device("viewer", false).await;
    let plugins = viewer.hello["result"]["plugins"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(plugins.len(), 1, "{plugins:?}");
    assert_eq!(plugins[0]["ns"], PLANNING);
    assert_eq!(plugins[0]["version"], "0.1.1");
    assert_eq!(plugins[0]["enabled"], false);
    assert_eq!(
        plugins[0]["menu"],
        json!([{"id": "plans", "label": "Plans in this project", "opens": "menu", "screens": "both", "icon": "plans"}])
    );
    let r = viewer
        .call(
            "plugin.call",
            json!({"ns": PLANNING, "id": 1, "method": "plans.list", "params": {}}),
        )
        .await;
    assert_eq!(code(&r), codes::PLUGIN_DISABLED);
    let r = viewer
        .call(
            "plugin.call",
            json!({"ns": "dev.flankr.plugins.none", "id": 1, "method": "x", "params": {}}),
        )
        .await;
    assert_eq!(code(&r), codes::PLUGIN_UNKNOWN);
    let r = viewer
        .call(
            "plugin.call",
            json!({"ns": PLANNING, "id": 1, "method": "Plans!", "params": {}}),
        )
        .await;
    assert_eq!(code(&r), codes::INVALID_PARAMS);
    let r = viewer
        .call(
            "plugin.set_enabled",
            json!({"ns": PLANNING, "enabled": true}),
        )
        .await;
    assert_eq!(code(&r), codes::MAY_NOT_APPROVE);
    assert_eq!(
        std::fs::read_to_string(&env.config).unwrap(),
        "# kept\n[naming]\nspell_max_len = 6\n"
    );

    let mut owner = env.device("owner", true).await;
    let r = owner
        .call(
            "plugin.set_enabled",
            json!({"ns": PLANNING, "enabled": true}),
        )
        .await;
    assert_eq!(r["result"], json!({"enabled": true}), "{r}");
    assert_eq!(
        std::fs::read_to_string(&env.config).unwrap(),
        "# kept\n[naming]\nspell_max_len = 6\n\n[plugins.\"dev.flankr.plugins.planning\"]\nenabled = true\n"
    );
    // Every companion hears the new list.
    for client in [&mut owner, &mut viewer] {
        let caps = client.note("server.capabilities").await;
        assert_eq!(caps["params"]["plugins"][0]["enabled"], true, "{caps}");
        assert!(caps["params"]["llm"].is_object());
        // On, without `read`: the reason is given, and reads say why.
        assert_eq!(
            caps["params"]["plugins"][0]["reason"],
            "Reading files is off: [llm.uses.planning] has no read patterns."
        );
    }
    assert!(
        owner.hello["result"]["plugins"][0].get("reason").is_none(),
        "off: no reason"
    );
    let result = owner.plugin(2, "plans.list", json!({})).await;
    assert_eq!(
        result["error"]["message"],
        "Reading files is off: [llm.uses.planning] has no read patterns."
    );
}

/// Step 3's Done when: with `enabled = true` and the template's `read`,
/// `plans.list` on this checkout lists every plan of the README's table
/// with matching statuses.
#[tokio::test]
async fn plans_list_on_this_checkout_matches_the_readme() {
    let env = start(&format!("{READ}[plugins.\"{PLANNING}\"]\nenabled = true\n")).await;
    let mut owner = env.device("owner", true).await;
    assert_eq!(owner.hello["result"]["plugins"][0]["enabled"], true);
    let result = owner.plugin(1, "plans.list", json!({})).await;
    assert!(result["error"].is_null(), "{result}");
    let list = &result["result"];
    assert_eq!(list["schema"], 1);
    let plans = list["plans"].as_array().unwrap();
    let readme = std::fs::read_to_string(checkout().join("README.md")).unwrap();
    let rows = flankr::plugins::planning::reader::readme_rows(&readme);
    let files: Vec<String> = std::fs::read_dir(checkout().join("docs/design"))
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            flankr::plugins::planning::reader::file_name(&name).map(|_| name)
        })
        .collect();
    assert_eq!(plans.len(), files.len());
    assert_eq!(rows.len(), plans.len(), "a row per plan");
    for row in &rows {
        let file = flankr::plugins::planning::reader::root_relative(&row.target).unwrap();
        let plan = plans
            .iter()
            .find(|p| p["file"] == file.as_str())
            .unwrap_or_else(|| panic!("no plan for the row {}", row.target));
        let status = plan["status"].as_str().unwrap();
        assert!(
            flankr::plugins::planning::reader::same_status(status, &row.status),
            "{file}: {status} / {}",
            row.status
        );
        assert_eq!(plan["readme"]["status"], row.status.as_str());
    }
    // This plan, shown.
    let shown = owner
        .plugin(2, "plans.show", json!({"name": "20261028_04_PLUGINS_PLAN"}))
        .await;
    let plan = &shown["result"]["plan"];
    assert_eq!(
        plan["title"],
        "Built-in plugins, with a namespaced message flow, a title-bar menu and the planning plugin"
    );
    assert_eq!(
        plan["amendments"][0]["timestamp"], "2026-10-28 19:47 EDT",
        "{plan}"
    );
    let shown = owner
        .plugin(3, "plans.show", json!({"name": "20261028_99_NONE_PLAN"}))
        .await;
    assert_eq!(shown["error"]["code"], -32111);
}
