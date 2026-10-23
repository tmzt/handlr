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

use crate::config::{AgentConfig, NamingConfig};
use crate::core::agents;
use crate::core::confirm::{self, Broker, ConfirmError, HandleRequest, Lifetime};
use crate::core::naming::Names;
use crate::core::policy;
use crate::core::principal::Principal;
use crate::watch::WatchRegistry;
use std::sync::{Arc, Mutex};

pub use crate::core::agents::{extract_agent_name_for_pane, resolve_pane_id};

/// What one companion turn's tool calls did, read by the turn service
/// (plan 20261022_02, decisions 9–10).
#[derive(Debug, Default)]
pub struct TurnLog {
    /// Commands sent to panes, in order: `(pane_id, command)`.
    pub ran: Mutex<Vec<(String, String)>>,
    /// The voice-turn confirmations this turn opened, in order.
    pub confirmations: Mutex<Vec<u64>>,
    /// The pane the turn last addressed: the session's current agent.
    pub addressed: Mutex<Option<String>>,
}

/// A companion turn: confirmation-required commands open a
/// `Lifetime::VoiceTurn` confirmation, and what happened goes to `log`.
#[derive(Clone)]
pub struct VoiceTurn {
    pub broker: Arc<Broker>,
    pub log: Arc<TurnLog>,
    /// What the user said in this turn: a confirmation's handle must come
    /// from it (plan 20261022_08, 23:44 EDT amendment).
    pub utterance: Option<String>,
}

/// Adapter to implement `libinfer::ToolExecutor` for `HerdrClient`.
pub struct HerdrToolExecutor {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
    pub watches: Option<Arc<WatchRegistry>>,
    /// Who the commands are sent as.
    pub principal: Principal,
    /// When set, targets are resolved by name (plan 20261022_05,
    /// decision 10); otherwise a pane id is used as given.
    pub naming: Option<NamingConfig>,
    /// When set, a confirmation-required command opens a voice-turn
    /// confirmation; otherwise it is refused.
    pub voice_turn: Option<VoiceTurn>,
}

impl HerdrToolExecutor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self {
            client,
            agents,
            watches: None,
            principal: Principal::local("handlr"),
            naming: None,
            voice_turn: None,
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

    pub fn with_naming(mut self, naming: NamingConfig) -> Self {
        self.naming = Some(naming);
        self
    }

    pub fn with_voice_turn(mut self, voice_turn: VoiceTurn) -> Self {
        self.voice_turn = Some(voice_turn);
        self
    }

    /// Resolve `target` in `snapshot`: by name when naming is set, else as
    /// a pane id.
    fn resolve(&self, snapshot: &serde_json::Value, target: &str) -> Result<String, String> {
        match &self.naming {
            Some(naming) => Names::from_snapshot(snapshot, naming)
                .resolve(target)
                .map(|a| a.agent.pane_id.clone())
                .map_err(|e| e.to_string()),
            None => resolve_pane_id(snapshot, target).ok_or_else(|| {
                format!("No pane matching '{target}' exists in the current herdr session.")
            }),
        }
    }

    fn addressed(&self, pane_id: &str) {
        if let Some(turn) = &self.voice_turn {
            *turn.log.addressed.lock().unwrap() = Some(pane_id.to_string());
        }
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

        let snapshot = self.client.list_panes().await.map_err(|e| {
            log::warn!("Could not resolve pane '{requested}' for {name}: {e}");
            format!("No pane matching '{requested}' exists in the current herdr session.")
        })?;
        let pane_id = self.resolve(&snapshot, requested)?;
        self.addressed(&pane_id);

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

    /// `run_command`: the policy check as `principal`, then a send, a
    /// voice-turn confirmation, or a refusal.
    async fn execute_run_command(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let cmd = args
            .get("command")
            .or_else(|| args.get("cmd"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let requested = args
            .get("pane_id")
            .or_else(|| args.get("target_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let snapshot = match self.client.list_panes().await {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                log::warn!("Failed to read the herdr snapshot for pane {requested}: {e}");
                None
            }
        };
        // With naming on, the target must resolve; without it, a pane id is
        // used as given, as before.
        let pane_id = match (&snapshot, &self.naming) {
            (Some(snap), Some(_)) => self.resolve(snap, requested)?,
            (None, Some(_)) => return Err("herdr is not answering; nothing was sent.".to_string()),
            (_, None) => requested.to_string(),
        };
        let agent_name = match &snapshot {
            Some(snap) => extract_agent_name_for_pane(snap, &pane_id),
            None => "shell".to_string(),
        };
        self.addressed(&pane_id);

        log::info!(
            "Permission check: agent='{}' pane='{}' command='{}'",
            agent_name,
            pane_id,
            cmd
        );

        let checked = match policy::check(&self.principal, &self.agents, &agent_name, &pane_id, cmd)
        {
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

        if checked.require_confirmation {
            let Some(turn) = &self.voice_turn else {
                // No confirmation path outside a companion turn: refuse
                // rather than send unconfirmed.
                log::warn!(
                    "Refused '{}' on pane '{}': confirmation unavailable",
                    final_cmd,
                    pane_id
                );
                return Err(format!(
                    "Cannot send '{}' to pane '{}': confirmation unavailable (this agent requires confirmation)",
                    final_cmd, pane_id
                ));
            };
            let named = snapshot
                .as_ref()
                .zip(self.naming.as_ref())
                .and_then(|(snap, naming)| {
                    Names::from_snapshot(snap, naming).get(&pane_id).cloned()
                });
            let spoken = named
                .as_ref()
                .map(|a| a.spoken.clone())
                .unwrap_or_else(|| crate::core::naming::capitalise(&agent_name));
            // The agent's tab code spelled out: the first distinguishing word.
            let tab_code = named
                .as_ref()
                .zip(self.naming.as_ref())
                .and_then(|(a, naming)| {
                    a.tab.as_ref().map(|tab| {
                        if tab.word_form.is_empty() {
                            tab.spoken.clone()
                        } else {
                            crate::core::naming::spell(&tab.word_form, naming)
                        }
                    })
                });
            let request = HandleRequest {
                given: args.get("handle").and_then(|v| v.as_str()),
                utterance: turn.utterance.as_deref(),
                command: &final_cmd,
                agent: &spoken,
                tab_code: tab_code.as_deref(),
            };
            // A handle taken between settling it and asking is settled again.
            let mut attempt = 0;
            let (confirm_id, handle) = loop {
                let pending = turn.broker.pending_handles(&self.principal.id);
                let handle = confirm::settle_handle(&request, &pending, confirm::random_word);
                match turn.broker.request(
                    &self.principal,
                    &pane_id,
                    &agent_name,
                    &final_cmd,
                    Some(&handle),
                    Lifetime::VoiceTurn,
                ) {
                    Ok(id) => break (id, handle),
                    Err(ConfirmError::DuplicateHandle(_)) if attempt < 3 => attempt += 1,
                    Err(e) => return Err(e.to_string()),
                }
            };
            turn.log.confirmations.lock().unwrap().push(confirm_id);
            return Ok(serde_json::json!({
                "status": "awaiting_user_approval",
                "command": final_cmd,
                "agent": spoken,
                "handle": handle,
                "note": format!(
                    "Not sent yet. Right after your reply, handlr itself asks the user to approve sending '{final_cmd}' to {spoken}, and sends it only if they approve. Do not ask for confirmation yourself and do not call run_command again for this; say in a few words that it is waiting for their approval."
                ),
            }));
        }

        let mut final_args = args.clone();
        if let Some(obj) = final_args.as_object_mut() {
            obj.insert(
                "command".to_string(),
                serde_json::Value::String(final_cmd.clone()),
            );
            obj.insert(
                "pane_id".to_string(),
                serde_json::Value::String(pane_id.clone()),
            );
        }
        let result = self
            .client
            .execute_tool_call(name, &final_args)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(turn) = &self.voice_turn {
            turn.log.ran.lock().unwrap().push((pane_id, final_cmd));
        }
        Ok(result)
    }

    /// `focus`: by name when naming is set, a workspace or tab id as given.
    async fn execute_focus(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let mut args = args.clone();
        if self.naming.is_some() {
            let requested = args
                .get("target_id")
                .or_else(|| args.get("pane_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let snapshot = self.client.list_panes().await.map_err(|e| e.to_string())?;
            if !agents::is_workspace_or_tab_id(&snapshot, &requested) {
                let pane_id = self.resolve(&snapshot, &requested)?;
                self.addressed(&pane_id);
                if let Some(obj) = args.as_object_mut() {
                    obj.insert("target_id".to_string(), serde_json::Value::String(pane_id));
                }
            }
        }
        self.client
            .execute_tool_call(name, &args)
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl libinfer::ToolExecutor for HerdrToolExecutor {
    async fn execute_tool(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        match name {
            "watch_pane" | "unwatch_pane" => self.execute_watch_tool(name, args).await,
            "run_command" | "send_command" => self.execute_run_command(name, args).await,
            "focus" => self.execute_focus(name, args).await,
            _ => self
                .client
                .execute_tool_call(name, args)
                .await
                .map_err(|e| e.to_string()),
        }
    }
}

/// Context contributor that queries `herdr` for the active workspace snapshot and tails active panes,
/// injecting this layout/agent status context as a system prompt/RAG turn before inference.
pub struct HerdrContextContributor {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
    pub watches: Option<Arc<WatchRegistry>>,
    /// When set, the context lists each pane's names (plan 20261022_05),
    /// which tools accept as targets and replies should say.
    pub naming: Option<NamingConfig>,
}

impl HerdrContextContributor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self {
            client,
            agents,
            watches: None,
            naming: None,
        }
    }

    pub fn with_watches(mut self, watches: Arc<WatchRegistry>) -> Self {
        self.watches = Some(watches);
        self
    }

    pub fn with_naming(mut self, naming: NamingConfig) -> Self {
        self.naming = Some(naming);
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
            content.push('\n');
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

        if let Some(ref naming) = self.naming {
            let names = Names::from_snapshot(&snapshot, naming);
            if !names.agents.is_empty() {
                content.push_str("### Agent Names\n");
                content.push_str(
                    "Say these names, never pane ids. Tools accept a name, a tab label or a pane id as the target.\n",
                );
                for named in &names.agents {
                    content.push_str(&format!(
                        "- Pane {}: \"{}\" (say \"{}\")\n",
                        named.agent.pane_id, named.shown, named.spoken
                    ));
                }
                content.push('\n');
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
#[allow(clippy::too_many_arguments)]
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
    async fn test_voice_turn_confirmations_carry_a_settled_handle() {
        use crate::core::confirm::{StatusKey, DEFAULT_TTL};
        let client = HerdrClient::mock();
        let broker = Broker::new(Arc::new(client.clone()), DEFAULT_TTL);
        let log = Arc::new(TurnLog::default());
        let executor = |utterance: &str| {
            HerdrToolExecutor::new(
                client.clone(),
                vec![AgentConfig::new(
                    "claude",
                    vec!["continue".to_string(), "/usage".to_string()],
                    true,
                )],
            )
            .with_voice_turn(VoiceTurn {
                broker: Arc::clone(&broker),
                log: Arc::clone(&log),
                utterance: Some(utterance.to_string()),
            })
        };
        let run = |handle: &'static str, command: &'static str| json!({ "pane_id": "p1", "command": command, "handle": handle });

        // The model's handle, from the user's words, normalised.
        let said = "send usage to claude in high bay";
        let res = executor(said)
            .execute_tool("run_command", &run("Send-Usage-Claude-Highbay", "/usage"))
            .await
            .unwrap();
        assert_eq!(res["status"], "awaiting_user_approval");
        assert_eq!(res["handle"], "send-usage-claude-highbay");
        // The same again while pending: `-2`.
        let res = executor(said)
            .execute_tool("run_command", &run("send-usage-claude-highbay", "/usage"))
            .await
            .unwrap();
        assert_eq!(res["handle"], "send-usage-claude-highbay-2");
        // A handle not drawn from what the user said is replaced.
        let res = executor("continue claude please")
            .execute_tool("run_command", &run("restart-everything-now", "continue"))
            .await
            .unwrap();
        assert_eq!(res["handle"], "continue-claude-please");

        assert_eq!(log.confirmations.lock().unwrap().len(), 3);
        let principal = Principal::local("handlr");
        let (view, _) = broker
            .status(
                &principal,
                &StatusKey::Handle("continue-claude-please".to_string()),
            )
            .unwrap();
        assert_eq!(view.command, "continue");
        assert_eq!(
            broker.pending_handles(&principal.id),
            [
                "send-usage-claude-highbay",
                "send-usage-claude-highbay-2",
                "continue-claude-please"
            ]
        );
        assert!(match &client.transport() {
            libherdr::transport::HerdrTransport::Mock { state } =>
                state.read().await.command_log.is_empty(),
            _ => unreachable!(),
        });
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
