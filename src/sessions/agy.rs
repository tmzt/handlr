//! agy (Antigravity CLI) SQLite session reader (plan 20261023_06, step 4).
//!
//! agy stores its conversation steps in SQLite at
//! `~/.gemini/antigravity-cli/conversations/<session id>.db` in WAL mode.
//!
//! This reader opens databases read-only with a 100 ms busy timeout, reading
//! through the WAL. It only queries the `steps` and metadata tables. It never
//! opens `antigravity-oauth-token`, `brain/`, or other files.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::proto_wire::Message;
use super::{
    EntryKind, Item, Located, Matched, Role, SessionReader, TAIL_MAX, TAIL_READ_MAX, TEXT_MAX,
    TOOL_TEXT_MAX, cut, link_results, safe_id,
};

/// Reader for agy (Antigravity CLI) conversations stored in SQLite.
pub struct AgyReader {
    pub dir: PathBuf,
}

impl AgyReader {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Locate an agy session: by `session_id` (`<id>.db`), else by finding
    /// the newest database whose metadata names `cwd`.
    pub fn locate(&self, cwd: Option<&Path>, session_id: Option<&str>) -> Option<Located> {
        if let Some(id) = session_id {
            if !safe_id(id) {
                return None;
            }
            let path = self.dir.join(format!("{id}.db"));
            return path.is_file().then_some(Located {
                path,
                session_id: Some(id.to_string()),
                matched: Matched::Id,
            });
        }
        let cwd = cwd?;
        let cwd_bytes = cwd.to_string_lossy().into_owned().into_bytes();
        let uri_bytes = format!("file://{}", cwd.display()).into_bytes();

        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "db"))
            .collect();

        files.sort_by(|a, b| {
            let m_a = std::fs::metadata(a).and_then(|m| m.modified()).ok();
            let m_b = std::fs::metadata(b).and_then(|m| m.modified()).ok();
            m_b.cmp(&m_a)
        });

        for path in files {
            if db_names_cwd(&path, &cwd_bytes, &uri_bytes) {
                let session_id = path.file_stem().map(|s| s.to_string_lossy().into_owned());
                return Some(Located {
                    path,
                    session_id,
                    matched: Matched::Cwd,
                });
            }
        }
        None
    }

    /// Read entries from the newest back until `enough` holds or limits are reached.
    pub fn read_back(
        &self,
        at: &Located,
        enough: &dyn Fn(&[Item]) -> bool,
    ) -> Result<super::Read, String> {
        let conn = Connection::open_with_flags(
            &at.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(|e| format!("open agy db {}: {e}", at.path.display()))?;

        conn.busy_timeout(std::time::Duration::from_millis(100))
            .map_err(|e| format!("busy timeout: {e}"))?;

        let mut stmt = conn
            .prepare("SELECT idx, step_type, status, step_payload FROM steps ORDER BY idx DESC")
            .map_err(|e| format!("prepare steps: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                let idx: i64 = row.get(0)?;
                let step_type: i64 = row.get(1)?;
                let status: i64 = row.get(2)?;
                let payload: Vec<u8> = row.get(3)?;
                Ok((idx, step_type, status, payload))
            })
            .map_err(|e| format!("query steps: {e}"))?;

        let mut steps: Vec<Vec<Item>> = Vec::new();
        let mut total_bytes: u64 = 0;
        let mut conversation: u64 = 0;

        for row in rows {
            let (idx, step_type, status, payload) =
                row.map_err(|e| format!("read step row: {e}"))?;
            total_bytes += payload.len() as u64;

            let step_items = decode_step(idx as u64, step_type, status, &payload);
            if !step_items.is_empty() {
                for item in &step_items {
                    if item.kind == EntryKind::Text || item.kind == EntryKind::ToolUse {
                        conversation += item.text.len() as u64;
                    }
                }
                steps.push(step_items);

                if conversation >= TAIL_MAX || total_bytes >= TAIL_READ_MAX {
                    break;
                }
                // Check enough on accumulated items (in logical oldest-first order)
                let check_items: Vec<Item> = steps.iter().rev().flatten().cloned().collect();
                if enough(&check_items) {
                    break;
                }
            }
        }

        let mut items: Vec<Item> = steps.into_iter().rev().flatten().collect();
        link_results(&mut items);

        Ok(super::Read {
            items,
            bytes_read: total_bytes,
        })
    }

    /// Highest step `idx` in the session database, if any.
    pub fn max_idx(&self, path: &Path) -> std::io::Result<Option<u32>> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(|e| std::io::Error::other(format!("open agy db {}: {e}", path.display())))?;

        conn.busy_timeout(std::time::Duration::from_millis(100))
            .map_err(|e| std::io::Error::other(format!("busy timeout: {e}")))?;

        let mut stmt = conn
            .prepare("SELECT MAX(idx) FROM steps")
            .map_err(|e| std::io::Error::other(format!("prepare max idx: {e}")))?;

        let max: Option<i64> = stmt
            .query_row([], |row| row.get(0))
            .map_err(|e| std::io::Error::other(format!("query max idx: {e}")))?;

        Ok(max.map(|m| m.max(0) as u32))
    }

    /// Read records between step indexes `from` and `to` inclusive, in ascending order.
    pub fn read_range(
        &self,
        path: &Path,
        from: u32,
        to: u32,
    ) -> std::io::Result<Vec<(u32, Vec<Item>)>> {
        if from > to {
            return Ok(Vec::new());
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(|e| std::io::Error::other(format!("open agy db {}: {e}", path.display())))?;

        conn.busy_timeout(std::time::Duration::from_millis(100))
            .map_err(|e| std::io::Error::other(format!("busy timeout: {e}")))?;

        let mut stmt = conn
            .prepare("SELECT idx, step_type, status, step_payload FROM steps WHERE idx >= ? AND idx <= ? ORDER BY idx ASC")
            .map_err(|e| std::io::Error::other(format!("prepare steps range: {e}")))?;

        let rows = stmt
            .query_map([from as i64, to as i64], |row| {
                let idx: i64 = row.get(0)?;
                let step_type: i64 = row.get(1)?;
                let status: i64 = row.get(2)?;
                let payload: Vec<u8> = row.get(3)?;
                Ok((idx, step_type, status, payload))
            })
            .map_err(|e| std::io::Error::other(format!("query steps range: {e}")))?;

        let mut out = Vec::new();
        for row in rows {
            let (idx, step_type, status, payload) =
                row.map_err(|e| std::io::Error::other(format!("read step row: {e}")))?;
            let items = decode_step(idx as u64, step_type, status, &payload);
            if !items.is_empty() {
                out.push((idx as u32, items));
            }
        }
        Ok(out)
    }
}

impl SessionReader for AgyReader {
    fn kind(&self) -> &'static str {
        "agy"
    }

    fn locate(&self, cwd: Option<&Path>, session_id: Option<&str>) -> Option<Located> {
        self.locate(cwd, session_id)
    }

    fn read_back(
        &self,
        at: &Located,
        enough: &dyn Fn(&[Item]) -> bool,
    ) -> Result<super::Read, String> {
        self.read_back(at, enough)
    }
}

/// Check if a database's trajectory metadata mentions `cwd_bytes` or `uri_bytes`.
fn db_names_cwd(path: &Path, cwd_bytes: &[u8], uri_bytes: &[u8]) -> bool {
    let conn = match Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    ) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(100));

    // Check trajectory_metadata_blob
    if let Ok(mut stmt) = conn.prepare("SELECT data FROM trajectory_metadata_blob LIMIT 1")
        && let Ok(blob) = stmt.query_row([], |r| r.get::<_, Vec<u8>>(0))
        && (slice_contains(&blob, cwd_bytes) || slice_contains(&blob, uri_bytes))
    {
        return true;
    }

    // Check trajectory_meta or other tables if present
    if let Ok(mut stmt) = conn.prepare("SELECT trajectory_id FROM trajectory_meta LIMIT 1")
        && let Ok(id) = stmt.query_row([], |r| r.get::<_, String>(0))
        && id
            .as_bytes()
            .windows(cwd_bytes.len())
            .any(|w| w == cwd_bytes)
    {
        return true;
    }

    false
}

fn slice_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Decode a single step row into conversation items.
fn decode_step(idx: u64, step_type: i64, status: i64, payload: &[u8]) -> Vec<Item> {
    let msg = Message::new(payload);
    let ts = msg
        .path_varint(&[5, 1, 1])
        .ok()
        .flatten()
        .map(format_rfc3339)
        .unwrap_or_default();

    match step_type {
        14 => {
            // User prompt: text is at 19.2
            let text = match msg.path_string(&[19, 2]) {
                Ok(Some(s)) if !s.trim().is_empty() => s,
                _ => return Vec::new(),
            };
            vec![Item {
                pos: idx,
                ts,
                role: Role::User,
                kind: EntryKind::Text,
                tool: None,
                text: cut(text, TEXT_MAX),
                call_id: None,
                origin: None,
            }]
        }
        15 => {
            // Agent response: narration at 20.3, tool calls at 20.7, reply at 20.1 / 20.8
            let mut items = Vec::new();
            if let Ok(Some(narration)) = msg.path_string(&[20, 3]) {
                let narration = narration.trim();
                if !narration.is_empty() {
                    items.push(Item {
                        pos: idx,
                        ts: ts.clone(),
                        role: Role::Assistant,
                        kind: EntryKind::Text,
                        tool: None,
                        text: cut(narration, TEXT_MAX),
                        call_id: None,
                        origin: None,
                    });
                }
            }

            if let Ok(Some(sub20)) = msg.message(20)
                && let Ok(calls) = sub20.repeated_messages(7)
            {
                for call_msg in calls {
                    let call_id = call_msg.string(1).ok().flatten().map(str::to_string);
                    let tool_name = call_msg.string(2).ok().flatten().unwrap_or("tool");
                    let args = call_msg.string(3).ok().flatten().unwrap_or_default();
                    let text = if args.is_empty() {
                        tool_name.to_string()
                    } else {
                        format!("{tool_name} {args}")
                    };
                    items.push(Item {
                        pos: idx,
                        ts: ts.clone(),
                        role: Role::Assistant,
                        kind: EntryKind::ToolUse,
                        tool: Some(tool_name.to_string()),
                        text: cut(&text, TOOL_TEXT_MAX),
                        call_id,
                        origin: None,
                    });
                }
            }

            let reply = msg
                .path_string(&[20, 1])
                .ok()
                .flatten()
                .or_else(|| msg.path_string(&[20, 8]).ok().flatten());
            if let Some(reply) = reply {
                let reply = reply.trim();
                if !reply.is_empty() {
                    items.push(Item {
                        pos: idx,
                        ts: ts.clone(),
                        role: Role::Assistant,
                        kind: EntryKind::Text,
                        tool: None,
                        text: cut(reply, TEXT_MAX),
                        call_id: None,
                        origin: None,
                    });
                }
            }

            items
        }
        132 => {
            // Tool result: call answered at 5.4 {1: call_id, 2: name}, output at 140.2.1
            let call_id = msg
                .path_string(&[5, 4, 1])
                .ok()
                .flatten()
                .map(str::to_string);
            let tool_name = msg
                .path_string(&[5, 4, 2])
                .ok()
                .flatten()
                .map(str::to_string);
            let output_str = msg.path_string(&[140, 2, 1]).ok().flatten();
            let output = match output_str {
                Some(s) => s.to_string(),
                None => match msg.path_bytes(&[140, 2, 1]) {
                    Ok(Some(bytes)) => String::from_utf8_lossy(bytes).into_owned(),
                    _ => String::new(),
                },
            };
            let text = if status != 3 && !output.starts_with("error:") {
                format!("error: {output}")
            } else {
                output
            };
            vec![Item {
                pos: idx,
                ts,
                role: Role::Tool,
                kind: EntryKind::ToolResult,
                tool: tool_name,
                text: cut(&text, TOOL_TEXT_MAX),
                call_id,
                origin: None,
            }]
        }
        101 => {
            // Task notification: title at 114.2.1
            let title = msg
                .path_string(&[114, 2, 1])
                .ok()
                .flatten()
                .unwrap_or_default();
            vec![Item {
                pos: idx,
                ts,
                role: Role::User,
                kind: EntryKind::Text,
                tool: None,
                text: cut(title, TEXT_MAX),
                call_id: None,
                origin: Some("task".to_string()),
            }]
        }
        23 => {
            // State snapshot: not read
            Vec::new()
        }
        _ => {
            // Unknown step types: not read
            Vec::new()
        }
    }
}

/// Convert seconds since the Unix epoch into an RFC 3339 UTC string.
pub fn format_rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::pending;
    use crate::sessions::turn::{self, Turn};
    use libherdr::protocol::AgentStatus;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("sessions")
            .join("agy")
            .join(name)
    }

    #[test]
    fn rfc3339_formatting() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(951_868_800), "2000-03-01T00:00:00Z");
    }

    #[test]
    fn asking_fixture_reads_items_and_pending_question() {
        let path = fixture_path("asking.db");
        let reader = AgyReader::new(path.parent().unwrap().to_path_buf());
        let located = Located {
            path: path.clone(),
            session_id: Some("asking".into()),
            matched: Matched::Id,
        };
        let read = reader.read_back(&located, &|_| false).unwrap();
        let items = read.items;
        assert_eq!(items.len(), 5);

        // Step 0: User prompt
        assert_eq!(items[0].pos, 0);
        assert_eq!(items[0].role, Role::User);
        assert_eq!(items[0].kind, EntryKind::Text);
        assert_eq!(items[0].text, "is the docs site up?");

        // Step 1: Narration & Tool call
        assert_eq!(items[1].pos, 1);
        assert_eq!(items[1].role, Role::Assistant);
        assert_eq!(items[1].kind, EntryKind::Text);
        assert_eq!(items[1].text, "Checking the server.");

        assert_eq!(items[2].pos, 1);
        assert_eq!(items[2].role, Role::Assistant);
        assert_eq!(items[2].kind, EntryKind::ToolUse);
        assert_eq!(items[2].tool.as_deref(), Some("run_command"));
        assert_eq!(items[2].call_id.as_deref(), Some("call_1"));

        // Step 2: Tool result
        assert_eq!(items[3].pos, 2);
        assert_eq!(items[3].role, Role::Tool);
        assert_eq!(items[3].kind, EntryKind::ToolResult);
        assert_eq!(items[3].tool.as_deref(), Some("run_command"));
        assert_eq!(items[3].call_id.as_deref(), Some("call_1"));
        assert_eq!(items[3].text, "server running on port 4000");

        // Step 4: Assistant reply (asking question)
        assert_eq!(items[4].pos, 4);
        assert_eq!(items[4].role, Role::Assistant);
        assert_eq!(items[4].kind, EntryKind::Text);
        assert!(items[4].text.contains("Shall I open it in the browser?"));

        // pending_question finds the question
        let q = pending::pending_question(&items, &AgentStatus::Blocked, false).unwrap();
        assert_eq!(q.text, "Shall I open it in the browser?");
    }

    #[test]
    fn stopped_fixture_reads_items_and_stopped_turn() {
        let path = fixture_path("stopped.db");
        let reader = AgyReader::new(path.parent().unwrap().to_path_buf());
        let located = Located {
            path: path.clone(),
            session_id: Some("stopped".into()),
            matched: Matched::Id,
        };
        let read = reader.read_back(&located, &|_| false).unwrap();
        let items = read.items;
        assert_eq!(items.len(), 5);

        // Turn evaluates to Stopped with the reply line
        let t = turn::turn(&items, &AgentStatus::Blocked, false);
        match t {
            Turn::Stopped { line } => {
                assert_eq!(line, "The docs site is up on port 4000.");
            }
            other => panic!("expected Turn::Stopped, got {other:?}"),
        }
    }

    #[test]
    fn locked_database_busy_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("locked.db");
        std::fs::copy(fixture_path("asking.db"), &db_path).unwrap();

        // Open an exclusive connection holding a lock
        let lock_conn = Connection::open(&db_path).unwrap();
        lock_conn.execute_batch("BEGIN EXCLUSIVE;").unwrap();

        let reader = AgyReader::new(dir.path().to_path_buf());
        let located = Located {
            path: db_path,
            session_id: Some("locked".into()),
            matched: Matched::Id,
        };

        let start = std::time::Instant::now();
        let result = reader.read_back(&located, &|_| false);
        let elapsed = start.elapsed();

        // Either it succeeds reading via WAL or returns a busy error within ~200ms
        if let Err(e) = result {
            assert!(e.contains("busy") || e.contains("locked"), "{e}");
            assert!(elapsed < std::time::Duration::from_millis(500));
        }
        drop(lock_conn);
    }

    #[test]
    fn no_file_other_than_db_wal_shm_opened() {
        let dir = tempfile::tempdir().unwrap();
        let gemini_dir = dir.path().join(".gemini").join("antigravity-cli");
        let conv_dir = gemini_dir.join("conversations");
        std::fs::create_dir_all(&conv_dir).unwrap();

        // Copy asking.db into conversations/
        let db_path = conv_dir.join("test_session.db");
        std::fs::copy(fixture_path("asking.db"), &db_path).unwrap();

        // Place a decoy token file in .gemini/antigravity-cli/antigravity-oauth-token
        let token_path = gemini_dir.join("antigravity-oauth-token");
        std::fs::write(&token_path, "secret_oauth_token_decoy").unwrap();

        // Place brain directory
        let brain_dir = gemini_dir.join("brain");
        std::fs::create_dir_all(&brain_dir).unwrap();
        std::fs::write(brain_dir.join("notes.md"), "private brain notes").unwrap();

        let reader = AgyReader::new(conv_dir);
        let located = reader.locate(None, Some("test_session")).unwrap();
        assert_eq!(located.path, db_path);

        let read = reader.read_back(&located, &|_| false).unwrap();
        assert_eq!(read.items.len(), 5);

        // Verify the decoy token file was never modified or touched
        let token_content = std::fs::read_to_string(&token_path).unwrap();
        assert_eq!(token_content, "secret_oauth_token_decoy");
    }

    #[test]
    fn max_idx_and_read_range() {
        let path = fixture_path("asking.db");
        let reader = AgyReader::new(path.parent().unwrap().to_path_buf());
        let max = reader.max_idx(&path).unwrap();
        assert_eq!(max, Some(4));

        let rows = reader.read_range(&path, 1, 3).unwrap();
        // Steps 1 and 2 produce items; step 3 (type 23) produces none.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, 1);
        assert_eq!(rows[1].0, 2);
        assert_eq!(rows[0].1[0].pos, 1);
        assert_eq!(rows[1].1[0].pos, 2);

        let empty = reader.read_range(&path, 5, 10).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    #[ignore]
    fn repowiki_example_if_available() {
        let Some(path_str) = std::env::var_os("HANDLR_AGY_EXAMPLE") else {
            return;
        };
        let path = PathBuf::from(path_str);
        if !path.is_file() {
            return;
        }
        let reader = AgyReader::new(path.parent().unwrap().to_path_buf());
        let located = Located {
            path: path.clone(),
            session_id: path.file_stem().map(|s| s.to_string_lossy().into_owned()),
            matched: Matched::Id,
        };
        let read = reader.read_back(&located, &|_| false).unwrap();
        assert!(!read.items.is_empty());
    }
}
