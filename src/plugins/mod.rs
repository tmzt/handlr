//! Built-in plugins, their daemon halves
//! (docs/design/20261028_04_PLUGINS_PLAN.md, decisions 3.1–3.5).
//!
//! A plugin is Rust compiled into the daemon, named by a reverse-DNS
//! package name (`dev.flankr.plugins.<name>`) that is its namespace on the
//! wire and its key under `[plugins]` in the config. It is off until its
//! `[plugins."<ns>"] enabled = true`. It is called through `plugin.call`
//! with methods of its own, and reaches the project only through
//! [`PluginCtx`]: the general agent's read-only operations with the same
//! restrictions, and the commands it declares in [`Plugin::commands`].
//! It never sends anything to an agent pane.
//!
//! A plugin may also contribute tools to the general agent
//! ([`Plugin::tools`], the plan's 02:10 EDT amendment of 2026-10-29),
//! named `<short name>_<tool>` and run through the same [`PluginCtx`].

pub mod command;
pub mod ctx;
pub mod host;
pub mod planning;
pub mod registry;

pub use command::{ArgShape, CommandOutput, CommandSpec};
pub use ctx::{HerdrLookup, PluginCtx, Project, ProjectLookup};
pub use host::{
    AgentToolCaller, AgentToolsSetting, CallSlots, Caller, HostOptions, OfferedTool, PluginHost,
};

/// A plugin's daemon half (decision 3.1, item 3).
#[async_trait::async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// The package name, e.g. "dev.flankr.plugins.planning".
    fn ns(&self) -> &'static str;
    /// Shown in Settings → Plugins and as the menu group.
    fn name(&self) -> &'static str;
    /// One sentence for Settings → Plugins.
    fn description(&self) -> &'static str;
    fn version(&self) -> PluginVersion;
    /// The actions it adds to the ⋮ menu.
    fn menu(&self) -> &'static [MenuItem];
    /// The project paths it needs, in `ReadPattern` syntax; reads are
    /// allowed only where these AND the configured `read` match.
    fn read_patterns(&self) -> &'static [&'static str];
    /// The commands it may run (decision 3.3, item 4); empty for none.
    fn commands(&self) -> &'static [CommandSpec];
    /// One plugin-local method. `params` is the call's JSON.
    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
        ctx: &PluginCtx,
    ) -> Result<serde_json::Value, PluginError>;
    /// The tools it offers the general agent (the 02:10 EDT amendment,
    /// item 1); none by default. The general agent sees each as
    /// [`agent_tool_name`]`(ns, tool.name)`.
    fn tools(&self) -> &[ToolSpec] {
        &[]
    }
    /// Runs one of its [`tools`](Self::tools), by its own `name` (without
    /// the short-name prefix). `args` is the model's JSON.
    async fn run_tool(
        &self,
        name: &str,
        _args: serde_json::Value,
        _ctx: &PluginCtx,
    ) -> Result<serde_json::Value, PluginError> {
        Err(PluginError::new(
            libflankr_proto::envelope::codes::METHOD_NOT_FOUND,
            format!("This plugin has no tool {name}."),
        ))
    }
}

/// One tool a plugin offers the general agent (the 02:10 EDT amendment,
/// item 1): a plain description; the plugin's own
/// [`Plugin::run_tool`] runs it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// The plugin-local name, e.g. "plans_list"; the general agent sees
    /// `planning_plans_list`.
    pub name: &'static str,
    /// What the model is told the tool does.
    pub description: &'static str,
    /// Its parameters as a JSON schema object, in the subset Gemini's
    /// function declarations take.
    pub params: serde_json::Value,
    /// It only reads. A tool that changes something is offered only with
    /// `agent_tools = "all"` and runs only after the voice confirmation.
    pub read_only: bool,
}

/// A plugin's short name: its namespace's last part (`planning` for
/// `dev.flankr.plugins.planning`).
pub fn short_name(ns: &str) -> &str {
    ns.rsplit('.').next().unwrap_or(ns)
}

/// What the general agent calls a plugin's tool: `<short name>_<tool>`,
/// e.g. `planning_plans_list`.
pub fn agent_tool_name(ns: &str, tool: &str) -> String {
    format!("{}_{tool}", short_name(ns))
}

/// A general agent tool's name: `[a-z][a-z0-9_]{0,47}`.
pub fn is_agent_tool_name(name: &str) -> bool {
    (1..=48).contains(&name.len())
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A plugin's version (decision 3.4): a release's semver, or a git hash
/// before its first release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginVersion {
    Semver(&'static str),
    Hash(&'static str),
}

impl PluginVersion {
    /// As `PluginInfo.version` carries it: `0.1.0`, or `g1a2b3c4`.
    pub fn text(self) -> String {
        match self {
            PluginVersion::Semver(v) => v.to_string(),
            PluginVersion::Hash(h) => format!("g{h}"),
        }
    }

    /// Whether it is well formed: `<major>.<minor>.<patch>` in digits, or
    /// 7 to 40 lowercase hex digits.
    pub fn valid(self) -> bool {
        match self {
            PluginVersion::Semver(v) => {
                let parts: Vec<&str> = v.split('.').collect();
                parts.len() == 3
                    && parts.iter().all(|p| {
                        !p.is_empty()
                            && p.bytes().all(|b| b.is_ascii_digit())
                            && (p.len() == 1 || !p.starts_with('0'))
                    })
            }
            PluginVersion::Hash(h) => {
                (7..=40).contains(&h.len())
                    && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            }
        }
    }
}

/// What a menu item opens (decision 3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opens {
    Modal,
    Card,
    /// A popup menu anchored to the item's title icon, with items the
    /// device half gives (plan 20261028_04, its 22:03 EDT amendment).
    Menu,
}

impl Opens {
    pub fn as_str(self) -> &'static str {
        match self {
            Opens::Modal => "modal",
            Opens::Card => "card",
            Opens::Menu => "menu",
        }
    }
}

/// Where a menu item is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screens {
    Overview,
    Agent,
    Both,
}

impl Screens {
    pub fn as_str(self) -> &'static str {
        match self {
            Screens::Overview => "overview",
            Screens::Agent => "agent",
            Screens::Both => "both",
        }
    }
}

/// One action a plugin adds to the ⋮ menu, or, with an `icon`, to the
/// title bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItem {
    /// Unique within the plugin, e.g. "plans".
    pub id: &'static str,
    /// e.g. "Plans in this project".
    pub label: &'static str,
    pub opens: Opens,
    pub screens: Screens,
    /// One of the app's icon names, e.g. "plans" (☑): the item is then a
    /// title-bar icon, not a ⋮ item.
    pub icon: Option<&'static str>,
}

/// A plugin's error, sent as `plugin.result`'s `error`. Plugins use the
/// codes -32100 … -32199 (see [`codes`]) or `INVALID_PARAMS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginError {
    pub code: i64,
    pub message: String,
}

impl PluginError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        PluginError {
            code,
            message: message.into(),
        }
    }

    /// `INVALID_PARAMS` with `message`.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(libflankr_proto::envelope::codes::INVALID_PARAMS, message)
    }

    /// `METHOD_NOT_FOUND` for `method`.
    pub fn no_method(method: &str) -> Self {
        Self::new(
            libflankr_proto::envelope::codes::METHOD_NOT_FOUND,
            format!("This plugin has no method {method}."),
        )
    }
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

/// The codes [`PluginCtx`] answers with, in the plugins' range.
pub mod codes {
    /// No project to work on: none is open, its folder is on another
    /// machine or unknown, or this device may not reach its agent.
    pub const PROJECT_UNAVAILABLE: i64 = -32100;
    /// A declared command can't run: its binary was not on `PATH` at start,
    /// or the config switched it off (decision 3.3, item 4).
    pub const COMMAND_UNAVAILABLE: i64 = -32101;
    /// A command or `git` ran and failed: a non-zero exit, the timeout, or
    /// output over its cap.
    pub const COMMAND_FAILED: i64 = -32102;
    /// A read was refused or failed: reading is off, the path may not be
    /// read, or it does not exist.
    pub const READ_REFUSED: i64 = -32103;
    /// The lowest and highest code a plugin may answer with.
    pub const FIRST: i64 = -32199;
    pub const LAST: i64 = -32100;
}
