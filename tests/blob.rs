//! Binary transfers, the daemon's half, end to end over the link
//! (docs/design/20261028_05_BINARY_FILES_PLAN.md, steps 3-6): companions
//! over TCP on 127.0.0.1 with an ephemeral port, their keys in a
//! temporary `auth.toml`, the images store in a temporary folder standing
//! in for `temp_dir()`, over the herdr mock whose panes' folder is a
//! temporary project. Nothing touches `~/.config/flankr` or a live herdr.

use std::collections::VecDeque;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use flankr::config::{AgentConfig, AppConfig, TerminalConfig};
use flankr::llm::LlmOptions;
use flankr::server::companion::TurnOptions;
use flankr::server::link::auth::{self, AuthEntry};
use flankr::server::link::{Link, LinkSettings};
use flankr::server::{self, Daemon, ServeOptions};
use flankr::sessions::Sessions;
use flankr::transport::HerdrClient;
use libflankr_blob::{BlobMessage, Body, CHUNK_DEFAULT, Code, Namespace, Offer, TransferId};
use libflankr_proto::envelope::codes;
use libflankr_proto::link::{LinkStream, Message, StaticKeypair, generate_keypair, reconnect};
use libherdr::protocol::PaneInfo;
use libherdr::transport::{HerdrTransport, MockState};
use serde_json::{Value, json};
use tokio::net::TcpStream;

const WAIT: Duration = Duration::from_secs(10);
const READ: &str = "[llm.uses.planning]\nread = [\"*.md\", \"docs/**/*.md\", \"docs/*.png\"]\n";

/// 64 bytes that sniff as a PNG.
fn png(fill: u8) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    v.resize(64, fill);
    v
}

struct Env {
    dir: tempfile::TempDir,
    temp: PathBuf,
    project: PathBuf,
    address: String,
    link: Arc<Link>,
    mock: HerdrClient,
}

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

fn pane(id: &str, agent: &str, cwd: &Path) -> PaneInfo {
    let mut extra = serde_json::Map::new();
    extra.insert("agent".to_string(), json!(agent));
    PaneInfo {
        pane_id: id.to_string(),
        terminal_title: None,
        cwd: Some(cwd.display().to_string()),
        agent_status: None,
        focused: false,
        command: None,
        agent_session: None,
        extra,
    }
}

async fn start(read: &str) -> Env {
    let dir = tempfile::Builder::new()
        .prefix("hb")
        .tempdir_in("/tmp")
        .unwrap();
    let project = dir.path().join("project");
    for (rel, text) in [
        ("README.md", &b"# Hello\n"[..]),
        ("docs/a.md", b"a"),
        ("src/main.rs", b"fn main() {}"),
        (".env", b"SECRET=1"),
    ] {
        let p = project.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    std::fs::write(project.join("docs/a.png"), png(1)).unwrap();
    std::fs::write(project.join("docs/big.md"), vec![b'a'; 3 << 20]).unwrap();
    let project = std::fs::canonicalize(project).unwrap();
    let temp = dir.path().join("tmp");
    std::fs::create_dir(&temp).unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(&config_path, read).unwrap();
    let mut app: AppConfig = toml::from_str(read).unwrap();
    app.agents = vec![
        AgentConfig::new("claude", vec!["continue".to_string()], true)
            .with_terminal_input(true, false),
        AgentConfig::new("omp", vec!["continue".to_string()], true).with_messages(false),
    ];
    app.summary = Some(flankr::config::SummaryConfig { cards: false });
    app.terminal = TerminalConfig {
        input: true,
        ..TerminalConfig::default()
    };
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
    state.snapshot.workspaces[0].tabs[0].panes[0].cwd = Some(project.display().to_string());
    state.snapshot.workspaces[0].tabs[0]
        .panes
        .push(pane("p2", "omp", &project));
    state
        .pane_buffers
        .insert("p1".to_string(), vec!["hello".to_string()]);
    let mock = HerdrClient::mock_with_state(state);
    let daemon = Daemon::with_turns(
        mock.clone(),
        &app,
        Some(Arc::clone(&link)),
        TurnOptions {
            session_dir: Some(dir.path().join("history")),
            agent_sessions: Some(Arc::new(Sessions::with_home(&dir.path().join("home")))),
            llm: LlmOptions {
                config_path: Some(config_path),
                interval: Some(Duration::from_secs(3600)),
                ..Default::default()
            },
            file_watcher: Some(Err("not in tests".to_string())),
            blob_temp: Some(temp.clone()),
            ..TurnOptions::default()
        },
    );
    let unix = server::bind(&dir.path().join("h.sock")).await.unwrap();
    server::link::spawn(Arc::clone(&daemon), listeners);
    tokio::spawn(server::run(
        daemon,
        unix,
        ServeOptions {
            show_tokens: false,
            event_transport: None,
        },
    ));
    Env {
        dir,
        temp,
        project,
        address,
        link,
        mock,
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
            blobs: VecDeque::new(),
            hello: Value::Null,
            ids: libflankr_blob::Ids::phone(),
        };
        client.hello = client.reply(0).await;
        client
    }

    fn pane_dir(&self, pane: &str) -> PathBuf {
        self.temp
            .join(format!("flankr-{}", uid()))
            .join("images")
            .join(pane)
    }

    async fn mock<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        let HerdrTransport::Mock { state } = self.mock.transport() else {
            unreachable!()
        };
        f(&mut *state.write().await)
    }
}

struct Client {
    stream: LinkStream<TcpStream>,
    next_id: i64,
    notes: Vec<Value>,
    blobs: VecDeque<BlobMessage>,
    hello: Value,
    ids: libflankr_blob::Ids,
}

enum Got {
    Json(Value),
    Blob,
    Other,
}

impl Client {
    async fn pump(&mut self) -> Got {
        let message = tokio::time::timeout(WAIT, self.stream.recv())
            .await
            .expect("no message in time")
            .expect("the link closed");
        match message {
            Message::Json(bytes) => {
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                if value.get("method").is_some() {
                    self.notes.push(value);
                    Got::Other
                } else {
                    Got::Json(value)
                }
            }
            Message::Binary(bytes) => {
                self.blobs
                    .push_back(BlobMessage::decode(&bytes).expect("a message that decodes"));
                Got::Blob
            }
            _ => Got::Other,
        }
    }

    async fn reply(&mut self, id: i64) -> Value {
        loop {
            if let Got::Json(r) = self.pump().await
                && r["id"] == id
            {
                return r;
            }
        }
    }

    async fn send_call(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.stream
            .send_message(&serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        id
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_call(method, params).await;
        self.reply(id).await
    }

    async fn blob(&mut self, msg: BlobMessage) {
        self.stream.send_binary(&msg.encode()).await.unwrap();
    }

    async fn next_blob(&mut self) -> BlobMessage {
        loop {
            if let Some(m) = self.blobs.pop_front() {
                return m;
            }
            self.pump().await;
        }
    }

    /// An `images` OFFER of `bytes` named `mime`, for `session`.
    async fn offer(&mut self, session: &str, mime: &str, bytes: &[u8]) -> (TransferId, Body) {
        let id = self.ids.next();
        let offer = Offer::for_bytes(session, "IMG_0001.png", mime, bytes, 0);
        self.blob(BlobMessage::new(
            id,
            Namespace::images(),
            Body::Offer(offer),
        ))
        .await;
        let answer = self.next_blob().await;
        assert_eq!(answer.transfer, id);
        (id, answer.body)
    }

    /// A whole upload: its DONE's reference as hex, or the code.
    async fn upload(&mut self, session: &str, bytes: &[u8]) -> Result<String, Code> {
        let (id, answer) = self.offer(session, "image/png", bytes).await;
        match answer {
            Body::Accept { chunk_max } => assert_eq!(chunk_max, libflankr_blob::CHUNK_MAX),
            Body::Refuse { code, .. } => return Err(code),
            other => panic!("{other:?}"),
        }
        let ns = Namespace::images();
        for (i, chunk) in bytes.chunks(24).enumerate() {
            self.blob(BlobMessage::new(
                id,
                ns.clone(),
                Body::Chunk {
                    offset: (i * 24) as u64,
                    data: chunk.to_vec(),
                },
            ))
            .await;
        }
        self.blob(BlobMessage::new(
            id,
            ns,
            Body::End {
                size: bytes.len() as u64,
            },
        ))
        .await;
        match self.next_blob().await.body {
            Body::Done { body } => {
                assert_eq!(body.len(), 8);
                Ok(libflankr_blob::hex(&body))
            }
            Body::Cancel { code, .. } => Err(code),
            other => panic!("{other:?}"),
        }
    }

    /// A REQUEST in `ns`, then the whole download: the OFFER and the bytes.
    async fn download(&mut self, ns: &str, request: &str) -> Result<(Offer, Vec<u8>), Code> {
        let id = self.ids.next();
        let ns = Namespace::new(ns).unwrap();
        self.blob(BlobMessage::new(
            id,
            ns.clone(),
            Body::Request {
                session: "p1".into(),
                request: request.into(),
                max_size: 8 << 20,
            },
        ))
        .await;
        let offer = match self.next_blob().await.body {
            Body::Offer(o) => o,
            Body::Refuse { code, .. } => return Err(code),
            other => panic!("{other:?}"),
        };
        self.blob(BlobMessage::new(
            id,
            ns.clone(),
            Body::Accept {
                chunk_max: CHUNK_DEFAULT,
            },
        ))
        .await;
        let mut bytes = Vec::new();
        loop {
            let m = self.next_blob().await;
            assert_eq!(m.transfer, id);
            match m.body {
                Body::Chunk { offset, data } => {
                    assert_eq!(offset, bytes.len() as u64);
                    bytes.extend(data);
                }
                Body::End { size } => {
                    assert_eq!(size, bytes.len() as u64);
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(libflankr_blob::sha256(&bytes), offer.sha256);
        self.blob(BlobMessage::new(id, ns, Body::Done { body: vec![] }))
            .await;
        Ok((offer, bytes))
    }
}

fn code(r: &Value) -> i64 {
    r["error"]["code"].as_i64().unwrap_or(0)
}

#[tokio::test]
async fn hello_offers_blob_and_an_image_lands_in_the_protected_folder() {
    let env = start(READ).await;
    let mut phone = env.device("s23", false).await;
    let blob = &phone.hello["result"]["blob"];
    assert_eq!(blob["version"], 1, "{}", phone.hello);
    let namespaces = blob["namespaces"].as_array().unwrap();
    assert!(namespaces.contains(&json!("images")) && namespaces.contains(&json!("files")));
    assert_eq!(blob["read"], json!(["*.md", "docs/**/*.md", "docs/*.png"]));
    assert_eq!(blob["images_per_send"], 4);

    let bytes = png(7);
    let reference = phone.upload("p1", &bytes).await.unwrap();
    assert_eq!(reference.len(), 16);
    let pane = env.pane_dir("p1");
    let files: Vec<PathBuf> = std::fs::read_dir(&pane)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_str().unwrap();
    assert!(
        name.ends_with(&format!(
            "-{}.png",
            libflankr_blob::hex(&libflankr_blob::sha256(&bytes)[..4])
        )),
        "{name}"
    );
    assert_eq!(std::fs::read(&files[0]).unwrap(), bytes);
    assert_eq!(std::fs::metadata(&files[0]).unwrap().mode() & 0o777, 0o600);
    for d in [
        env.temp.join(format!("flankr-{}", uid())),
        pane.parent().unwrap().to_path_buf(),
        pane.clone(),
    ] {
        assert_eq!(
            std::fs::metadata(&d).unwrap().mode() & 0o777,
            0o700,
            "{d:?}"
        );
    }
    // JSON-RPC goes on beside it.
    let r = phone.call("ping", json!({})).await;
    assert!(r.get("result").is_some(), "{r}");
}

#[tokio::test]
async fn wrong_types_unknown_sessions_and_agents_are_refused_and_leave_no_part() {
    let env = start(READ).await;
    let mut phone = env.device("s23", false).await;
    let (_, a) = phone.offer("p1", "image/heic", &png(1)).await;
    assert!(
        matches!(
            a,
            Body::Refuse {
                code: Code::BAD_TYPE,
                ..
            }
        ),
        "{a:?}"
    );
    let (_, a) = phone.offer("p404", "image/png", &png(1)).await;
    assert!(
        matches!(
            a,
            Body::Refuse {
                code: Code::NO_SESSION,
                ..
            }
        ),
        "{a:?}"
    );
    // A PNG named JPEG: accepted by its MIME, cancelled by its bytes.
    let (id, a) = phone.offer("p1", "image/jpeg", &png(1)).await;
    assert!(matches!(a, Body::Accept { .. }), "{a:?}");
    phone
        .blob(BlobMessage::new(
            id,
            Namespace::images(),
            Body::Chunk {
                offset: 0,
                data: png(1),
            },
        ))
        .await;
    let m = phone.next_blob().await;
    assert!(
        matches!(
            m.body,
            Body::Cancel {
                code: Code::BAD_TYPE,
                ..
            }
        ),
        "{m:?}"
    );
    // A damaged upload: hash_mismatch.
    let bytes = png(3);
    let (id, _) = phone.offer("p1", "image/png", &bytes).await;
    let mut flipped = bytes.clone();
    flipped[40] ^= 1;
    let ns = Namespace::images();
    phone
        .blob(BlobMessage::new(
            id,
            ns.clone(),
            Body::Chunk {
                offset: 0,
                data: flipped,
            },
        ))
        .await;
    phone
        .blob(BlobMessage::new(id, ns, Body::End { size: 64 }))
        .await;
    let m = phone.next_blob().await;
    assert!(
        matches!(
            m.body,
            Body::Cancel {
                code: Code::HASH_MISMATCH,
                ..
            }
        ),
        "{m:?}"
    );
    let left: Vec<_> = std::fs::read_dir(env.pane_dir("p1"))
        .unwrap()
        .flatten()
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // A CHUNK for no transfer: CANCEL unknown_transfer, once.
    for _ in 0..2 {
        phone
            .blob(BlobMessage::new(
                TransferId(77),
                Namespace::images(),
                Body::Chunk {
                    offset: 0,
                    data: vec![1],
                },
            ))
            .await;
    }
    let r = phone.call("ping", json!({})).await;
    assert!(r.get("result").is_some());
    let unknown: Vec<_> = phone.blobs.drain(..).collect();
    assert_eq!(unknown.len(), 1, "{unknown:?}");
    assert!(matches!(
        unknown[0].body,
        Body::Cancel {
            code: Code::UNKNOWN_TRANSFER,
            ..
        }
    ));
}

#[tokio::test]
async fn the_fifth_live_transfer_is_busy_and_a_dropped_connection_leaves_no_part() {
    let env = start(READ).await;
    let mut phone = env.device("s23", false).await;
    let bytes = png(5);
    let mut ids = Vec::new();
    for _ in 0..4 {
        let (id, a) = phone.offer("p1", "image/png", &bytes).await;
        assert!(matches!(a, Body::Accept { .. }), "{a:?}");
        ids.push(id);
    }
    let (_, a) = phone.offer("p1", "image/png", &bytes).await;
    assert!(
        matches!(
            a,
            Body::Refuse {
                code: Code::BUSY,
                ..
            }
        ),
        "{a:?}"
    );
    // Half of one written, then the phone goes.
    phone
        .blob(BlobMessage::new(
            ids[0],
            Namespace::images(),
            Body::Chunk {
                offset: 0,
                data: bytes[..32].to_vec(),
            },
        ))
        .await;
    let _ = phone.call("ping", json!({})).await;
    let parts = std::fs::read_dir(env.pane_dir("p1")).unwrap().count();
    assert_eq!(parts, 4, "four .part files while live");
    drop(phone);
    let pane = env.pane_dir("p1");
    let deadline = tokio::time::Instant::now() + WAIT;
    while std::fs::read_dir(&pane).unwrap().count() > 0 {
        assert!(tokio::time::Instant::now() < deadline, "parts left");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn an_unreadable_header_closes_the_connection() {
    let env = start(READ).await;
    let mut phone = env.device("s23", false).await;
    phone.stream.send_binary(&[1, 0, 0]).await.unwrap();
    let r = tokio::time::timeout(WAIT, phone.stream.recv())
        .await
        .unwrap();
    assert!(r.is_err(), "{r:?}");
}

#[tokio::test]
async fn files_are_served_under_read_files_rules() {
    let env = start(READ).await;
    let mut phone = env.device("s23", false).await;
    let (offer, bytes) = phone.download("files", "README.md").await.unwrap();
    assert_eq!(offer.name, "README.md");
    assert_eq!(offer.mime, "text/markdown");
    assert_eq!(offer.session, "p1");
    assert!(offer.modified_ms > 0);
    assert_eq!(bytes, b"# Hello\n");
    let (offer, bytes) = phone.download("files", "/docs/a.png").await.unwrap();
    assert_eq!(
        (offer.mime.as_str(), offer.name.as_str()),
        ("image/png", "docs/a.png")
    );
    assert_eq!(bytes, png(1));
    for refused in ["src/main.rs", ".env", "../x.md"] {
        assert_eq!(
            phone.download("files", refused).await.unwrap_err(),
            Code::NOT_PERMITTED,
            "{refused}"
        );
    }
    assert_eq!(
        phone.download("files", "docs/none.md").await.unwrap_err(),
        Code::NOT_FOUND
    );
    // Several CHUNKs, with a ping answered before the download's END.
    let id = phone.ids.next();
    let ns = Namespace::files();
    phone
        .blob(BlobMessage::new(
            id,
            ns.clone(),
            Body::Request {
                session: "p1".into(),
                request: "docs/big.md".into(),
                max_size: 8 << 20,
            },
        ))
        .await;
    assert!(matches!(phone.next_blob().await.body, Body::Offer(_)));
    phone
        .blob(BlobMessage::new(
            id,
            ns.clone(),
            Body::Accept {
                chunk_max: CHUNK_DEFAULT,
            },
        ))
        .await;
    let ping = phone.send_call("ping", json!({})).await;
    let mut chunks_before = 0;
    let mut chunks = 0;
    let mut answered = false;
    loop {
        match phone.pump().await {
            Got::Json(r) if r["id"] == ping => {
                answered = true;
                chunks_before = chunks;
            }
            Got::Blob => match phone.blobs.pop_front().unwrap().body {
                Body::Chunk { .. } => chunks += 1,
                Body::End { .. } => break,
                other => panic!("{other:?}"),
            },
            _ => {}
        }
    }
    assert!(answered, "the ping was answered before the END");
    assert_eq!(chunks, 12, "3 MiB in 256 KiB chunks");
    assert!(
        chunks_before <= 2,
        "{chunks_before} CHUNKs went before the ping's answer"
    );
}

#[tokio::test]
async fn files_are_off_with_no_read_patterns() {
    let env = start("").await;
    let mut phone = env.device("s23", false).await;
    let namespaces = phone.hello["result"]["blob"]["namespaces"].clone();
    assert!(
        !namespaces.as_array().unwrap().contains(&json!("files")),
        "{namespaces}"
    );
    assert_eq!(
        phone.download("files", "README.md").await.unwrap_err(),
        Code::FILES_OFF
    );
    // A namespace nobody serves.
    assert_eq!(
        phone.download("dev.example.plugin", "x").await.unwrap_err(),
        Code::UNSUPPORTED
    );
}

#[tokio::test]
async fn git_show_is_served_whatever_read_says() {
    let Some(git) = flankr::git::binary() else {
        return;
    };
    let env = start("").await;
    let sh = |args: &[&str]| {
        let ok = std::process::Command::new(git)
            .arg("-C")
            .arg(&env.project)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.com")
            .output()
            .unwrap();
        assert!(ok.status.success(), "{args:?}");
    };
    sh(&["init", "-q", "-b", "main"]);
    sh(&["add", "src/main.rs"]);
    sh(&["commit", "-q", "-m", "first"]);
    let mut phone = env.device("s23", false).await;
    let (offer, bytes) = phone
        .download("git", "show HEAD -- src/main.rs")
        .await
        .unwrap();
    assert_eq!(offer.mime, "text/x-diff");
    assert_eq!(offer.name, "show HEAD -- src/main.rs");
    assert_eq!(offer.modified_ms, 0);
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("+fn main() {}"), "{text}");
    assert_eq!(
        phone.download("git", "show main").await.unwrap_err(),
        Code::BAD_REQUEST
    );
}

#[tokio::test]
async fn agents_send_with_parts() {
    let env = start(READ).await;
    let mut phone = env.device("s23", false).await;
    let r1 = phone.upload("p1", &png(1)).await.unwrap();
    let r2 = phone.upload("p1", &png(2)).await.unwrap();
    let send = |parts: Value| json!({"pane_id": "p1", "command": "", "kind": "message", "confirmed": true, "parts": parts});
    // Image first, quoted; the text in its place.
    let r = phone
        .call(
            "agents.send",
            send(json!([{"image": r1}, {"text": "  what is   this"}, {"image": r2}])),
        )
        .await;
    assert_eq!(r["result"]["status"], "sent", "{r}");
    let log = env.mock(|m| m.command_log.clone()).await;
    let (pane, line) = log.last().unwrap().clone();
    assert_eq!(pane, "p1");
    let words: Vec<&str> = line.split(' ').collect();
    assert_eq!(words.len(), 5, "{line}");
    let dir = env.pane_dir("p1").display().to_string();
    assert!(
        words[0].starts_with(&format!("\"{dir}/")) && words[0].ends_with(".png\""),
        "{line}"
    );
    assert_eq!(&words[1..4], ["what", "is", "this"]);
    assert!(
        words[4].starts_with(&dir) && !words[4].contains('"'),
        "{line}"
    );
    // A reference is single-use.
    let r = phone
        .call("agents.send", send(json!([{"image": r1}])))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    // An image-only send to an agent without allow_messages is refused.
    let r3 = phone.upload("p2", &png(3)).await.unwrap();
    let r = phone
        .call(
            "agents.send",
            json!({"pane_id": "p2", "command": "", "kind": "message", "confirmed": true,
                   "parts": [{"image": r3}]}),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("allow_messages")
    );
    // Another pane's reference, and another principal's.
    let r4 = phone.upload("p1", &png(4)).await.unwrap();
    let r = phone
        .call(
            "agents.send",
            json!({"pane_id": "p2", "command": "", "kind": "message", "confirmed": true,
                   "parts": [{"image": r4}]}),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    let mut tablet = env.device("tablet", false).await;
    let r = tablet
        .call("agents.send", send(json!([{"image": r4}])))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    // A typed path into the temp folder is still outside the project.
    let r = phone
        .call(
            "agents.send",
            send(json!([{"text": format!("see {dir}/x.png")}, {"image": r4}])),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("outside the project"),
        "{r}"
    );
    // Without the tap: a confirmation whose card shows [image N].
    let r = phone
        .call(
            "agents.send",
            json!({"pane_id": "p1", "command": "", "kind": "message",
                   "parts": [{"text": "look at"}, {"image": r4}]}),
        )
        .await;
    assert_eq!(r["result"]["status"], "pending", "{r}");
    // parts with a command is invalid.
    let r = phone
        .call(
            "agents.send",
            json!({"pane_id": "p1", "command": "continue", "kind": "message",
                   "parts": [{"text": "x"}]}),
        )
        .await;
    assert_eq!(code(&r), codes::INVALID_PARAMS, "{r}");
}

#[tokio::test]
async fn terminal_paste_without_the_attachment_types_literal_keys() {
    let env = start(READ).await;
    let mut phone = env.device("s23", true).await;
    let opened = phone.call("terminal.open", json!({"pane_id": "p1"})).await;
    let tid = opened["result"]["terminal_id"].as_u64().unwrap();
    // Not armed yet.
    let r = phone
        .call("terminal.paste", json!({"terminal_id": tid, "text": "ls"}))
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    let r = phone
        .call("terminal.input_arm", json!({"terminal_id": tid}))
        .await;
    assert!(r.get("result").is_some(), "{r}");
    let r = phone
        .call(
            "terminal.paste",
            json!({"terminal_id": tid, "text": "ls -la"}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let keys = env.mock(|m| m.keystroke_log.clone()).await;
    assert_eq!(
        keys.last().unwrap(),
        &("p1".to_string(), "l s space - l a".to_string())
    );
    // Multi-line needs the attachment; both or neither is invalid.
    let r = phone
        .call(
            "terminal.paste",
            json!({"terminal_id": tid, "text": "a\r\nb"}),
        )
        .await;
    assert_eq!(code(&r), codes::NOT_PERMITTED, "{r}");
    let r = phone
        .call("terminal.paste", json!({"terminal_id": tid}))
        .await;
    assert_eq!(code(&r), codes::INVALID_PARAMS, "{r}");
    // Images: their paths, joined by a space.
    let reference = phone.upload("p1", &png(9)).await.unwrap();
    let before = env.mock(|m| m.keystroke_log.len()).await;
    let r = phone
        .call(
            "terminal.paste",
            json!({"terminal_id": tid, "images": [reference]}),
        )
        .await;
    assert_eq!(r["result"], json!({}), "{r}");
    let typed: String = env
        .mock(|m| m.keystroke_log[before..].to_vec())
        .await
        .into_iter()
        .map(|(_, k)| k.replace(' ', "").replace("space", " "))
        .collect();
    assert!(
        typed.starts_with(&env.pane_dir("p1").display().to_string()),
        "{typed}"
    );
    assert!(typed.ends_with(".png"), "{typed}");
    // Used.
    let r = phone
        .call(
            "terminal.paste",
            json!({"terminal_id": tid, "images": [reference]}),
        )
        .await;
    assert_eq!(code(&r), codes::INVALID_PARAMS, "{r}");
}
