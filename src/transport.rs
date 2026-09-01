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

        // 2. Read logs for active panes ("working") to provide deeper context (RAG)
        let mut working_panes_details = Vec::new();
        if let Some(workspaces) = snapshot.get("workspaces").and_then(|w| w.as_array()) {
            for ws in workspaces {
                if let Some(tabs) = ws.get("tabs").and_then(|t| t.as_array()) {
                    for tab in tabs {
                        if let Some(panes) = tab.get("panes").and_then(|p| p.as_array()) {
                            for pane in panes {
                                let status = pane.get("agent_status").and_then(|s| s.as_str()).unwrap_or("unknown");
                                if status == "working" {
                                    if let Some(pane_id) = pane.get("pane_id").and_then(|id| id.as_str()) {
                                        // Read tail buffer lines (e.g. 50 lines) to find logs / compiler failures
                                        if let Ok(inspect_res) = self.client.inspect_pane(pane_id, 50).await {
                                            if let Some(buffer) = inspect_res.get("buffer").and_then(|b| b.as_str()) {
                                                working_panes_details.push(format!(
                                                    "Pane {} tail buffer:\n---\n{}\n---",
                                                    pane_id, buffer
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Format overall herdr snapshot
        let snapshot_json = serde_json::to_string_pretty(&snapshot)
            .unwrap_or_default();

        let mut content = format!(
            "Current herdr environment snapshot:\n```json\n{}\n```\n",
            snapshot_json
        );

        if !working_panes_details.is_empty() {
            content.push_str("\nActive working agent tail buffers:\n");
            for detail in working_panes_details {
                content.push_str(&detail);
                content.push_str("\n");
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
