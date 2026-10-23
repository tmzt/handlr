//! Claude Code sessions: `~/.claude/projects/<encoded cwd>/<session id>.jsonl`
//! (plan 20261022_03, decision 3 and the 16:23 EDT amendment).
//!
//! One JSON object per line. `user` and `assistant` lines carry
//! `message.content`, a string or a list of `text`, `thinking`,
//! `tool_use` and `tool_result` blocks; every other `type` is bookkeeping.
//! Lines from subagents (`isSidechain`), injected context (`isMeta`),
//! compaction summaries and task notifications are not the conversation
//! and are skipped, as is user text that is only a harness tag such as
//! `<command-name>…</command-name>`.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    EntryKind, Item, Located, Matched, Role, SessionReader, TEXT_MAX, TOOL_TEXT_MAX, cut,
    jsonl_newest_first, safe_id,
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

    fn parse_line(&self, line: &str) -> Vec<Item> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let role = match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => return Vec::new(),
        };
        let flag = |k: &str| v.get(k).and_then(|f| f.as_bool()) == Some(true);
        if flag("isSidechain") || flag("isMeta") || flag("isCompactSummary") {
            return Vec::new();
        }
        let origin = v
            .get("origin")
            .and_then(|o| o.get("kind"))
            .and_then(|k| k.as_str());
        if origin == Some("task-notification") {
            return Vec::new();
        }
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
            return Vec::new();
        };
        let text_item = |text: &str| -> Option<Item> {
            let text = text.trim();
            if text.is_empty() || (role == Role::User && is_harness_text(text)) {
                return None;
            }
            Some(Item {
                ts: ts.clone(),
                role,
                kind: EntryKind::Text,
                tool: None,
                text: cut(text, TEXT_MAX),
                call_id: None,
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
                        Some(Item {
                            ts: ts.clone(),
                            role: Role::Assistant,
                            kind: EntryKind::ToolUse,
                            tool: Some(name.to_string()),
                            text: cut(&format!("{name} {input}"), TOOL_TEXT_MAX),
                            call_id: b.get("id").and_then(|i| i.as_str()).map(str::to_string),
                        })
                    }
                    Some("tool_result") => {
                        let body = result_text(b.get("content"));
                        let error = b.get("is_error").and_then(|e| e.as_bool()) == Some(true);
                        let text = if error {
                            format!("error: {body}")
                        } else {
                            body
                        };
                        Some(Item {
                            ts: ts.clone(),
                            role: Role::Tool,
                            kind: EntryKind::ToolResult,
                            tool: None,
                            text: cut(&text, TOOL_TEXT_MAX),
                            call_id: b
                                .get("tool_use_id")
                                .and_then(|i| i.as_str())
                                .map(str::to_string),
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
        // Results carry their call's tool name; timestamps come through.
        assert_eq!(items[3].tool.as_deref(), Some("Read"));
        assert_eq!(items[5].tool.as_deref(), Some("Bash"));
        assert_eq!(items[0].ts, "2026-01-02T10:00:00.000Z");
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
        assert!(s.reader_for("agy").is_none());
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
}
