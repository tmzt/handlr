//! The `net` thread over handlr link, against an in-test server built from
//! `libhandlr_proto::link::accept` with a minimal JSON-RPC responder:
//! pairing, reconnecting with `IK` and the stored `session_id`, refusals,
//! the first message, and failing over between addresses.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use handlr_companion::{
    Companion, CompanionConfig, Event, FileKeyStore, Input, KeyStore, ServerRecord,
};
use libhandlr_proto::link::{self, AcceptedMode, PairingId, PairingPayload, StaticKeypair};
use serde_json::{json, Value};

const TIMEOUT: Duration = Duration::from_secs(5);

/// What the server saw of one accepted connection.
#[derive(Debug)]
struct Seen {
    mode: AcceptedMode,
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
    pending: Arc<Mutex<HashMap<PairingId, [u8; 32]>>>,
    authorized: Arc<Mutex<HashSet<[u8; 32]>>>,
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
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let authorized = Arc::new(Mutex::new(HashSet::new()));
        let close_after_hello = Arc::new(AtomicUsize::new(0));
        let ctx = Ctx {
            private: key.private,
            seen: seen_tx,
            accepts: accepts.clone(),
            pending: pending.clone(),
            authorized: authorized.clone(),
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
            pending,
            authorized,
            close_after_hello,
        }
    }

    /// A pending pairing, as `handlr pair` would make it.
    fn pairing(&self, name: &str, addresses: Vec<String>, expiry: u32) -> PairingPayload {
        let secret = link::generate_secret().unwrap();
        self.pending
            .lock()
            .unwrap()
            .insert(PairingId::from_secret(&secret), secret);
        PairingPayload {
            server_key: self.key.public,
            secret,
            expiry,
            name: name.to_string(),
            addresses,
        }
    }

    fn next_seen(&self) -> Seen {
        self.seen
            .recv_timeout(TIMEOUT)
            .expect("a connection within 5 s")
    }
}

#[derive(Clone)]
struct Ctx {
    private: [u8; 32],
    seen: mpsc::Sender<Seen>,
    accepts: Arc<AtomicUsize>,
    pending: Arc<Mutex<HashMap<PairingId, [u8; 32]>>>,
    authorized: Arc<Mutex<HashSet<[u8; 32]>>>,
    close_after_hello: Arc<AtomicUsize>,
}

impl Ctx {
    async fn serve(self, stream: tokio::net::TcpStream) {
        let pending = self.pending.clone();
        let authorized = self.authorized.clone();
        let accepted = link::accept(
            stream,
            &self.private,
            |id| pending.lock().unwrap().remove(&id),
            |key| authorized.lock().unwrap().contains(key),
        )
        .await;
        let Ok(accepted) = accepted else {
            return;
        };
        if let AcceptedMode::Pairing(_) = accepted.mode {
            self.authorized.lock().unwrap().insert(accepted.peer_key);
        }
        let first: Value = serde_json::from_slice(&accepted.first_message.unwrap_or_default())
            .unwrap_or(Value::Null);
        let hello_id = first["id"].clone();
        let _ = self.seen.send(Seen {
            mode: accepted.mode,
            peer_key: accepted.peer_key,
            first,
        });
        let mut stream = accepted.stream;
        let hello = json!({"jsonrpc": "2.0", "id": hello_id, "result": {
            "server": "handlr", "server_version": "0.1.0", "protocol_version": 1,
            "session_id": "companion:phone", "vocabulary": ["claude"]
        }});
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

fn unix_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

/// An address where nothing listens.
fn closed_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

/// A companion with no daemon socket, keeping its keys in `dir/keys`.
fn companion(dir: &tempfile::TempDir) -> Companion {
    let mut config = CompanionConfig::new("test");
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

#[test]
fn pairing_stores_the_key_and_the_server_and_connects() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let payload = server.pairing("phone", vec![server.addr.clone()], unix_now() + 300);
    c.send(Input::Pair(payload.encode().unwrap()));

    assert_eq!(
        next_event(&c),
        Event::Paired {
            name: "phone".into()
        }
    );
    expect_connected(&c);
    let seen = server.next_seen();
    assert!(matches!(seen.mode, AcceptedMode::Pairing(_)), "{seen:?}");
    let hello = assert_exact_hello(&seen.first);
    assert!(hello.get("session_id").is_none(), "{hello}");

    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let private = keys.secret().unwrap().expect("the client's key is stored");
    assert_eq!(link::public_key(&private).unwrap(), seen.peer_key);
    assert_eq!(
        keys.server().unwrap(),
        Some(ServerRecord {
            name: "phone".into(),
            public_key: server.key.public,
            addresses: vec![server.addr.clone()],
            session_id: Some("companion:phone".into()),
        })
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

#[test]
fn reconnecting_uses_ik_and_the_stored_session_id_and_fails_over() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let client = link::generate_keypair().unwrap();
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    keys.set_secret(client.private).unwrap();
    let addresses = vec![closed_addr(), server.addr.clone()];
    keys.set_server(ServerRecord {
        name: "phone".into(),
        public_key: server.key.public,
        addresses: addresses.clone(),
        session_id: Some("companion:old".into()),
    })
    .unwrap();
    server.authorized.lock().unwrap().insert(client.public);
    server.close_after_hello.store(1, Ordering::SeqCst);

    let c = companion(&dir);
    let seen = server.next_seen();
    assert_eq!(seen.mode, AcceptedMode::Reconnect);
    assert_eq!(seen.peer_key, client.public);
    assert_eq!(
        assert_exact_hello(&seen.first)["session_id"],
        "companion:old"
    );
    expect_connected(&c);

    // The server closes; the client comes back with the session_id that
    // hello gave, which is now stored.
    let seen = server.next_seen();
    assert_eq!(seen.mode, AcceptedMode::Reconnect);
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
    let payload = server.pairing("phone", addresses.clone(), unix_now() + 300);
    c.send(Input::Pair(payload.encode().unwrap()));
    assert_eq!(
        next_event(&c),
        Event::Paired {
            name: "phone".into()
        }
    );
    expect_connected(&c);
    let record = FileKeyStore::new(keys_dir(&dir)).server().unwrap().unwrap();
    assert_eq!(record.addresses, addresses);
}

#[test]
fn a_wrong_server_key_in_the_string_fails_the_pairing() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let mut payload = server.pairing("phone", vec![server.addr.clone()], unix_now() + 300);
    payload.server_key = link::generate_keypair().unwrap().public;
    c.send(Input::Pair(payload.encode().unwrap()));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("differs"), "{why}"),
        e => panic!("{e:?}"),
    }
    assert_eq!(FileKeyStore::new(keys_dir(&dir)).server().unwrap(), None);
    assert!(server.authorized.lock().unwrap().is_empty());
}

#[test]
fn an_expired_string_is_refused_before_connecting() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let payload = server.pairing("phone", vec![server.addr.clone()], unix_now() - 10);
    c.send(Input::Pair(payload.encode().unwrap()));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("expired"), "{why}"),
        e => panic!("{e:?}"),
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(server.accepts.load(Ordering::SeqCst), 0);
    assert_eq!(FileKeyStore::new(keys_dir(&dir)).secret().unwrap(), None);
}

#[test]
fn an_invalid_string_is_refused_before_connecting() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let mut text = server
        .pairing("phone", vec![server.addr.clone()], unix_now() + 300)
        .encode()
        .unwrap();
    // Change one character, keeping it base58: the checksum fails.
    let last = text.pop().unwrap();
    text.push(if last == '2' { '3' } else { '2' });
    c.send(Input::Pair(text));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("invalid pairing string"), "{why}"),
        e => panic!("{e:?}"),
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(server.accepts.load(Ordering::SeqCst), 0);
}
