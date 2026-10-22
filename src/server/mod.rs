//! `handlr serve`: the daemon (plan 20261022_01, decisions 7–9 and 11).
//!
//! One Unix socket, mode 0600, one JSON-RPC object per line. Each
//! connection says `hello` first, then sends any of the common methods;
//! requests on a connection run concurrently and `cancel` aborts one.

pub mod conn;
pub mod link;
mod watcher;

use std::io::IsTerminal;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use libherdr::client::HerdrClient;
use libherdr::protocol::AgentStatus;
use libherdr::transport::HerdrTransport;
use tokio::net::UnixListener;
use tokio::sync::broadcast;

use crate::config::{AgentConfig, AppConfig, NamingConfig};
use crate::core::confirm::{Broker, ConfirmEvent, ConfirmView, DEFAULT_TTL};
use crate::core::naming::{self, Names};
use crate::watch::WatchRegistry;

/// The socket `handlr serve` listens on unless `[server] socket` says otherwise.
pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("handlr")
        .join("handlr.sock")
}

/// Events from herdr that connections may subscribe to.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    AgentStatus {
        pane_id: String,
        agent: String,
        status: AgentStatus,
    },
}

/// Everything a connection needs, shared by all of them.
pub struct Daemon {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
    /// How agents are named and addressed (`[naming]`).
    pub naming: NamingConfig,
    pub broker: Arc<Broker>,
    pub watches: Arc<WatchRegistry>,
    pub events: broadcast::Sender<CoreEvent>,
    /// The daemon's own uid. A connecting peer must have the same one.
    pub uid: u32,
    /// The link for paired devices, when it is on.
    pub link: Option<Arc<link::Link>>,
}

impl Daemon {
    /// A daemon without the link.
    pub fn new(client: HerdrClient, config: &AppConfig) -> Arc<Self> {
        Self::with_link(client, config, None)
    }

    /// A daemon serving `link` too (see `link::spawn`).
    pub fn with_link(
        client: HerdrClient,
        config: &AppConfig,
        link: Option<Arc<link::Link>>,
    ) -> Arc<Self> {
        let ttl = config
            .server
            .confirm_ttl
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_TTL);
        let (events, _) = broadcast::channel(256);
        Arc::new(Daemon {
            broker: Broker::new(Arc::new(client.clone()), ttl),
            client,
            agents: config.agents.clone(),
            naming: config.naming.clone(),
            watches: Arc::new(WatchRegistry::new()),
            events,
            // SAFETY: getuid has no preconditions and cannot fail.
            uid: unsafe { libc::getuid() },
            link,
        })
    }

    /// Every pane in `snapshot`, named by `[naming]`.
    pub fn names(&self, snapshot: &serde_json::Value) -> Names {
        Names::from_snapshot(snapshot, &self.naming)
    }

    /// The names of `pane_id` from a fresh snapshot, or `None` when herdr
    /// cannot be read within `NAME_LOOKUP_TIMEOUT` or has no such pane.
    pub async fn names_for(&self, pane_id: &str) -> Option<PaneNames> {
        let snapshot = tokio::time::timeout(NAME_LOOKUP_TIMEOUT, self.client.list_panes())
            .await
            .ok()?
            .ok()?;
        let names = self.names(&snapshot);
        let named = names.get(pane_id)?;
        Some(PaneNames {
            name: named.shown.clone(),
            spoken: named.spoken.clone(),
            project: named.agent.project.clone(),
            tab: named.tab.as_ref().map(|t| t.shown.clone()),
        })
    }
}

/// How long a name lookup waits for herdr before going without names.
pub const NAME_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// One pane's names, as messages and token lines carry them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneNames {
    /// The shown name ("Claude in libmorph, tab -mb").
    pub name: String,
    /// The spoken name ("Claude in libmorph, tab em bee").
    pub spoken: String,
    /// As `agents.list` reports it.
    pub project: Option<String>,
    /// The tab as shown: its label as written, an alias, or its number.
    pub tab: Option<String>,
}

/// Bind the daemon socket at `path`.
///
/// A socket file that refuses connections is stale and is replaced; one
/// that accepts them belongs to a running daemon, which is an error.
pub async fn bind(path: &Path) -> Result<UnixListener, String> {
    if path.exists() {
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return Err(format!(
                "another handlr daemon is already listening on {}",
                path.display()
            ));
        }
        log::info!("Removing stale socket {}", path.display());
        std::fs::remove_file(path).map_err(|e| format!("remove {}: {e}", path.display()))?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let listener = UnixListener::bind(path).map_err(|e| format!("bind {}: {e}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("chmod {}: {e}", path.display()))?;
    Ok(listener)
}

/// Options for `run`.
pub struct ServeOptions {
    /// Print each confirmation token to standard output (never to the log).
    /// Defaults to whether standard output is a terminal.
    pub show_tokens: bool,
    /// The herdr transport for the event subscription, which needs its own
    /// held-open connection. `None` runs without agent status events.
    pub event_transport: Option<HerdrTransport>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        ServeOptions {
            show_tokens: std::io::stdout().is_terminal(),
            event_transport: None,
        }
    }
}

/// Serve connections on `listener` until the process ends.
pub async fn run(daemon: Arc<Daemon>, listener: UnixListener, options: ServeOptions) {
    if options.show_tokens {
        tokio::spawn(print_tokens(Arc::clone(&daemon), daemon.broker.subscribe()));
    }
    if let Some(transport) = options.event_transport {
        tokio::spawn(watcher::run(
            transport,
            daemon.client.clone(),
            Arc::clone(&daemon.watches),
            daemon.events.clone(),
        ));
    }
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(conn::handle(Arc::clone(&daemon), stream));
            }
            Err(e) => {
                log::warn!("accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// `confirm <id> [<handle>] (<agent>, tab <tab>): <token>` on standard
/// output, so a local client can approve with the token read off the
/// daemon's terminal. The agent is named, never its pane.
async fn print_tokens(daemon: Arc<Daemon>, mut events: broadcast::Receiver<ConfirmEvent>) {
    loop {
        match events.recv().await {
            Ok(ConfirmEvent::Requested { view, token, .. }) => {
                let names = daemon.names_for(&view.pane_id).await;
                println!(
                    "{}",
                    token_line(&view, names.as_ref(), token.expose_for_terminal())
                );
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// One line of `serve`'s token output. The agent is its shown name, with
/// its tab added when the name does not already carry it; without names
/// (herdr unreadable), the agent's own name. Never the pane id.
pub fn token_line(view: &ConfirmView, names: Option<&PaneNames>, token: &str) -> String {
    let handle = view
        .handle
        .as_deref()
        .map(|h| format!(" {h}"))
        .unwrap_or_default();
    let who = match names {
        Some(n) => match &n.tab {
            Some(tab) if !n.name.contains(", tab ") => format!("{}, tab {tab}", n.name),
            _ => n.name.clone(),
        },
        None => naming::capitalise(&view.agent),
    };
    format!("confirm {}{handle} ({who}): {token}", view.confirm_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(handle: Option<&str>) -> ConfirmView {
        ConfirmView {
            confirm_id: 3,
            pane_id: "w2:pZ".to_string(),
            agent: "claude".to_string(),
            command: "continue".to_string(),
            handle: handle.map(str::to_string),
            requested_by: "local:mcp".to_string(),
        }
    }

    fn names(name: &str, tab: Option<&str>) -> PaneNames {
        PaneNames {
            name: name.to_string(),
            spoken: name.to_string(),
            project: None,
            tab: tab.map(str::to_string),
        }
    }

    #[test]
    fn token_lines_name_the_agent_and_its_tab() {
        let hr = names("the handlr agent", Some("17-hr"));
        assert_eq!(
            token_line(&view(Some("keep-going")), Some(&hr), "a-b-c"),
            "confirm 3 keep-going (the handlr agent, tab 17-hr): a-b-c"
        );
        // A name that already carries its tab does not repeat it.
        let mb = names("Claude in libmorph, tab -mb", Some("-mb"));
        assert_eq!(
            token_line(&view(None), Some(&mb), "a-b-c"),
            "confirm 3 (Claude in libmorph, tab -mb): a-b-c"
        );
        // No tab: the name alone.
        let lone = names("Claude", None);
        assert_eq!(
            token_line(&view(None), Some(&lone), "a-b-c"),
            "confirm 3 (Claude): a-b-c"
        );
        // herdr unreadable: the agent, never the pane.
        let line = token_line(&view(Some("keep-going")), None, "a-b-c");
        assert_eq!(line, "confirm 3 keep-going (Claude): a-b-c");
        assert!(!line.contains("pZ"));
    }
}
