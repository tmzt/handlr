//! `libherdr` — Transport and client library for the `herdr` terminal multiplexer.
//!
//! Provides the transport layer (Local Unix Domain Socket, Remote SSH with connection
//! multiplexing, and In-Memory Mock) and `ToolExecutor` bindings for LLM and voice-driven
//! terminal orchestration.

pub mod client;
pub mod error;
pub mod events;
pub mod protocol;
pub mod transport;

pub use client::HerdrClient;
pub use error::{HerdrError, HerdrResult};
pub use events::{HerdrEvent, PaneAgentStatusChanged, Subscription};
pub use protocol::{
    AgentSession, AgentStatus, EnvironmentSnapshot, HerdrRequest, HerdrResponse, LayoutAction,
    PaneInfo, ReadSource, TabInfo, WorkspaceInfo, DEFAULT_INSPECT_LINES,
};
pub use transport::{
    default_local_socket_path, HerdrEventStream, HerdrTransport, MockState, RemoteSshConfig,
    SshInvocationMethod,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::HerdrJsonRpcRequest;
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
        let req = HerdrRequest::inspect_pane("pane_9", 100);
        let rpc = req.to_jsonrpc();
        assert_eq!(rpc.method, "pane.read");
        assert_eq!(rpc.params["pane_id"], "pane_9");
        assert_eq!(rpc.params["count"], 100);
        assert_eq!(rpc.params["source"], "recent");
        assert_eq!(rpc.params["lines"], 100);

        let unwrapped =
            HerdrRequest::inspect_pane_from("pane_9", 80, ReadSource::RecentUnwrapped).to_jsonrpc();
        assert_eq!(unwrapped.params["source"], "recent_unwrapped");
        assert_eq!(unwrapped.params["lines"], 80);

        let cmd_req = HerdrRequest::run_command("w2:p1", "/compact");
        let cmd_rpc = cmd_req.to_jsonrpc();
        assert_eq!(cmd_rpc.method, "pane.send_input");
        assert_eq!(cmd_rpc.params["pane_id"], "w2:p1");
        assert_eq!(cmd_rpc.params["text"], "/compact");
        assert_eq!(cmd_rpc.params["keys"], serde_json::json!(["Enter"]));

        let serialized = serde_json::to_string(&rpc).unwrap();
        let parsed: HerdrJsonRpcRequest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.method, "pane.read");
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
                        if let Ok(req) = serde_json::from_str::<HerdrJsonRpcRequest>(line.trim()) {
                            let resp = match req.method.as_str() {
                                "session.snapshot" => json!({
                                    "id": req.id,
                                    "result": {
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
                                "pane.read" => json!({
                                    "id": req.id,
                                    "result": {
                                        "pane_id": req.params.get("pane_id"),
                                        "buffer": "Live buffer stream text\nLine 2"
                                    }
                                }),
                                "pane.focus" | "workspace.focus" | "tab.focus" => json!({
                                    "id": req.id,
                                    "result": {
                                        "focused_id": req.params.get("pane_id").or_else(|| req.params.get("workspace_id"))
                                    }
                                }),
                                _ => json!({"id": req.id, "result": {"status": "ok"}}),
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
        assert_eq!(exec_res["focused_id"], "ws-live");
    }

    #[tokio::test]
    async fn test_event_stream_over_real_unix_socket() {
        use crate::events::Subscription;
        use crate::protocol::AgentStatus;

        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("herdr_events.sock");
        let listener = UnixListener::bind(&sock_path).expect("failed to bind events UDS");

        // Stand-in herdr: acknowledge the subscription, push two events, close.
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.split();
            let mut buf_reader = BufReader::new(reader);
            let mut line = String::new();
            buf_reader.read_line(&mut line).await.unwrap();
            let req: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
            assert_eq!(req["method"], "events.subscribe");
            assert_eq!(
                req["params"]["subscriptions"][0]["type"],
                "pane.agent_status_changed"
            );
            assert_eq!(req["params"]["subscriptions"][0]["pane_id"], "w1:p6");

            for payload in [
                json!({"id": req["id"], "result": {"type": "subscription_started"}}),
                json!({"event": "pane_agent_status_changed", "data": {
                    "pane_id": "w1:p6", "workspace_id": "w1",
                    "agent_status": "blocked", "agent": "claude"}}),
                json!({"event": "pane_agent_status_changed", "data": {
                    "pane_id": "w1:p6", "workspace_id": "w1",
                    "agent_status": "done", "agent": "claude"}}),
            ] {
                let mut out = serde_json::to_vec(&payload).unwrap();
                out.push(b'\n');
                writer.write_all(&out).await.unwrap();
            }
        });

        let transport = HerdrTransport::local(&sock_path);
        let mut stream = transport
            .open_event_stream(&[Subscription::agent_status("w1:p6")])
            .await
            .expect("open event stream");

        assert_eq!(
            stream.next_event().await,
            Some(HerdrEvent::SubscriptionStarted)
        );
        match stream.next_event().await.expect("blocked event") {
            HerdrEvent::AgentStatusChanged(c) => {
                assert_eq!(c.agent_status, AgentStatus::Blocked);
                assert_eq!(c.pane_id, "w1:p6");
            }
            other => panic!("expected blocked, got {other:?}"),
        }
        match stream.next_event().await.expect("done event") {
            HerdrEvent::AgentStatusChanged(c) => assert_eq!(c.agent_status, AgentStatus::Done),
            other => panic!("expected done, got {other:?}"),
        }
        // Server hung up: the stream ends rather than stalling.
        assert_eq!(stream.next_event().await, None);
    }

    #[tokio::test]
    async fn test_event_stream_requires_subscriptions() {
        let transport = HerdrTransport::mock();
        assert!(transport.open_event_stream(&[]).await.is_err());
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

    #[test]
    fn test_pane_agent_session_is_typed() {
        // The shape of `herdr pane list` on 2026-10-23.
        let pane: PaneInfo = serde_json::from_value(json!({
            "pane_id": "w2:pZ",
            "agent": "claude",
            "agent_session": {"agent": "claude", "kind": "id", "source": "herdr:claude",
                              "value": "00000000-0000-4000-8000-000000000001"},
            "agent_status": "idle",
            "cwd": "/work/example"
        }))
        .unwrap();
        let session = pane.agent_session.as_ref().unwrap();
        assert_eq!(session.source, "herdr:claude");
        assert_eq!(session.id(), Some("00000000-0000-4000-8000-000000000001"));
        assert!(!pane.extra.contains_key("agent_session"));
        assert_eq!(pane.extra["agent"], "claude");
        // A path, or no session at all.
        let path = AgentSession {
            source: "x".into(),
            agent: "x".into(),
            kind: "path".into(),
            value: "/tmp/s.jsonl".into(),
        };
        assert_eq!(path.id(), None);
        let bare: PaneInfo = serde_json::from_value(json!({"pane_id": "w1:p1"})).unwrap();
        assert_eq!(bare.agent_session, None);
        let wire = serde_json::to_value(&bare).unwrap();
        assert!(wire.get("agent_session").is_none());
    }

    #[tokio::test]
    async fn test_mock_logs_the_read_source() {
        let client = HerdrClient::mock();
        client
            .inspect_pane_from("p1", 80, ReadSource::RecentUnwrapped)
            .await
            .unwrap();
        match client.transport() {
            HerdrTransport::Mock { state } => assert_eq!(
                state.read().await.read_log,
                [("p1".to_string(), ReadSource::RecentUnwrapped, 80)]
            ),
            _ => unreachable!(),
        }
    }
}
