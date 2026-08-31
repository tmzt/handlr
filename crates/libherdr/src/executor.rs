//! Tool executor trait and implementations for bridge integration.

use async_trait::async_trait;
use serde_json::Value;

use crate::client::HerdrClient;
use crate::transport::HerdrTransport;

pub use libinfer::ToolExecutor;

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
