//! The `net` thread over flankr link, against an in-test server built from
//! `libflankr_proto::link::accept` with a minimal JSON-RPC responder that
//! authorizes the keys in a set: pairing by device code (plan 20261022_02,
//! amendment of 23:07 EDT) from a string or the QR code's data URL (plan
//! 20261022_06, step 5), reconnecting with `IK` and the stored
//! `session_id`, refusals, the first message, and failing over between
//! addresses; several remotes (plan 20261028_01, step 4): pairing beside
//! the open connection, cancelling, re-pairing, the limit, and removing
//! the active remote or another. The server's set of authorized keys
//! stands in for the daemon's `auth.toml`; `link.unpair` removes the
//! caller's key from it.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use base64::Engine;
use flankr_companion::{
    Companion, CompanionConfig, Event, FileKeyStore, Input, KeyStore, LEGACY_KEY_NAME, MAX_REMOTES,
    PAIRING_WINDOW, REVOKE_TIMEOUT, RemoteId, RemoteInfo, RemoteState, Remotes, ServerRecord,
    Speaker,
};
use libflankr_proto::link::{self, DeviceCode, PairingPayload, StaticKeypair};
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

    /// The pairing string `flankr pair` would show for `addresses`.
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
        let peer_key = accepted.peer_key;
        let first: Value = serde_json::from_slice(&accepted.first_message.unwrap_or_default())
            .unwrap_or(Value::Null);
        let hello_id = first["id"].clone();
        let _ = self.seen.send(Seen {
            peer_key: accepted.peer_key,
            first,
        });
        let mut stream = accepted.stream;
        let mut hello = json!({"jsonrpc": "2.0", "id": hello_id, "result": {
            "server": "flankr", "server_version": "0.1.0", "protocol_version": 1,
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
                // As the daemon's `Link::unpair`: the key leaves auth.toml.
                Some("link.unpair") => {
                    self.authorized.lock().unwrap().remove(&peer_key);
                    json!({})
                }
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

/// A companion with no daemon socket, keeping its keys in `dir/keys` and
/// its state in `dir/data`.
fn companion(dir: &tempfile::TempDir) -> Companion {
    companion_with(dir, PAIRING_WINDOW)
}

/// As [`companion`], with a pairing window of `window`.
fn companion_with(dir: &tempfile::TempDir, window: Duration) -> Companion {
    let mut config = CompanionConfig::new("test");
    config.device_name = DEVICE.to_string();
    config.socket = dir.path().join("no-socket");
    config.backoff_initial = Duration::from_millis(50);
    config.backoff_max = Duration::from_millis(400);
    config.data_dir = Some(data_dir(dir));
    config.pairing_window = window;
    Companion::spawn(config, Box::new(FileKeyStore::new(keys_dir(dir))))
}

fn keys_dir(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("keys")
}

fn data_dir(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("data")
}

fn stored(dir: &tempfile::TempDir) -> Remotes {
    FileKeyStore::new(keys_dir(dir)).remotes().unwrap()
}

/// The next event that is not a `Disconnected` from the attempts on the
/// missing socket, the remotes list or the cache's size.
fn next_event(c: &Companion) -> Event {
    loop {
        match c.wait_timeout(TIMEOUT).expect("an event within 5 s") {
            Event::Disconnected { .. } | Event::Remotes(_) | Event::Cache { .. } => continue,
            e => return e,
        }
    }
}

/// Waits for an [`Event::Remotes`] for which `f` holds, skipping others.
fn expect_remotes(c: &Companion, what: &str, f: impl Fn(&[RemoteInfo]) -> bool) -> Vec<RemoteInfo> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.wait_timeout(left) {
            Some(Event::Remotes(rows)) if f(&rows) => return rows,
            Some(_) => {}
            None => panic!("no remotes list where {what} within 5 s"),
        }
    }
}

fn expect_connected(c: &Companion) {
    assert_eq!(
        next_event(c),
        Event::Connected {
            server: "flankr".into()
        }
    );
    assert_eq!(next_event(c), Event::Vocabulary(vec!["claude".into()]));
    // The hello has no `llm`: both, as from an older daemon.
    assert_eq!(
        next_event(c),
        Event::Llm(flankr_companion::wire::LlmInfo::available())
    );
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
    assert_eq!(first["params"]["protocol"], "flankr.companion");
    assert_eq!(first["params"]["client"], "test");
    first["params"].clone()
}

/// The device code the companion shows, and the server it names; the
/// confirmation code is the server key's.
fn expect_code(c: &Companion, server: &Server) -> (String, String) {
    match next_event(c) {
        Event::PairingCode {
            code,
            server: name,
            confirmation,
        } => {
            assert_eq!(confirmation, link::confirmation_code(&server.key.public));
            (code, name)
        }
        e => panic!("expected the pairing code, got {e:?}"),
    }
}

fn expect_paired(c: &Companion) {
    assert_eq!(
        next_event(c),
        Event::Paired {
            name: DEVICE.into()
        }
    );
}

/// A remote's record for `server` at `addresses`, paired at `paired_at`,
/// with a new phone key that the server authorizes when `authorized`.
fn remote(
    keys: &mut FileKeyStore,
    server: &Server,
    addresses: Vec<String>,
    paired_at: Option<u64>,
    authorized: bool,
) -> ServerRecord {
    let mut record = ServerRecord::new("phone", server.key.public, addresses);
    record.paired_at = paired_at;
    let client = keys.generate_static_key(&record.key_name).unwrap().public();
    if authorized {
        server.authorized.lock().unwrap().insert(client);
    }
    record
}

/// The ids of the stored remotes.
fn ids(dir: &tempfile::TempDir) -> Vec<RemoteId> {
    stored(dir).list.iter().map(|r| r.id).collect()
}

fn key_exists(dir: &tempfile::TempDir, key_name: &str) -> bool {
    FileKeyStore::new(keys_dir(dir))
        .static_key(key_name)
        .unwrap()
        .is_some()
}

#[test]
fn pairing_shows_the_code_and_connects_once_it_is_added() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let payload = server.pairing(vec![server.addr.clone()]);
    c.send(Input::Pair(payload.encode().unwrap()));

    let (code, name) = expect_code(&c, &server);
    // A bare string: named after its first address's host.
    assert_eq!(name, "127.0.0.1");
    let id = RemoteId::of(&server.key.public);
    let key = FileKeyStore::new(keys_dir(&dir))
        .static_key(&id.key_name())
        .unwrap()
        .expect("the remote's own key is stored");
    // Nothing is listed before the server accepts.
    assert_eq!(stored(&dir), Remotes::default());

    // Until the code is pasted, the server drops it, and it tries again.
    std::thread::sleep(Duration::from_millis(2500));
    assert!(server.accepts.load(Ordering::SeqCst) >= 2);
    assert!(server.seen.try_recv().is_err());

    let added = server.add(&code);
    assert_eq!(added.key, key.public());
    assert_eq!(added.name, DEVICE);
    expect_paired(&c);
    // The probe's hello, then the main connection's.
    for _ in 0..2 {
        let seen = server.next_seen();
        assert_eq!(seen.peer_key, key.public());
        let hello = assert_exact_hello(&seen.first);
        assert!(hello.get("session_id").is_none(), "{hello}");
    }
    expect_connected(&c);
    let rows = expect_remotes(&c, "the remote is online", |rows| {
        rows.len() == 1 && rows[0].online
    });
    assert_eq!(rows[0].id, id);
    assert!(rows[0].active);
    assert_eq!(rows[0].host, server.addr);
    assert_eq!(
        rows[0].fingerprint,
        link::fingerprint_text(&server.key.public)
    );
    let remotes = stored(&dir);
    assert_eq!(remotes.active, Some(id));
    let record = remotes.active_record().unwrap();
    assert_eq!(record.name, "127.0.0.1");
    assert_eq!(record.addresses, std::slice::from_ref(&server.addr));
    assert_eq!(record.key_name, id.key_name());
    assert!(record.paired_at.is_some());
    assert_eq!(record.session_id.as_deref(), Some("companion:phone"));
    // Its state database is open.
    assert!(RemoteState::file(&data_dir(&dir), id).exists());

    // And the connection carries a turn.
    c.send(Input::Utterance("what is claude doing".into()));
    assert_eq!(
        next_event(&c),
        Event::Conversation {
            speaker: Speaker::User,
            text: "what is claude doing".into()
        }
    );
    assert_eq!(
        next_event(&c),
        Event::TurnEnded {
            outcome: "done".into(),
            stopped: vec![]
        }
    );
}

/// Plan 20261022_06, step 5: the data URL `flankr pair` prints pairs.
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

    let (code, name) = expect_code(&c, &server);
    assert_eq!(name, "127.0.0.1");
    let added = server.add(&code);
    expect_paired(&c);
    assert_eq!(server.next_seen().peer_key, added.key);
    expect_connected(&c);
}

#[test]
fn reconnecting_uses_ik_and_the_stored_session_id_and_fails_over() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let addresses = vec![closed_addr(), server.addr.clone()];
    let mut record = remote(&mut keys, &server, addresses.clone(), None, true);
    record.session_id = Some("companion:old".into());
    let client = keys.static_key(&record.key_name).unwrap().unwrap().public();
    keys.set_remotes(&Remotes {
        active: Some(record.id),
        list: vec![record.clone()],
    })
    .unwrap();
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
    record.session_id = Some("companion:phone".into());
    assert_eq!(ids(&dir), [record.id]);
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
    let (code, name) = expect_code(&c, &server);
    // The URL names the first address that is not loopback, or the first.
    assert_eq!(name, "127.0.0.1");
    server.add(&code);
    expect_paired(&c);
    let remotes = stored(&dir);
    assert_eq!(remotes.active_record().unwrap().addresses, addresses);
}

#[test]
fn each_server_gets_its_own_key_and_a_new_pairing_replaces_the_probe() {
    let server = Server::start();
    let other = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    c.send(Input::Pair(
        server.pairing(vec![server.addr.clone()]).encode().unwrap(),
    ));
    let (code, _) = expect_code(&c, &server);
    assert!(DeviceCode::decode(&code, &other.key.public).is_err());
    let first_name = RemoteId::of(&server.key.public).key_name();
    assert!(key_exists(&dir, &first_name));

    // Pairing another server ends the first probe and deletes its key.
    c.send(Input::Pair(
        other.pairing(vec![other.addr.clone()]).encode().unwrap(),
    ));
    let (again, _) = expect_code(&c, &other);
    let first = DeviceCode::decode(&code, &server.key.public).unwrap();
    let second = DeviceCode::decode(&again, &other.key.public).unwrap();
    assert_ne!(first.key, second.key, "a key per remote");
    assert!(!key_exists(&dir, &first_name));
    other.add(&again);
    expect_paired(&c);
    let remotes = stored(&dir);
    assert_eq!(remotes.list.len(), 1);
    assert_eq!(remotes.list[0].public_key, other.key.public);
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
    assert_eq!(stored(&dir), Remotes::default());
    let id = RemoteId::of(&server.key.public);
    assert!(!key_exists(&dir, &id.key_name()));
}

// Several remotes (plan 20261028_01, step 4).

/// A companion connected to `first`, its only remote; its record.
fn connected_to(dir: &tempfile::TempDir, first: &Server) -> (Companion, ServerRecord) {
    let mut keys = FileKeyStore::new(keys_dir(dir));
    let record = remote(&mut keys, first, vec![first.addr.clone()], Some(10), true);
    keys.set_remotes(&Remotes {
        active: Some(record.id),
        list: vec![record.clone()],
    })
    .unwrap();
    let c = companion(dir);
    first.next_seen();
    expect_connected(&c);
    (c, record)
}

/// A turn goes over the open connection.
fn assert_turn(c: &Companion) {
    c.send(Input::Utterance("status".into()));
    assert!(matches!(next_event(c), Event::Conversation { .. }));
    assert!(matches!(next_event(c), Event::TurnEnded { .. }));
}

#[test]
fn a_second_pairing_while_connected_adds_a_remote_and_keeps_the_connection() {
    let first = Server::start();
    let second = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let (c, record) = connected_to(&dir, &first);

    c.send(Input::Pair(
        second.pairing(vec![second.addr.clone()]).encode().unwrap(),
    ));
    let (code, _) = expect_code(&c, &second);
    std::thread::sleep(Duration::from_millis(500));
    second.add(&code);
    expect_paired(&c);
    let rows = expect_remotes(&c, "two remotes", |rows| rows.len() == 2);
    assert!(rows[0].active && rows[0].online, "{rows:?}");
    assert!(!rows[1].active && !rows[1].online, "{rows:?}");
    assert_eq!(rows[1].id, RemoteId::of(&second.key.public));
    let remotes = stored(&dir);
    assert_eq!(remotes.active, Some(record.id));
    assert_eq!(remotes.list.len(), 2);

    // The first connection never dropped: one accept, and it still works.
    assert_eq!(first.accepts.load(Ordering::SeqCst), 1);
    assert_turn(&c);
    assert_eq!(first.accepts.load(Ordering::SeqCst), 1);
}

#[test]
fn an_unaccepted_probe_adds_nothing_and_deletes_its_key() {
    let first = Server::start();
    let second = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let record = remote(&mut keys, &first, vec![first.addr.clone()], Some(10), true);
    keys.set_remotes(&Remotes {
        active: Some(record.id),
        list: vec![record.clone()],
    })
    .unwrap();
    let c = companion_with(&dir, Duration::from_secs(1));
    first.next_seen();
    expect_connected(&c);
    let name = RemoteId::of(&second.key.public).key_name();

    // The window ends.
    c.send(Input::Pair(
        second.pairing(vec![second.addr.clone()]).encode().unwrap(),
    ));
    expect_code(&c, &second);
    assert!(key_exists(&dir, &name));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("in time"), "{why}"),
        e => panic!("{e:?}"),
    }
    assert!(!key_exists(&dir, &name));
    assert_eq!(ids(&dir), [record.id]);

    // Cancelled.
    c.send(Input::Pair(
        second.pairing(vec![second.addr.clone()]).encode().unwrap(),
    ));
    let (code, _) = expect_code(&c, &second);
    c.send(Input::CancelPairing);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!key_exists(&dir, &name));
    // Adding the code now pairs nothing.
    second.add(&code);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(second.seen.try_recv().is_err());
    assert_eq!(ids(&dir), [record.id]);
    assert_turn(&c);
}

#[test]
fn pairing_the_same_server_again_adds_no_row_and_updates_the_addresses() {
    let first = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let (c, record) = connected_to(&dir, &first);
    let addresses = vec![first.addr.clone(), "m4.tail.ts.net:7347".to_string()];
    c.send(Input::Pair(first.pairing(addresses.clone()).url().unwrap()));
    assert_eq!(
        next_event(&c),
        Event::RemoteUpdated {
            id: record.id,
            // The URL names the first address that is not loopback.
            name: "m4".into()
        }
    );
    let remotes = stored(&dir);
    assert_eq!(remotes.list.len(), 1);
    assert_eq!(remotes.list[0].addresses, addresses);
    // Same key, same pairing time.
    assert_eq!(remotes.list[0].key_name, record.key_name);
    assert_eq!(remotes.list[0].paired_at, record.paired_at);
    assert_turn(&c);
}

#[test]
fn a_ninth_pairing_is_refused() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let list: Vec<ServerRecord> = (1..=MAX_REMOTES as u8)
        .map(|n| ServerRecord::new(format!("m{n}"), [n; 32], vec![closed_addr()]))
        .collect();
    FileKeyStore::new(keys_dir(&dir))
        .set_remotes(&Remotes { active: None, list })
        .unwrap();
    let c = companion(&dir);
    c.send(Input::Pair(
        server.pairing(vec![server.addr.clone()]).encode().unwrap(),
    ));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("8 remotes"), "{why}"),
        e => panic!("{e:?}"),
    }
    assert_eq!(stored(&dir).list.len(), MAX_REMOTES);
    assert!(!key_exists(
        &dir,
        &RemoteId::of(&server.key.public).key_name()
    ));
}

#[test]
fn removing_another_reachable_remote_unpairs_it_and_keeps_the_connection() {
    let first = Server::start();
    let second = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let a = remote(&mut keys, &first, vec![first.addr.clone()], Some(10), true);
    let b = remote(
        &mut keys,
        &second,
        vec![second.addr.clone()],
        Some(20),
        true,
    );
    let b_key = keys.static_key(&b.key_name).unwrap().unwrap().public();
    keys.set_remotes(&Remotes {
        active: Some(a.id),
        list: vec![a.clone(), b.clone()],
    })
    .unwrap();
    // A state directory for b, as a switch would have left.
    drop(RemoteState::open(&data_dir(&dir), &b).unwrap());
    let c = companion(&dir);
    first.next_seen();
    expect_connected(&c);

    c.send(Input::RemoveRemote(b.id));
    assert_eq!(
        next_event(&c),
        Event::RemoteRemoved {
            id: b.id,
            name: "phone".into(),
            daemon: true
        }
    );
    // It asked with b's key, and b's auth no longer lists it.
    assert_eq!(second.next_seen().peer_key, b_key);
    assert!(!second.authorized.lock().unwrap().contains(&b_key));
    assert!(!key_exists(&dir, &b.key_name));
    assert!(!RemoteState::dir(&data_dir(&dir), b.id).exists());
    assert_eq!(ids(&dir), [a.id]);
    assert_eq!(stored(&dir).active, Some(a.id));
    // The active connection stays up.
    assert_eq!(first.accepts.load(Ordering::SeqCst), 1);
    assert_turn(&c);
    assert!(RemoteState::file(&data_dir(&dir), a.id).exists());
}

#[test]
fn removing_an_unreachable_remote_deletes_it_here_only() {
    let first = Server::start();
    let second = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let a = remote(&mut keys, &first, vec![first.addr.clone()], Some(10), true);
    let b = remote(&mut keys, &second, vec![closed_addr()], Some(20), true);
    keys.set_remotes(&Remotes {
        active: Some(a.id),
        list: vec![a.clone(), b.clone()],
    })
    .unwrap();
    let c = companion(&dir);
    first.next_seen();
    expect_connected(&c);

    let asked = Instant::now();
    c.send(Input::RemoveRemote(b.id));
    assert_eq!(
        next_event(&c),
        Event::RemoteRemoved {
            id: b.id,
            name: "phone".into(),
            daemon: false
        }
    );
    assert!(asked.elapsed() < REVOKE_TIMEOUT);
    assert!(!key_exists(&dir, &b.key_name));
    assert_eq!(ids(&dir), [a.id]);
    assert_turn(&c);
}

#[test]
fn removing_the_active_remote_moves_to_the_most_recently_paired() {
    let servers = [Server::start(), Server::start(), Server::start()];
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    // a, the active one, then b (paired last) and c.
    let a = remote(
        &mut keys,
        &servers[0],
        vec![servers[0].addr.clone()],
        None,
        true,
    );
    let b = remote(
        &mut keys,
        &servers[1],
        vec![servers[1].addr.clone()],
        Some(30),
        true,
    );
    let c_ = remote(
        &mut keys,
        &servers[2],
        vec![servers[2].addr.clone()],
        Some(20),
        true,
    );
    keys.set_remotes(&Remotes {
        active: Some(a.id),
        list: vec![a.clone(), b.clone(), c_.clone()],
    })
    .unwrap();
    let c = companion(&dir);
    servers[0].next_seen();
    expect_connected(&c);

    c.send(Input::RemoveRemote(a.id));
    assert_eq!(
        next_event(&c),
        Event::RemoteRemoved {
            id: a.id,
            name: "phone".into(),
            daemon: true
        }
    );
    // b is in use now, and connected to.
    servers[1].next_seen();
    expect_connected(&c);
    let rows = expect_remotes(&c, "b online", |rows| {
        rows.len() == 2 && rows[0].id == b.id && rows[0].online
    });
    assert!(!rows[1].active);
    assert_eq!(stored(&dir).active, Some(b.id));
    assert!(!key_exists(&dir, &a.key_name));
    assert!(!RemoteState::dir(&data_dir(&dir), a.id).exists());
    assert!(RemoteState::file(&data_dir(&dir), b.id).exists());

    // Both of the rest at once, as *Delete data* does: the last gives
    // Unpaired, and the list is empty.
    c.send(Input::RemoveRemote(b.id));
    c.send(Input::RemoveRemote(c_.id));
    let mut removed = Vec::new();
    loop {
        match next_event(&c) {
            Event::RemoteRemoved { id, daemon, .. } => {
                assert!(daemon);
                removed.push(id);
            }
            Event::Unpaired { daemon } => {
                assert!(daemon);
                break;
            }
            Event::Connected { .. } | Event::Vocabulary(_) | Event::Llm(_) => {}
            e => panic!("{e:?}"),
        }
    }
    removed.sort();
    let mut expected = vec![b.id, c_.id];
    expected.sort();
    assert_eq!(removed, expected);
    assert_eq!(stored(&dir), Remotes::default());
    assert!(
        std::fs::read_dir(RemoteState::remotes_dir(&data_dir(&dir)))
            .unwrap()
            .next()
            .is_none()
    );
}

// Unpairing (plan 20261024_02, decision 6 and step 4): the active remote.

#[test]
fn unpairing_online_forgets_after_the_daemon_answers() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let (c, record) = connected_to(&dir, &server);

    c.send(Input::Unpair);
    // The in-test server answers link.unpair with `{}`.
    assert_eq!(
        next_event(&c),
        Event::RemoteRemoved {
            id: record.id,
            name: "phone".into(),
            daemon: true
        }
    );
    assert_eq!(next_event(&c), Event::Unpaired { daemon: true });
    assert_eq!(stored(&dir), Remotes::default());
    assert!(!key_exists(&dir, &record.key_name));
    // It does not connect to the server again.
    assert!(
        server
            .seen
            .recv_timeout(Duration::from_millis(500))
            .is_err()
    );
}

#[test]
fn unpairing_offline_forgets_at_once() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    // Paired, but the server is not reachable.
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let record = remote(&mut keys, &server, vec![closed_addr()], None, false);
    keys.set_remotes(&Remotes {
        active: Some(record.id),
        list: vec![record.clone()],
    })
    .unwrap();
    let c = companion(&dir);
    c.send(Input::RemoveRemote(record.id));
    assert_eq!(
        next_event(&c),
        Event::RemoteRemoved {
            id: record.id,
            name: "phone".into(),
            daemon: false
        }
    );
    assert_eq!(next_event(&c), Event::Unpaired { daemon: false });
    assert_eq!(stored(&dir), Remotes::default());
    assert!(!key_exists(&dir, &record.key_name));
}

#[test]
fn todays_pairing_is_migrated_and_connected_to() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let keys = keys_dir(&dir);
    std::fs::create_dir_all(&keys).unwrap();
    // The files the old store wrote: `key` and `server.toml`.
    let phone = link::generate_keypair().unwrap();
    use std::os::unix::fs::PermissionsExt;
    let key_path = keys.join("key");
    std::fs::write(
        &key_path,
        format!(
            "{}\n",
            base64::engine::general_purpose::STANDARD.encode(phone.private)
        ),
    )
    .unwrap();
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(
        keys.join("server.toml"),
        format!(
            "name = \"m4\"\npublic_key = \"{}\"\naddresses = [\"{}\"]\n",
            base64::engine::general_purpose::STANDARD.encode(server.key.public),
            server.addr
        ),
    )
    .unwrap();
    server.authorized.lock().unwrap().insert(phone.public);
    let c = companion(&dir);
    assert_eq!(server.next_seen().peer_key, phone.public);
    expect_connected(&c);
    let remotes = stored(&dir);
    assert_eq!(remotes.list.len(), 1);
    assert_eq!(remotes.active, Some(RemoteId::of(&server.key.public)));
    assert_eq!(remotes.list[0].key_name, LEGACY_KEY_NAME);
    assert!(!keys.join("server.toml").exists());
}
