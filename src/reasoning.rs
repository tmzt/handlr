//! Reasoning engine trait, Gemini integration, and mock implementations for `handlr`.
//!
//! Grounded in the `herdr` terminal multiplexer architecture, converting spoken intent
//! and natural-language transcripts into tool calls and concise spoken feedback.

use async_trait::async_trait;
pub use libinfer::{ReasoningEngine, ToolExecutor, GeminiAgent, Tool, FunctionDeclaration, GeminiReasoningEngine, EngineType, ReasoningEngineBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Embedded system prompt from `AGENTS.md` at compile time.
pub const AGENTS_MD_RAW: &str = include_str!("../AGENTS.md");

// -------------------------------------------------------------------
//  System Prompt Extraction
// -------------------------------------------------------------------

/// Parses the system prompt content between `<system_prompt>` and `</system_prompt>` tags.
pub fn parse_system_prompt(content: &str) -> Option<String> {
    let start_tag = "<system_prompt>";
    let end_tag = "</system_prompt>";

    let start_idx = content.find(start_tag)? + start_tag.len();
    let end_idx = content[start_idx..].find(end_tag)?;
    Some(content[start_idx..start_idx + end_idx].trim().to_string())
}

/// Extracts the system prompt from the embedded `AGENTS.md` file.
pub fn default_system_prompt() -> Option<String> {
    parse_system_prompt(AGENTS_MD_RAW)
}

/// Loads the system prompt from a specific file path (e.g. `AGENTS.md`),
/// falling back to the embedded compile-time prompt if file is missing.
pub fn load_system_prompt_from_file(path: impl AsRef<std::path::Path>) -> Result<String, String> {
    let p = path.as_ref();
    if p.exists() {
        let content = std::fs::read_to_string(p)
            .map_err(|e| format!("failed to read system prompt file {}: {e}", p.display()))?;
        parse_system_prompt(&content)
            .ok_or_else(|| format!("no <system_prompt> tag found in {}", p.display()))
    } else {
        default_system_prompt()
            .ok_or_else(|| "failed to parse system prompt from embedded AGENTS.md".to_string())
    }
}

// -------------------------------------------------------------------
//  Tool Schemas (Gemini Function Declarations)
// -------------------------------------------------------------------

/// Default number of buffer lines to tail when inspecting a pane.
pub const DEFAULT_INSPECT_LINES: usize = 50;

fn default_inspect_lines() -> Option<usize> {
    Some(DEFAULT_INSPECT_LINES)
}

/// Parameters for the `list_panes` tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ListPanesParams {
    /// Optional workspace ID to filter the pane listing to a specific project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

/// Parameters for the `inspect_pane` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct InspectPaneParams {
    /// The target pane ID to read the tail buffer from (e.g. "p1", "pane-1").
    pub pane_id: String,
    /// Number of tail buffer lines to inspect (default: 50).
    #[serde(default = "default_inspect_lines")]
    pub lines: Option<usize>,
}

impl InspectPaneParams {
    pub fn new(pane_id: impl Into<String>) -> Self {
        Self {
            pane_id: pane_id.into(),
            lines: Some(DEFAULT_INSPECT_LINES),
        }
    }

    pub fn with_lines(pane_id: impl Into<String>, lines: usize) -> Self {
        Self {
            pane_id: pane_id.into(),
            lines: Some(lines),
        }
    }
}

/// Parameters for the `run_command` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct RunCommandParams {
    /// Target pane ID in which to execute the command.
    pub pane_id: String,
    /// The shell command string to execute.
    pub command: String,
}

impl RunCommandParams {
    pub fn new(pane_id: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            pane_id: pane_id.into(),
            command: command.into(),
        }
    }
}

/// Parameters for the `send_keys` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SendKeysParams {
    /// Target pane ID to send keystrokes to.
    pub pane_id: String,
    /// The keystroke or key combo string (e.g. "ctrl+c", "enter", "q").
    pub keys: String,
}

impl SendKeysParams {
    pub fn new(pane_id: impl Into<String>, keys: impl Into<String>) -> Self {
        Self {
            pane_id: pane_id.into(),
            keys: keys.into(),
        }
    }
}

/// Parameters for the `manage_layout` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ManageLayoutParams {
    /// The layout action to perform (e.g. "split", "split_vertical", "split_horizontal", "zoom", "close").
    pub action: String,
    /// Optional target pane or workspace ID to apply the action to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

impl ManageLayoutParams {
    pub fn new(action: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            target_id: None,
        }
    }

    pub fn with_target(action: impl Into<String>, target_id: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            target_id: Some(target_id.into()),
        }
    }
}

/// Parameters for the `focus` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct FocusParams {
    /// The target workspace or pane ID to shift visual focus to (e.g. "w1", "p2").
    pub target_id: String,
}

impl FocusParams {
    pub fn new(target_id: impl Into<String>) -> Self {
        Self {
            target_id: target_id.into(),
        }
    }
}

/// Construct the full list of Gemini tool declarations for `herdr` orchestration.
pub fn declare_gemini_tools() -> Vec<Tool> {
    vec![Tool::with_functions(vec![
        FunctionDeclaration::new(
            "list_panes",
            "Returns the structural JSON layout of workspaces, tabs, and agent_status across the environment.",
            None,
        )
        .with_parameters::<ListPanesParams>(),

        FunctionDeclaration::new(
            "inspect_pane",
            "Reads the raw tail buffer lines of a specific terminal pane to inspect logs, errors, or compiler output.",
            None,
        )
        .with_parameters::<InspectPaneParams>(),

        FunctionDeclaration::new(
            "run_command",
            "Executes a command in the target terminal pane.",
            None,
        )
        .with_parameters::<RunCommandParams>(),

        FunctionDeclaration::new(
            "send_keys",
            "Sends literal keystrokes (e.g. 'ctrl+c', 'enter') to the specified pane.",
            None,
        )
        .with_parameters::<SendKeysParams>(),

        FunctionDeclaration::new(
            "manage_layout",
            "Handles spatial layout commands ('split', 'zoom', 'close') for workspaces and panes.",
            None,
        )
        .with_parameters::<ManageLayoutParams>(),

        FunctionDeclaration::new(
            "focus",
            "Shifts the user's active visual focus to a specific pane or workspace (project).",
            None,
        )
        .with_parameters::<FocusParams>(),
    ])]
}

// -------------------------------------------------------------------
//  GeminiReasoningEngine
// -------------------------------------------------------------------


// -------------------------------------------------------------------
//  MockReasoningEngine
// -------------------------------------------------------------------

pub use libinfer::{MockHandler, MockReasoningEngine, MockToolExecutor};

// -------------------------------------------------------------------
//  Unit Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_system_prompt_extraction() {
        let prompt = default_system_prompt().expect("should extract system prompt from AGENTS.md");
        assert!(prompt.contains("You are the reasoning engine for `handlr`"));
        assert!(prompt.contains("Data Processing Rules"));
        assert!(prompt.contains("Execution Rules"));
        assert!(prompt.contains("Voice Output Guidelines"));
        assert!(!prompt.contains("<system_prompt>"));
        assert!(!prompt.contains("</system_prompt>"));
    }

    #[test]
    fn test_parse_system_prompt_custom() {
        let sample = "Header\n<system_prompt>\nCustom prompt here\n</system_prompt>\nFooter";
        let parsed = parse_system_prompt(sample).expect("should parse tag content");
        assert_eq!(parsed, "Custom prompt here");
    }

    #[test]
    fn test_gemini_tool_declarations_schemas() {
        let tools = declare_gemini_tools();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];

        let decls = match tool {
            Tool::Function {
                function_declarations,
            } => function_declarations,
            _ => panic!("expected function tool"),
        };
        assert_eq!(decls.len(), 6);

        let names: Vec<&str> = decls.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "list_panes",
                "inspect_pane",
                "run_command",
                "send_keys",
                "manage_layout",
                "focus"
            ]
        );
    }

    #[test]
    fn test_tool_params_serialization() {
        // 1. ListPanesParams
        let lp = ListPanesParams {
            workspace_id: Some("w1".into()),
        };
        let lp_json = serde_json::to_value(&lp).unwrap();
        assert_eq!(lp_json["workspace_id"], "w1");

        // 2. InspectPaneParams
        let ip = InspectPaneParams::with_lines("p1", 20);
        let ip_json = serde_json::to_value(&ip).unwrap();
        assert_eq!(ip_json["pane_id"], "p1");
        assert_eq!(ip_json["lines"], 20);

        // 3. RunCommandParams
        let rc = RunCommandParams::new("p1", "cargo check");
        let rc_json = serde_json::to_value(&rc).unwrap();
        assert_eq!(rc_json["pane_id"], "p1");
        assert_eq!(rc_json["command"], "cargo check");

        // 4. SendKeysParams
        let sk = SendKeysParams::new("p1", "ctrl+c");
        let sk_json = serde_json::to_value(&sk).unwrap();
        assert_eq!(sk_json["pane_id"], "p1");
        assert_eq!(sk_json["keys"], "ctrl+c");

        // 5. ManageLayoutParams
        let ml = ManageLayoutParams::with_target("split_vertical", "p1");
        let ml_json = serde_json::to_value(&ml).unwrap();
        assert_eq!(ml_json["action"], "split_vertical");
        assert_eq!(ml_json["target_id"], "p1");

        // 6. FocusParams
        let fp = FocusParams::new("w2");
        let fp_json = serde_json::to_value(&fp).unwrap();
        assert_eq!(fp_json["target_id"], "w2");
    }

    #[tokio::test]
    async fn test_mock_reasoning_engine_canned_response() {
        let engine = MockReasoningEngine::with_response("Switched to project alpha.");
        let executor = MockToolExecutor::new();

        let response = engine
            .process_transcript("focus on alpha", &executor)
            .await
            .expect("process_transcript failed");

        assert_eq!(response, Some("Switched to project alpha.".to_string()));
        assert_eq!(engine.recorded_transcripts(), vec!["focus on alpha"]);
    }

    struct StatusHandler;
    #[async_trait]
    impl MockHandler for StatusHandler {
        async fn handle(
            &self,
            transcript: &str,
            executor: &dyn ToolExecutor,
        ) -> Result<Option<String>, String> {
            if transcript.contains("status") {
                let tool_res = executor.execute_tool("list_panes", &json!({})).await?;
                let count = tool_res["workspaces"]
                    .as_array()
                    .map(|w| w.len())
                    .unwrap_or(0);
                Ok(Some(format!("You have {count} active workspaces.")))
            } else {
                Ok(Some("Ready".to_string()))
            }
        }
    }

    #[tokio::test]
    async fn test_mock_reasoning_engine_with_tool_execution() {
        let engine = MockReasoningEngine::with_handler(StatusHandler);

        let executor = MockToolExecutor::new().with_canned_response(
            "list_panes",
            json!({
                "workspaces": [
                    {"workspace_id": "w1", "name": "handlr"},
                    {"workspace_id": "w2", "name": "herdr"}
                ]
            }),
        );

        let response = engine
            .process_transcript("check status", &executor)
            .await
            .expect("should process transcript");

        assert_eq!(response, Some("You have 2 active workspaces.".to_string()));
        assert_eq!(executor.recorded_calls().len(), 1);
        assert_eq!(executor.recorded_calls()[0].0, "list_panes");
    }

    #[tokio::test]
    async fn test_mock_reasoning_engine_queued_responses() {
        let engine = MockReasoningEngine::with_queued_responses(vec![
            Some("First response".into()),
            Some("Second response".into()),
            None,
        ]);
        let executor = MockToolExecutor::new();

        let r1 = engine
            .process_transcript("query 1", &executor)
            .await
            .unwrap();
        let r2 = engine
            .process_transcript("query 2", &executor)
            .await
            .unwrap();
        let r3 = engine
            .process_transcript("query 3", &executor)
            .await
            .unwrap();

        assert_eq!(r1, Some("First response".to_string()));
        assert_eq!(r2, Some("Second response".to_string()));
        assert_eq!(r3, None);
    }

    #[tokio::test]
    async fn test_mock_reasoning_engine_error() {
        let engine = MockReasoningEngine::with_error("Network timeout");
        let executor = MockToolExecutor::new();

        let res = engine.process_transcript("hello", &executor).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), "Network timeout");
    }

    #[test]
    fn test_gemini_reasoning_engine_initialization() {
        // Verify constructor builds agent with prompt and all 6 tools
        let system_prompt = default_system_prompt().unwrap();
        let tools = declare_gemini_tools();
        let engine = GeminiReasoningEngine::new("test_api_key".to_string(), Some(system_prompt), tools)
            .expect("failed to construct GeminiReasoningEngine");

        assert_eq!(engine.max_turns(), 5);
        let prompt = engine
            .agent()
            .system_prompt()
            .expect("system prompt should be set");
        assert!(prompt.contains("reasoning engine for `handlr`"));
        assert_eq!(engine.agent().tools().len(), 1);

        let decls = match &engine.agent().tools()[0] {
            Tool::Function {
                function_declarations,
            } => function_declarations,
            _ => panic!("expected function tool"),
        };
        assert_eq!(decls.len(), 6);
    }
}
