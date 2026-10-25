//! Codex sessions: `~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<id>.jsonl`
//! (plan 20261022_03, decision 3 and the 16:23 EDT amendment).
//!
//! The first line is `session_meta`, with `payload.cwd` and `payload.id`.
//! `response_item` lines carry the conversation in `payload`: `message`
//! (by its role; `developer` and `system` messages are instructions and
//! are skipped), `function_call` and `custom_tool_call` (tool use),
//! their `_output`s (tool results), and `reasoning`, which is left out.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    EntryKind, Item, LineReader, Located, Matched, Role, SessionReader, cut, first_line, safe_id,
};
use super::{TEXT_MAX, TOOL_TEXT_MAX};

/// How many day directories are searched, newest first.
pub const DAYS_SEARCHED: usize = 14;
/// How much of a rollout's first line is read for its `session_meta`.
const META_MAX: usize = 1024 * 1024;

/// The Codex reader, over one `sessions` directory.
pub struct Codex {
    sessions: PathBuf,
}

impl Codex {
    /// A reader of the rollouts under `sessions` (`~/.codex/sessions`).
    pub fn new(sessions: PathBuf) -> Self {
        Self { sessions }
    }

    /// The newest `DAYS_SEARCHED` day directories, newest first.
    fn days(&self) -> Vec<PathBuf> {
        let mut days = Vec::new();
        for year in sorted_dirs_desc(&self.sessions) {
            for month in sorted_dirs_desc(&year) {
                for day in sorted_dirs_desc(&month) {
                    days.push(day);
                    if days.len() == DAYS_SEARCHED {
                        return days;
                    }
                }
            }
        }
        days
    }

    /// Every rollout in the searched days, newest first by name (names
    /// start with their timestamp).
    fn rollouts(&self) -> impl Iterator<Item = PathBuf> + use<> {
        self.days().into_iter().flat_map(|day| {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&day)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().is_some_and(|x| x == "jsonl")
                        && p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with("rollout-"))
                })
                .collect();
            files.sort_by(|a, b| b.cmp(a));
            files
        })
    }
}

/// The subdirectories of `dir`, by name, last first.
fn sorted_dirs_desc(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort_by(|a, b| b.cmp(a));
    dirs
}

/// A rollout's `session_meta` payload.
fn meta(path: &Path) -> Option<Value> {
    let line = first_line(path, META_MAX)?;
    let v: Value = serde_json::from_str(&line).ok()?;
    (v.get("type").and_then(|t| t.as_str()) == Some("session_meta"))
        .then(|| v.get("payload").cloned())
        .flatten()
}

impl SessionReader for Codex {
    fn kind(&self) -> &'static str {
        "codex"
    }

    fn locate(&self, cwd: Option<&Path>, session_id: Option<&str>) -> Option<Located> {
        if let Some(id) = session_id {
            if !safe_id(id) {
                return None;
            }
            let suffix = format!("-{id}.jsonl");
            return self
                .rollouts()
                .find(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().ends_with(&suffix))
                })
                .map(|path| Located {
                    path,
                    session_id: Some(id.to_string()),
                    matched: Matched::Id,
                });
        }
        let want = cwd?.to_string_lossy().into_owned();
        self.rollouts().find_map(|path| {
            let meta = meta(&path)?;
            if meta.get("cwd").and_then(|c| c.as_str()) != Some(want.as_str()) {
                return None;
            }
            let session_id = meta
                .get("id")
                .or_else(|| meta.get("session_id"))
                .and_then(|i| i.as_str())
                .map(str::to_string);
            Some(Located {
                path,
                session_id,
                matched: Matched::Cwd,
            })
        })
    }

    fn read_back(
        &self,
        at: &Located,
        enough: &dyn Fn(&[Item]) -> bool,
    ) -> Result<super::Read, String> {
        super::read_tail(self, &at.path, enough)
    }
}

impl LineReader for Codex {
    fn parse_line(&self, line: &str) -> Vec<Item> {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            return Vec::new();
        }
        let Some(p) = v.get("payload") else {
            return Vec::new();
        };
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        let s = |k: &str| p.get(k).and_then(|x| x.as_str());
        let call_id = s("call_id").map(str::to_string);
        let item = match s("type") {
            Some("message") => {
                let role = match s("role") {
                    Some("user") => Role::User,
                    Some("assistant") => Role::Assistant,
                    _ => return Vec::new(),
                };
                let text = p
                    .get("content")
                    .and_then(|c| c.as_array())
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                            .map(str::trim)
                            .filter(|t| !t.is_empty())
                            .filter(|t| !(role == Role::User && super::claude::is_harness_text(t)))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default();
                if text.is_empty() {
                    return Vec::new();
                }
                Item {
                    pos: 0,
                    ts,
                    role,
                    kind: EntryKind::Text,
                    tool: None,
                    text: cut(&text, TEXT_MAX),
                    call_id: None,
                    origin: None,
                }
            }
            Some("function_call") | Some("custom_tool_call") | Some("local_shell_call") => {
                let name = s("name").unwrap_or("shell").to_string();
                let input = match p.get("arguments").or_else(|| p.get("input")) {
                    // Arguments are a JSON string: shown compact.
                    Some(Value::String(raw)) => serde_json::from_str::<Value>(raw)
                        .map(|a| a.to_string())
                        .unwrap_or_else(|_| raw.clone()),
                    Some(other) => other.to_string(),
                    None => p.get("action").map(Value::to_string).unwrap_or_default(),
                };
                Item {
                    pos: 0,
                    ts,
                    role: Role::Assistant,
                    kind: EntryKind::ToolUse,
                    tool: Some(name.clone()),
                    text: cut(&format!("{name} {input}"), TOOL_TEXT_MAX),
                    call_id,
                    origin: None,
                }
            }
            Some("function_call_output") | Some("custom_tool_call_output") => {
                let text = match p.get("output") {
                    Some(Value::String(out)) => out.clone(),
                    Some(Value::Object(o)) => o
                        .get("content")
                        .or_else(|| o.get("output"))
                        .and_then(|c| c.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| Value::Object(o.clone()).to_string()),
                    Some(other) => other.to_string(),
                    None => String::new(),
                };
                Item {
                    pos: 0,
                    ts,
                    role: Role::Tool,
                    kind: EntryKind::ToolResult,
                    tool: None,
                    text: cut(&text, TOOL_TEXT_MAX),
                    call_id,
                    origin: None,
                }
            }
            // reasoning, and anything new.
            _ => return Vec::new(),
        };
        vec![item]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::test_support::fixture;

    fn reader() -> Codex {
        Codex::new(fixture("home/.codex/sessions"))
    }

    #[test]
    fn messages_calls_and_outputs_without_reasoning() {
        let r = reader();
        let l = r.locate(Some(Path::new("/work/example")), None).unwrap();
        assert_eq!(l.matched, Matched::Cwd);
        assert_eq!(
            l.session_id.as_deref(),
            Some("00000000-0000-7000-8000-0000000000c3")
        );
        let items = r.tail(&l.path, 50).unwrap();
        let got: Vec<(Role, EntryKind, Option<String>, String)> = items
            .iter()
            .map(|i| (i.role, i.kind, i.tool.clone(), i.text.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                (Role::User, EntryKind::Text, None, "list the agents".into()),
                (
                    Role::Assistant,
                    EntryKind::Text,
                    None,
                    "I will check which agents are running.".into()
                ),
                (
                    Role::Assistant,
                    EntryKind::ToolUse,
                    Some("list_agents".into()),
                    r#"list_agents {"all":true}"#.into()
                ),
                (
                    Role::Tool,
                    EntryKind::ToolResult,
                    Some("list_agents".into()),
                    "one agent: primary".into()
                ),
                (
                    Role::Assistant,
                    EntryKind::Text,
                    None,
                    "Only the primary agent is running. Do you want me to start another?".into()
                ),
            ]
        );
        assert_eq!(items[0].ts, "2026-01-03T09:00:01.000Z");
        assert_eq!(
            items.iter().map(|i| i.pos).collect::<Vec<_>>(),
            vec![5, 8, 9, 10, 12]
        );
    }

    #[test]
    fn located_by_id_in_the_file_name() {
        let r = reader();
        let l = r
            .locate(None, Some("00000000-0000-7000-8000-0000000000c3"))
            .unwrap();
        assert_eq!(l.matched, Matched::Id);
        assert!(
            l.path
                .to_string_lossy()
                .ends_with("-00000000-0000-7000-8000-0000000000c3.jsonl")
        );
        assert_eq!(
            r.locate(None, Some("00000000-0000-7000-8000-00000000ffff")),
            None
        );
        // Another cwd: the older rollout.
        let l = r.locate(Some(Path::new("/work/other")), None).unwrap();
        assert_eq!(
            l.session_id.as_deref(),
            Some("00000000-0000-7000-8000-0000000000d4")
        );
        assert_eq!(r.locate(Some(Path::new("/nowhere")), None), None);
    }

    #[test]
    fn only_fourteen_days_are_searched() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=15 {
            let d = dir.path().join(format!("2026/01/{day:02}"));
            std::fs::create_dir_all(&d).unwrap();
            let id = format!("00000000-0000-7000-8000-0000000000{day:02}");
            std::fs::write(
                d.join(format!("rollout-2026-01-{day:02}T00-00-00-{id}.jsonl")),
                format!("{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/w/{day}\"}}}}\n"),
            )
            .unwrap();
        }
        let r = Codex::new(dir.path().to_path_buf());
        assert!(r.locate(Some(Path::new("/w/2")), None).is_some());
        assert!(r.locate(Some(Path::new("/w/1")), None).is_none());
        assert!(
            r.locate(None, Some("00000000-0000-7000-8000-000000000001"))
                .is_none()
        );
    }
}
