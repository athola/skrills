//! Detect which CLI environment is currently active.

use std::env;

/// Detected CLI environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CliEnvironment {
    /// Running under Claude Code CLI.
    ClaudeCode,
    /// Running under Codex CLI.
    CodexCli,
    /// Unknown environment.
    #[default]
    Unknown,
}

impl std::fmt::Display for CliEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeCode => write!(f, "claude"),
            Self::CodexCli => write!(f, "codex"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Detect which CLI environment is currently active.
pub fn detect_cli_environment() -> CliEnvironment {
    // Honor explicit skrills client selection first.
    if let Ok(client) = env::var("SKRILLS_CLIENT") {
        if client.eq_ignore_ascii_case("claude") {
            return CliEnvironment::ClaudeCode;
        }
        if client.eq_ignore_ascii_case("codex") {
            return CliEnvironment::CodexCli;
        }
    }

    // Check Claude Code environment variables
    if env::var("CLAUDE_CODE_SESSION").is_ok()
        || env::var("CLAUDE_CLI").is_ok()
        || env::var("__CLAUDE_MCP_SERVER").is_ok()
        || env::var("CLAUDE_CODE_ENTRYPOINT").is_ok()
    {
        return CliEnvironment::ClaudeCode;
    }

    // Check Codex CLI environment variables
    if env::var("CODEX_CLI").is_ok()
        || env::var("CODEX_SESSION_ID").is_ok()
        || env::var("CODEX_HOME").is_ok()
    {
        return CliEnvironment::CodexCli;
    }

    // Check skrills configuration (low priority; API backend may differ from CLI)
    if let Ok(backend) = env::var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND") {
        match backend.to_lowercase().as_str() {
            "claude" => return CliEnvironment::ClaudeCode,
            "codex" => return CliEnvironment::CodexCli,
            _ => {}
        }
    }

    // Try to detect from parent process (Linux)
    #[cfg(target_os = "linux")]
    {
        if let Ok(cmdline) = std::fs::read_to_string("/proc/self/cmdline") {
            let cmdline_lower = cmdline.to_lowercase();
            if cmdline_lower.contains("claude") {
                return CliEnvironment::ClaudeCode;
            }
            if cmdline_lower.contains("codex") {
                return CliEnvironment::CodexCli;
            }
        }
    }

    CliEnvironment::Unknown
}

/// Get the appropriate CLI binary for the environment.
pub fn get_cli_binary(env: CliEnvironment) -> &'static str {
    match env {
        CliEnvironment::ClaudeCode => "claude",
        CliEnvironment::CodexCli => "codex",
        CliEnvironment::Unknown => "claude", // Default to claude
    }
}

/// Check if a CLI binary is available in PATH.
#[allow(dead_code)]
pub fn is_cli_available(binary: &str) -> bool {
    std::process::Command::new("which")
        .arg(binary)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Get the best available CLI binary.
#[allow(dead_code)]
pub fn get_available_cli() -> Option<&'static str> {
    let env = detect_cli_environment();
    let preferred = get_cli_binary(env);

    if is_cli_available(preferred) {
        return Some(preferred);
    }

    // Try alternatives
    if is_cli_available("claude") {
        return Some("claude");
    }
    if is_cli_available("codex") {
        return Some("codex");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_environment() {
        // In test environment, should be Unknown
        // unless specific env vars are set
        let env = detect_cli_environment();
        // Just ensure it doesn't panic
        let _ = env.to_string();
    }

    #[test]
    fn test_get_cli_binary() {
        assert_eq!(get_cli_binary(CliEnvironment::ClaudeCode), "claude");
        assert_eq!(get_cli_binary(CliEnvironment::CodexCli), "codex");
        assert_eq!(get_cli_binary(CliEnvironment::Unknown), "claude");
    }
}
