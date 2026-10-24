//! The `net` thread against a fake daemon on a Unix socket: hello, a turn,
//! reconnecting with the `session_id`, and backoff.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use handlr_companion::{Companion, CompanionConfig, Event, FileKeyStore, Input, Speaker};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(5);

/// A short directory: Unix socket paths on macOS must be under 104 bytes.
fn short_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("hc")
        .tempdir_in(std::env::temp_dir())
        .unwrap()
}

fn companion(socket: &Path, keys: &Path, initial_ms: u64, max_ms: u64) -> Companion {
    let mut config = CompanionConfig::new("test");
    config.socket = socket.to_path_buf();
    config.backoff_initial = Duration::from_millis(initial_ms);
    config.backoff_max = Duration::from_millis(max_ms);
    Companion::spawn(config, Box::new(FileKeyStore::new(keys)))
}

fn next_event(c: &Companion) -> Event {
    c.wait_timeout(TIMEOUT).expect("an event within 5 s")
}

/// Accepts one connection, failing after 5 s.
fn accept(listener: &UnixListener) -> Conn {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(TIMEOUT)).unwrap();
                return Conn {
                    reader: BufReader::new(stream.try_clone().unwrap()),
                    stream,
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "no connection within 5 s");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("accept: {e}"),
        }
    }
}

struct Conn {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("a line within 5 s");
        serde_json::from_str(&line).unwrap()
    }

    fn write(&mut self, value: Value) {
        let mut line = serde_json::to_vec(&value).unwrap();
        line.push(b'\n');
        self.stream.write_all(&line).unwrap();
    }

    /// Reads `hello` and answers it, then the `client.options` (plan
    /// 20261023_05, A4) and `agents.list` (for herdr's focus, plan
    /// 20261023_02) that follow, returning hello's params.
    fn greet(&mut self) -> Value {
        let hello = self.read();
        assert_eq!(hello["method"], "hello");
        self.write(json!({
            "jsonrpc": "2.0", "id": hello["id"],
            "result": {"server": "handlr", "server_version": "0.1.0", "protocol_version": 1,
                       "session_id": "companion:test", "vocabulary": ["claude", "hr"]}
        }));
        let options = self.read();
        assert_eq!(options["method"], "client.options");
        assert_eq!(
            options["params"],
            json!({"spoken_output": false, "background": false})
        );
        self.write(json!({"jsonrpc": "2.0", "id": options["id"], "result": {}}));
        let list = self.read();
        assert_eq!(list["method"], "agents.list");
        self.write(json!({"jsonrpc": "2.0", "id": list["id"], "result": {"agents": []}}));
        hello["params"].clone()
    }
}

fn expect_connected(c: &Companion) {
    assert_eq!(
        next_event(c),
        Event::Connected {
            server: "handlr".into()
        }
    );
    assert_eq!(
        next_event(c),
        Event::Vocabulary(vec!["claude".into(), "hr".into()])
    );
    // The first list answer, `greet`'s.
    assert_eq!(next_event(c), Event::Agents(vec![]));
}

/// The user's words of a turn, as the overview's conversation shows them.
fn expect_user(c: &Companion, text: &str) {
    assert_eq!(
        next_event(c),
        Event::Conversation {
            speaker: Speaker::User,
            text: text.into()
        }
    );
}

fn paths(dir: &tempfile::TempDir) -> (PathBuf, PathBuf) {
    (dir.path().join("s"), dir.path().join("keys"))
}

#[test]
fn hello_then_a_turn_and_its_say() {
    let dir = short_dir();
    let (socket, keys) = paths(&dir);
    let listener = UnixListener::bind(&socket).unwrap();
    let c = companion(&socket, &keys, 50, 400);
    let mut conn = accept(&listener);
    let hello = conn.greet();
    assert_eq!(
        hello,
        json!({"protocol": "handlr.companion", "version": 1, "client": "test"})
    );
    expect_connected(&c);

    c.send(Input::Utterance("what is claude doing".into()));
    let start = conn.read();
    assert_eq!(start["method"], "turn.start");
    assert_eq!(
        start["params"],
        json!({"turn_id": 1, "text": "what is claude doing"})
    );
    conn.write(json!({"jsonrpc": "2.0", "method": "turn.say",
                      "params": {"turn_id": 1, "seq": 0, "text": "Claude is idle."}}));
    conn.write(json!({"jsonrpc": "2.0", "id": start["id"],
                      "result": {"outcome": "done", "ran": []}}));
    expect_user(&c, "what is claude doing");
    assert_eq!(
        next_event(&c),
        Event::Conversation {
            speaker: Speaker::Gemini,
            text: "Claude is idle.".into()
        }
    );
    assert_eq!(next_event(&c), Event::Say("Claude is idle.".into()));
    assert_eq!(
        next_event(&c),
        Event::TurnEnded {
            outcome: "done".into(),
            stopped: vec![]
        }
    );
}

#[test]
fn a_dropped_connection_reconnects_with_the_session_id() {
    let dir = short_dir();
    let (socket, keys) = paths(&dir);
    let listener = UnixListener::bind(&socket).unwrap();
    let c = companion(&socket, &keys, 50, 400);
    let mut conn = accept(&listener);
    conn.greet();
    expect_connected(&c);
    drop(conn);

    // It had said hello, so the backoff starts again from the first delay.
    assert_eq!(next_event(&c), Event::Disconnected { retry_in_ms: 50 });
    let mut conn = accept(&listener);
    let hello = conn.greet();
    assert_eq!(hello["session_id"], "companion:test");
    expect_connected(&c);
}

#[test]
fn backoff_doubles_while_the_daemon_is_away() {
    let dir = short_dir();
    let (socket, keys) = paths(&dir);
    let c = companion(&socket, &keys, 20, 80);
    let delays: Vec<Event> = (0..4).map(|_| next_event(&c)).collect();
    let expected: Vec<Event> = [20, 40, 80, 80]
        .into_iter()
        .map(|retry_in_ms| Event::Disconnected { retry_in_ms })
        .collect();
    assert_eq!(delays, expected);

    let listener = UnixListener::bind(&socket).unwrap();
    let mut conn = accept(&listener);
    // Events from attempts made before the bind may still be queued.
    while let Some(e) = c.poll().next() {
        assert!(matches!(e, Event::Disconnected { .. }), "{e:?}");
    }
    conn.greet();
    let mut e = next_event(&c);
    while matches!(e, Event::Disconnected { .. }) {
        e = next_event(&c);
    }
    assert_eq!(
        e,
        Event::Connected {
            server: "handlr".into()
        }
    );
}

#[test]
fn a_refused_hello_is_an_error_and_a_retry() {
    let dir = short_dir();
    let (socket, keys) = paths(&dir);
    let listener = UnixListener::bind(&socket).unwrap();
    let c = companion(&socket, &keys, 50, 400);
    let mut conn = accept(&listener);
    let hello = conn.read();
    conn.write(json!({"jsonrpc": "2.0", "id": hello["id"],
                      "error": {"code": -32011, "message": "unsupported protocol"}}));
    assert!(matches!(next_event(&c), Event::Error(m) if m.contains("-32011")));
    assert_eq!(next_event(&c), Event::Disconnected { retry_in_ms: 50 });
    // And it tries again.
    let mut conn = accept(&listener);
    assert_eq!(conn.read()["method"], "hello");
}

#[test]
fn a_tap_during_a_turn_interrupts_it_over_the_socket() {
    let dir = short_dir();
    let (socket, keys) = paths(&dir);
    let listener = UnixListener::bind(&socket).unwrap();
    let c = companion(&socket, &keys, 50, 400);
    let mut conn = accept(&listener);
    conn.greet();
    expect_connected(&c);
    c.send(Input::Utterance("send continue to the handlr agent".into()));
    let start = conn.read();
    c.send(Input::Tap);
    expect_user(&c, "send continue to the handlr agent");
    assert_eq!(next_event(&c), Event::StopSpeaking);
    let interrupt = conn.read();
    assert_eq!(interrupt["method"], "turn.interrupt");
    assert_eq!(interrupt["params"], json!({"turn_id": 1}));
    conn.write(json!({"jsonrpc": "2.0", "id": interrupt["id"], "result": {}}));
    conn.write(json!({"jsonrpc": "2.0", "id": start["id"], "result": {
        "outcome": "interrupted", "ran": [],
        "interrupted": [{"pane_id": "w2:pZ", "name": "handlr", "spoken": "the handlr agent"}]
    }}));
    assert_eq!(
        next_event(&c),
        Event::Say("Stopped the handlr agent.".into())
    );
    assert!(matches!(next_event(&c), Event::TurnEnded { outcome, .. } if outcome == "interrupted"));
}
