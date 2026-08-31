use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use async_compat::CompatExt;

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute_tool(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String>;
}

// Re-export key libgenai types
pub use libgenai::{GeminiAgent, Tool, FunctionDeclaration, Model, Role, Message as GenaiMessage, Content};

/// Supported reasoning engine types.
pub enum EngineType {
    Gemini {
        api_key: Option<String>,
        tools: Vec<Tool>,
    },
    Mock {
        default_response: Option<String>,
    },
}

/// Builder for setting up a ReasoningEngine instance.
pub struct ReasoningEngineBuilder {
    engine_type: EngineType,
    system_prompt: Option<String>,
    max_turns: usize,
}

impl ReasoningEngineBuilder {
    pub fn new(engine_type: EngineType) -> Self {
        Self {
            engine_type,
            system_prompt: None,
            max_turns: 5,
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn build(self) -> Result<std::sync::Arc<dyn ReasoningEngine>, String> {
        match self.engine_type {
            EngineType::Gemini { api_key, tools } => {
                let engine = if let Some(key) = api_key {
                    GeminiReasoningEngine::new(key, self.system_prompt, tools)?
                } else {
                    GeminiReasoningEngine::from_env(self.system_prompt, tools)?
                };
                let engine = engine.with_max_turns(self.max_turns);
                Ok(std::sync::Arc::new(engine))
            }
            EngineType::Mock { default_response } => {
                let engine = MockReasoningEngine::new(default_response);
                Ok(std::sync::Arc::new(engine))
            }
        }
    }
}

/// A reasoning engine powered by Google Gemini, implementing `ReasoningEngine`.
pub struct GeminiReasoningEngine {
    agent: GeminiAgent,
    max_turns: usize,
}

impl GeminiReasoningEngine {
    /// Create a new `GeminiReasoningEngine` with the given API key.
    pub fn new(api_key: String, system_prompt: Option<String>, tools: Vec<Tool>) -> Result<Self, String> {
        let mut agent = GeminiAgent::new(api_key)?;
        if let Some(prompt) = system_prompt {
            agent = agent.with_system_prompt(prompt);
        }
        for tool in tools {
            agent = agent.with_tool(tool);
        }
        Ok(Self {
            agent,
            max_turns: 5,
        })
    }

    /// Create from the environment (resolving either `GEMINI_API_KEY` or Google credentials).
    pub fn from_env(system_prompt: Option<String>, tools: Vec<Tool>) -> Result<Self, String> {
        let mut agent = GeminiAgent::from_env()?;
        if let Some(prompt) = system_prompt {
            agent = agent.with_system_prompt(prompt);
        }
        for tool in tools {
            agent = agent.with_tool(tool);
        }
        Ok(Self {
            agent,
            max_turns: 5,
        })
    }

    /// Construct with a pre-configured `GeminiAgent`.
    pub fn from_agent(agent: GeminiAgent) -> Self {
        Self {
            agent,
            max_turns: 5,
        }
    }

    /// Set the maximum multi-turn tool calling steps allowed per query.
    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    /// Override the system prompt for the agent.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.agent = self.agent.with_system_prompt(prompt);
        self
    }

    /// Reference to the underlying `GeminiAgent`.
    pub fn agent(&self) -> &GeminiAgent {
        &self.agent
    }

    /// Get the maximum multi-turn tool calling steps allowed per query.
    pub fn max_turns(&self) -> usize {
        self.max_turns
    }
}

#[async_trait]
impl ReasoningEngine for GeminiReasoningEngine {
    async fn process_transcript(
        &self,
        transcript: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        let msg = Message {
            role: "user".to_string(),
            content: transcript.to_string(),
            name: None,
            thought_signature: None,
        };
        self.process_turns(&[msg], executor).await
    }

    async fn process_turns(
        &self,
        turns: &[Message],
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        if turns.is_empty() {
            return Ok(None);
        }

        let mut builder = self.agent.client().generate_content();

        if let Some(ref prompt) = self.agent.system_prompt() {
            builder = builder.with_system_prompt(*prompt);
        }

        for tool in self.agent.tools() {
            builder = builder.with_tool(tool.clone());
        }

        if !self.agent.tools().is_empty() {
            builder = builder.with_function_calling_mode(libgenai::FunctionCallingMode::Auto);
        }

        // Add all historical messages to the builder
        for turn in turns {
            let role = match turn.role.as_str() {
                "model" => libgenai::Role::Model,
                _ => libgenai::Role::User,
            };

            let content = libgenai::Content::text(&turn.content);
            builder = builder.with_message(GenaiMessage { role, content });
        }

        // Loop for tool execution within the current turn
        for _ in 0..self.max_turns {
            let response = builder
                .clone()
                .execute()
                .compat()
                .await
                .map_err(|e| format!("gemini generate: {e}"))?;

            let calls = response.function_calls_with_thoughts();
            if let Some((fc, thought_signature)) = calls.first() {
                let args_val = &fc.args;
                let exec_result = executor.execute_tool(&fc.name, args_val).await?;

                // Construct model message with thought signature if present
                let model_content = if let Some(sig) = thought_signature {
                    libgenai::Content::function_call_with_thought((*fc).clone(), sig.to_string())
                } else {
                    libgenai::Content::function_call((*fc).clone())
                };
                let model_msg = GenaiMessage {
                    content: model_content,
                    role: libgenai::Role::Model,
                };
                builder = builder
                    .with_message(model_msg)
                    .with_function_response(&fc.name, exec_result)
                    .map_err(|e| format!("function response: {e}"))?;
            } else {
                let text = response.text().trim().to_string();
                return if text.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(text))
                };
            }
        }

        Err(format!("exceeded maximum multi-turn tool steps ({})", self.max_turns))
    }
}

#[async_trait]
impl<T: ToolExecutor + ?Sized> ToolExecutor for std::sync::Arc<T> {
    async fn execute_tool(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        (**self).execute_tool(name, args).await
    }
}

#[async_trait]
impl<T: ToolExecutor + ?Sized> ToolExecutor for Box<T> {
    async fn execute_tool(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        (**self).execute_tool(name, args).await
    }
}

#[async_trait]
pub trait ReasoningEngine: Send + Sync {
    async fn process_transcript(
        &self,
        transcript: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String>;

    async fn process_turns(
        &self,
        turns: &[Message],
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String>;
}

#[async_trait]
impl<T: ReasoningEngine + ?Sized> ReasoningEngine for std::sync::Arc<T> {
    async fn process_transcript(
        &self,
        transcript: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        (**self).process_transcript(transcript, executor).await
    }

    async fn process_turns(
        &self,
        turns: &[Message],
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        (**self).process_turns(turns, executor).await
    }
}

#[async_trait]
impl<T: ReasoningEngine + ?Sized> ReasoningEngine for Box<T> {
    async fn process_transcript(
        &self,
        transcript: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        (**self).process_transcript(transcript, executor).await
    }

    async fn process_turns(
        &self,
        turns: &[Message],
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        (**self).process_turns(turns, executor).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub role: String, // "user", "model", "system", "tool"
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

#[derive(Clone)]
pub struct SessionManager {
    cache_dir: PathBuf,
}

impl SessionManager {
    pub fn new() -> Result<Self, String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/tmeade".to_string());
        let cache_dir = PathBuf::from(home).join(".cache").join("libinfer");
        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| format!("failed to create session cache dir: {e}"))?;
        Ok(Self { cache_dir })
    }

    pub fn with_dir(cache_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&cache_dir)
            .map_err(|e| format!("failed to create session cache dir: {e}"))?;
        Ok(Self { cache_dir })
    }

    fn session_path(&self, session_id: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.jsonl", session_id))
    }

    pub fn load_session(&self, session_id: &str) -> Result<Vec<Message>, String> {
        let path = self.session_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&path).map_err(|e| format!("failed to open session file: {e}"))?;
        let reader = BufReader::new(file);
        let mut messages = Vec::new();
        for (i, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| format!("failed to read line {}: {e}", i + 1))?;
            let msg: Message = serde_json::from_str(&line)
                .map_err(|e| format!("failed to deserialize message at line {}: {e}", i + 1))?;
            messages.push(msg);
        }
        Ok(messages)
    }

    pub fn append_message(&self, session_id: &str, message: &Message) -> Result<(), String> {
        let path = self.session_path(session_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("failed to open session file for append: {e}"))?;
        let line = serde_json::to_string(message)
            .map_err(|e| format!("failed to serialize message: {e}"))?;
        writeln!(file, "{}", line)
            .map_err(|e| format!("failed to write message to session file: {e}"))?;
        Ok(())
    }

    pub fn save_session(&self, session_id: &str, messages: &[Message]) -> Result<(), String> {
        let path = self.session_path(session_id);
        let mut file = File::create(&path)
            .map_err(|e| format!("failed to create session file: {e}"))?;
        for msg in messages {
            let line = serde_json::to_string(msg)
                .map_err(|e| format!("failed to serialize message: {e}"))?;
            writeln!(file, "{}", line)
                .map_err(|e| format!("failed to write message: {e}"))?;
        }
        Ok(())
    }
}

/// A contributor that dynamically injects context or modifies turns in a session.
#[async_trait]
pub trait TurnContributor: Send + Sync {
    /// Modify or inject turns into the conversation history.
    async fn contribute(&self, messages: &mut Vec<Message>) -> Result<(), String>;
}

/// Builder pattern for compiling session turns using contributors.
pub struct SessionBuilder {
    session_id: String,
    manager: SessionManager,
    contributors: Vec<Box<dyn TurnContributor>>,
}

impl SessionBuilder {
    pub fn new(session_id: impl Into<String>, manager: SessionManager) -> Self {
        Self {
            session_id: session_id.into(),
            manager,
            contributors: Vec::new(),
        }
    }

    /// Add a contributor to the session builder.
    pub fn with_contributor(mut self, contributor: Box<dyn TurnContributor>) -> Self {
        self.contributors.push(contributor);
        self
    }

    /// Build the final list of messages, running all turn contributors in sequence.
    pub async fn build(self) -> Result<Vec<Message>, String> {
        let mut messages = self.manager.load_session(&self.session_id)?;
        for contributor in &self.contributors {
            contributor.contribute(&mut messages).await?;
        }
        Ok(messages)
    }
}

/// Handler trait for custom mock logic.
#[async_trait]
pub trait MockHandler: Send + Sync {
    async fn handle(
        &self,
        transcript: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String>;
}

/// A mock reasoning engine for unit tests and local simulations without LLM network calls.
pub struct MockReasoningEngine {
    transcripts: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    executed_tools: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    responses: std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>,
    default_response: Option<String>,
    handler: Option<std::sync::Arc<dyn MockHandler>>,
}

impl MockReasoningEngine {
    /// Create a new mock reasoning engine with an optional canned response.
    pub fn new(default_response: Option<String>) -> Self {
        Self {
            transcripts: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            executed_tools: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            responses: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            default_response,
            handler: None,
        }
    }

    /// Create with a deterministic string response.
    pub fn with_response(response: impl Into<String>) -> Self {
        Self::new(Some(response.into()))
    }

    /// Create a mock engine that fails with the given error message.
    pub fn with_error(error: impl Into<String>) -> Self {
        struct ErrorHandler(String);
        #[async_trait]
        impl MockHandler for ErrorHandler {
            async fn handle(
                &self,
                _transcript: &str,
                _executor: &dyn ToolExecutor,
            ) -> Result<Option<String>, String> {
                Err(self.0.clone())
            }
        }

        Self {
            transcripts: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            executed_tools: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            responses: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            default_response: None,
            handler: Some(std::sync::Arc::new(ErrorHandler(error.into()))),
        }
    }

    /// Create with a custom dynamic asynchronous handler implementing `MockHandler`.
    pub fn with_handler(handler: impl MockHandler + 'static) -> Self {
        Self {
            transcripts: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            executed_tools: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            responses: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            default_response: None,
            handler: Some(std::sync::Arc::new(handler)),
        }
    }

    /// Queue multiple canned responses to be returned in sequential order.
    pub fn with_queued_responses(responses: Vec<Option<String>>) -> Self {
        Self {
            transcripts: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            executed_tools: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            responses: std::sync::Arc::new(std::sync::Mutex::new(responses)),
            default_response: None,
            handler: None,
        }
    }

    /// Retrieve all transcripts processed by this mock engine.
    pub fn recorded_transcripts(&self) -> Vec<String> {
        self.transcripts.lock().unwrap().clone()
    }

    /// Retrieve all tool calls executed through this mock engine.
    pub fn recorded_tool_calls(&self) -> Vec<(String, serde_json::Value)> {
        self.executed_tools.lock().unwrap().clone()
    }

    /// Clear recorded transcripts and tool calls.
    pub fn clear_records(&self) {
        self.transcripts.lock().unwrap().clear();
        self.executed_tools.lock().unwrap().clear();
    }
}

#[async_trait]
impl ReasoningEngine for MockReasoningEngine {
    async fn process_transcript(
        &self,
        transcript: &str,
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        let msg = Message {
            role: "user".to_string(),
            content: transcript.to_string(),
            name: None,
            thought_signature: None,
        };
        self.process_turns(&[msg], executor).await
    }

    async fn process_turns(
        &self,
        turns: &[Message],
        executor: &dyn ToolExecutor,
    ) -> Result<Option<String>, String> {
        let last_content = turns.last().map(|m| m.content.as_str()).unwrap_or("");
        
        self.transcripts
            .lock()
            .unwrap()
            .push(last_content.to_string());

        if let Some(ref handler) = self.handler {
            return handler.handle(last_content, executor).await;
        }

        let mut queue = self.responses.lock().unwrap();
        if !queue.is_empty() {
            return Ok(queue.remove(0));
        }

        if last_content.contains("force_error") {
            return Err("simulated reasoning error".to_string());
        }

        Ok(self.default_response.clone())
    }
}

/// In-memory mock tool executor for testing reasoning engines without a live Herdr socket.
#[derive(Default, Clone)]
pub struct MockToolExecutor {
    calls: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
    canned_responses: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, serde_json::Value>>>,
}

impl MockToolExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_canned_response(self, tool_name: impl Into<String>, response: serde_json::Value) -> Self {
        self.canned_responses
            .lock()
            .unwrap()
            .insert(tool_name.into(), response);
        self
    }

    pub fn recorded_calls(&self) -> Vec<(String, serde_json::Value)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolExecutor for MockToolExecutor {
    async fn execute_tool(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.calls
            .lock()
            .unwrap()
            .push((name.to_string(), args.clone()));

        if let Some(resp) = self.canned_responses.lock().unwrap().get(name) {
            Ok(resp.clone())
        } else {
            Ok(serde_json::json!({
                "status": "success",
                "tool": name,
                "args": args
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_session_manager_save_load() {
        let dir = tempdir().unwrap();
        let manager = SessionManager::with_dir(dir.path().to_path_buf()).unwrap();
        let session_id = "test_session";

         let messages = vec![
            Message {
                role: "user".to_string(),
                content: "Hello".to_string(),
                name: None,
                thought_signature: None,
            },
            Message {
                role: "model".to_string(),
                content: "Hi there!".to_string(),
                name: None,
                thought_signature: Some("opaque_base64_sig".to_string()),
            },
        ];

        manager.save_session(session_id, &messages).unwrap();
        let loaded = manager.load_session(session_id).unwrap();
        assert_eq!(loaded, messages);

        // Test append
        let new_msg = Message {
            role: "user".to_string(),
            content: "How are you?".to_string(),
            name: None,
            thought_signature: None,
        };
        manager.append_message(session_id, &new_msg).unwrap();

        let loaded_after_append = manager.load_session(session_id).unwrap();
        assert_eq!(loaded_after_append.len(), 3);
        assert_eq!(loaded_after_append[2], new_msg);
    }

    #[tokio::test]
    async fn test_session_builder_with_contributor() {
        struct MockContributor;
        #[async_trait]
        impl TurnContributor for MockContributor {
            async fn contribute(&self, messages: &mut Vec<Message>) -> Result<(), String> {
                messages.push(Message {
                    role: "system".to_string(),
                    content: "Injected context".to_string(),
                    name: None,
                    thought_signature: None,
                });
                Ok(())
            }
        }

        let dir = tempdir().unwrap();
        let manager = SessionManager::with_dir(dir.path().to_path_buf()).unwrap();
        let session_id = "test_builder";

        let initial_msgs = vec![Message {
            role: "user".to_string(),
            content: "Hello".to_string(),
            name: None,
            thought_signature: None,
        }];
        manager.save_session(session_id, &initial_msgs).unwrap();

        let builder = SessionBuilder::new(session_id, manager)
            .with_contributor(Box::new(MockContributor));

        let built = builder.build().await.unwrap();
        assert_eq!(built.len(), 2);
        assert_eq!(built[0], initial_msgs[0]);
        assert_eq!(built[1].role, "system");
        assert_eq!(built[1].content, "Injected context");
    }
}
