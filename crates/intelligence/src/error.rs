//! Structured error type for the intelligence crate (T2.7 scaffold).
//!
//! Public API note: the crate's existing functions return
//! `anyhow::Result<T>` and that surface is **preserved** to avoid
//! rippling type changes across all callers (server, analyze, the
//! cold-window engine). [`IntelligenceError`] is exposed alongside
//! `anyhow::Error` for callers that want to match on specific
//! failure modes — e.g. retry on `GitHubApi { status: 502 .. }`.
//!
//! Internal call sites currently use `anyhow::bail!` for ergonomics.
//! Migration of those sites to construct [`IntelligenceError`]
//! variants is follow-up work; the conversion is automatic via the
//! `?` operator because `IntelligenceError` implements
//! `std::error::Error` and `anyhow::Error: From<E>`.

use std::path::PathBuf;

/// Errors that may arise from the intelligence crate.
///
/// Variants enumerate the *user-meaningful* failure modes — the
/// situations a caller might want to handle differently (retry,
/// fall back, surface to user) rather than just propagate.
#[derive(Debug, thiserror::Error)]
pub enum IntelligenceError {
    /// GitHub Search API returned a non-2xx response.
    ///
    /// Callers may want to retry on 5xx, surface on 4xx, or ignore
    /// rate-limit responses (HTTP 403/429) gracefully.
    #[error("GitHub API error (HTTP {status}): {message}")]
    GitHubApi {
        /// HTTP status code returned by the GitHub API.
        status: u16,
        /// Body text or parsed error message from the response.
        message: String,
    },

    /// Fetching skill content from a raw URL failed.
    #[error("Failed to fetch skill content: HTTP {status}")]
    FetchFailed {
        /// HTTP status code returned by the upstream server.
        status: u16,
    },

    /// Could not determine the user's home directory.
    ///
    /// Surfaces when neither `$HOME` (Unix) nor `%USERPROFILE%`
    /// (Windows) is set — typically only in stripped CI environments.
    #[error("Cannot determine home directory")]
    HomeDirectoryNotFound,

    /// No README could be located at the project root.
    #[error("No README found in {path}")]
    ReadmeNotFound {
        /// Project root that was searched.
        path: PathBuf,
    },

    /// `git log` failed during commit-keyword extraction.
    #[error("git log failed: {0}")]
    GitLogFailed(String),
}
