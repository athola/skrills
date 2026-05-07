//! Structured error type for the discovery crate.
//!
//! T2.7 migration: replaces the prior `pub type Error = anyhow::Error`
//! alias with a `thiserror`-based enum. Callers that consumed errors
//! via the `Error` type alias keep working — `DiscoveryError`
//! implements `std::error::Error`, converts into `anyhow::Error`
//! automatically, and renders the same `Display` text the old
//! `anyhow!` constructions did.

use std::io;

/// Errors that may arise from filesystem discovery and metadata parsing.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    /// Underlying filesystem failure (read, walk, hash).
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// YAML frontmatter on a SKILL.md or AGENT.md failed to parse.
    #[error("Invalid YAML frontmatter: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
}
