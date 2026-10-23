//! `handlr` — Voice-native CLI and LLM tool-calling bridge for `herdr`.

pub mod commands;
pub mod config;
pub mod core;
pub mod ctl;
pub mod notify;
pub mod pair;
pub mod reasoning;
pub mod server;
pub mod sessions;
pub mod transport;
pub mod watch;

pub use commands::{
    ConfirmationRequest, confirmation_match_candidates, normalize_command_for_match,
    parse_confirmation_request, validate_arbitrary_command, validate_message,
};
pub use config::AppConfig;
pub use reasoning::{
    GeminiReasoningEngine, MockReasoningEngine, ReasoningEngine, ToolExecutor,
    declare_gemini_tools, default_system_prompt,
};
pub use transport::HerdrClient;
pub use watch::WatchRegistry;
