//! Transport layer for `flankr`, bridging to the `libherdr` crate and multiplexer backend.

use async_trait::async_trait;

pub use libherdr::{
    client::HerdrClient,
    error::{HerdrError, HerdrResult},
    protocol::{
        AgentSession, AgentStatus, DEFAULT_INSPECT_LINES, EnvironmentSnapshot, HerdrRequest,
        HerdrResponse, LayoutAction, PaneInfo, ReadSource, TabInfo, WorkspaceInfo,
    },
    transport::{
        HerdrTransport, MockState, RemoteSshConfig, SshInvocationMethod, default_local_socket_path,
    },
};

use crate::config::{AgentConfig, NamingConfig};
use crate::core::agents;
use crate::core::confirm::{self, Broker, ConfirmError, HandleRequest, Lifetime};
use crate::core::naming::Names;
use crate::core::policy;
use crate::core::principal::Principal;
use crate::llm::{PlanningConfig, PlanningPrompt};
use crate::project::{MAX_READ_BYTES, ProjectFiles};
use crate::sessions::context::{self as session_context, AgentContext};
use crate::sessions::{Sessions, pending};
use crate::watch::WatchRegistry;
use crate::watch_files::{FileWatcher, Subscription};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Lines of terminal output read for a pane in the turn context, with soft
/// wraps joined (plan 20261022_03, decision 7).
pub const CONTEXT_TAIL_LINES: usize = 80;

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
    /// What the user said in this turn, for the log only: the model's
    /// handle is kept as given (plan 20261022_08, 01:26 EDT amendment).
    pub utterance: Option<String>,
}

/// The conversation's current agent, a pane id (plan 20261027_04,
/// decision 3 and the 11:25 EDT amendment): the session open on the
/// phone, or the agent `focus_project` last picked. Shared by a
/// conversation's turns.
pub type CurrentAgent = Arc<Mutex<Option<String>>>;

/// The current agent's pane in `snapshot`.
fn current_ref(snapshot: &serde_json::Value, pane_id: &str) -> Option<agents::AgentRef> {
    agents::agents(snapshot)
        .into_iter()
        .find(|a| crate::watch::pane_ids_match(&a.pane_id, pane_id))
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
    /// When set, `focus_project` sets it and `send_agent_command` without
    /// an agent goes to it.
    pub current_agent: Option<CurrentAgent>,
    /// `focus_project` also moves herdr's focus (`client.options`
    /// `tab_follows_focus`).
    pub tab_follows_focus: bool,
    /// `[llm.uses.planning]` while it is on: what `list_files` and
    /// `read_file` may read (plan 20261027_04, decision 4).
    pub planning: Option<PlanningConfig>,
}

impl HerdrToolExecutor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self {
            client,
            agents,
            watches: None,
            principal: Principal::local("flankr"),
            naming: None,
            voice_turn: None,
            current_agent: None,
            tab_follows_focus: false,
            planning: None,
        }
    }

    pub fn with_planning(mut self, planning: PlanningConfig) -> Self {
        self.planning = Some(planning);
        self
    }

    /// The current project: the current agent's folder, opened with the
    /// `read` patterns.
    async fn current_project(&self) -> Result<ProjectFiles, String> {
        let read = match &self.planning {
            Some(planning) if planning.file_tools() => planning.read.clone(),
            _ => {
                return Err(
                    "Reading files is off: [llm.uses.planning] has no read patterns.".to_string(),
                );
            }
        };
        let cwd = self.current_folder().await?;
        tokio::task::spawn_blocking(move || ProjectFiles::open(&cwd, read))
            .await
            .map_err(|e| e.to_string())?
    }

    /// The current agent's folder, on this machine.
    async fn current_folder(&self) -> Result<String, String> {
        if matches!(self.client.transport(), HerdrTransport::RemoteSsh { .. }) {
            return Err(
                "The project's files are on another machine; flankr reads only its own."
                    .to_string(),
            );
        }
        let Some(pane_id) = self.current() else {
            return Err("No project is open: open a session, or focus an agent first".to_string());
        };
        let snapshot = self
            .client
            .list_panes()
            .await
            .map_err(|e| format!("herdr is not answering: {e}"))?;
        current_ref(&snapshot, &pane_id)
            .and_then(|a| a.cwd)
            .ok_or_else(|| {
                format!("The folder of the current agent (pane {pane_id}) is not known.")
            })
    }

    /// `recent_commits(count?, path?, files?)` (plan 20261027_05, decision
    /// 1, as amended at 12:37 EDT): the current project's last commits,
    /// newest first, with the `git` binary; `path` is checked as a
    /// message's paths are, and need not match `read`.
    async fn execute_recent_commits(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if self.planning.is_none() {
            return Err(
                "The project's history is available only while planning is on.".to_string(),
            );
        }
        let Some(git) = crate::git::binary() else {
            return Err("git is not installed where flankr runs.".to_string());
        };
        let count = crate::git::clamp_count(
            args.get("count")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))),
        );
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|p| !p.is_empty());
        let files = args.get("files").and_then(|v| v.as_bool()).unwrap_or(false);
        let root = self.current_folder().await?;
        let spec = match path {
            Some(path) => Some(crate::git::check_path(path, &root).inspect_err(|e| {
                log::info!("recent_commits in {root}: '{path}' refused: {e}");
            })?),
            None => None,
        };
        let history = crate::git::recent_commits(
            git,
            std::path::Path::new(&root),
            count,
            spec.as_deref(),
            files,
        )
        .await?;
        match history {
            crate::git::History::NotARepository => {
                log::info!("recent_commits in {root}: not a git work tree");
                Ok(serde_json::json!({
                    "project": root,
                    "commits": [],
                    "note": "The project has no git history: its folder is not a git work tree.",
                }))
            }
            crate::git::History::Commits(commits) => {
                log::info!(
                    "recent_commits in {root} (count {count}, path {path:?}, files {files}): {}",
                    commits.len()
                );
                let mut result = serde_json::json!({ "project": root, "commits": commits });
                if commits.is_empty() {
                    result["note"] = serde_json::json!(match path {
                        Some(_) => "No commits touch that path.",
                        None => "The project has no commits yet.",
                    });
                }
                Ok(result)
            }
        }
    }

    /// `list_files(glob?)` (decision 4): the readable files, sorted, at
    /// most 500.
    async fn execute_list_files(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let project = self.current_project().await?;
        let glob = args
            .get("glob")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let root = project.root().display().to_string();
        let asked = glob.clone();
        let listing = tokio::task::spawn_blocking(move || project.list(asked.as_deref()))
            .await
            .map_err(|e| e.to_string())??;
        log::info!(
            "list_files in {root} (glob {glob:?}): {} path(s), {} more",
            listing.paths.len(),
            listing.more
        );
        let mut result = serde_json::json!({ "project": root, "files": listing.paths });
        if listing.more > 0 {
            result["more"] = serde_json::json!(listing.more);
            result["note"] = serde_json::json!(format!(
                "{} more files match and are not listed; narrow the list with glob.",
                listing.more
            ));
        }
        Ok(result)
    }

    /// `read_file(path)` (decision 4): at most 64 KiB of one readable file.
    async fn execute_read_file(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if path.trim().is_empty() {
            return Err("'read_file' requires a 'path'.".to_string());
        }
        let project = self.current_project().await?;
        let root = project.root().display().to_string();
        let asked = path.clone();
        let read = tokio::task::spawn_blocking(move || project.read(&asked))
            .await
            .map_err(|e| e.to_string());
        let read = match read.and_then(|r| r) {
            Ok(read) => read,
            Err(e) => {
                log::info!("read_file in {root}: '{path}' refused: {e}");
                return Err(e);
            }
        };
        log::info!(
            "read_file in {root}: '{path}' ({} bytes{})",
            read.len,
            if read.cut { ", cut" } else { "" }
        );
        let mut result = serde_json::json!({ "path": path, "text": read.text });
        if read.cut {
            result["cut"] = serde_json::json!(true);
            result["note"] = serde_json::json!(format!(
                "Only the first {} KiB of {} bytes are shown.",
                MAX_READ_BYTES / 1024,
                read.len
            ));
        }
        Ok(result)
    }

    pub fn with_current_agent(mut self, current: CurrentAgent, tab_follows_focus: bool) -> Self {
        self.current_agent = Some(current);
        self.tab_follows_focus = tab_follows_focus;
        self
    }

    /// The current agent's pane id, if there is one.
    fn current(&self) -> Option<String> {
        self.current_agent
            .as_ref()
            .and_then(|c| c.lock().unwrap().clone())
    }

    /// `focus_project`: `agent` becomes the current agent; herdr's focus
    /// moves to it only with `tab_follows_focus`.
    async fn execute_focus_project(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let Some(current) = &self.current_agent else {
            return Err("Focusing a project is not available in this session.".to_string());
        };
        let requested = args
            .get("agent")
            .or_else(|| args.get("target_id"))
            .or_else(|| args.get("pane_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if requested.is_empty() {
            return Err("'focus_project' requires an 'agent'.".to_string());
        }
        let snapshot = self
            .client
            .list_panes()
            .await
            .map_err(|e| format!("herdr is not answering: {e}"))?;
        let pane_id = self.resolve(&snapshot, requested)?;
        *current.lock().unwrap() = Some(pane_id.clone());
        self.addressed(&pane_id);
        let agent = current_ref(&snapshot, &pane_id);
        let moved = if self.tab_follows_focus {
            self.client.focus(&pane_id).await.map_err(|e| {
                format!("The project is current, but herdr's focus did not move: {e}")
            })?;
            true
        } else {
            false
        };
        log::info!("focus_project: '{pane_id}' is current (herdr focus moved: {moved})");
        Ok(serde_json::json!({
            "current_agent": pane_id,
            "agent": agent.as_ref().map(|a| a.agent.clone()),
            "project": agent.as_ref().and_then(|a| a.project.clone()),
            "herdr_focus_moved": moved,
        }))
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

    /// `send_agent_command` (plan 20261027_04, decision 1), also called
    /// as `run_command` or `send_command`: allow-listed text checked by
    /// `policy::check` with the agent's `require_confirmation`, any other
    /// text by `policy::check_message`, always confirmed. Then a send, a
    /// voice-turn confirmation, or a refusal.
    async fn execute_send_agent_command(
        &self,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let text = args
            .get("text")
            .or_else(|| args.get("command"))
            .or_else(|| args.get("cmd"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let named = args
            .get("agent")
            .or_else(|| args.get("pane_id"))
            .or_else(|| args.get("target_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string);
        // Left out, the current agent (the 11:25 EDT amendment, item 2).
        let Some(requested) = named.or_else(|| self.current()) else {
            return Err("Which agent? Name one, or focus a project first".to_string());
        };
        let requested = requested.as_str();

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

        let message = !policy::is_allow_listed(&self.agents, &agent_name, text);
        log::info!(
            "Permission check: agent='{}' pane='{}' {}='{}'",
            agent_name,
            pane_id,
            if message { "message" } else { "command" },
            text
        );

        let checked = if message {
            // Paths in the text are checked against the agent's project.
            let root = snapshot
                .as_ref()
                .and_then(|snap| current_ref(snap, &pane_id))
                .and_then(|a| a.cwd);
            policy::check_message(
                &self.principal,
                &self.agents,
                &agent_name,
                &pane_id,
                text,
                root.as_deref(),
            )
        } else {
            policy::check(&self.principal, &self.agents, &agent_name, &pane_id, text)
        };
        let checked = match checked {
            Ok(checked) => checked,
            Err(denied) => {
                log::warn!(
                    "Refused '{}' for agent '{}' on pane '{}': {}",
                    text,
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
            // A message's handle, when the model gives none: "message" and
            // its first words, as for one typed in the app.
            let fallback = if message {
                let first_words: Vec<&str> = final_cmd.split(' ').take(3).collect();
                format!("message {}", first_words.join(" "))
            } else {
                final_cmd.clone()
            };
            let request = HandleRequest {
                given: args.get("handle").and_then(|v| v.as_str()),
                command: &fallback,
                agent: &spoken,
                tab_code: tab_code.as_deref(),
            };
            // A handle taken between settling it and asking is settled again.
            let mut attempt = 0;
            let priors = turn.broker.word_priors();
            let (confirm_id, handle) = loop {
                let pending = turn.broker.pending_handle_infos(&self.principal.id);
                let settled =
                    confirm::settle_handle(&request, &pending, &priors, confirm::random_word);
                let opened = if message {
                    turn.broker.request_message(
                        &self.principal,
                        &pane_id,
                        &agent_name,
                        &final_cmd,
                        Some(&settled),
                        Lifetime::VoiceTurn,
                    )
                } else {
                    turn.broker.request_with(
                        &self.principal,
                        &pane_id,
                        &agent_name,
                        &final_cmd,
                        Some(&settled),
                        Lifetime::VoiceTurn,
                    )
                };
                match opened {
                    Ok(id) => break (id, settled.handle),
                    Err(ConfirmError::DuplicateHandle(_)) if attempt < 3 => attempt += 1,
                    Err(e) => return Err(e.to_string()),
                }
            };
            log::info!(
                "Confirmation {} handle '{}' (given {:?}; heard {:?})",
                confirm_id,
                handle,
                request.given,
                turn.utterance
            );
            turn.log.confirmations.lock().unwrap().push(confirm_id);
            return Ok(serde_json::json!({
                "status": "awaiting_user_approval",
                "command": final_cmd,
                "agent": spoken,
                "handle": handle,
                "note": format!(
                    "Not sent yet. Right after your reply, flankr itself asks the user to approve sending '{final_cmd}' to {spoken}, and sends it only if they approve. Do not ask for confirmation yourself and do not call send_agent_command again for this; say in a few words that it is waiting for their approval."
                ),
            }));
        }

        // What reaches herdr is unchanged: its `run_command`, the text and
        // Enter.
        let final_args = serde_json::json!({ "pane_id": pane_id, "command": final_cmd });
        let result = self
            .client
            .execute_tool_call("run_command", &final_args)
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
            crate::reasoning::SEND_AGENT_COMMAND | "run_command" | "send_command" => {
                self.execute_send_agent_command(args).await
            }
            crate::reasoning::FOCUS_PROJECT => self.execute_focus_project(args).await,
            crate::reasoning::LIST_FILES => self.execute_list_files(args).await,
            crate::reasoning::READ_FILE => self.execute_read_file(args).await,
            crate::reasoning::RECENT_COMMITS => self.execute_recent_commits(args).await,
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
    /// When set, the context has an `### Agent sessions` section read from
    /// the agents' own transcripts (plan 20261022_03, decision 7).
    pub sessions: Option<Arc<Sessions>>,
    /// When set, the context names the conversation's current agent (plan
    /// 20261027_04, the 11:25 EDT amendment).
    pub current_agent: Option<CurrentAgent>,
    /// `[llm.uses.planning]` and its prompt while it is on: with
    /// `agents_prompt`, the current project's instructions are added
    /// (decisions 5 and 6).
    pub planning: Option<(PlanningConfig, PlanningPrompt)>,
    /// The conversation's instructions as last read, and the file watcher
    /// that says when to read them again (plan 20261027_05, decision 3).
    /// Without it, they are read every turn.
    pub instructions: Option<(Arc<InstructionsCache>, Option<FileWatcher>)>,
    /// Messages this contributor added that belong in the stored history:
    /// the instructions block, when it was sent.
    added: Mutex<Vec<libinfer::Message>>,
}

/// The first line of an instructions block in the stored history (plan
/// 20261027_05, 12:33 EDT amendment, item 3):
/// `[flankr instructions <project> <sha256, first 16 hex>]`.
pub const INSTRUCTIONS_MARKER: &str = "[flankr instructions ";

/// The marker for `project`'s instructions `text`.
pub fn instructions_marker(project: &str, text: &str) -> String {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(text.as_bytes());
    let hex: String = hash.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{INSTRUCTIONS_MARKER}{project} {hex}]")
}

/// A marker line's project and hash.
fn parse_marker(line: &str) -> Option<(&str, &str)> {
    line.strip_prefix(INSTRUCTIONS_MARKER)?
        .strip_suffix(']')?
        .rsplit_once(' ')
}

/// Whether `history` already gives `marker`'s project these instructions:
/// the last instructions block for that project is this one.
pub fn history_holds(history: &[libinfer::Message], marker: &str) -> bool {
    let Some((project, _)) = parse_marker(marker) else {
        return false;
    };
    history
        .iter()
        .rev()
        .filter_map(|m| m.content.lines().next())
        .find(|line| parse_marker(line).is_some_and(|(p, _)| p == project))
        .is_some_and(|line| line == marker)
}

/// One conversation's instructions as last read (plan 20261027_05,
/// decision 3): read again when the current project's folder changes, or
/// when the file watcher reports a change to one of its instruction files.
#[derive(Default)]
pub struct InstructionsCache {
    state: Mutex<Option<CachedInstructions>>,
}

struct CachedInstructions {
    cwd: String,
    /// The block, its marker first; `None` without an instructions file.
    block: Option<String>,
    watch: Option<Subscription>,
    /// The watch's ticks when the block was read.
    seen: u64,
}

impl InstructionsCache {
    /// Whether the current project's instruction files are watched.
    pub fn is_watched(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|c| c.watch.is_some())
    }
}

impl HerdrContextContributor {
    pub fn new(client: HerdrClient, agents: Vec<AgentConfig>) -> Self {
        Self {
            client,
            agents,
            watches: None,
            naming: None,
            sessions: None,
            current_agent: None,
            planning: None,
            instructions: None,
            added: Mutex::new(Vec::new()),
        }
    }

    pub fn with_planning(mut self, config: PlanningConfig, prompt: PlanningPrompt) -> Self {
        self.planning = Some((config, prompt));
        self
    }

    /// The conversation's instructions cache, and the file watcher that
    /// tells it when to read again.
    pub fn with_instructions(
        mut self,
        cache: Arc<InstructionsCache>,
        files: Option<FileWatcher>,
    ) -> Self {
        self.instructions = Some((cache, files));
        self
    }

    /// The messages added to the conversation that belong in its stored
    /// history, taken.
    pub fn take_added(&self) -> Vec<libinfer::Message> {
        std::mem::take(&mut *self.added.lock().unwrap())
    }

    /// The `instructions` block for the current project (plan 20261027_04,
    /// decision 6, item 3), its marker first, with `agents_prompt` on and
    /// an instructions file found; from the cache while the project and
    /// its files are unchanged (plan 20261027_05, decision 3).
    fn instructions_block(&self, snapshot: &serde_json::Value) -> Option<String> {
        let (config, prompt) = self.planning.as_ref()?;
        if !config.agents_prompt
            || matches!(self.client.transport(), HerdrTransport::RemoteSsh { .. })
        {
            return None;
        }
        let pane_id = self.current_agent.as_ref()?.lock().unwrap().clone()?;
        let agent = current_ref(snapshot, &pane_id)?;
        let cwd = agent.cwd.clone()?;
        let Some((cache, files)) = &self.instructions else {
            return Self::read_instructions(&agent, &cwd, prompt);
        };
        let mut state = cache.state.lock().unwrap();
        if let Some(c) = state.as_ref()
            && c.cwd == cwd
            && c.watch.as_ref().is_some_and(|w| w.ticks() == c.seen)
        {
            return c.block.clone();
        }
        // The same folder keeps its watch; another one is watched anew,
        // before reading, so no change is missed.
        let watch = match state.take() {
            Some(c) if c.cwd == cwd && c.watch.is_some() => c.watch,
            _ => files.as_ref().and_then(|files| {
                let paths: Vec<std::path::PathBuf> = crate::project::INSTRUCTION_FILES
                    .iter()
                    .map(|name| std::path::Path::new(&cwd).join(name))
                    .collect();
                files
                    .subscribe(&paths)
                    .inspect_err(|e| log::warn!("agents_prompt: can't watch {cwd}: {e}"))
                    .ok()
            }),
        };
        let seen = watch.as_ref().map_or(0, |w| w.ticks());
        let block = Self::read_instructions(&agent, &cwd, prompt);
        *state = Some(CachedInstructions {
            cwd,
            block: block.clone(),
            watch,
            seen,
        });
        block
    }

    /// The block read from `cwd`'s instructions file, its marker first.
    fn read_instructions(
        agent: &agents::AgentRef,
        cwd: &str,
        prompt: &PlanningPrompt,
    ) -> Option<String> {
        let files = ProjectFiles::open(cwd, Vec::new())
            .inspect_err(|e| log::warn!("agents_prompt: {e}"))
            .ok()?;
        let max = prompt.max_instructions_bytes();
        let (file, mut text, cut) = files.instructions(max)?;
        let project = agent.project.clone().unwrap_or_else(|| {
            std::path::Path::new(cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(cwd.to_string())
        });
        let marker = instructions_marker(&project, &text);
        if cut {
            text.push_str(&format!("\n\n(cut at {} KiB)", max / 1024));
        }
        log::info!("agents_prompt: {file} of {} read", files.root().display());
        prompt
            .instructions(
                &project,
                &crate::core::naming::capitalise(&agent.agent),
                &file,
                &text,
            )
            .inspect_err(|e| log::warn!("agents_prompt: {e}"))
            .ok()
            .map(|block| format!("{marker}\n{block}"))
    }

    pub fn with_current_agent(mut self, current: CurrentAgent) -> Self {
        self.current_agent = Some(current);
        self
    }

    /// The `### Current agent` section.
    fn current_agent_section(&self, snapshot: &serde_json::Value) -> Option<String> {
        let current = self.current_agent.as_ref()?.lock().unwrap().clone();
        let mut section = String::from("### Current agent\n");
        let Some(pane_id) = current else {
            section.push_str(
                "None: name the agent in `send_agent_command`, or call `focus_project` when the \
                 user picks an agent or a project.\n\n",
            );
            return Some(section);
        };
        let Some(agent) = current_ref(snapshot, &pane_id) else {
            section.push_str(&format!(
                "Pane {pane_id}, which herdr no longer has. Ask which agent the user means.\n\n"
            ));
            return Some(section);
        };
        let spoken = self
            .naming
            .as_ref()
            .and_then(|naming| {
                Names::from_snapshot(snapshot, naming)
                    .get(&agent.pane_id)
                    .map(|n| n.spoken.clone())
            })
            .unwrap_or_else(|| crate::core::naming::capitalise(&agent.agent));
        section.push_str(&format!(
            "Current agent: {spoken} (pane {}, project {}). The user means this agent \
             unless they name another; `send_agent_command` without `agent` goes to it, and \
             `focus_project` changes it. File paths for it are relative to its project's \
             root.\n\n",
            agent.pane_id,
            agent.project.as_deref().unwrap_or("unknown"),
        ));
        Some(section)
    }

    pub fn with_sessions(mut self, sessions: Arc<Sessions>) -> Self {
        self.sessions = Some(sessions);
        self
    }

    /// The `### Agent sessions` section and the panes it covers (decision
    /// 7): the focused agent, the agents `utterance` names, and any agent
    /// with a pending question, each from its own transcript. Empty when
    /// there are no sessions, or herdr is remote (decision 4: session files
    /// are only read on this machine).
    pub fn session_section(
        &self,
        snapshot: &serde_json::Value,
        utterance: Option<&str>,
    ) -> (String, HashSet<String>) {
        let Some(sessions) = &self.sessions else {
            return (String::new(), HashSet::new());
        };
        if matches!(self.client.transport(), HerdrTransport::RemoteSsh { .. }) {
            return (String::new(), HashSet::new());
        }
        let naming = self.naming.clone().unwrap_or_default();
        let names = Names::from_snapshot(snapshot, &naming);
        let focused = agents::focused_pane_id(snapshot);
        let named: HashSet<String> = utterance
            .map(|u| {
                names
                    .named_in(u)
                    .into_iter()
                    .map(|a| a.agent.pane_id.clone())
                    .collect()
            })
            .unwrap_or_default();

        // (priority, block): focused, then named, then pending only.
        let mut blocks: Vec<(u8, AgentContext)> = Vec::new();
        for named_agent in &names.agents {
            let agent = &named_agent.agent;
            if sessions.reader_for(&agent.agent).is_none() {
                continue;
            }
            let is_focused = focused
                .as_deref()
                .is_some_and(|f| crate::watch::pane_ids_match(&agent.pane_id, f));
            let is_named = named.contains(&agent.pane_id);
            let may_ask = matches!(agent.status, AgentStatus::Idle | AgentStatus::Blocked);
            if !(is_focused || is_named || may_ask) {
                continue;
            }
            let Some(read) = sessions.read(agent, |items| {
                items.iter().filter(|i| i.is_text()).count() >= session_context::ENTRIES_PER_AGENT
            }) else {
                continue;
            };
            let question =
                pending::pending_question(&read.items, &agent.status, read.located.quiet());
            let priority = match (is_focused, is_named) {
                (true, _) => 0,
                (false, true) => 1,
                _ if question.is_some() => 2,
                _ => continue,
            };
            let status = serde_json::to_value(&agent.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            blocks.push((
                priority,
                AgentContext::new(
                    &crate::core::naming::capitalise(&named_agent.spoken),
                    &agent.pane_id,
                    &status,
                    &read.items,
                    question,
                ),
            ));
        }
        blocks.sort_by_key(|(priority, _)| *priority);
        let covered = blocks.iter().map(|(_, b)| b.pane_id.clone()).collect();
        let section = session_context::section(blocks.into_iter().map(|(_, b)| b).collect());
        (section, covered)
    }

    /// The terminal tail of `pane_id` for the context: `recent_unwrapped`,
    /// `CONTEXT_TAIL_LINES` lines.
    async fn pane_tail(&self, pane_id: &str) -> Option<String> {
        let read = self
            .client
            .inspect_pane_from(pane_id, CONTEXT_TAIL_LINES, ReadSource::RecentUnwrapped)
            .await
            .ok()?;
        agents::pane_text(&read).map(str::to_string)
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

        // The agents' own transcripts. A pane they cover needs no terminal
        // tail, unless it is blocked: what blocks it (a permission prompt)
        // is on its screen, not in its transcript.
        let (sessions_section, covered) = self.session_section(&snapshot, last_user_query);
        let tail_wanted = |pane_id: &str, status: &str| {
            status == "blocked"
                || !covered
                    .iter()
                    .any(|c| crate::watch::pane_ids_match(c, pane_id))
        };

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

                if should_include
                    && !pane_id.is_empty()
                    && tail_wanted(pane_id, status)
                    && let Some(buffer) = self.pane_tail(pane_id).await
                {
                    let trimmed = buffer.trim();
                    if !trimmed.is_empty() {
                        pane_details.push(format!(
                                "#### Pane {} (ID: {}) [Agent: {}, Status: {}, Focused: {}, CWD: '{}']\nRecent {} Lines of Output:\n```\n{}\n```",
                                idx + 1, pane_id, agent, status, focused, cwd, CONTEXT_TAIL_LINES, trimmed
                            ));
                    } else {
                        pane_details.push(format!(
                                "#### Pane {} (ID: {}) [Agent: {}, Status: {}, Focused: {}, CWD: '{}']\n(Buffer is idle/empty)",
                                idx + 1, pane_id, agent, status, focused, cwd
                            ));
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

                                    if should_include
                                        && tail_wanted(pane_id, status)
                                        && let Some(buffer) = self.pane_tail(pane_id).await
                                    {
                                        let trimmed = buffer.trim();
                                        if !trimmed.is_empty() {
                                            pane_details.push(format!(
                                                    "#### Pane {} (ID: {}) [Workspace: {}, Title: '{}', Status: {}, Focused: {}]:\nRecent {} Lines of Output:\n```\n{}\n```",
                                                    global_idx, pane_id, ws_name, title, status, focused, CONTEXT_TAIL_LINES, trimmed
                                                ));
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
            focused_ws_id,
            focused_pane_id,
            pane_summaries.join("\n")
        );

        if !pane_details.is_empty() {
            for detail in pane_details {
                content.push_str(&detail);
                content.push_str("\n\n");
            }
        } else {
            content.push_str("No active/working terminal pane output to report.\n\n");
        }

        if !sessions_section.is_empty() {
            content.push_str(&sessions_section);
            content.push('\n');
        }

        if !self.agents.is_empty() {
            content.push_str("### Configured Agents & Permitted Commands\n");
            for agent in &self.agents {
                content.push_str(&format!(
                    "- Agent '{}': Allowed commands: {:?}, Requests in words: {}, Requires Confirmation: {}\n",
                    agent.name, agent.allowed_commands, agent.allow_messages, agent.require_confirmation
                ));
            }
            // Plan 20261027_04, decision 1: text that is not an allowed
            // command goes as a message, checked as one typed in the app.
            if self.agents.iter().any(|a| a.allow_messages) {
                content.push_str(&format!(
                    "With `send_agent_command`, an allowed command is typed as it is. Any other text \
                     is sent to an agent that takes requests in words as a request, checked as a \
                     message typed in the app: one line, at most {} characters, not starting with \
                     '/', '!' or '#' (only allowed commands may). Paths and flags are fine inside \
                     it, but every path must be inside that agent's project, at most {} '..' \
                     deep. The user always approves a request before it is sent.\n",
                    crate::commands::MAX_MESSAGE_LEN,
                    crate::commands::MAX_PARENT_PARTS
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

        // The current agent last, so the parts that vary stay at the end.
        if let Some(section) = self.current_agent_section(&snapshot) {
            content.push_str(&section);
        }

        // The instructions go into the conversation itself, before the
        // user's turn, unless the stored history already holds them (plan
        // 20261027_05, decision 3, as amended at 12:33 EDT).
        if let Some(block) = self.instructions_block(&snapshot) {
            let marker = block.lines().next().unwrap_or_default();
            let at = match messages.last() {
                Some(m) if m.role == "user" => messages.len() - 1,
                _ => messages.len(),
            };
            if !history_holds(&messages[..at], marker) {
                log::info!("agents_prompt: {marker} sent");
                let message = libinfer::Message {
                    role: "system".to_string(),
                    content: block,
                    name: None,
                    thought_signature: None,
                };
                messages.insert(at, message.clone());
                self.added.lock().unwrap().push(message);
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
        if let Some(ord) = ordinal
            && (q.contains(&format!("{} pane", ord)) || q.contains(&format!("{} pain", ord)))
        {
            return true;
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
        if !cwd.is_empty()
            && let Some(dir_name) = std::path::Path::new(cwd)
                .file_name()
                .and_then(|f| f.to_str())
            && q.contains(&dir_name.to_lowercase())
        {
            return true;
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

    async fn command_log(client: &HerdrClient) -> Vec<(String, String)> {
        match client.transport() {
            HerdrTransport::Mock { state } => state.read().await.command_log.clone(),
            _ => unreachable!(),
        }
    }

    fn voice_executor(
        client: &HerdrClient,
        broker: &Arc<Broker>,
        log: &Arc<TurnLog>,
        agents: Vec<AgentConfig>,
    ) -> HerdrToolExecutor {
        HerdrToolExecutor::new(client.clone(), agents).with_voice_turn(VoiceTurn {
            broker: Arc::clone(broker),
            log: Arc::clone(log),
            utterance: None,
        })
    }

    #[tokio::test]
    async fn test_allow_listed_commands_follow_require_confirmation() {
        use crate::core::confirm::{DEFAULT_TTL, StatusKey};
        let client = HerdrClient::mock();
        let broker = Broker::new(Arc::new(client.clone()), DEFAULT_TTL);
        let log = Arc::new(TurnLog::default());

        // Not required: sent at once, through herdr's run_command.
        let executor = voice_executor(
            &client,
            &broker,
            &log,
            vec![AgentConfig::new(
                "claude",
                vec!["continue".to_string(), "/compact".to_string()],
                false,
            )],
        );
        executor
            .execute_tool(
                "send_agent_command",
                &json!({ "agent": "p1", "text": "/COMPACT", "handle": "send-compact-claude" }),
            )
            .await
            .unwrap();
        assert_eq!(
            command_log(&client).await,
            [("p1".to_string(), "/COMPACT".to_string())]
        );
        assert!(log.confirmations.lock().unwrap().is_empty());

        // Required: a command confirmation, not a message.
        let executor = voice_executor(
            &client,
            &broker,
            &log,
            vec![AgentConfig::new(
                "claude",
                vec!["continue".to_string()],
                true,
            )],
        );
        let res = executor
            .execute_tool(
                "send_agent_command",
                &json!({ "agent": "p1", "text": "continue", "handle": "send-continue-claude" }),
            )
            .await
            .unwrap();
        assert_eq!(res["status"], "awaiting_user_approval");
        let (view, _) = broker
            .status(
                &Principal::local("flankr"),
                &StatusKey::Handle("send-continue-claude".to_string()),
            )
            .unwrap();
        assert_eq!(view.command, "continue");
        assert!(!view.message);
        assert_eq!(command_log(&client).await.len(), 1, "not sent");
    }

    #[tokio::test]
    async fn test_free_text_is_a_message_always_awaiting_approval() {
        use crate::core::confirm::{DEFAULT_TTL, StatusKey};
        let client = HerdrClient::mock();
        let broker = Broker::new(Arc::new(client.clone()), DEFAULT_TTL);
        let log = Arc::new(TurnLog::default());
        // The agent requires no confirmation for its commands.
        let executor = voice_executor(
            &client,
            &broker,
            &log,
            vec![AgentConfig::new(
                "claude",
                vec!["continue".to_string()],
                false,
            )],
        );
        // Longer than the old voice check's 256 characters.
        let text = format!(
            "Read the general agent planning plan in the design docs, then {}",
            "add the tests its fourth step asks for and run them. ".repeat(6)
        );
        let text = text.trim_end().trim_end_matches('.').to_string();
        assert!(text.len() > crate::commands::MAX_ARBITRARY_COMMAND_LEN);
        let res = executor
            .execute_tool(
                "send_agent_command",
                &json!({ "agent": "p1", "text": text, "handle": "ask-claude-tests" }),
            )
            .await
            .unwrap();
        assert_eq!(res["status"], "awaiting_user_approval");
        let (view, _) = broker
            .status(
                &Principal::local("flankr"),
                &StatusKey::Handle("ask-claude-tests".to_string()),
            )
            .unwrap();
        assert_eq!(view.command, text);
        assert!(view.message);
        assert!(command_log(&client).await.is_empty());

        // What validate_message and check_paths refuse, as for a message
        // typed in the app: a command first that isn't allowed, and a path
        // out of the project.
        for text in [
            "/status please",
            "  !ls",
            "read ../../../../../../etc/passwd",
        ] {
            let err = executor
                .execute_tool(
                    "send_agent_command",
                    &json!({ "agent": "p1", "text": text, "handle": "h2" }),
                )
                .await
                .unwrap_err();
            assert!(err.contains("Message rejected"), "{text}: {err}");
        }

        // Outside a companion turn there is no way to approve it.
        let res = HerdrToolExecutor::new(
            client.clone(),
            vec![AgentConfig::new("claude", vec![], false)],
        )
        .execute_tool(
            "send_agent_command",
            &json!({ "agent": "p1", "text": "please run the tests", "handle": "h" }),
        )
        .await;
        assert!(res.unwrap_err().contains("confirmation unavailable"));
    }

    #[tokio::test]
    async fn test_free_text_is_refused_without_allow_messages() {
        let client = HerdrClient::mock();
        let broker = Broker::new(Arc::new(client.clone()), crate::core::confirm::DEFAULT_TTL);
        let log = Arc::new(TurnLog::default());
        let executor = voice_executor(
            &client,
            &broker,
            &log,
            vec![
                AgentConfig::new("claude", vec!["continue".to_string()], false)
                    .with_arbitrary_commands(true)
                    .with_messages(false),
            ],
        );
        let err = executor
            .execute_tool(
                "send_agent_command",
                &json!({ "agent": "p1", "text": "please run the tests", "handle": "h" }),
            )
            .await
            .unwrap_err();
        assert!(err.contains("allow_messages is off"), "{err}");
        // The allow list still works.
        executor
            .execute_tool(
                "send_agent_command",
                &json!({ "agent": "p1", "text": "continue", "handle": "h" }),
            )
            .await
            .unwrap();
        assert!(log.confirmations.lock().unwrap().is_empty());
        assert_eq!(command_log(&client).await.len(), 1);
        // An unconfigured agent is refused either way.
        let err = HerdrToolExecutor::new(client.clone(), vec![])
            .execute_tool(
                "send_agent_command",
                &json!({ "agent": "p1", "text": "continue", "handle": "h" }),
            )
            .await
            .unwrap_err();
        assert!(err.contains("not configured"), "{err}");
    }

    async fn focused_target(client: &HerdrClient) -> Option<String> {
        match client.transport() {
            HerdrTransport::Mock { state } => state.read().await.focused_target.clone(),
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn test_focus_project_sets_the_current_agent_and_herdr_focus_only_when_asked() {
        let client = session_mock().await;
        if let HerdrTransport::Mock { state } = client.transport() {
            state.write().await.focused_target = None;
        }
        let current = CurrentAgent::default();
        let agents = vec![AgentConfig::new(
            "claude",
            vec!["continue".to_string()],
            false,
        )];
        let executor = HerdrToolExecutor::new(client.clone(), agents.clone())
            .with_current_agent(Arc::clone(&current), false);

        // Without a current agent, an unnamed send is refused.
        let err = executor
            .execute_tool(
                "send_agent_command",
                &json!({ "text": "continue", "handle": "h" }),
            )
            .await
            .unwrap_err();
        assert_eq!(err, "Which agent? Name one, or focus a project first");

        let res = executor
            .execute_tool("focus_project", &json!({ "agent": "p1" }))
            .await
            .unwrap();
        assert_eq!(res["current_agent"], "p1");
        assert_eq!(res["herdr_focus_moved"], false);
        assert_eq!(current.lock().unwrap().as_deref(), Some("p1"));
        assert_eq!(focused_target(&client).await, None, "herdr left alone");

        // Now an unnamed send goes to it.
        executor
            .execute_tool(
                "send_agent_command",
                &json!({ "text": "continue", "handle": "h" }),
            )
            .await
            .unwrap();
        assert_eq!(
            command_log(&client).await,
            [("p1".to_string(), "continue".to_string())]
        );

        // With tab_follows_focus, herdr's focus moves too.
        let following = HerdrToolExecutor::new(client.clone(), agents)
            .with_current_agent(Arc::clone(&current), true);
        let res = following
            .execute_tool("focus_project", &json!({ "agent": "p2" }))
            .await
            .unwrap();
        assert_eq!(res["herdr_focus_moved"], true);
        assert_eq!(current.lock().unwrap().as_deref(), Some("p2"));
        assert_eq!(focused_target(&client).await.as_deref(), Some("p2"));

        // Outside a conversation there is no current agent to set.
        let err = HerdrToolExecutor::new(client.clone(), vec![])
            .execute_tool("focus_project", &json!({ "agent": "p1" }))
            .await
            .unwrap_err();
        assert!(err.contains("not available"), "{err}");
    }

    #[tokio::test]
    async fn test_the_context_names_the_current_agent() {
        let client = session_mock().await;
        let current = CurrentAgent::default();
        let contributor = HerdrContextContributor::new(client.clone(), vec![])
            .with_naming(NamingConfig::default())
            .with_current_agent(Arc::clone(&current));
        let content = context_for(&contributor, "hello").await;
        assert!(content.contains("### Current agent\nNone:"), "{content}");

        *current.lock().unwrap() = Some("p1".to_string());
        let content = context_for(&contributor, "hello").await;
        let start = content.find("### Current agent").unwrap();
        let section = &content[start..];
        assert!(section.contains("Current agent: Claude"), "{section}");
        assert!(section.contains("pane p1"), "{section}");
        assert!(section.contains("project "), "{section}");
        assert!(
            !section.contains("/work/example"),
            "no absolute folder: {section}"
        );

        // Without one kept, there is no section.
        let plain = HerdrContextContributor::new(client, vec![]);
        assert!(
            !context_for(&plain, "hello")
                .await
                .contains("### Current agent")
        );
    }

    /// Two projects, `a` (p1's) and `b` (p2's), under one folder that
    /// also holds `x.md`.
    fn two_projects() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, text: &str| {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        for rel in [
            "a/README.md",
            "a/docs/design/x.md",
            "a/docs/x.txt",
            "a/src/README.md",
            "a/src/main.rs",
            "a/.env",
            "a/.private/key",
        ] {
            write(rel, rel);
        }
        write("a/AGENTS.md", "Agents rules for a.");
        write("a/CLAUDE.md", "Claude rules for a.");
        write("b/NOTES.md", "b's notes");
        write("x.md", "outside");
        std::os::unix::fs::symlink(dir.path().join("x.md"), dir.path().join("a/docs/out.md"))
            .unwrap();
        dir
    }

    /// The session mock with p1 in `a` and p2 in `b`.
    async fn project_mock(dir: &std::path::Path) -> HerdrClient {
        let client = session_mock().await;
        if let HerdrTransport::Mock { state } = client.transport() {
            let mut state = state.write().await;
            let panes = &mut state.snapshot.workspaces[0].tabs[0].panes;
            panes[0].cwd = Some(dir.join("a").display().to_string());
            panes[1].cwd = Some(dir.join("b").display().to_string());
        }
        client
    }

    fn planning(read: &[&str], agents_prompt: bool) -> PlanningConfig {
        PlanningConfig::from_use(&crate::config::LlmUseConfig {
            agents_prompt: Some(agents_prompt),
            read: Some(read.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        })
        .0
    }

    /// Plan 20261027_05, step 3: `recent_commits` on the current project,
    /// with no `read` patterns; a path outside refused; a folder that
    /// isn't a work tree answered plainly.
    #[tokio::test]
    async fn test_recent_commits_reads_the_current_projects_history() {
        let dir = two_projects();
        let a = dir.path().join("a");
        let git = |args: &[&str]| {
            let ok = std::process::Command::new(crate::git::binary().unwrap())
                .arg("-C")
                .arg(&a)
                .args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_AUTHOR_NAME", "Ada")
                .env("GIT_AUTHOR_EMAIL", "ada@example.com")
                .env("GIT_COMMITTER_NAME", "Ada")
                .env("GIT_COMMITTER_EMAIL", "ada@example.com")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap()
                .success();
            assert!(ok, "{args:?}");
        };
        git(&["init", "-q"]);
        git(&["add", "README.md"]);
        git(&["commit", "-q", "-m", "first: the readme"]);
        git(&["add", "docs"]);
        git(&["commit", "-q", "-m", "second: the docs"]);

        let client = project_mock(dir.path()).await;
        let current = CurrentAgent::default();
        let executor = HerdrToolExecutor::new(client.clone(), vec![])
            .with_current_agent(Arc::clone(&current), false)
            .with_planning(planning(&[], false));
        *current.lock().unwrap() = Some("p1".to_string());

        let r = executor
            .execute_tool("recent_commits", &json!({}))
            .await
            .unwrap();
        let subjects: Vec<&str> = r["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["subject"].as_str().unwrap())
            .collect();
        assert_eq!(subjects, ["second: the docs", "first: the readme"]);
        assert!(r["commits"][0].get("files").is_none(), "{r}");

        let r = executor
            .execute_tool(
                "recent_commits",
                &json!({ "count": 1, "path": "README.md", "files": true }),
            )
            .await
            .unwrap();
        assert_eq!(r["commits"].as_array().unwrap().len(), 1, "{r}");
        assert_eq!(r["commits"][0]["subject"], "first: the readme");
        assert_eq!(r["commits"][0]["files"], json!(["README.md"]));

        for path in ["../b", "/../b", "~/x"] {
            let e = executor
                .execute_tool("recent_commits", &json!({ "path": path }))
                .await
                .unwrap_err();
            assert!(e.contains("outside the project"), "{path}: {e}");
        }

        *current.lock().unwrap() = Some("p2".to_string());
        let r = executor
            .execute_tool("recent_commits", &json!({}))
            .await
            .unwrap();
        assert_eq!(r["commits"], json!([]));
        assert!(
            r["note"].as_str().unwrap().contains("no git history"),
            "{r}"
        );

        // Without planning, refused.
        let off =
            HerdrToolExecutor::new(client, vec![]).with_current_agent(Arc::clone(&current), false);
        assert!(
            off.execute_tool("recent_commits", &json!({}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_the_file_tools_read_the_current_project_only() {
        let dir = two_projects();
        let client = project_mock(dir.path()).await;
        let current = CurrentAgent::default();
        let executor = HerdrToolExecutor::new(client.clone(), vec![])
            .with_current_agent(Arc::clone(&current), false)
            .with_planning(planning(&["*.md", "docs/**/*.md"], false));

        // No current agent: refused.
        let err = executor
            .execute_tool("list_files", &json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            "No project is open: open a session, or focus an agent first"
        );

        *current.lock().unwrap() = Some("p1".to_string());
        let listed = executor
            .execute_tool("list_files", &json!({}))
            .await
            .unwrap();
        assert_eq!(
            listed["files"],
            json!(["AGENTS.md", "CLAUDE.md", "README.md", "docs/design/x.md"])
        );
        assert!(listed.get("more").is_none());
        let narrowed = executor
            .execute_tool("list_files", &json!({ "glob": "docs/**" }))
            .await
            .unwrap();
        assert_eq!(narrowed["files"], json!(["docs/design/x.md"]));

        for path in ["README.md", "docs/design/x.md"] {
            let read = executor
                .execute_tool("read_file", &json!({ "path": path }))
                .await
                .unwrap();
            assert_eq!(read["text"], format!("a/{path}"));
        }
        for path in [
            "docs/x.txt",
            "src/README.md",
            "src/main.rs",
            ".env",
            ".private/key",
            "../x.md",
            "docs/out.md",
        ] {
            let err = executor
                .execute_tool("read_file", &json!({ "path": path }))
                .await
                .unwrap_err();
            assert!(err.contains("may not be read"), "{path}: {err}");
        }

        // focus_project moves the project.
        executor
            .execute_tool("focus_project", &json!({ "agent": "p2" }))
            .await
            .unwrap();
        let listed = executor
            .execute_tool("list_files", &json!({}))
            .await
            .unwrap();
        assert_eq!(listed["files"], json!(["NOTES.md"]));

        // Without read patterns the tools are off.
        let off = HerdrToolExecutor::new(client.clone(), vec![])
            .with_current_agent(Arc::clone(&current), false)
            .with_planning(planning(&[], true));
        let err = off
            .execute_tool("read_file", &json!({ "path": "NOTES.md" }))
            .await
            .unwrap_err();
        assert!(err.contains("Reading files is off"), "{err}");
    }

    /// One turn on a stored `history`, as the companion runs it: the
    /// messages sent, with what the contributor added appended to the
    /// history before the user's text and a reply.
    async fn stored_turn(
        contributor: &HerdrContextContributor,
        history: &mut Vec<libinfer::Message>,
        said: &str,
    ) -> Vec<libinfer::Message> {
        use libinfer::TurnContributor;
        let message = |role: &str, content: &str| libinfer::Message {
            role: role.to_string(),
            content: content.to_string(),
            name: None,
            thought_signature: None,
        };
        let mut messages = history.clone();
        messages.push(message("user", said));
        contributor.contribute(&mut messages).await.unwrap();
        history.extend(contributor.take_added());
        history.push(message("user", said));
        history.push(message("model", "ok"));
        messages
    }

    /// The instructions blocks among `messages`.
    fn blocks(messages: &[libinfer::Message]) -> Vec<&str> {
        messages
            .iter()
            .filter(|m| m.content.starts_with(INSTRUCTIONS_MARKER))
            .map(|m| m.content.as_str())
            .collect()
    }

    /// Plan 20261027_05, step 4: the block goes into the stored history,
    /// once per project and content, and never into the per-turn context.
    #[tokio::test]
    async fn test_the_instructions_are_sent_once_per_project_and_content() {
        let dir = two_projects();
        std::fs::write(dir.path().join("b/AGENTS.md"), "Rules for b.").unwrap();
        let client = project_mock(dir.path()).await;
        let current = CurrentAgent::default();
        let prompt = PlanningPrompt::load(None).unwrap();
        let files = crate::watch_files::FileWatcher::start().unwrap();
        let cache = Arc::new(InstructionsCache::default());
        let contributor = || {
            HerdrContextContributor::new(client.clone(), vec![])
                .with_current_agent(Arc::clone(&current))
                .with_planning(planning(&[], true), prompt.clone())
                .with_instructions(Arc::clone(&cache), Some(files.clone()))
        };
        let mut history = Vec::new();

        // No current agent: no instructions.
        let sent = stored_turn(&contributor(), &mut history, "hello").await;
        assert!(blocks(&sent).is_empty());

        // AGENTS.md over CLAUDE.md, as a system message before the user's
        // turn, its marker first; the context ends with the current agent.
        *current.lock().unwrap() = Some("p1".to_string());
        let sent = stored_turn(&contributor(), &mut history, "hello").await;
        let found = blocks(&sent);
        assert_eq!(found.len(), 1, "{sent:?}");
        let block = found[0];
        let marker = instructions_marker("a", "Agents rules for a.");
        assert!(block.starts_with(&format!("{marker}\n")), "{block}");
        assert!(block.contains("The current project is a, where the Claude"));
        assert!(block.ends_with("from AGENTS.md:\n\nAgents rules for a."));
        let at = sent.iter().position(|m| m.content == block).unwrap();
        assert_eq!(sent[at].role, "system");
        assert_eq!(sent[at + 1].role, "user", "before the user's turn");
        let context = &sent.last().unwrap().content;
        assert!(!context.contains("The current project is"), "{context}");
        assert!(!context.contains(INSTRUCTIONS_MARKER));
        let last_line = context.trim_end().lines().last().unwrap();
        assert!(last_line.starts_with("Current agent: "), "{last_line}");
        assert!(cache.is_watched());
        assert_eq!(blocks(&history), [block], "kept in the history");

        // Unchanged: not again.
        let sent = stored_turn(&contributor(), &mut history, "again").await;
        assert_eq!(blocks(&sent).len(), 1, "only the stored one");
        assert_eq!(blocks(&history).len(), 1);

        // Another project: its own block.
        *current.lock().unwrap() = Some("p2".to_string());
        stored_turn(&contributor(), &mut history, "b").await;
        assert_eq!(blocks(&history).len(), 2);
        assert!(blocks(&history)[1].ends_with("Rules for b."));

        // Back to a: its block is in the history already.
        *current.lock().unwrap() = Some("p1".to_string());
        stored_turn(&contributor(), &mut history, "a").await;
        assert_eq!(blocks(&history).len(), 2);

        // The file changes: sent again once the watcher says so.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        std::fs::write(dir.path().join("a/AGENTS.md"), "New rules for a.").unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        stored_turn(&contributor(), &mut history, "edited").await;
        assert_eq!(blocks(&history).len(), 3, "{:?}", blocks(&history));
        assert!(blocks(&history)[2].ends_with("New rules for a."));
        stored_turn(&contributor(), &mut history, "once").await;
        assert_eq!(blocks(&history).len(), 3);

        // A restart (a new cache, the same history): not again.
        let fresh = HerdrContextContributor::new(client.clone(), vec![])
            .with_current_agent(Arc::clone(&current))
            .with_planning(planning(&[], true), prompt.clone())
            .with_instructions(Arc::default(), None);
        stored_turn(&fresh, &mut history, "restarted").await;
        assert_eq!(blocks(&history).len(), 3);

        // A history without the marker (trimmed): sent again.
        let mut trimmed: Vec<libinfer::Message> = history
            .iter()
            .filter(|m| !m.content.starts_with(INSTRUCTIONS_MARKER))
            .cloned()
            .collect();
        stored_turn(&contributor(), &mut trimmed, "trimmed").await;
        assert_eq!(blocks(&trimmed).len(), 1);

        // agents_prompt off: nothing more is added.
        let off = HerdrContextContributor::new(client, vec![])
            .with_current_agent(Arc::clone(&current))
            .with_planning(planning(&["*.md"], false), prompt)
            .with_instructions(Arc::clone(&cache), Some(files.clone()));
        let mut empty = Vec::new();
        let sent = stored_turn(&off, &mut empty, "off").await;
        assert!(blocks(&sent).is_empty());
        assert!(blocks(&empty).is_empty());
    }

    #[test]
    fn test_the_history_holds_only_the_projects_latest_marker() {
        let message = |content: String| libinfer::Message {
            role: "system".to_string(),
            content,
            name: None,
            thought_signature: None,
        };
        let v1 = instructions_marker("my project", "one");
        let v2 = instructions_marker("my project", "two");
        let other = instructions_marker("other", "one");
        assert_ne!(v1, v2);
        assert_eq!(
            v1.len(),
            INSTRUCTIONS_MARKER.len() + "my project".len() + 1 + 16 + 1
        );
        let history = vec![
            message(format!("{v1}\nblock")),
            message(format!("{v2}\nblock")),
            message(format!("{other}\nblock")),
        ];
        assert!(history_holds(&history, &v2));
        assert!(!history_holds(&history, &v1), "reverted to v1: sent again");
        assert!(history_holds(&history, &other));
        assert!(!history_holds(&[], &v1));
    }

    #[tokio::test]
    async fn test_the_old_names_are_still_dispatched() {
        let client = HerdrClient::mock();
        let executor = HerdrToolExecutor::new(
            client.clone(),
            vec![AgentConfig::new(
                "claude",
                vec!["continue".to_string()],
                false,
            )],
        );
        for name in ["run_command", "send_command"] {
            executor
                .execute_tool(name, &json!({ "pane_id": "p1", "command": "continue" }))
                .await
                .unwrap();
        }
        assert_eq!(command_log(&client).await.len(), 2);
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
                "send_agent_command",
                &json!({ "agent": "p1", "text": "continue" }),
            )
            .await;
        assert!(res.unwrap_err().contains("confirmation unavailable"));
    }

    #[tokio::test]
    async fn test_voice_turn_confirmations_carry_a_settled_handle() {
        use crate::core::confirm::{DEFAULT_TTL, StatusKey};
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
        let run = |handle: &'static str, command: &'static str| json!({ "agent": "p1", "text": command, "handle": handle });

        // The model's handle, normalised.
        let said = "send usage to claude in high bay";
        let res = executor(said)
            .execute_tool(
                "send_agent_command",
                &run("Send-Usage-Claude-Highbay", "/usage"),
            )
            .await
            .unwrap();
        assert_eq!(res["status"], "awaiting_user_approval");
        assert_eq!(res["handle"], "send-usage-claude-highbay");
        // The same again while pending: `-2`.
        let res = executor(said)
            .execute_tool(
                "send_agent_command",
                &run("send-usage-claude-highbay", "/usage"),
            )
            .await
            .unwrap();
        assert_eq!(res["handle"], "send-usage-claude-highbay-2");
        // A garbled transcript does not replace the model's handle.
        let res = executor("sin did again")
            .execute_tool(
                "send_agent_command",
                &run("send-continue-again-claude", "continue"),
            )
            .await
            .unwrap();
        assert_eq!(res["handle"], "send-continue-again-claude");

        assert_eq!(log.confirmations.lock().unwrap().len(), 3);
        let principal = Principal::local("flankr");
        let (view, _) = broker
            .status(
                &principal,
                &StatusKey::Handle("send-continue-again-claude".to_string()),
            )
            .unwrap();
        assert_eq!(view.command, "continue");
        assert_eq!(
            broker.pending_handles(&principal.id),
            [
                "send-usage-claude-highbay",
                "send-usage-claude-highbay-2",
                "send-continue-again-claude"
            ]
        );
        assert!(match &client.transport() {
            libherdr::transport::HerdrTransport::Mock { state } =>
                state.read().await.command_log.is_empty(),
            _ => unreachable!(),
        });
    }

    /// The mock with p1 as Claude Code in `/work/example`, idle, with the
    /// fixture session's id, and p2 a working shell.
    async fn session_mock() -> HerdrClient {
        let client = HerdrClient::mock();
        if let HerdrTransport::Mock { state } = client.transport() {
            let mut state = state.write().await;
            let tab = &mut state.snapshot.workspaces[0].tabs[0];
            let p1 = &mut tab.panes[0];
            p1.cwd = Some("/work/example".to_string());
            p1.agent_status = Some(AgentStatus::Idle);
            p1.agent_session = Some(AgentSession {
                source: "herdr:claude".to_string(),
                agent: "claude".to_string(),
                kind: "id".to_string(),
                value: "00000000-0000-4000-8000-0000000000a1".to_string(),
            });
            let mut p2 = p1.clone();
            p2.pane_id = "p2".to_string();
            p2.focused = false;
            p2.agent_session = None;
            p2.agent_status = Some(AgentStatus::Working);
            p2.terminal_title = Some("zsh".to_string());
            p2.extra.remove("agent");
            tab.panes.push(p2);
        }
        client
    }

    fn fixture_sessions() -> Arc<Sessions> {
        Arc::new(Sessions::with_home(
            &crate::sessions::test_support::fixture("home"),
        ))
    }

    async fn read_log(client: &HerdrClient) -> Vec<(String, ReadSource, usize)> {
        match client.transport() {
            HerdrTransport::Mock { state } => state.read().await.read_log.clone(),
            _ => unreachable!(),
        }
    }

    async fn context_for(contributor: &HerdrContextContributor, said: &str) -> String {
        use libinfer::TurnContributor;
        let mut messages = vec![libinfer::Message {
            role: "user".to_string(),
            content: said.to_string(),
            name: None,
            thought_signature: None,
        }];
        contributor.contribute(&mut messages).await.unwrap();
        messages.pop().unwrap().content
    }

    #[tokio::test]
    async fn test_context_carries_the_agent_sessions_within_limits() {
        let client = session_mock().await;
        let contributor = HerdrContextContributor::new(client.clone(), vec![])
            .with_naming(NamingConfig::default())
            .with_sessions(fixture_sessions());
        let content = context_for(&contributor, "what is the flankr agent waiting on?").await;
        let start = content.find("### Agent sessions").expect("the section");
        let section = &content[start..];
        assert!(section.len() <= crate::sessions::context::TOTAL_MAX);
        assert!(section.contains("#### Claude (pane p1, idle)"), "{section}");
        assert!(section.contains("- user [2026-01-04T07:00:00.000Z]: Where do the notices stand?"));
        assert!(section.contains("(1 tool call in between)"));
        assert!(section.contains(
            "Pending question [2026-01-04T07:15:00.000Z]: Should the notices wait for the phone to finish speaking, or interrupt it?"
        ));
        // The covered agent's screen is not read; the shell's is, unwrapped.
        assert_eq!(
            read_log(&client).await,
            [(
                "p2".to_string(),
                ReadSource::RecentUnwrapped,
                CONTEXT_TAIL_LINES
            )]
        );
        assert!(content.contains("Recent 80 Lines of Output"));
    }

    #[tokio::test]
    async fn test_context_without_sessions_reads_every_tail() {
        let client = session_mock().await;
        let contributor = HerdrContextContributor::new(client.clone(), vec![]);
        let content = context_for(&contributor, "hello").await;
        assert!(!content.contains("### Agent sessions"));
        let panes: Vec<String> = read_log(&client).await.into_iter().map(|r| r.0).collect();
        assert_eq!(panes, ["p1", "p2"]);
    }

    #[tokio::test]
    async fn test_a_blocked_agent_keeps_its_screen_and_a_remote_herdr_has_no_sessions() {
        let client = session_mock().await;
        if let HerdrTransport::Mock { state } = client.transport() {
            state.write().await.snapshot.workspaces[0].tabs[0].panes[0].agent_status =
                Some(AgentStatus::Blocked);
        }
        let contributor =
            HerdrContextContributor::new(client.clone(), vec![]).with_sessions(fixture_sessions());
        let content = context_for(&contributor, "status?").await;
        assert!(content.contains("#### Claude (pane p1, blocked)"));
        let panes: Vec<String> = read_log(&client).await.into_iter().map(|r| r.0).collect();
        assert_eq!(panes, ["p1", "p2"]);

        let remote = HerdrContextContributor::new(
            HerdrClient::remote_with_config(RemoteSshConfig::new("example.invalid")),
            vec![],
        )
        .with_sessions(fixture_sessions());
        let snapshot = client.list_panes().await.unwrap();
        let (section, covered) = remote.session_section(&snapshot, Some("status?"));
        assert!(section.is_empty() && covered.is_empty());
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
}
