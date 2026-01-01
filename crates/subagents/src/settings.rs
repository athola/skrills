//! Configuration and settings for subagent service.
//!
//! This module handles:
//! - File-based configuration from `~/.claude/subagents.toml` or `~/.codex/subagents.toml`
//! - Environment variable overrides
//! - Backend and execution mode parsing

use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use serde::Deserialize;

use crate::cli_detection::{client_hint_from_env, client_hint_from_exe_path};
use crate::store::BackendKind;

/// Execution mode for subagent runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    /// Use local headless CLI (default)
    #[default]
    Cli,
    /// Use network APIs
    Api,
}

/// Error returned when parsing an invalid execution mode string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseExecutionModeError(String);

impl std::fmt::Display for ParseExecutionModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid execution mode '{}': expected 'cli', 'headless', or 'api'",
            self.0
        )
    }
}

impl std::error::Error for ParseExecutionModeError {}

impl FromStr for ExecutionMode {
    type Err = ParseExecutionModeError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("cli") || s.eq_ignore_ascii_case("headless") {
            Ok(ExecutionMode::Cli)
        } else if s.eq_ignore_ascii_case("api") {
            Ok(ExecutionMode::Api)
        } else {
            Err(ParseExecutionModeError(s.to_string()))
        }
    }
}

impl ExecutionMode {
    /// Parse execution mode, returning error for invalid values.
    pub fn parse(raw: &str) -> Result<Self> {
        raw.parse()
            .map_err(|e: ParseExecutionModeError| anyhow!("{}", e))
    }
}

/// File-based configuration for subagents.
#[derive(Debug, Default, Deserialize)]
pub struct SubagentsFileConfig {
    #[serde(default)]
    pub default_backend: Option<String>,
    #[serde(default)]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub cli_binary: Option<String>,
}

/// Get paths to check for subagents configuration files.
pub fn config_paths() -> Vec<PathBuf> {
    let Ok(home) = skrills_state::home_dir() else {
        return Vec::new();
    };
    let claude = home.join(".claude/subagents.toml");
    let codex = home.join(".codex/subagents.toml");
    match client_hint_from_env().or_else(client_hint_from_exe_path) {
        Some("codex") => vec![codex, claude],
        Some("claude") => vec![claude, codex],
        _ => vec![claude, codex],
    }
}

/// Load subagents configuration from file.
pub fn load_file_config() -> SubagentsFileConfig {
    for path in config_paths() {
        if !path.exists() {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<SubagentsFileConfig>(&raw) {
                Ok(config) => return config,
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "failed to parse subagents config"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "failed to read subagents config"
                );
            }
        }
    }
    SubagentsFileConfig::default()
}

/// Parse backend kind from string.
pub fn backend_from_str(raw: &str) -> BackendKind {
    if raw.eq_ignore_ascii_case("codex")
        || raw.eq_ignore_ascii_case("gpt")
        || raw.eq_ignore_ascii_case("openai")
    {
        BackendKind::Codex
    } else if raw.eq_ignore_ascii_case("claude") || raw.eq_ignore_ascii_case("anthropic") {
        BackendKind::Claude
    } else {
        BackendKind::Other(raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_mode_parsing() {
        assert_eq!("cli".parse(), Ok(ExecutionMode::Cli));
        assert_eq!("CLI".parse(), Ok(ExecutionMode::Cli));
        assert_eq!("headless".parse(), Ok(ExecutionMode::Cli));
        assert_eq!("api".parse(), Ok(ExecutionMode::Api));
        assert_eq!("API".parse(), Ok(ExecutionMode::Api));
        assert!("invalid".parse::<ExecutionMode>().is_err());
    }

    #[test]
    fn backend_parsing() {
        assert_eq!(backend_from_str("codex"), BackendKind::Codex);
        assert_eq!(backend_from_str("CODEX"), BackendKind::Codex);
        assert_eq!(backend_from_str("gpt"), BackendKind::Codex);
        assert_eq!(backend_from_str("openai"), BackendKind::Codex);
        assert_eq!(backend_from_str("claude"), BackendKind::Claude);
        assert_eq!(backend_from_str("anthropic"), BackendKind::Claude);
        assert_eq!(
            backend_from_str("custom"),
            BackendKind::Other("custom".to_string())
        );
    }
}
