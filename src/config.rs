//! Configuration manager for `handlr`.
//!
//! Loads TOML settings from `~/.config/handlr/config.toml` (or custom path),
//! covering transports (`[[transports]]`), per-agent command permissions
//! (`[[agents]]`), and LLM parameters.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    #[serde(default)]
    pub transports: Vec<TransportConfig>,
    #[serde(default = "default_agents")]
    pub agents: Vec<AgentConfig>,
    /// Master switch for arbitrary (non allow-listed) commands. `Some(true)`
    /// enables them for every agent, `Some(false)` disables them for every
    /// agent, and `None` leaves each agent's own setting untouched.
    #[serde(default)]
    pub allow_arbitrary_commands: Option<bool>,
    #[serde(default)]
    pub gemini: GeminiConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub naming: NamingConfig,
    #[serde(default)]
    pub link: LinkConfig,
    #[serde(default)]
    pub urls: UrlsConfig,
}

/// `[urls]`: addresses the agent view links to (plan 20261022_03, the
/// 21:57 EDT amendment; owner, 2026-10-23: "make the section urls then
/// option repowiki_url").
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UrlsConfig {
    /// The repowiki's address (`http://host:8088`): `docs/…` paths link to
    /// their pages there. Without it they are not linked.
    pub repowiki_url: Option<String>,
}

fn default_agents() -> Vec<AgentConfig> {
    vec![
        AgentConfig {
            name: "claude".to_string(),
            allowed_commands: vec!["continue".to_string(), "/compact".to_string()],
            require_confirmation: true,
            allow_arbitrary_commands: false,
            allow_messages: true,
            interrupt_key: default_interrupt_key(),
        },
        AgentConfig {
            name: "omp".to_string(),
            allowed_commands: vec!["continue".to_string()],
            require_confirmation: true,
            allow_arbitrary_commands: false,
            allow_messages: true,
            interrupt_key: default_interrupt_key(),
        },
        AgentConfig {
            name: "agy".to_string(),
            allowed_commands: vec!["continue".to_string()],
            require_confirmation: true,
            allow_arbitrary_commands: false,
            allow_messages: true,
            interrupt_key: default_interrupt_key(),
        },
    ]
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            transports: Vec::new(),
            agents: default_agents(),
            allow_arbitrary_commands: None,
            gemini: GeminiConfig::default(),
            server: ServerConfig::default(),
            naming: NamingConfig::default(),
            link: LinkConfig::default(),
            urls: UrlsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TransportConfig {
    /// Optional name/label for this transport (e.g. "local", "mac-mini", "prod")
    pub name: Option<String>,
    /// Transport mode: "local" or "remote"
    #[serde(default = "default_transport_mode")]
    pub mode: String,
    /// Unix Domain Socket path for local transport (default: ~/.config/herdr/herdr.sock)
    pub socket: Option<String>,
    /// SSH connection string for remote transport (e.g. "user@host")
    pub remote: Option<String>,
    /// Whether this transport is the default active transport
    pub default: Option<bool>,
}

fn default_transport_mode() -> String {
    "local".to_string()
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            name: Some("local".to_string()),
            mode: default_transport_mode(),
            socket: None,
            remote: None,
            default: Some(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    /// Herdr's determined agent name (lowercased, e.g. "claude", "omp", "agy", "shell")
    pub name: String,
    /// Permitted commands that voice/Gemini is allowed to send to this agent's panes
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    /// Whether vocal confirmation is required before executing a command
    #[serde(
        default = "default_require_confirmation",
        alias = "required_confirmation"
    )]
    pub require_confirmation: bool,
    /// Whether arbitrary safe commands are allowed (must be ASCII, no '/' or '-', '.' and ',' allowed)
    #[serde(default)]
    pub allow_arbitrary_commands: bool,
    /// Whether dictated messages may be sent: the strict characters of an
    /// arbitrary command, at most 4000 of them, always confirmed (plan
    /// 20261023_02, decision 6 as amended). Default `true`.
    #[serde(default = "default_allow_messages")]
    pub allow_messages: bool,
    /// The one key, in herdr's key names, that stops this agent's work
    /// when a companion turn is interrupted or `agents.interrupt` is
    /// called. Sent as a keystroke (`pane.send_keys`), never typed as text.
    /// Empty turns interrupting off (plan 20261022_02, 18:05 EDT amendment).
    #[serde(
        default = "default_interrupt_key",
        deserialize_with = "deserialize_interrupt_key"
    )]
    pub interrupt_key: String,
}

fn default_require_confirmation() -> bool {
    true
}

fn default_allow_messages() -> bool {
    true
}

/// herdr's canonical name for the Escape key (`herdr pane send-keys
/// --help`: "Use esc as the canonical Escape key name; escape is also
/// accepted").
pub const DEFAULT_INTERRUPT_KEY: &str = "esc";

fn default_interrupt_key() -> String {
    DEFAULT_INTERRUPT_KEY.to_string()
}

/// herdr's named keys (its keybinding parser: `enter`, `esc`, `tab`,
/// `backspace`, `space`, `f1`–`f12`; `escape` is also accepted).
const NAMED_KEYS: [&str; 6] = ["esc", "escape", "enter", "tab", "backspace", "space"];
/// Modifiers in a herdr key combo such as `ctrl+c`.
const KEY_MODIFIERS: [&str; 3] = ["ctrl", "alt", "shift"];

/// Whether `key` is one key in herdr's vocabulary: a named key, a function
/// key, or one printable ASCII character, optionally after `ctrl+`, `alt+`
/// or `shift+`. The empty string (interrupting off) is accepted.
pub fn validate_interrupt_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Ok(());
    }
    let bad = || {
        format!(
            "interrupt_key {key:?} is not a single herdr key name (such as \"esc\", \"enter\", \"f2\" or \"ctrl+c\"); use \"\" to turn interrupting off"
        )
    };
    if key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    let lower = key.to_ascii_lowercase();
    // The last part is the key; "+" itself is a key ("+", "ctrl++").
    let (mods, base) = if lower == "+" {
        ("", "+")
    } else if let Some(mods) = lower.strip_suffix("++") {
        (mods, "+")
    } else {
        match lower.rsplit_once('+') {
            Some((_, "")) | Some(("", _)) => return Err(bad()),
            Some((mods, base)) => (mods, base),
            None => ("", lower.as_str()),
        }
    };
    if !mods.is_empty() && !mods.split('+').all(|m| KEY_MODIFIERS.contains(&m)) {
        return Err(bad());
    }
    let function_key = base
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n));
    let single = base.chars().count() == 1 && base.chars().all(|c| c.is_ascii_graphic());
    if NAMED_KEYS.contains(&base) || function_key || single {
        Ok(())
    } else {
        Err(bad())
    }
}

fn deserialize_interrupt_key<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let key = String::deserialize(d)?;
    let key = key.trim().to_string();
    validate_interrupt_key(&key).map_err(serde::de::Error::custom)?;
    Ok(key)
}

impl AgentConfig {
    pub fn new(
        name: impl Into<String>,
        allowed_commands: Vec<String>,
        require_confirmation: bool,
    ) -> Self {
        Self {
            name: name.into(),
            allowed_commands,
            require_confirmation,
            allow_arbitrary_commands: false,
            allow_messages: true,
            interrupt_key: default_interrupt_key(),
        }
    }

    pub fn with_arbitrary_commands(mut self, allow: bool) -> Self {
        self.allow_arbitrary_commands = allow;
        self
    }

    pub fn with_messages(mut self, allow: bool) -> Self {
        self.allow_messages = allow;
        self
    }

    /// Set the interrupt key; `""` turns interrupting off.
    pub fn with_interrupt_key(mut self, key: impl Into<String>) -> Self {
        self.interrupt_key = key.into();
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GeminiConfig {
    /// The file holding the Gemini API key, alone, with mode 0600 (default:
    /// `.private/gemini_api_key`, from the daemon's working directory;
    /// `.private` is gitignored). Without the file, `GEMINI_API_KEY` is
    /// used. The key itself is never put in this file.
    pub api_key_file: Option<String>,
    /// Model identifier (default: "gemini-2.5-flash")
    pub model: Option<String>,
}

/// `[server]`: the daemon (`handlr serve`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ServerConfig {
    /// The daemon socket (default: ~/.config/handlr/handlr.sock).
    pub socket: Option<String>,
    /// Seconds a confirmation waits for an answer, other than one asked for
    /// by a companion's own voice turn (default: 3600).
    pub confirm_ttl: Option<u64>,
}

/// `[link]`: the encrypted link for paired devices
/// (docs/design/20261022_02_LINK_AND_COMPANION_PLAN.md, the 11:58 EDT
/// amendment).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LinkConfig {
    /// Addresses to listen on, `<addr>:<port>` or `<addr>` (port 7347).
    /// Replaces the default `127.0.0.1:7347` and turns off the Tailscale
    /// check; these are the addresses a pairing string carries. An empty
    /// list turns the link off.
    pub listen: Option<Vec<String>>,
    /// The server's private key (default: ~/.config/handlr/link_key).
    pub key: Option<String>,
    /// The paired clients (default: ~/.config/handlr/auth.toml).
    pub auth: Option<String>,
    /// Seconds between checks of open link connections against
    /// `auth.toml` (default: 60).
    pub revoke_check_secs: Option<u64>,
}

/// `[naming]`: how agents are named aloud and on screen
/// (docs/design/20261022_05_SPOKEN_NAMES_PLAN.md, decision 8).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NamingConfig {
    /// Words of at most this many letters with no vowel are spelled out
    /// letter by letter. 0 turns spelling off.
    pub spell_max_len: usize,
    /// The letters that make a word readable as written.
    pub vowels: String,
    /// Letter -> the name it is spelled with; unlisted letters keep the
    /// default English names.
    pub letters: BTreeMap<String, String>,
    /// A tab label as written, a project, or an agent name (matched
    /// case-insensitively) -> what to say and show instead.
    pub aliases: BTreeMap<String, String>,
    /// Where an agent's project name comes from: its directory (the
    /// default), or herdr's workspace label for those who name workspaces.
    pub project_from: ProjectFrom,
}

/// `[naming] project_from` (owner, 2026-10-23: "the herdr session name
/// isn't useful compared to the agent/dir name, it could be an option for
/// others that use them properly but I just use the tabs").
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectFrom {
    /// The pane's cwd's last part; the workspace label when there is no
    /// cwd.
    #[default]
    Directory,
    /// The workspace label, or the cwd's basename when another agent
    /// shares that label (plan 20261022_05, decision 3).
    Workspace,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            spell_max_len: 6,
            vowels: "aeiouy".to_string(),
            letters: BTreeMap::new(),
            aliases: BTreeMap::new(),
            project_from: ProjectFrom::default(),
        }
    }
}

impl AppConfig {
    /// Resolve default config location: `~/.config/handlr/config.toml`
    pub fn default_config_path() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home)
                .join(".config")
                .join("handlr")
                .join("config.toml")
        } else {
            PathBuf::from("config.toml")
        }
    }

    /// Load config from file if present, or return defaults.
    pub fn load_or_default(custom_path: Option<&Path>) -> Self {
        let path = custom_path
            .map(PathBuf::from)
            .unwrap_or_else(Self::default_config_path);
        if !path.exists() {
            log::debug!(
                "Config file not found at {}, using defaults",
                path.display()
            );
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<AppConfig>(&content) {
                Ok(mut config) => {
                    log::info!("Loaded configuration from {}", path.display());
                    let global = config.allow_arbitrary_commands;
                    config.apply_arbitrary_commands_override(global);
                    config
                }
                Err(e) => {
                    log::warn!(
                        "Failed to parse config at {}: {}, using defaults",
                        path.display(),
                        e
                    );
                    Self::default()
                }
            },
            Err(e) => {
                log::warn!(
                    "Failed to read config file {}: {}, using defaults",
                    path.display(),
                    e
                );
                Self::default()
            }
        }
    }

    /// Apply a global arbitrary-command override to every configured agent.
    ///
    /// `Some(true)`/`Some(false)` force the setting on/off for all agents;
    /// `None` is a no-op so per-agent settings survive.
    pub fn apply_arbitrary_commands_override(&mut self, allow: Option<bool>) {
        let Some(allow) = allow else { return };
        self.allow_arbitrary_commands = Some(allow);
        for agent in &mut self.agents {
            agent.allow_arbitrary_commands = allow;
        }
        log::info!(
            "Arbitrary command support globally {} for all {} configured agents",
            if allow { "enabled" } else { "disabled" },
            self.agents.len()
        );
    }

    /// Find an active transport by name/remote string or default flag.
    pub fn find_transport(&self, name_or_target: Option<&str>) -> Option<&TransportConfig> {
        if let Some(target) = name_or_target
            && let Some(t) = self
                .transports
                .iter()
                .find(|t| t.name.as_deref() == Some(target) || t.remote.as_deref() == Some(target))
        {
            return Some(t);
        }
        // Look for explicitly marked default: true
        if let Some(t) = self.transports.iter().find(|t| t.default == Some(true)) {
            return Some(t);
        }
        // Fall back to first declared transport
        self.transports.first()
    }

    /// Find an agent configuration by name (case-insensitive).
    pub fn find_agent(&self, name: &str) -> Option<&AgentConfig> {
        let lower = name.to_lowercase();
        self.agents.iter().find(|a| a.name.to_lowercase() == lower)
    }

    /// Documented default TOML template, embedded from `config.default.toml`
    /// so the shipped file and the `--init` output can never drift apart.
    pub fn default_toml_template() -> String {
        include_str!("../config.default.toml").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sample_transports_config() {
        let sample = r#"
[[transports]]
name = "local"
mode = "local"
socket = "/tmp/herdr.sock"
default = false

[[transports]]
name = "mini"
mode = "remote"
remote = "tmeade@timothys-mac-mini-1"
default = true

[[agents]]
name = "claude"
allowed_commands = ["continue", "/compact"]
require_confirmation = true
"#;
        let cfg: AppConfig = toml::from_str(sample).unwrap();
        assert_eq!(cfg.transports.len(), 2);
        let active = cfg.find_transport(None).unwrap();
        assert_eq!(active.name.as_deref(), Some("mini"));
        assert_eq!(active.mode, "remote");
        assert_eq!(active.remote.as_deref(), Some("tmeade@timothys-mac-mini-1"));

        let local = cfg.find_transport(Some("local")).unwrap();
        assert_eq!(local.mode, "local");
        assert_eq!(local.socket.as_deref(), Some("/tmp/herdr.sock"));

        let claude_cfg = cfg.find_agent("Claude").unwrap();
        assert_eq!(claude_cfg.name, "claude");
        assert_eq!(claude_cfg.allowed_commands, vec!["continue", "/compact"]);
        assert!(claude_cfg.require_confirmation);
    }

    #[test]
    fn test_parse_arbitrary_commands_settings() {
        let sample = r#"
[[agents]]
name = "claude"
allowed_commands = ["continue"]
require_confirmation = true
allow_arbitrary_commands = true

[[agents]]
name = "omp"
allowed_commands = ["continue"]
"#;
        let cfg: AppConfig = toml::from_str(sample).unwrap();
        assert_eq!(cfg.server, ServerConfig::default());
        assert!(cfg.find_agent("claude").unwrap().allow_arbitrary_commands);
        // Defaults to off when the key is omitted.
        assert!(!cfg.find_agent("omp").unwrap().allow_arbitrary_commands);
        assert_eq!(cfg.allow_arbitrary_commands, None);
    }

    #[test]
    fn test_global_arbitrary_commands_override() {
        let sample = r#"
allow_arbitrary_commands = true

[[agents]]
name = "claude"
allowed_commands = ["continue"]

[[agents]]
name = "agy"
allowed_commands = ["continue"]
allow_arbitrary_commands = false
"#;
        let mut cfg: AppConfig = toml::from_str(sample).unwrap();
        let global = cfg.allow_arbitrary_commands;
        assert_eq!(global, Some(true));

        // Some(true) turns it on everywhere, including agents that opted out.
        cfg.apply_arbitrary_commands_override(global);
        assert!(cfg.agents.iter().all(|a| a.allow_arbitrary_commands));

        // Some(false) is a master kill switch.
        cfg.apply_arbitrary_commands_override(Some(false));
        assert!(cfg.agents.iter().all(|a| !a.allow_arbitrary_commands));

        // None leaves per-agent settings untouched.
        cfg.agents[0].allow_arbitrary_commands = true;
        cfg.apply_arbitrary_commands_override(None);
        assert!(cfg.agents[0].allow_arbitrary_commands);
        assert!(!cfg.agents[1].allow_arbitrary_commands);
    }

    #[test]
    fn interrupt_keys_default_to_esc_and_are_validated() {
        let cfg: AppConfig = toml::from_str(
            r#"
[[agents]]
name = "claude"

[[agents]]
name = "omp"
interrupt_key = ""

[[agents]]
name = "codex"
interrupt_key = "ctrl+c"
"#,
        )
        .unwrap();
        assert_eq!(cfg.find_agent("claude").unwrap().interrupt_key, "esc");
        assert_eq!(cfg.find_agent("omp").unwrap().interrupt_key, "");
        assert_eq!(cfg.find_agent("codex").unwrap().interrupt_key, "ctrl+c");
        assert!(
            AppConfig::default()
                .agents
                .iter()
                .all(|a| a.interrupt_key == DEFAULT_INTERRUPT_KEY)
        );

        for good in [
            "esc",
            "Escape",
            "enter",
            "f12",
            "q",
            "ctrl+c",
            "alt+shift+x",
            "+",
        ] {
            assert!(validate_interrupt_key(good).is_ok(), "{good}");
        }
        for bad in [
            "esc enter",
            "continue",
            "f13",
            "ctrl+",
            "hyper+c",
            "ctrl+cc",
            "\u{1b}",
        ] {
            assert!(validate_interrupt_key(bad).is_err(), "{bad}");
        }
        // A bad key fails the whole entry at load.
        let err = toml::from_str::<AppConfig>(
            r#"
[[agents]]
name = "claude"
interrupt_key = "esc enter"
"#,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("not a single herdr key name"),
            "{err}"
        );
    }

    #[test]
    fn link_settings_parse() {
        let cfg: AppConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.link, LinkConfig::default());
        let cfg: AppConfig = toml::from_str(
            r#"
[link]
listen = ["100.64.0.1", "127.0.0.1:7400"]
key = "/tmp/k"
auth = "/tmp/a.toml"
revoke_check_secs = 5
"#,
        )
        .unwrap();
        assert_eq!(
            cfg.link.listen,
            Some(vec!["100.64.0.1".to_string(), "127.0.0.1:7400".to_string()])
        );
        assert_eq!(cfg.link.key.as_deref(), Some("/tmp/k"));
        assert_eq!(cfg.link.auth.as_deref(), Some("/tmp/a.toml"));
        assert_eq!(cfg.link.revoke_check_secs, Some(5));
    }

    #[test]
    fn repowiki_is_off_unless_set() {
        let cfg: AppConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.urls.repowiki_url, None);
        let cfg: AppConfig = toml::from_str("[urls]\nrepowiki_url = \"http://w:8088\"\n").unwrap();
        assert_eq!(cfg.urls.repowiki_url.as_deref(), Some("http://w:8088"));
    }

    #[test]
    fn naming_defaults_when_absent() {
        let cfg: AppConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.naming, NamingConfig::default());
        assert_eq!(cfg.naming.spell_max_len, 6);
        assert_eq!(cfg.naming.vowels, "aeiouy");
    }

    #[test]
    fn naming_parses_and_ignores_unknown_keys() {
        let sample = r#"
[naming]
spell_max_len = 0
vowels = "aeiou"
no_such_key = true

[naming.letters]
w = "dub"

[naming.aliases]
"ctrls" = "controls"
"17-hr" = "handlr"
"#;
        let cfg: AppConfig = toml::from_str(sample).unwrap();
        assert_eq!(cfg.naming.spell_max_len, 0);
        assert_eq!(cfg.naming.vowels, "aeiou");
        assert_eq!(cfg.naming.letters.get("w").map(String::as_str), Some("dub"));
        assert_eq!(
            cfg.naming.aliases.get("17-hr").map(String::as_str),
            Some("handlr")
        );
        assert_eq!(cfg.naming.aliases.len(), 2);
    }

    #[test]
    fn messages_are_allowed_by_default_and_can_be_turned_off() {
        let cfg = AppConfig::default();
        for name in ["claude", "omp", "agy"] {
            assert!(cfg.find_agent(name).unwrap().allow_messages, "{name}");
        }
        assert!(AgentConfig::new("x", vec![], true).allow_messages);
        let sample = r#"
[[agents]]
name = "claude"

[[agents]]
name = "omp"
allow_messages = false
"#;
        let cfg: AppConfig = toml::from_str(sample).unwrap();
        assert!(cfg.find_agent("claude").unwrap().allow_messages);
        assert!(!cfg.find_agent("omp").unwrap().allow_messages);
    }

    #[test]
    fn test_default_toml_template_valid() {
        let template = AppConfig::default_toml_template();
        let cfg: Result<AppConfig, _> = toml::from_str(&template);
        assert!(
            cfg.is_ok(),
            "Template should be valid TOML: {:?}",
            cfg.err()
        );
        let parsed = cfg.unwrap();
        assert_eq!(parsed.transports.len(), 2);
        assert_eq!(parsed.agents.len(), 3);
        let claude = parsed.find_agent("claude").unwrap();
        assert_eq!(claude.allowed_commands, vec!["continue", "/compact"]);
        assert!(claude.require_confirmation);
        assert_eq!(parsed.naming, NamingConfig::default());
    }
}
