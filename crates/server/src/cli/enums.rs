//! Supporting CLI enums used by clap subcommands.
//!
//! Split out of `cli/mod.rs` (FU-4 of v0.8.0 refinement). These
//! enums are public because clap derives `ValueEnum` for parsing
//! `--target=claude` style arguments. They re-export from the
//! parent module so external callers continue to use the existing
//! `crate::cli::OutputFormat` paths unchanged.

use clap::{Subcommand, ValueEnum};
use indexmap::IndexMap;
use std::path::PathBuf;

/// Validation target for skills.
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum ValidationTarget {
    /// Validate for Claude Code (permissive).
    Claude,
    /// Validate for Codex CLI (strict).
    Codex,
    /// Validate for GitHub Copilot CLI (strict).
    Copilot,
    /// Validate for all targets (Claude, Codex, and Copilot).
    #[default]
    All,
    /// Validate for Claude and Codex (legacy, use 'all' for new code).
    Both,
}

/// Source/target for sync operations.
#[derive(Debug, Clone, Copy, ValueEnum, Default, PartialEq, Eq)]
pub enum SyncSource {
    /// Claude Code CLI.
    #[default]
    Claude,
    /// OpenAI Codex CLI.
    Codex,
    /// GitHub Copilot CLI.
    Copilot,
    /// Cursor IDE.
    Cursor,
}

impl SyncSource {
    /// Returns the default target for a given source.
    /// Claude → Codex, Codex/Copilot/Cursor → Claude.
    pub fn default_target(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex | Self::Copilot | Self::Cursor => Self::Claude,
        }
    }

    /// Returns true if this source is Claude.
    pub fn is_claude(self) -> bool {
        matches!(self, Self::Claude)
    }

    /// Returns true if this source is Codex.
    pub fn is_codex(self) -> bool {
        matches!(self, Self::Codex)
    }

    /// Returns true if this source is Copilot.
    pub fn is_copilot(self) -> bool {
        matches!(self, Self::Copilot)
    }

    /// Returns true if this source is Cursor.
    pub fn is_cursor(self) -> bool {
        matches!(self, Self::Cursor)
    }

    /// Returns the string name for this source.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
        }
    }

    /// Returns all other targets (CLIs other than this one).
    /// Used when `--to` is not specified to sync to all other CLIs.
    pub fn other_targets(self) -> Vec<Self> {
        match self {
            Self::Claude => vec![Self::Codex, Self::Copilot, Self::Cursor],
            Self::Codex => vec![Self::Claude, Self::Copilot, Self::Cursor],
            Self::Copilot => vec![Self::Claude, Self::Codex, Self::Cursor],
            Self::Cursor => vec![Self::Claude, Self::Codex, Self::Copilot],
        }
    }
}

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
