//! `flankr serve`: the daemon (plan 20261022_01, decisions 7–9 and 11).
//!
//! One Unix socket, mode 0600, one JSON-RPC object per line. Each
//! connection says `hello` first, then sends any of the common methods;
//! requests on a connection run concurrently and `cancel` aborts one.

pub mod companion;
pub mod conn;
pub mod herdr_attach;
pub mod link;
pub mod speech;
mod subscriptions;
pub mod terminal;
mod watcher;

use std::io::IsTerminal;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use libherdr::client::HerdrClient;
use libherdr::protocol::AgentStatus;
use libherdr::transport::HerdrTransport;
use tokio::net::UnixListener;
use tokio::sync::broadcast;

use crate::config::{AgentConfig, AppConfig, NamingConfig};
use crate::core::agents;
use crate::core::confirm::{Broker, ConfirmEvent, ConfirmView, DEFAULT_TTL, WordPriors};
use crate::core::naming::{self, Names};
use crate::sessions::Sessions;
use crate::sessions::hub::SessionHub;
use crate::watch::WatchRegistry;
use crate::watch_files::FileWatcher;

/// The socket `flankr serve` listens on unless `[server] socket` says otherwise.
pub fn default_socket_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("flankr")
        .join("flankr.sock")
}

/// Events from herdr that connections may subscribe to.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    AgentStatus {
        pane_id: String,
        agent: String,
        status: AgentStatus,
    },
    /// herdr's focus moved: to `pane_id`, or to a tab whose focused pane
    /// a snapshot gives (plan 20261023_03, decision 12).
    Focus { pane_id: Option<String> },
    /// A pane changed, or closed: its session subscriptions read it again.
    Pane { pane_id: String, closed: bool },
    /// herdr says a pane with an open terminal scrolled
    /// (`pane.scroll_changed`): its terminals read it again (plan
    /// 20261027_07, decision 5).
    Output { pane_id: String },
}

/// Everything a connection needs, shared by all of them.
pub struct Daemon {
    pub client: HerdrClient,
    pub agents: Vec<AgentConfig>,
    /// How agents are named and addressed (`[naming]`).
    pub naming: NamingConfig,
    /// `[urls] repowiki_url`: where `docs/…` paths link to, if anywhere.
    pub repowiki: Option<String>,
    pub broker: Arc<Broker>,
    pub watches: Arc<WatchRegistry>,
    pub events: broadcast::Sender<CoreEvent>,
    /// The daemon's own uid. A connecting peer must have the same one.
    pub uid: u32,
    /// The link for paired devices, when it is on.
    pub link: Option<Arc<link::Link>>,
    /// Companion sessions and their turns (plan 20261022_02, decisions 9–13).
    pub turns: companion::TurnService,
    /// The agents' own session transcripts (plan 20261022_03).
    pub sessions: Arc<Sessions>,
    /// Their tailers, for session subscriptions (plan 20261023_03).
    pub hub: Arc<SessionHub>,
    /// `speech.summarize`: the summary prompt and its kept answers (plan
    /// 20261023_05).
    pub speech: speech::SpeechService,
    /// `[llm]`: which of the daemon's own LLM uses are available, and
    /// their clients (plan 20261027_02).
    pub llm: Arc<crate::llm::LlmService>,
    /// The file watcher as given, if it was (see [`Daemon::file_watcher`]).
    file_watcher_option: Option<Result<FileWatcher, String>>,
    file_watcher: OnceLock<Option<FileWatcher>>,
    /// `[terminal] input`: typing into terminals may be allowed at all
    /// (plan 20261027_07, decision 6).
    pub terminal_input: bool,
    /// The panes of open terminals, for `pane.scroll_changed`.
    pub terminal_panes: Arc<terminal::TerminalPanes>,
    /// The terminal's attachment to herdr as one of its clients (plan
    /// 20261027_08): with `[terminal] attach` on and herdr local (or the
    /// test hook `TurnOptions::herdr_attach`), else `None` and terminals
    /// use the poll only. Started by [`run`].
    pub herdr_attach: Option<Arc<herdr_attach::HerdrAttach>>,
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
        Self::with_turns(client, config, link, companion::TurnOptions::default())
    }

    /// A daemon whose companion turns use `turns` (an engine other than
    /// Gemini, or another history directory).
    pub fn with_turns(
        client: HerdrClient,
        config: &AppConfig,
        link: Option<Arc<link::Link>>,
        turns: companion::TurnOptions,
    ) -> Arc<Self> {
        let ttl = config
            .server
            .confirm_ttl
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_TTL);
        let (events, _) = broadcast::channel(256);
        let broker = Broker::new(Arc::new(client.clone()), ttl);
        broker.set_word_priors(word_priors(&config.agents, &config.naming, None));
        let sessions = turns
            .agent_sessions
            .clone()
            .unwrap_or_else(|| Arc::new(Sessions::new()));
        // The pending questions' answers (plan 20261027_06, step 3).
        if turns.suggestions_file.is_some() || turns.suggestions_user_file.is_some() {
            sessions.suggestions.set_files(
                turns.suggestions_file.clone(),
                turns.suggestions_user_file.clone(),
            );
        }
        let llm = crate::llm::LlmService::new(
            config,
            turns.llm.clone(),
            crate::llm::Overrides {
                agent: turns.engine.clone(),
                summary: turns.speech.engine.clone(),
            },
        );
        let speech = speech::SpeechService::new(Arc::clone(&llm), turns.speech.clone());
        let hub = SessionHub::new(Arc::clone(&sessions), config.urls.repowiki_url.clone());
        let herdr_attach =
            attach_settings(&client, config, &turns).map(herdr_attach::HerdrAttach::new);
        let daemon = Arc::new(Daemon {
            sessions,
            hub,
            speech,
            broker,
            client,
            agents: config.agents.clone(),
            naming: config.naming.clone(),
            repowiki: config.urls.repowiki_url.clone(),
            watches: Arc::new(WatchRegistry::new()),
            events,
            // SAFETY: getuid has no preconditions and cannot fail.
            uid: unsafe { libc::getuid() },
            link,
            turns: companion::TurnService::new(Arc::clone(&llm), &turns),
            llm,
            file_watcher_option: turns.file_watcher.clone(),
            file_watcher: OnceLock::new(),
            terminal_input: config.terminal.input,
            terminal_panes: Arc::new(terminal::TerminalPanes::new()),
            herdr_attach,
        });
        // `[llm.uses.card_summaries]`: each new Finished card summarised
        // once, while card summaries are available.
        daemon
            .hub
            .summarize_cards_with(Arc::new(speech::DaemonCards(Arc::downgrade(&daemon))));
        daemon
    }

    /// The daemon's one file watcher (plan 20261027_05, decision 2),
    /// started on first use; `None` when it can't start, which is logged
    /// once, and its users fall back to their intervals.
    pub fn file_watcher(&self) -> Option<FileWatcher> {
        self.file_watcher
            .get_or_init(|| {
                let started = match &self.file_watcher_option {
                    Some(given) => given.clone(),
                    None => FileWatcher::start(),
                };
                started
                    .inspect_err(|e| {
                        log::warn!(
                            "the file watcher can't start ({e}): [llm] is checked every 5 s \
                             and auth.toml every [link] revoke_check_secs instead"
                        )
                    })
                    .ok()
            })
            .clone()
    }

    /// Every pane in `snapshot`, named by `[naming]`. The broker's word
    /// priors are refreshed from it on the way.
    pub fn names(&self, snapshot: &serde_json::Value) -> Names {
        let names = Names::from_snapshot(snapshot, &self.naming);
        self.broker.set_word_priors(word_priors(
            &self.agents,
            &self.naming,
            Some((snapshot, &names)),
        ));
        names
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

/// Where the terminal's herdr attachment connects: the test hook, else the
/// client socket beside a local herdr's API socket; `None` with
/// `[terminal] attach = false`, a remote herdr or the mock.
fn attach_settings(
    client: &HerdrClient,
    config: &AppConfig,
    turns: &companion::TurnOptions,
) -> Option<herdr_attach::AttachSettings> {
    if !config.terminal.attach {
        return None;
    }
    if let Some(settings) = &turns.herdr_attach {
        return Some(settings.clone());
    }
    match client.transport() {
        HerdrTransport::LocalSocket { socket_path } => Some(herdr_attach::AttachSettings::new(
            libherdr_term::client_socket_for(socket_path),
        )),
        _ => None,
    }
}

/// What confirmation handle words are worth (plan 20261023_01, section
/// 3.2): the words of every allowed command, every agent's `[[agents]]`
/// name, `[naming]` alias and (from `snapshot`) shown and spoken name, the
/// snapshot's project and workspace names, and the names letters are
/// spelled with.
pub fn word_priors(
    agents: &[AgentConfig],
    naming: &NamingConfig,
    snapshot: Option<(&serde_json::Value, &Names)>,
) -> WordPriors {
    let mut agent_words: Vec<String> = agents.iter().map(|a| a.name.clone()).collect();
    agent_words.extend(naming.aliases.values().cloned());
    let mut projects: Vec<String> = Vec::new();
    if let Some((snap, names)) = snapshot {
        for named in &names.agents {
            agent_words.push(named.agent_part.shown.clone());
            agent_words.push(named.agent_part.spoken.clone());
            if let Some(project) = &named.project {
                projects.push(project.shown.clone());
                projects.push(project.spoken.clone());
            }
        }
        projects.extend(agents::workspace_labels(snap).into_values());
    }
    WordPriors::new()
        .with_commands(agents.iter().flat_map(|a| a.allowed_commands.iter()))
        .with_agents(agent_words)
        .with_projects(projects)
        .with_letter_names(('a'..='z').filter_map(|c| naming::letter_name(c, naming)))
}

/// herdr's focused pane in `snapshot`, named by `names` as `agents.list`
/// names it (plan 20261023_02, decision 4). `None` when nothing is focused
/// or the focused pane is not in the snapshot.
pub fn focus_info(
    snapshot: &serde_json::Value,
    names: &Names,
) -> Option<libflankr_proto::common::FocusInfo> {
    let pane_id = agents::focused_pane_id(snapshot)?;
    let named = names.get(&pane_id)?;
    let a = &named.agent;
    Some(libflankr_proto::common::FocusInfo {
        pane_id: a.pane_id.clone(),
        project: named
            .project
            .as_ref()
            .map(|p| p.shown.clone())
            .or_else(|| a.project.clone()),
        agent: (a.agent != "shell").then(|| named.agent_part.shown.clone()),
        tab_label: a.tab_label.clone(),
        tab_number: a.tab_number,
        name: named.shown.clone(),
        spoken: named.spoken.clone(),
    })
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
                "another flankr daemon is already listening on {}",
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
    // The word priors from the first snapshot; later ones refresh them.
    {
        let daemon = Arc::clone(&daemon);
        tokio::spawn(async move {
            if let Ok(Ok(snap)) =
                tokio::time::timeout(NAME_LOOKUP_TIMEOUT, daemon.client.list_panes()).await
            {
                daemon.names(&snap);
            }
        });
    }
    tokio::spawn(subscriptions::journal_confirmations(Arc::clone(&daemon)));
    // `[llm]` read again on a change to its file or its key files (plan
    // 20261027_05, decision 2), or within 5 s without the file watcher.
    daemon.llm.spawn_watch(daemon.file_watcher().as_ref());
    // The suggestion rules read again on a change to their files (plan
    // 20261027_06, decision 6 as the 14:13 EDT amendment has it).
    daemon
        .sessions
        .suggestions
        .spawn_watch(daemon.file_watcher().as_ref());
    // The terminal's herdr attachment, held for the daemon's run (plan
    // 20261027_08, the 23:32 EDT amendment).
    if let Some(attach) = &daemon.herdr_attach {
        attach.spawn();
    }
    if let Some(transport) = options.event_transport {
        tokio::spawn(watcher::run(
            transport,
            daemon.client.clone(),
            Arc::clone(&daemon.watches),
            Arc::clone(&daemon.terminal_panes),
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
            lifetime: crate::core::confirm::Lifetime::Standing,
            message: false,
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
    fn word_priors_come_from_the_config_and_the_snapshot() {
        let agents = vec![AgentConfig::new(
            "claude",
            vec!["continue".to_string(), "/usage".to_string()],
            true,
        )];
        let naming = NamingConfig::default();
        let p = word_priors(&agents, &naming, None);
        assert_eq!(p.prior("usage", None), 0.25);
        assert_eq!(p.prior("claude", None), 0.25);
        assert_eq!(p.prior("aitch", None), 1.0 / 26.0);
        assert_eq!(p.prior("highbay", None), 1.0 / 64.0);
        let snap = serde_json::json!({
            "workspaces": [{"workspace_id": "w1", "label": "highbay"}],
            "panes": [{"pane_id": "p1", "workspace_id": "w1", "agent": "claude"}]
        });
        let names = Names::from_snapshot(&snap, &naming);
        let p = word_priors(&agents, &naming, Some((&snap, &names)));
        assert_eq!(p.prior("highbay", None), 0.25);
    }

    #[test]
    fn focus_is_named_like_the_agents() {
        let naming = NamingConfig::default();
        let snap = serde_json::json!({
            "focused_pane_id": "p2",
            "workspaces": [{"workspace_id": "w1", "label": "libmorph"}],
            "tabs": [{"tab_id": "w1:t1", "workspace_id": "w1", "number": 4, "label": "-ad"}],
            "panes": [
                {"pane_id": "w1:p1", "workspace_id": "w1", "cwd": "/src/libmorph"},
                {"pane_id": "w1:p2", "workspace_id": "w1", "tab_id": "w1:t1",
                 "agent": "claude", "cwd": "/src/libmorph"}
            ]
        });
        let names = Names::from_snapshot(&snap, &naming);
        let focus = focus_info(&snap, &names).unwrap();
        assert_eq!(focus.pane_id, "w1:p2");
        assert_eq!(focus.agent.as_deref(), Some("Claude"));
        assert_eq!(focus.project.as_deref(), Some("libmorph"));
        assert_eq!(focus.tab_label.as_deref(), Some("-ad"));
        assert_eq!(focus.tab_number, Some(4));
        assert_eq!(focus.name, names.get("w1:p2").unwrap().shown);
        // A shell has no agent; nothing focused is none.
        let shell =
            serde_json::json!({"focused_pane_id": "w1:p1", "panes": [{"pane_id": "w1:p1"}]});
        let names = Names::from_snapshot(&shell, &naming);
        assert_eq!(focus_info(&shell, &names).unwrap().agent, None);
        let none = serde_json::json!({"panes": [{"pane_id": "w1:p1"}]});
        let names = Names::from_snapshot(&none, &naming);
        assert_eq!(focus_info(&none, &names), None);
    }

    #[test]
    fn token_lines_name_the_agent_and_its_tab() {
        let fl = names("the flankr agent", Some("17-fl"));
        assert_eq!(
            token_line(&view(Some("keep-going")), Some(&fl), "a-b-c"),
            "confirm 3 keep-going (the flankr agent, tab 17-fl): a-b-c"
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
