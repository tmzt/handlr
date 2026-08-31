//! Tool executor trait and implementations for bridge integration.

use async_trait::async_trait;
use serde_json::Value;

use crate::client::HerdrClient;
use crate::transport::HerdrTransport;

/// Asynchronous tool execution interface for voice and LLM agent dispatching.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool by name with arguments, returning the output as JSON.
    async fn execute_tool(&self, name: &str, args: &Value) -> Result<Value, String>;
}

#[async_trait]
impl ToolExecutor for HerdrClient {
    async fn execute_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        self.execute_tool_call(name, args)
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl ToolExecutor for HerdrTransport {
    async fn execute_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        let client = HerdrClient::new(self.clone());
        client.execute_tool(name, args).await
    }
}

#[async_trait]
impl<T: ToolExecutor + ?Sized> ToolExecutor for std::sync::Arc<T> {
    async fn execute_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        (**self).execute_tool(name, args).await
    }
}

#[async_trait]
impl<T: ToolExecutor + ?Sized> ToolExecutor for Box<T> {
    async fn execute_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        (**self).execute_tool(name, args).await
    }
}
