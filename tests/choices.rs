//! Choice questions end to end (docs/design/20261029_01_CHOICE_QUESTIONS_PLAN.md,
//! steps 3 and 4): `agents.send` refused while one is open, and
//! `choice.answer` driven into herdr's mock, whose visible frame a script
//! changes as keys arrive (recorded frames written from finding 3's
//! strings), with the agent's result appended to a synthetic session file.
//! Nothing reaches a live herdr, a real session or Gemini.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use flankr::config::{AgentConfig, AppConfig};
use flankr::reasoning::ToolExecutor;
use flankr::server::companion::TurnOptions;
use flankr::server::{self, Daemon, ServeOptions};
use flankr::sessions::Sessions;
use flankr::transport::{
    AgentSession, AgentStatus, HerdrClient, HerdrToolExecutor, HerdrTransport,
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::codec::{FramedRead, LinesCodec};

const WAIT: Duration = Duration::from_secs(15);
const SESSION: &str = "00000000-0000-4000-8000-0000000000c1";
const CALL: &str = "toolu_01CHOICETEST";

struct Env {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    file: PathBuf,
    daemon: Arc<Daemon>,
}

fn claude(messages: bool) -> AppConfig {
    AppConfig {
        agents: vec![
            AgentConfig::new("claude", vec!["continue".to_string()], false).with_messages(messages),
        ],
        summary: Some(flankr::config::SummaryConfig { cards: false }),
        ..AppConfig::default()
    }
}

fn line(v: Value) -> String {
    let mut s = v.to_string();
    s.push('\n');
    s
}

fn prompt() -> String {
    line(
        json!({"type": "user", "timestamp": "2026-10-29T04:10:00.000Z", "isSidechain": false,
        "message": {"role": "user", "content": "style the badges"}, "origin": {"kind": "human"}}),
    )
}

fn ask(input: Value) -> String {
    line(json!({
        "type": "assistant", "timestamp": "2026-10-29T04:11:00.000Z", "isSidechain": false,
        "version": "2.1.310",
        "message": {"role": "assistant", "type": "message", "content": [
            {"type": "tool_use", "id": CALL, "name": "AskUserQuestion", "input": input}
        ]}
    }))
}

fn result(answers: Value) -> String {
    line(json!({
        "type": "user", "timestamp": "2026-10-29T04:12:00.000Z", "isSidechain": false,
        "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": CALL, "content": "Your questions have been answered."}
        ]},
        "toolUseResult": {"questions": [], "answers": answers, "annotations": {}}
    }))
}

/// Two questions: a single-select, then a multi-select.
fn two_questions() -> Value {
    json!({"questions": [
        {"question": "Which colour should the badge be?", "header": "Colour",
         "multiSelect": false, "options": [
            {"label": "Teal", "description": "Matches the bar"},
            {"label": "Amber", "description": "Stands out"}]},
        {"question": "Which rows get icons?", "header": "Icons",
         "multiSelect": true, "options": [
            {"label": "Files", "description": "File rows"},
            {"label": "Folders", "description": "Folder rows"},
            {"label": "Links", "description": "Link rows"}]}
    ]})
}

fn one_question() -> Value {
    json!({"questions": [
        {"question": "Which colour should the badge be?", "header": "Colour",
         "multiSelect": false, "options": [
            {"label": "Teal", "description": "Matches the bar"},
            {"label": "Amber", "description": "Stands out"}]}
    ]})
}

async fn start(config: AppConfig, session: &str) -> Env {
    let dir = tempfile::Builder::new()
        .prefix("hch")
        .tempdir_in("/tmp")
        .unwrap();
    let home = dir.path().join("home");
    let projects = home.join(".claude/projects/-work-example");
    std::fs::create_dir_all(&projects).unwrap();
    let file = projects.join(format!("{SESSION}.jsonl"));
    std::fs::write(&file, session).unwrap();
    let socket = dir.path().join("h.sock");
    let client = HerdrClient::mock();
    if let HerdrTransport::Mock { state } = client.transport() {
        let mut state = state.write().await;
        let pane = &mut state.snapshot.workspaces[0].tabs[0].panes[0];
        pane.cwd = Some("/work/example".to_string());
        pane.agent_status = Some(AgentStatus::Blocked);
        pane.agent_session = Some(AgentSession {
            source: "herdr:claude".to_string(),
            agent: "claude".to_string(),
            kind: "id".to_string(),
            value: SESSION.to_string(),
        });
    }
    let daemon = Daemon::with_turns(
        client,
        &config,
        None,
        TurnOptions {
            session_dir: Some(dir.path().join("sessions")),
            agent_sessions: Some(Arc::new(Sessions::with_home(&home))),
            ..TurnOptions::default()
        },
    );
    let listener = server::bind(&socket).await.unwrap();
    tokio::spawn(server::run(
        Arc::clone(&daemon),
        listener,
        ServeOptions {
            show_tokens: false,
            event_transport: None,
        },
    ));
    Env {
        _dir: dir,
        socket,
        file,
        daemon,
    }
}

fn append(file: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(file).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

async fn keys(daemon: &Daemon) -> Vec<String> {
    match daemon.client.transport() {
        HerdrTransport::Mock { state } => state
            .read()
            .await
            .keystroke_log
            .iter()
            .map(|(_, k)| k.clone())
            .collect(),
        _ => unreachable!("tests use the mock"),
    }
}

async fn commands(daemon: &Daemon) -> Vec<String> {
    match daemon.client.transport() {
        HerdrTransport::Mock { state } => state
            .read()
            .await
            .command_log
            .iter()
            .map(|(_, c)| c.clone())
            .collect(),
        _ => unreachable!("tests use the mock"),
    }
}

/// Show `frames[n]` as pane p1's visible frame once `n` keys have been
/// sent (the last frame from then on), and append `then` to `file` once
/// `after` keys have been sent.
fn script(daemon: &Arc<Daemon>, frames: Vec<String>, then: Option<(PathBuf, String, usize)>) {
    let daemon = Arc::clone(daemon);
    tokio::spawn(async move {
        let mut done = false;
        loop {
            if let HerdrTransport::Mock { state } = daemon.client.transport() {
                let mut state = state.write().await;
                let n = state.keystroke_log.len();
                let frame = &frames[n.min(frames.len() - 1)];
                state.pane_buffers.insert(
                    "p1".to_string(),
                    frame.lines().map(str::to_string).collect(),
                );
                if let Some((file, text, after)) = &then
                    && !done
                    && n >= *after
                {
                    append(file, text);
                    done = true;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
}

/// Claude Code's question screen as finding 3 gives its strings.
fn frame(question: &str, labels: &[&str], multi: bool, other_text: Option<&str>) -> String {
    let mut f = String::from("╭──────────────────────────────╮\n");
    f.push_str("│ ← ☐ Colour  ☐ Icons  ✔ Submit → │\n│\n");
    f.push_str(&format!("│ {question}\n│\n"));
    for (i, l) in labels.iter().enumerate() {
        let pointer = if i == 0 { "❯" } else { " " };
        let check = if multi { "[ ] " } else { "" };
        f.push_str(&format!("│ {pointer} {}. {check}{l}\n", i + 1));
    }
    let placeholder = if multi {
        "Type something"
    } else {
        "Type something."
    };
    f.push_str(&format!(
        "│   {}. {}\n",
        labels.len() + 1,
        other_text.unwrap_or(placeholder)
    ));
    if !multi {
        f.push_str(&format!(
            "│ ────────\n│   {}. Chat about this\n",
            labels.len() + 2
        ));
    }
    f.push_str("│ Enter to select · Tab/Arrow keys to navigate · Esc to cancel\n");
    f.push_str("╰──────────────────────────────╯\n");
    f
}

const REVIEW: &str = "│ Review your answers\n│\n│ ● Which colour should the badge be?\n│   → Amber\n│\n│ Ready to submit your answers?\n│\n│ ❯ 1. Submit answers\n│   2. Cancel\n";

fn colour_frame(other_text: Option<&str>) -> String {
    frame(
        "Which colour should the badge be?",
        &["Teal", "Amber"],
        false,
        other_text,
    )
}

fn icons_frame() -> String {
    frame(
        "Which rows get icons?",
        &["Files", "Folders", "Links"],
        true,
        None,
    )
}

/// The frames of the two-question answer `{picked: [1]}`, `{picked: [0,
/// 2]}`: question 1 until its key, question 2 for its 6 keys, then the
/// review.
fn two_question_frames() -> Vec<String> {
    let mut frames = vec![colour_frame(None)];
    frames.extend(std::iter::repeat_n(icons_frame(), 7));
    frames.push(REVIEW.to_string());
    frames
}

/// `2`; `1`, `3`, `down` ×4, `enter`; `1` on the review.
const TWO_QUESTION_KEYS: [&str; 9] = ["2", "1", "3", "down", "down", "down", "down", "enter", "1"];

fn two_question_answer() -> Value {
    json!({"pane_id": "p1", "id": CALL, "answers": [{"picked": [1]}, {"picked": [0, 2]}]})
}

struct Client {
    write: OwnedWriteHalf,
    lines: FramedRead<OwnedReadHalf, LinesCodec>,
    next_id: i64,
}

impl Client {
    async fn connect(env: &Env, protocol: &str) -> Client {
        let (read, write) = UnixStream::connect(&env.socket).await.unwrap().into_split();
        let mut c = Client {
            write,
            lines: FramedRead::new(read, LinesCodec::new()),
            next_id: 0,
        };
        let hello = c
            .call(
                "hello",
                json!({"protocol": protocol, "version": 1, "client": "t"}),
            )
            .await;
        assert_eq!(hello["result"]["server"], "flankr", "{hello}");
        c
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let mut text =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        text.push('\n');
        self.write.write_all(text.as_bytes()).await.unwrap();
        loop {
            let line = tokio::time::timeout(WAIT, self.lines.next())
                .await
                .expect("no response in time")
                .unwrap()
                .unwrap();
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg["id"] == id && msg.get("method").is_none() {
                return msg;
            }
        }
    }
}

// ── Step 3: messages refused while a choice question is open ───────────

#[tokio::test]
async fn sends_are_refused_while_a_choice_question_is_open_and_accepted_after() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    let mut c = Client::connect(&env, "flankr.companion").await;
    let message =
        json!({"pane_id": "p1", "text": "use teal", "kind": "message", "confirmed": true});
    let command = json!({"pane_id": "p1", "command": "continue", "confirmed": true});
    for params in [&message, &command] {
        let r = c.call("agents.send", params.clone()).await;
        assert_eq!(r["error"]["code"], -32001, "{r}");
        assert_eq!(
            r["error"]["message"], "Claude is showing a question; answer it with its options",
            "{r}"
        );
    }
    // Gemini's send path is refused the same way.
    let executor = HerdrToolExecutor::new(env.daemon.client.clone(), env.daemon.agents.clone())
        .with_sessions(Arc::clone(&env.daemon.sessions));
    let refused = executor
        .execute_tool(
            "send_agent_command",
            &json!({"agent": "p1", "text": "continue"}),
        )
        .await
        .unwrap_err();
    assert!(refused.contains("is showing a question"), "{refused}");
    assert!(commands(&env.daemon).await.is_empty());

    // Answered (here, on the Mac): both go.
    append(
        &env.file,
        &result(
            json!({"Which colour should the badge be?": "Teal", "Which rows get icons?": "Files"}),
        ),
    );
    for params in [&message, &command] {
        let r = c.call("agents.send", params.clone()).await;
        assert_eq!(r["result"]["status"], "sent", "{r}");
    }
    assert_eq!(commands(&env.daemon).await, ["use teal", "continue"]);
}

// ── Step 4: choice.answer and the Claude Code driver ───────────────────

#[tokio::test]
async fn an_answer_whose_frames_match_is_sent_and_answered() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    let fixture = result(json!({
        "Which colour should the badge be?": "Amber",
        "Which rows get icons?": "Links, Files"
    }));
    script(
        &env.daemon,
        two_question_frames(),
        Some((env.file.clone(), fixture, TWO_QUESTION_KEYS.len())),
    );
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c.call("choice.answer", two_question_answer()).await;
    assert_eq!(r["result"], json!({"status": "answered"}), "{r}");
    assert_eq!(keys(&env.daemon).await, TWO_QUESTION_KEYS);
    // The question is gone, and a stale answer is refused.
    let r = c.call("choice.answer", two_question_answer()).await;
    assert_eq!(r["error"]["code"], -32040, "{r}");
}

#[tokio::test]
async fn a_result_that_differs_is_a_mismatch() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    let fixture = result(json!({
        "Which colour should the badge be?": "Teal",
        "Which rows get icons?": "Files"
    }));
    script(
        &env.daemon,
        two_question_frames(),
        Some((env.file.clone(), fixture, TWO_QUESTION_KEYS.len())),
    );
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c.call("choice.answer", two_question_answer()).await;
    assert_eq!(
        r["result"],
        json!({"status": "mismatch", "got": ["Colour: Teal", "Icons: Files"]}),
        "{r}"
    );
}

#[tokio::test]
async fn no_result_within_five_seconds_is_no_result() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    script(&env.daemon, two_question_frames(), None);
    let mut c = Client::connect(&env, "flankr.companion").await;
    let started = std::time::Instant::now();
    let r = c.call("choice.answer", two_question_answer()).await;
    assert_eq!(r["result"], json!({"status": "no_result"}), "{r}");
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(keys(&env.daemon).await, TWO_QUESTION_KEYS);
}

#[tokio::test]
async fn question_two_never_appearing_is_a_screen_mismatch() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    script(&env.daemon, vec![colour_frame(None)], None);
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c.call("choice.answer", two_question_answer()).await;
    assert_eq!(r["error"]["code"], -32042, "{r}");
    assert_eq!(r["error"]["message"], "question 2 did not appear", "{r}");
    // Question 1's key went; nothing after it, and never escape.
    assert_eq!(keys(&env.daemon).await, ["2"]);
}

#[tokio::test]
async fn an_other_row_with_text_is_refused_before_any_key() {
    let env = start(claude(true), &(prompt() + &ask(one_question()))).await;
    script(
        &env.daemon,
        vec![colour_frame(Some("typed on the Mac"))],
        None,
    );
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c
        .call(
            "choice.answer",
            json!({"pane_id": "p1", "id": CALL, "answers": [{"other": "warm orange"}]}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32042, "{r}");
    assert_eq!(
        r["error"]["message"], "the Other row of question 1 already has text",
        "{r}"
    );
    assert!(keys(&env.daemon).await.is_empty());
}

#[tokio::test]
async fn an_other_text_is_typed_after_its_digit() {
    let env = start(claude(true), &(prompt() + &ask(one_question()))).await;
    let fixture = result(json!({"Which colour should the badge be?": "warm orange"}));
    script(
        &env.daemon,
        vec![colour_frame(None)],
        Some((env.file.clone(), fixture, 2)),
    );
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c
        .call(
            "choice.answer",
            json!({"pane_id": "p1", "id": CALL, "answers": [{"other": "warm   orange"}]}),
        )
        .await;
    assert_eq!(r["result"], json!({"status": "answered"}), "{r}");
    // The digit focuses the input; the text goes by pane.send_text; Enter.
    assert_eq!(keys(&env.daemon).await, ["3", "enter"]);
}

#[tokio::test]
async fn bad_shapes_permissions_and_harness_clients_are_refused() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c
        .call(
            "choice.answer",
            json!({"pane_id": "p1", "id": CALL, "answers": [{"picked": [0, 1]}, {"picked": [0]}]}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32602, "{r}");
    let r = c
        .call(
            "choice.answer",
            json!({"pane_id": "p1", "id": "toolu_other", "answers": [{"picked": [0]}, {"picked": [0]}]}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32040, "{r}");
    let r = c
        .call(
            "choice.answer",
            json!({"pane_id": "p1", "id": CALL, "answers": [{"picked": [0]}, {"other": "/compact"}]}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    let mut harness = Client::connect(&env, "flankr.harness").await;
    let r = harness.call("choice.answer", two_question_answer()).await;
    assert_eq!(r["error"]["code"], -32601, "{r}");
    assert!(keys(&env.daemon).await.is_empty());

    let env = start(claude(false), &(prompt() + &ask(two_questions()))).await;
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c.call("choice.answer", two_question_answer()).await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    assert!(keys(&env.daemon).await.is_empty());
}

// ── The 2026-10-30 16:42 EDT amendment: choice.cancel ──────────────────

fn cancel_params() -> Value {
    json!({"id": CALL, "pane_id": "p1"})
}

#[tokio::test]
async fn a_cancel_sends_one_escape_after_question_one_shows() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    script(&env.daemon, vec![colour_frame(None)], None);
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c.call("choice.cancel", cancel_params()).await;
    assert_eq!(r["result"], json!({"cancelled": true}), "{r}");
    assert_eq!(keys(&env.daemon).await, ["escape"]);
}

#[tokio::test]
async fn a_cancel_whose_screen_check_fails_sends_nothing() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    // Question 2 is showing (moved on at the Mac): not question 1.
    script(&env.daemon, vec![icons_frame()], None);
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c.call("choice.cancel", cancel_params()).await;
    assert_eq!(r["result"], json!({"cancelled": false}), "{r}");
    assert!(keys(&env.daemon).await.is_empty());
}

#[tokio::test]
async fn a_cancel_is_refused_when_stale_not_permitted_or_from_a_harness() {
    let env = start(claude(true), &(prompt() + &ask(two_questions()))).await;
    script(&env.daemon, vec![colour_frame(None)], None);
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c
        .call(
            "choice.cancel",
            json!({"id": "toolu_other", "pane_id": "p1"}),
        )
        .await;
    assert_eq!(r["error"]["code"], -32040, "{r}");
    let mut harness = Client::connect(&env, "flankr.harness").await;
    let r = harness.call("choice.cancel", cancel_params()).await;
    assert_eq!(r["error"]["code"], -32601, "{r}");
    assert!(keys(&env.daemon).await.is_empty());

    let env = start(claude(false), &(prompt() + &ask(two_questions()))).await;
    script(&env.daemon, vec![colour_frame(None)], None);
    let mut c = Client::connect(&env, "flankr.companion").await;
    let r = c.call("choice.cancel", cancel_params()).await;
    assert_eq!(r["error"]["code"], -32001, "{r}");
    assert!(keys(&env.daemon).await.is_empty());
}
