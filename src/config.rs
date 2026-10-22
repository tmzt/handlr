//! Configuration manager for `handlr`.
//!
//! Loads TOML settings from `~/.config/handlr/config.toml` (or custom path),
//! covering transports (`[[transports]]`), per-agent command permissions
//! (`[[agents]]`), and LLM parameters.

use serde::{Deserialize, Serialize};
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
}

fn default_agents() -> Vec<AgentConfig> {
    vec![
        AgentConfig {
            name: "claude".to_string(),
            allowed_commands: vec!["continue".to_string(), "/compact".to_string()],
            require_confirmation: true,
            allow_arbitrary_commands: false,
        },
        AgentConfig {
            name: "omp".to_string(),
            allowed_commands: vec!["continue".to_string()],
            require_confirmation: true,
            allow_arbitrary_commands: false,
        },
        AgentConfig {
            name: "agy".to_string(),
            allowed_commands: vec!["continue".to_string()],
            require_confirmation: true,
            allow_arbitrary_commands: false,
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
}

fn default_require_confirmation() -> bool {
    true
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
        }
    }

    pub fn with_arbitrary_commands(mut self, allow: bool) -> Self {
        self.allow_arbitrary_commands = allow;
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GeminiConfig {
    /// Gemini API Key (falls back to GEMINI_API_KEY environment variable)
    pub api_key: Option<String>,
    /// Model identifier (default: "gemini-2.5-flash")
    pub model: Option<String>,
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
        if let Some(target) = name_or_target {
            if let Some(t) = self
                .transports
                .iter()
                .find(|t| t.name.as_deref() == Some(target) || t.remote.as_deref() == Some(target))
            {
                return Some(t);
            }
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
    }
}
