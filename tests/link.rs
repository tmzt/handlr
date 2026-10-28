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
    start_with(Duration::from_millis(200), None).await
}

/// A link checking revocations every `revoke_interval`, with the file
/// watcher `file_watcher` (`None`: the platform's).
async fn start_with(
    revoke_interval: Duration,
    file_watcher: Option<Result<flankr::watch_files::FileWatcher, String>>,
) -> Env {
    let dir = tempfile::Builder::new()
        .prefix("hl")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("h.sock");
    let settings = LinkSettings {
        key_path: dir.path().join("link_key"),
        auth_path: dir.path().join("auth.toml"),
        listen: Some(vec!["127.0.0.1:0".to_string()]),
        revoke_interval,
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
            file_watcher,
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

/// A paired device's open connection, on `env`.
async fn paired_connection(env: &Env) -> LinkStream<TcpStream> {
    let mut local = Local::connect(&env.socket).await;
    let begun = local.begin().await;
    let payload = PairingPayload::decode(begun["pairing"].as_str().unwrap()).unwrap();
    let (keypair, code) = device(&payload.server_key, "s23");
    let r = local
        .add(json!({"code": code, "protocols": ["harness"]}))
        .await;
    assert!(r["result"].is_object(), "{r}");
    let mut paired = first_connect(env, &keypair).await;
    let r = call(&mut paired, 1, "agents.list").await;
    assert!(r["result"]["agents"].as_array().is_some(), "{r}");
    paired
}

/// Plan 20261027_05, step 1: auth.toml's change closes a revoked key's
/// connection at once, long before the 60 s fallback.
#[tokio::test]
async fn a_change_to_auth_toml_closes_a_revoked_connection_through_the_file_watcher() {
    let env = start_with(Duration::from_secs(60), None).await;
    let mut paired = paired_connection(&env).await;
    // Let FSEvents' report of the pairing's write pass.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(env.link.open_connections(), 1);
    std::fs::write(env.auth_path(), "").unwrap();
    assert!(closes(&mut paired, Duration::from_secs(2)).await);
    assert_eq!(env.link.open_connections(), 0);
}

/// When the file watcher can't start, the revoke interval still closes it.
#[tokio::test]
async fn without_the_file_watcher_the_revoke_interval_takes_over() {
    let env = start_with(
        Duration::from_millis(200),
        Some(Err("no inotify instances".to_string())),
    )
    .await;
    let mut paired = paired_connection(&env).await;
    std::fs::write(env.auth_path(), "").unwrap();
    assert!(closes(&mut paired, Duration::from_secs(3)).await);
    assert_eq!(env.link.open_connections(), 0);
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
        summary: Some(flankr::config::SummaryConfig { cards: false }),
        ..AppConfig::default()
    }
}

// Pair mode: the six-word pairing (plan 20261028_01, the amendments of
// 14:37, 14:40, 14:42 and 14:44 EDT). The phone is played over the
// loopback link with libflankr_proto's NK request.

mod pair_mode {
    use super::*;
    use libflankr_proto::link::{
        LinkError, OFFER_LEN, PairAck, PairRefusal, PairRequest, PairResponse, PairResult,
        pairing_words_text, recv_pair_result, request_pairing,
    };

    type Response = Result<PairResponse<TcpStream>, LinkError>;

    /// `link.pair_begin` with `params`: the payload, with its offer, and
    /// the result.
    async fn begin(local: &mut Local, params: Value) -> (PairingPayload, Value) {
        let r = local.call("link.pair_begin", params).await;
        let result = r["result"].clone();
        let payload = PairingPayload::decode(result["pairing"].as_str().expect("pairing")).unwrap();
        assert!(payload.offer.is_some(), "{r}");
        (payload, result)
    }

    /// The phone: a new key, and a pairing request with `offer`.
    async fn request(env: &Env, name: &str, offer: [u8; OFFER_LEN]) -> (StaticKeypair, Response) {
        let keypair = generate_keypair().unwrap();
        let tcp = TcpStream::connect(&env.address).await.unwrap();
        let response = tokio::time::timeout(
            WAIT,
            request_pairing(
                tcp,
                &env.link.public_key(),
                &PairRequest {
                    device_key: keypair.public,
                    name: name.to_string(),
                    offer,
                },
            ),
        )
        .await
        .expect("an answer in time");
        (keypair, response)
    }

    fn ack(response: &Response) -> PairAck {
        response.as_ref().map(|r| r.ack).expect("an ack")
    }

    fn has_key(env: &Env, key: &[u8; 32]) -> bool {
        env.auth_path().exists() && env.auth().find_key(key).is_some()
    }

    async fn result_of(response: Response) -> PairResult {
        let mut session = response.unwrap().stream;
        tokio::time::timeout(WAIT, recv_pair_result(&mut session))
            .await
            .expect("a result in time")
            .unwrap()
    }

    #[tokio::test]
    async fn a_request_with_the_offer_is_added_only_after_yes() {
        let env = start().await;
        let mut local = Local::connect(&env.socket).await;
        let (payload, begun) = begin(&mut local, json!({})).await;
        assert_eq!(begun["expires_in_secs"], 60, "{begun}");
        let offer = payload.offer.unwrap();

        let (phone, response) = request(&env, "s23", offer).await;
        let PairAck::Accepted { expires_in_secs } = ack(&response) else {
            panic!("refused: {:?}", ack(&response));
        };
        assert!((58..=60).contains(&expires_in_secs), "{expires_in_secs}");

        // Both sides show the same six words; nothing is written yet.
        let note = local.note("link.pair_request").await;
        let words = pairing_words_text(&payload.server_key, &offer, &phone.public);
        assert_eq!(note["params"]["name"], "s23", "{note}");
        assert_eq!(note["params"]["words"], words, "{note}");
        assert_eq!(note["params"]["via"], "link", "{note}");
        assert!(!has_key(&env, &phone.public));

        let r = local.call("link.pair_answer", json!({"yes": true})).await;
        assert_eq!(
            r["result"],
            json!({"added": true, "name": "s23", "protocols": ["companion"]}),
            "{r}"
        );
        assert_eq!(result_of(response).await, PairResult::Added);
        assert!(has_key(&env, &phone.public));

        // The phone then connects with IK: its first hello says paired.
        let companion = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 0, "method": "hello",
            "params": {"protocol": "flankr.companion", "version": 1, "client": "phone"}
        }))
        .unwrap();
        let mut stream = reconnect_with(&env, &phone.private, &companion)
            .await
            .unwrap();
        assert_eq!(recv(&mut stream).await["result"]["paired"], true);

        // The run is over: another request is refused.
        let (_, again) = request(&env, "t3", offer).await;
        assert_eq!(ack(&again), PairAck::Refused(PairRefusal::Busy));
    }

    #[tokio::test]
    async fn no_adds_nothing_and_ends_the_run() {
        let env = start().await;
        let mut local = Local::connect(&env.socket).await;
        let (payload, _) = begin(&mut local, json!({})).await;
        let offer = payload.offer.unwrap();
        let (phone, response) = request(&env, "s23", offer).await;
        local.note("link.pair_request").await;

        let r = local.call("link.pair_answer", json!({"yes": false})).await;
        assert_eq!(r["result"], json!({"added": false}), "{r}");
        assert_eq!(result_of(response).await, PairResult::Refused);
        assert!(!has_key(&env, &phone.public));
        assert!(!env.auth_path().exists());

        // One attempt per run: no second answer, no second request.
        let r = local.call("link.pair_answer", json!({"yes": true})).await;
        assert_eq!(r["error"]["code"], -32602, "{r}");
        let (_, again) = request(&env, "s23", offer).await;
        assert_eq!(ack(&again), PairAck::Refused(PairRefusal::Busy));
        assert!(!env.auth_path().exists());
    }

    #[tokio::test]
    async fn the_offer_expiring_drops_a_held_request() {
        let env = start().await;
        let mut local = Local::connect(&env.socket).await;
        let (payload, begun) = begin(&mut local, json!({"timeout_secs": 1})).await;
        assert_eq!(begun["expires_in_secs"], 1, "{begun}");
        let offer = payload.offer.unwrap();
        let (phone, response) = request(&env, "s23", offer).await;
        local.note("link.pair_request").await;

        let ended = local.note("link.pair_ended").await;
        assert_eq!(ended["params"], json!({"reason": "expired"}));
        assert_eq!(result_of(response).await, PairResult::Expired);

        // A yes after expiry adds nothing; the offer is refused as expired.
        let r = local.call("link.pair_answer", json!({"yes": true})).await;
        assert_eq!(r["error"]["code"], -32602, "{r}");
        let (_, late) = request(&env, "t3", offer).await;
        assert_eq!(ack(&late), PairAck::Refused(PairRefusal::Expired));
        assert!(!has_key(&env, &phone.public));
        assert!(!env.auth_path().exists());
    }

    #[tokio::test]
    async fn an_expired_offer_with_nothing_held_is_refused() {
        let env = start().await;
        let mut local = Local::connect(&env.socket).await;
        let (payload, _) = begin(&mut local, json!({"timeout_secs": 1})).await;
        let ended = local.note("link.pair_ended").await;
        assert_eq!(ended["params"], json!({"reason": "expired"}));
        let (_, late) = request(&env, "s23", payload.offer.unwrap()).await;
        assert_eq!(ack(&late), PairAck::Refused(PairRefusal::Expired));
        assert!(!env.auth_path().exists());
    }

    #[tokio::test]
    async fn the_phone_cancelling_or_dropping_ends_the_run() {
        let env = start().await;
        let mut local = Local::connect(&env.socket).await;
        let (payload, _) = begin(&mut local, json!({})).await;
        let offer = payload.offer.unwrap();
        let (phone, response) = request(&env, "s23", offer).await;
        local.note("link.pair_request").await;
        // Cancel on the phone closes its session.
        drop(response);
        let ended = local.note("link.pair_ended").await;
        assert_eq!(ended["params"], json!({"reason": "cancelled"}));
        let r = local.call("link.pair_answer", json!({"yes": true})).await;
        assert_eq!(r["error"]["code"], -32602, "{r}");
        assert!(!has_key(&env, &phone.public));
        let (_, again) = request(&env, "s23", offer).await;
        assert_eq!(ack(&again), PairAck::Refused(PairRefusal::Busy));
        assert!(!env.auth_path().exists());
    }

    #[tokio::test]
    async fn refused_requests_do_not_count_and_a_held_one_makes_others_busy() {
        let env = start().await;
        let mut local = Local::connect(&env.socket).await;
        let (payload, _) = begin(&mut local, json!({})).await;
        let offer = payload.offer.unwrap();

        // An offer from another run, or none at all: refused, not counted.
        let mut wrong = offer;
        wrong[0] ^= 1;
        let (_, refused) = request(&env, "mallory", wrong).await;
        assert_eq!(ack(&refused), PairAck::Refused(PairRefusal::WrongOffer));
        let (_, refused) = request(&env, "mallory", [0; OFFER_LEN]).await;
        assert_eq!(ack(&refused), PairAck::Refused(PairRefusal::WrongOffer));

        // The run's attempt is still open.
        let (phone, response) = request(&env, "s23", offer).await;
        assert!(matches!(ack(&response), PairAck::Accepted { .. }));
        let note = local.note("link.pair_request").await;
        assert_eq!(note["params"]["name"], "s23");

        // While it is held, any other request is busy, even with the offer.
        let (other, busy) = request(&env, "t3", offer).await;
        assert_eq!(ack(&busy), PairAck::Refused(PairRefusal::Busy));

        let r = local
            .call(
                "link.pair_answer",
                json!({"yes": true, "name": "phone", "protocols": ["harness"]}),
            )
            .await;
        assert_eq!(r["result"]["name"], "phone", "{r}");
        assert_eq!(result_of(response).await, PairResult::Added);
        let file = env.auth();
        assert_eq!(file.clients.len(), 1);
        assert_eq!(
            file.find_name("phone").unwrap().key,
            format_key(&phone.public)
        );
        assert!(file.find_key(&other.public).is_none());
    }

    #[tokio::test]
    async fn without_pair_mode_a_request_gets_no_reply() {
        let env = start().await;
        let (_, response) = request(&env, "s23", [1; OFFER_LEN]).await;
        assert!(response.is_err());

        // Pair mode lasts as long as the connection that began it.
        let mut local = Local::connect(&env.socket).await;
        let (payload, _) = begin(&mut local, json!({})).await;
        assert!(env.link.in_pair_mode());
        drop(local);
        let deadline = tokio::time::Instant::now() + WAIT;
        while env.link.in_pair_mode() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "pair mode stayed on"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let (_, response) = request(&env, "s23", payload.offer.unwrap()).await;
        assert!(response.is_err());
        assert!(!env.auth_path().exists());
    }

    #[tokio::test]
    async fn each_run_has_a_fresh_offer_and_replaces_the_last() {
        let env = start().await;
        let mut first = Local::connect(&env.socket).await;
        let (one, _) = begin(&mut first, json!({})).await;
        let mut second = Local::connect(&env.socket).await;
        let (two, _) = begin(&mut second, json!({})).await;
        assert_ne!(one.offer, two.offer);
        let ended = first.note("link.pair_ended").await;
        assert_eq!(ended["params"], json!({"reason": "replaced"}));
        let (_, stale) = request(&env, "s23", one.offer.unwrap()).await;
        assert_eq!(ack(&stale), PairAck::Refused(PairRefusal::WrongOffer));
        let (_, fresh) = request(&env, "s23", two.offer.unwrap()).await;
        assert!(matches!(ack(&fresh), PairAck::Accepted { .. }));
        second.note("link.pair_request").await;
        // The first run's connection cannot answer the second's request.
        let r = first.call("link.pair_answer", json!({"yes": true})).await;
        assert_eq!(r["error"]["code"], -32602, "{r}");
        assert!(!env.auth_path().exists());
    }

    #[tokio::test]
    async fn the_timeout_is_1_to_300_seconds() {
        let env = start().await;
        let mut local = Local::connect(&env.socket).await;
        for bad in [0, 301, 3600] {
            let r = local
                .call("link.pair_begin", json!({"timeout_secs": bad}))
                .await;
            assert_eq!(r["error"]["code"], -32602, "{r}");
            let message = r["error"]["message"].as_str().unwrap();
            assert!(message.contains("1 to 300"), "{message}");
        }
        let (_, begun) = begin(&mut local, json!({"timeout_secs": 300})).await;
        assert_eq!(begun["expires_in_secs"], 300);
        let (_, begun) = begin(&mut local, json!({"timeout_secs": 1})).await;
        assert_eq!(begun["expires_in_secs"], 1);
    }

    // `flankr pair` itself, with its input and output in memory.

    #[derive(Clone, Default)]
    struct Out(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Out {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Out {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }

        /// Waits until the output contains `part`.
        async fn until_contains(&self, part: &str) -> String {
            let deadline = tokio::time::Instant::now() + WAIT;
            loop {
                let text = self.text();
                if text.contains(part) {
                    return text;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "output never had {part:?}:\n{text}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        /// Waits until the output ends with `tail`.
        async fn until_ends_with(&self, tail: &str) -> String {
            let deadline = tokio::time::Instant::now() + WAIT;
            loop {
                let text = self.text();
                if text.ends_with(tail) {
                    return text;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "output never ended with {tail:?}:\n{text}"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }

    struct Run {
        out: Out,
        input: tokio::sync::mpsc::UnboundedSender<String>,
        exit: tokio::task::JoinHandle<Result<i32, String>>,
        /// The payload it printed.
        payload: PairingPayload,
    }

    const WAITING: &str =
        "Scan the QR code with the phone, or paste its device code here. The link expires in ";
    const PROMPT: &str = "Do the six words on the phone match? Type yes to pair: ";

    /// Starts `flankr pair` with `options`, waits for its link, and checks
    /// all it printed so far.
    async fn flankr_pair(env: &Env, options: flankr::pair::PairOptions) -> Run {
        let secs = options.timeout_secs.unwrap_or(60);
        let out = Out::default();
        let (input, lines) = tokio::sync::mpsc::unbounded_channel();
        let socket = env.socket.clone();
        let printed = out.clone();
        let exit =
            tokio::spawn(
                async move { flankr::pair::run_with(&socket, options, lines, printed).await },
            );
        let waiting = format!("{WAITING}{secs} s.\n");
        let text = out.until_contains(&waiting).await;
        let text = &text[..text.find(&waiting).unwrap() + waiting.len()];
        let url = text.lines().next().unwrap();
        let payload = PairingPayload::decode(url).unwrap();
        let shown = flankr::pair::Shown::new(url).unwrap();
        let server = env.link.public_key();
        assert_eq!(
            text,
            format!(
                "{}\n\n{}\n\nDaemon fingerprint: {} ({})\n\n{WAITING}{secs} s.\n",
                shown.url,
                shown.data_url(),
                libflankr_proto::link::fingerprint_text(&server),
                libflankr_proto::link::fingerprint_words_text(&server),
            )
        );
        Run {
            out,
            input,
            exit,
            payload,
        }
    }

    async fn exit_code(run: Run) -> i32 {
        tokio::time::timeout(WAIT, run.exit)
            .await
            .expect("flankr pair exits in time")
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn flankr_pair_prints_the_words_and_adds_on_yes() {
        let env = start().await;
        let run = flankr_pair(&env, flankr::pair::PairOptions::default()).await;
        let offer = run.payload.offer.unwrap();
        let (phone, response) = request(&env, "s23", offer).await;
        let words = pairing_words_text(&env.link.public_key(), &offer, &phone.public);
        let text = run.out.until_ends_with(PROMPT).await;
        assert!(
            text.ends_with(&format!(
                " s.\nPairing request from s23\nPairing words: {words}\n{PROMPT}"
            )),
            "{text}"
        );
        assert!(!has_key(&env, &phone.public));
        run.input.send(" YES \n".to_string()).unwrap();
        let out = run.out.clone();
        assert_eq!(exit_code(run).await, 0);
        assert!(
            out.text()
                .ends_with(&format!("{PROMPT}Added s23 (companion)\n"))
        );
        assert_eq!(result_of(response).await, PairResult::Added);
        assert!(has_key(&env, &phone.public));
    }

    #[tokio::test]
    async fn flankr_pair_adds_nothing_without_yes() {
        let env = start().await;
        let run = flankr_pair(&env, flankr::pair::PairOptions::default()).await;
        let (phone, response) = request(&env, "s23", run.payload.offer.unwrap()).await;
        run.out.until_ends_with(PROMPT).await;
        run.input.send("y\n".to_string()).unwrap();
        let out = run.out.clone();
        assert_eq!(exit_code(run).await, 1);
        assert!(
            out.text()
                .ends_with(&format!("{PROMPT}Not paired: the answer was not yes.\n"))
        );
        assert_eq!(result_of(response).await, PairResult::Refused);
        assert!(!has_key(&env, &phone.public));
        assert!(!env.auth_path().exists());
    }

    #[tokio::test]
    async fn flankr_pair_just_exits_when_the_offer_expires() {
        let env = start().await;
        // Waiting for the phone: nothing more is printed.
        let options = flankr::pair::PairOptions {
            timeout_secs: Some(1),
            ..Default::default()
        };
        let run = flankr_pair(&env, options.clone()).await;
        let out = run.out.clone();
        assert_eq!(exit_code(run).await, 1);
        assert!(out.text().ends_with(&format!("{WAITING}1 s.\n")));

        // At the prompt: the prompt line is ended, and nothing else.
        let run = flankr_pair(&env, options).await;
        let (phone, response) = request(&env, "s23", run.payload.offer.unwrap()).await;
        run.out.until_ends_with(PROMPT).await;
        let out = run.out.clone();
        assert_eq!(exit_code(run).await, 1);
        assert!(
            out.text().ends_with(&format!("{PROMPT}\n")),
            "{}",
            out.text()
        );
        assert_eq!(result_of(response).await, PairResult::Expired);
        assert!(!has_key(&env, &phone.public));
    }

    #[tokio::test]
    async fn flankr_pair_exits_when_the_phone_cancels() {
        let env = start().await;
        let run = flankr_pair(&env, flankr::pair::PairOptions::default()).await;
        let (phone, response) = request(&env, "s23", run.payload.offer.unwrap()).await;
        run.out.until_ends_with(PROMPT).await;
        drop(response);
        let out = run.out.clone();
        assert_eq!(exit_code(run).await, 1);
        assert!(out.text().ends_with(&format!("{PROMPT}\n")));
        assert!(!has_key(&env, &phone.public));
        assert!(!env.auth_path().exists());
    }

    #[tokio::test]
    async fn the_device_code_fallback_asks_for_yes_with_the_words() {
        let env = start().await;
        let options = flankr::pair::PairOptions {
            protocols: Some(vec!["harness".to_string()]),
            ..Default::default()
        };
        let run = flankr_pair(&env, options).await;
        let offer = run.payload.offer.unwrap();
        let server = run.payload.server_key;

        // A server address, an empty line and a foreign code don't count.
        run.input
            .send(format!("{}\n", run.payload.url().unwrap()))
            .unwrap();
        run.out
            .until_ends_with("paste it on the device, then paste the device's code here.\n")
            .await;
        run.input.send("\n".to_string()).unwrap();
        let other = generate_keypair().unwrap();
        let (_, foreign) = device(&other.public, "s23");
        run.input.send(format!("{foreign}\n")).unwrap();

        let (keypair, code) = device(&server, "s23");
        run.input.send(format!("{code}\n")).unwrap();
        let words = pairing_words_text(&server, &offer, &keypair.public);
        let text = run
            .out
            .until_ends_with(&format!(
                "Pairing request from s23\nPairing words: {words}\n{PROMPT}"
            ))
            .await;
        assert!(!text.contains("Confirmation code"), "{text}");
        assert!(!has_key(&env, &keypair.public));

        // While the code is held, a phone's request is busy.
        let (_, busy) = request(&env, "t3", offer).await;
        assert_eq!(ack(&busy), PairAck::Refused(PairRefusal::Busy));

        run.input.send("yes\n".to_string()).unwrap();
        let out = run.out.clone();
        assert_eq!(exit_code(run).await, 0);
        assert!(out.text().ends_with("Added s23 (harness)\n"));
        assert!(has_key(&env, &keypair.public));
        // The device connects with IK afterwards.
        let _stream = first_connect(&env, &keypair).await;
    }

    #[tokio::test]
    async fn a_substituted_device_code_shows_other_words_and_no_adds_nothing() {
        let env = start().await;
        let (keypair, _) = device(&env.link.public_key(), "s23");
        // An attacker's key, in a code built for this daemon.
        let (attacker, substituted) = device(&env.link.public_key(), "s23");
        let options = flankr::pair::PairOptions {
            code: Some(substituted),
            ..Default::default()
        };
        let run = flankr_pair(&env, options).await;
        let offer = run.payload.offer.unwrap();
        let shown = pairing_words_text(&env.link.public_key(), &offer, &attacker.public);
        let phone = pairing_words_text(&env.link.public_key(), &offer, &keypair.public);
        assert_ne!(shown, phone);
        run.out
            .until_ends_with(&format!("Pairing words: {shown}\n{PROMPT}"))
            .await;
        run.input.send("no\n".to_string()).unwrap();
        assert_eq!(exit_code(run).await, 1);
        assert!(!env.auth_path().exists());
    }
}
