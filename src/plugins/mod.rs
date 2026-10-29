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

pub mod command;
pub mod ctx;
pub mod host;
pub mod planning;
pub mod registry;

pub use command::{ArgShape, CommandOutput, CommandSpec};
pub use ctx::{HerdrLookup, PluginCtx, Project, ProjectLookup};
pub use host::{CallSlots, Caller, HostOptions, PluginHost};

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
}

impl Opens {
    pub fn as_str(self) -> &'static str {
        match self {
            Opens::Modal => "modal",
            Opens::Card => "card",
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

/// One action a plugin adds to the ⋮ menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuItem {
    /// Unique within the plugin, e.g. "plans".
    pub id: &'static str,
    /// e.g. "Plans in this project".
    pub label: &'static str,
    pub opens: Opens,
    pub screens: Screens,
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
