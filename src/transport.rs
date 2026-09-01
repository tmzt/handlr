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

/// Adapter to implement `libinfer::ToolExecutor` for `HerdrClient`.
pub struct HerdrToolExecutor(pub HerdrClient);

#[async_trait]
impl libinfer::ToolExecutor for HerdrToolExecutor {
    async fn execute_tool(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.0.execute_tool_call(name, args).await.map_err(|e| e.to_string())
    }
}

/// Context contributor that queries `herdr` for the active workspace snapshot and tails active panes,
/// injecting this layout/agent status context as a system prompt/RAG turn before inference.
pub struct HerdrContextContributor {
    pub client: HerdrClient,
}

impl HerdrContextContributor {
    pub fn new(client: HerdrClient) -> Self {
        Self { client }
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

        // 2. Read tail buffers for all panes in the snapshot
        let snap_obj = snapshot.get("snapshot").unwrap_or(&snapshot);

        let focused_ws_id = snap_obj
            .get("focused_workspace_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let focused_pane_id = snap_obj
            .get("focused_pane_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

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

                if !pane_id.is_empty() {
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
            "### Herdr Multiplexer State\n- Active Workspace: {}\n- Focused Pane: {}\n\nPanes Overview:\n{}\n\n### Recent Terminal Output (Last 50 Lines per Pane)\n\n",
            focused_ws_id, focused_pane_id, pane_summaries.join("\n")
        );

        if !pane_details.is_empty() {
            for detail in pane_details {
                content.push_str(&detail);
                content.push_str("\n\n");
            }
        } else {
            content.push_str("No active terminal pane output available.\n\n");
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
