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
        default_local_socket_path, HerdrTransport, MockState, RemoteSshConfig, SshInvocationMethod,
    },
};

use crate::config::AgentConfig;
use crate::core::policy;
use crate::core::principal::Principal;
use crate::watch::WatchRegistry;
use std::sync::Arc;

pub use crate::core::agents::{extract_agent_name_for_pane, resolve_pane_id};

/// Adapter to implement `libinfer::ToolExecutor` for `HerdrClient`.
pub struct HerdrToolExecutor {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
    pub watches: Option<Arc<WatchRegistry>>,
    /// Who the commands are sent as.
    pub principal: Principal,
}

impl HerdrToolExecutor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self {
            client,
            agents,
            watches: None,
            principal: Principal::local("handlr"),
        }
    }

    pub fn with_principal(mut self, principal: Principal) -> Self {
        self.principal = principal;
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
                    log::warn!(
                        "Failed to query snapshot to determine agent for pane {}: {}",
                        pane_id,
                        e
                    );
                    "shell".to_string()
                }
            };

            log::info!(
                "Permission check: agent='{}' pane='{}' command='{}'",
                agent_name,
                pane_id,
                cmd
            );

            let checked =
                match policy::check(&self.principal, &self.agents, &agent_name, pane_id, cmd) {
                    Ok(checked) => checked,
                    Err(denied) => {
                        log::warn!(
                            "Refused '{}' for agent '{}' on pane '{}': {}",
                            cmd,
                            agent_name,
                            pane_id,
                            denied
                        );
                        return Err(denied.to_string());
                    }
                };
            let final_cmd = checked.command;

            // No confirmation path exists until the daemon's confirmation
            // broker does (docs/design/20261022_01_ORCHESTRATION_DAEMON_PLAN.md,
            // step 3). Refuse rather than send unconfirmed.
            if checked.require_confirmation {
                log::warn!(
                    "Refused '{}' on pane '{}': confirmation unavailable",
                    final_cmd,
                    pane_id
                );
                return Err(format!(
                    "Cannot send '{}' to pane '{}': confirmation unavailable (this agent requires confirmation)",
                    final_cmd, pane_id
                ));
            }

            let mut final_args = args.clone();
            if let Some(obj) = final_args.as_object_mut() {
                obj.insert("command".to_string(), serde_json::Value::String(final_cmd));
            }
            return self
                .client
                .execute_tool_call(name, &final_args)
                .await
                .map_err(|e| e.to_string());
        }
        self.client
            .execute_tool_call(name, args)
            .await
            .map_err(|e| e.to_string())
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
                let status = pane
                    .get("agent_status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                let focused = pane
                    .get("focused")
                    .and_then(|f| f.as_bool())
                    .unwrap_or(false);
                let agent = pane
                    .get("agent")
                    .and_then(|a| a.as_str())
                    .unwrap_or("shell");
                let cwd = pane.get("cwd").and_then(|c| c.as_str()).unwrap_or("");
                let ws_id = pane
                    .get("workspace_id")
                    .and_then(|w| w.as_str())
                    .unwrap_or("");
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
                let ws_id = ws
                    .get("workspace_id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("");
                let ws_name = ws.get("name").and_then(|n| n.as_str()).unwrap_or(ws_id);

                if let Some(tabs) = ws.get("tabs").and_then(|t| t.as_array()) {
                    for tab in tabs {
                        let tab_title = tab.get("title").and_then(|t| t.as_str()).unwrap_or("");
                        if let Some(panes) = tab.get("panes").and_then(|p| p.as_array()) {
                            for pane in panes {
                                if let Some(pane_id) =
                                    pane.get("pane_id").and_then(|id| id.as_str())
                                {
                                    let status = pane
                                        .get("agent_status")
                                        .and_then(|s| s.as_str())
                                        .unwrap_or("unknown");
                                    let focused = pane
                                        .get("focused")
                                        .and_then(|f| f.as_bool())
                                        .unwrap_or(false);
                                    let title = pane
                                        .get("terminal_title")
                                        .and_then(|t| t.as_str())
                                        .unwrap_or("untitled");

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
                                        if let Ok(inspect_res) =
                                            self.client.inspect_pane(pane_id, 50).await
                                        {
                                            let buffer_opt = inspect_res
                                                .get("read")
                                                .and_then(|r| r.get("text"))
                                                .and_then(|t| t.as_str())
                                                .or_else(|| {
                                                    inspect_res
                                                        .get("buffer")
                                                        .and_then(|b| b.as_str())
                                                })
                                                .or_else(|| {
                                                    inspect_res.get("text").and_then(|t| t.as_str())
                                                });

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
        if q.contains("all pane")
            || q.contains("all buffer")
            || q.contains("every pane")
            || q.contains("everything")
        {
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
            if let Some(dir_name) = std::path::Path::new(cwd)
                .file_name()
                .and_then(|f| f.to_str())
            {
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
                AgentConfig::new(
                    "claude",
                    vec!["continue".to_string(), "/compact".to_string()],
                    false,
                ),
                AgentConfig::new("omp", vec!["continue".to_string()], false),
            ],
        );

        // Allowed command for claude (mock pane p1 defaults to agent 'claude') should execute
        let ok_res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "continue" }),
            )
            .await;
        assert!(ok_res.is_ok());

        let ok_res2 = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "/compact" }),
            )
            .await;
        assert!(ok_res2.is_ok());

        // Disallowed command for claude should be rejected
        let err_res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "rm -rf /" }),
            )
            .await;
        assert!(err_res.is_err());
        assert!(err_res
            .unwrap_err()
            .contains("not permitted for agent 'claude'"));
    }

    #[tokio::test]
    async fn test_confirmation_required_commands_are_refused() {
        let executor = HerdrToolExecutor::new(
            HerdrClient::mock(),
            vec![AgentConfig::new(
                "claude",
                vec!["continue".to_string()],
                true,
            )],
        );
        let res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "continue" }),
            )
            .await;
        assert!(res.unwrap_err().contains("confirmation unavailable"));
    }

    #[tokio::test]
    async fn test_watch_and_unwatch_pane_tools() {
        let watches = Arc::new(WatchRegistry::new());
        let executor =
            HerdrToolExecutor::new(HerdrClient::mock(), vec![]).with_watches(Arc::clone(&watches));

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
        let executor =
            HerdrToolExecutor::new(HerdrClient::mock(), vec![]).with_watches(Arc::clone(&watches));

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

    #[tokio::test]
    async fn test_arbitrary_commands_disabled_by_default() {
        let executor = HerdrToolExecutor::new(
            HerdrClient::mock(),
            vec![AgentConfig::new(
                "claude",
                vec!["continue".to_string()],
                false,
            )],
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
        let executor = HerdrToolExecutor::new(
            HerdrClient::mock(),
            vec![
                AgentConfig::new("claude", vec!["continue".to_string()], false)
                    .with_arbitrary_commands(true),
            ],
        );

        // A safe arbitrary command with periods and commas

        let ok_res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "please run cargo test, check output." }),
            )
            .await;
        assert!(ok_res.is_ok());

        // Attempting a command with slash '/' must be rejected
        let slash_res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "cat /etc/passwd" }),
            )
            .await;
        assert!(slash_res.is_err());
        assert!(slash_res
            .unwrap_err()
            .contains("'/' and '-' are not permitted"));

        // Attempting a command with hyphen '-' must be rejected
        let dash_res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "cargo test --all" }),
            )
            .await;
        assert!(dash_res.is_err());
        assert!(dash_res
            .unwrap_err()
            .contains("'/' and '-' are not permitted"));

        // Non-ASCII command must be rejected
        let non_ascii_res = executor
            .execute_tool(
                "run_command",
                &json!({ "pane_id": "p1", "command": "你好" }),
            )
            .await;
        assert!(non_ascii_res.is_err());
        assert!(non_ascii_res.unwrap_err().contains("must be ASCII only"));
    }
}
