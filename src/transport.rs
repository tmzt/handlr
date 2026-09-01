//! Transport layer for `handlr`, bridging to the `libherdr` crate and multiplexer backend.

use async_trait::async_trait;

pub use libherdr::{
    client::HerdrClient,
    error::{HerdrError, HerdrResult},
    protocol::{
        AgentStatus, EnvironmentSnapshot, HerdrRequest, HerdrResponse, LayoutAction, PaneInfo,
        TabInfo, WorkspaceInfo, DEFAULT_INSPECT_LINES,
    },
    transport::{
        HerdrTransport, MockState, RemoteSshConfig, SshInvocationMethod, default_local_socket_path,
    },
};

use crate::commands::{confirmation_match_candidates, validate_arbitrary_command};
use crate::config::AgentConfig;
use crate::watch::WatchRegistry;
use libconversate::VoiceState;
use std::sync::Arc;

/// Adapter to implement `libinfer::ToolExecutor` for `HerdrClient`.
pub struct HerdrToolExecutor {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
    pub voice_state: Option<Arc<VoiceState>>,
    pub watches: Option<Arc<WatchRegistry>>,
}

impl HerdrToolExecutor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self {
            client,
            agents,
            voice_state: None,
            watches: None,
        }
    }

    pub fn with_voice_state(mut self, voice_state: Arc<VoiceState>) -> Self {
        self.voice_state = Some(voice_state);
        self
    }

    pub fn with_watches(mut self, watches: Arc<WatchRegistry>) -> Self {
        self.watches = Some(watches);
        self
    }

    /// Handle the local `watch_pane` / `unwatch_pane` tools.
    ///
    /// These touch no herdr state — they only decide which panes the event
    /// subscription covers — but the pane id is canonicalized against the live
    /// snapshot first, because herdr matches subscriptions by exact id and a
    /// watch registered as "p6" would silently never fire.
    async fn execute_watch_tool(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let Some(ref watches) = self.watches else {
            return Err("Pane watching is not available in this session.".to_string());
        };
        let requested = args
            .get("pane_id")
            .or_else(|| args.get("target_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if requested.is_empty() {
            return Err(format!("'{name}' requires a 'pane_id'."));
        }

        let resolved = match self.client.list_panes().await {
            Ok(snapshot) => resolve_pane_id(&snapshot, requested),
            Err(e) => {
                log::warn!("Could not resolve pane '{requested}' for {name}: {e}");
                None
            }
        };
        let Some(pane_id) = resolved else {
            return Err(format!(
                "No pane matching '{requested}' exists in the current herdr session."
            ));
        };

        if name == "watch_pane" {
            let added = watches.watch(&pane_id);
            log::info!("watch_pane '{pane_id}' (newly added: {added})");
            Ok(serde_json::json!({
                "pane_id": pane_id,
                "watching": true,
                "already_watching": !added,
                "watched_panes": watches.watched(),
            }))
        } else {
            let removed = watches.unwatch(&pane_id);
            log::info!("unwatch_pane '{pane_id}' (was watching: {removed})");
            Ok(serde_json::json!({
                "pane_id": pane_id,
                "watching": false,
                "was_watching": removed,
                "watched_panes": watches.watched(),
            }))
        }
    }
}

/// Unwrap the session snapshot from whatever envelope it arrived in.
pub(crate) fn snapshot_root(snapshot: &serde_json::Value) -> &serde_json::Value {
    snapshot
        .get("result")
        .and_then(|r| r.get("snapshot").or(Some(r)))
        .or_else(|| snapshot.get("snapshot"))
        .unwrap_or(snapshot)
}

/// Collect every pane object in a session snapshot, from both the flat `panes`
/// list and the nested workspaces -> tabs -> panes hierarchy.
pub(crate) fn collect_panes(snapshot: &serde_json::Value) -> Vec<&serde_json::Value> {
    let root = snapshot_root(snapshot);
    let mut panes = Vec::new();

    if let Some(flat) = root.get("panes").and_then(|p| p.as_array()) {
        panes.extend(flat.iter());
    }
    if let Some(workspaces) = root.get("workspaces").and_then(|w| w.as_array()) {
        for ws in workspaces {
            if let Some(tabs) = ws.get("tabs").and_then(|t| t.as_array()) {
                for tab in tabs {
                    if let Some(tab_panes) = tab.get("panes").and_then(|p| p.as_array()) {
                        panes.extend(tab_panes.iter());
                    }
                }
            }
            if let Some(ws_panes) = ws.get("panes").and_then(|p| p.as_array()) {
                panes.extend(ws_panes.iter());
            }
        }
    }
    panes
}

/// Whether a pane object is the one the caller meant, tolerating the short
/// (`p6`) and fully-qualified (`w1:p6`) spellings of an id.
fn pane_matches(pane: &serde_json::Value, target: &str) -> bool {
    pane.get("pane_id")
        .and_then(|id| id.as_str())
        .is_some_and(|pid| crate::watch::pane_ids_match(pid, target))
}

/// Resolve a pane id to the fully-qualified form herdr uses (`w1:p6`).
///
/// Event subscriptions are matched by exact pane id server-side, so a watch
/// registered as "p6" would silently never fire.
pub fn resolve_pane_id(snapshot: &serde_json::Value, target: &str) -> Option<String> {
    let clean = target.trim();
    if clean.is_empty() {
        return None;
    }
    collect_panes(snapshot)
        .into_iter()
        .find(|pane| pane_matches(pane, clean))
        .and_then(|pane| pane.get("pane_id"))
        .and_then(|id| id.as_str())
        .map(|id| id.to_string())
}

/// Extracts the lowercased agent name for a target pane from Herdr's session snapshot.
pub fn extract_agent_name_for_pane(snapshot: &serde_json::Value, target_pane_id: &str) -> String {
    let clean_target = target_pane_id.trim();

    for pane in collect_panes(snapshot) {
        if !pane_matches(pane, clean_target) {
            continue;
        }
        if let Some(agent) = pane.get("agent").and_then(|a| a.as_str()) {
            let lower = agent.trim().to_lowercase();
            if !lower.is_empty() {
                return lower;
            }
        }
        if let Some(extra) = pane.get("extra").and_then(|e| e.as_object()) {
            if let Some(agent) = extra.get("agent").and_then(|a| a.as_str()) {
                let lower = agent.trim().to_lowercase();
                if !lower.is_empty() {
                    return lower;
                }
            }
        }
        if let Some(title) = pane.get("terminal_title").and_then(|t| t.as_str()) {
            let lower_title = title.to_lowercase();
            for known in ["claude", "omp", "agy"] {
                if lower_title.contains(known) {
                    return known.to_string();
                }
            }
        }
    }

    "shell".to_string()
}

#[async_trait]
impl libinfer::ToolExecutor for HerdrToolExecutor {
    async fn execute_tool(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if name == "watch_pane" || name == "unwatch_pane" {
            return self.execute_watch_tool(name, args).await;
        }
        if name == "run_command" || name == "send_command" {
            let cmd = args
                .get("command")
                .or_else(|| args.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let pane_id = args
                .get("pane_id")
                .or_else(|| args.get("target_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Determine agent name from herdr session snapshot (lowercased)
            let agent_name = match self.client.list_panes().await {
                Ok(snap_val) => extract_agent_name_for_pane(&snap_val, pane_id),
                Err(e) => {
                    log::warn!("Failed to query snapshot to determine agent for pane {}: {}", pane_id, e);
                    "shell".to_string()
                }
            };

            log::info!("Permission check: agent='{}' pane='{}' command='{}'", agent_name, pane_id, cmd);

            let agent_config = self.agents.iter().find(|a| a.name.to_lowercase() == agent_name);
            let Some(agent_cfg) = agent_config else {
                return Err(format!(
                    "Agent '{}' (in pane '{}') is not configured in [[agents]]. Permitted agents: {:?}",
                    agent_name,
                    pane_id,
                    self.agents.iter().map(|a| &a.name).collect::<Vec<_>>()
                ));
            };

            let is_explicitly_allowed = agent_cfg.allowed_commands.iter().any(|allowed| {
                allowed.trim().eq_ignore_ascii_case(cmd.trim())
            });

            let final_cmd = if is_explicitly_allowed {
                cmd.trim().to_string()
            } else if agent_cfg.allow_arbitrary_commands {
                match validate_arbitrary_command(cmd) {
                    Ok(cleaned) => cleaned,
                    Err(reason) => {
                        log::warn!(
                            "Rejected arbitrary command for agent '{}' on pane '{}': {}",
                            agent_name, pane_id, reason
                        );
                        return Err(reason);
                    }
                }
            } else {
                return Err(format!(
                    "Command '{}' is not permitted for agent '{}'. Allowed commands for {}: {:?}",
                    cmd, agent_name, agent_name, agent_cfg.allowed_commands
                ));
            };

            // Mechanical confirmation gate check
            if agent_cfg.require_confirmation {
                if let Some(ref vs) = self.voice_state {
                    // The gate was armed from the assistant's own phrasing, which may
                    // differ cosmetically from the string it passes here.
                    let confirmed = confirmation_match_candidates(&final_cmd, cmd)
                        .iter()
                        .any(|candidate| vs.verify_and_consume_confirmation(pane_id, candidate));
                    if !confirmed {
                        log::warn!("Mechanical confirmation gate blocked '{}' on pane '{}' (confirmation not armed or confirmed)", final_cmd, pane_id);
                        return Err(format!(
                            "Mechanical confirmation gate blocked execution: Command '{}' on pane '{}' requires vocal confirmation. You must ask the user for confirmation first.",
                            final_cmd, pane_id
                        ));
                    }
                    log::info!("Mechanical confirmation gate validated for '{}' on pane '{}'", final_cmd, pane_id);
                }
            }

            let mut final_args = args.clone();
            if let Some(obj) = final_args.as_object_mut() {
                obj.insert("command".to_string(), serde_json::Value::String(final_cmd));
            }
            return self.client.execute_tool_call(name, &final_args).await.map_err(|e| e.to_string());
        }
        self.client.execute_tool_call(name, args).await.map_err(|e| e.to_string())
    }
}

/// Context contributor that queries `herdr` for the active workspace snapshot and tails active panes,
/// injecting this layout/agent status context as a system prompt/RAG turn before inference.
pub struct HerdrContextContributor {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
    pub watches: Option<Arc<WatchRegistry>>,
}

impl HerdrContextContributor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self {
            client,
            agents,
            watches: None,
        }
    }

    pub fn with_watches(mut self, watches: Arc<WatchRegistry>) -> Self {
        self.watches = Some(watches);
        self
    }
}

#[async_trait]
impl libinfer::TurnContributor for HerdrContextContributor {
    async fn contribute(&self, messages: &mut Vec<libinfer::Message>) -> Result<(), String> {
        // 1. Query the list of all panes from herdr
        let snapshot = match self.client.list_panes().await {
            Ok(s) => s,
            Err(e) => {
                log::warn!("failed to list herdr panes: {}", e);
                messages.push(libinfer::Message {
                    role: "system".to_string(),
                    content: format!(
                        "Current herdr environment status:\nMultiplexer connection warning: {e}. If the user asks about agents or panes, inform them that the herdr multiplexer is currently unreachable or returned an error."
                    ),
                    name: None,
                    thought_signature: None,
                });
                return Ok(());
            }
        };

        // 2. Read tail buffers for active/working/focused panes (or explicitly requested panes)
        let snap_obj = snapshot.get("snapshot").unwrap_or(&snapshot);

        let focused_ws_id = snap_obj
            .get("focused_workspace_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let focused_pane_id = snap_obj
            .get("focused_pane_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let last_user_query = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str());

        let mut pane_details = Vec::new();
        let mut pane_summaries = Vec::new();

        // Check flat panes list from session.snapshot
        if let Some(panes) = snap_obj.get("panes").and_then(|p| p.as_array()) {
            for (idx, pane) in panes.iter().enumerate() {
                let pane_id = pane.get("pane_id").and_then(|id| id.as_str()).unwrap_or("");
                let status = pane.get("agent_status").and_then(|s| s.as_str()).unwrap_or("unknown");
                let focused = pane.get("focused").and_then(|f| f.as_bool()).unwrap_or(false);
                let agent = pane.get("agent").and_then(|a| a.as_str()).unwrap_or("shell");
                let cwd = pane.get("cwd").and_then(|c| c.as_str()).unwrap_or("");
                let ws_id = pane.get("workspace_id").and_then(|w| w.as_str()).unwrap_or("");
                let tab_id = pane.get("tab_id").and_then(|t| t.as_str()).unwrap_or("");

                pane_summaries.push(format!(
                    "- Pane {} (ID: {}) [Workspace: {}, Tab: {}, Agent: {}, Status: {}, Focused: {}, CWD: {}]",
                    idx + 1, pane_id, ws_id, tab_id, agent, status, focused, cwd
                ));

                let should_include = should_include_pane_buffer(
                    idx,
                    pane_id,
                    status,
                    focused,
                    agent,
                    cwd,
                    ws_id,
                    last_user_query,
                );

                if should_include && !pane_id.is_empty() {
                    if let Ok(inspect_res) = self.client.inspect_pane(pane_id, 50).await {
                        let buffer_opt = inspect_res
                            .get("read")
                            .and_then(|r| r.get("text"))
                            .and_then(|t| t.as_str())
                            .or_else(|| inspect_res.get("buffer").and_then(|b| b.as_str()))
                            .or_else(|| inspect_res.get("text").and_then(|t| t.as_str()));

                        if let Some(buffer) = buffer_opt {
                            let trimmed = buffer.trim();
                            if !trimmed.is_empty() {
                                pane_details.push(format!(
                                    "#### Pane {} (ID: {}) [Agent: {}, Status: {}, Focused: {}, CWD: '{}']\nRecent 50 Lines of Output:\n```\n{}\n```",
                                    idx + 1, pane_id, agent, status, focused, cwd, trimmed
                                ));
                            } else {
                                pane_details.push(format!(
                                    "#### Pane {} (ID: {}) [Agent: {}, Status: {}, Focused: {}, CWD: '{}']\n(Buffer is idle/empty)",
                                    idx + 1, pane_id, agent, status, focused, cwd
                                ));
                            }
                        }
                    }
                }
            }
        } else if let Some(workspaces) = snap_obj.get("workspaces").and_then(|w| w.as_array()) {
            // Fallback for nested workspace/tab structure
            let mut global_idx = 1;
            for ws in workspaces {
                let ws_id = ws.get("workspace_id").and_then(|i| i.as_str()).unwrap_or("");
                let ws_name = ws.get("name").and_then(|n| n.as_str()).unwrap_or(ws_id);

                if let Some(tabs) = ws.get("tabs").and_then(|t| t.as_array()) {
                    for tab in tabs {
                        let tab_title = tab.get("title").and_then(|t| t.as_str()).unwrap_or("");
                        if let Some(panes) = tab.get("panes").and_then(|p| p.as_array()) {
                            for pane in panes {
                                if let Some(pane_id) = pane.get("pane_id").and_then(|id| id.as_str()) {
                                    let status = pane.get("agent_status").and_then(|s| s.as_str()).unwrap_or("unknown");
                                    let focused = pane.get("focused").and_then(|f| f.as_bool()).unwrap_or(false);
                                    let title = pane.get("terminal_title").and_then(|t| t.as_str()).unwrap_or("untitled");

                                    pane_summaries.push(format!(
                                        "- Pane {} (ID: {}) [Workspace: {}, Tab: {}, Title: '{}', Status: {}, Focused: {}]",
                                        global_idx, pane_id, ws_name, tab_title, title, status, focused
                                    ));

                                    let should_include = should_include_pane_buffer(
                                        global_idx - 1,
                                        pane_id,
                                        status,
                                        focused,
                                        title,
                                        "",
                                        ws_id,
                                        last_user_query,
                                    );

                                    if should_include {
                                        if let Ok(inspect_res) = self.client.inspect_pane(pane_id, 50).await {
                                            let buffer_opt = inspect_res
                                                .get("read")
                                                .and_then(|r| r.get("text"))
                                                .and_then(|t| t.as_str())
                                                .or_else(|| inspect_res.get("buffer").and_then(|b| b.as_str()))
                                                .or_else(|| inspect_res.get("text").and_then(|t| t.as_str()));

                                            if let Some(buffer) = buffer_opt {
                                                let trimmed = buffer.trim();
                                                if !trimmed.is_empty() {
                                                    pane_details.push(format!(
                                                        "#### Pane {} (ID: {}) [Workspace: {}, Title: '{}', Status: {}, Focused: {}]:\nRecent 50 Lines of Output:\n```\n{}\n```",
                                                        global_idx, pane_id, ws_name, title, status, focused, trimmed
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                    global_idx += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Format overall RAG payload with buffers placed prominently
        let mut content = format!(
            "### Herdr Multiplexer State\n- Active Workspace: {}\n- Focused Pane: {}\n\nPanes Overview:\n{}\n\n### Recent Terminal Output (Active / Working / Requested Panes)\n\n",
            focused_ws_id, focused_pane_id, pane_summaries.join("\n")
        );

        if !pane_details.is_empty() {
            for detail in pane_details {
                content.push_str(&detail);
                content.push_str("\n\n");
            }
        } else {
            content.push_str("No active/working terminal pane output to report.\n\n");
        }

        if !self.agents.is_empty() {
            content.push_str("### Configured Agents & Permitted Commands\n");
            for agent in &self.agents {
                content.push_str(&format!(
                    "- Agent '{}': Allowed commands: {:?}, Allow Arbitrary Safe Commands: {}, Requires Confirmation: {}\n",
                    agent.name, agent.allowed_commands, agent.allow_arbitrary_commands, agent.require_confirmation
                ));
            }
            if self.agents.iter().any(|a| a.allow_arbitrary_commands) {
                content.push_str(&format!(
                    "Arbitrary commands are validated mechanically before they are sent: ASCII only, \
                     only letters, digits, spaces and the punctuation . , ' ? : (no '/', no '-', no shell \
                     metacharacters, no newlines), {} characters max. Rephrase in plain words rather than \
                     declining when the user's wording needs a forbidden character.\n",
                    crate::commands::MAX_ARBITRARY_COMMAND_LEN
                ));
            }
            content.push_str("\n");
        }

        if let Some(ref watches) = self.watches {
            let watched = watches.watched();
            content.push_str("### Watched Panes\n");
            if watched.is_empty() {
                content.push_str(
                    "No panes are being watched. Use `watch_pane` when the user asks to be \
                     notified when an agent needs input or finishes.\n\n",
                );
            } else {
                content.push_str(&format!(
                    "Announcing status changes for: {}. Use `unwatch_pane` to stop.\n\n",
                    watched.join(", ")
                ));
            }
        }

        // 4. Inject as a system instruction/turn
        messages.push(libinfer::Message {
            role: "system".to_string(),
            content,
            name: None,
            thought_signature: None,
        });

        Ok(())
    }
}

/// Determines whether a pane's 50-line buffer should be fetched and injected into the turn RAG context.
fn should_include_pane_buffer(
    pane_idx: usize,
    pane_id: &str,
    status: &str,
    focused: bool,
    agent: &str,
    cwd: &str,
    workspace_id: &str,
    last_user_query: Option<&str>,
) -> bool {
    // 1. Always include working or focused panes
    if status == "working" || focused {
        return true;
    }

    // 2. Check if the user query explicitly requested this pane or all panes
    if let Some(query) = last_user_query {
        let q = query.to_lowercase();

        // Did user ask for all panes?
        if q.contains("all pane") || q.contains("all buffer") || q.contains("every pane") || q.contains("everything") {
            return true;
        }

        // Check pane index (e.g. "pane 1", "pain 1", "first pane", "first pain")
        let idx_str = (pane_idx + 1).to_string();
        if q.contains(&format!("pane {}", idx_str)) || q.contains(&format!("pain {}", idx_str)) {
            return true;
        }

        let ordinal = match pane_idx + 1 {
            1 => Some("first"),
            2 => Some("second"),
            3 => Some("third"),
            4 => Some("fourth"),
            5 => Some("fifth"),
            6 => Some("sixth"),
            7 => Some("seventh"),
            8 => Some("eighth"),
            9 => Some("ninth"),
            10 => Some("tenth"),
            _ => None,
        };
        if let Some(ord) = ordinal {
            if q.contains(&format!("{} pane", ord)) || q.contains(&format!("{} pain", ord)) {
                return true;
            }
        }

        // Check pane ID (e.g. "w1:p1", "p1")
        if !pane_id.is_empty() && q.contains(&pane_id.to_lowercase()) {
            return true;
        }

        // Check agent name if not generic shell (e.g. "claude", "omp", "agy")
        if agent != "shell" && !agent.is_empty() && q.contains(&agent.to_lowercase()) {
            return true;
        }

        // Check directory name if user mentioned it
        if !cwd.is_empty() {
            if let Some(dir_name) = std::path::Path::new(cwd).file_name().and_then(|f| f.to_str()) {
                if q.contains(&dir_name.to_lowercase()) {
                    return true;
                }
            }
        }

        // Check workspace ID (e.g. "w1", "workspace 1", "workspace one")
        if !workspace_id.is_empty() && q.contains(&workspace_id.to_lowercase()) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use libinfer::ToolExecutor;
    use serde_json::json;

    #[tokio::test]
    async fn test_allowed_commands_filtering() {
        let client = HerdrClient::mock();
        let executor = HerdrToolExecutor::new(
            client,
            vec![
                AgentConfig::new("claude", vec!["continue".to_string(), "/compact".to_string()], true),
                AgentConfig::new("omp", vec!["continue".to_string()], true),
            ],
        );

        // Allowed command for claude (mock pane p1 defaults to agent 'claude') should execute
        let ok_res = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "continue" }))
            .await;
        assert!(ok_res.is_ok());

        let ok_res2 = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "/compact" }))
            .await;
        assert!(ok_res2.is_ok());

        // Disallowed command for claude should be rejected
        let err_res = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "rm -rf /" }))
            .await;
        assert!(err_res.is_err());
        assert!(err_res.unwrap_err().contains("not permitted for agent 'claude'"));
    }

    #[test]
    fn test_extract_agent_name_for_pane() {
        let nested_snap = json!({
            "snapshot": {
                "panes": [
                    { "pane_id": "w2:p1", "agent": "claude", "cwd": "/Users/tmeade/src/highbay" },
                    { "pane_id": "w2:pA", "agent": "agy", "cwd": "/Users/tmeade/src/highbay" }
                ],
                "workspaces": [
                    {
                        "workspace_id": "w2",
                        "tabs": [
                            {
                                "tab_id": "w2:t1",
                                "panes": [
                                    { "pane_id": "w2:p1", "agent": "claude" }
                                ]
                            }
                        ]
                    }
                ]
            }
        });

        assert_eq!(extract_agent_name_for_pane(&nested_snap, "w2:p1"), "claude");
        assert_eq!(extract_agent_name_for_pane(&nested_snap, "p1"), "claude");
        assert_eq!(extract_agent_name_for_pane(&nested_snap, "w2:pA"), "agy");
        assert_eq!(extract_agent_name_for_pane(&nested_snap, "unknown"), "shell");
    }

    #[tokio::test]
    async fn test_mechanical_confirmation_gate_enforcement() {
        let client = HerdrClient::mock();
        let vs = Arc::new(VoiceState::default());
        let executor = HerdrToolExecutor::new(
            client,
            vec![
                AgentConfig::new("claude", vec!["continue".to_string(), "/compact".to_string()], true),
            ],
        )
        .with_voice_state(Arc::clone(&vs));

        // 1. Initial attempt without confirmation should be blocked by the mechanical gate
        let blocked_res = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "continue" }))
            .await;
        assert!(blocked_res.is_err());
        assert!(blocked_res.unwrap_err().contains("Mechanical confirmation gate blocked execution"));

        // 2. Arm confirmation gate
        vs.arm_confirmation(Some("p1".to_string()), Some("continue".to_string()));
        assert!(vs.is_confirmation_armed());

        // Still blocked while armed (not yet confirmed)
        let blocked_armed = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "continue" }))
            .await;
        assert!(blocked_armed.is_err());

        // 3. Confirm the gate
        assert!(vs.confirm_gate());

        // 4. Execution now succeeds and consumes the confirmation
        let ok_res = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "continue" }))
            .await;
        assert!(ok_res.is_ok());

        // 5. Subsequent execution without a new confirmation cycle is blocked again
        let blocked_after = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "continue" }))
            .await;
        assert!(blocked_after.is_err());
    }

    /// End-to-end: the gate is armed from the assistant's own confirmation
    /// question, so an arbitrary command only runs if it matches what the user
    /// was actually asked about.
    #[tokio::test]
    async fn test_arbitrary_command_gate_bound_to_confirmation_question() {
        use crate::commands::parse_confirmation_request;

        let executor = HerdrToolExecutor::new(
            HerdrClient::mock(),
            vec![AgentConfig::new("claude", vec!["continue".to_string()], true)
                .with_arbitrary_commands(true)],
        );
        let vs = Arc::new(VoiceState::default());
        let executor = executor.with_voice_state(Arc::clone(&vs));

        // The assistant asks, so the gate is armed with the pane and command it named.
        let req = parse_confirmation_request(
            "Should I send 'please run the tests' to Claude in pane w2:p1?",
        )
        .expect("assistant turn is a confirmation request");
        vs.arm_confirmation(req.pane_id.clone(), req.command.clone());
        assert!(vs.confirm_gate(), "user said yes");

        // A different command than the one confirmed is refused even though the
        // gate is in the Confirmed state.
        let swapped = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "w2:p1", "command": "delete the repo" }),
            )
            .await;
        assert!(swapped.is_err());
        assert!(swapped.unwrap_err().contains("requires vocal confirmation"));

        // The confirmed command runs, with the model's trailing punctuation and
        // casing normalized away.
        let ok = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "w2:p1", "command": "Please run the tests." }),
            )
            .await;
        assert!(ok.is_ok(), "expected success, got {:?}", ok);

        // The confirmation is single-use.
        let replay = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "w2:p1", "command": "Please run the tests." }),
            )
            .await;
        assert!(replay.is_err());
    }

    #[tokio::test]
    async fn test_watch_and_unwatch_pane_tools() {
        let watches = Arc::new(WatchRegistry::new());
        let executor = HerdrToolExecutor::new(HerdrClient::mock(), vec![])
            .with_watches(Arc::clone(&watches));

        let res = executor
            .execute_tool("watch_pane", &json!({ "pane_id": "p1" }))
            .await
            .expect("watch_pane should succeed");
        assert_eq!(res["pane_id"], "p1");
        assert_eq!(res["watching"], true);
        assert_eq!(res["already_watching"], false);
        assert!(watches.is_watched("p1"));

        // Watching again is reported as a no-op rather than an error.
        let again = executor
            .execute_tool("watch_pane", &json!({ "pane_id": "p1" }))
            .await
            .expect("second watch_pane should succeed");
        assert_eq!(again["already_watching"], true);

        let off = executor
            .execute_tool("unwatch_pane", &json!({ "pane_id": "p1" }))
            .await
            .expect("unwatch_pane should succeed");
        assert_eq!(off["watching"], false);
        assert_eq!(off["was_watching"], true);
        assert!(watches.is_empty());
    }

    #[tokio::test]
    async fn test_watch_pane_rejects_unknown_and_missing_panes() {
        let watches = Arc::new(WatchRegistry::new());
        let executor = HerdrToolExecutor::new(HerdrClient::mock(), vec![])
            .with_watches(Arc::clone(&watches));

        // A pane herdr does not know about would subscribe to nothing.
        let err = executor
            .execute_tool("watch_pane", &json!({ "pane_id": "p999" }))
            .await
            .unwrap_err();
        assert!(err.contains("No pane matching"), "got: {err}");

        let err = executor
            .execute_tool("watch_pane", &json!({}))
            .await
            .unwrap_err();
        assert!(err.contains("requires a 'pane_id'"), "got: {err}");
        assert!(watches.is_empty());
    }

    #[tokio::test]
    async fn test_watch_tools_unavailable_without_registry() {
        let executor = HerdrToolExecutor::new(HerdrClient::mock(), vec![]);
        let err = executor
            .execute_tool("watch_pane", &json!({ "pane_id": "p1" }))
            .await
            .unwrap_err();
        assert!(err.contains("not available"), "got: {err}");
    }

    #[test]
    fn test_resolve_pane_id_canonicalizes() {
        let snap = json!({"snapshot": {"panes": [
            {"pane_id": "w1:p6", "agent": "claude"},
            {"pane_id": "w1:p1", "agent": "agy"}
        ]}});
        assert_eq!(resolve_pane_id(&snap, "p6").as_deref(), Some("w1:p6"));
        assert_eq!(resolve_pane_id(&snap, "w1:p6").as_deref(), Some("w1:p6"));
        assert_eq!(resolve_pane_id(&snap, "  P1 ").as_deref(), Some("w1:p1"));
        assert_eq!(resolve_pane_id(&snap, "p9"), None);
        assert_eq!(resolve_pane_id(&snap, ""), None);
    }

    #[tokio::test]
    async fn test_arbitrary_commands_disabled_by_default() {
        let executor = HerdrToolExecutor::new(
            HerdrClient::mock(),
            vec![AgentConfig::new("claude", vec!["continue".to_string()], false)],
        );
        let res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "please run the tests" }),
            )
            .await;
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("is not permitted for agent"));
    }

    #[tokio::test]
    async fn test_arbitrary_safe_commands_validation() {
        let client = HerdrClient::mock();
        let vs = Arc::new(VoiceState::default());
        let executor = HerdrToolExecutor::new(
            client,
            vec![
                AgentConfig::new("claude", vec!["continue".to_string()], true)
                    .with_arbitrary_commands(true),
            ],
        )
        .with_voice_state(Arc::clone(&vs));

        // Arm and confirm for a safe arbitrary command with periods and commas
        vs.arm_confirmation(Some("p1".to_string()), Some("please run cargo test, check output.".to_string()));
        vs.confirm_gate();

        let ok_res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "please run cargo test, check output." }),
            )
            .await;
        assert!(ok_res.is_ok());

        // Attempting a command with slash '/' must be rejected
        vs.arm_confirmation(Some("p1".to_string()), Some("cat /etc/passwd".to_string()));
        vs.confirm_gate();
        let slash_res = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "cat /etc/passwd" }))
            .await;
        assert!(slash_res.is_err());
        assert!(slash_res.unwrap_err().contains("'/' and '-' are not permitted"));

        // Attempting a command with hyphen '-' must be rejected
        vs.arm_confirmation(Some("p1".to_string()), Some("cargo test --all".to_string()));
        vs.confirm_gate();
        let dash_res = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "cargo test --all" }))
            .await;
        assert!(dash_res.is_err());
        assert!(dash_res.unwrap_err().contains("'/' and '-' are not permitted"));

        // Non-ASCII command must be rejected
        vs.arm_confirmation(Some("p1".to_string()), Some("你好".to_string()));
        vs.confirm_gate();
        let non_ascii_res = executor
            .execute_tool("run_command", &json!({ "pane_id": "p1", "command": "你好" }))
            .await;
        assert!(non_ascii_res.is_err());
        assert!(non_ascii_res.unwrap_err().contains("must be ASCII only"));
    }
}
