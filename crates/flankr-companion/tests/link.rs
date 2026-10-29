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
//!
//! The server also answers pairing requests (`link::accept_incoming`, with
//! `PairIncoming`) while a test puts it in pair mode, standing in for a
//! `flankr pair` run (plan 20261028_01, the 14:37, 14:42 and 14:44 EDT
//! amendments): the test is handed each held request and answers it as
//! the owner would on the Mac.

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
use libflankr_proto::link::{
    self, DeviceCode, Incoming, OFFER_LEN, PairAck, PairRefusal, PairRequest, PairResult,
    PairingPayload, StaticKeypair,
};
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
    /// The `flankr pair` run, if any: requests are dropped unanswered
    /// without one.
    pair_mode: Arc<Mutex<Option<PairMode>>>,
    /// The requests held, one per accepted request.
    held: mpsc::Receiver<Held>,
    /// Every request on a link, with the phone key that sent it.
    requests: Requests,
    /// A sender per open link, for notifications.
    links: Links,
}

/// Every request on a link, with the phone key that sent it.
type Requests = Arc<Mutex<Vec<([u8; 32], Value)>>>;
/// A sender per open link.
type Links = Arc<Mutex<Vec<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>>>;

/// A `flankr pair` run on the test server.
#[derive(Clone, Copy)]
struct PairMode {
    offer: [u8; OFFER_LEN],
    expires_in_secs: u16,
    /// Refuse every request at once with this, as for an expired offer.
    refuse: Option<PairRefusal>,
}

/// A pairing request the test server holds: the test answers it as the
/// owner would.
struct Held {
    request: PairRequest,
    answer: tokio::sync::mpsc::UnboundedSender<PairResult>,
    /// Signalled when the phone closes the connection before an answer.
    closed: mpsc::Receiver<()>,
}

impl Held {
    /// The owner's answer; `Added` adds the key to the server's set.
    fn answer(&self, result: PairResult) {
        self.answer.send(result).unwrap();
    }
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
        let pair_mode = Arc::new(Mutex::new(None));
        let (held_tx, held) = mpsc::channel();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let agents = Arc::new(Mutex::new(json!([])));
        let links = Arc::new(Mutex::new(Vec::new()));
        let ctx = Ctx {
            requests: requests.clone(),
            agents,
            links: links.clone(),
            pair_mode: pair_mode.clone(),
            held: held_tx,
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
            pair_mode,
            held,
            requests,
            links,
        }
    }

    /// Sends `method` with `params` on every open link.
    fn notify(&self, method: &str, params: Value) {
        let note = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let bytes = serde_json::to_vec(&note).unwrap();
        for link in self.links.lock().unwrap().iter() {
            let _ = link.send(bytes.clone());
        }
    }

    /// The requests of `method` seen so far.
    fn seen(&self, method: &str) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, r)| r["method"] == method)
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// The pairing string an older `flankr pair` would show for
    /// `addresses`: version 2, without an offer.
    fn pairing(&self, addresses: Vec<String>) -> PairingPayload {
        PairingPayload {
            server_key: self.key.public,
            addresses,
            offer: None,
        }
    }

    /// Puts the server in pair mode with a new offer, as `flankr pair`
    /// does, and returns the link it shows for `addresses` (version 3).
    fn pair_run(&self, addresses: Vec<String>) -> PairingPayload {
        let offer = link::generate_offer().unwrap();
        *self.pair_mode.lock().unwrap() = Some(PairMode {
            offer,
            expires_in_secs: 60,
            refuse: None,
        });
        PairingPayload {
            server_key: self.key.public,
            addresses,
            offer: Some(offer),
        }
    }

    /// The next request the server holds.
    fn next_held(&self) -> Held {
        self.held
            .recv_timeout(TIMEOUT)
            .expect("a pairing request within 5 s")
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
    pair_mode: Arc<Mutex<Option<PairMode>>>,
    held: mpsc::Sender<Held>,
    requests: Requests,
    /// The `agents` that `agents.list` answers with.
    agents: Arc<Mutex<Value>>,
    links: Links,
}

impl Ctx {
    async fn serve(self, stream: tokio::net::TcpStream) {
        let authorized = self.authorized.clone();
        let incoming = link::accept_incoming(stream, &self.key, |key| {
            authorized.lock().unwrap().contains(key)
        })
        .await;
        let accepted = match incoming {
            Ok(Incoming::Link(accepted)) => accepted,
            Ok(Incoming::Pair(pair)) => return self.pair(*pair).await,
            Err(_) => return,
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
        let (mut read, mut write) = stream.into_split();
        let (out, mut outgoing) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        self.links.lock().unwrap().push(out.clone());
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if write.send_message(&message).await.is_err() {
                    return;
                }
            }
        });
        while let Ok(message) = read.recv_message().await {
            let request: Value = serde_json::from_slice(&message).unwrap();
            self.requests
                .lock()
                .unwrap()
                .push((peer_key, request.clone()));
            if request.get("id").is_none() {
                continue;
            }
            let result = match request["method"].as_str() {
                Some("agents.list") => json!({"agents": self.agents.lock().unwrap().clone()}),
                Some("turn.start") => json!({"outcome": "done", "ran": []}),
                // As the daemon's `Link::unpair`: the key leaves auth.toml.
                Some("link.unpair") => {
                    self.authorized.lock().unwrap().remove(&peer_key);
                    json!({})
                }
                _ => json!({}),
            };
            let response = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
            if out.send(serde_json::to_vec(&response).unwrap()).is_err() {
                return;
            }
        }
        self.links.lock().unwrap().retain(|l| !l.same_channel(&out));
    }
}

impl Ctx {
    /// A pairing request, as the daemon's pair mode answers it: dropped
    /// with no run, refused for a wrong offer, else held until the test
    /// answers or the phone closes the connection.
    async fn pair(self, incoming: link::PairIncoming<tokio::net::TcpStream>) {
        let Some(mode) = *self.pair_mode.lock().unwrap() else {
            return;
        };
        let refusal = match &incoming.request {
            Err(_) => Some(PairRefusal::Malformed),
            Ok(_) if mode.refuse.is_some() => mode.refuse,
            Ok(request) if request.offer != mode.offer => Some(PairRefusal::WrongOffer),
            Ok(_) => None,
        };
        if let Some(reason) = refusal {
            let _ = incoming.respond(PairAck::Refused(reason)).await;
            return;
        }
        let request = incoming.request.clone().unwrap();
        let Ok(mut stream) = incoming
            .respond(PairAck::Accepted {
                expires_in_secs: mode.expires_in_secs,
            })
            .await
        else {
            return;
        };
        let (answer, mut answers) = tokio::sync::mpsc::unbounded_channel();
        let (closed_tx, closed) = mpsc::channel();
        let device_key = request.device_key;
        let _ = self.held.send(Held {
            request,
            answer,
            closed,
        });
        tokio::select! {
            result = answers.recv() => {
                let Some(result) = result else { return };
                if result == PairResult::Added {
                    self.authorized.lock().unwrap().insert(device_key);
                    self.fresh.lock().unwrap().insert(device_key);
                }
                let _ = link::send_pair_result(&mut stream, result).await;
                // Until the phone closes it.
                let _ = stream.recv_message().await;
            }
            _ = stream.recv_message() => {
                let _ = closed_tx.send(());
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
/// missing socket, the remotes list, the cache's size, a remote's in the
/// background, or what a remote coming in front gives of itself (its
/// agents, pending panes, focus, cards, dictation card and plugins).
fn next_event(c: &Companion) -> Event {
    loop {
        match c.wait_timeout(TIMEOUT).expect("an event within 5 s") {
            Event::Disconnected { .. }
            | Event::Remotes(_)
            | Event::Cache { .. }
            | Event::Background { .. }
            | Event::Agents(_)
            | Event::PendingPanes(_)
            | Event::Focus(_)
            | Event::SessionCards(_)
            | Event::Dictation(_)
            | Event::Plugins(_) => continue,
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

/// The device code the companion shows in the fallback for a version 2
/// link, and the server it names: no pairing words (the link has no
/// offer), the daemon's fingerprint words, no time left.
fn expect_code(c: &Companion, server: &Server) -> (String, String) {
    expect_code_with(c, server, false)
}

/// As [`expect_code`], with `replace` as given: whether the code replaces
/// a lost key the daemon still lists.
fn expect_code_with(c: &Companion, server: &Server, replace: bool) -> (String, String) {
    let (code, name, replacing) = expect_fallback(c, server, None);
    assert_eq!(replacing, replace, "replace");
    (code, name)
}

/// The fallback's event for `server`, whose link had `offer`: the device
/// code, and the pairing words for its key and the offer when there is
/// one. Returns the code, the server's name and `replace`.
fn expect_fallback(
    c: &Companion,
    server: &Server,
    offer: Option<[u8; OFFER_LEN]>,
) -> (String, String, bool) {
    match next_event(c) {
        Event::PairingWords {
            server: name,
            address,
            words,
            fingerprint,
            fingerprint_hex,
            expires_in_secs,
            fallback: Some(fallback),
        } => {
            assert_eq!(
                fingerprint_hex,
                flankr_companion::keys::fingerprint_hex(&server.key.public)
            );
            let device = DeviceCode::decode(&fallback.code, &server.key.public)
                .expect("a code for this server");
            assert_eq!(
                words,
                offer.map(|offer| link::pairing_words_text(
                    &server.key.public,
                    &offer,
                    &device.key
                ))
            );
            assert_eq!(
                fingerprint,
                link::fingerprint_words_text(&server.key.public)
            );
            assert_eq!(expires_in_secs, None);
            assert!(!address.is_empty());
            (fallback.code, name, fallback.replace)
        }
        e => panic!("expected the device-code fallback, got {e:?}"),
    }
}

/// The six words the companion shows once `server` holds its request, as
/// the daemon computes them from the request it holds. Returns the held
/// request.
fn expect_words(c: &Companion, server: &Server) -> Held {
    let held = server.next_held();
    match next_event(c) {
        Event::PairingWords {
            server: _,
            address,
            words,
            fingerprint,
            fingerprint_hex,
            expires_in_secs,
            fallback: None,
        } => {
            assert_eq!(fingerprint_hex.len(), 64);
            assert_eq!(
                fingerprint_hex,
                flankr_companion::keys::fingerprint_hex(&server.key.public)
            );
            let offer = server.pair_mode.lock().unwrap().unwrap().offer;
            assert_eq!(held.request.offer, offer);
            assert_eq!(held.request.name, DEVICE);
            // What `flankr pair` prints for the request it holds.
            let daemon =
                link::pairing_words_text(&server.key.public, &offer, &held.request.device_key);
            assert_eq!(words.as_deref(), Some(daemon.as_str()));
            assert_eq!(words.unwrap().split(' ').count(), 6);
            assert_eq!(
                fingerprint,
                link::fingerprint_words_text(&server.key.public)
            );
            assert_eq!(expires_in_secs, Some(60));
            assert_eq!(address, server.addr);
        }
        e => panic!("expected the pairing words, got {e:?}"),
    }
    held
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

// The review's changes (plan 20261028_01, the 14:37 EDT amendment, item 4).

#[test]
fn a_link_for_the_remote_being_paired_keeps_its_key_and_code() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let name = RemoteId::of(&server.key.public).key_name();
    c.send(Input::Pair(
        server.pairing(vec![server.addr.clone()]).encode().unwrap(),
    ));
    let (code, _) = expect_code(&c, &server);
    let key = FileKeyStore::new(keys_dir(&dir))
        .static_key(&name)
        .unwrap()
        .unwrap()
        .public();

    // The same link again, and its URL form: the same code, the same key.
    c.send(Input::Pair(
        server.pairing(vec![server.addr.clone()]).encode().unwrap(),
    ));
    assert_eq!(expect_code(&c, &server).0, code);
    c.send(Input::Pair(
        server.pairing(vec![server.addr.clone()]).url().unwrap(),
    ));
    assert_eq!(expect_code(&c, &server).0, code);
    let still = FileKeyStore::new(keys_dir(&dir))
        .static_key(&name)
        .unwrap()
        .unwrap()
        .public();
    assert_eq!(still, key, "not regenerated");

    // The code shown first still pairs.
    server.add(&code);
    expect_paired(&c);
    assert_eq!(ids(&dir), [RemoteId::of(&server.key.public)]);
}

#[test]
fn a_listed_remotes_new_addresses_are_saved_only_once_reached() {
    let first = Server::start();
    let other = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let (c, record) = connected_to(&dir, &first);

    // Nothing listens there: refused, the old address kept.
    c.send(Input::Pair(
        first.pairing(vec![closed_addr()]).url().unwrap(),
    ));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("unchanged"), "{why}"),
        e => panic!("{e:?}"),
    }
    assert_eq!(stored(&dir).list[0].addresses, record.addresses);
    assert_eq!(stored(&dir).list[0].name, record.name);

    // Another daemon answers there: its key is not this remote's, so the
    // handshake fails and nothing is saved.
    c.send(Input::Pair(
        first.pairing(vec![other.addr.clone()]).url().unwrap(),
    ));
    assert!(matches!(next_event(&c), Event::PairFailed(_)));
    assert_eq!(stored(&dir).list[0].addresses, record.addresses);
    assert!(
        other.seen.try_recv().is_err(),
        "no handshake with the other"
    );

    // Reached at the second address: saved, with the link's name.
    let addresses = vec![closed_addr(), first.addr.clone()];
    c.send(Input::Pair(
        first.pairing(addresses.clone()).encode().unwrap(),
    ));
    assert_eq!(
        next_event(&c),
        Event::RemoteUpdated {
            id: record.id,
            name: "127.0.0.1".into()
        }
    );
    let remotes = stored(&dir);
    assert_eq!(remotes.list[0].addresses, addresses);
    assert_eq!(remotes.list[0].name, "127.0.0.1");
    assert_eq!(remotes.list[0].key_name, record.key_name);
    assert_turn(&c);
}

#[test]
fn a_listed_remote_whose_key_was_lost_pairs_again_with_replace() {
    let first = Server::start();
    let second = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let (c, active) = connected_to(&dir, &first);
    // `second` is listed, but this phone's key for it is gone.
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let lost = remote(&mut keys, &second, vec![closed_addr()], Some(11), true);
    keys.delete_static_key(&lost.key_name).unwrap();
    drop(c);
    keys.set_remotes(&Remotes {
        active: Some(active.id),
        list: vec![active.clone(), lost.clone()],
    })
    .unwrap();
    let c = companion(&dir);
    first.next_seen();
    expect_connected(&c);

    let addresses = vec![second.addr.clone()];
    c.send(Input::Pair(
        second.pairing(addresses.clone()).encode().unwrap(),
    ));
    let (code, _) = expect_code_with(&c, &second, true);
    // Not saved yet: the old address stays until the probe is answered.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(stored(&dir).list[1].addresses, lost.addresses);
    second.add(&code);
    expect_paired(&c);
    let remotes = stored(&dir);
    assert_eq!(remotes.list.len(), 2);
    assert_eq!(remotes.list[1].addresses, addresses);
    assert_eq!(remotes.active, Some(active.id));
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
fn a_nickname_is_saved_and_shown_and_cleared_back_to_the_default() {
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
    // Both at 127.0.0.1: each default shows its port.
    let port = |r: &ServerRecord| r.host().rsplit_once(':').unwrap().1.to_string();
    let rows = expect_remotes(&c, "both listed", |rows| rows.len() == 2);
    assert_eq!(rows[0].display_name, format!("127.0.0.1:{}", port(&a)));
    assert_eq!(rows[1].display_name, format!("127.0.0.1:{}", port(&b)));

    c.send(Input::SetNickname {
        id: b.id,
        nickname: Some(" Office ".into()),
    });
    let rows = expect_remotes(&c, "b named", |rows| {
        rows.get(1).is_some_and(|r| r.nickname.is_some())
    });
    assert_eq!(rows[1].nickname.as_deref(), Some("Office"));
    assert_eq!(rows[1].display_name, "Office");
    // No conflict left: the base hostname alone.
    assert_eq!(rows[0].display_name, "127.0.0.1");
    let kept = stored(&dir);
    assert_eq!(kept.get(b.id).unwrap().nickname.as_deref(), Some("Office"));
    assert_eq!(kept.get(a.id).unwrap().nickname, None);

    c.send(Input::SetNickname {
        id: b.id,
        nickname: Some(String::new()),
    });
    let rows = expect_remotes(&c, "b cleared", |rows| {
        rows.get(1).is_some_and(|r| r.nickname.is_none())
    });
    assert_eq!(rows[1].display_name, format!("127.0.0.1:{}", port(&b)));
    assert_eq!(stored(&dir).get(b.id).unwrap().nickname, None);
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

// The pairing request and its six words (plan 20261028_01, the 14:37,
// 14:42 and 14:44 EDT amendments).

/// The public key of the phone's key `key_name`, if stored.
fn stored_key(dir: &tempfile::TempDir, key_name: &str) -> Option<[u8; 32]> {
    FileKeyStore::new(keys_dir(dir))
        .static_key(key_name)
        .unwrap()
        .map(|k| k.public())
}

#[test]
fn pairing_by_request_shows_the_daemons_words_and_stores_only_once_added() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let id = RemoteId::of(&server.key.public);
    let link = server.pair_run(vec![server.addr.clone()]);
    c.send(Input::Pair(link.url().unwrap()));

    let held = expect_words(&c, &server);
    // The request carries the key the phone keeps for this remote.
    assert_eq!(
        stored_key(&dir, &id.key_name()),
        Some(held.request.device_key)
    );
    // Nothing is stored before the owner's yes: no record, no state.
    assert_eq!(stored(&dir), Remotes::default());
    assert!(!RemoteState::dir(&data_dir(&dir), id).exists());

    // The same link again: the same words, the same request, no second.
    c.send(Input::Pair(link.encode().unwrap()));
    assert!(matches!(
        next_event(&c),
        Event::PairingWords { fallback: None, .. }
    ));
    assert!(
        server
            .held
            .recv_timeout(Duration::from_millis(300))
            .is_err()
    );
    assert_eq!(
        stored_key(&dir, &id.key_name()),
        Some(held.request.device_key)
    );

    // Yes on the Mac: added, stored, and connected with IK.
    held.answer(PairResult::Added);
    expect_paired(&c);
    let seen = server.next_seen();
    assert_eq!(seen.peer_key, held.request.device_key);
    assert_exact_hello(&seen.first);
    expect_connected(&c);
    let remotes = stored(&dir);
    assert_eq!(remotes.active, Some(id));
    let record = remotes.active_record().unwrap();
    assert_eq!(record.addresses, std::slice::from_ref(&server.addr));
    assert_eq!(record.key_name, id.key_name());
    assert!(record.paired_at.is_some());
    assert!(RemoteState::file(&data_dir(&dir), id).exists());
    assert_turn(&c);
}

#[test]
fn a_refused_expired_or_cancelled_request_discards_the_key() {
    let first = Server::start();
    let second = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let (c, record) = connected_to(&dir, &first);
    let name = RemoteId::of(&second.key.public).key_name();

    // Not confirmed on the Mac.
    c.send(Input::Pair(
        second.pair_run(vec![second.addr.clone()]).url().unwrap(),
    ));
    let held = expect_words(&c, &second);
    assert!(key_exists(&dir, &name));
    held.answer(PairResult::Refused);
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("not confirmed"), "{why}"),
        e => panic!("{e:?}"),
    }
    assert!(!key_exists(&dir, &name));
    assert_eq!(ids(&dir), [record.id]);

    // The offer expired before yes.
    c.send(Input::Pair(
        second.pair_run(vec![second.addr.clone()]).url().unwrap(),
    ));
    let held = expect_words(&c, &second);
    held.answer(PairResult::Expired);
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("expired"), "{why}"),
        e => panic!("{e:?}"),
    }
    assert!(!key_exists(&dir, &name));

    // Cancel on the phone closes the request's connection.
    c.send(Input::Pair(
        second.pair_run(vec![second.addr.clone()]).url().unwrap(),
    ));
    let held = expect_words(&c, &second);
    c.send(Input::CancelPairing);
    held.closed
        .recv_timeout(TIMEOUT)
        .expect("the daemon sees the connection close");
    assert!(!key_exists(&dir, &name));
    // An answer now adds nothing here.
    let _ = held.answer.send(PairResult::Added);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(ids(&dir), [record.id]);
    assert!(second.seen.try_recv().is_err());

    // A link from another run (a wrong offer): refused at once, with no
    // fallback, and the key discarded.
    let mut stale = second.pair_run(vec![second.addr.clone()]);
    stale.offer = Some([7; OFFER_LEN]);
    c.send(Input::Pair(stale.url().unwrap()));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("current flankr pair run"), "{why}"),
        e => panic!("{e:?}"),
    }
    assert!(!key_exists(&dir, &name));

    // An expired run refuses at once too.
    let link = second.pair_run(vec![second.addr.clone()]);
    second.pair_mode.lock().unwrap().as_mut().unwrap().refuse = Some(PairRefusal::Expired);
    c.send(Input::Pair(link.url().unwrap()));
    match next_event(&c) {
        Event::PairFailed(why) => assert!(why.contains("expired"), "{why}"),
        e => panic!("{e:?}"),
    }
    assert!(!key_exists(&dir, &name));
    assert_eq!(ids(&dir), [record.id]);
    // The connection in use never dropped.
    assert_eq!(first.accepts.load(Ordering::SeqCst), 1);
    assert_turn(&c);
}

#[test]
fn a_request_no_daemon_answers_falls_back_to_the_device_code_with_words() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    // A link with an offer, but the daemon is not in pair mode (an older
    // daemon, or the phone cannot reach its pair mode): it drops the
    // request, and nothing listens at the first address.
    let offer = [3; OFFER_LEN];
    let link = PairingPayload {
        server_key: server.key.public,
        addresses: vec![closed_addr(), server.addr.clone()],
        offer: Some(offer),
    };
    c.send(Input::Pair(link.encode().unwrap()));
    let (code, _, replace) = expect_fallback(&c, &server, Some(offer));
    assert!(!replace);
    assert!(server.held.try_recv().is_err());
    assert_eq!(stored(&dir), Remotes::default());

    // The code pasted into flankr pair (which shows the same words for it)
    // pairs as before.
    let added = server.add(&code);
    expect_paired(&c);
    assert_eq!(server.next_seen().peer_key, added.key);
    expect_connected(&c);
    assert_eq!(ids(&dir), [RemoteId::of(&server.key.public)]);
}

#[test]
fn a_new_run_for_the_daemon_being_paired_keeps_the_key_and_asks_again() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let c = companion(&dir);
    let name = RemoteId::of(&server.key.public).key_name();
    c.send(Input::Pair(
        server.pair_run(vec![server.addr.clone()]).url().unwrap(),
    ));
    let first = expect_words(&c, &server);
    // A new run: a new offer, so new words, but the same key.
    c.send(Input::Pair(
        server.pair_run(vec![server.addr.clone()]).url().unwrap(),
    ));
    first
        .closed
        .recv_timeout(TIMEOUT)
        .expect("the first request's connection closes");
    let second = expect_words(&c, &server);
    assert_eq!(second.request.device_key, first.request.device_key);
    assert_ne!(second.request.offer, first.request.offer);
    assert_eq!(stored_key(&dir, &name), Some(first.request.device_key));
    second.answer(PairResult::Added);
    expect_paired(&c);
    assert_eq!(ids(&dir), [RemoteId::of(&server.key.public)]);
}

// The connection manager (plan 20261028_03, the 18:51 EDT amendment):
// every listed remote has a link, the inactive ones in the background.

/// Waits until `f` holds, for up to 5 s.
fn eventually(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !f() {
        assert!(Instant::now() < deadline, "not within 5 s: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits for an event for which `f` holds, skipping others.
fn expect_event(c: &Companion, what: &str, f: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.wait_timeout(left) {
            Some(e) if f(&e) => return e,
            Some(_) => {}
            None => panic!("no event where {what} within 5 s"),
        }
    }
}

/// The `background` of the last `client.options` the server saw.
fn last_background(server: &Server) -> Option<bool> {
    server
        .seen("client.options")
        .last()
        .map(|r| r["params"]["background"].as_bool().unwrap())
}

/// A companion with `a` (active) and `b` listed, both authorized, once both
/// links are online.
fn both_online(
    dir: &tempfile::TempDir,
    a: &Server,
    b: &Server,
) -> (Companion, ServerRecord, ServerRecord) {
    let mut keys = FileKeyStore::new(keys_dir(dir));
    let ra = remote(&mut keys, a, vec![a.addr.clone()], Some(10), true);
    let rb = remote(&mut keys, b, vec![b.addr.clone()], Some(20), true);
    keys.set_remotes(&Remotes {
        active: Some(ra.id),
        list: vec![ra.clone(), rb.clone()],
    })
    .unwrap();
    let c = companion(dir);
    a.next_seen();
    b.next_seen();
    expect_remotes(&c, "both online", |rows| {
        rows.len() == 2 && rows.iter().all(|r| r.online) && rows[0].active
    });
    eventually("a in front and b in the background", || {
        last_background(a) == Some(false) && last_background(b) == Some(true)
    });
    (c, ra, rb)
}

#[test]
fn every_remote_is_connected_and_a_switch_flips_the_background_without_reconnecting() {
    let (a, b) = (Server::start(), Server::start());
    let dir = tempfile::tempdir().unwrap();
    let (c, ra, rb) = both_online(&dir, &a, &b);
    assert_eq!(a.accepts.load(Ordering::SeqCst), 1);
    assert_eq!(b.accepts.load(Ordering::SeqCst), 1);

    // Ignored: the remote in use, and one not paired.
    c.send(Input::SwitchRemote(ra.id));
    c.send(Input::SwitchRemote(RemoteId::of(&[9; 32])));
    c.send(Input::SwitchRemote(rb.id));
    // b's connection as it stands comes untagged.
    expect_event(
        &c,
        "b's connection, in front",
        |e| matches!(e, Event::Connected { server } if server == "flankr"),
    );
    let rows = expect_remotes(&c, "b in use", |rows| rows[1].active);
    assert!(!rows[0].active && rows.iter().all(|r| r.online));
    assert_eq!(stored(&dir).active, Some(rb.id));
    eventually("a in the background and b in front", || {
        last_background(&a) == Some(true) && last_background(&b) == Some(false)
    });
    // No link was closed or made.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(a.accepts.load(Ordering::SeqCst), 1);
    assert_eq!(b.accepts.load(Ordering::SeqCst), 1);
    assert_eq!(a.links.lock().unwrap().len(), 1);
    assert_eq!(b.links.lock().unwrap().len(), 1);

    // The app's backgrounding puts both in the background; back in front,
    // only b.
    c.send(Input::Background(true));
    eventually("both in the background", || {
        last_background(&a) == Some(true) && last_background(&b) == Some(true)
    });
    c.send(Input::Background(false));
    eventually("b in front again", || last_background(&b) == Some(false));
    assert_eq!(last_background(&a), Some(true));
}

#[test]
fn background_events_arrive_tagged_with_their_remote() {
    let (a, b) = (Server::start(), Server::start());
    let dir = tempfile::tempdir().unwrap();
    let (c, ra, rb) = both_online(&dir, &a, &b);
    b.notify("vocabulary.changed", json!({"vocabulary": ["bee"]}));
    let e = expect_event(
        &c,
        "b's vocabulary",
        |e| matches!(e, Event::Background { event, .. } if matches!(**event, Event::Vocabulary(_))),
    );
    assert_eq!(
        e,
        Event::Background {
            remote: rb.id,
            event: Box::new(Event::Vocabulary(vec!["bee".into()]))
        }
    );
    a.notify("vocabulary.changed", json!({"vocabulary": ["ay"]}));
    expect_event(&c, "a's vocabulary, untagged", |e| {
        *e == Event::Vocabulary(vec!["ay".into()])
    });

    // After the switch, the other way round.
    c.send(Input::SwitchRemote(rb.id));
    expect_remotes(&c, "b in use", |rows| rows[1].active);
    a.notify("vocabulary.changed", json!({"vocabulary": ["ay2"]}));
    expect_event(&c, "a's vocabulary, tagged", |e| {
        *e == Event::Background {
            remote: ra.id,
            event: Box::new(Event::Vocabulary(vec!["ay2".into()])),
        }
    });
}

#[test]
fn each_state_redb_stays_open_while_listed_and_removal_closes_one_link() {
    let (a, b) = (Server::start(), Server::start());
    let dir = tempfile::tempdir().unwrap();
    let (c, ra, rb) = both_online(&dir, &a, &b);
    let data = data_dir(&dir);
    // Both are open (redb holds a lock on each), before and after a switch.
    assert!(RemoteState::open(&data, &ra).is_err());
    assert!(RemoteState::open(&data, &rb).is_err());
    c.send(Input::SwitchRemote(rb.id));
    expect_remotes(&c, "b in use", |rows| rows[1].active);
    assert!(RemoteState::open(&data, &ra).is_err());
    assert!(RemoteState::open(&data, &rb).is_err());

    // Removing a (now in the background) closes its link and its state;
    // b's stay.
    c.send(Input::RemoveRemote(ra.id));
    expect_event(
        &c,
        "a removed",
        |e| matches!(e, Event::RemoteRemoved { id, daemon: true, .. } if *id == ra.id),
    );
    eventually("a's link closed", || a.links.lock().unwrap().is_empty());
    assert!(!RemoteState::dir(&data, ra.id).exists());
    assert!(RemoteState::open(&data, &rb).is_err());
    assert_eq!(b.links.lock().unwrap().len(), 1);
    assert_eq!(b.accepts.load(Ordering::SeqCst), 1);
}

#[test]
fn an_unreachable_remote_stays_in_use_and_retries() {
    let a = Server::start();
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(keys_dir(&dir));
    let ra = remote(&mut keys, &a, vec![a.addr.clone()], Some(10), true);
    let unreachable = Server::start();
    let rb = remote(&mut keys, &unreachable, vec![closed_addr()], Some(20), true);
    keys.set_remotes(&Remotes {
        active: Some(ra.id),
        list: vec![ra.clone(), rb.clone()],
    })
    .unwrap();
    let c = companion(&dir);
    a.next_seen();
    expect_remotes(&c, "a online", |rows| rows[0].online && !rows[1].online);

    c.send(Input::SwitchRemote(rb.id));
    expect_remotes(&c, "b in use, offline", |rows| {
        rows[1].active && !rows[1].online && rows[0].online
    });
    // Its retries come untagged, with their delay; it stays in use.
    for _ in 0..2 {
        expect_event(
            &c,
            "b retrying",
            |e| matches!(e, Event::Disconnected { retry_in_ms } if *retry_in_ms > 0),
        );
    }
    assert_eq!(stored(&dir).active, Some(rb.id));
    eventually("a in the background", || last_background(&a) == Some(true));
    assert_eq!(a.accepts.load(Ordering::SeqCst), 1);
}

#[test]
fn the_drawer_lists_the_active_remote_every_3_s_with_a_view_open() {
    let (a, b) = (Server::start(), Server::start());
    let dir = tempfile::tempdir().unwrap();
    let (c, _ra, _rb) = both_online(&dir, &a, &b);
    c.send(Input::OpenAgent("w1:p1".into()));
    eventually("the view subscribed", || {
        !a.seen("session.subscribe").is_empty()
    });
    std::thread::sleep(Duration::from_millis(200));
    let (before_a, before_b) = (a.seen("agents.list").len(), b.seen("agents.list").len());

    // Shown: once at once, then every 3 s, the view open.
    c.send(Input::DrawerShown(true));
    eventually("a list at once", || {
        a.seen("agents.list").len() == before_a + 1
    });
    std::thread::sleep(Duration::from_millis(3300));
    assert_eq!(a.seen("agents.list").len(), before_a + 2);
    // Only the active remote's.
    assert_eq!(b.seen("agents.list").len(), before_b);

    // Hidden: today's cadence, which lists nothing with a view open.
    c.send(Input::DrawerShown(false));
    std::thread::sleep(Duration::from_millis(3300));
    assert_eq!(a.seen("agents.list").len(), before_a + 2);
}
