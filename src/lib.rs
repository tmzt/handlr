//! `handlr` — Voice-native CLI and LLM tool-calling bridge for `herdr`.

pub mod commands;
pub mod config;
pub mod core;
pub mod notify;
pub mod reasoning;
pub mod transport;
pub mod watch;

pub use commands::{
    confirmation_match_candidates, normalize_command_for_match, parse_confirmation_request,
    validate_arbitrary_command, ConfirmationRequest,
};
pub use config::AppConfig;
pub use reasoning::{
    declare_gemini_tools, default_system_prompt, GeminiReasoningEngine, MockReasoningEngine,
    ReasoningEngine, ToolExecutor,
};
pub use transport::HerdrClient;
pub use watch::WatchRegistry;
