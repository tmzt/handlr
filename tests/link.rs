//! The link end to end (plan 20261022_02, step 3, and the 18:20 EDT
//! amendment): pairing over TCP on 127.0.0.1 with an ephemeral port, in a
//! temporary config directory, over the herdr mock. Nothing touches
//! `~/.config/handlr` or a live herdr.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use handlr::config::AppConfig;
use handlr::server::link::auth::{format_key, AuthFile};
use handlr::server::link::{Link, LinkSettings};
use handlr::server::{self, Daemon, ServeOptions};
use handlr::transport::HerdrClient;
use libhandlr_proto::link::{generate_keypair, pair, reconnect, LinkStream, PairingPayload};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpStream, UnixStream};
use tokio_util::codec::{FramedRead, LinesCodec};

const WAIT: Duration = Duration::from_secs(5);

struct Env {
    dir: tempfile::TempDir,
    socket: PathBuf,
    address: String,
    link: Arc<Link>,
}

impl Env {
    fn auth_path(&self) -> PathBuf {
        self.dir.path().join("auth.toml")
    }

    fn auth(&self) -> AuthFile {
        AuthFile::load(&self.auth_path()).unwrap()
    }
}

async fn start() -> Env {
    let dir = tempfile::Builder::new()
        .prefix("hl")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("h.sock");
    let settings = LinkSettings {
        key_path: dir.path().join("link_key"),
        auth_path: dir.path().join("auth.toml"),
        listen: Some(vec!["127.0.0.1:0".to_string()]),
        revoke_interval: Duration::from_millis(200),
    };
    let link = Link::open(settings).unwrap();
    let listeners = link.bind().await.unwrap();
    let address = listeners[0].local_addr().unwrap().to_string();
    let daemon = Daemon::with_link(
        HerdrClient::mock(),
        &AppConfig::default(),
        Some(Arc::clone(&link)),
    );
    let unix = server::bind(&socket).await.unwrap();
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
        socket,
        address,
        link,
    }
}

/// `handlr pair`, as a daemon-socket client.
struct Local {
    write: OwnedWriteHalf,
    lines: FramedRead<OwnedReadHalf, LinesCodec>,
    next_id: i64,
    notes: Vec<Value>,
}

impl Local {
    async fn connect(socket: &Path) -> Local {
        let (read, write) = UnixStream::connect(socket).await.unwrap().into_split();
        let mut c = Local {
            write,
            lines: FramedRead::new(read, LinesCodec::new()),
            next_id: 0,
            notes: Vec::new(),
        };
        let r = c
            .call(
                "hello",
                json!({"protocol": "handlr.harness", "version": 1, "client": "pair"}),
            )
            .await;
        assert_eq!(r["result"]["server"], "handlr", "{r}");
        c
    }

    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(WAIT, self.lines.next())
            .await
            .expect("no reply in time")
            .expect("closed")
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let mut line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        self.write.write_all(line.as_bytes()).await.unwrap();
        loop {
            let msg = self.next().await;
            if msg.get("method").is_some() {
                self.notes.push(msg);
            } else if msg["id"] == id {
                return msg;
            }
        }
    }

    async fn note(&mut self, method: &str) -> Value {
        if let Some(i) = self.notes.iter().position(|n| n["method"] == method) {
            return self.notes.remove(i);
        }
        loop {
            let msg = self.next().await;
            if msg["method"] == method {
                return msg;
            }
            self.notes.push(msg);
        }
    }

    /// `link.pair_begin`, returning the pairing string.
    async fn begin(&mut self, params: Value) -> String {
        let r = self.call("link.pair_begin", params).await;
        r["result"]["pairing"]
            .as_str()
            .unwrap_or_else(|| panic!("{r}"))
            .to_string()
    }
}

fn hello_bytes() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": {"protocol": "handlr.harness", "version": 1, "client": "test"}
    }))
    .unwrap()
}

async fn recv(stream: &mut LinkStream<TcpStream>) -> Value {
    let message = tokio::time::timeout(WAIT, stream.recv_message())
        .await
        .expect("no message in time")
        .expect("the link closed");
    serde_json::from_slice(&message).unwrap()
}

async fn call(stream: &mut LinkStream<TcpStream>, id: i64, method: &str) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": {}});
    stream
        .send_message(&serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    loop {
        let msg = recv(stream).await;
        if msg["id"] == id {
            return msg;
        }
    }
}

/// Pair from `pairing` with a new key; returns the stream and private key.
async fn pair_with(
    pairing: &str,
) -> Result<(LinkStream<TcpStream>, [u8; 32]), libhandlr_proto::link::LinkError> {
    let payload = PairingPayload::decode(pairing).unwrap();
    let keypair = generate_keypair().unwrap();
    let tcp = TcpStream::connect(&payload.addresses[0]).await.unwrap();
    let (mut stream, server) = pair(
        tcp,
        &keypair.private,
        &payload.server_key,
        &payload.secret,
        Some(&hello_bytes()),
    )
    .await?;
    assert_eq!(server, payload.server_key);
    // The server closes a refused pairing; the hello reply shows it took.
    let reply = tokio::time::timeout(WAIT, stream.recv_message())
        .await
        .expect("no hello reply in time")?;
    let reply: Value = serde_json::from_slice(&reply).unwrap();
    assert_eq!(reply["result"]["paired"], true, "{reply}");
    Ok((stream, keypair.private))
}

async fn reconnect_with(
    env: &Env,
    private: &[u8; 32],
    first: &[u8],
) -> Result<LinkStream<TcpStream>, libhandlr_proto::link::LinkError> {
    let tcp = TcpStream::connect(&env.address).await.unwrap();
    reconnect(tcp, private, &env.link.public_key(), Some(first)).await
}

/// Whether the server closes `stream` within `within`.
async fn closes(stream: &mut LinkStream<TcpStream>, within: Duration) -> bool {
    match tokio::time::timeout(within, stream.recv_message()).await {
        Ok(Err(_)) => true,
        Ok(Ok(m)) => panic!("unexpected message {}", String::from_utf8_lossy(&m)),
        Err(_) => false,
    }
}

#[tokio::test]
async fn pairing_reconnecting_and_revoking() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    let pairing = local
        .begin(json!({"name": "s23", "protocols": ["harness"]}))
        .await;
    let payload = PairingPayload::decode(&pairing).unwrap();
    assert_eq!(payload.addresses, vec![env.address.clone()]);
    assert_eq!(payload.server_key, env.link.public_key());

    let (mut paired, private) = pair_with(&pairing).await.unwrap();
    let done = local.note("link.pair_done").await;
    assert_eq!(done["params"], json!({"name": "s23", "paired": true}));

    // auth.toml gained the entry.
    let file = env.auth();
    let entry = file.find_name("s23").unwrap();
    assert_eq!(
        entry.key,
        format_key(&libhandlr_proto::link::public_key(&private).unwrap())
    );
    assert_eq!(entry.protocols, vec!["harness"]);
    let r = call(&mut paired, 1, "agents.list").await;
    assert!(r["result"]["agents"].as_array().is_some(), "{r}");
    drop(paired);

    // Reconnecting with IK, hello as the first message, lists the agents.
    let mut stream = reconnect_with(&env, &private, &hello_bytes())
        .await
        .unwrap();
    let hello = recv(&mut stream).await;
    assert_eq!(hello["result"]["server"], "handlr", "{hello}");
    assert!(hello["result"].get("paired").is_none(), "{hello}");
    let r = call(&mut stream, 1, "agents.list").await;
    let agents = r["result"]["agents"].as_array().unwrap();
    assert!(agents.iter().any(|a| a["agent"] == "claude"), "{r}");
    // A paired client may not begin pairings.
    let r = call(&mut stream, 2, "link.pair_begin").await;
    assert_eq!(r["error"]["code"], -32001, "{r}");

    // A first message other than hello closes the connection unanswered.
    let send = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 0, "method": "agents.send",
        "params": {"pane_id": "p1", "command": "continue"}
    }))
    .unwrap();
    let mut replayed = reconnect_with(&env, &private, &send).await.unwrap();
    assert!(closes(&mut replayed, WAIT).await);

    // A key that is not paired gets no reply.
    let stranger = generate_keypair().unwrap();
    assert!(reconnect_with(&env, &stranger.private, &hello_bytes())
        .await
        .is_err());

    // Deleting the entry closes the open connection.
    std::fs::write(env.auth_path(), "").unwrap();
    assert!(closes(&mut stream, Duration::from_secs(3)).await);
    assert_eq!(env.link.open_connections(), 0);
    assert!(reconnect_with(&env, &private, &hello_bytes())
        .await
        .is_err());
}

#[tokio::test]
async fn expired_and_reused_secrets_are_refused() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;

    // Reused: the second use of a string finds nothing.
    let pairing = local
        .begin(json!({"name": "once", "protocols": ["harness"]}))
        .await;
    pair_with(&pairing).await.unwrap();
    assert!(pair_with(&pairing).await.is_err());
    assert_eq!(env.auth().clients.len(), 1);

    // Expired: refused, and `handlr pair` hears why.
    let pairing = local
        .begin(json!({"name": "late", "protocols": ["harness"], "ttl": 1}))
        .await;
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert!(pair_with(&pairing).await.is_err());
    let done = loop {
        let done = local.note("link.pair_done").await;
        if done["params"]["name"] == "late" {
            break done;
        }
    };
    assert_eq!(done["params"]["paired"], false);
    assert!(done["params"]["error"]
        .as_str()
        .unwrap()
        .contains("expired"));
    assert!(env.auth().find_name("late").is_none());
}

#[tokio::test]
async fn a_name_in_use_is_refused_and_replace_swaps_it() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    let pairing = local
        .begin(json!({"name": "s23", "protocols": ["harness"]}))
        .await;
    let (_first, old_private) = pair_with(&pairing).await.unwrap();
    let mut old = reconnect_with(&env, &old_private, &hello_bytes())
        .await
        .unwrap();
    recv(&mut old).await;

    // Without --replace, the name is refused and auth.toml is unchanged.
    let before = std::fs::read_to_string(env.auth_path()).unwrap();
    let r = local
        .call(
            "link.pair_begin",
            json!({"name": "s23", "protocols": ["harness"]}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
    assert!(r["error"]["message"]
        .as_str()
        .unwrap()
        .contains("already paired"));
    assert_eq!(std::fs::read_to_string(env.auth_path()).unwrap(), before);

    // With it, the new key replaces the old, whose connection closes.
    let pairing = local
        .begin(json!({"name": "s23", "protocols": ["harness"], "replace": true}))
        .await;
    let (_new, new_private) = pair_with(&pairing).await.unwrap();
    assert!(closes(&mut old, Duration::from_secs(1)).await);
    let file = env.auth();
    assert_eq!(file.clients.len(), 1);
    let new_key = libhandlr_proto::link::public_key(&new_private).unwrap();
    assert_eq!(file.find_name("s23").unwrap().key, format_key(&new_key));
    assert!(reconnect_with(&env, &old_private, &hello_bytes())
        .await
        .is_err());
    let mut again = reconnect_with(&env, &new_private, &hello_bytes())
        .await
        .unwrap();
    assert_eq!(recv(&mut again).await["result"]["server"], "handlr");
}

#[tokio::test]
async fn hello_is_limited_to_the_entrys_protocols() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    // Paired for the companion only (the default).
    let pairing = local.begin(json!({"name": "phone"})).await;
    let payload = PairingPayload::decode(&pairing).unwrap();
    let keypair = generate_keypair().unwrap();
    let tcp = TcpStream::connect(&payload.addresses[0]).await.unwrap();
    let (mut stream, _) = pair(
        tcp,
        &keypair.private,
        &payload.server_key,
        &payload.secret,
        Some(&hello_bytes()),
    )
    .await
    .unwrap();
    // A harness hello from a companion-only device is not permitted.
    let r = recv(&mut stream).await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    assert_eq!(
        env.auth().find_name("phone").unwrap().protocols,
        vec!["companion"]
    );

    // handlr.companion is served (plan 02, step 4), with the session of
    // the paired name.
    let companion = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": {"protocol": "handlr.companion", "version": 1, "client": "phone"}
    }))
    .unwrap();
    let mut stream = reconnect_with(&env, &keypair.private, &companion)
        .await
        .unwrap();
    let r = recv(&mut stream).await;
    assert_eq!(r["result"]["session_id"], "companion:phone", "{r}");
    assert!(r["result"]["vocabulary"].is_array(), "{r}");
}
