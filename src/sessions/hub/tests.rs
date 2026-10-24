//! The hub, the sections and the backward reader (plan 20261023_03,
//! sections 10.2 and A8.2 to A8.5), over synthetic session files.

use super::*;
use crate::sessions::test_support::fixture;
use crate::sessions::{EntryKind, Matched};
use libhandlr_proto::common::{CardKind, SectionKind};
use std::io::Write as _;

pub(crate) fn ts(n: u32) -> String {
    format!(
        "2026-01-02T{:02}:{:02}:{:02}.000Z",
        n / 3600 % 24,
        n / 60 % 60,
        n % 60
    )
}

pub(crate) fn user(n: u32, text: &str) -> String {
    serde_json::json!({"type": "user", "timestamp": ts(n), "isSidechain": false,
        "message": {"role": "user", "content": text}, "origin": {"kind": "human"}})
    .to_string()
}

pub(crate) fn reply(n: u32, text: &str) -> String {
    serde_json::json!({"type": "assistant", "timestamp": ts(n), "isSidechain": false,
        "message": {"role": "assistant", "type": "message",
                    "content": [{"type": "text", "text": text}]}})
    .to_string()
}

pub(crate) fn calls(n: u32, calls: &[(&str, &str)]) -> String {
    let content: Vec<serde_json::Value> = calls
        .iter()
        .map(|(id, command)| {
            serde_json::json!({"type": "tool_use", "id": id, "name": "Bash",
                               "input": {"command": command}})
        })
        .collect();
    serde_json::json!({"type": "assistant", "timestamp": ts(n), "isSidechain": false,
        "message": {"role": "assistant", "type": "message", "content": content}})
    .to_string()
}

pub(crate) fn results(n: u32, results: &[(&str, &str, bool)]) -> String {
    let content: Vec<serde_json::Value> = results
        .iter()
        .map(|(id, text, error)| {
            serde_json::json!({"type": "tool_result", "tool_use_id": id,
                               "is_error": error, "content": text})
        })
        .collect();
    serde_json::json!({"type": "user", "timestamp": ts(n), "isSidechain": false,
        "message": {"role": "user", "content": content}})
    .to_string()
}

pub(crate) fn blob() -> String {
    serde_json::json!({"type": "api-request-blob", "hash": "h", "message": "b"}).to_string()
}

fn write(path: &Path, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(path, text).unwrap();
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn append_lines(path: &Path, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    append(path, &text);
}

fn located(path: &Path) -> Located {
    Located {
        path: path.to_path_buf(),
        session_id: path.file_stem().map(|s| s.to_string_lossy().into_owned()),
        matched: Matched::Id,
    }
}

fn tailer(home: &Path, agent: &str, path: &Path) -> Tailer {
    let s = Arc::new(Sessions::with_home(home));
    let journal = Journal::new(s.state.clone());
    Tailer::open(s, agent, &located(path), Linker::default(), &journal).unwrap()
}

/// Every section of the file, as one page.
fn all(t: &Tailer) -> Vec<Section> {
    t.check();
    let snap = t.snapshot();
    page(&t.source(&snap).unwrap(), None, 1_000_000)
        .unwrap()
        .sections
}

type Shape = (u32, u8, SectionKind, usize, Option<(CardKind, bool)>);

fn shape(sections: &[Section]) -> Vec<Shape> {
    sections
        .iter()
        .map(|s| {
            (
                s.serial.line,
                s.serial.slot,
                s.kind,
                s.entries.len(),
                s.card.as_ref().map(|c| (c.card, c.live)),
            )
        })
        .collect()
}

/// Every section of `path` read through `reader`, with no live state.
fn read_with(reader: &dyn SessionReader, path: &Path) -> Vec<Section> {
    let (index, _) = LineIndex::scan(path).unwrap();
    let live = LiveState::default();
    let secrets = HashSet::new();
    let src = Source {
        reader,
        path,
        index: &index,
        file: file_hash(path),
        linker: &Linker::default(),
        approvals: &[],
        live: &live,
        secrets: &secrets,
    };
    page(&src, None, 1_000_000).unwrap().sections
}

/// Apply a batch to a map of sections, as a client does.
fn apply(map: &mut BTreeMap<u64, Section>, batch: &Batch) {
    for s in &batch.sections {
        map.insert(s.serial.to_u64(), s.clone());
    }
    for r in &batch.removed {
        map.remove(&r.to_u64());
    }
}

fn by_serial(sections: Vec<Section>) -> BTreeMap<u64, Section> {
    sections
        .into_iter()
        .map(|s| (s.serial.to_u64(), s))
        .collect()
}

use SectionKind::{Card, Context, Text, Tool, Turn};

#[test]
fn a_claude_transcript_gives_its_sections_and_serials() {
    let home = tempfile::tempdir().unwrap();
    let path = fixture("claude_basic.jsonl");
    let t = tailer(home.path(), "claude", &path);
    let sections = all(&t);
    // Lines 1, 2, 8, 14 and 15 are bookkeeping, 4 is thinking and 11 a
    // timing line: they make no section but are counted.
    assert_eq!(
        shape(&sections),
        [
            (3, 0, Turn, 1, None),
            (5, 0, Text, 1, None),
            (6, 0, Tool, 2, None),
            (9, 0, Tool, 2, None),
            (10, 130, Card, 0, Some((CardKind::Broke, false))),
            (12, 0, Turn, 1, None),
            (13, 0, Text, 1, None),
            // The file is long quiet: the turn has ended on a question.
            (13, 128, Card, 0, Some((CardKind::Question, true))),
        ]
    );
    let name = "claude_basic.jsonl";
    assert!(
        sections
            .iter()
            .all(|s| s.serial.file == Serial::file_hash(name))
    );
    // A tool section holds its call and its result.
    let read = &sections[2].entries;
    assert_eq!(
        (read[0].kind, read[1].kind),
        (EntryKind::ToolUse, EntryKind::ToolResult)
    );
    assert_eq!(read[1].tool.as_deref(), Some("Read"));
    let broke = sections[4].card.as_ref().unwrap();
    assert_eq!(broke.title, "Something broke");
    assert!(broke.text.starts_with("Bash: "), "{}", broke.text);
}

#[test]
fn a_codex_transcript_gives_its_sections_and_serials() {
    let home = tempfile::tempdir().unwrap();
    let path = fixture(
        "home/.codex/sessions/2026/01/03/rollout-2026-01-03T09-00-00-00000000-0000-7000-8000-0000000000c3.jsonl",
    );
    let t = tailer(home.path(), "codex", &path);
    assert_eq!(
        shape(&all(&t)),
        [
            (5, 0, Turn, 1, None),
            (8, 0, Text, 1, None),
            (9, 0, Tool, 2, None),
            (12, 0, Text, 1, None),
            (12, 128, Card, 0, Some((CardKind::Question, true))),
        ]
    );
}

#[test]
fn a_line_with_several_calls_gets_slots_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(
        &path,
        &[
            user(1, "run them"),
            calls(2, &[("a", "ls"), ("b", "pwd"), ("c", "date")]),
            results(
                3,
                &[
                    ("b", "/work", false),
                    ("a", "x", false),
                    ("c", "now", false),
                ],
            ),
            reply(4, "Ran all three."),
        ],
    );
    let t = tailer(dir.path(), "claude", &path);
    let sections = all(&t);
    assert_eq!(
        shape(&sections)[..5],
        [
            (1, 0, Turn, 1, None),
            (2, 0, Tool, 2, None),
            (2, 1, Tool, 2, None),
            (2, 2, Tool, 2, None),
            (4, 0, Text, 1, None),
        ]
    );
    // Each result went to its own call.
    assert_eq!(sections[2].entries[1].summary, "/work");
    assert_eq!(sections[1].entries[1].summary, "x");
}

#[test]
fn the_same_lines_keep_their_serials_read_twice_and_after_appending() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(
        &path,
        &[
            user(1, "go"),
            blob(),
            reply(3, "First."),
            calls(4, &[("a", "ls")]),
        ],
    );
    let t = tailer(dir.path(), "claude", &path);
    let first = all(&t);
    assert_eq!(first, all(&t), "read twice");
    let again = tailer(dir.path(), "claude", &path);
    assert_eq!(first, all(&again), "a new tailer, as after a restart");
    append_lines(
        &path,
        &[
            results(5, &[("a", "x", false)]),
            reply(6, "Second."),
            user(7, "ok"),
        ],
    );
    let after = by_serial(all(&t));
    for s in first.iter().filter(|s| s.kind != Card) {
        let now = &after[&s.serial.to_u64()];
        assert_eq!(now.kind, s.kind);
        // The same entries, and more for the tool that got its result.
        assert_eq!(now.entries[..s.entries.len()], s.entries[..]);
    }
}

/// Claude Code's reader, and one that also reads `note` lines, which the
/// first skips.
struct Wider(crate::sessions::claude::ClaudeCode);

impl SessionReader for Wider {
    fn kind(&self) -> &'static str {
        "wider"
    }
    fn locate(&self, _: Option<&Path>, _: Option<&str>) -> Option<Located> {
        None
    }
    fn parse_line(&self, line: &str) -> Vec<Item> {
        if line.contains("\"type\":\"note\"") {
            return vec![Item {
                ts: ts(0),
                role: crate::sessions::Role::User,
                kind: EntryKind::Text,
                tool: None,
                text: "a note".to_string(),
                call_id: None,
                origin: Some("recap".to_string()),
            }];
        }
        self.0.parse_line(line)
    }
    fn is_bookkeeping(&self, line: &[u8]) -> bool {
        !line.windows(6).any(|w| w == b"\"note\"") && self.0.is_bookkeeping(line)
    }
}

#[test]
fn reading_a_skipped_line_type_does_not_renumber() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let note = serde_json::json!({"type": "note", "text": "a note"}).to_string();
    write(
        &path,
        &[
            user(1, "go"),
            note.clone(),
            calls(3, &[("a", "ls")]),
            note,
            results(5, &[("a", "x", false)]),
            reply(6, "Done."),
        ],
    );
    let narrow = crate::sessions::claude::ClaudeCode::new(dir.path().to_path_buf());
    let before = read_with(&narrow, &path);
    let wider = read_with(
        &Wider(crate::sessions::claude::ClaudeCode::new(
            dir.path().to_path_buf(),
        )),
        &path,
    );
    let wider_map = by_serial(wider.clone());
    for s in &before {
        assert_eq!(wider_map.get(&s.serial.to_u64()), Some(s), "{}", s.serial);
    }
    // The notes are new sections on their own lines.
    let notes: Vec<u32> = wider
        .iter()
        .filter(|s| s.kind == Context)
        .map(|s| s.serial.line)
        .collect();
    assert_eq!(notes, [2, 4]);
}

#[test]
fn long_lines_across_read_chunks_and_past_the_tail_budget_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut lines = Vec::new();
    for i in 0..6 {
        lines.push(user(2 * i, &format!("prompt {i}")));
        // 300 KiB each: every one crosses a 256 KiB boundary.
        lines.push(reply(
            2 * i + 1,
            &format!("reply {i} {}", "x".repeat(300 * 1024)),
        ));
    }
    // Longer than TAIL_MAX (2 MiB).
    lines.push(user(20, "the big one"));
    lines.push(reply(21, &format!("big {}", "y".repeat(3 * 1024 * 1024))));
    write(&path, &lines);
    let t = tailer(dir.path(), "claude", &path);
    let sections: Vec<Section> = all(&t).into_iter().filter(|s| s.kind != Card).collect();
    assert_eq!(sections.len(), 14);
    for (i, s) in sections.iter().enumerate() {
        assert_eq!(s.serial.line as usize, i + 1);
    }
    assert!(sections[13].entries[0].summary.starts_with("big y"));
    assert_eq!(sections[13].entries[0].summary.chars().count(), 500);
}

#[test]
fn pages_walk_back_to_the_start_and_together_equal_one_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut lines = Vec::new();
    for i in 0..400u32 {
        let n = i * 10;
        lines.push(user(n, &format!("prompt {i}")));
        if i % 3 == 0 {
            let id = format!("c{i}");
            lines.push(calls(n + 1, &[(&id, "cargo test")]));
            for _ in 0..(i % 5) {
                lines.push(blob());
            }
            let failed = i % 6 == 0;
            lines.push(results(
                n + 2,
                &[(
                    &id,
                    if failed {
                        "test result: FAILED. 1 failed"
                    } else {
                        "ok"
                    },
                    failed,
                )],
            ));
        }
        lines.push(reply(n + 3, &format!("Reply {i}.")));
        if i % 4 == 0 {
            lines.push(reply(n + 4, "More."));
        }
        if i % 7 == 0 {
            lines.push(reply(n + 5, "Shall I go on?"));
        }
    }
    write(&path, &lines);
    let t = tailer(dir.path(), "claude", &path);
    let whole = all(&t);
    let snap = t.snapshot();
    let src = t.source(&snap).unwrap();
    let mut pages: Vec<Vec<Section>> = Vec::new();
    let mut below = None;
    loop {
        let p = page(&src, below, 37).unwrap();
        assert!(p.sections.len() <= 37);
        if let Some(first) = p.sections.first() {
            below = Some(first.serial);
        }
        let start = p.start;
        pages.push(p.sections);
        if start {
            break;
        }
        assert!(pages.len() < 1000, "no start");
    }
    let joined: Vec<Section> = pages.into_iter().rev().flatten().collect();
    assert_eq!(joined.len(), whole.len());
    assert_eq!(joined, whole);
    // In serial order, each once.
    assert!(
        joined
            .windows(2)
            .all(|w| w[0].serial.to_u64() < w[1].serial.to_u64())
    );
    // Some of each card: resolved Broke and Question, resolved Finished.
    let kinds: HashSet<CardKind> = whole
        .iter()
        .filter_map(|s| s.card.as_ref().map(|c| c.card))
        .collect();
    assert!(kinds.contains(&CardKind::Broke) && kinds.contains(&CardKind::Question));
    assert!(kinds.contains(&CardKind::Finished));
}

#[test]
fn an_old_result_of_a_secret_call_shows_nothing_in_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut lines = vec![
        user(1, "read the env"),
        calls(2, &[("k", "cat /work/example/.env")]),
    ];
    // Far enough that no builder near the result sees its call.
    for _ in 0..(REACH + 40) {
        lines.push(blob());
    }
    lines.push(results(
        400,
        &[("k", "EXAMPLE_TOKEN=not-a-real-token", false)],
    ));
    lines.push(reply(401, "Read it."));
    write(&path, &lines);
    let t = tailer(dir.path(), "claude", &path);
    t.check();
    let snap = t.snapshot();
    let src = t.source(&snap).unwrap();
    // The newest page alone, then all of it: the result is redacted
    // either way, and its text appears nowhere.
    let newest = page(&src, None, 2).unwrap();
    let whole = page(&src, None, 1000).unwrap();
    for sections in [&newest.sections, &whole.sections] {
        let text = serde_json::to_string(sections).unwrap();
        assert!(!text.contains("not-a-real-token"), "{text}");
    }
    let result = newest
        .sections
        .iter()
        .find(|s| s.kind == Tool)
        .expect("the result's own section");
    assert_eq!(result.entries[0].summary, "[redacted]");
    // A page whose oldest result's call is in the next older page.
    let near = dir.path().join("near.jsonl");
    write(
        &near,
        &[
            user(1, "read the key"),
            calls(2, &[("p", "cat /work/example/.private/key")]),
            results(3, &[("p", "not-a-real-secret", false)]),
            user(4, "and again"),
            reply(5, "Done."),
        ],
    );
    let t = tailer(dir.path(), "claude", &near);
    t.check();
    let snap = t.snapshot();
    let src = t.source(&snap).unwrap();
    let mut below = None;
    loop {
        let p = page(&src, below, 1).unwrap();
        let text = serde_json::to_string(&p.sections).unwrap();
        assert!(!text.contains("not-a-real-secret"), "{text}");
        if p.start {
            break;
        }
        below = p.sections.first().map(|s| s.serial);
    }
}

#[test]
fn cards_found_live_are_the_cards_history_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(
        &path,
        &[
            user(1, "go"),
            reply(2, "Working on it."),
            calls(3, &[("c1", "cargo test")]),
        ],
    );
    let t = tailer(dir.path(), "claude", &path);
    let working = AgentStatus::Working;
    let idle = AgentStatus::Idle;
    t.set_live("Claude", &working);
    let mut shown = by_serial(all(&t));
    let mut step = |lines: &[String], status: &AgentStatus| {
        append_lines(&path, lines);
        t.set_live("Claude", status);
        while let Some(batch) = t.check() {
            apply(&mut shown, &batch);
        }
        shown.clone()
    };
    let cards = |m: &BTreeMap<u64, Section>| -> Vec<(u32, u8, CardKind, bool)> {
        m.values()
            .filter_map(|s| {
                s.card
                    .as_ref()
                    .map(|c| (s.serial.line, s.serial.slot, c.card, c.live))
            })
            .collect()
    };
    let m = step(&[results(4, &[("c1", "boom", true)])], &working);
    assert_eq!(cards(&m), [(4, 130, CardKind::Broke, true)]);
    let m = step(
        &[
            calls(5, &[("c2", "cargo test")]),
            results(6, &[("c2", "ok", false)]),
        ],
        &working,
    );
    assert_eq!(cards(&m), [(4, 130, CardKind::Broke, false)]);
    let m = step(&[reply(7, "All good.\n\nShall I push?")], &idle);
    assert_eq!(
        cards(&m),
        [
            (4, 130, CardKind::Broke, false),
            (7, 128, CardKind::Question, true)
        ]
    );
    let m = step(&[user(8, "yes")], &working);
    assert_eq!(cards(&m)[1], (7, 128, CardKind::Question, false));
    let m = step(&[reply(9, "Pushed.")], &idle);
    assert_eq!(cards(&m)[2], (9, 194, CardKind::Finished, true));
    // The agent carries on without the user: that reply did not end the
    // turn, and its card goes.
    let m = step(&[calls(10, &[("c3", "git log")])], &working);
    assert_eq!(cards(&m).len(), 2, "{:?}", cards(&m));
    let m = step(
        &[
            results(11, &[("c3", "ok", false)]),
            reply(12, "Done."),
            user(13, "thanks"),
            reply(14, "You're welcome."),
        ],
        &idle,
    );
    assert_eq!(
        cards(&m),
        [
            (4, 130, CardKind::Broke, false),
            (7, 128, CardKind::Question, false),
            (12, 194, CardKind::Finished, false),
            (14, 194, CardKind::Finished, true),
        ]
    );
    // Blocked: a card after the newest line, gone when it clears.
    t.set_live("Claude", &AgentStatus::Blocked);
    while let Some(batch) = t.check() {
        apply(&mut shown, &batch);
    }
    assert!(cards(&shown).contains(&(14, 129, CardKind::Broke, true)));
    t.set_live("Claude", &idle);
    while let Some(batch) = t.check() {
        apply(&mut shown, &batch);
    }
    // Everything the live run shows, a history read rebuilds.
    assert_eq!(shown, by_serial(all(&t)));
    let fresh = tailer(dir.path(), "claude", &path);
    fresh.set_live("Claude", &idle);
    assert_eq!(cards(&shown), cards(&by_serial(all(&fresh))));
}

#[test]
fn the_journals_approvals_land_at_their_serials_and_only_they_go_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(
        &path,
        &[user(1, "go"), reply(2, "Shall I compact?"), user(3, "yes")],
    );
    let sessions = Arc::new(Sessions::with_home(dir.path()));
    let journal = Journal::new(sessions.state.clone());
    let a = journal
        .asked(
            "s",
            2,
            17,
            "/compact",
            "Approve sending /compact to Claude?",
        )
        .unwrap()
        .unwrap();
    let t = Tailer::open(
        Arc::clone(&sessions),
        "claude",
        &located(&path),
        Linker::default(),
        &journal,
    )
    .unwrap();
    let with = all(&t);
    let approval = with
        .iter()
        .find(|s| {
            s.card
                .as_ref()
                .is_some_and(|c| c.card == CardKind::Approval)
        })
        .expect("the approval card");
    assert_eq!((approval.serial.line, approval.serial.slot), (2, 64));
    let body = approval.card.as_ref().unwrap();
    assert_eq!(body.confirm_id, Some(17));
    assert!(!body.live, "no broker says it is pending");
    t.approval(a, true);
    assert!(all(&t).iter().any(|s| {
        s.card
            .as_ref()
            .is_some_and(|c| c.confirm_id == Some(17) && c.live)
    }));
    t.resolved(17, "approved");
    assert_eq!(all(&t), with);
    // A deleted journal loses the approval card, and nothing else moves.
    std::fs::remove_file(journal.path("s").unwrap()).unwrap();
    let t = Tailer::open(
        sessions,
        "claude",
        &located(&path),
        Linker::default(),
        &journal,
    )
    .unwrap();
    let without: Vec<Section> = with
        .into_iter()
        .filter(|s| s.card.as_ref().is_none_or(|c| c.card != CardKind::Approval))
        .collect();
    assert_eq!(all(&t), without);
}

/// The next batch from `rx`, within `wait`.
async fn next_batch(
    rx: &mut broadcast::Receiver<Arc<Batch>>,
    wait: Duration,
) -> Option<Arc<Batch>> {
    tokio::time::timeout(wait, rx.recv()).await.ok()?.ok()
}

#[tokio::test]
async fn a_growing_file_delivers_each_line_once_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &[user(1, "go")]);
    let hub = SessionHub::new(Arc::new(Sessions::with_home(dir.path())), None);
    let (_t, mut rx, snap) = hub
        .subscribe("claude", None, &located(&path))
        .await
        .unwrap();
    assert_eq!(snap.index.lines(), 1);
    let mut seen: Vec<Serial> = Vec::new();
    for i in 0..5u32 {
        append_lines(&path, &[user(10 + i, &format!("prompt {i}"))]);
        let started = std::time::Instant::now();
        let batch = loop {
            let b = next_batch(&mut rx, Duration::from_secs(2))
                .await
                .expect("a batch");
            if !b.sections.is_empty() {
                break b;
            }
        };
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        for s in batch.sections.iter().filter(|s| s.kind == Turn) {
            seen.push(s.serial);
        }
    }
    let lines: Vec<u32> = seen.iter().map(|s| s.line).collect();
    assert_eq!(lines, [2, 3, 4, 5, 6]);
    // A half-written line waits for its newline.
    let line = user(30, "half");
    let (a, b) = line.split_at(line.len() / 2);
    append(&path, a);
    tokio::time::sleep(Duration::from_millis(700)).await;
    while let Ok(b) = rx.try_recv() {
        assert!(
            b.sections.iter().all(|s| s.serial.line != 7),
            "parsed half a line"
        );
    }
    append(&path, &format!("{b}\n"));
    let batch = loop {
        let b = next_batch(&mut rx, Duration::from_secs(2))
            .await
            .expect("a batch");
        if !b.sections.is_empty() {
            break b;
        }
    };
    assert_eq!(batch.sections[0].serial.line, 7);
    assert_eq!(batch.sections[0].entries[0].summary, "half");
    // Truncated: the subscribers reset.
    write(&path, &[user(1, "again")]);
    let truncated = loop {
        let b = next_batch(&mut rx, Duration::from_secs(2))
            .await
            .expect("a batch");
        if b.truncated {
            break b;
        }
    };
    assert!(truncated.truncated);
}

#[tokio::test]
async fn two_subscribers_share_a_tailer_which_stops_when_both_leave() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &[user(1, "go")]);
    let hub = SessionHub::new(Arc::new(Sessions::with_home(dir.path())), None);
    let (a, rx_a, _) = hub
        .subscribe("claude", None, &located(&path))
        .await
        .unwrap();
    let (b, rx_b, _) = hub
        .subscribe("claude", None, &located(&path))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(hub.tailers().len(), 1);
    drop(rx_a);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(hub.tailer(&path).is_some(), "one is still subscribed");
    drop(rx_b);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(hub.tailer(&path).is_none(), "the tailer left");
    // Subscribing again starts a new one.
    let (c, _rx, _) = hub
        .subscribe("claude", None, &located(&path))
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
}

#[tokio::test]
async fn a_secret_call_and_its_result_in_different_batches_come_out_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &[user(1, "go")]);
    let hub = SessionHub::new(Arc::new(Sessions::with_home(dir.path())), None);
    let (_t, mut rx, _) = hub
        .subscribe("claude", None, &located(&path))
        .await
        .unwrap();
    append_lines(&path, &[calls(2, &[("s", "cat /work/example/.env")])]);
    let first = next_batch(&mut rx, Duration::from_secs(2)).await.unwrap();
    assert_eq!(first.sections[0].kind, Tool);
    append_lines(
        &path,
        &[results(
            3,
            &[("s", "EXAMPLE_TOKEN=not-a-real-token", false)],
        )],
    );
    let second = loop {
        let b = next_batch(&mut rx, Duration::from_secs(2)).await.unwrap();
        if !b.sections.is_empty() {
            break b;
        }
    };
    let tool = &second.sections[0];
    assert_eq!(
        tool.serial, first.sections[0].serial,
        "the same section, resent"
    );
    assert_eq!(tool.entries.len(), 2);
    assert_eq!(tool.entries[1].summary, "[redacted]");
}

#[tokio::test]
async fn a_three_mebibyte_burst_arrives_whole_over_several_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &[user(1, "go")]);
    let hub = SessionHub::new(Arc::new(Sessions::with_home(dir.path())), None);
    let (_t, mut rx, _) = hub
        .subscribe("claude", None, &located(&path))
        .await
        .unwrap();
    let body = "z".repeat(1000);
    let lines: Vec<String> = (0..3000u32)
        .map(|i| user(i, &format!("{i} {body}")))
        .collect();
    append_lines(&path, &lines);
    let mut got: Vec<u32> = Vec::new();
    let mut batches = 0;
    while got.len() < 3000 {
        let b = next_batch(&mut rx, Duration::from_secs(5))
            .await
            .expect("more");
        if b.sections.is_empty() {
            continue;
        }
        batches += 1;
        got.extend(
            b.sections
                .iter()
                .filter(|s| s.kind == Turn)
                .map(|s| s.serial.line),
        );
    }
    assert!(batches >= 3, "{batches} batches for 3 MiB");
    let want: Vec<u32> = (2..3002).collect();
    assert_eq!(got, want);
}

/// The outline's serials by index, `None` for a removed index.
fn outline_serials(o: &Outline) -> Vec<Option<Serial>> {
    (0..o.len()).map(|i| o.slot(i).map(|s| s.serial)).collect()
}

#[test]
fn indexes_run_from_0_without_gaps_and_read_the_same_twice_and_after_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let path = fixture("claude_basic.jsonl");
    let t = tailer(home.path(), "claude", &path);
    let sections = all(&t);
    let (outline, epoch) = t.outline().unwrap();
    assert_eq!(epoch, 0);
    assert_eq!(outline.len(), sections.len() as u64);
    assert_eq!(outline.last(), Some(7));
    // Contiguous, in serial order, every index a section.
    let serials: Vec<Option<Serial>> = sections.iter().map(|s| Some(s.serial)).collect();
    assert_eq!(outline_serials(&outline), serials);
    assert!(outline.removed().is_empty());
    for (i, s) in sections.iter().enumerate() {
        let slot = outline.slot(i as u64).unwrap();
        assert_eq!((slot.index, slot.kind, slot.rev), (i as u64, s.kind, 0));
        assert_eq!((slot.lines, slot.chars), sections::size(s));
        assert!(slot.lines >= 1 && slot.chars >= 1, "{slot:?}");
    }
    // The contents by index carry it.
    let c = t.content(Some(2), Some(4), 50, false).unwrap();
    let got: Vec<(u64, Serial)> = c.sections.iter().map(|s| (s.index, s.serial)).collect();
    assert_eq!(
        got,
        [
            (2, sections[2].serial),
            (3, sections[3].serial),
            (4, sections[4].serial)
        ]
    );
    assert_eq!(c.sections[2].card, sections[4].card);
    // Read again, and by a new tailer as after a restart (in the same
    // live state: the file long quiet): the same.
    assert_eq!(t.outline().unwrap().0, outline);
    let again = tailer(home.path(), "claude", &path);
    again.check();
    assert_eq!(again.outline().unwrap().0, outline);
}

#[test]
fn appended_sections_take_the_next_indexes_as_a_fresh_read_numbers_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &[user(1, "go"), reply(2, "On it.")]);
    let t = tailer(dir.path(), "claude", &path);
    t.set_live("Claude", &AgentStatus::Working);
    t.ensure_outline().unwrap();
    t.check();
    let mut lines = Vec::new();
    for i in 0..20u32 {
        lines.push(calls(10 + i, &[(&format!("c{i}"), "ls")]));
        lines.push(results(11 + i, &[(&format!("c{i}"), "a\nb\nc", false)]));
        lines.push(reply(12 + i, &format!("Step {i}.")));
    }
    let mut numbered: Vec<u64> = Vec::new();
    for chunk in lines.chunks(7) {
        append_lines(&path, chunk);
        while let Some(batch) = t.check() {
            assert!(batch.outlined);
            numbered.extend(batch.slots.iter().filter(|s| s.rev == 0).map(|s| s.index));
            for s in &batch.sections {
                assert_eq!(
                    batch
                        .slots
                        .iter()
                        .find(|x| x.serial == s.serial)
                        .unwrap()
                        .index,
                    s.index
                );
            }
        }
    }
    // Each new one numbered once, next after the last.
    let (outline, _) = t.outline().unwrap();
    assert_eq!(numbered, (2..outline.len()).collect::<Vec<u64>>());
    // A tool section that got its result in a later check was replaced.
    let fresh = tailer(dir.path(), "claude", &path);
    fresh.set_live("Claude", &AgentStatus::Working);
    let (read, _) = fresh.outline().unwrap();
    assert_eq!(outline_serials(&read), outline_serials(&outline));
}

#[test]
fn a_replaced_section_keeps_its_index_and_a_removed_index_is_never_reused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(
        &path,
        &[user(1, "go"), reply(2, "All good.\n\nShall I push?")],
    );
    let t = tailer(dir.path(), "claude", &path);
    t.set_live("Claude", &AgentStatus::Idle);
    t.check();
    let (outline, _) = t.outline().unwrap();
    // The turn, the reply, and the live Question card.
    assert_eq!(outline.len(), 3);
    let question = outline.slot(2).unwrap();
    assert_eq!((question.serial.slot, question.rev), (128, 0));
    let step = |lines: &[String], status: &AgentStatus| -> Vec<Batch> {
        append_lines(&path, lines);
        t.set_live("Claude", status);
        let mut batches = Vec::new();
        while let Some(batch) = t.check() {
            batches.push(batch);
        }
        batches
    };
    // Answered: the card resolves in place, its rev bumped.
    step(&[user(3, "yes")], &AgentStatus::Working);
    let (o, _) = t.outline().unwrap();
    let resolved = o.slot(2).unwrap();
    assert_eq!((resolved.serial, resolved.rev), (question.serial, 1));
    assert_eq!(o.index_of(question.serial), Some(2));
    assert_eq!(o.slot(3).unwrap().serial.line, 3);
    // A reply that would end the turn: a live Finished card at index 5.
    step(&[reply(4, "Pushed.")], &AgentStatus::Idle);
    let (o, _) = t.outline().unwrap();
    assert_eq!(o.len(), 6);
    let finished = o.slot(5).unwrap();
    assert_eq!((finished.serial.line, finished.serial.slot), (4, 194));
    // The agent carries on: the card goes, its index stays removed.
    let batches = step(&[calls(5, &[("c1", "git log")])], &AgentStatus::Working);
    assert!(
        batches.iter().any(|b| b.removed_index == [5]),
        "{batches:?}"
    );
    let (o, _) = t.outline().unwrap();
    assert_eq!(o.removed(), [5]);
    assert!(o.slot(5).is_none());
    let tool = o.slot(6).unwrap();
    assert_eq!((tool.serial.line, tool.kind), (5, Tool));
    // More sections come after it; none takes index 5.
    step(
        &[results(6, &[("c1", "ok", false)]), user(7, "and?")],
        &AgentStatus::Working,
    );
    let (o, _) = t.outline().unwrap();
    assert_eq!(o.removed(), [5]);
    assert_eq!(o.slot(7).unwrap().serial.line, 7);
    assert_eq!(o.len(), 8);
    // The range around it says so.
    let c = t.content(Some(4), Some(7), 50, true).unwrap();
    let idx: Vec<u64> = c.slots.iter().map(|s| s.index).collect();
    assert_eq!((idx, c.removed, c.more), (vec![4, 6, 7], vec![5], false));
}

#[test]
fn a_new_or_rewritten_file_starts_again_at_0() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    write(&path, &[user(1, "one"), user(2, "two"), user(3, "three")]);
    let t = tailer(dir.path(), "claude", &path);
    assert_eq!(t.outline().unwrap().0.len(), 3);
    // Rewritten shorter, as a replaced file: a new epoch, numbered from 0.
    write(&path, &[user(1, "fresh")]);
    assert!(t.check().unwrap().truncated);
    let (o, epoch) = t.outline().unwrap();
    assert_eq!((epoch, o.len()), (1, 1));
    assert_eq!(o.slot(0).unwrap().serial.line, 1);
    // Another file, as after `/clear`.
    let other = dir.path().join("t.jsonl");
    write(&other, &[user(1, "new"), reply(2, "Hi.")]);
    let (o, epoch) = tailer(dir.path(), "claude", &other).outline().unwrap();
    assert_eq!((epoch, o.slot(0).unwrap().index, o.len()), (0, 0, 2));
    assert_eq!(o.slot(0).unwrap().serial.file, file_hash(&other));
}

#[test]
fn ranges_take_the_four_shapes_and_say_what_was_cut_and_removed() {
    let serial = |line| Serial::new(1, line, 0);
    let section = |line| Section {
        serial: serial(line),
        kind: Turn,
        entries: Vec::new(),
        card: None,
        index: 0,
    };
    let all: Vec<Section> = (1..=10).map(section).collect();
    let mut o = Outline::of(&all);
    o.remove(serial(5));
    o.remove(serial(6));
    assert_eq!(o.remove(serial(6)), None, "removed once");
    let r = |f, t, n| {
        let r = o.range(f, t, n);
        (r.indexes, r.removed, r.more)
    };
    // From only: the next `limit` from `from`.
    assert_eq!(r(Some(2), None, 3), (vec![2, 3, 6], vec![4, 5], true));
    assert_eq!(r(Some(7), None, 50), (vec![7, 8, 9], vec![], false));
    assert_eq!(r(Some(40), None, 50), (vec![], vec![], false));
    // To only: the `limit` ending at `to`, oldest first.
    assert_eq!(r(None, Some(6), 2), (vec![3, 6], vec![4, 5], true));
    assert_eq!(r(None, Some(1), 5), (vec![0, 1], vec![], false));
    // Both: the range, cut at `limit` from `from`.
    assert_eq!(r(Some(1), Some(8), 3), (vec![1, 2, 3], vec![], true));
    assert_eq!(r(Some(3), Some(6), 3), (vec![3, 6], vec![4, 5], false));
    assert_eq!(
        r(Some(3), Some(99), 50),
        (vec![3, 6, 7, 8, 9], vec![4, 5], false)
    );
    // Neither: the newest `limit`.
    assert_eq!(r(None, None, 2), (vec![8, 9], vec![], true));
    assert_eq!(r(None, None, 50).0.len(), 8);
    // A removed section that comes back keeps its index.
    let back = o.put(&section(6));
    assert_eq!((back.index, back.rev), (5, 1));
    assert_eq!(o.removed(), [4]);
    assert!(Outline::default().range(None, None, 5).indexes.is_empty());
}
