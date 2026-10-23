//! The `net` thread over handlr link, against an in-test server built from
//! `libhandlr_proto::link::accept` with a minimal JSON-RPC responder that
//! authorizes the keys in a set: pairing by device code (plan 20261022_02,
//! amendment of 23:07 EDT) from a string or the QR code's data URL (plan
//! 20261022_06, step 5), reconnecting with `IK` and the stored
//! `session_id`, refusals, the first message, and failing over between
//! addresses.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use base64::Engine;
use handlr_companion::{
    Companion, CompanionConfig, Event, FileKeyStore, Input, KeyStore, ServerRecord,
};
use libhandlr_proto::link::{self, DeviceCode, PairingPayload, StaticKeypair};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(5);

/// The device name the test companions ask to be paired as.
const DEVICE: &str = "test-device";

/// What the server saw of one accepted connection.
#[derive(Debug)]
struct Seen {
    peer_key: [u8; 32],
    first: Value,
}

/// A link server on 127.0.0.1, on its own thread and runtime.
struct Server {
    addr: String,
    key: StaticKeypair,
    seen: mpsc::Receiver<Seen>,
    /// TCP connections accepted, handshake or not.
    accepts: Arc<AtomicUsize>,
    authorized: Arc<Mutex<HashSet<[u8; 32]>>>,
    /// Keys added and not yet heard from: their `hello` says `paired`.
    fresh: Arc<Mutex<HashSet<[u8; 32]>>>,
    /// Connections still to be closed right after answering `hello`.
    close_after_hello: Arc<AtomicUsize>,
}

impl Server {
    fn start() -> Server {
        let key = link::generate_keypair().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (seen_tx, seen) = mpsc::channel();
        let accepts = Arc::new(AtomicUsize::new(0));
        let authorized = Arc::new(Mutex::new(HashSet::new()));
        let fresh = Arc::new(Mutex::new(HashSet::new()));
        let close_after_hello = Arc::new(AtomicUsize::new(0));
        let ctx = Ctx {
            key: key.clone(),
            seen: seen_tx,
            accepts: accepts.clone(),
            authorized: authorized.clone(),
            fresh: fresh.clone(),
            close_after_hello: close_after_hello.clone(),
        };
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    ctx.accepts.fetch_add(1, Ordering::SeqCst);
                    let ctx = ctx.clone();
                    tokio::spawn(async move { ctx.serve(stream).await });
                }
            });
        });
        Server {
            addr,
            key,
            seen,
            accepts,
            authorized,
            fresh,
            close_after_hello,
        }
    }

    /// The pairing string `handlr pair` would show for `addresses`.
    fn pairing(&self, addresses: Vec<String>) -> PairingPayload {
        PairingPayload {
            server_key: self.key.public,
            addresses,
        }
    }

    /// `link.pair_add`: checks the code against this server's key and
    /// authorizes its key. Returns the code.
    fn add(&self, code: &str) -> DeviceCode {
        let code = DeviceCode::decode(code, &self.key.public).expect("a code for this server");
        self.authorized.lock().unwrap().insert(code.key);
        self.fresh.lock().unwrap().insert(code.key);
        code
    }

    fn next_seen(&self) -> Seen {
        self.seen
            .recv_timeout(TIMEOUT)
            .expect("a connection within 5 s")
    }
}

#[derive(Clone)]
struct Ctx {
    key: StaticKeypair,
    seen: mpsc::Sender<Seen>,
    accepts: Arc<AtomicUsize>,
    authorized: Arc<Mutex<HashSet<[u8; 32]>>>,
    fresh: Arc<Mutex<HashSet<[u8; 32]>>>,
    close_after_hello: Arc<AtomicUsize>,
}

impl Ctx {
    async fn serve(self, stream: tokio::net::TcpStream) {
        let authorized = self.authorized.clone();
        let accepted = link::accept(stream, &self.key, |key| {
            authorized.lock().unwrap().contains(key)
        })
        .await;
        let Ok(accepted) = accepted else {
            return;
        };
        let paired = self.fresh.lock().unwrap().remove(&accepted.peer_key);
        let first: Value = serde_json::from_slice(&accepted.first_message.unwrap_or_default())
            .unwrap_or(Value::Null);
        let hello_id = first["id"].clone();
        let _ = self.seen.send(Seen {
            peer_key: accepted.peer_key,
            first,
        });
        let mut stream = accepted.stream;
        let mut hello = json!({"jsonrpc": "2.0", "id": hello_id, "result": {
            "server": "handlr", "server_version": "0.1.0", "protocol_version": 1,
            "session_id": "companion:phone", "vocabulary": ["claude"]
        }});
        if paired {
            hello["result"]["paired"] = json!(true);
        }
        if stream
            .send_message(&serde_json::to_vec(&hello).unwrap())
            .await
            .is_err()
        {
            return;
        }
        let close = self
            .close_after_hello
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if close {
            let _ = stream.shutdown().await;
            return;
        }
        while let Ok(message) = stream.recv_message().await {
            let request: Value = serde_json::from_slice(&message).unwrap();
            if request.get("id").is_none() {
                continue;
            }
            let result = match request["method"].as_str() {
                Some("turn.start") => json!({"outcome": "done", "ran": []}),
                _ => json!({}),
            };
            let response = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
            if stream
                .send_message(&serde_json::to_vec(&response).unwrap())
                .await
                .is_err()
            {
                return;
            }
        }
    }
}

/// An address where nothing listens.
fn closed_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// A companion with no daemon socket, keeping its keys in `dir/keys`.
fn companion(dir: &tempfile::TempDir) -> Companion {
    let mut config = CompanionConfig::new("test");
    config.device_name = DEVICE.to_string();
    config.socket = dir.path().join("no-socket");
    config.backoff_initial = Duration::from_millis(50);
    config.backoff_max = Duration::from_millis(400);
    Companion::spawn(config, Box::new(FileKeyStore::new(keys_dir(dir))))
}

fn keys_dir(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("keys")
}

/// The next event that is not a `Disconnected` from the attempts on the
/// missing socket.
fn next_event(c: &Companion) -> Event {
    loop {
        match c.wait_timeout(TIMEOUT).expect("an event within 5 s") {
            Event::Disconnected { .. } => continue,
            e => return e,
        }
    }
}

fn expect_connected(c: &Companion) {
    assert_eq!(
        next_event(c),
        Event::Connected {
            server: "handlr".into()
        }
    );
    assert_eq!(next_event(c), Event::Vocabulary(vec!["claude".into()]));
}

/// The handshake's first message is one JSON-RPC `hello` request and
/// nothing else. Returns its params.
fn assert_exact_hello(first: &Value) -> Value {
    let object = first
        .as_object()
        .expect("the first message is a JSON object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["id", "jsonrpc", "method", "params"], "{first}");
    assert_eq!(first["jsonrpc"], "2.0");
    assert_eq!(first["method"], "hello");
    assert_eq!(first["params"]["protocol"], "handlr.companion");
    assert_eq!(first["params"]["client"], "test");
    first["params"].clone()
}

/// The device code the companion shows, and the server it names.
fn expect_code(c: &Companion) -> (String, String) {
    match next_event(c) {
        Event::PairingCode { code, server } => (code, server),
        e => panic!("expected the pairing code, got {e:?}"),
    }
}

/// The events of the first connection after the code was added.
fn expect_paired(c: &Companion) {
    assert_eq!(
        next_event(c),
        Event::Paired {
            name: DEVICE.into()
        }
    );
    expect_connected(c);
}

#[test]
fn pairing_shows_the_code_and_connects_once_it_is_added() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let payload = server.pairing(vec![server.addr.clone()]);
    c.send(Input::Pair(payload.encode().unwrap()));

    let (code, name) = expect_code(&c);
    // A bare string: named after its first address's host.
    assert_eq!(name, "127.0.0.1");
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let key = keys
        .static_key()
        .unwrap()
        .expect("the client's key is stored");
    assert_eq!(
        keys.server().unwrap(),
        Some(ServerRecord {
            name: "127.0.0.1".into(),
            public_key: server.key.public,
            addresses: vec![server.addr.clone()],
            session_id: None,
        })
    );

    // Until the code is pasted, the server drops it, and it tries again.
    std::thread::sleep(Duration::from_millis(2500));
    assert!(server.accepts.load(Ordering::SeqCst) >= 2);
    assert!(server.seen.try_recv().is_err());

    let added = server.add(&code);
    assert_eq!(added.key, key.public());
    assert_eq!(added.name, DEVICE);
    expect_paired(&c);
    let seen = server.next_seen();
    assert_eq!(seen.peer_key, key.public());
    let hello = assert_exact_hello(&seen.first);
    assert!(hello.get("session_id").is_none(), "{hello}");
    assert_eq!(
        keys.server().unwrap().unwrap().session_id.as_deref(),
        Some("companion:phone")
    );

    // And the connection carries a turn.
    c.send(Input::Utterance("what is claude doing".into()));
    assert_eq!(
        next_event(&c),
        Event::TurnEnded {
            outcome: "done".into(),
            stopped: vec![]
        }
    );
}

/// Plan 20261022_06, step 5: the data URL `handlr pair` prints pairs.
#[test]
fn pairing_from_the_qr_codes_data_url() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let url = server.pairing(vec![server.addr.clone()]).url().unwrap();
    let png = qrcodeurl::png(&url).unwrap();
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    c.send(Input::Pair(data_url));

    let (code, name) = expect_code(&c);
    assert_eq!(name, "127.0.0.1");
    let added = server.add(&code);
    expect_paired(&c);
    assert_eq!(server.next_seen().peer_key, added.key);
}

#[test]
fn reconnecting_uses_ik_and_the_stored_session_id_and_fails_over() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let client = keys.generate_static_key().unwrap().public();
    let addresses = vec![closed_addr(), server.addr.clone()];
    keys.set_server(ServerRecord {
        name: "phone".into(),
        public_key: server.key.public,
        addresses: addresses.clone(),
        session_id: Some("companion:old".into()),
    })
    .unwrap();
    server.authorized.lock().unwrap().insert(client);
    server.close_after_hello.store(1, Ordering::SeqCst);

    let c = companion(&dir);
    let seen = server.next_seen();
    assert_eq!(seen.peer_key, client);
    assert_eq!(
        assert_exact_hello(&seen.first)["session_id"],
        "companion:old"
    );
    expect_connected(&c);

    // The server closes; the client comes back with the session_id that
    // hello gave, which is now stored.
    let seen = server.next_seen();
    assert_eq!(
        assert_exact_hello(&seen.first)["session_id"],
        "companion:phone"
    );
    expect_connected(&c);
    assert_eq!(
        keys.server().unwrap().unwrap(),
        ServerRecord {
            name: "phone".into(),
            public_key: server.key.public,
            addresses,
            session_id: Some("companion:phone".into()),
        }
    );
}

#[test]
fn pairing_fails_over_to_the_second_address() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let addresses = vec![closed_addr(), server.addr.clone()];
    c.send(Input::Pair(
        server.pairing(addresses.clone()).url().unwrap(),
    ));
    let (code, name) = expect_code(&c);
    // The URL names the first address that is not loopback, or the first.
    assert_eq!(name, "127.0.0.1");
    server.add(&code);
    expect_paired(&c);
    let record = FileKeyStore::new(keys_dir(&dir)).server().unwrap().unwrap();
    assert_eq!(record.addresses, addresses);
}

#[test]
fn a_code_is_for_one_server_and_keeps_the_key() {
    let server = Server::start();
    let other = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    c.send(Input::Pair(
        server.pairing(vec![server.addr.clone()]).encode().unwrap(),
    ));
    let (code, _) = expect_code(&c);
    assert!(DeviceCode::decode(&code, &other.key.public).is_err());

    // Pairing again, with another server, keeps the device's key.
    c.send(Input::Pair(
        other.pairing(vec![other.addr.clone()]).encode().unwrap(),
    ));
    let (again, _) = expect_code(&c);
    let first = DeviceCode::decode(&code, &server.key.public).unwrap();
    let second = DeviceCode::decode(&again, &other.key.public).unwrap();
    assert_eq!(first.key, second.key);
    other.add(&again);
    expect_paired(&c);
    assert_eq!(
        FileKeyStore::new(keys_dir(&dir))
            .server()
            .unwrap()
            .unwrap()
            .public_key,
        other.key.public
    );
}

#[test]
fn an_invalid_string_is_refused_before_connecting() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let mut text = server.pairing(vec![server.addr.clone()]).encode().unwrap();
    // Change one character, keeping it base58: the checksum fails.
    let last = text.pop().unwrap();
    text.push(if last == '2' { '3' } else { '2' });
    c.send(Input::Pair(text));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("invalid pairing string"), "{why}"),
        e => panic!("{e:?}"),
    }
    // A data URL whose QR code holds something else is refused too.
    let png = qrcodeurl::png("https://example.com/").unwrap();
    c.send(Input::Pair(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )));
    assert!(matches!(next_event(&c), Event::PairFailed(_)));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(server.accepts.load(Ordering::SeqCst), 0);
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    assert_eq!(keys.server().unwrap(), None);
    assert!(keys.static_key().unwrap().is_none());
}
