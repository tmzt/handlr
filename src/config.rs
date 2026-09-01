//! Configuration manager for `handlr`.
//!
//! Loads TOML settings from `~/.config/handlr/config.toml` (or custom path),
//! covering transports (`[[transports]]`), voice configuration (TTS & STT engines,
//! voices, speed, models), and LLM parameters.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    #[serde(default)]
    pub transports: Vec<TransportConfig>,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default = "default_agents")]
    pub agents: Vec<AgentConfig>,
    #[serde(default)]
    pub gemini: GeminiConfig,
}

fn default_agents() -> Vec<AgentConfig> {
    vec![
        AgentConfig {
            name: "claude".to_string(),
            allowed_commands: vec!["continue".to_string(), "/compact".to_string()],
            require_confirmation: true,
        },
        AgentConfig {
            name: "omp".to_string(),
            allowed_commands: vec!["continue".to_string()],
            require_confirmation: true,
        },
        AgentConfig {
            name: "agy".to_string(),
            allowed_commands: vec!["continue".to_string()],
            require_confirmation: true,
        },
    ]
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            transports: Vec::new(),
            voice: VoiceConfig::default(),
            agents: default_agents(),
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VoiceConfig {
    #[serde(default)]
    pub tts: TtsSettings,
    #[serde(default)]
    pub stt: SttSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TtsSettings {
    /// TTS engine backend: "system", "sherpa-kokoro", "kokoro", or "sine"
    #[serde(default = "default_tts_engine")]
    pub engine: String,
    /// Voice preset name (e.g. "Samantha" for System, "af_bella" for Kokoro)
    pub voice: Option<String>,
    /// Playback speed multiplier (default: 1.0)
    #[serde(default = "default_speed")]
    pub speed: f32,
    /// Custom path to Kokoro ONNX model
    pub kokoro_model: Option<String>,
    /// Custom path to Kokoro voices.bin
    pub kokoro_voices: Option<String>,
    /// Custom directory holding sherpa-onnx Kokoro assets
    pub sherpa_model_dir: Option<String>,
}

fn default_tts_engine() -> String {
    "system".to_string()
}

fn default_speed() -> f32 {
    1.0
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            engine: default_tts_engine(),
            voice: None,
            speed: default_speed(),
            kokoro_model: None,
            kokoro_voices: None,
            sherpa_model_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SttSettings {
    /// STT engine backend: "whisper"
    #[serde(default = "default_stt_engine")]
    pub engine: String,
    /// Custom path to Silero VAD ONNX model
    pub silero_model: Option<String>,
    /// Custom path to Whisper GGML model
    pub whisper_model: Option<String>,
}

fn default_stt_engine() -> String {
    "whisper".to_string()
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            engine: default_stt_engine(),
            silero_model: None,
            whisper_model: None,
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
    #[serde(default = "default_require_confirmation", alias = "required_confirmation")]
    pub require_confirmation: bool,
}

fn default_require_confirmation() -> bool {
    true
}

impl AgentConfig {
    pub fn new(name: impl Into<String>, allowed_commands: Vec<String>, require_confirmation: bool) -> Self {
        Self {
            name: name.into(),
            allowed_commands,
            require_confirmation,
        }
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
            PathBuf::from(home).join(".config").join("handlr").join("config.toml")
        } else {
            PathBuf::from("config.toml")
        }
    }

    /// Load config from file if present, or return defaults.
    pub fn load_or_default(custom_path: Option<&Path>) -> Self {
        let path = custom_path.map(PathBuf::from).unwrap_or_else(Self::default_config_path);
        if !path.exists() {
            log::debug!("Config file not found at {}, using defaults", path.display());
            return Self::default();
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<AppConfig>(&content) {
                Ok(config) => {
                    log::info!("Loaded configuration from {}", path.display());
                    config
                }
                Err(e) => {
                    log::warn!("Failed to parse config at {}: {}, using defaults", path.display(), e);
                    Self::default()
                }
            },
            Err(e) => {
                log::warn!("Failed to read config file {}: {}, using defaults", path.display(), e);
                Self::default()
            }
        }
    }

    /// Find an active transport by name/remote string or default flag.
    pub fn find_transport(&self, name_or_target: Option<&str>) -> Option<&TransportConfig> {
        if let Some(target) = name_or_target {
            if let Some(t) = self.transports.iter().find(|t| {
                t.name.as_deref() == Some(target) || t.remote.as_deref() == Some(target)
            }) {
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

    /// Generate documented default TOML template string.
    pub fn default_toml_template() -> String {
        r#"# Default configuration for handlr
# Copy to ~/.config/handlr/config.toml to customize

[[transports]]
name = "local"
mode = "local"
# Optional socket path for local mode (default: ~/.config/herdr/herdr.sock)
# socket = "~/.config/herdr/herdr.sock"
default = true

[[transports]]
name = "mini"
mode = "remote"
remote = "tmeade@timothys-mac-mini-1"

[voice.tts]
# TTS engine backend: "sherpa-kokoro" (neural), "system" (macOS say), "kokoro", or "sine"
engine = "sherpa-kokoro"

# Voice preset:
# For sherpa-kokoro / kokoro: "af_bella", "af_sarah", "af_nicole", "am_adam", "bf_emma", "bm_george", etc.
voice = "am_adam"

# Playback speed multiplier (1.5x speed)
speed = 1.5

# Custom directory holding sherpa-onnx Kokoro assets (optional)
# sherpa_model_dir = "~/.handlr/models/sherpa-kokoro"

[voice.stt]
# STT engine: "whisper"
engine = "whisper"

# Custom model paths (optional)
# silero_model = "~/.handlr/models/silero_vad.onnx"
# whisper_model = "~/.handlr/models/ggml-base.en.bin"

[[agents]]
name = "claude"
allowed_commands = ["continue", "/compact"]
require_confirmation = true

[[agents]]
name = "omp"
allowed_commands = ["continue"]
require_confirmation = true

[[agents]]
name = "agy"
allowed_commands = ["continue"]
require_confirmation = true

[gemini]
# Optional Gemini API Key (defaults to GEMINI_API_KEY environment variable)
# api_key = "..."
# model = "gemini-2.5-flash"
"#.to_string()
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

[voice.tts]
engine = "sherpa-kokoro"
voice = "af_bella"
speed = 1.1

[voice.stt]
engine = "whisper"

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

        assert_eq!(cfg.voice.tts.engine, "sherpa-kokoro");
        assert_eq!(cfg.voice.tts.voice.as_deref(), Some("af_bella"));
        assert_eq!(cfg.voice.tts.speed, 1.1);
        assert_eq!(cfg.voice.stt.engine, "whisper");

        let claude_cfg = cfg.find_agent("Claude").unwrap();
        assert_eq!(claude_cfg.name, "claude");
        assert_eq!(claude_cfg.allowed_commands, vec!["continue", "/compact"]);
        assert!(claude_cfg.require_confirmation);
    }

    #[test]
    fn test_default_toml_template_valid() {
        let template = AppConfig::default_toml_template();
        let cfg: Result<AppConfig, _> = toml::from_str(&template);
        assert!(cfg.is_ok(), "Template should be valid TOML: {:?}", cfg.err());
        let parsed = cfg.unwrap();
        assert_eq!(parsed.transports.len(), 2);
        assert_eq!(parsed.voice.tts.engine, "sherpa-kokoro");
        assert_eq!(parsed.voice.tts.voice.as_deref(), Some("am_adam"));
        assert_eq!(parsed.voice.tts.speed, 1.5);
        assert_eq!(parsed.agents.len(), 3);
        let claude = parsed.find_agent("claude").unwrap();
        assert_eq!(claude.allowed_commands, vec!["continue", "/compact"]);
        assert!(claude.require_confirmation);
    }
}
