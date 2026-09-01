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

use crate::config::AgentConfig;
use libconversate::VoiceState;
use std::sync::Arc;

/// Adapter to implement `libinfer::ToolExecutor` for `HerdrClient`.
pub struct HerdrToolExecutor {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
    pub voice_state: Option<Arc<VoiceState>>,
}

impl HerdrToolExecutor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self {
            client,
            agents,
            voice_state: None,
        }
    }

    pub fn with_voice_state(mut self, voice_state: Arc<VoiceState>) -> Self {
        self.voice_state = Some(voice_state);
        self
    }
}

/// Extracts the lowercased agent name for a target pane from Herdr's session snapshot.
pub fn extract_agent_name_for_pane(snapshot: &serde_json::Value, target_pane_id: &str) -> String {
    let clean_target = target_pane_id.trim();

    // Unwrap if wrapped in JSON-RPC result or snapshot key
    let snap_obj = snapshot
        .get("result")
        .and_then(|r| r.get("snapshot").or(Some(r)))
        .or_else(|| snapshot.get("snapshot"))
        .unwrap_or(snapshot);

    let get_agent_from_pane = |pane: &serde_json::Value| -> Option<String> {
        let pid = pane.get("pane_id").and_then(|id| id.as_str())?;
        if pid == clean_target
            || pid.ends_with(&format!(":{}", clean_target))
            || clean_target.ends_with(&format!(":{}", pid))
        {
            if let Some(agent) = pane.get("agent").and_then(|a| a.as_str()) {
                let lower = agent.trim().to_lowercase();
                if !lower.is_empty() {
                    return Some(lower);
                }
            }
            if let Some(extra) = pane.get("extra").and_then(|e| e.as_object()) {
                if let Some(agent) = extra.get("agent").and_then(|a| a.as_str()) {
                    let lower = agent.trim().to_lowercase();
                    if !lower.is_empty() {
                        return Some(lower);
                    }
                }
            }
            if let Some(title) = pane.get("terminal_title").and_then(|t| t.as_str()) {
                let lower_title = title.to_lowercase();
                if lower_title.contains("claude") {
                    return Some("claude".to_string());
                } else if lower_title.contains("omp") {
                    return Some("omp".to_string());
                } else if lower_title.contains("agy") {
                    return Some("agy".to_string());
                }
            }
        }
        None
    };

    // 1. Check flat panes list from session.snapshot
    if let Some(panes) = snap_obj.get("panes").and_then(|p| p.as_array()) {
        for pane in panes {
            if let Some(agent) = get_agent_from_pane(pane) {
                return agent;
            }
        }
    }

    // 2. Check nested workspaces -> tabs -> panes
    if let Some(workspaces) = snap_obj.get("workspaces").and_then(|w| w.as_array()) {
        for ws in workspaces {
            if let Some(tabs) = ws.get("tabs").and_then(|t| t.as_array()) {
                for tab in tabs {
                    if let Some(panes) = tab.get("panes").and_then(|p| p.as_array()) {
                        for pane in panes {
                            if let Some(agent) = get_agent_from_pane(pane) {
                                return agent;
                            }
                        }
                    }
                }
            }
            if let Some(panes) = ws.get("panes").and_then(|p| p.as_array()) {
                for pane in panes {
                    if let Some(agent) = get_agent_from_pane(pane) {
                        return agent;
                    }
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

            let is_allowed = agent_cfg.allowed_commands.iter().any(|allowed| {
                allowed.trim().eq_ignore_ascii_case(cmd.trim())
            });
            if !is_allowed {
                return Err(format!(
                    "Command '{}' is not permitted for agent '{}'. Allowed commands for {}: {:?}",
                    cmd, agent_name, agent_name, agent_cfg.allowed_commands
                ));
            }

            // Mechanical confirmation gate check
            if agent_cfg.require_confirmation {
                if let Some(ref vs) = self.voice_state {
                    let confirmed = vs.verify_and_consume_confirmation(pane_id, cmd);
                    if !confirmed {
                        log::warn!("Mechanical confirmation gate blocked '{}' on pane '{}' (confirmation not armed or confirmed)", cmd, pane_id);
                        return Err(format!(
                            "Mechanical confirmation gate blocked execution: Command '{}' on pane '{}' requires vocal confirmation. You must ask the user for confirmation first.",
                            cmd, pane_id
                        ));
                    }
                    log::info!("Mechanical confirmation gate validated for '{}' on pane '{}'", cmd, pane_id);
                }
            }
        }
        self.client.execute_tool_call(name, args).await.map_err(|e| e.to_string())
    }
}

/// Context contributor that queries `herdr` for the active workspace snapshot and tails active panes,
/// injecting this layout/agent status context as a system prompt/RAG turn before inference.
pub struct HerdrContextContributor {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
}

impl HerdrContextContributor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self { client, agents }
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
                    "- Agent '{}': Allowed commands: {:?}, Requires Confirmation: {}\n",
                    agent.name, agent.allowed_commands, agent.require_confirmation
                ));
            }
            content.push_str("\n");
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
}
