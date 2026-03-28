//! Error types for the tome crate.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TomeError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Cache error: {0}")]
    Cache(#[from] rusqlite::Error),

    #[error("Rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("API error ({api}): {message}")]
    Api { api: String, message: String },

    #[error("PDF not found for DOI: {doi}")]
    PdfNotFound { doi: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type TomeResult<T> = std::result::Result<T, TomeError>;
