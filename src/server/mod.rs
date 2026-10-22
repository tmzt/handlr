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

use crate::config::{AgentConfig, AppConfig};
use crate::core::confirm::{Broker, ConfirmEvent, DEFAULT_TTL};
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
            watches: Arc::new(WatchRegistry::new()),
            events,
            // SAFETY: getuid has no preconditions and cannot fail.
            uid: unsafe { libc::getuid() },
            link,
        })
    }
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
        tokio::spawn(print_tokens(daemon.broker.subscribe()));
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

/// `confirm <id> [<handle>]: <token>` on standard output, so a local client
/// can approve with the token read off the daemon's terminal.
async fn print_tokens(mut events: broadcast::Receiver<ConfirmEvent>) {
    loop {
        match events.recv().await {
            Ok(ConfirmEvent::Requested { view, token, .. }) => {
                let handle = view.handle.map(|h| format!(" {h}")).unwrap_or_default();
                println!(
                    "confirm {}{}: {}",
                    view.confirm_id,
                    handle,
                    token.expose_for_terminal()
                );
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}
