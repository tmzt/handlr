//! High-level client for interacting with the `herdr` terminal multiplexer.

use serde_json::Value;
use std::path::PathBuf;

use crate::error::{HerdrError, HerdrResult};
use crate::protocol::{DEFAULT_INSPECT_LINES, EnvironmentSnapshot, HerdrRequest, ReadSource};
use crate::transport::{HerdrTransport, MockState, RemoteSshConfig};

/// High-level client providing structured and ergonomic access to `herdr`.
#[derive(Clone)]
pub struct HerdrClient {
    transport: HerdrTransport,
}

impl HerdrClient {
    /// Create a new client wrapping the specified transport.
    pub fn new(transport: HerdrTransport) -> Self {
        Self { transport }
    }

    /// Create a client targeting a local Unix Domain Socket.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::new(HerdrTransport::local(path))
    }

    /// Create a client targeting the default local socket path (`/tmp/herdr.sock`).
    pub fn local_default() -> Self {
        Self::new(HerdrTransport::local_default())
    }

    /// Create a client targeting a remote host via SSH.
    pub fn remote(destination: impl Into<String>) -> Self {
        Self::new(HerdrTransport::remote_ssh(destination))
    }

    /// Create a client with full custom SSH configuration.
    pub fn remote_with_config(config: RemoteSshConfig) -> Self {
        Self::new(HerdrTransport::remote_ssh_with_config(config))
    }

    /// Create an in-memory mock client for unit testing.
    pub fn mock() -> Self {
        Self::new(HerdrTransport::mock())
    }

    /// Create a mock client with a custom state fixture.
    pub fn mock_with_state(state: MockState) -> Self {
        Self::new(HerdrTransport::mock_with_state(state))
    }

    /// Create a client from standard environment variables:
    /// - `HERDR_MOCK`: If set to `1` or `true`, uses in-memory mock.
    /// - `HERDR_REMOTE`: If set (e.g. `user@server`), uses SSH remote mode.
    /// - `HERDR_SOCK`: Local socket path (defaults to `/tmp/herdr.sock`).
    pub fn from_env() -> HerdrResult<Self> {
        if let Ok(mock_val) = std::env::var("HERDR_MOCK")
            && (mock_val == "1" || mock_val.eq_ignore_ascii_case("true"))
        {
            return Ok(Self::mock());
        }

        if let Ok(remote_dest) = std::env::var("HERDR_REMOTE")
            && !remote_dest.trim().is_empty()
        {
            let mut config = RemoteSshConfig::new(remote_dest.trim());
            if let Ok(sock) = std::env::var("HERDR_REMOTE_SOCK") {
                config = config.with_remote_socket_path(PathBuf::from(sock));
            }
            return Ok(Self::remote_with_config(config));
        }

        let socket_path = std::env::var("HERDR_SOCK")
            .map(PathBuf::from)
            .unwrap_or_else(|_| crate::transport::default_local_socket_path());

        Ok(Self::local(socket_path))
    }

    /// Access the underlying transport reference.
    pub fn transport(&self) -> &HerdrTransport {
        &self.transport
    }

    /// Send an arbitrary `HerdrRequest` to the multiplexer.
    pub async fn send_request(&self, request: &HerdrRequest) -> HerdrResult<Value> {
        self.transport.send_request(request).await
    }

    /// List all workspaces, tabs, and panes as raw JSON.
    pub async fn list_panes(&self) -> HerdrResult<Value> {
        self.send_request(&HerdrRequest::list_panes()).await
    }

    /// List panes and deserialize into a strongly-typed `EnvironmentSnapshot`.
    pub async fn list_panes_snapshot(&self) -> HerdrResult<EnvironmentSnapshot> {
        let val = self.list_panes().await?;
        let snapshot: EnvironmentSnapshot = serde_json::from_value(val)?;
        Ok(snapshot)
    }

    /// Read the tail buffer of a terminal pane.
    pub async fn inspect_pane(&self, pane_id: &str, lines: usize) -> HerdrResult<Value> {
        let req = HerdrRequest::inspect_pane(pane_id, lines);
        self.send_request(&req).await
    }

    /// Read the last `lines` lines of `source` from a terminal pane.
    pub async fn inspect_pane_from(
        &self,
        pane_id: &str,
        lines: usize,
        source: ReadSource,
    ) -> HerdrResult<Value> {
        let req = HerdrRequest::inspect_pane_from(pane_id, lines, source);
        self.send_request(&req).await
    }

    /// Inspect a pane with default number of lines (50).
    pub async fn inspect_pane_default(&self, pane_id: &str) -> HerdrResult<Value> {
        self.inspect_pane(pane_id, DEFAULT_INSPECT_LINES).await
    }

    /// Execute a command in the target pane.
    pub async fn run_command(&self, pane_id: &str, command: &str) -> HerdrResult<Value> {
        let req = HerdrRequest::run_command(pane_id, command);
        self.send_request(&req).await
    }

    /// Send literal keystrokes to the target pane.
    pub async fn send_keys(&self, pane_id: &str, keys: &str) -> HerdrResult<Value> {
        let req = HerdrRequest::send_keys(pane_id, keys);
        self.send_request(&req).await
    }

    /// Manage layout (split, zoom, close, etc.).
    pub async fn manage_layout(&self, action: &str, target_id: Option<&str>) -> HerdrResult<Value> {
        let req = HerdrRequest::manage_layout(action, target_id);
        self.send_request(&req).await
    }

    /// Change user's visual focus to a specific pane or workspace.
    pub async fn focus(&self, target_id: &str) -> HerdrResult<Value> {
        let req = HerdrRequest::focus(target_id);
        self.send_request(&req).await
    }

    /// Execute a tool call dynamically based on tool name and JSON argument object.
    pub async fn execute_tool_call(&self, name: &str, args: &Value) -> HerdrResult<Value> {
        let request =
            HerdrRequest::from_tool_call(name, args).map_err(|e| HerdrError::InvalidArguments {
                tool: name.to_string(),
                details: e,
            })?;
        self.send_request(&request).await
    }
}
