//! Error types for `libherdr`.

use thiserror::Error;

/// Result alias for `libherdr` operations.
pub type HerdrResult<T> = Result<T, HerdrError>;

/// Represents errors that can occur within the `libherdr` client and transport.
#[derive(Debug, Error)]
pub enum HerdrError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Unix socket error: {0}")]
    Socket(String),

    #[error("SSH transport error: {0}")]
    Ssh(String),

    #[error("Invalid tool arguments for '{tool}': {details}")]
    InvalidArguments { tool: String, details: String },

    #[error("Unknown tool: '{0}'")]
    UnknownTool(String),

    #[error("Server returned error: {0}")]
    ServerError(String),

    #[error("Transport execution failed: {0}")]
    Transport(String),

    #[error("Timeout while waiting for response")]
    Timeout,
}
