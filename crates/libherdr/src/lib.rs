//! `libherdr` — Transport and client library for the `herdr` terminal multiplexer.
//!
//! Provides the transport layer (Local Unix Domain Socket, Remote SSH with connection
//! multiplexing, and In-Memory Mock) and `ToolExecutor` bindings for LLM and voice-driven
//! terminal orchestration.

pub mod client;
pub mod error;
pub mod protocol;
pub mod transport;

pub use client::HerdrClient;
pub use error::{HerdrError, HerdrResult};
pub use protocol::{
    AgentStatus, EnvironmentSnapshot, HerdrRequest, HerdrResponse, LayoutAction, PaneInfo,
    TabInfo, WorkspaceInfo, DEFAULT_INSPECT_LINES,
};
pub use transport::{
    HerdrTransport, MockState, RemoteSshConfig, SshInvocationMethod, default_local_socket_path,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn test_mock_list_panes() {
        let client = HerdrClient::mock();
        let val = client.list_panes().await.expect("list_panes failed");
        assert!(val.get("workspaces").is_some());

        let snapshot: EnvironmentSnapshot =
            serde_json::from_value(val).expect("failed to deserialize snapshot");
        assert_eq!(snapshot.workspaces.len(), 1);
        let working = snapshot.find_working_panes();
        assert_eq!(working.len(), 1);
        assert_eq!(working[0].pane_id, "p1");
    }

    #[tokio::test]
    async fn test_mock_inspect_pane() {
        let client = HerdrClient::mock();
        let val = client
            .inspect_pane("p1", 50)
            .await
            .expect("inspect_pane failed");
        let buffer = val
            .get("buffer")
            .and_then(|v| v.as_str())
            .expect("buffer should be string");
        assert!(buffer.contains("Finished dev"));
    }

    #[tokio::test]
    async fn test_mock_run_command_and_send_keys() {
        let client = HerdrClient::mock();
        client
            .run_command("p1", "echo hello")
            .await
            .expect("run_command failed");
        client
            .send_keys("p1", "ctrl+c")
            .await
            .expect("send_keys failed");

        let val = client
            .inspect_pane("p1", 50)
            .await
            .expect("inspect_pane failed");
        let buffer = val.get("buffer").unwrap().as_str().unwrap();
        assert!(buffer.contains("echo hello"));
        assert!(buffer.contains("[keys: ctrl+c]"));
    }

    #[tokio::test]
    async fn test_tool_executor_dispatch() {
        let client = HerdrClient::mock();

        // 1. list_panes
        let res = client
            .execute_tool_call("list_panes", &json!({}))
            .await
            .expect("execute list_panes");
        assert!(res.get("workspaces").is_some());

        // 2. inspect_pane
        let res = client
            .execute_tool_call("inspect_pane", &json!({"pane_id": "p1", "lines": 10}))
            .await
            .expect("execute inspect_pane");
        assert!(res.get("buffer").is_some());

        // 3. run_command
        let res = client
            .execute_tool_call(
                "run_command",
                &json!({"pane_id": "p1", "command": "git status"}),
            )
            .await
            .expect("execute run_command");
        assert_eq!(res["status"], "started");

        // 4. send_keys
        let res = client
            .execute_tool_call("send_keys", &json!({"pane_id": "p1", "keys": "enter"}))
            .await
            .expect("execute send_keys");
        assert_eq!(res["keys_sent"], "enter");

        // 5. manage_layout
        let res = client
            .execute_tool_call(
                "manage_layout",
                &json!({"action": "split_vertical", "target_id": "p1"}),
            )
            .await
            .expect("execute manage_layout");
        assert_eq!(res["action"], "split_vertical");

        // 6. focus
        let res = client
            .execute_tool_call("focus", &json!({"target_id": "w1"}))
            .await
            .expect("execute focus");
        assert_eq!(res["focused_id"], "w1");
    }

    #[tokio::test]
    async fn test_protocol_serialization() {
        let req = HerdrRequest::InspectPane {
            pane_id: "pane_9".to_string(),
            lines: 100,
        };
        let serialized = serde_json::to_string(&req).unwrap();
        assert_eq!(
            serialized,
            r#"{"action":"inspect_pane","pane_id":"pane_9","lines":100}"#
        );

        let parsed: HerdrRequest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn test_ssh_config_builder() {
        let config = RemoteSshConfig::new("remote-host")
            .with_user("alice")
            .with_port(2222)
            .with_identity_file("/home/alice/.ssh/id_ed25519")
            .with_remote_socket_path("/tmp/custom_herdr.sock");

        assert_eq!(config.target(), "alice@remote-host");
        assert_eq!(config.port, Some(2222));
        assert_eq!(
            config.remote_socket_path.unwrap().to_str().unwrap(),
            "/tmp/custom_herdr.sock"
        );
    }

    #[tokio::test]
    async fn test_real_unix_socket_transport() {
        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("herdr_test.sock");

        let listener = UnixListener::bind(&sock_path).expect("failed to bind test UDS");

        // Spawn mock server
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (reader, mut writer) = stream.split();
                    let mut buf_reader = BufReader::new(reader);
                    let mut line = String::new();
                    if buf_reader.read_line(&mut line).await.is_ok() {
                        if let Ok(req) = serde_json::from_str::<HerdrRequest>(line.trim()) {
                            let resp = match req {
                                HerdrRequest::ListPanes { .. } => json!({
                                    "success": true,
                                    "data": {
                                        "workspaces": [{
                                            "workspace_id": "ws-live",
                                            "name": "Live Server",
                                            "tabs": [],
                                            "panes": [{
                                                "pane_id": "p-live",
                                                "terminal_title": "live-server",
                                                "agent_status": "working",
                                                "focused": true
                                            }],
                                            "focused": true
                                        }]
                                    }
                                }),
                                HerdrRequest::InspectPane { pane_id, .. } => json!({
                                    "success": true,
                                    "data": {
                                        "pane_id": pane_id,
                                        "buffer": "Live buffer stream text\nLine 2"
                                    }
                                }),
                                HerdrRequest::Focus { target_id } => json!({
                                    "success": true,
                                    "data": {
                                        "focused_target": target_id
                                    }
                                }),
                                _ => json!({"success": true, "data": {"status": "ok"}}),
                            };
                            let mut out = serde_json::to_vec(&resp).unwrap();
                            out.push(b'\n');
                            let _ = writer.write_all(&out).await;
                        }
                    }
                });
            }
        });

        // Test client connection to the real Unix socket
        let client = HerdrClient::local(&sock_path);

        // Test list_panes
        let panes_val = client.list_panes().await.expect("list_panes over socket");
        assert!(panes_val.get("workspaces").is_some());

        // Test inspect_pane
        let inspect_val = client
            .inspect_pane("p-live", 10)
            .await
            .expect("inspect_pane over socket");
        let buffer = inspect_val.get("buffer").unwrap().as_str().unwrap();
        assert!(buffer.contains("Live buffer stream text"));

        // Test execute_tool_call
        let exec_res = client
            .execute_tool_call("focus", &json!({"target_id": "ws-live"}))
            .await
            .expect("execute focus over socket");
        assert_eq!(exec_res["focused_target"], "ws-live");
    }

    #[tokio::test]
    async fn test_tool_executor_error_handling() {
        let client = HerdrClient::mock();

        // Unknown tool
        let err = client
            .execute_tool_call("unknown_tool", &json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Unknown tool name"));

        // Missing required parameter
        let err = client
            .execute_tool_call("inspect_pane", &json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("requires 'pane_id'"));
    }
}
