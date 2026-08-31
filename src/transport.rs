//! Transport layer for `handlr`, bridging to the `libherdr` crate and multiplexer backend.

pub use libherdr::{
    client::HerdrClient,
    error::{HerdrError, HerdrResult},
    executor::ToolExecutor,
    protocol::{
        AgentStatus, EnvironmentSnapshot, HerdrRequest, HerdrResponse, LayoutAction, PaneInfo,
        TabInfo, WorkspaceInfo, DEFAULT_INSPECT_LINES,
    },
    transport::{
        HerdrTransport, MockState, RemoteSshConfig, SshInvocationMethod, DEFAULT_LOCAL_SOCKET_PATH,
    },
};
