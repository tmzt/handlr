//! Binary transfers over flankr link
//! (docs/design/20261028_05_BINARY_FILES_PLAN.md, step 7), against an
//! in-test daemon: a `link::accept` server whose `hello` carries `blob`
//! and which answers kind 2 with libflankr_blob's state machines as the
//! daemon's own half does: it ACCEPTs an image OFFER (after a pause, to
//! catch a CHUNK sent before the ACCEPT), answers END with DONE and an
//! 8-byte reference, serves `files` and `git` REQUESTs from a table, and
//! takes `have_it`. It records `agents.send` and `terminal.paste`.
//!
//! The client must upload two images and send `parts` naming their
//! references, fetch a Markdown file and a `git show` into the preview
//! cache, refuse a cached file's OFFER with `have_it`, detach an image
//! whose token was deleted, restart an upload after a reconnect, send no
//! kind 2 to a daemon without `blob`, and paste into the terminal.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use flankr_companion::{
    Attachment, Companion, CompanionConfig, Event, FileKeyStore, Input, KeyStore, LEGACY_KEY_NAME,
    Remotes, ServerRecord,
};
use libflankr_blob::{
    BlobMessage, Body, CHUNK_MAX, Code, NS_FILES, NS_GIT, NS_IMAGES, Offer, Receiving, Sending,
    Step,
};
use libflankr_proto::link::{self, Message, StaticKeypair};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

const TIMEOUT: Duration = Duration::from_secs(5);
const SESSION: &str = "w1:p1";
const MARKDOWN: &[u8] = b"# A plan\n\nSome *text*.\n";
const SHOW: &[u8] = b"commit 1a2b3c4d\n\ndiff --git a/a.txt b/a.txt\n+one\n";

/// What the daemon saw.
#[derive(Debug, Clone, PartialEq)]
enum Seen {
    /// A JSON-RPC message.
    Json(Value),
    /// A kind-2 message: its subcommand, transfer, namespace and, for a
    /// REQUEST or OFFER, its session.
    Blob { body: Body, id: u32, ns: String },
    /// A CHUNK that came before the image's ACCEPT was sent.
    EarlyChunk(u32),
    /// A transfer's bytes stored, by id.
    Stored(u32, Vec<u8>),
}

/// What the test tells the daemon.
enum Cmd {
    Hangup,
}

struct Daemon {
    addr: String,
    key: StaticKeypair,
    seen: mpsc::Receiver<Seen>,
    cmds: UnboundedSender<Cmd>,
}

#[derive(Clone)]
struct Ctx {
    key: StaticKeypair,
    client: [u8; 32],
    seen: mpsc::Sender<Seen>,
    cmds: Arc<tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<Cmd>>>,
    /// `blob` in hello.
    blob: bool,
    /// Never ACCEPT on the first connection (the reconnect test).
    stall_first: bool,
    connections: Arc<Mutex<u32>>,
}

impl Daemon {
    fn start(client: [u8; 32], blob: bool, stall_first: bool) -> Daemon {
        let key = link::generate_keypair().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (seen_tx, seen) = mpsc::channel();
        let (cmds, cmds_rx) = unbounded_channel();
        let ctx = Ctx {
            key: key.clone(),
            client,
            seen: seen_tx,
            cmds: Arc::new(tokio::sync::Mutex::new(cmds_rx)),
            blob,
            stall_first,
            connections: Arc::new(Mutex::new(0)),
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
                    let ctx = ctx.clone();
                    tokio::spawn(async move { ctx.serve(stream).await });
                }
            });
        });
        Daemon {
            addr,
            key,
            seen,
            cmds,
        }
    }

    /// The next seen thing for which `f` gives `Some`, others dropped.
    fn next<T>(&self, what: &str, f: impl Fn(&Seen) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let s = self
                .seen
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("{what} within 5 s"));
            if let Seen::EarlyChunk(id) = s {
                panic!("transfer {id}: a CHUNK before its ACCEPT");
            }
            if let Some(t) = f(&s) {
                return t;
            }
        }
    }

    fn request(&self, method: &str) -> Value {
        self.next(method, |s| match s {
            Seen::Json(m) if m["method"] == method => Some(m.clone()),
            _ => None,
        })
    }

    /// Everything seen within `wait`.
    fn drain(&self, wait: Duration) -> Vec<Seen> {
        let deadline = Instant::now() + wait;
        let mut all = Vec::new();
        while let Ok(s) = self
            .seen
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            all.push(s);
        }
        all
    }
}

/// One transfer on the daemon's side.
enum Live {
    Up(Box<Receiving>, Vec<u8>),
    Down(Sending, Vec<u8>),
}

impl Ctx {
    async fn serve(self, stream: tokio::net::TcpStream) {
        let client = self.client;
        let Ok(accepted) = link::accept(stream, &self.key, |key| *key == client).await else {
            return;
        };
        let connection = {
            let mut n = self.connections.lock().unwrap();
            *n += 1;
            *n
        };
        let stall = self.stall_first && connection == 1;
        let first: Value =
            serde_json::from_slice(&accepted.first_message.unwrap_or_default()).unwrap();
        let mut result = json!({
            "server": "flankr", "server_version": "0.1.0", "protocol_version": 1,
            "session_id": "companion:phone", "vocabulary": [],
            "terminal": {"input": true}
        });
        if self.blob {
            result["blob"] = json!({
                "version": 1, "namespaces": ["images", "files", "git"],
                "read": ["docs/**/*.md"], "image_max": 20971520, "file_max": 8388608,
                "git_max": 8388608, "chunk_max": 983040, "images_per_send": 4
            });
        }
        let (mut read, mut write) = accepted.stream.into_split();
        let hello = json!({"jsonrpc": "2.0", "id": first["id"], "result": result});
        if write
            .send_message(&serde_json::to_vec(&hello).unwrap())
            .await
            .is_err()
        {
            return;
        }
        let (from_tx, mut from) = unbounded_channel::<Message>();
        let reader = tokio::spawn(async move {
            while let Ok(m) = read.recv().await {
                if from_tx.send(m).is_err() {
                    break;
                }
            }
        });
        let mut cmds = self.cmds.lock().await;
        let mut live: HashMap<u32, Live> = HashMap::new();
        let mut references = 0u8;
        let mut early: Vec<Message> = Vec::new();
        loop {
            let m = if early.is_empty() {
                tokio::select! {
                    m = from.recv() => match m { Some(m) => m, None => break },
                    cmd = cmds.recv() => match cmd {
                        Some(Cmd::Hangup) | None => break,
                    },
                }
            } else {
                early.remove(0)
            };
            let mut replies: Vec<Vec<u8>> = Vec::new();
            match m {
                Message::Json(bytes) => {
                    let m: Value = serde_json::from_slice(&bytes).unwrap();
                    let method = m["method"].as_str().unwrap_or("").to_string();
                    let result = match method.as_str() {
                        "agents.list" => json!({"agents": [{
                            "pane_id": SESSION, "agent": "claude", "status": "idle",
                            "focused": true, "cwd": "/p", "project": "p"
                        }]}),
                        "agents.send" => json!({"status": "sent"}),
                        "terminal.open" => json!({"terminal_id": 7, "cols": 80, "rows": 24}),
                        "terminal.input_arm" => json!({"expires_in_s": 0}),
                        _ => json!({}),
                    };
                    if m.get("id").is_some() {
                        let r = json!({"jsonrpc": "2.0", "id": m["id"], "result": result});
                        if write
                            .send_message(&serde_json::to_vec(&r).unwrap())
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    let _ = self.seen.send(Seen::Json(m));
                }
                Message::Binary(bytes) => {
                    let msg = BlobMessage::decode(&bytes).expect("a message that decodes");
                    let id = msg.transfer;
                    let ns = msg.ns.clone();
                    let _ = self.seen.send(Seen::Blob {
                        body: msg.body.clone(),
                        id: id.0,
                        ns: ns.as_str().to_string(),
                    });
                    let now = Instant::now();
                    let encode = |body: Body| BlobMessage::new(id, ns.clone(), body).encode();
                    match (ns.as_str(), msg.body, live.remove(&id.0)) {
                        (NS_IMAGES, Body::Offer(offer), None) => {
                            assert_eq!(offer.session, SESSION);
                            let mut rx = Box::new(Receiving::offered(offer, now));
                            if stall {
                                live.insert(id.0, Live::Up(rx, Vec::new()));
                                continue;
                            }
                            // A pause: any CHUNK that comes meanwhile was
                            // sent before the ACCEPT.
                            tokio::time::sleep(Duration::from_millis(150)).await;
                            while let Ok(m) = from.try_recv() {
                                if let Message::Binary(b) = &m
                                    && let Ok(BlobMessage {
                                        transfer,
                                        body: Body::Chunk { .. },
                                        ..
                                    }) = BlobMessage::decode(b)
                                    && transfer == id
                                {
                                    let _ = self.seen.send(Seen::EarlyChunk(id.0));
                                }
                                early.push(m);
                            }
                            replies.push(encode(rx.accept(CHUNK_MAX, now).unwrap()));
                            live.insert(id.0, Live::Up(rx, Vec::new()));
                        }
                        (NS_IMAGES, body, Some(Live::Up(mut rx, mut buf))) => {
                            match rx.on_message(&body, now) {
                                Step::Store => {
                                    if let Body::Chunk { data, .. } = body {
                                        buf.extend_from_slice(&data);
                                    }
                                    live.insert(id.0, Live::Up(rx, buf));
                                }
                                Step::Complete => {
                                    references += 1;
                                    let reference = vec![0xa0 + references; 8];
                                    let _ = self.seen.send(Seen::Stored(id.0, buf));
                                    replies.push(encode(rx.done(reference)));
                                }
                                Step::Abort(b) => replies.push(encode(b)),
                                _ => {}
                            }
                        }
                        (
                            NS_FILES | NS_GIT,
                            Body::Request {
                                session, request, ..
                            },
                            None,
                        ) => {
                            assert_eq!(session, SESSION);
                            let (mime, bytes): (&str, &[u8]) = match (ns.as_str(), request.as_str())
                            {
                                (NS_FILES, "docs/a.md") => ("text/markdown", MARKDOWN),
                                (NS_GIT, "show 1a2b3c4d") => ("text/x-diff", SHOW),
                                _ => {
                                    replies.push(encode(Body::refuse(Code::NOT_FOUND, "no")));
                                    ("", b"")
                                }
                            };
                            if !mime.is_empty() {
                                let offer = Offer::for_bytes(&session, &request, mime, bytes, 0);
                                live.insert(
                                    id.0,
                                    Live::Down(Sending::new(&offer, now), bytes.to_vec()),
                                );
                                replies.push(encode(Body::Offer(offer)));
                            }
                        }
                        (NS_FILES | NS_GIT, body, Some(Live::Down(mut tx, bytes))) => {
                            match tx.on_message(&body, now) {
                                Step::Accepted { .. } => {
                                    // Small chunks, so a file takes several.
                                    while let Some((at, n)) = tx.next_chunk(5) {
                                        let at_ = at as usize;
                                        replies.push(BlobMessage::encode_chunk(
                                            id,
                                            &ns,
                                            at,
                                            &bytes[at_..at_ + n],
                                        ));
                                    }
                                    replies.push(encode(tx.end(now).unwrap()));
                                    live.insert(id.0, Live::Down(tx, bytes));
                                }
                                Step::Done(body) => assert!(body.is_empty(), "a files DONE body"),
                                Step::Ended { .. } => {}
                                Step::Abort(b) => replies.push(encode(b)),
                                _ => {}
                            }
                        }
                        (_, Body::Cancel { .. }, _) => {}
                        (ns, body, _) => panic!("unexpected {body:?} in {ns}"),
                    }
                }
                _ => {}
            }
            for r in replies {
                if write.send_binary(&r).await.is_err() {
                    break;
                }
            }
        }
        reader.abort();
        let _ = write.shutdown().await;
    }
}

fn paired(blob: bool, stall_first: bool) -> (Daemon, Companion, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut keys = FileKeyStore::new(dir.path().join("keys"));
    let client = keys.generate_static_key(LEGACY_KEY_NAME).unwrap().public();
    let daemon = Daemon::start(client, blob, stall_first);
    let mut record = ServerRecord::new("phone", daemon.key.public, vec![daemon.addr.clone()]);
    record.key_name = LEGACY_KEY_NAME.into();
    keys.set_remotes(&Remotes {
        active: Some(record.id),
        list: vec![record],
    })
    .unwrap();
    let mut config = CompanionConfig::new("test");
    config.socket = dir.path().join("no-socket");
    config.backoff_initial = Duration::from_millis(50);
    config.backoff_max = Duration::from_millis(200);
    config.cache_dir = Some(dir.path().join("cache"));
    let c = Companion::spawn(config, Box::new(FileKeyStore::new(dir.path().join("keys"))));
    (daemon, c, dir)
}

/// The next event for which `f` gives `Some`, others dropped.
fn wait<T>(c: &Companion, what: &str, f: impl Fn(Event) -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match c.wait_timeout(left) {
            Some(e) => {
                if let Some(t) = f(e) {
                    return t;
                }
            }
            None => panic!("{what} within 5 s"),
        }
    }
}

/// Connected, the agent listed, its view open and the message card open.
fn ready(d: &Daemon, c: &Companion) {
    wait(c, "connected", |e| {
        matches!(e, Event::Connected { .. }).then_some(())
    });
    d.request("agents.list");
    wait(c, "the agents", |e| match e {
        Event::Agents(a) if !a.is_empty() => Some(()),
        _ => None,
    });
    c.send(Input::OpenAgent(SESSION.into()));
    c.send(Input::StartTextPrompt);
}

/// A PNG's magic, then `len` bytes in all.
fn png(len: usize) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.extend((0..len - 8).map(|i| i as u8));
    b
}

fn jpeg(len: usize) -> Vec<u8> {
    let mut b = vec![0xff, 0xd8, 0xff, 0xe0];
    b.extend((0..len - 4).map(|i| (i * 7) as u8));
    b
}

fn image_ready(c: &Companion, n: u8) {
    wait(c, &format!("image {n} ready"), |e| {
        (e == Event::ImageReady { n }).then_some(())
    });
}

/// The next [`Event::FileReady`] or [`Event::TransferFailed`].
fn file(c: &Companion) -> Result<(std::path::PathBuf, String, String), (u16, Option<String>)> {
    wait(c, "a file or a failure", |e| match e {
        Event::FileReady {
            path, mime, name, ..
        } => Some(Ok((path, mime, name))),
        Event::TransferFailed { code, line, .. } => Some(Err((code, line))),
        _ => None,
    })
}

#[test]
fn uploads_two_images_sends_parts_and_fetches_a_file_and_a_git_show() {
    let (d, c, dir) = paired(true, false);
    ready(&d, &c);
    // 600 KiB: three CHUNKs of 256 KiB at most.
    let one = png(600 << 10);
    let two = jpeg(40);
    c.send(Input::AttachImage {
        bytes: one.clone(),
        mime: "image/png".into(),
    });
    c.send(Input::AttachImage {
        bytes: two.clone(),
        mime: "image/jpeg".into(),
    });
    image_ready(&c, 1);
    image_ready(&c, 2);
    // The daemon stored both, byte for byte.
    let stored: Vec<Vec<u8>> = (0..2)
        .map(|_| {
            d.next("stored", |s| match s {
                Seen::Stored(_, b) => Some(b.clone()),
                _ => None,
            })
        })
        .collect();
    assert!(stored.contains(&one) && stored.contains(&two));

    c.send(Input::SetText(
        "look at [image 1] and [image 2] please".into(),
    ));
    c.send(Input::Submit);
    let send = d.request("agents.send");
    let p = &send["params"];
    assert_eq!(p["pane_id"], SESSION);
    // `text` is `command`'s alias, as every message sends it.
    assert_eq!(p["text"], "");
    assert_eq!(p["kind"], "message");
    assert_eq!(p["confirmed"], true);
    // References in upload order: the first DONE's is a1…, the second's a2….
    let parts = p["parts"].as_array().unwrap().clone();
    assert_eq!(parts.len(), 5, "{parts:?}");
    assert_eq!(parts[0], json!({"text": "look at"}));
    assert_eq!(parts[2], json!({"text": "and"}));
    assert_eq!(parts[4], json!({"text": "please"}));
    let refs: Vec<&str> = [&parts[1], &parts[3]]
        .iter()
        .map(|p| p["image"].as_str().unwrap())
        .collect();
    let mut sorted = refs.clone();
    sorted.sort();
    assert_eq!(sorted, ["a1a1a1a1a1a1a1a1", "a2a2a2a2a2a2a2a2"]);
    // Sent: the images go with the card.
    wait(&c, "no attachments", |e| match e {
        Event::Attachments(a) if a.is_empty() => Some(()),
        _ => None,
    });

    // A Markdown file, into the preview cache.
    c.send(Input::RequestFile {
        session: SESSION.into(),
        path: "docs/a.md".into(),
    });
    let (path, mime, name) = file(&c).unwrap();
    assert_eq!(
        (mime.as_str(), name.as_str()),
        ("text/markdown", "docs/a.md")
    );
    assert!(path.starts_with(dir.path().join("cache").join("libmobile-share")));
    assert!(
        path.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("fl-")
    );
    assert_eq!(std::fs::read(&path).unwrap(), MARKDOWN);

    // Again: the OFFER is refused with have_it, and the cached copy used.
    c.send(Input::RequestFile {
        session: SESSION.into(),
        path: "docs/a.md".into(),
    });
    let (again, ..) = file(&c).unwrap();
    assert_eq!(again, path);
    let code = d.next("have_it", |s| match s {
        Seen::Blob {
            body: Body::Refuse { code, .. },
            ..
        } => Some(*code),
        _ => None,
    });
    assert_eq!(code, Code::HAVE_IT);

    // A git show, as a diff.
    c.send(Input::RequestGit {
        session: SESSION.into(),
        request: "show 1a2b3c4d".into(),
    });
    let (path, mime, name) = file(&c).unwrap();
    assert_eq!(
        (mime.as_str(), name.as_str()),
        ("text/x-diff", "show 1a2b3c4d")
    );
    assert!(path.extension().unwrap() == "diff");
    assert_eq!(std::fs::read(&path).unwrap(), SHOW);

    // A file the daemon refuses gives its line.
    c.send(Input::RequestFile {
        session: SESSION.into(),
        path: "docs/none.md".into(),
    });
    assert_eq!(
        file(&c).unwrap_err(),
        (Code::NOT_FOUND.0, Some("File not found.".to_string()))
    );
    // Every message the client sent decoded, and no CHUNK came early.
    for s in d.drain(Duration::from_millis(200)) {
        assert!(!matches!(s, Seen::EarlyChunk(_)), "{s:?}");
    }
}

#[test]
fn a_deleted_token_detaches_its_image_and_the_x_removes_both() {
    let (d, c, _dir) = paired(true, false);
    ready(&d, &c);
    for _ in 0..2 {
        c.send(Input::AttachImage {
            bytes: png(64),
            mime: "image/png".into(),
        });
    }
    image_ready(&c, 1);
    image_ready(&c, 2);
    c.send(Input::SetText("a [image 1] b [image 2] c".into()));
    // Image 1's token deleted: it goes.
    c.send(Input::SetText("a b [image 2] c".into()));
    let left = wait(&c, "image 1 detached", |e| match e {
        Event::Attachments(a) if a.len() == 1 => Some(a),
        _ => None,
    });
    assert_eq!(left[0].n, 2);
    // The ✕ on image 2: it goes, and its token with it.
    c.send(Input::DetachImage(2));
    wait(&c, "image 2 detached", |e| match e {
        Event::Attachments(a) if a.is_empty() => Some(()),
        _ => None,
    });
    let text = wait(&c, "the token gone", |e| match e {
        Event::Dictation(d) if d.active => Some(d.text),
        _ => None,
    });
    assert_eq!(text, "a b c");
    // Not ready, not attached: sent as text.
    c.send(Input::SetText("a [image 3]".into()));
    c.send(Input::Submit);
    let send = d.request("agents.send");
    assert_eq!(send["params"]["text"], "a [image 3]");
    assert!(send["params"].get("parts").is_none());
}

#[test]
fn an_upload_restarts_after_a_reconnect() {
    let (d, c, _dir) = paired(true, true);
    ready(&d, &c);
    c.send(Input::AttachImage {
        bytes: png(64),
        mime: "image/png".into(),
    });
    let first = d.next("the first OFFER", |s| match s {
        Seen::Blob {
            body: Body::Offer(_),
            id,
            ..
        } => Some(*id),
        _ => None,
    });
    // No ACCEPT: nothing more is sent for it.
    for s in d.drain(Duration::from_millis(300)) {
        assert!(
            !matches!(s, Seen::Blob { id, .. } if id == first),
            "{s:?} before ACCEPT"
        );
    }
    d.cmds.send(Cmd::Hangup).unwrap();
    // The next connection's OFFER, with the ids counted from 1 again.
    let again = d.next("the OFFER again", |s| match s {
        Seen::Blob {
            body: Body::Offer(o),
            id,
            ..
        } => Some((*id, o.size)),
        _ => None,
    });
    assert_eq!(again, (1, 64));
    image_ready(&c, 1);
}

#[test]
fn a_daemon_without_blob_gets_no_kind_two() {
    let (d, c, _dir) = paired(false, false);
    ready(&d, &c);
    c.send(Input::AttachImage {
        bytes: png(64),
        mime: "image/png".into(),
    });
    let e = wait(&c, "the refusal", |e| match e {
        Event::Error(e) => Some(e),
        _ => None,
    });
    assert_eq!(e, "This daemon can't take images.");
    c.send(Input::RequestFile {
        session: SESSION.into(),
        path: "docs/a.md".into(),
    });
    assert_eq!(
        file(&c).unwrap_err(),
        (
            Code::FILES_OFF.0,
            Some("File access is off on this daemon.".to_string())
        )
    );
    for s in d.drain(Duration::from_millis(300)) {
        assert!(matches!(s, Seen::Json(_)), "{s:?}");
    }
}

#[test]
fn a_bad_image_is_refused_on_the_phone() {
    let (d, c, _dir) = paired(true, false);
    ready(&d, &c);
    // PNG bytes said to be a JPEG.
    c.send(Input::AttachImage {
        bytes: png(64),
        mime: "image/jpeg".into(),
    });
    let e = wait(&c, "the refusal", |e| match e {
        Event::Error(e) => Some(e),
        _ => None,
    });
    assert_eq!(e, "Not a PNG, JPEG, WebP or GIF image.");
    for s in d.drain(Duration::from_millis(200)) {
        assert!(matches!(s, Seen::Json(_)), "{s:?}");
    }
}

#[test]
fn text_and_images_paste_into_the_armed_terminal() {
    let (d, c, _dir) = paired(true, false);
    ready(&d, &c);
    c.send(Input::TerminalOpen {
        session: SESSION.into(),
        cols_hint: None,
        rows_hint: None,
    });
    d.request("terminal.open");
    let deadline = Instant::now() + TIMEOUT;
    while !c.terminals().status(SESSION).is_some_and(|s| s.open) {
        assert!(Instant::now() < deadline, "open within 5 s");
        let _ = c.poll().count();
        std::thread::sleep(Duration::from_millis(10));
    }
    c.send(Input::TerminalArm(SESSION.into()));
    d.request("terminal.input_arm");
    let deadline = Instant::now() + TIMEOUT;
    while !c.terminals().armed(SESSION) {
        assert!(Instant::now() < deadline, "armed within 5 s");
        let _ = c.poll().count();
        std::thread::sleep(Duration::from_millis(10));
    }
    c.send(Input::TerminalPasteText("hello\nworld".into()));
    let paste = d.request("terminal.paste");
    assert_eq!(
        paste["params"],
        json!({"terminal_id": 7, "text": "hello\nworld"})
    );
    c.send(Input::AttachImage {
        bytes: png(64),
        mime: "image/png".into(),
    });
    image_ready(&c, 1);
    c.send(Input::TerminalPasteImages(vec![1]));
    let paste = d.request("terminal.paste");
    assert_eq!(
        paste["params"],
        json!({"terminal_id": 7, "images": ["a1a1a1a1a1a1a1a1"]})
    );
    // Pasted: used up.
    let a: Vec<Attachment> = wait(&c, "image 1 used", |e| match e {
        Event::Attachments(a) if a.is_empty() => Some(a),
        _ => None,
    });
    assert!(a.is_empty());
}
