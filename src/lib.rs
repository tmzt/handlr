//! `handlr` — Voice-native CLI and LLM tool-calling bridge for `herdr`.

pub mod commands;
pub mod config;
pub mod reasoning;
pub mod transport;

pub use commands::validate_arbitrary_command;
pub use config::AppConfig;
pub use reasoning::{
    declare_gemini_tools, default_system_prompt, GeminiReasoningEngine, MockReasoningEngine,
    ReasoningEngine, ToolExecutor,
};
pub use transport::HerdrClient;
