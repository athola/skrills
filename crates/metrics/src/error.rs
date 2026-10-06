//! Error types for metrics operations.

use thiserror::Error;

/// Errors that can occur during metrics operations.
#[derive(Error, Debug)]
pub enum MetricsError {
    /// Database error from rusqlite.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// JSON serialization/deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// A stored validation run holds a column that is not valid JSON.
    ///
    /// Carries the row's identity because the only repair is to find and fix
    /// that row, and a bare decode error names neither the skill nor the column.
    #[error("validation run for skill {skill_name}: column {column} is not valid JSON: {source}")]
    CorruptValidationRow {
        /// Skill the unreadable run belongs to.
        skill_name: String,
        /// Column that failed to decode (`checks_passed` or `checks_failed`).
        column: &'static str,
        /// Underlying decode failure.
        source: serde_json::Error,
    },

    /// IO error (e.g., creating directories).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Home directory not found.
    #[error("home directory not found")]
    HomeNotFound,

    /// Channel send error.
    #[error("channel send error")]
    ChannelSend,

    /// Invalid argument.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

/// Result type for metrics operations.
pub type Result<T> = std::result::Result<T, MetricsError>;
