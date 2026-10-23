//! Transport implementations for `herdr` (Local Unix Domain Socket, Remote SSH, and Mock).

use log::trace;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::{mpsc, RwLock};

use crate::error::{HerdrError, HerdrResult};
use crate::events::{HerdrEvent, Subscription};
use crate::protocol::{
    AgentStatus, EnvironmentSnapshot, HerdrRequest, PaneInfo, ReadSource, TabInfo, WorkspaceInfo,
};

/// Default socket path on local Unix systems.
pub fn default_local_socket_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".config")
            .join("herdr")
            .join("herdr.sock")
    } else {
        PathBuf::from("/tmp/herdr.sock")
    }
}

/// Remote invocation strategy when bridging over SSH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshInvocationMethod {
    /// Pipes serialized JSON into `nc -U <socket_path>` on the remote host.
    SocketPipe,
    /// Executes `herdr cli` passing the JSON via standard input.
    HerdrCli,
    /// Custom shell command executed on the remote host.
    CustomCommand(String),
}

impl Default for SshInvocationMethod {
    fn default() -> Self {
        Self::SocketPipe
    }
}

/// Configuration for pure-Rust or OpenSSH-multiplexed remote connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSshConfig {
    /// Destination host (e.g. `user@host` or `192.168.1.50`).
    pub destination: String,
    /// Optional remote username if not specified in `destination`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Optional SSH port (default: 22).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Optional path to private key / identity file (`-i`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    /// Path to the remote herdr Unix Domain Socket (default: `/tmp/herdr.sock`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_socket_path: Option<PathBuf>,
    /// Path pattern for SSH ControlMaster multiplexing socket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_path: Option<PathBuf>,
    /// ControlPersist idle duration (default: "10m").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_persist: Option<String>,
    /// SSH client binary name or absolute path (default: "ssh").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_binary: Option<String>,
    /// Additional command-line flags passed directly to SSH.
    #[serde(default)]
    pub extra_options: Vec<String>,
    /// Invocation method for dispatching commands on the remote machine.
    #[serde(default)]
    pub invocation_method: SshInvocationMethod,
    /// Connection timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

impl RemoteSshConfig {
    /// Create a new configuration with defaults for the given destination.
    pub fn new(destination: impl Into<String>) -> Self {
        Self {
            destination: destination.into(),
            user: None,
            port: None,
            identity_file: None,
            remote_socket_path: Some(default_local_socket_path()),
            control_path: Some(PathBuf::from("/tmp/handlr_ssh_%C")),
            control_persist: Some("10m".to_string()),
            ssh_binary: Some("ssh".to_string()),
            extra_options: Vec::new(),
            invocation_method: SshInvocationMethod::SocketPipe,
            timeout_secs: Some(15),
        }
    }

    /// Set the remote username.
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Set the SSH port.
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Set an identity file (private key).
    pub fn with_identity_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.identity_file = Some(path.into());
        self
    }

    /// Set the remote Unix domain socket path.
    pub fn with_remote_socket_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.remote_socket_path = Some(path.into());
        self
    }

    /// Set custom extra SSH arguments.
    pub fn with_extra_options(mut self, options: Vec<String>) -> Self {
        self.extra_options = options;
        self
    }

    /// Build the SSH target string (e.g. `user@host` or `host`).
    pub fn target(&self) -> String {
        if let Some(ref user) = self.user {
            if !self.destination.contains('@') {
                return format!("{}@{}", user, self.destination);
            }
        }
        self.destination.clone()
    }
}

/// In-memory state for mock testing.
#[derive(Debug, Clone)]
pub struct MockState {
    pub snapshot: EnvironmentSnapshot,
    pub pane_buffers: std::collections::HashMap<String, Vec<String>>,
    pub command_log: Vec<(String, String)>,
    pub keystroke_log: Vec<(String, String)>,
    pub focused_target: Option<String>,
    /// Every pane read, in order: `(pane_id, source, lines)`.
    pub read_log: Vec<(String, ReadSource, usize)>,
}

impl Default for MockState {
    fn default() -> Self {
        let mut extra = serde_json::Map::new();
        extra.insert(
            "agent".to_string(),
            serde_json::Value::String("claude".to_string()),
        );
        let default_pane = PaneInfo {
            pane_id: "p1".to_string(),
            terminal_title: Some("claude-worker".to_string()),
            cwd: Some("/Users/tmeade/src/handlr".to_string()),
            agent_status: Some(AgentStatus::Working),
            focused: true,
            command: Some("cargo build".to_string()),
            agent_session: None,
            extra,
        };

        let default_workspace = WorkspaceInfo {
            workspace_id: "w1".to_string(),
            name: Some("handlr-dev".to_string()),
            tabs: vec![TabInfo {
                tab_id: "t1".to_string(),
                title: Some("build".to_string()),
                panes: vec![default_pane],
                focused: true,
            }],
            panes: Vec::new(),
            focused: true,
        };

        let mut buffers = std::collections::HashMap::new();
        buffers.insert(
            "p1".to_string(),
            vec![
                "Compiling handlr v0.1.0 (/Users/tmeade/src/handlr)".to_string(),
                "   Compiling libherdr v0.1.0 (/Users/tmeade/src/handlr/crates/libherdr)"
                    .to_string(),
                "    Finished dev [unoptimized + debuginfo] target(s) in 1.42s".to_string(),
            ],
        );

        Self {
            snapshot: EnvironmentSnapshot {
                workspaces: vec![default_workspace],
                active_workspace_id: Some("w1".to_string()),
                active_pane_id: Some("p1".to_string()),
            },
            pane_buffers: buffers,
            command_log: Vec::new(),
            keystroke_log: Vec::new(),
            focused_target: Some("p1".to_string()),
            read_log: Vec::new(),
        }
    }
}

/// A held-open subscription connection.
///
/// Unlike [`HerdrTransport::send_request`], which opens a connection per call,
/// this keeps one connection alive and yields every event herdr pushes down it.
/// Dropping the stream aborts the reader task, which closes the connection —
/// that is how a subscription set is changed, since herdr closes any connection
/// that receives a second `events.subscribe`.
pub struct HerdrEventStream {
    rx: mpsc::Receiver<HerdrEvent>,
    reader: tokio::task::JoinHandle<()>,
}

impl HerdrEventStream {
    /// Next event, or `None` once herdr closed the stream or the reader stopped.
    pub async fn next_event(&mut self) -> Option<HerdrEvent> {
        self.rx.recv().await
    }
}

impl Drop for HerdrEventStream {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Transport variants supported by `libherdr`.
#[derive(Clone)]
pub enum HerdrTransport {
    /// Connects to a local Unix Domain Socket.
    LocalSocket { socket_path: PathBuf },
    /// Executes requests remotely over SSH using multiplexed connection channels.
    RemoteSsh { config: RemoteSshConfig },
    /// In-memory mock transport for testing and deterministic simulation.
    Mock { state: Arc<RwLock<MockState>> },
}

impl HerdrTransport {
    /// Create a local Unix Domain Socket transport.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::LocalSocket {
            socket_path: path.into(),
        }
    }

    /// Create a local transport targeting the default socket path `/tmp/herdr.sock`.
    pub fn local_default() -> Self {
        Self::local(default_local_socket_path())
    }

    /// Create a remote SSH transport with destination `user@host`.
    pub fn remote_ssh(destination: impl Into<String>) -> Self {
        Self::RemoteSsh {
            config: RemoteSshConfig::new(destination),
        }
    }

    /// Create a remote SSH transport with custom configuration.
    pub fn remote_ssh_with_config(config: RemoteSshConfig) -> Self {
        Self::RemoteSsh { config }
    }

    /// Create an in-memory mock transport with default test fixtures.
    pub fn mock() -> Self {
        Self::Mock {
            state: Arc::new(RwLock::new(MockState::default())),
        }
    }

    /// Create an in-memory mock transport with custom state.
    pub fn mock_with_state(state: MockState) -> Self {
        Self::Mock {
            state: Arc::new(RwLock::new(state)),
        }
    }

    /// Open a live subscription connection and stream decoded events from it.
    ///
    /// The connection stays open until the returned [`HerdrEventStream`] is
    /// dropped or herdr closes it. Callers that need to change what they are
    /// subscribed to must drop the stream and open a new one.
    pub async fn open_event_stream(
        &self,
        subscriptions: &[Subscription],
    ) -> HerdrResult<HerdrEventStream> {
        if subscriptions.is_empty() {
            return Err(HerdrError::Socket(
                "Refusing to open an event stream with no subscriptions".to_string(),
            ));
        }
        let request = serde_json::json!({
            "id": crate::protocol::generate_request_id(),
            "method": "events.subscribe",
            "params": { "subscriptions": subscriptions },
        });
        let payload = {
            let mut bytes = serde_json::to_vec(&request)?;
            bytes.push(b'\n');
            bytes
        };

        // Bounded so a stalled consumer applies backpressure to the reader
        // rather than growing without limit on a chatty subscription.
        let (tx, rx) = mpsc::channel(256);

        let reader = match self {
            Self::LocalSocket { socket_path } => {
                let mut stream = UnixStream::connect(socket_path).await.map_err(|e| {
                    HerdrError::Socket(format!(
                        "Failed to connect to herdr socket at {:?} for events: {}",
                        socket_path, e
                    ))
                })?;
                stream.write_all(&payload).await.map_err(|e| {
                    HerdrError::Socket(format!("Failed to write events.subscribe: {}", e))
                })?;
                stream.flush().await.map_err(|e| {
                    HerdrError::Socket(format!("Failed to flush events.subscribe: {}", e))
                })?;
                let lines = BufReader::new(stream).lines();
                tokio::spawn(Self::pump_event_lines(lines, tx))
            }
            Self::RemoteSsh { config } => {
                let mut cmd = Self::build_ssh_command(config);
                cmd.stdin(std::process::Stdio::piped());
                cmd.stdout(std::process::Stdio::piped());
                cmd.stderr(std::process::Stdio::piped());
                cmd.kill_on_drop(true);

                let mut child = cmd.spawn().map_err(|e| {
                    HerdrError::Ssh(format!("Failed to spawn SSH process for events: {}", e))
                })?;
                let mut stdin = child.stdin.take().ok_or_else(|| {
                    HerdrError::Ssh("SSH child has no stdin for events.subscribe".to_string())
                })?;
                stdin.write_all(&payload).await.map_err(|e| {
                    HerdrError::Ssh(format!("Failed writing events.subscribe to SSH: {}", e))
                })?;
                stdin.flush().await.map_err(|e| {
                    HerdrError::Ssh(format!("Failed flushing events.subscribe to SSH: {}", e))
                })?;
                // Held for the life of the stream: dropping stdin can make the
                // remote `nc` see EOF and hang up the subscription.
                let stdout = child.stdout.take().ok_or_else(|| {
                    HerdrError::Ssh("SSH child has no stdout for events".to_string())
                })?;
                let lines = BufReader::new(stdout).lines();
                tokio::spawn(async move {
                    let _stdin = stdin;
                    let _child = child;
                    Self::pump_event_lines(lines, tx).await;
                })
            }
            Self::Mock { .. } => {
                // Mock transports have no event source; the stream stays open
                // and idle so callers can be exercised without a live herdr.
                tokio::spawn(async move {
                    tx.closed().await;
                })
            }
        };

        Ok(HerdrEventStream { rx, reader })
    }

    /// Decode lines off a subscription connection until it closes.
    async fn pump_event_lines<R>(
        mut lines: tokio::io::Lines<BufReader<R>>,
        tx: mpsc::Sender<HerdrEvent>,
    ) where
        R: tokio::io::AsyncRead + Unpin,
    {
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if let Some(event) = HerdrEvent::from_line(&line) {
                        if tx.send(event).await.is_err() {
                            log::debug!("libherdr: event consumer dropped; closing subscription");
                            return;
                        }
                    }
                }
                Ok(None) => {
                    log::info!("libherdr: herdr closed the event subscription");
                    return;
                }
                Err(e) => {
                    log::warn!("libherdr: error reading event subscription: {e}");
                    return;
                }
            }
        }
    }

    /// Execute a typed `HerdrRequest` and return the resulting JSON `Value`.
    pub async fn send_request(&self, request: &HerdrRequest) -> HerdrResult<Value> {
        match self {
            Self::LocalSocket { socket_path } => {
                Self::execute_local_socket(socket_path, request).await
            }
            Self::RemoteSsh { config } => Self::execute_remote_ssh(config, request).await,
            Self::Mock { state } => Self::execute_mock(state, request).await,
        }
    }

    /// Execute a raw JSON `Value` directly.
    pub async fn send_raw(&self, raw: &Value) -> HerdrResult<Value> {
        let request: HerdrRequest =
            serde_json::from_value(raw.clone()).map_err(|e| HerdrError::InvalidArguments {
                tool: "raw".to_string(),
                details: e.to_string(),
            })?;
        self.send_request(&request).await
    }

    /// Internal executor for Local Unix Domain Sockets.
    async fn execute_local_socket(
        socket_path: &Path,
        request: &HerdrRequest,
    ) -> HerdrResult<Value> {
        let rpc = request.to_jsonrpc();
        let req_json = serde_json::to_string(&rpc).unwrap_or_default();
        log::info!(
            "libherdr: Connecting to Unix socket at {:?} (method: '{}', id: {})",
            socket_path,
            rpc.method,
            rpc.id
        );
        log::debug!("libherdr: Sending JSON-RPC payload to socket: {}", req_json);

        let mut stream = UnixStream::connect(socket_path).await.map_err(|e| {
            log::warn!(
                "libherdr: Failed connecting to Unix socket at {:?}: {}",
                socket_path,
                e
            );
            HerdrError::Socket(format!(
                "Failed to connect to herdr socket at {:?}: {}",
                socket_path, e
            ))
        })?;

        let mut payload = serde_json::to_vec(&rpc)?;
        payload.push(b'\n');

        stream.write_all(&payload).await.map_err(|e| {
            log::warn!("libherdr: Failed writing request to socket: {}", e);
            HerdrError::Socket(format!("Failed to write request to socket: {}", e))
        })?;
        stream.flush().await.map_err(|e| {
            log::warn!("libherdr: Failed flushing socket write: {}", e);
            HerdrError::Socket(format!("Failed to flush socket write: {}", e))
        })?;

        let mut reader = BufReader::new(stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line).await.map_err(|e| {
            log::warn!("libherdr: Failed reading response from socket: {}", e);
            HerdrError::Socket(format!("Failed to read response from socket: {}", e))
        })?;

        let trimmed_resp = response_line.trim();
        log::debug!(
            "libherdr: Raw response line from socket: '{}'",
            trimmed_resp
        );
        if trimmed_resp.is_empty() {
            log::warn!("libherdr: Received empty response from socket");
            return Err(HerdrError::Socket(
                "Received empty response from socket".to_string(),
            ));
        }

        let parsed = Self::parse_herdr_response_json(trimmed_resp)?;
        log::info!(
            "libherdr: Successfully received and parsed socket response for method '{}'",
            rpc.method
        );
        Ok(parsed)
    }

    /// Build the `ssh` invocation for a remote herdr socket: multiplexing
    /// flags, connection options, and the remote command that bridges stdio to
    /// the socket. Shared by the one-shot request path and the subscription
    /// path so both reach herdr the same way.
    fn build_ssh_command(config: &RemoteSshConfig) -> Command {
        let binary = config.ssh_binary.as_deref().unwrap_or("ssh");
        let mut cmd = Command::new(binary);

        // Configure multiplexing and connection flags
        cmd.arg("-o").arg("BatchMode=yes");
        cmd.arg("-o").arg("ControlMaster=auto");

        if let Some(ref cp) = config.control_path {
            cmd.arg("-o").arg(format!("ControlPath={}", cp.display()));
        }
        if let Some(ref persist) = config.control_persist {
            cmd.arg("-o").arg(format!("ControlPersist={}", persist));
        }
        if let Some(port) = config.port {
            cmd.arg("-p").arg(port.to_string());
        }
        if let Some(ref id) = config.identity_file {
            cmd.arg("-i").arg(id.display().to_string());
        }
        for opt in &config.extra_options {
            cmd.arg(opt);
        }

        let target = config.target();
        cmd.arg(&target);

        let default_sock = default_local_socket_path();
        let remote_sock = config
            .remote_socket_path
            .as_deref()
            .unwrap_or_else(|| &default_sock)
            .to_string_lossy();

        // Determine remote command invocation
        match &config.invocation_method {
            SshInvocationMethod::SocketPipe => {
                // Pipe to nc -U <socket> or fallback to socat
                let remote_cmd = format!(
                    "if command -v nc >/dev/null 2>&1; then nc -U '{}'; elif command -v socat >/dev/null 2>&1; then socat - UNIX-CONNECT:'{}'; else echo '{{\"success\":false,\"error\":\"Neither nc nor socat found on remote host\"}}'; fi",
                    remote_sock, remote_sock
                );
                cmd.arg(remote_cmd);
            }
            SshInvocationMethod::HerdrCli => {
                let remote_cmd = format!("herdr cli --socket '{}'", remote_sock);
                cmd.arg(remote_cmd);
            }
            SshInvocationMethod::CustomCommand(template) => {
                let remote_cmd = template.replace("{socket}", &remote_sock);
                cmd.arg(remote_cmd);
            }
        }

        cmd
    }

    /// Internal executor for Remote SSH Multiplexed connections.
    async fn execute_remote_ssh(
        config: &RemoteSshConfig,
        request: &HerdrRequest,
    ) -> HerdrResult<Value> {
        let rpc = request.to_jsonrpc();
        let binary = config.ssh_binary.as_deref().unwrap_or("ssh");
        let mut cmd = Self::build_ssh_command(config);

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        trace!("Spawning SSH process: {:?}", cmd);
        let mut child = cmd.spawn().map_err(|e| {
            HerdrError::Ssh(format!("Failed to spawn SSH process '{}': {}", binary, e))
        })?;

        let request_bytes = serde_json::to_vec(&rpc)?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&request_bytes).await.map_err(|e| {
                HerdrError::Ssh(format!("Failed writing payload to SSH stdin: {}", e))
            })?;
            stdin.write_all(b"\n").await.map_err(|e| {
                HerdrError::Ssh(format!("Failed writing newline to SSH stdin: {}", e))
            })?;
            stdin
                .flush()
                .await
                .map_err(|e| HerdrError::Ssh(format!("Failed flushing SSH stdin: {}", e)))?;
        }

        let timeout_duration = Duration::from_secs(config.timeout_secs.unwrap_or(15));
        let output = tokio::time::timeout(timeout_duration, child.wait_with_output())
            .await
            .map_err(|_| HerdrError::Timeout)?
            .map_err(|e| HerdrError::Ssh(format!("SSH execution failed: {}", e)))?;

        if !output.status.success() {
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            return Err(HerdrError::Ssh(format!(
                "SSH remote process exited with status {}: {}",
                output.status,
                stderr_str.trim()
            )));
        }

        let stdout_str = String::from_utf8_lossy(&output.stdout);
        if stdout_str.trim().is_empty() {
            return Err(HerdrError::Ssh(
                "Remote command returned empty output".to_string(),
            ));
        }

        Self::parse_herdr_response_json(&stdout_str)
    }

    /// Internal executor for In-Memory Mock.
    async fn execute_mock(
        state_lock: &Arc<RwLock<MockState>>,
        request: &HerdrRequest,
    ) -> HerdrResult<Value> {
        let mut state = state_lock.write().await;
        match request {
            HerdrRequest::ListPanes { .. } => {
                let snapshot = &state.snapshot;
                serde_json::to_value(snapshot).map_err(HerdrError::from)
            }
            HerdrRequest::InspectPane {
                pane_id,
                lines,
                source,
                ..
            } => {
                state.read_log.push((pane_id.clone(), *source, *lines));
                let buffer = state
                    .pane_buffers
                    .get(pane_id)
                    .cloned()
                    .unwrap_or_else(|| vec![format!("Mock terminal buffer for {}", pane_id)]);

                let start_idx = buffer.len().saturating_sub(*lines);
                let tail_lines = &buffer[start_idx..];
                let joined = tail_lines.join("\n");

                Ok(serde_json::json!({
                    "pane_id": pane_id,
                    "lines_returned": tail_lines.len(),
                    "buffer": joined,
                }))
            }
            HerdrRequest::RunCommand {
                pane_id, command, ..
            } => {
                state.command_log.push((pane_id.clone(), command.clone()));
                if let Some(buf) = state.pane_buffers.get_mut(pane_id) {
                    buf.push(format!("$ {}", command));
                    buf.push("Command executed successfully (mock)".to_string());
                }
                Ok(serde_json::json!({
                    "success": true,
                    "pane_id": pane_id,
                    "command": command,
                    "status": "started"
                }))
            }
            HerdrRequest::SendKeys { pane_id, keys, .. } => {
                state.keystroke_log.push((pane_id.clone(), keys.clone()));
                if let Some(buf) = state.pane_buffers.get_mut(pane_id) {
                    buf.push(format!("[keys: {}]", keys));
                }
                Ok(serde_json::json!({
                    "success": true,
                    "pane_id": pane_id,
                    "keys_sent": keys,
                }))
            }
            HerdrRequest::ManageLayout {
                layout_action,
                target_id,
                ..
            } => Ok(serde_json::json!({
                "success": true,
                "action": layout_action,
                "target_id": target_id,
            })),
            HerdrRequest::Focus { target_id, .. } => {
                state.focused_target = Some(target_id.clone());
                state.snapshot.active_pane_id = Some(target_id.clone());
                Ok(serde_json::json!({
                    "success": true,
                    "focused_id": target_id,
                }))
            }
            HerdrRequest::Custom(val) => Ok(val.clone()),
        }
    }

    /// Helper to parse response string (handling structured `HerdrResponse`, JSON-RPC, and raw JSON objects).
    fn parse_herdr_response_json(text: &str) -> HerdrResult<Value> {
        let trimmed = text.trim();
        let val: Value = serde_json::from_str(trimmed)?;

        // Check for JSON-RPC or envelope error objects: { "error": { "code": ..., "message": ... } } or { "error": "..." }
        if let Some(err_obj) = val.get("error") {
            if !err_obj.is_null() {
                let err_msg = if let Some(msg) = err_obj.get("message").and_then(|m| m.as_str()) {
                    msg.to_string()
                } else if let Some(msg_str) = err_obj.as_str() {
                    msg_str.to_string()
                } else {
                    err_obj.to_string()
                };
                log::warn!("libherdr: Multiplexer server reported error: {}", err_msg);
                return Err(HerdrError::ServerError(err_msg));
            }
        }

        // If the response follows { "success": false, ... }
        if let Some(success) = val.get("success").and_then(|v| v.as_bool()) {
            if !success {
                let err_msg = val
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Server reported failure")
                    .to_string();
                log::warn!("libherdr: Multiplexer server reported failure: {}", err_msg);
                return Err(HerdrError::ServerError(err_msg));
            }
        }

        // If the response follows JSON-RPC { "result": ... } pattern:
        if let Some(result) = val.get("result") {
            return Ok(result.clone());
        }

        // If the response follows { "data": ... } pattern:
        if let Some(data) = val.get("data") {
            return Ok(data.clone());
        }

        // Otherwise, return the entire JSON object / array directly
        Ok(val)
    }
}
