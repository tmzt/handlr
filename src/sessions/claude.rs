//! Claude Code sessions: `~/.claude/projects/<encoded cwd>/<session id>.jsonl`
//! (plan 20261022_03, decision 3 and the 16:23 EDT amendment).
//!
//! One JSON object per line. `user` and `assistant` lines carry
//! `message.content`, a string or a list of `text`, `thinking`,
//! `tool_use` and `tool_result` blocks. What the user types while the
//! agent works is an `attachment` of type `queued_command`. Lines from
//! subagents (`isSidechain`), injected context (`isMeta`) and compaction
//! summaries are not the conversation and are skipped, as is user text
//! that is only a harness tag such as `<command-name>…</command-name>`.
//!
//! What the terminal shows as context is read as user text entries with
//! an `origin` (the 2026-10-23 22:31 EDT addendum to `CHANGES.md`): task
//! notifications (`task`), other sessions' messages (`peer:<name>`),
//! subagents' hand-backs (`agent:<id>`), recaps (`system` `away_summary`,
//! `recap`) and compactions (`compact_boundary`, `compact`). Every other
//! `type` is bookkeeping, and [`is_bookkeeping`] knows most of it from a
//! line's first bytes.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use std::time::SystemTime;

use libflankr_proto::common::{ChoiceOption, ChoiceQuestion, ChoiceSet};

use super::{
    ChoiceOutcome, EntryKind, Item, LineReader, Located, Matched, Role, SUBAGENT_LIVE,
    SessionReader, Subagents, TEXT_MAX, TOOL_TEXT_MAX, cut, jsonl_newest_first, safe_id,
};

/// How much of a file's start is searched for its `cwd`.
const HEAD_MAX: usize = 64 * 1024;

/// The Claude Code reader, over one `projects` directory.
pub struct ClaudeCode {
    projects: PathBuf,
}

impl ClaudeCode {
    /// A reader of the sessions under `projects` (`~/.claude/projects`).
    pub fn new(projects: PathBuf) -> Self {
        Self { projects }
    }

    /// The project directory for `cwd`: every character other than an
    /// ASCII letter or digit becomes `-`, as Claude Code names them (this
    /// covers the plan's `/` and `.`).
    pub fn project_dir(&self, cwd: &Path) -> PathBuf {
        let encoded: String = cwd
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.projects.join(encoded)
    }

    fn project_dirs(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.projects) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()
    }

    fn by_id(&self, cwd: Option<&Path>, id: &str) -> Option<PathBuf> {
        if !safe_id(id) {
            return None;
        }
        let file = format!("{id}.jsonl");
        if let Some(cwd) = cwd {
            let path = self.project_dir(cwd).join(&file);
            if path.is_file() {
                return Some(path);
            }
        }
        // The cwd changed since the session started.
        self.project_dirs()
            .into_iter()
            .map(|d| d.join(&file))
            .find(|p| p.is_file())
    }

    fn by_cwd(&self, cwd: &Path) -> Option<PathBuf> {
        let dir = self.project_dir(cwd);
        if dir.is_dir() {
            return jsonl_newest_first(&dir).into_iter().next();
        }
        // No directory by that name: the newest file anywhere whose first
        // line with a cwd names this one.
        let want = cwd.to_string_lossy();
        let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
        for dir in self.project_dirs() {
            for path in jsonl_newest_first(&dir) {
                if file_cwd(&path).as_deref() == Some(want.as_ref()) {
                    let modified = path
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::UNIX_EPOCH);
                    found.push((modified, path));
                }
            }
        }
        found.sort_by(|a, b| b.cmp(a));
        found.into_iter().next().map(|(_, p)| p)
    }
}

/// The `cwd` of the first line near the start of the file that has one.
fn file_cwd(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    (&mut file)
        .take(HEAD_MAX as u64)
        .read_to_end(&mut buf)
        .ok()?;
    let text = String::from_utf8_lossy(&buf);
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| v.get("cwd").and_then(|c| c.as_str()).map(str::to_string))
}

impl LineReader for ClaudeCode {
    fn is_bookkeeping(&self, line: &[u8]) -> bool {
        is_bookkeeping(line)
    }

    fn parse_line(&self, line: &str) -> Vec<Item> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let flag = |k: &str| v.get(k).and_then(|f| f.as_bool()) == Some(true);
        if flag("isSidechain") {
            return Vec::new();
        }
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        let role = match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            Some("attachment") => return queued_item(&v, &ts).into_iter().collect(),
            Some("system") => return system_item(&v, &ts).into_iter().collect(),
            _ => return Vec::new(),
        };
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
            return Vec::new();
        };
        // A task notification or a peer's message delivered as a user line
        // (a peer's is also `isMeta`): context, not the user.
        let origin = v.get("origin");
        if role == Role::User
            && let Some(kind) = origin
                .and_then(|o| o.get("kind"))
                .and_then(|k| k.as_str())
                .filter(|k| matches!(*k, "task-notification" | "peer"))
        {
            return context_item(kind, origin, &content_text(content), &ts)
                .into_iter()
                .collect();
        }
        if flag("isMeta") || flag("isCompactSummary") {
            return Vec::new();
        }
        let text_item = |text: &str| -> Option<Item> {
            let text = text.trim();
            if text.is_empty() || (role == Role::User && is_harness_text(text)) {
                return None;
            }
            Some(Item {
                pos: 0,
                ts: ts.clone(),
                role,
                kind: EntryKind::Text,
                tool: None,
                text: cut(text, TEXT_MAX),
                call_id: None,
                origin: None,
                choices: None,
                choice_result: None,
            })
        };
        match content {
            Value::String(s) => text_item(s).into_iter().collect(),
            Value::Array(blocks) => blocks
                .iter()
                .filter_map(|b| match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => b.get("text").and_then(|t| t.as_str()).and_then(text_item),
                    Some("tool_use") => {
                        let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                        let input = b.get("input").map(Value::to_string).unwrap_or_default();
                        let call_id = b.get("id").and_then(|i| i.as_str()).map(str::to_string);
                        // Parsed from the whole input, before the cut.
                        let choices = match (name, &call_id, b.get("input")) {
                            (ASK_TOOL, Some(id), Some(input)) => ask_choices(id, input),
                            _ => None,
                        };
                        Some(Item {
                            pos: 0,
                            ts: ts.clone(),
                            role: Role::Assistant,
                            kind: EntryKind::ToolUse,
                            tool: Some(name.to_string()),
                            text: cut(&format!("{name} {input}"), TOOL_TEXT_MAX),
                            call_id,
                            origin: None,
                            choices,
                            choice_result: None,
                        })
                    }
                    Some("tool_result") => {
                        let body = result_text(b.get("content"));
                        let is_error = b.get("is_error").and_then(|e| e.as_bool()) == Some(true);
                        let choice_result = choice_outcome(v.get("toolUseResult"), &body, is_error);
                        let error = is_error && super::parse_command_exit_code(&body) != Some(0);
                        let text = if error {
                            format!("error: {body}")
                        } else {
                            body
                        };
                        Some(Item {
                            pos: 0,
                            ts: ts.clone(),
                            role: Role::Tool,
                            kind: EntryKind::ToolResult,
                            tool: None,
                            text: cut(&text, TOOL_TEXT_MAX),
                            call_id: b
                                .get("tool_use_id")
                                .and_then(|i| i.as_str())
                                .map(str::to_string),
                            origin: None,
                            choices: None,
                            choice_result,
                        })
                    }
                    // thinking, redacted_thinking, images and anything new.
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl SessionReader for ClaudeCode {
    fn kind(&self) -> &'static str {
        "claude_code"
    }

    fn locate(&self, cwd: Option<&Path>, session_id: Option<&str>) -> Option<Located> {
        if let Some(id) = session_id {
            // herdr named the session: that file or nothing, never another
            // agent's newest file.
            return self.by_id(cwd, id).map(|path| Located {
                path,
                session_id: Some(id.to_string()),
                matched: Matched::Id,
            });
        }
        let path = self.by_cwd(cwd?)?;
        let session_id = path.file_stem().map(|s| s.to_string_lossy().into_owned());
        Some(Located {
            path,
            session_id,
            matched: Matched::Cwd,
        })
    }

    fn read_back(
        &self,
        at: &Located,
        enough: &dyn Fn(&[Item]) -> bool,
    ) -> Result<super::Read, String> {
        super::read_tail(self, &at.path, enough)
    }

    /// From `<session dir>/subagents/agent-<id>.jsonl` and its
    /// `agent-<id>.meta.json` (the subagents plan, sources): running when
    /// the meta does not say `finished` and the transcript was written
    /// within [`SUBAGENT_LIVE`]. A meta is read only for a transcript that
    /// recent; a missing or unreadable one is taken as not finished. No
    /// directory is no subagents.
    fn subagents(&self, at: &Located, now: SystemTime) -> Option<Subagents> {
        let dir = subagents_dir(&at.path);
        let mut found = Subagents::default();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Some(found);
        };
        for path in entries.flatten().map(|e| e.path()) {
            let Some(stem) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".jsonl"))
                .filter(|s| s.starts_with("agent-"))
            else {
                continue;
            };
            let Some(modified) = path.metadata().ok().and_then(|m| m.modified().ok()) else {
                continue;
            };
            found.newest = found.newest.max(Some(modified));
            let recent = now.duration_since(modified).unwrap_or_default() <= SUBAGENT_LIVE;
            if recent && !meta_finished(&dir.join(format!("{stem}.meta.json"))) {
                found.running += 1;
            }
        }
        Some(found)
    }
}

/// The subagents' directory of the session file at `path`:
/// `<project>/<session id>/subagents`.
fn subagents_dir(path: &Path) -> PathBuf {
    path.with_extension("").join("subagents")
}

/// The most of a subagent's meta file that is read.
const META_MAX: u64 = 64 * 1024;

/// Whether the subagent meta file at `path` says `"finished": true`.
fn meta_finished(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = Vec::new();
    if file.take(META_MAX).read_to_end(&mut buf).is_err() {
        return false;
    }
    serde_json::from_slice::<Value>(&buf)
        .ok()
        .and_then(|v| v.get("finished").and_then(Value::as_bool))
        == Some(true)
}

/// Claude Code's question tool (plan 20261029_01).
pub(crate) const ASK_TOOL: &str = "AskUserQuestion";
/// The `source` of a Claude Code choice question.
pub(crate) const ASK_SOURCE: &str = "claude_code/AskUserQuestion";
/// The most of a choice result's text kept for finding its answers.
const CHOICE_CONTENT_MAX: usize = 64 * 1024;

/// The choice question an `AskUserQuestion` call with id `id` asks, from
/// its whole `input` (plan 20261029_01, decision 4): each question's
/// `header`, `question`, `multiSelect` and `options[].{label, description,
/// preview}`; `other` unless the question is single-select with a
/// preview (finding 4). `None` for the extended form (a `title`, or a
/// question whose `kind` is not `"choice"`), for a malformed input, and
/// for one outside decision 1's bounds: it stays a plain call, answered on
/// the Mac.
pub(crate) fn ask_choices(id: &str, input: &Value) -> Option<ChoiceSet> {
    if input.get("title").is_some() {
        return None;
    }
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let mut questions = Vec::new();
    for q in input.get("questions")?.as_array()? {
        if q.get("kind").is_some_and(|k| k.as_str() != Some("choice")) {
            return None;
        }
        let multi = q.get("multiSelect").and_then(Value::as_bool) == Some(true);
        let mut options = Vec::new();
        for o in q.get("options")?.as_array()? {
            options.push(ChoiceOption::new(
                str_of(o, "label")?,
                str_of(o, "description").unwrap_or_default(),
                str_of(o, "preview"),
            ));
        }
        let preview = options.iter().any(|o| o.preview.is_some());
        questions.push(ChoiceQuestion::new(
            str_of(q, "header").unwrap_or_default(),
            str_of(q, "question")?,
            multi,
            multi || !preview,
            options,
        ));
    }
    let set = ChoiceSet::new(id, questions, ASK_SOURCE);
    set.within_bounds().then_some(set)
}

/// What a `tool_result` says about a choice question, if it may be the
/// answer to one (plan 20261029_01, decision 4): `toolUseResult`'s
/// `answers` by question when it has `questions` and `answers`; else the
/// content, when it reads as answered (`Your questions have been
/// answered:` or `The user answered:`); `rejected` when the result is an
/// error saying the tool use was rejected. Whether it is the answer to a
/// choice question is known by its `call_id`.
pub(crate) fn choice_outcome(
    structured: Option<&Value>,
    body: &str,
    is_error: bool,
) -> Option<ChoiceOutcome> {
    if is_error {
        return body
            .trim_start()
            .starts_with("The user doesn't want to proceed with this tool use")
            .then(|| ChoiceOutcome {
                rejected: true,
                ..ChoiceOutcome::default()
            });
    }
    if let Some(r) = structured.filter(|r| r.get("questions").is_some())
        && let Some(answers) = r.get("answers").and_then(Value::as_object)
    {
        let order: Vec<&str> = r
            .get("questions")
            .and_then(Value::as_array)
            .map(|qs| {
                qs.iter()
                    .filter_map(|q| q.get("question").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        let mut pairs: Vec<(String, String)> = order
            .iter()
            .filter_map(|q| {
                let a = answers.get(*q)?.as_str()?;
                Some((q.to_string(), a.to_string()))
            })
            .collect();
        for (q, a) in answers {
            if !order.contains(&q.as_str())
                && let Some(a) = a.as_str()
            {
                pairs.push((q.clone(), a.to_string()));
            }
        }
        return Some(ChoiceOutcome {
            answers: pairs,
            ..ChoiceOutcome::default()
        });
    }
    let t = body.trim_start();
    (t.starts_with("Your questions have been answered:") || t.starts_with("The user answered:"))
        .then(|| ChoiceOutcome {
            content: Some(cut(t, CHOICE_CONTENT_MAX)),
            ..ChoiceOutcome::default()
        })
}

/// The most of a session file's end searched for a call's line.
const VERSION_SEARCH_MAX: u64 = 4 * 1024 * 1024;

/// The Claude Code `version` on the line of tool call `id` in the session
/// file at `path`, searched for in the file's last 4 MiB (plan 20261029_01,
/// decision 5: logged with each answer).
pub(crate) fn call_version(path: &Path, id: &str) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(VERSION_SEARCH_MAX)))
        .ok()?;
    let mut buf = Vec::new();
    file.take(VERSION_SEARCH_MAX).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let quoted = format!("\"{id}\"");
    text.lines()
        .rev()
        .filter(|l| l.contains(&quoted) && l.contains("\"tool_use\""))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| v.get("version").and_then(Value::as_str).map(str::to_string))
}

/// The longest text of an entry that is context (a notification, a
/// peer's message, a recap), in characters.
pub(crate) const NOTICE_MAX: usize = 300;

/// How much of a line's start [`is_bookkeeping`] looks at.
const BOOKKEEPING_HEAD: usize = 256;

/// The lines no entry comes from, known from their first bytes: a line
/// that starts with a `type` other than the conversation's
/// (`api-request-blob`, `file-history-snapshot`, `mode`, `ai-title` and
/// the like are written with `type` first; the conversation's lines start
/// with `parentUuid`), and an attachment other than a queued command
/// (`"attachment":{"type":"…"` comes before the line's own `type`).
pub(crate) fn is_bookkeeping(line: &[u8]) -> bool {
    const TYPE_FIRST: &[u8] = b"{\"type\":\"";
    const ATTACHMENT: &[u8] = b"\"attachment\":{\"type\":\"";
    let head = &line[..line.len().min(BOOKKEEPING_HEAD)];
    if let Some(rest) = head.strip_prefix(TYPE_FIRST) {
        return !["user\"", "assistant\"", "system\"", "attachment\""]
            .iter()
            .any(|t| rest.starts_with(t.as_bytes()));
    }
    head.windows(ATTACHMENT.len())
        .position(|w| w == ATTACHMENT)
        .is_some_and(|at| !head[at + ATTACHMENT.len()..].starts_with(b"queued_command\""))
}

/// A message `content`'s text: a string, or its text blocks joined.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// A command queued while the agent worked (an `attachment` of type
/// `queued_command`): what the user typed, a task notification, or a
/// peer's message. Other attachments give nothing.
fn queued_item(v: &Value, line_ts: &str) -> Option<Item> {
    let a = v.get("attachment")?;
    if a.get("type").and_then(|t| t.as_str()) != Some("queued_command") {
        return None;
    }
    let ts = a
        .get("timestamp")
        .and_then(|t| t.as_str())
        .unwrap_or(line_ts);
    let text = a.get("prompt").map(content_text).unwrap_or_default();
    let origin = a.get("origin");
    let kind = origin
        .and_then(|o| o.get("kind"))
        .and_then(|k| k.as_str())
        .unwrap_or("human");
    if a.get("commandMode").and_then(|m| m.as_str()) == Some("task-notification") {
        return context_item("task-notification", origin, &text, ts);
    }
    if kind != "human" {
        return context_item(kind, origin, &text, ts);
    }
    let text = text.trim();
    if text.is_empty() || is_harness_text(text) {
        return None;
    }
    Some(Item {
        pos: 0,
        ts: ts.to_string(),
        role: Role::User,
        kind: EntryKind::Text,
        tool: None,
        text: cut(text, TEXT_MAX),
        call_id: None,
        origin: None,
        choices: None,
        choice_result: None,
    })
}

/// A `system` line the terminal shows: a recap (`away_summary`) or a
/// compaction (`compact_boundary`, and the `informational` notice about
/// one). Others (`turn_duration` and the like) give nothing.
fn system_item(v: &Value, ts: &str) -> Option<Item> {
    let content = v
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .trim();
    let origin = match v.get("subtype").and_then(|s| s.as_str())? {
        "away_summary" if !content.is_empty() => "recap",
        "compact_boundary" => "compact",
        "informational" if content.to_lowercase().contains("compact") => "compact",
        _ => return None,
    };
    let text = match (origin, content) {
        ("compact", "") => "Conversation compacted",
        (_, text) => text,
    };
    notice(ts, origin.to_string(), text)
}

/// The entry for context the harness gave the agent, by its origin's
/// `kind`: a task notification (`task-notification`), or a subagent's
/// hand-back or another session's message (`peer`). `None` for any other
/// kind.
fn context_item(kind: &str, origin: Option<&Value>, text: &str, ts: &str) -> Option<Item> {
    match kind {
        "task-notification" => notice(ts, "task".to_string(), &task_summary(text)),
        "peer" => {
            let field = |k: &str| {
                origin
                    .and_then(|o| o.get(k))
                    .and_then(|f| f.as_str())
                    .filter(|f| !f.is_empty())
            };
            if let Some((attrs, body)) = element(text, "agent-message") {
                let id = attr(attrs, "from").or_else(|| field("from")).unwrap_or("?");
                let body = match body.split_once("The report follows:") {
                    Some((_, report)) => report,
                    None => body.trim_start().trim_start_matches("[Subagent hand-back]"),
                };
                return notice(ts, format!("agent:{id}"), body);
            }
            let (from, body) = match element(text, "cross-session-message") {
                Some((attrs, body)) => (
                    attr(attrs, "from-name")
                        .or_else(|| attr(attrs, "from"))
                        .or_else(|| field("name"))
                        .or_else(|| field("from")),
                    body,
                ),
                None => (field("name").or_else(|| field("from")), text),
            };
            notice(ts, format!("peer:{}", from.unwrap_or("?")), body)
        }
        _ => None,
    }
}

/// A user text entry that is context from `origin`: `text` from its first
/// line that is not blank, cut to [`NOTICE_MAX`]. `None` when it is blank.
fn notice(ts: &str, origin: String, text: &str) -> Option<Item> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(Item {
        pos: 0,
        ts: ts.to_string(),
        role: Role::User,
        kind: EntryKind::Text,
        tool: None,
        text: cut(text, NOTICE_MAX),
        call_id: None,
        origin: Some(origin),
        choices: None,
        choice_result: None,
    })
}

/// A task notification's `<summary>`, else its first line that is not a
/// tag.
fn task_summary(text: &str) -> String {
    if let Some((_, summary)) = element(text, "summary")
        && !summary.trim().is_empty()
    {
        return summary.trim().to_string();
    }
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('<'))
        .unwrap_or_default()
        .to_string()
}

/// The first `<name …>…</name>` element in `text`: its attributes and its
/// body. An element left open runs to the end of `text`.
fn element<'a>(text: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let open = format!("<{name}");
    let mut from = 0;
    let start = loop {
        let at = from + text[from..].find(&open)?;
        from = at + open.len();
        if text[from..].starts_with(['>', ' ', '\n', '\t']) {
            break from;
        }
    };
    let tag_end = start + text[start..].find('>')?;
    let attrs = &text[start..tag_end];
    let body = &text[tag_end + 1..];
    let close = format!("</{name}>");
    let body = body.find(&close).map_or(body, |end| &body[..end]);
    Some((attrs, body))
}

/// An attribute's value in `attrs` (`name="value"`), when not empty.
fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let at = attrs.find(&key)? + key.len();
    let end = attrs[at..].find('"')?;
    Some(&attrs[at..at + end]).filter(|v| !v.is_empty())
}

/// A tool result's text: a string, or its text blocks joined, with other
/// blocks named.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|b| match b.get("type").and_then(|t| t.as_str()) {
                Some("text") => b
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .to_string(),
                Some(other) => format!("[{other}]"),
                None => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Whether user text is only harness tags (`<command-name>…`,
/// `<task-notification>…`, `<environment_context>…`), not something the
/// user said.
pub(crate) fn is_harness_text(text: &str) -> bool {
    let t = text.trim();
    let Some(rest) = t.strip_prefix('<') else {
        return false;
    };
    let name_len = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(rest.len());
    name_len > 0 && rest[name_len..].starts_with(['>', ' ']) && t.ends_with('>')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::test_support::fixture;
    use crate::sessions::{Sessions, read_tail};

    fn reader() -> ClaudeCode {
        ClaudeCode::new(fixture("home/.claude/projects"))
    }

    fn texts(items: &[Item]) -> Vec<(Role, EntryKind, String)> {
        items
            .iter()
            .map(|i| (i.role, i.kind, i.text.clone()))
            .collect()
    }

    #[test]
    fn the_project_directory_encodes_the_cwd() {
        let r = ClaudeCode::new(PathBuf::from("/p"));
        assert_eq!(
            r.project_dir(Path::new("/work/example.app/my_repo")),
            PathBuf::from("/p/-work-example-app-my-repo")
        );
    }

    #[test]
    fn string_and_block_content_tool_use_and_results() {
        let path = fixture("claude_basic.jsonl");
        let items = reader().tail(&path, 50).unwrap();
        let got = texts(&items);
        assert_eq!(
            got,
            vec![
                (
                    Role::User,
                    EntryKind::Text,
                    "Please add a greeting to the example app.".into()
                ),
                (
                    Role::Assistant,
                    EntryKind::Text,
                    "I will look at the source first.".into()
                ),
                (
                    Role::Assistant,
                    EntryKind::ToolUse,
                    r#"Read {"file_path":"/work/example/src/main.rs"}"#.into()
                ),
                (Role::Tool, EntryKind::ToolResult, "fn main() {}".into()),
                (
                    Role::Assistant,
                    EntryKind::ToolUse,
                    r#"Bash {"command":"cargo test"}"#.into()
                ),
                (
                    Role::Tool,
                    EntryKind::ToolResult,
                    "error: test failed: 1 of 3".into()
                ),
                (
                    Role::User,
                    EntryKind::Text,
                    "Try again with the fix.".into()
                ),
                (
                    Role::Assistant,
                    EntryKind::Text,
                    "Fixed. The tests pass now.\n\nShall I commit the change?".into()
                ),
            ]
        );
        // Results carry their call's tool name; timestamps come through; line numbers match the file.
        assert_eq!(items[3].tool.as_deref(), Some("Read"));
        assert_eq!(items[5].tool.as_deref(), Some("Bash"));
        assert_eq!(items[0].ts, "2026-01-02T10:00:00.000Z");
        assert_eq!(
            items.iter().map(|i| i.pos).collect::<Vec<_>>(),
            vec![3, 5, 6, 7, 9, 10, 12, 13]
        );
        let e = items[5].entry();
        assert!(e.summary.starts_with("error: "));
    }

    #[test]
    fn bookkeeping_meta_sidechain_and_malformed_lines_are_skipped() {
        let path = fixture("claude_noise.jsonl");
        let items = reader().tail(&path, 50).unwrap();
        assert_eq!(
            texts(&items),
            vec![
                (Role::User, EntryKind::Text, "What changed today?".into()),
                (
                    Role::Assistant,
                    EntryKind::Text,
                    "Two files changed.".into()
                ),
            ]
        );
    }

    fn with_origin(items: &[Item]) -> Vec<(Option<&str>, &str, &str)> {
        items
            .iter()
            .map(|i| (i.origin.as_deref(), i.ts.as_str(), i.text.as_str()))
            .collect()
    }

    #[test]
    fn queued_messages_notifications_peers_recaps_and_compactions_are_read() {
        let path = fixture("claude_context.jsonl");
        let items = reader().tail(&path, 50).unwrap();
        assert!(items.iter().all(|i| i.kind == EntryKind::Text));
        assert_eq!(
            with_origin(&items),
            vec![
                (None, "2026-01-05T09:00:00.000Z", "Start the example build."),
                (
                    None,
                    "2026-01-05T09:00:01.000Z",
                    "Starting the build in the background."
                ),
                // Typed while the agent worked, at the attachment's time.
                (
                    None,
                    "2026-01-05T09:00:02.000Z",
                    "Also update the changelog, please."
                ),
                (
                    Some("task"),
                    "2026-01-05T09:00:04.000Z",
                    "Background command \"Build the example\" completed (exit code 0)"
                ),
                (
                    Some("task"),
                    "2026-01-05T09:00:05.000Z",
                    "Agent \"Check the example docs\" finished"
                ),
                (
                    Some("peer:docs-writer"),
                    "2026-01-05T09:00:06.000Z",
                    "The example guide is rebased on main.\n\nNothing else changed."
                ),
                (
                    Some("agent:a0123456789abcdef"),
                    "2026-01-05T09:00:07.000Z",
                    "Added the greeting and its test.\n  Both pass."
                ),
                (
                    Some("recap"),
                    "2026-01-05T09:00:09.000Z",
                    "You asked for the example build; it passed and the changelog is next."
                ),
                (
                    Some("compact"),
                    "2026-01-05T09:00:10.000Z",
                    "Conversation compacted"
                ),
                (None, "2026-01-05T09:00:14.000Z", "Done. Shall I push?"),
            ]
        );
        let roles: Vec<Role> = items.iter().map(|i| i.role).collect();
        assert_eq!(roles[1], Role::Assistant);
        assert_eq!(roles[9], Role::Assistant);
        assert!(
            roles[2..9].iter().all(|r| *r == Role::User),
            "context entries are user entries"
        );
        // The origin reaches the client.
        assert_eq!(items[5].entry().origin.as_deref(), Some("peer:docs-writer"));
        assert_eq!(items[2].entry().origin, None);
    }

    #[test]
    fn context_text_is_cut_and_redacted() {
        let key = "AIzaSyD0nTuSeTh1sFakeKeyAbCdEfGhIjKlMn";
        let prompt = format!(
            "<cross-session-message from=\"x\">\n\nUse {key} now. {}\n</cross-session-message>",
            "w".repeat(400)
        );
        let line = serde_json::json!({
            "parentUuid": "u",
            "isSidechain": false,
            "attachment": {
                "type": "queued_command",
                "prompt": prompt,
                "commandMode": "prompt",
                "origin": {"kind": "peer"},
                "timestamp": "t"
            },
            "type": "attachment"
        });
        let items = reader().parse_line(&line.to_string());
        assert_eq!(items.len(), 1);
        // No from-name: the from id names the peer.
        assert_eq!(items[0].origin.as_deref(), Some("peer:x"));
        assert!(items[0].text.starts_with("Use "));
        assert_eq!(items[0].text.chars().count(), NOTICE_MAX);
        let entry = items[0].entry();
        assert!(!entry.summary.contains(key), "{}", entry.summary);
        // A notification with no summary falls back to its first plain line.
        assert_eq!(
            task_summary("<task-notification>\n<status>x</status>\nIt ended.\n"),
            "It ended."
        );
        assert_eq!(task_summary("<task-notification></task-notification>"), "");
    }

    #[test]
    fn bookkeeping_is_known_from_the_first_bytes() {
        for line in [
            r#"{"type":"api-request-blob","sessionId":"s","hash":"h"}"#,
            r#"{"type":"file-history-snapshot","messageId":"m"}"#,
            r#"{"type":"mode","mode":"normal"}"#,
            r#"{"type":"queue-operation","operation":"enqueue"}"#,
            r#"{"type":"ai-title","aiTitle":"x"}"#,
            r#"{"parentUuid":"u","isSidechain":false,"attachment":{"type":"total_tokens_reminder"},"type":"attachment"}"#,
        ] {
            assert!(is_bookkeeping(line.as_bytes()), "{line}");
        }
        for line in [
            r#"{"parentUuid":"u","isSidechain":false,"promptId":"p","type":"user"}"#,
            r#"{"parentUuid":"u","isSidechain":false,"attachment":{"type":"queued_command","prompt":"x"},"type":"attachment"}"#,
            r#"{"parentUuid":"u","isSidechain":false,"type":"system","subtype":"away_summary"}"#,
            r#"{"type":"user","message":{"content":"older files start with type"}}"#,
            r#"{"type":"assistant","message":{"content":"x"}}"#,
            "not json",
        ] {
            assert!(!is_bookkeeping(line.as_bytes()), "{line}");
        }
    }

    #[test]
    fn padding_with_large_bookkeeping_lines_changes_nothing() {
        // The context fixture with 1.5 MiB of blob and attachment lines after
        // each of its lines (27 MiB): far past the 2 MiB budget if they counted.
        let plain = std::fs::read_to_string(fixture("claude_context.jsonl")).unwrap();
        let blob = format!(
            "{{\"type\":\"api-request-blob\",\"sessionId\":\"s\",\"hash\":\"h\",\"message\":{{\"content\":\"{}\"}}}}\n",
            "b".repeat(1024 * 1024)
        );
        let reminder = format!(
            "{{\"parentUuid\":\"u\",\"isSidechain\":false,\"attachment\":{{\"type\":\"deferred_tools_record\",\"text\":\"{}\"}},\"type\":\"attachment\"}}\n",
            "r".repeat(512 * 1024)
        );
        let mut padded = String::new();
        for line in plain.lines() {
            padded.push_str(line);
            padded.push('\n');
            padded.push_str(&blob);
            padded.push_str(&reminder);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("padded.jsonl");
        std::fs::write(&path, &padded).unwrap();
        assert!(padded.len() > 16 * 1024 * 1024);
        let r = reader();
        let want = r.tail(&fixture("claude_context.jsonl"), 200).unwrap();
        assert_eq!(want.len(), 10);
        let got = r.tail(&path, 200).unwrap();
        let strip_pos = |items: Vec<Item>| {
            items
                .into_iter()
                .map(|mut i| {
                    i.pos = 0;
                    i
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(strip_pos(got.clone()), strip_pos(want.clone()));
        assert_eq!(got[0].pos, 1);
        assert!(got[1].pos > want[1].pos);
    }

    #[test]
    fn a_result_of_reading_a_secret_file_is_redacted_whole() {
        let path = fixture("claude_secret.jsonl");
        let items = reader().tail(&path, 50).unwrap();
        let result = items
            .iter()
            .find(|i| i.kind == EntryKind::ToolResult)
            .unwrap();
        assert_eq!(result.text, crate::sessions::redact::REDACTED);
        for item in &items {
            assert!(!item.entry().summary.contains("not-a-real"), "{item:?}");
        }
    }

    #[test]
    fn located_by_herdr_id_then_by_cwd() {
        let r = reader();
        let cwd = Path::new("/work/example");
        // By id, in the cwd's directory.
        let l = r
            .locate(Some(cwd), Some("00000000-0000-4000-8000-0000000000a1"))
            .unwrap();
        assert_eq!(l.matched, Matched::Id);
        assert!(
            l.path
                .ends_with("-work-example/00000000-0000-4000-8000-0000000000a1.jsonl")
        );
        // By id, after the cwd changed: found in another directory.
        let l = r
            .locate(
                Some(Path::new("/work/elsewhere")),
                Some("00000000-0000-4000-8000-0000000000b2"),
            )
            .unwrap();
        assert!(
            l.path
                .ends_with("-work-moved/00000000-0000-4000-8000-0000000000b2.jsonl")
        );
        // An id with no file is no session, not the cwd's newest.
        assert_eq!(
            r.locate(Some(cwd), Some("00000000-0000-4000-8000-0000000000ff")),
            None
        );
        assert_eq!(r.locate(Some(cwd), Some("../../etc/passwd")), None);
        // By cwd: the newest file in the cwd's directory.
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("-w-x");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("old.jsonl"), "{}\n").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(project.join("old.jsonl"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        std::fs::write(project.join("new.jsonl"), "{}\n").unwrap();
        let r2 = ClaudeCode::new(dir.path().to_path_buf());
        let l = r2.locate(Some(Path::new("/w/x")), None).unwrap();
        assert_eq!(l.matched, Matched::Cwd);
        assert_eq!(l.session_id.as_deref(), Some("new"));
        // No directory by that name: a file whose first cwd names it.
        let l = r
            .locate(Some(Path::new("/work/renamed.dir")), None)
            .unwrap();
        assert!(
            l.path
                .ends_with("-work-moved/00000000-0000-4000-8000-0000000000b2.jsonl")
        );
        assert_eq!(r.locate(Some(Path::new("/nowhere")), None), None);
    }

    #[test]
    fn few_entries_in_the_last_window_reads_further_back() {
        // Generated: 10 replies, then 600 KiB of bookkeeping lines.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let mut body = String::new();
        for i in 0..10 {
            body.push_str(&format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"t{i}\",\"message\":{{\"content\":\"reply {i}\"}}}}\n"
            ));
        }
        let noise = format!(
            "{{\"type\":\"progress\",\"data\":\"{}\"}}\n",
            "n".repeat(1000)
        );
        while body.len() < 600 * 1024 {
            body.push_str(&noise);
        }
        std::fs::write(&path, body).unwrap();
        let r = reader();
        let tail = read_tail(&r, &path, |items| items.len() >= 5).unwrap();
        assert_eq!(tail.items.len(), 10);
        assert!(tail.bytes_read > 512 * 1024);
        let last = r.tail(&path, 5).unwrap();
        assert_eq!(last[0].text, "reply 5");
        assert_eq!(last[4].text, "reply 9");
    }

    #[test]
    fn sessions_choose_the_reader_by_agent() {
        let s = Sessions::with_home(&fixture("home"));
        assert_eq!(s.reader_for("claude").unwrap().kind(), "claude_code");
        assert_eq!(s.reader_for("codex").unwrap().kind(), "codex");
        assert_eq!(s.reader_for("agy").unwrap().kind(), "agy");
        assert!(s.reader_for("shell").is_none());
    }

    #[test]
    fn harness_tags_are_not_user_text() {
        assert!(is_harness_text("<command-name>/compact</command-name>"));
        assert!(is_harness_text(
            "<task-notification>\n<status>done</status>\n</task-notification>"
        ));
        assert!(is_harness_text(
            "<environment_context>x</environment_context>"
        ));
        assert!(!is_harness_text("[Request interrupted by user]"));
        assert!(!is_harness_text("<3 thanks"));
        assert!(!is_harness_text("use <T> here"));
    }

    /// Set the modification time of the file at `path` to `at`.
    fn touch(path: &Path, at: SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    #[test]
    fn subagents_running_are_unfinished_and_recently_written() {
        // Plan 20261028_03, 3.12 item 47: the fixture session with a
        // running, a finished, a quiet and a meta-less subagent.
        let dir = tempfile::tempdir().unwrap();
        let session = dir
            .path()
            .join("00000000-0000-4000-8000-0000000000a1.jsonl");
        std::fs::copy(fixture("claude_basic.jsonl"), &session).unwrap();
        let now = SystemTime::now();
        let secs = std::time::Duration::from_secs;
        touch(&session, now - secs(600));
        let at = Located {
            path: session.clone(),
            session_id: None,
            matched: Matched::Id,
        };
        let r = reader();
        // No subagents directory: none running, no time of theirs.
        assert_eq!(r.subagents(&at, now), Some(Subagents::default()));
        let activity = crate::sessions::activity(&r, &at, now);
        assert_eq!(activity.subagents_running, Some(0));
        assert_eq!(activity.idle_ms, Some(600_000), "{activity:?}");

        let subs = dir
            .path()
            .join("00000000-0000-4000-8000-0000000000a1")
            .join("subagents");
        std::fs::create_dir_all(&subs).unwrap();
        for (id, age, finished) in [
            ("a1", secs(5), Some(false)),
            ("b2", secs(2), Some(true)),
            ("c3", secs(61), Some(false)),
            ("d4", secs(30), None),
        ] {
            let transcript = subs.join(format!("agent-{id}.jsonl"));
            std::fs::write(&transcript, "{}\n").unwrap();
            touch(&transcript, now - age);
            if let Some(finished) = finished {
                std::fs::write(
                    subs.join(format!("agent-{id}.meta.json")),
                    serde_json::json!({"agentType": "Explore", "finished": finished}).to_string(),
                )
                .unwrap();
            }
        }
        // Not a transcript: ignored.
        std::fs::write(subs.join("notes.txt"), "x").unwrap();
        let found = r.subagents(&at, now).unwrap();
        // a1 (5 s) and d4 (30 s, no meta); b2 is finished, c3 too quiet.
        assert_eq!(found.running, 2, "{found:?}");
        assert_eq!(found.newest, Some(now - secs(2)));
        let activity = crate::sessions::activity(&r, &at, now);
        assert_eq!(activity.subagents_running, Some(2));
        // The idle age is from the newest file, a subagent's.
        assert_eq!(activity.idle_ms, Some(2_000), "{activity:?}");
        // At the edge: a1, written exactly SUBAGENT_LIVE ago, still counts.
        assert_eq!(r.subagents(&at, now + secs(55)).unwrap().running, 1);
        assert_eq!(r.subagents(&at, now + secs(56)).unwrap().running, 0);
        // A write after `now` is no time ago.
        let earlier = now - secs(10);
        assert_eq!(crate::sessions::activity(&r, &at, earlier).idle_ms, Some(0));
    }

    #[test]
    fn other_readers_read_no_subagents() {
        let s = Sessions::with_home(&fixture("home"));
        let codex = s.reader_for("codex").unwrap();
        let at = Located {
            path: fixture(
                "home/.codex/sessions/2026/01/02/rollout-2026-01-02T08-00-00-00000000-0000-7000-8000-0000000000d4.jsonl",
            ),
            session_id: None,
            matched: Matched::Id,
        };
        let now = SystemTime::now();
        assert_eq!(codex.subagents(&at, now), None);
        let activity = crate::sessions::activity(codex, &at, now);
        assert_eq!(activity.subagents_running, None);
        assert!(activity.idle_ms.is_some());
    }
}

#[cfg(test)]
mod choice_tests;
