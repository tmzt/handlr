//! Protocol definitions and JSON message models for `herdr`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default number of buffer lines to tail when inspecting a pane.
pub const DEFAULT_INSPECT_LINES: usize = 50;

/// Standard `agent_status` for terminal panes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// The agent in this pane is actively computing/running commands.
    Working,
    /// The agent or shell in this pane is idle, waiting for input.
    Idle,
    /// The agent is stopped waiting on the user — a prompt, a permission
    /// request, a confirmation it cannot answer itself.
    Blocked,
    /// The agent finished the task it was working on.
    Done,
    /// Status cannot be reliably determined.
    Unknown,
    /// Custom status reported by a specific agent runner.
    #[serde(untagged)]
    Custom(String),
}

impl Default for AgentStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Information describing a single terminal pane within `herdr`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneInfo {
    /// Unique identifier for this pane (e.g. "p1", "pane-42").
    pub pane_id: String,
    /// Terminal title if set (e.g. "claude", "cargo test").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_title: Option<String>,
    /// Current working directory of the process running in the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Autonomous agent activity status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_status: Option<AgentStatus>,
    /// Whether this pane currently has active input focus.
    #[serde(default)]
    pub focused: bool,
    /// The primary foreground command or process name if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The session herdr detected for the pane's agent, where it detects
    /// one (Claude Code and the Antigravity CLI as of 2026-10-22).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<AgentSession>,
    /// Additional metadata fields forwarded from `herdr`.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// The session herdr reports for a pane's agent, such as
/// `{"source": "herdr:claude", "agent": "claude", "kind": "id", "value": "<uuid>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSession {
    /// The detector that found it (`herdr:claude`).
    #[serde(default)]
    pub source: String,
    /// The agent it belongs to (`claude`, `agy`).
    #[serde(default)]
    pub agent: String,
    /// What `value` is: `id` (a session id) or `path` (a session file).
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub value: String,
}

impl AgentSession {
    /// The session id, when `kind` is `id` and the value is not empty.
    pub fn id(&self) -> Option<&str> {
        let value = self.value.trim();
        (self.kind == "id" && !value.is_empty()).then_some(value)
    }
}

/// Which of a pane's terminal snapshots `pane.read` returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadSource {
    /// The visible frame only.
    Visible,
    /// The recent lines, as the terminal wrapped them (herdr's default).
    #[default]
    Recent,
    /// The recent lines with soft wraps joined back into logical lines.
    RecentUnwrapped,
    /// What herdr's agent detection reads.
    Detection,
}

/// How `pane.read` renders the snapshot it returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadFormat {
    /// Plain text, escapes stripped (herdr's default).
    #[default]
    Text,
    /// The text with SGR colour and attribute escapes kept, as the
    /// terminal emulator renders them (herdr's `visible_ansi()` for the
    /// visible frame).
    Ansi,
}

impl ReadFormat {
    /// True for [`ReadFormat::Text`], herdr's default, which is left off
    /// the wire so a text read is byte-identical to one without `format`.
    pub fn is_text(&self) -> bool {
        matches!(self, ReadFormat::Text)
    }
}

/// Information describing a tab within a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabInfo {
    /// Unique identifier for this tab.
    pub tab_id: String,
    /// Optional user-defined tab title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// List of panes contained in this tab.
    #[serde(default)]
    pub panes: Vec<PaneInfo>,
    /// Whether this tab is currently selected in its workspace.
    #[serde(default)]
    pub focused: bool,
}

/// Information describing a workspace (project).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Unique identifier for this workspace (e.g. "w1", "workspace-project").
    pub workspace_id: String,
    /// Human-readable workspace / project name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tabs within the workspace.
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
    /// Flat list of panes if tabs are not used.
    #[serde(default)]
    pub panes: Vec<PaneInfo>,
    /// Whether this workspace is the active viewport for the user.
    #[serde(default)]
    pub focused: bool,
}

/// Comprehensive layout snapshot of the user's `herdr` environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    /// All configured workspaces.
    #[serde(default)]
    pub workspaces: Vec<WorkspaceInfo>,
    /// ID of the currently focused workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_workspace_id: Option<String>,
    /// ID of the currently focused pane across the whole session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_pane_id: Option<String>,
}

impl EnvironmentSnapshot {
    /// Helper to find all panes with `AgentStatus::Working`.
    pub fn find_working_panes(&self) -> Vec<&PaneInfo> {
        let mut working = Vec::new();
        for ws in &self.workspaces {
            for tab in &ws.tabs {
                for pane in &tab.panes {
                    if pane.agent_status == Some(AgentStatus::Working) {
                        working.push(pane);
                    }
                }
            }
            for pane in &ws.panes {
                if pane.agent_status == Some(AgentStatus::Working) {
                    working.push(pane);
                }
            }
        }
        working
    }

    /// Helper to find a pane by its ID.
    pub fn find_pane(&self, pane_id: &str) -> Option<&PaneInfo> {
        for ws in &self.workspaces {
            for tab in &ws.tabs {
                for pane in &tab.panes {
                    if pane.pane_id == pane_id {
                        return Some(pane);
                    }
                }
            }
            for pane in &ws.panes {
                if pane.pane_id == pane_id {
                    return Some(pane);
                }
            }
        }
        None
    }
}

/// Standard layout manipulation actions supported by `manage_layout`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutAction {
    Split,
    SplitVertical,
    SplitHorizontal,
    Zoom,
    Unzoom,
    Close,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    #[serde(untagged)]
    Custom(String),
}

static REQ_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Generate a unique request ID for herdr protocol requests.
pub fn generate_request_id() -> String {
    let count = REQ_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    format!("req-{}", count)
}

/// Standard `herdr` protocol request corresponding to LLM tool calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HerdrRequest {
    /// Returns the structural JSON layout of workspaces, tabs, and `agent_status`.
    ListPanes {
        #[serde(default = "generate_request_id")]
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_id: Option<String>,
    },
    /// Reads the raw tail buffer of a specific terminal.
    InspectPane {
        #[serde(default = "generate_request_id")]
        id: String,
        pane_id: String,
        #[serde(default = "default_inspect_lines_val")]
        lines: usize,
        /// Which snapshot to read; herdr's default, `recent`, when absent.
        #[serde(default)]
        source: ReadSource,
        /// How the snapshot is rendered; herdr's default, `text`, when
        /// absent. Not serialised when `text`.
        #[serde(default, skip_serializing_if = "ReadFormat::is_text")]
        format: ReadFormat,
    },
    /// Executes a command in the target pane.
    RunCommand {
        #[serde(default = "generate_request_id")]
        id: String,
        pane_id: String,
        command: String,
    },
    /// Sends literal keystrokes (e.g. `ctrl+c`, `enter`).
    SendKeys {
        #[serde(default = "generate_request_id")]
        id: String,
        pane_id: String,
        keys: String,
    },
    /// Handles spatial commands (`split`, `zoom`, `close`).
    ManageLayout {
        #[serde(default = "generate_request_id")]
        id: String,
        #[serde(rename = "layout_action", alias = "action_type")]
        layout_action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_id: Option<String>,
    },
    /// Shifts the user's active visual focus to a specific pane or workspace.
    Focus {
        #[serde(default = "generate_request_id")]
        id: String,
        target_id: String,
    },
    /// Pass-through for custom or extended tools.
    #[serde(untagged)]
    Custom(Value),
}

fn default_inspect_lines_val() -> usize {
    DEFAULT_INSPECT_LINES
}

impl HerdrRequest {
    /// Create a `list_panes` request.
    pub fn list_panes() -> Self {
        Self::ListPanes {
            id: generate_request_id(),
            workspace_id: None,
        }
    }

    /// Create a `list_panes` request scoped to a specific workspace.
    pub fn list_panes_scoped(workspace_id: impl Into<String>) -> Self {
        Self::ListPanes {
            id: generate_request_id(),
            workspace_id: Some(workspace_id.into()),
        }
    }

    /// Create an `inspect_pane` request with the given number of buffer lines.
    pub fn inspect_pane(pane_id: impl Into<String>, lines: usize) -> Self {
        Self::inspect_pane_from(pane_id, lines, ReadSource::Recent)
    }

    /// Create an `inspect_pane` request reading `source`.
    pub fn inspect_pane_from(pane_id: impl Into<String>, lines: usize, source: ReadSource) -> Self {
        Self::inspect_pane_as(pane_id, lines, source, ReadFormat::Text)
    }

    /// Create an `inspect_pane` request reading `source` rendered as
    /// `format`; `ReadFormat::Ansi` keeps the SGR escapes.
    pub fn inspect_pane_as(
        pane_id: impl Into<String>,
        lines: usize,
        source: ReadSource,
        format: ReadFormat,
    ) -> Self {
        Self::InspectPane {
            id: generate_request_id(),
            pane_id: pane_id.into(),
            lines,
            source,
            format,
        }
    }

    /// Create a `run_command` request.
    pub fn run_command(pane_id: impl Into<String>, command: impl Into<String>) -> Self {
        Self::RunCommand {
            id: generate_request_id(),
            pane_id: pane_id.into(),
            command: command.into(),
        }
    }

    /// Create a `send_keys` request.
    pub fn send_keys(pane_id: impl Into<String>, keys: impl Into<String>) -> Self {
        Self::SendKeys {
            id: generate_request_id(),
            pane_id: pane_id.into(),
            keys: keys.into(),
        }
    }

    /// Create a `manage_layout` request.
    pub fn manage_layout(
        layout_action: impl Into<String>,
        target_id: Option<impl Into<String>>,
    ) -> Self {
        Self::ManageLayout {
            id: generate_request_id(),
            layout_action: layout_action.into(),
            target_id: target_id.map(|t| t.into()),
        }
    }

    /// Create a `focus` request.
    pub fn focus(target_id: impl Into<String>) -> Self {
        Self::Focus {
            id: generate_request_id(),
            target_id: target_id.into(),
        }
    }

    /// Parse a tool invocation from Gemini function calling (`name` and `args` JSON).
    pub fn from_tool_call(name: &str, args: &Value) -> Result<Self, String> {
        match name {
            "list_panes" => {
                let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
                if let Some(ws_id) = workspace_id {
                    Ok(Self::list_panes_scoped(ws_id))
                } else {
                    Ok(Self::list_panes())
                }
            }
            "inspect_pane" => {
                let pane_id = args
                    .get("pane_id")
                    .or_else(|| args.get("target_id"))
                    .or_else(|| args.get("id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "inspect_pane requires 'pane_id' argument".to_string())?;

                let lines = args
                    .get("lines")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(DEFAULT_INSPECT_LINES);

                Ok(Self::inspect_pane(pane_id, lines))
            }
            "run_command" | "send_command" => {
                let pane_id = args
                    .get("pane_id")
                    .or_else(|| args.get("target_id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "run_command requires 'pane_id' argument".to_string())?;

                let command = args
                    .get("command")
                    .or_else(|| args.get("cmd"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "run_command requires 'command' argument".to_string())?;

                Ok(Self::run_command(pane_id, command))
            }
            "send_keys" => {
                let pane_id = args
                    .get("pane_id")
                    .or_else(|| args.get("target_id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "send_keys requires 'pane_id' argument".to_string())?;

                let keys = args
                    .get("keys")
                    .or_else(|| args.get("keystrokes"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "send_keys requires 'keys' argument".to_string())?;

                Ok(Self::send_keys(pane_id, keys))
            }
            "manage_layout" => {
                let layout_action = args
                    .get("action")
                    .or_else(|| args.get("layout_action"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "manage_layout requires 'action' argument".to_string())?;

                let target_id = args
                    .get("target_id")
                    .or_else(|| args.get("pane_id"))
                    .and_then(|v| v.as_str());

                Ok(Self::manage_layout(layout_action, target_id))
            }
            "focus" => {
                let target_id = args
                    .get("target_id")
                    .or_else(|| args.get("workspace_id"))
                    .or_else(|| args.get("pane_id"))
                    .or_else(|| args.get("id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "focus requires 'target_id' argument".to_string())?;

                Ok(Self::focus(target_id))
            }
            custom_name => Err(format!("Unknown tool name '{custom_name}'")),
        }
    }

    /// Convert this `HerdrRequest` to standard JSON-RPC 2.0 format expected by the `herdr` daemon.
    pub fn to_jsonrpc(&self) -> HerdrJsonRpcRequest {
        match self {
            HerdrRequest::ListPanes { id, workspace_id } => {
                let params = if let Some(ws) = workspace_id {
                    serde_json::json!({ "workspace_id": ws })
                } else {
                    serde_json::json!({})
                };
                HerdrJsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: id.clone(),
                    method: "session.snapshot".to_string(),
                    params,
                }
            }
            HerdrRequest::InspectPane {
                id,
                pane_id,
                lines,
                source,
                format,
            } => {
                // herdr's schema names the line count `lines`; `count` is
                // kept for herdr versions that read it. `format` is sent
                // only when it is not herdr's default, `text`.
                let mut params = serde_json::json!({
                    "pane_id": pane_id,
                    "source": source,
                    "lines": lines,
                    "count": lines,
                });
                if !format.is_text() {
                    params["format"] = serde_json::json!(format);
                }
                HerdrJsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: id.clone(),
                    method: "pane.read".to_string(),
                    params,
                }
            }
            HerdrRequest::RunCommand {
                id,
                pane_id,
                command,
            } => {
                let trimmed = command.trim_end_matches('\n');
                HerdrJsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: id.clone(),
                    method: "pane.send_input".to_string(),
                    params: serde_json::json!({
                        "pane_id": pane_id,
                        "text": trimmed,
                        "keys": ["Enter"],
                    }),
                }
            }
            HerdrRequest::SendKeys { id, pane_id, keys } => {
                let key_list: Vec<&str> = keys.split_whitespace().collect();
                let key_array: Vec<&str> = if key_list.is_empty() {
                    vec![keys.as_str()]
                } else {
                    key_list
                };
                HerdrJsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: id.clone(),
                    method: "pane.send_keys".to_string(),
                    params: serde_json::json!({
                        "pane_id": pane_id,
                        "keys": key_array,
                    }),
                }
            }
            HerdrRequest::ManageLayout {
                id,
                layout_action,
                target_id,
            } => {
                let (method, params) = match layout_action.as_str() {
                    "zoom" | "unzoom" => (
                        "pane.zoom".to_string(),
                        if let Some(tid) = target_id {
                            serde_json::json!({ "pane_id": tid })
                        } else {
                            serde_json::json!({})
                        },
                    ),
                    "split" | "split_vertical" => (
                        "pane.split".to_string(),
                        if let Some(tid) = target_id {
                            serde_json::json!({ "pane_id": tid, "direction": "vertical" })
                        } else {
                            serde_json::json!({ "direction": "vertical" })
                        },
                    ),
                    "split_horizontal" => (
                        "pane.split".to_string(),
                        if let Some(tid) = target_id {
                            serde_json::json!({ "pane_id": tid, "direction": "horizontal" })
                        } else {
                            serde_json::json!({ "direction": "horizontal" })
                        },
                    ),
                    "close" => (
                        "pane.close".to_string(),
                        if let Some(tid) = target_id {
                            serde_json::json!({ "pane_id": tid })
                        } else {
                            serde_json::json!({})
                        },
                    ),
                    other => (
                        other.to_string(),
                        if let Some(tid) = target_id {
                            serde_json::json!({ "target_id": tid })
                        } else {
                            serde_json::json!({})
                        },
                    ),
                };
                HerdrJsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: id.clone(),
                    method,
                    params,
                }
            }
            HerdrRequest::Focus { id, target_id } => {
                let (method, params) = if target_id.starts_with('w') && !target_id.contains(':') {
                    (
                        "workspace.focus".to_string(),
                        serde_json::json!({ "workspace_id": target_id }),
                    )
                } else if target_id.contains(":t") && !target_id.contains(":p") {
                    (
                        "tab.focus".to_string(),
                        serde_json::json!({ "tab_id": target_id }),
                    )
                } else {
                    (
                        "pane.focus".to_string(),
                        serde_json::json!({ "pane_id": target_id }),
                    )
                };
                HerdrJsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    id: id.clone(),
                    method,
                    params,
                }
            }
            HerdrRequest::Custom(val) => {
                if let Some(obj) = val.as_object() {
                    let id = obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(generate_request_id);
                    let method = obj
                        .get("method")
                        .or_else(|| obj.get("action"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("custom")
                        .to_string();
                    let params = obj.get("params").cloned().unwrap_or_else(|| val.clone());
                    HerdrJsonRpcRequest {
                        jsonrpc: "2.0".to_string(),
                        id,
                        method,
                        params,
                    }
                } else {
                    HerdrJsonRpcRequest {
                        jsonrpc: "2.0".to_string(),
                        id: generate_request_id(),
                        method: "custom".to_string(),
                        params: val.clone(),
                    }
                }
            }
        }
    }
}

/// Standard JSON-RPC 2.0 request envelope for `herdr`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HerdrJsonRpcRequest {
    pub jsonrpc: String,
    pub id: String,
    pub method: String,
    pub params: Value,
}

/// Standard response wrapper from `herdr`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HerdrResponse {
    /// Whether the command succeeded.
    pub success: bool,
    /// Result payload (structured snapshot, buffer text, status, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// Error message if `success` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl HerdrResponse {
    /// Create a successful response with data.
    pub fn ok(data: impl Serialize) -> Result<Self, serde_json::Error> {
        let value = serde_json::to_value(data)?;
        Ok(Self {
            success: true,
            data: Some(value),
            error: None,
        })
    }

    /// Create an error response.
    pub fn err(message: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }

    /// Unwraps the inner `data` payload or returns an error string.
    pub fn into_result(self) -> Result<Value, String> {
        if self.success {
            Ok(self.data.unwrap_or(Value::Null))
        } else {
            Err(self
                .error
                .unwrap_or_else(|| "Unknown herdr error occurred".to_string()))
        }
    }
}
