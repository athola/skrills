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

    /// IO error (e.g., creating directories).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Home directory not found.
    #[error("home directory not found")]
    HomeNotFound,

    /// Channel send error.
    #[error("channel send error")]
    ChannelSend,

    /// Mutex poisoned.
    #[error("Mutex poisoned")]
    MutexPoisoned,
}

/// Result type for metrics operations.
pub type Result<T> = std::result::Result<T, MetricsError>;
