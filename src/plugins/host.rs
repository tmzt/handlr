//! The plugin host: the registry with its config, `plugin.call` and
//! `plugin.set_enabled` (docs/design/20261028_04_PLUGINS_PLAN.md,
//! decisions 3.2, 3.3 and 3.5).
//!
//! `plugin.call` is answered at once with `PluginAccepted` or an error;
//! the plugin runs in a task of its own, at most [`MAX_CALLS`] per
//! connection and [`CALL_TIMEOUT`] each, and its answer goes back as
//! `plugin.result` on the calling connection only, tagged with the
//! plugin's own namespace. `[plugins]` is read again when the config file
//! changes, and `plugin.set_enabled` writes `enabled` into it.
//!
//! The general agent's plugin tools (the plan's 02:10 EDT amendment of
//! 2026-10-29) are offered from the same state: an enabled plugin's
//! read-only tools unless its `agent_tools = false`, and the others too
//! with `agent_tools = "all"`. A tool call runs through the plugin's
//! [`PluginCtx`] with the same limits as `plugin.call`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime};

use libflankr_proto::companion::{
    self, PLUGIN_MAX_BYTES, PluginAccepted, PluginCall, PluginInfo, PluginMenuItem, PluginNotify,
    PluginResult, PluginSetEnabledParams, PluginSetEnabledResult,
};
use libflankr_proto::envelope::codes;
use libflankr_proto::{ErrorObject, Notification};
use serde_json::Value;
use tokio::sync::{oneshot, watch};
use tokio::task::AbortHandle;

use super::command::{self, CommandEnv};
use super::ctx::{self, Commands, CtxParts, PluginCtx, ProjectLookup};
use super::{Plugin, PluginError, ToolSpec, agent_tool_name};
use crate::config::AppConfig;
use crate::core::principal::Principal;
use crate::llm::{AgentToolSource, ReadPattern};
use crate::reasoning::AgentTool;
use crate::watch_files::FileWatcher;

/// At most this many plugin calls run at once on one connection.
pub const MAX_CALLS: usize = 4;
/// How long one plugin call may run.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the config is checked without the file watcher.
pub const RELOAD_INTERVAL: Duration = Duration::from_secs(5);
/// What `plugin.set_enabled` says of a config file that does not parse.
pub const BROKEN_CONFIG: &str = "The config file has an error; fix it first.";

/// How the host is set up.
#[derive(Clone, Default)]
pub struct HostOptions {
    /// The config file, read again on change and written by
    /// `plugin.set_enabled`; without it, `[plugins]` is as at start and
    /// cannot be written.
    pub config_path: Option<PathBuf>,
    /// The plugins, instead of [`super::registry::builtin`] (tests). They
    /// are checked as the built-in ones are.
    pub plugins: Option<Vec<Arc<dyn Plugin>>>,
    /// The commands' environment and `PATH`, instead of the process's.
    pub env: Option<CommandEnv>,
    /// Instead of [`RELOAD_INTERVAL`].
    pub interval: Option<Duration>,
}

/// Sends one notification on the calling connection.
pub type Sink = Arc<dyn Fn(Notification) + Send + Sync>;

/// Who calls, and where the answer goes.
pub struct Caller {
    pub principal: Principal,
    /// The call's `session`, else the conversation's current agent.
    pub pane: Option<String>,
    pub lookup: Arc<dyn ProjectLookup>,
    pub sink: Sink,
    /// Fires once the `plugin.call` answer is written; the plugin starts
    /// after it, so its notifications and result follow the answer.
    pub answered: Option<oneshot::Receiver<()>>,
}

/// Who calls a plugin's tool for the general agent: the turn's
/// principal and current agent, and the lookup of its folder.
#[derive(Clone)]
pub struct AgentToolCaller {
    pub principal: Principal,
    /// The conversation's current agent.
    pub pane: Option<String>,
    pub lookup: Arc<dyn ProjectLookup>,
}

/// `[plugins."<ns>"] agent_tools` (the 02:10 EDT amendment, item 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentToolsSetting {
    /// `false`: none of its tools.
    Off,
    /// `true`, or left out: its read-only tools.
    ReadOnly,
    /// `"all"`: every tool, those that change something confirmed.
    All,
}

impl AgentToolsSetting {
    /// The setting in a plugin's table; anything else is logged and read
    /// as the default.
    pub fn of(table: Option<&toml::Value>, ns: &str) -> AgentToolsSetting {
        match table.and_then(|t| t.get("agent_tools")) {
            None | Some(toml::Value::Boolean(true)) => AgentToolsSetting::ReadOnly,
            Some(toml::Value::Boolean(false)) => AgentToolsSetting::Off,
            Some(toml::Value::String(s)) if s == "all" => AgentToolsSetting::All,
            Some(other) => {
                log::warn!(
                    "[plugins.\"{ns}\"] agent_tools = {other} is not true, false or \"all\"; \
                     read-only tools only"
                );
                AgentToolsSetting::ReadOnly
            }
        }
    }

    fn offers(self, tool: &ToolSpec) -> bool {
        match self {
            AgentToolsSetting::Off => false,
            AgentToolsSetting::ReadOnly => tool.read_only,
            AgentToolsSetting::All => true,
        }
    }
}

/// A plugin tool the general agent is offered now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferedTool {
    pub ns: &'static str,
    /// The plugin's name, e.g. "Planning".
    pub plugin: &'static str,
    /// The tool's own name, without the prefix.
    pub tool: &'static str,
    pub read_only: bool,
}

/// One connection's running plugin calls.
#[derive(Default)]
pub struct CallSlots {
    running: Arc<AtomicUsize>,
    tasks: Mutex<Vec<AbortHandle>>,
}

/// Holds one of a connection's call slots; dropping it frees the slot.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl CallSlots {
    fn take(&self) -> Option<Slot> {
        self.running
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < MAX_CALLS).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(Arc::clone(&self.running)))
    }

    fn hold(&self, task: AbortHandle) {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|t| !t.is_finished());
        tasks.push(task);
    }

    /// How many calls are running.
    pub fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    /// The connection closed: its calls stop.
    pub fn close(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

/// One registered plugin, with what was settled at start.
struct Entry {
    plugin: Arc<dyn Plugin>,
    /// Its `read_patterns()`, parsed.
    read: Vec<ReadPattern>,
    /// Each of its `commands()`' binary, as found on `PATH` at start.
    binaries: Vec<Option<PathBuf>>,
}

/// `[plugins]` and the configured `read`, as last read.
struct State {
    plugins: toml::Table,
    read: Vec<ReadPattern>,
    stamp: Option<(SystemTime, u64)>,
}

/// The daemon's plugins while it runs.
pub struct PluginHost {
    entries: Vec<Entry>,
    env: CommandEnv,
    config_path: Option<PathBuf>,
    interval: Duration,
    state: Mutex<State>,
    infos: watch::Sender<Vec<PluginInfo>>,
    /// Held while `plugin.set_enabled` writes the file.
    writing: Mutex<()>,
}

impl PluginHost {
    /// The host for `config`, as loaded at start.
    pub fn new(config: &AppConfig, options: HostOptions) -> Arc<Self> {
        let plugins = match options.plugins {
            Some(plugins) => {
                let (kept, problems) = super::registry::check(plugins);
                for problem in problems {
                    log::warn!("plugins: {problem}; left out");
                }
                kept
            }
            None => super::registry::builtin(),
        };
        let env = options.env.unwrap_or_else(CommandEnv::from_process);
        let entries = plugins
            .into_iter()
            .map(|plugin| {
                let binaries = plugin
                    .commands()
                    .iter()
                    .map(|spec| {
                        let found = env.find(spec.binary);
                        if found.is_none() {
                            log::info!(
                                "plugin {}: {} is not on PATH; its command {} is unavailable",
                                plugin.ns(),
                                spec.binary,
                                spec.name
                            );
                        }
                        found
                    })
                    .collect();
                let read = plugin
                    .read_patterns()
                    .iter()
                    .filter_map(|p| ReadPattern::parse(p).ok())
                    .collect();
                Entry {
                    plugin,
                    read,
                    binaries,
                }
            })
            .collect();
        let read = config
            .llm
            .as_ref()
            .and_then(|l| l.uses.get(crate::llm::Use::Planning.name()))
            .and_then(|u| u.read.clone())
            .unwrap_or_default();
        let state = State {
            plugins: config.plugins.clone(),
            read: parse_read(read.iter().map(String::as_str)),
            stamp: options.config_path.as_deref().and_then(stamp),
        };
        let host = PluginHost {
            entries,
            env,
            config_path: options.config_path,
            interval: options.interval.unwrap_or(RELOAD_INTERVAL),
            state: Mutex::new(state),
            infos: watch::Sender::new(Vec::new()),
            writing: Mutex::new(()),
        };
        let infos = host.infos_of(&host.state.lock().unwrap());
        host.infos.send_replace(infos);
        for (ns, _) in &host.state.lock().unwrap().plugins {
            if host.entry(ns).is_none() {
                log::warn!("[plugins.\"{ns}\"]: no such built-in plugin; ignored");
            }
        }
        Arc::new(host)
    }

    /// Every built-in plugin, enabled or not, for `hello` and
    /// `server.capabilities`.
    pub fn infos(&self) -> Vec<PluginInfo> {
        self.infos.borrow().clone()
    }

    /// The list, as it changes.
    pub fn subscribe(&self) -> watch::Receiver<Vec<PluginInfo>> {
        self.infos.subscribe()
    }

    fn entry(&self, ns: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.plugin.ns() == ns)
    }

    fn infos_of(&self, state: &State) -> Vec<PluginInfo> {
        self.entries
            .iter()
            .map(|e| {
                let p = &e.plugin;
                let enabled = enabled(&state.plugins, p.ns());
                PluginInfo {
                    ns: p.ns().to_string(),
                    name: p.name().to_string(),
                    description: p.description().to_string(),
                    version: p.version().text(),
                    enabled,
                    menu: p
                        .menu()
                        .iter()
                        .map(|m| PluginMenuItem {
                            id: m.id.to_string(),
                            label: m.label.to_string(),
                            opens: m.opens.as_str().to_string(),
                            screens: m.screens.as_str().to_string(),
                            icon: m.icon.map(str::to_string),
                        })
                        .collect(),
                    reason: (enabled && !e.read.is_empty() && state.read.is_empty())
                        .then(|| ctx::READING_OFF.to_string()),
                }
            })
            .collect()
    }

    /// One check: the config file read again when it changed (or always,
    /// with `force`). Returns whether the plugin list changed; when it did,
    /// subscribers are told.
    pub fn check(&self, force: bool) -> bool {
        let Some(path) = &self.config_path else {
            return false;
        };
        let mut state = self.state.lock().unwrap();
        let now = stamp(path);
        if !force && now == state.stamp {
            return false;
        }
        state.stamp = now;
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                log::warn!(
                    "[plugins] {}: {e}; the last good settings stay",
                    path.display()
                );
                return false;
            }
        };
        let table = match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(e) => {
                log::warn!(
                    "[plugins] {} no longer parses ({e}); the last good settings stay",
                    path.display()
                );
                return false;
            }
        };
        let plugins = match table.get("plugins") {
            None => toml::Table::new(),
            Some(toml::Value::Table(t)) => t.clone(),
            Some(_) => {
                log::warn!(
                    "[plugins] in {} is not a table; every plugin is off",
                    path.display()
                );
                toml::Table::new()
            }
        };
        let read = table
            .get("llm")
            .and_then(|v| v.get("uses"))
            .and_then(|v| v.get("planning"))
            .and_then(|v| v.get("read"))
            .and_then(|v| v.as_array())
            .map(|list| parse_read(list.iter().filter_map(|v| v.as_str())))
            .unwrap_or_default();
        let table_changed = plugins != state.plugins;
        if table_changed {
            log::info!(
                "[plugins] {} changed; reading [plugins] again",
                path.display()
            );
            for (ns, _) in &plugins {
                if self.entry(ns).is_none() {
                    log::warn!("[plugins.\"{ns}\"]: no such built-in plugin; ignored");
                }
            }
        }
        state.plugins = plugins;
        state.read = read;
        let infos = self.infos_of(&state);
        drop(state);
        if table_changed {
            let tools: Vec<String> = self.agent_tools().into_iter().map(|t| t.name).collect();
            log::info!("[plugins] the general agent's plugin tools: {tools:?}");
        }
        self.infos.send_if_modified(|old| {
            let changed = *old != infos;
            if changed {
                for info in &infos {
                    log::info!(
                        "plugin {}@{}: {}",
                        info.ns,
                        info.version,
                        if info.enabled { "on" } else { "off" }
                    );
                }
            }
            *old = infos;
            changed
        })
    }

    /// Checks on each change `files` reports to the config file, else
    /// every interval while the host is in use.
    pub fn spawn_watch(self: &Arc<Self>, files: Option<&FileWatcher>) {
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let weak: Weak<Self> = Arc::downgrade(self);
        if let Some(files) = files {
            match files.subscribe(std::slice::from_ref(&path)) {
                Ok(mut sub) => {
                    tokio::spawn(async move {
                        while sub.changed().await {
                            let Some(host) = weak.upgrade() else {
                                return;
                            };
                            host.check(false);
                        }
                    });
                    return;
                }
                Err(e) => log::warn!(
                    "[plugins] {} can't be watched ({e}); checking every {:?} instead",
                    path.display(),
                    self.interval
                ),
            }
        }
        let interval = self.interval;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            loop {
                tick.tick().await;
                let Some(host) = weak.upgrade() else {
                    return;
                };
                host.check(false);
            }
        });
    }

    /// `plugin.call`: checked and handed to the plugin, or refused.
    pub fn call(
        &self,
        call: PluginCall,
        caller: Caller,
        slots: &CallSlots,
    ) -> Result<PluginAccepted, ErrorObject> {
        if !companion::is_plugin_ns(&call.ns) {
            return Err(ErrorObject::new(
                codes::INVALID_PARAMS,
                format!("{:?} is not a plugin's package name", call.ns),
            ));
        }
        if !companion::is_plugin_method(&call.method) {
            return Err(ErrorObject::new(
                codes::INVALID_PARAMS,
                format!("{:?} is not a plugin method's name", call.method),
            ));
        }
        let Some(entry) = self.entry(&call.ns) else {
            return Err(ErrorObject::new(
                codes::PLUGIN_UNKNOWN,
                format!("This flankr has no plugin {}.", call.ns),
            ));
        };
        let plugin = Arc::clone(&entry.plugin);
        let Some((table, read, on)) = self.settings_of(entry) else {
            return Err(ErrorObject::new(
                codes::PLUGIN_DISABLED,
                format!(
                    "{} is off; turn it on in Settings → Plugins.",
                    plugin.name()
                ),
            ));
        };
        let size = serde_json::to_vec(&call.params)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if size > PLUGIN_MAX_BYTES {
            return Err(ErrorObject::new(
                codes::PLUGIN_TOO_LARGE,
                "The call's params are over 256 KiB.",
            ));
        }
        let Some(slot) = slots.take() else {
            return Err(ErrorObject::new(
                codes::BUSY,
                format!("{MAX_CALLS} plugin calls are already running on this connection."),
            ));
        };
        let ns = plugin.ns();
        let sink = Arc::clone(&caller.sink);
        let notifier: ctx::Notifier = Arc::new(move |method: &str, params: Value| {
            sink(Notification::new(
                companion::methods::PLUGIN_NOTIFY,
                PluginNotify {
                    ns: ns.to_string(),
                    method: method.to_string(),
                    params,
                },
            ));
        });
        let parts = CtxParts {
            ns,
            principal: caller.principal,
            pane: caller.pane,
            lookup: caller.lookup,
            read,
            plugin_read: entry.read.clone(),
            commands: Commands {
                on,
                env: self.env.clone(),
            },
            config: table,
            notifier: Some(notifier),
        };
        let PluginCall {
            id, method, params, ..
        } = call;
        let sink = caller.sink;
        let answered = caller.answered;
        log::info!(
            "plugin {ns}: {method}{}",
            id.map(|id| format!(" (call {id})")).unwrap_or_default()
        );
        let task = tokio::spawn(async move {
            let _slot = slot;
            if let Some(answered) = answered {
                let _ = answered.await;
            }
            let outcome = tokio::time::timeout(CALL_TIMEOUT, async {
                let ctx = PluginCtx::build(parts).await;
                plugin.call(&method, params, &ctx).await
            })
            .await;
            let Some(id) = id else {
                if let Ok(Err(e)) | Err(e) = outcome.map_err(|_| timed_out()) {
                    log::info!("plugin {ns}: {method} (no id): {e}");
                }
                return;
            };
            let (result, error) = match outcome {
                Err(_) => (None, Some(to_error(timed_out()))),
                Ok(Err(e)) => (None, Some(to_error(e))),
                Ok(Ok(value)) => {
                    let size = serde_json::to_vec(&value)
                        .map(|v| v.len())
                        .unwrap_or(usize::MAX);
                    if size > PLUGIN_MAX_BYTES {
                        log::warn!("plugin {ns}: {method}'s answer is {size} bytes; refused");
                        (
                            None,
                            Some(ErrorObject::new(
                                codes::PLUGIN_TOO_LARGE,
                                "The plugin's answer is over 256 KiB.",
                            )),
                        )
                    } else {
                        (Some(value), None)
                    }
                }
            };
            sink(Notification::new(
                companion::methods::PLUGIN_RESULT,
                PluginResult {
                    ns: ns.to_string(),
                    id,
                    result,
                    error,
                },
            ));
        });
        slots.hold(task.abort_handle());
        Ok(PluginAccepted {})
    }

    /// An enabled plugin's table, the configured `read`, and the commands
    /// left on; `None` while it is off.
    #[allow(clippy::type_complexity)]
    fn settings_of(
        &self,
        entry: &Entry,
    ) -> Option<(
        toml::Table,
        Vec<ReadPattern>,
        Vec<(&'static super::CommandSpec, Option<PathBuf>)>,
    )> {
        let plugin = &entry.plugin;
        let state = self.state.lock().unwrap();
        if !enabled(&state.plugins, plugin.ns()) {
            return None;
        }
        let table = match state.plugins.get(plugin.ns()) {
            Some(toml::Value::Table(t)) => t.clone(),
            _ => toml::Table::new(),
        };
        let names = command::narrow(plugin.ns(), plugin.commands(), table.get("commands"));
        let on: Vec<_> = plugin
            .commands()
            .iter()
            .zip(&entry.binaries)
            .filter(|(spec, _)| names.contains(&spec.name))
            .map(|(spec, binary)| (spec, binary.clone()))
            .collect();
        Some((table, state.read.clone(), on))
    }

    /// The tools offered now, each with its entry: enabled plugins only,
    /// as their `agent_tools` allows.
    fn offered(&self) -> Vec<(&Entry, &ToolSpec)> {
        let state = self.state.lock().unwrap();
        let mut out = Vec::new();
        for entry in &self.entries {
            let ns = entry.plugin.ns();
            if !enabled(&state.plugins, ns) {
                continue;
            }
            let setting = AgentToolsSetting::of(state.plugins.get(ns), ns);
            for tool in entry.plugin.tools() {
                if setting.offers(tool) {
                    out.push((entry, tool));
                }
            }
        }
        out
    }

    /// The offered tool the general agent calls `name`, if there is one.
    pub fn agent_tool(&self, name: &str) -> Option<OfferedTool> {
        self.offered()
            .into_iter()
            .find(|(e, t)| agent_tool_name(e.plugin.ns(), t.name) == name)
            .map(|(e, t)| OfferedTool {
                ns: e.plugin.ns(),
                plugin: e.plugin.name(),
                tool: t.name,
                read_only: t.read_only,
            })
    }

    /// Runs the general agent's tool `name` (the 02:10 EDT amendment,
    /// item 4): offered now, through the plugin's [`PluginCtx`] for the
    /// caller's current agent, its args and answer at most 256 KiB, in at
    /// most [`CALL_TIMEOUT`]. A tool that changes something must have been
    /// confirmed by the caller first.
    pub async fn run_agent_tool(
        &self,
        name: &str,
        args: Value,
        caller: AgentToolCaller,
    ) -> Result<Value, PluginError> {
        let found = self
            .offered()
            .into_iter()
            .find(|(e, t)| agent_tool_name(e.plugin.ns(), t.name) == name)
            .map(|(e, t)| (Arc::clone(&e.plugin), e.read.clone(), t.name));
        let Some((plugin, plugin_read, tool)) = found else {
            return Err(PluginError::new(
                codes::METHOD_NOT_FOUND,
                format!("No plugin offers the tool {name} now."),
            ));
        };
        let entry = self.entry(plugin.ns()).expect("an offered tool's plugin");
        let Some((table, read, on)) = self.settings_of(entry) else {
            return Err(PluginError::new(
                codes::PLUGIN_DISABLED,
                format!("{} is off.", plugin.name()),
            ));
        };
        let size = serde_json::to_vec(&args)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if size > PLUGIN_MAX_BYTES {
            return Err(PluginError::new(
                codes::PLUGIN_TOO_LARGE,
                "The tool's arguments are over 256 KiB.",
            ));
        }
        let ns = plugin.ns();
        let parts = CtxParts {
            ns,
            principal: caller.principal,
            pane: caller.pane,
            lookup: caller.lookup,
            read,
            plugin_read,
            commands: Commands {
                on,
                env: self.env.clone(),
            },
            config: table,
            notifier: None,
        };
        log::info!("plugin {ns}: the general agent's tool {name}");
        let outcome = tokio::time::timeout(CALL_TIMEOUT, async {
            let ctx = PluginCtx::build(parts).await;
            plugin.run_tool(tool, args, &ctx).await
        })
        .await
        .unwrap_or_else(|_| Err(timed_out()));
        let value = outcome.inspect_err(|e| log::info!("plugin {ns}: tool {name}: {e}"))?;
        let size = serde_json::to_vec(&value)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if size > PLUGIN_MAX_BYTES {
            log::warn!("plugin {ns}: tool {name}'s answer is {size} bytes; refused");
            return Err(PluginError::new(
                codes::PLUGIN_TOO_LARGE,
                "The plugin's answer is over 256 KiB.",
            ));
        }
        Ok(value)
    }

    /// `plugin.set_enabled`: `[plugins."<ns>"] enabled` written, then read
    /// again, so subscribers hear the new list.
    pub fn set_enabled(
        &self,
        principal: &Principal,
        params: PluginSetEnabledParams,
    ) -> Result<PluginSetEnabledResult, ErrorObject> {
        if !principal.can_approve {
            return Err(ErrorObject::new(
                codes::MAY_NOT_APPROVE,
                "Only a device that can approve may change this.",
            ));
        }
        if self.entry(&params.ns).is_none() {
            return Err(ErrorObject::new(
                codes::PLUGIN_UNKNOWN,
                format!("This flankr has no plugin {}.", params.ns),
            ));
        }
        let Some(path) = &self.config_path else {
            return Err(ErrorObject::new(
                codes::INTERNAL_ERROR,
                "This flankr has no config file to write.",
            ));
        };
        {
            let _writing = self.writing.lock().unwrap();
            write_enabled(path, &params.ns, params.enabled)
                .map_err(|e| ErrorObject::new(codes::INTERNAL_ERROR, e))?;
        }
        log::info!(
            "{} turned plugin {} {} in {}",
            principal.id,
            params.ns,
            if params.enabled { "on" } else { "off" },
            path.display()
        );
        self.check(true);
        Ok(PluginSetEnabledResult {
            enabled: params.enabled,
        })
    }
}

impl AgentToolSource for PluginHost {
    /// The offered tools, in the registry's order and each plugin's own.
    fn agent_tools(&self) -> Vec<AgentTool> {
        self.offered()
            .into_iter()
            .map(|(e, t)| AgentTool {
                name: agent_tool_name(e.plugin.ns(), t.name),
                description: t.description.to_string(),
                params: t.params.to_string(),
            })
            .collect()
    }
}

fn timed_out() -> PluginError {
    PluginError::new(
        codes::CANCELLED,
        format!("The plugin took longer than {} s.", CALL_TIMEOUT.as_secs()),
    )
}

/// A plugin's error on the wire: its own codes, `INVALID_PARAMS`,
/// `METHOD_NOT_FOUND` and `CANCELLED` as they are; any other code is
/// `INTERNAL_ERROR`.
fn to_error(e: PluginError) -> ErrorObject {
    let own = (super::codes::FIRST..=super::codes::LAST).contains(&e.code)
        || matches!(
            e.code,
            codes::INVALID_PARAMS | codes::METHOD_NOT_FOUND | codes::CANCELLED
        );
    ErrorObject::new(if own { e.code } else { codes::INTERNAL_ERROR }, e.message)
}

/// Whether `[plugins."<ns>"] enabled = true`.
fn enabled(plugins: &toml::Table, ns: &str) -> bool {
    plugins
        .get(ns)
        .and_then(|t| t.get("enabled"))
        .and_then(toml::Value::as_bool)
        == Some(true)
}

fn parse_read<'a>(patterns: impl Iterator<Item = &'a str>) -> Vec<ReadPattern> {
    patterns
        .filter_map(|p| ReadPattern::parse(p).ok())
        .collect()
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Sets `plugins."<ns>".enabled` in the config at `path` with `toml_edit`,
/// keeping the rest as written: to a temporary file in the same folder,
/// then renamed over it (its mode kept). A missing file is created with
/// only that table; a file that does not parse is not written.
pub fn write_enabled(path: &Path, ns: &str, enabled: bool) -> Result<(), String> {
    // A link (a dotfiles checkout) is followed, so the file it names is
    // replaced and the link kept.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("The config file can't be read: {e}")),
    };
    let mut doc = match &text {
        Some(text) => text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| BROKEN_CONFIG.to_string())?,
        None => toml_edit::DocumentMut::new(),
    };
    let plugins = doc.entry("plugins").or_insert_with(|| {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        toml_edit::Item::Table(table)
    });
    let Some(plugins) = plugins.as_table_like_mut() else {
        return Err(BROKEN_CONFIG.to_string());
    };
    let table = plugins
        .entry(ns)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let Some(table) = table.as_table_like_mut() else {
        return Err(BROKEN_CONFIG.to_string());
    };
    table.insert("enabled", toml_edit::value(enabled));
    let out = doc.to_string();
    // What is written must still read as flankr's config.
    if out.parse::<toml::Table>().is_err() {
        return Err(BROKEN_CONFIG.to_string());
    }
    replace(&path, out.as_bytes())
}

/// `bytes` to a temporary file beside `path`, then renamed over it.
fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.toml".to_string());
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mode = std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o7777)
        .unwrap_or(0o600);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("The config file can't be written: {e}")
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::plugins::ctx::tests::FakeLookup;
    use crate::plugins::{CommandSpec, MenuItem, Opens, PluginVersion, Screens};
    use serde_json::json;
    use tokio::sync::mpsc;

    /// The test plugin, `dev.flankr.plugins.test` unless named otherwise.
    /// Its general agent tools: `echo`, `big` and `project` (read-only)
    /// and `write`, which changes something and counts its runs.
    pub(crate) struct TestPlugin {
        ns: &'static str,
        tools: Vec<ToolSpec>,
        pub(crate) writes: Arc<AtomicUsize>,
    }

    impl TestPlugin {
        pub(crate) fn named(ns: &str) -> Self {
            Self::with_tools(ns, test_tools())
        }

        pub(crate) fn with_tools(ns: &str, tools: Vec<ToolSpec>) -> Self {
            TestPlugin {
                ns: Box::leak(ns.to_string().into_boxed_str()),
                tools,
                writes: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    pub(crate) fn tool(name: &'static str, read_only: bool) -> ToolSpec {
        ToolSpec {
            name,
            description: "A test tool.",
            params: json!({"type": "object", "properties": {}}),
            read_only,
        }
    }

    fn test_tools() -> Vec<ToolSpec> {
        vec![
            tool("echo", true),
            tool("big", true),
            tool("project", true),
            tool("write", false),
        ]
    }

    const MENU: [MenuItem; 1] = [MenuItem {
        id: "test",
        label: "Test",
        opens: Opens::Card,
        screens: Screens::Overview,
        icon: None,
    }];

    #[async_trait::async_trait]
    impl Plugin for TestPlugin {
        fn ns(&self) -> &'static str {
            self.ns
        }
        fn name(&self) -> &'static str {
            "Test"
        }
        fn description(&self) -> &'static str {
            "A plugin for tests."
        }
        fn version(&self) -> PluginVersion {
            PluginVersion::Hash("1a2b3c4")
        }
        fn menu(&self) -> &'static [MenuItem] {
            &MENU
        }
        fn read_patterns(&self) -> &'static [&'static str] {
            &["*.md"]
        }
        fn commands(&self) -> &'static [CommandSpec] {
            &[]
        }
        async fn call(
            &self,
            method: &str,
            params: Value,
            ctx: &PluginCtx,
        ) -> Result<Value, PluginError> {
            match method {
                "echo" => Ok(params),
                "sleep" => {
                    let ms = params["ms"].as_u64().unwrap_or(0);
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    Ok(json!("slept"))
                }
                "big" => Ok(Value::String("x".repeat(PLUGIN_MAX_BYTES))),
                "fail" => Err(PluginError::new(-32150, "it failed")),
                "outside" => Err(PluginError::new(-1, "a bad code")),
                "notify" => {
                    ctx.notify("progress", json!({"step": 1}));
                    Ok(json!("notified"))
                }
                "project" => Ok(json!(ctx.project()?.name)),
                other => Err(PluginError::no_method(other)),
            }
        }
        fn tools(&self) -> &[ToolSpec] {
            &self.tools
        }
        async fn run_tool(
            &self,
            name: &str,
            args: Value,
            ctx: &PluginCtx,
        ) -> Result<Value, PluginError> {
            match name {
                "echo" => Ok(args),
                // Over the general agent's 64 KiB, under the 256 KiB cap.
                "big" => Ok(Value::String("€".repeat(30 * 1024))),
                "project" => Ok(json!(ctx.project()?.name)),
                "write" => {
                    self.writes.fetch_add(1, Ordering::SeqCst);
                    Ok(json!("written"))
                }
                other => Err(PluginError::no_method(other)),
            }
        }
    }

    struct Env {
        _dir: tempfile::TempDir,
        config: PathBuf,
        host: Arc<PluginHost>,
    }

    fn env(text: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        std::fs::write(&config, text).unwrap();
        let app: AppConfig = toml::from_str(text).unwrap();
        let host = PluginHost::new(
            &app,
            HostOptions {
                config_path: Some(config.clone()),
                plugins: Some(vec![Arc::new(TestPlugin::named("dev.flankr.plugins.test"))]),
                env: Some(CommandEnv::default()),
                interval: None,
            },
        );
        Env {
            _dir: dir,
            config,
            host,
        }
    }

    const ON: &str = "[plugins.\"dev.flankr.plugins.test\"]\nenabled = true\n";

    /// A caller whose notifications arrive on the receiver.
    fn caller(pane: Option<&str>) -> (Caller, mpsc::UnboundedReceiver<Notification>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let root = std::env::temp_dir().display().to_string();
        (
            Caller {
                principal: Principal::local("t"),
                pane: pane.map(str::to_string),
                lookup: Arc::new(FakeLookup(vec![("p".into(), "claude".into(), root)])),
                sink: Arc::new(move |n| {
                    let _ = tx.send(n);
                }),
                answered: None,
            },
            rx,
        )
    }

    fn call(method: &str, id: Option<u64>, params: Value) -> PluginCall {
        PluginCall {
            ns: "dev.flankr.plugins.test".into(),
            id,
            method: method.into(),
            params,
            session: None,
        }
    }

    async fn result(rx: &mut mpsc::UnboundedReceiver<Notification>) -> PluginResult {
        loop {
            let n = rx.recv().await.expect("a notification");
            if n.method == companion::methods::PLUGIN_RESULT {
                return serde_json::from_value(n.params).unwrap();
            }
        }
    }

    #[test]
    fn the_planning_icon_and_opens_reach_plugin_info() {
        let item = PluginMenuItem {
            id: "plans".into(),
            label: "Plans in this project".into(),
            opens: "menu".into(),
            screens: "both".into(),
            icon: Some("plans".into()),
        };
        let app: AppConfig = toml::from_str("").unwrap();
        let host = PluginHost::new(
            &app,
            HostOptions {
                config_path: None,
                plugins: Some(vec![Arc::new(crate::plugins::planning::Planning)]),
                env: Some(CommandEnv::default()),
                interval: None,
            },
        );
        let infos = host.infos();
        assert_eq!(infos[0].ns, "dev.flankr.plugins.planning");
        assert_eq!(infos[0].version, "0.1.2");
        assert_eq!(infos[0].menu, vec![item]);
    }

    #[test]
    fn every_plugin_is_listed_off_until_enabled() {
        let e = env("");
        let infos = e.host.infos();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].ns, "dev.flankr.plugins.test");
        assert_eq!(infos[0].version, "g1a2b3c4");
        assert!(!infos[0].enabled);
        assert_eq!(infos[0].menu[0].opens, "card");
        assert_eq!(infos[0].menu[0].screens, "overview");
        assert_eq!(infos[0].menu[0].icon, None);
        let e = env(&format!("{ON}[llm.uses.planning]\nread = [\"*.md\"]\n"));
        assert!(e.host.infos()[0].enabled);
        assert_eq!(e.host.infos()[0].reason, None);
        // Enabled, with no read patterns: the reason says so.
        let e = env(ON);
        assert_eq!(e.host.infos()[0].reason.as_deref(), Some(ctx::READING_OFF));
    }

    #[tokio::test]
    async fn refusals_answered_at_once() {
        let e = env("");
        let slots = CallSlots::default();
        let (c, _) = caller(None);
        let err = e
            .host
            .call(call("echo", Some(1), json!({})), c, &slots)
            .unwrap_err();
        assert_eq!(err.code, codes::PLUGIN_DISABLED);
        let e = env(ON);
        for (ns, method, code) in [
            ("dev.flankr.plugins.none", "echo", codes::PLUGIN_UNKNOWN),
            ("Dev.Flankr", "echo", codes::INVALID_PARAMS),
            ("dev.flankr.plugins.test", "Echo!", codes::INVALID_PARAMS),
            (
                "dev.flankr.plugins.test",
                "plugin call",
                codes::INVALID_PARAMS,
            ),
        ] {
            let (c, _) = caller(None);
            let mut p = call(method, Some(1), json!({}));
            p.ns = ns.into();
            assert_eq!(
                e.host.call(p, c, &slots).unwrap_err().code,
                code,
                "{ns} {method}"
            );
        }
        let (c, _) = caller(None);
        let big = json!({"x": "y".repeat(PLUGIN_MAX_BYTES)});
        let err = e
            .host
            .call(call("echo", Some(1), big), c, &slots)
            .unwrap_err();
        assert_eq!(err.code, codes::PLUGIN_TOO_LARGE);
        assert_eq!(slots.running(), 0);
    }

    #[tokio::test]
    async fn results_follow_on_the_calling_sink_with_the_plugins_ns() {
        let e = env(ON);
        let slots = CallSlots::default();
        let (a, mut a_rx) = caller(Some("p"));
        let (b, mut b_rx) = caller(None);
        e.host
            .call(call("echo", Some(7), json!({"x": 1})), a, &slots)
            .unwrap();
        let r = result(&mut a_rx).await;
        assert_eq!(r.ns, "dev.flankr.plugins.test");
        assert_eq!((r.id, r.result, r.error), (7, Some(json!({"x": 1})), None));
        // The other connection hears nothing.
        e.host
            .call(call("notify", Some(8), json!({})), b, &slots)
            .unwrap();
        let note = b_rx.recv().await.unwrap();
        assert_eq!(note.method, companion::methods::PLUGIN_NOTIFY);
        let note: PluginNotify = serde_json::from_value(note.params).unwrap();
        assert_eq!(
            (note.ns.as_str(), note.method.as_str()),
            ("dev.flankr.plugins.test", "progress")
        );
        assert_eq!(result(&mut b_rx).await.result, Some(json!("notified")));
        assert!(a_rx.try_recv().is_err());
        // The plugin's own error, a bad code, a result over the cap, a
        // method it does not have, a project.
        for (method, code) in [
            ("fail", -32150),
            ("outside", codes::INTERNAL_ERROR),
            ("big", codes::PLUGIN_TOO_LARGE),
            ("missing", codes::METHOD_NOT_FOUND),
        ] {
            let (c, mut rx) = caller(None);
            e.host
                .call(call(method, Some(9), json!({})), c, &slots)
                .unwrap();
            let r = result(&mut rx).await;
            assert_eq!(r.error.map(|e| e.code), Some(code), "{method}");
            assert_eq!(r.result, None);
        }
        let (c, mut rx) = caller(None);
        e.host
            .call(call("project", Some(10), json!({})), c, &slots)
            .unwrap();
        let r = result(&mut rx).await;
        assert_eq!(
            r.error.map(|e| e.message).as_deref(),
            Some(crate::project::NO_PROJECT)
        );
        let (c, mut rx) = caller(Some("p"));
        e.host
            .call(call("project", Some(11), json!({})), c, &slots)
            .unwrap();
        assert!(result(&mut rx).await.result.is_some());
    }

    #[tokio::test]
    async fn a_call_without_an_id_gets_no_result() {
        let e = env(ON);
        let slots = CallSlots::default();
        let (c, mut rx) = caller(None);
        e.host
            .call(call("echo", None, json!({})), c, &slots)
            .unwrap();
        // The task ends and drops its sink: the channel closes empty.
        assert!(rx.recv().await.is_none());
        assert_eq!(slots.running(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_fifth_call_is_busy_and_a_long_one_cancelled() {
        let e = env(ON);
        let slots = CallSlots::default();
        let mut rxs = Vec::new();
        for id in 0..4 {
            let (c, rx) = caller(None);
            e.host
                .call(call("sleep", Some(id), json!({"ms": 60_000})), c, &slots)
                .unwrap();
            rxs.push(rx);
        }
        let (c, _) = caller(None);
        let err = e
            .host
            .call(call("echo", Some(5), json!({})), c, &slots)
            .unwrap_err();
        assert_eq!(err.code, codes::BUSY);
        for rx in &mut rxs {
            let r = result(rx).await;
            assert_eq!(r.error.map(|e| e.code), Some(codes::CANCELLED));
        }
        tokio::task::yield_now().await;
        assert_eq!(slots.running(), 0);
        let (c, mut rx) = caller(None);
        e.host
            .call(call("sleep", Some(6), json!({"ms": 9_000})), c, &slots)
            .unwrap();
        assert_eq!(result(&mut rx).await.result, Some(json!("slept")));
    }

    #[tokio::test]
    async fn closing_the_connection_stops_its_calls() {
        let e = env(ON);
        let slots = CallSlots::default();
        let (c, mut rx) = caller(None);
        e.host
            .call(call("sleep", Some(1), json!({"ms": 60_000})), c, &slots)
            .unwrap();
        assert_eq!(slots.running(), 1);
        slots.close();
        assert!(rx.recv().await.is_none());
        assert_eq!(slots.running(), 0);
    }

    #[test]
    fn set_enabled_needs_approval_and_keeps_the_file() {
        let text = "# my flankr\n[link]\nlisten = []  # off\n\n[plugins.\"dev.flankr.plugins.other\"]\nenabled = true\n";
        let e = env(text);
        let mut rx = e.host.subscribe();
        let local = Principal::local("t");
        let params = PluginSetEnabledParams {
            ns: "dev.flankr.plugins.test".into(),
            enabled: true,
        };
        let err = e.host.set_enabled(&local, params.clone()).unwrap_err();
        assert_eq!(err.code, codes::MAY_NOT_APPROVE);
        let approver = Principal {
            can_approve: true,
            ..Principal::local("t")
        };
        let err = e
            .host
            .set_enabled(
                &approver,
                PluginSetEnabledParams {
                    ns: "dev.flankr.plugins.none".into(),
                    enabled: true,
                },
            )
            .unwrap_err();
        assert_eq!(err.code, codes::PLUGIN_UNKNOWN);
        assert!(!rx.has_changed().unwrap());
        assert_eq!(
            e.host.set_enabled(&approver, params).unwrap(),
            PluginSetEnabledResult { enabled: true }
        );
        let written = std::fs::read_to_string(&e.config).unwrap();
        assert!(written.starts_with(text), "{written}");
        assert!(
            written.ends_with("\n[plugins.\"dev.flankr.plugins.test\"]\nenabled = true\n"),
            "{written}"
        );
        // Read again at once: subscribers hear it.
        assert!(rx.has_changed().unwrap());
        assert!(rx.borrow_and_update()[0].enabled);
        // Off again: only that key changes.
        e.host
            .set_enabled(
                &approver,
                PluginSetEnabledParams {
                    ns: "dev.flankr.plugins.test".into(),
                    enabled: false,
                },
            )
            .unwrap();
        let written = std::fs::read_to_string(&e.config).unwrap();
        assert!(written.contains("[plugins.\"dev.flankr.plugins.test\"]\nenabled = false\n"));
        assert!(written.contains("listen = []  # off"));
        assert!(!e.host.infos()[0].enabled);
    }

    #[test]
    fn writing_a_missing_file_a_broken_one_and_keeping_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new").join("config.toml");
        write_enabled(&path, "dev.flankr.plugins.test", true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[plugins.\"dev.flankr.plugins.test\"]\nenabled = true\n"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let kept = dir.path().join("kept.toml");
        std::fs::write(&kept, "[naming]\n").unwrap();
        std::fs::set_permissions(&kept, std::fs::Permissions::from_mode(0o640)).unwrap();
        write_enabled(&kept, "dev.flankr.plugins.test", true).unwrap();
        assert_eq!(
            std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let broken = dir.path().join("broken.toml");
        std::fs::write(&broken, "[naming\n").unwrap();
        assert_eq!(
            write_enabled(&broken, "dev.flankr.plugins.test", true).unwrap_err(),
            BROKEN_CONFIG
        );
        assert_eq!(std::fs::read_to_string(&broken).unwrap(), "[naming\n");
        let scalar = dir.path().join("scalar.toml");
        std::fs::write(&scalar, "plugins = 3\n").unwrap();
        assert_eq!(
            write_enabled(&scalar, "dev.flankr.plugins.test", true).unwrap_err(),
            BROKEN_CONFIG
        );
        // No temporary files are left behind.
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty());
    }

    #[test]
    fn a_hand_edit_is_read_on_the_next_check() {
        let e = env("");
        let mut rx = e.host.subscribe();
        assert!(!e.host.check(false), "unchanged");
        std::fs::write(&e.config, format!("# edited\n{ON}")).unwrap();
        assert!(e.host.check(true));
        assert!(rx.has_changed().unwrap());
        assert!(rx.borrow_and_update()[0].enabled);
        // A broken file leaves the last good settings.
        std::fs::write(&e.config, "[plugins\n").unwrap();
        assert!(!e.host.check(true));
        assert!(e.host.infos()[0].enabled);
    }

    fn tool_names(host: &PluginHost) -> Vec<String> {
        host.agent_tools().into_iter().map(|t| t.name).collect()
    }

    #[test]
    fn agent_tools_are_offered_as_the_setting_allows() {
        // Off: none.
        let e = env("");
        assert!(tool_names(&e.host).is_empty());
        // On: the read-only ones, namespaced, in the plugin's order.
        let e = env(ON);
        assert_eq!(
            tool_names(&e.host),
            ["test_echo", "test_big", "test_project"]
        );
        let echo = &e.host.agent_tools()[0];
        assert_eq!(echo.description, "A test tool.");
        assert_eq!(
            serde_json::from_str::<Value>(&echo.params).unwrap(),
            json!({"type": "object", "properties": {}})
        );
        assert!(e.host.agent_tool("test_write").is_none());
        let found = e.host.agent_tool("test_echo").unwrap();
        assert_eq!(
            (found.plugin, found.tool, found.read_only),
            ("Test", "echo", true)
        );
        // `true` is the default; `false` offers none; "all" every one.
        let e = env(&format!("{ON}agent_tools = true\n"));
        assert_eq!(tool_names(&e.host).len(), 3);
        let e = env(&format!("{ON}agent_tools = false\n"));
        assert!(tool_names(&e.host).is_empty());
        let e = env(&format!("{ON}agent_tools = \"all\"\n"));
        assert_eq!(
            tool_names(&e.host),
            ["test_echo", "test_big", "test_project", "test_write"]
        );
        assert!(!e.host.agent_tool("test_write").unwrap().read_only);
        // Anything else is the default.
        let e = env(&format!("{ON}agent_tools = \"everything\"\n"));
        assert_eq!(tool_names(&e.host).len(), 3);
    }

    #[test]
    fn a_reload_changes_the_tools_offered() {
        let e = env(ON);
        assert_eq!(tool_names(&e.host).len(), 3);
        std::fs::write(&e.config, format!("{ON}agent_tools = \"all\"\n")).unwrap();
        e.host.check(true);
        assert_eq!(tool_names(&e.host).len(), 4);
        std::fs::write(&e.config, "").unwrap();
        e.host.check(true);
        assert!(tool_names(&e.host).is_empty());
    }

    fn tool_caller(pane: Option<&str>) -> AgentToolCaller {
        let root = std::env::temp_dir().display().to_string();
        AgentToolCaller {
            principal: Principal::local("t"),
            pane: pane.map(str::to_string),
            lookup: Arc::new(FakeLookup(vec![("p".into(), "claude".into(), root)])),
        }
    }

    #[tokio::test]
    async fn agent_tools_run_through_the_plugins_ctx() {
        let e = env(ON);
        let echoed = e
            .host
            .run_agent_tool("test_echo", json!({"x": 1}), tool_caller(None))
            .await
            .unwrap();
        assert_eq!(echoed, json!({"x": 1}));
        // The project is the caller's current agent's, as for menu calls.
        let err = e
            .host
            .run_agent_tool("test_project", json!({}), tool_caller(None))
            .await
            .unwrap_err();
        assert_eq!(err.code, super::super::codes::PROJECT_UNAVAILABLE);
        assert_eq!(err.message, crate::project::NO_PROJECT);
        let named = e
            .host
            .run_agent_tool("test_project", json!({}), tool_caller(Some("p")))
            .await
            .unwrap();
        assert!(named.is_string());
        // Not offered (read-only only by default), unknown, or over the
        // cap: refused.
        for name in ["test_write", "test_none", "echo"] {
            let err = e
                .host
                .run_agent_tool(name, json!({}), tool_caller(None))
                .await
                .unwrap_err();
            assert_eq!(err.code, codes::METHOD_NOT_FOUND, "{name}");
        }
        let big = json!({"x": "y".repeat(PLUGIN_MAX_BYTES)});
        let err = e
            .host
            .run_agent_tool("test_echo", big, tool_caller(None))
            .await
            .unwrap_err();
        assert_eq!(err.code, codes::PLUGIN_TOO_LARGE);
    }
}
