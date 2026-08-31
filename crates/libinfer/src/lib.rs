use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute_tool(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String>;
}

// Re-export key libgenai types
pub use libgenai::{GeminiAgent, Tool, FunctionDeclaration, Model, Role, Message as GenaiMessage, Content};

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
        let trimmed = transcript.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        let response = self
            .agent
            .query_multi_turn(trimmed, self.max_turns, |name, args| async move {
                executor.execute_tool(&name, &args).await
            })
            .await?;

        let text = response.trim();
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text.to_string()))
        }
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
