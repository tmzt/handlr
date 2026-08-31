//! `handlr` — Voice-native CLI and LLM tool-calling bridge for `herdr`.

pub mod reasoning;
pub mod transport;

pub use reasoning::{
    declare_gemini_tools, default_system_prompt, GeminiReasoningEngine, MockReasoningEngine,
    ReasoningEngine, ToolExecutor,
};
pub use transport::HerdrClient;
