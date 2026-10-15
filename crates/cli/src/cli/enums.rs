//! Supporting CLI enums used by clap subcommands.
//!
//! Split out of `cli/mod.rs`. These
//! enums are public because clap derives `ValueEnum` for parsing
//! `--target=claude` style arguments. They re-export from the
//! parent module so external callers continue to use the existing
//! `crate::cli::OutputFormat` paths unchanged.

use clap::{Subcommand, ValueEnum};
use indexmap::IndexMap;
use std::path::PathBuf;

/// Platforms for sync commands; the CLI parses it through the `clap`
/// feature of skrills_sync.
pub use skrills_sync::Platform;
/// Validation target for skills; parsed through the `clap` feature of
/// skrills-validate.
pub use skrills_validate::ValidationTarget;

/// Dependency traversal direction.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DependencyDirection {
    /// Resolve dependencies (what this skill needs).
    Dependencies,
    /// Resolve dependents (what uses this skill).
    Dependents,
}

/// Backend for multi-CLI agent routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum, Default)]
pub enum AgentBackend {
    /// Auto-detect the best available backend.
    #[default]
    Auto,
    /// Use Claude Code CLI.
    Claude,
    /// Use Codex CLI.
    Codex,
}

/// CLI binary names to probe for availability.
const CLAUDE_BINS: &[&str] = &["claude"];
const CODEX_BINS: &[&str] = &["codex"];

impl AgentBackend {
    /// Return a human-readable name for the backend.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentBackend::Auto => "auto",
            AgentBackend::Claude => "claude",
            AgentBackend::Codex => "codex",
        }
    }

    /// Return an ordered map of backends to try, with this preference first.
    ///
    /// Iteration order encodes fallback priority.
    pub fn backends(self) -> IndexMap<AgentBackend, &'static [&'static str]> {
        match self {
            AgentBackend::Claude | AgentBackend::Auto => IndexMap::from([
                (AgentBackend::Claude, CLAUDE_BINS),
                (AgentBackend::Codex, CODEX_BINS),
            ]),
            AgentBackend::Codex => IndexMap::from([
                (AgentBackend::Codex, CODEX_BINS),
                (AgentBackend::Claude, CLAUDE_BINS),
            ]),
        }
    }
}

/// Creation method for new skills.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CreateSkillMethod {
    /// Search GitHub for existing skills.
    Github,
    /// Generate skill content via LLM.
    Llm,
    /// Use both GitHub search and LLM generation.
    Both,
    /// Generate from empirical session patterns.
    Empirical,
}

impl CreateSkillMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Llm => "llm",
            Self::Both => "both",
            Self::Empirical => "empirical",
        }
    }
}

impl std::fmt::Display for CreateSkillMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Certificate management action.
#[derive(Debug, Clone, Subcommand)]
pub enum CertAction {
    /// Show certificate status and expiry information.
    Status {
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Renew or regenerate self-signed certificate.
    Renew {
        /// Force renewal even if not expiring.
        #[arg(long)]
        force: bool,
    },
    /// Install a certificate from external source.
    Install {
        /// Path to certificate file (PEM format).
        cert: PathBuf,
        /// Path to private key file (PEM format).
        #[arg(long)]
        key: Option<PathBuf>,
        /// Output format: text or json.
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

/// Output format for command results.
#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq)]
pub enum OutputFormat {
    /// Human-readable text output.
    #[default]
    Text,
    /// JSON output for machine parsing.
    Json,
}

impl OutputFormat {
    /// Check if this format is JSON.
    pub fn is_json(&self) -> bool {
        matches!(self, Self::Json)
    }
}
