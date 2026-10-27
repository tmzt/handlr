//! The `gemini` backend kind's defaults (plan 20261027_02, decision 2):
//! a Flash model for the general agent, Flash-Lite for the summaries,
//! which run often, and never a Pro model unless a config names one.

use super::Use;

/// The general agent's model: Flash.
pub const GENERAL_AGENT_MODEL: &str = "gemini-3.8-flash";
/// Card and spoken summaries' model: Flash-Lite.
pub const SUMMARY_MODEL: &str = "gemini-3.8-flash-lite";
/// The discovered backend's key file, against the daemon's working folder
/// (`.private` is gitignored).
pub const KEY_FILE: &str = ".private/gemini_api_key";
/// The environment variable read when the key file is not there.
pub const KEY_ENV: &str = "GEMINI_API_KEY";

/// The tier default for `use_`.
pub fn tier_model(use_: Use) -> &'static str {
    match use_ {
        Use::GeneralAgent => GENERAL_AGENT_MODEL,
        Use::CardSummaries | Use::SpokenSummaries => SUMMARY_MODEL,
    }
}
