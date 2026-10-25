//! The link end to end (plan 20261022_02, step 3, and the amendments of
//! 18:20 and 23:07 EDT): pairing by public keys over TCP on 127.0.0.1 with
//! an ephemeral port, in a temporary config directory, over the herdr mock.
//! Nothing touches `~/.config/flankr` or a live herdr.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use flankr::config::AppConfig;
use flankr::server::companion::TurnOptions;
use flankr::server::link::auth::{AuthFile, format_key};
use flankr::server::link::{Link, LinkSettings};
use flankr::server::{self, Daemon, ServeOptions};
use flankr::sessions::Sessions;
use flankr::transport::HerdrClient;
use futures_util::StreamExt;
use libflankr_proto::link::{
    DeviceCode, LinkStream, PairingPayload, StaticKeypair, generate_keypair, reconnect,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
    // Sessions read, and approvals journalled, under a temporary home,
    // never this machine's own.
    let daemon = Daemon::with_turns(
        HerdrClient::mock(),
        &test_config(),
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

/// `flankr pair`, as a daemon-socket client.
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
                json!({"protocol": "flankr.harness", "version": 1, "client": "pair"}),
            )
            .await;
        assert_eq!(r["result"]["server"], "flankr", "{r}");
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

    /// `link.pair_begin`, returning its result.
    async fn begin(&mut self) -> Value {
        let r = self.call("link.pair_begin", json!({})).await;
        assert!(r["result"]["pairing"].is_string(), "{r}");
        r["result"].clone()
    }

    /// `link.pair_add`, returning the whole response.
    async fn add(&mut self, params: Value) -> Value {
        self.call("link.pair_add", params).await
    }
}

/// A device: its key pair, and its code for `server`, named `name`.
fn device(server: &[u8; 32], name: &str) -> (StaticKeypair, String) {
    let keypair = generate_keypair().unwrap();
    let code = DeviceCode {
        key: keypair.public,
        name: name.to_string(),
    }
    .encode(server)
    .unwrap();
    (keypair, code)
}

fn hello_bytes() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": {"protocol": "flankr.harness", "version": 1, "client": "test"}
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

/// Connect as an added device for the first time: the hello reply says
/// `paired: true`.
async fn first_connect(env: &Env, keypair: &StaticKeypair) -> LinkStream<TcpStream> {
    let mut stream = reconnect_with(env, &keypair.private, &hello_bytes())
        .await
        .unwrap();
    let reply = recv(&mut stream).await;
    assert_eq!(reply["result"]["paired"], true, "{reply}");
    stream
}

/// The whole flow, as `flankr pair` and the device run it: the code is
/// added, the device connects, and `link.pair_done` arrives. Returns the
/// stream and the device's key pair.
async fn pair_device(
    env: &Env,
    local: &mut Local,
    name: &str,
    protocols: &[&str],
) -> (LinkStream<TcpStream>, StaticKeypair) {
    let (keypair, code) = device(&env.link.public_key(), name);
    let r = local
        .add(json!({"code": code, "protocols": protocols}))
        .await;
    assert_eq!(r["result"]["name"], name, "{r}");
    let stream = first_connect(env, &keypair).await;
    let done = local.note("link.pair_done").await;
    assert_eq!(done["params"], json!({"name": name, "paired": true}));
    (stream, keypair)
}

async fn reconnect_with(
    env: &Env,
    private: &[u8; 32],
    first: &[u8],
) -> Result<LinkStream<TcpStream>, libflankr_proto::link::LinkError> {
    let key = StaticKeypair::from_private(*private).unwrap();
    let tcp = TcpStream::connect(&env.address).await.unwrap();
    reconnect(tcp, &key, &env.link.public_key(), Some(first)).await
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
    let begun = local.begin().await;
    let payload = PairingPayload::decode(begun["pairing"].as_str().unwrap()).unwrap();
    assert_eq!(payload.addresses, vec![env.address.clone()]);
    assert_eq!(payload.server_key, env.link.public_key());
    assert_eq!(begun["addresses"], json!([env.address]));
    assert_eq!(
        begun["url"],
        format!(
            "flankr://{}/pair#{}",
            env.address,
            begun["pairing"].as_str().unwrap()
        )
    );

    // Before its code is added, the device's IK gets no reply.
    let (keypair, code) = device(&payload.server_key, "s23");
    assert!(
        reconnect_with(&env, &keypair.private, &hello_bytes())
            .await
            .is_err()
    );

    let r = local
        .add(json!({"code": code, "protocols": ["harness"]}))
        .await;
    assert_eq!(
        r["result"],
        json!({"name": "s23", "protocols": ["harness"]}),
        "{r}"
    );
    let mut paired = first_connect(&env, &keypair).await;
    let done = local.note("link.pair_done").await;
    assert_eq!(done["params"], json!({"name": "s23", "paired": true}));

    // auth.toml gained the entry.
    let file = env.auth();
    let entry = file.find_name("s23").unwrap();
    assert_eq!(entry.key, format_key(&keypair.public));
    assert_eq!(entry.protocols, vec!["harness"]);
    let r = call(&mut paired, 1, "agents.list").await;
    assert!(r["result"]["agents"].as_array().is_some(), "{r}");
    drop(paired);

    // Reconnecting with IK, hello as the first message, lists the agents;
    // it is no longer the first hello.
    let mut stream = reconnect_with(&env, &keypair.private, &hello_bytes())
        .await
        .unwrap();
    let hello = recv(&mut stream).await;
    assert_eq!(hello["result"]["server"], "flankr", "{hello}");
    assert!(hello["result"].get("paired").is_none(), "{hello}");
    let r = call(&mut stream, 1, "agents.list").await;
    let agents = r["result"]["agents"].as_array().unwrap();
    assert!(agents.iter().any(|a| a["agent"] == "claude"), "{r}");
    // A paired client may neither begin pairings nor add devices.
    let r = call(&mut stream, 2, "link.pair_begin").await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    let r = call(&mut stream, 3, "link.pair_add").await;
    assert_eq!(r["error"]["code"], -32001, "{r}");

    // A first message other than hello closes the connection unanswered.
    let send = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 0, "method": "agents.send",
        "params": {"pane_id": "p1", "command": "continue"}
    }))
    .unwrap();
    let mut replayed = reconnect_with(&env, &keypair.private, &send).await.unwrap();
    assert!(closes(&mut replayed, WAIT).await);

    // A key that is not paired gets no reply.
    let stranger = generate_keypair().unwrap();
    assert!(
        reconnect_with(&env, &stranger.private, &hello_bytes())
            .await
            .is_err()
    );

    // Deleting the entry closes the open connection.
    std::fs::write(env.auth_path(), "").unwrap();
    assert!(closes(&mut stream, Duration::from_secs(3)).await);
    assert_eq!(env.link.open_connections(), 0);
    assert!(
        reconnect_with(&env, &keypair.private, &hello_bytes())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn codes_for_another_server_mistyped_codes_and_server_urls_are_refused() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    let begun = local.begin().await;

    let other = generate_keypair().unwrap();
    let (_, foreign) = device(&other.public, "s23");
    let (_, good) = device(&env.link.public_key(), "s23");
    let at = good.len() - 5;
    let swapped = if good.as_bytes()[at] == b'2' {
        "3"
    } else {
        "2"
    };
    let mut mistyped = good.clone();
    mistyped.replace_range(at..at + 1, swapped);
    for (code, why) in [
        (foreign, "another server"),
        (mistyped, "mistyped"),
        (begun["url"].as_str().unwrap().to_string(), "flankrkey1"),
        (begun["pairing"].as_str().unwrap().to_string(), "flankrkey1"),
    ] {
        let r = local.add(json!({"code": code})).await;
        assert_eq!(r["error"]["code"], -32602, "{r}");
        let message = r["error"]["message"].as_str().unwrap();
        assert!(message.contains(why), "{message}");
    }
    // A bad --name is refused too.
    let r = local.add(json!({"code": good, "name": "has space"})).await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
    assert!(!env.auth_path().exists());
}

#[tokio::test]
async fn a_0x01_connection_is_closed_with_no_reply() {
    let env = start().await;
    let mut tcp = TcpStream::connect(&env.address).await.unwrap();
    // What the removed XXpsk3 pairing sent: the mode, a pairing id and `e`.
    let mut hello = vec![0x01, 1, 2, 3, 4, 0, 32];
    hello.extend_from_slice(&[7; 32]);
    tcp.write_all(&hello).await.unwrap();
    // Closed with nothing sent back: an end of stream, or a reset since
    // the server left bytes unread.
    let mut reply = Vec::new();
    let read = tokio::time::timeout(WAIT, tcp.read_to_end(&mut reply))
        .await
        .expect("closed in time");
    match read {
        Ok(_) => {}
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::ConnectionReset, "{e}"),
    }
    assert!(reply.is_empty());
}

/// What a phone reads from the pairing QR code's PNG.
fn scan(png_bytes: &[u8]) -> String {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().unwrap();
    let mut luma = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut luma).unwrap();
    let side = frame.width as usize;
    let mut img =
        rqrr::PreparedImage::prepare_from_greyscale(side, side, |x, y| luma[y * side + x]);
    let grids = img.detect_grids();
    assert_eq!(grids.len(), 1);
    grids[0].decode().unwrap().1
}

#[tokio::test]
async fn pairing_from_the_scanned_qr_code() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    let begun = local.begin().await;
    let shown = flankr::pair::Shown::new(begun["pairing"].as_str().unwrap()).unwrap();
    let url = scan(&shown.png);
    assert_eq!(url, shown.url);
    assert_eq!(url, begun["url"]);
    assert!(url.starts_with(&format!("flankr://{}/pair#flankr1", env.address)));

    // The phone has only what it scanned.
    let payload = PairingPayload::decode(&url).unwrap();
    let (keypair, code) = device(&payload.server_key, "s23");
    let r = local
        .add(json!({"code": code, "protocols": ["harness"]}))
        .await;
    assert_eq!(r["result"]["name"], "s23", "{r}");
    let tcp = TcpStream::connect(&payload.addresses[0]).await.unwrap();
    let mut stream = reconnect(tcp, &keypair, &payload.server_key, Some(&hello_bytes()))
        .await
        .unwrap();
    assert_eq!(recv(&mut stream).await["result"]["paired"], true);
    let done = local.note("link.pair_done").await;
    assert_eq!(done["params"], json!({"name": "s23", "paired": true}));
}

#[tokio::test]
async fn a_name_in_use_is_refused_and_replace_swaps_it() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    let (_first, old_key) = pair_device(&env, &mut local, "s23", &["harness"]).await;
    let mut old = reconnect_with(&env, &old_key.private, &hello_bytes())
        .await
        .unwrap();
    recv(&mut old).await;

    // Without replace, the name is refused and auth.toml is unchanged.
    let before = std::fs::read_to_string(env.auth_path()).unwrap();
    let (new_key, code) = device(&env.link.public_key(), "s23");
    let r = local
        .add(json!({"code": code, "protocols": ["harness"]}))
        .await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
    assert!(
        r["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already paired")
    );
    assert_eq!(std::fs::read_to_string(env.auth_path()).unwrap(), before);

    // With it, the new key replaces the old, whose connection closes.
    let r = local
        .add(json!({"code": code, "protocols": ["harness"], "replace": true}))
        .await;
    assert_eq!(r["result"]["name"], "s23", "{r}");
    assert!(closes(&mut old, Duration::from_secs(1)).await);
    let file = env.auth();
    assert_eq!(file.clients.len(), 1);
    assert_eq!(
        file.find_name("s23").unwrap().key,
        format_key(&new_key.public)
    );
    assert!(
        reconnect_with(&env, &old_key.private, &hello_bytes())
            .await
            .is_err()
    );
    let _new = first_connect(&env, &new_key).await;
    let done = local.note("link.pair_done").await;
    assert_eq!(done["params"], json!({"name": "s23", "paired": true}));
}

#[tokio::test]
async fn a_key_already_paired_under_another_name_is_refused() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    let (_, code) = device(&env.link.public_key(), "s23");
    let r = local.add(json!({"code": code})).await;
    assert_eq!(r["result"]["name"], "s23", "{r}");
    let before = std::fs::read_to_string(env.auth_path()).unwrap();

    // The same device, under --name tablet: refused, naming s23.
    let r = local.add(json!({"code": code, "name": "tablet"})).await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let message = r["error"]["message"].as_str().unwrap();
    assert!(message.contains("'s23'"), "{message}");
    assert_eq!(std::fs::read_to_string(env.auth_path()).unwrap(), before);

    // --name pairs a new device under a name of the user's choosing.
    let (_, other) = device(&env.link.public_key(), "s23");
    let r = local.add(json!({"code": other, "name": "tablet"})).await;
    assert_eq!(
        r["result"],
        json!({"name": "tablet", "protocols": ["companion"]}),
        "{r}"
    );
    assert_eq!(env.auth().clients.len(), 2);
}

#[tokio::test]
async fn hello_is_limited_to_the_entrys_protocols() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    // Paired for the companion only (the default).
    let (keypair, code) = device(&env.link.public_key(), "phone");
    let r = local.add(json!({"code": code})).await;
    assert_eq!(r["result"]["protocols"], json!(["companion"]), "{r}");
    let mut stream = reconnect_with(&env, &keypair.private, &hello_bytes())
        .await
        .unwrap();
    // A harness hello from a companion-only device is not permitted.
    let r = recv(&mut stream).await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    assert_eq!(
        env.auth().find_name("phone").unwrap().protocols,
        vec!["companion"]
    );

    // flankr.companion is served (plan 02, step 4), with the session of
    // the paired name; it is the first hello answered, so it says paired.
    let companion = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 0, "method": "hello",
        "params": {"protocol": "flankr.companion", "version": 1, "client": "phone"}
    }))
    .unwrap();
    let mut stream = reconnect_with(&env, &keypair.private, &companion)
        .await
        .unwrap();
    let r = recv(&mut stream).await;
    assert_eq!(r["result"]["session_id"], "companion:phone", "{r}");
    assert!(r["result"]["vocabulary"].is_array(), "{r}");
    assert_eq!(r["result"]["paired"], true, "{r}");
    let done = local.note("link.pair_done").await;
    assert_eq!(done["params"], json!({"name": "phone", "paired": true}));
}

// link.unpair (plan 20261024_02, decision 6 and step 3).

#[tokio::test]
async fn a_paired_device_unpairs_itself_and_its_key_no_longer_connects() {
    use std::os::unix::fs::PermissionsExt;
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    let (mut s23, keypair) = pair_device(&env, &mut local, "s23", &["harness"]).await;
    let (_t3, _) = pair_device(&env, &mut local, "t3", &["harness"]).await;
    assert_eq!(env.auth().clients.len(), 2);

    // Answered `{}`, then the link closes.
    let r = call(&mut s23, 1, "link.unpair").await;
    assert_eq!(r["result"], json!({}), "{r}");
    assert!(closes(&mut s23, Duration::from_secs(3)).await);

    // auth.toml no longer lists it; the other device stays.
    let file = env.auth();
    assert!(file.find_name("s23").is_none());
    assert!(file.find_key(&keypair.public).is_none());
    assert!(file.find_name("t3").is_some());
    let mode = std::fs::metadata(env.auth_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);

    // Its key gets no reply.
    assert!(
        reconnect_with(&env, &keypair.private, &hello_bytes())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn link_unpair_is_refused_on_the_daemon_socket() {
    let env = start().await;
    let mut local = Local::connect(&env.socket).await;
    let (_s23, _) = pair_device(&env, &mut local, "s23", &["harness"]).await;
    let r = local.call("link.unpair", json!({})).await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    assert_eq!(env.auth().clients.len(), 1);
}

/// The config the tests run with: no card summaries (`[summary] cards`),
/// so none reaches Gemini.
fn test_config() -> AppConfig {
    AppConfig {
        summary: flankr::config::SummaryConfig { cards: false },
        ..AppConfig::default()
    }
}
