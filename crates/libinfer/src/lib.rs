use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute_tool(&self, name: &str, args: &serde_json::Value) -> Result<serde_json::Value, String>;
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
}
