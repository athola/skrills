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

    // Try to detect from the parent process (Linux)
    #[cfg(target_os = "linux")]
    {
        if let Some(cmdline) = parent_cmdline() {
            return classify_parent_cmdline(&cmdline);
        }
    }

    CliEnvironment::Unknown
}

/// The parent process's raw `/proc/<ppid>/cmdline` (NUL-separated argv).
#[cfg(target_os = "linux")]
fn parent_cmdline() -> Option<String> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let ppid = status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:"))?
        .trim();
    std::fs::read_to_string(format!("/proc/{ppid}/cmdline")).ok()
}

/// Classify a parent process by the basename of its argv[0].
///
/// Substring matching on our own command line misfired: `skrills sync --from
/// codex` read as Codex, and any binary under `/home/claude/` read as Claude.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn classify_parent_cmdline(cmdline: &str) -> CliEnvironment {
    let argv0 = cmdline.split('\0').next().unwrap_or_default();
    let name = std::path::Path::new(argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name.eq_ignore_ascii_case("claude") {
        CliEnvironment::ClaudeCode
    } else if name.eq_ignore_ascii_case("codex") {
        CliEnvironment::CodexCli
    } else {
        CliEnvironment::Unknown
    }
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
pub fn is_cli_available(binary: &str) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("which")
            .arg(binary)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(binary)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// Get the best available CLI binary.
pub fn get_available_cli() -> Option<&'static str> {
    pick_available_cli(get_cli_binary(detect_cli_environment()), is_cli_available)
}

/// The preferred binary when available, else the first available alternative.
fn pick_available_cli(
    preferred: &'static str,
    is_available: impl Fn(&str) -> bool,
) -> Option<&'static str> {
    [preferred, "claude", "codex"]
        .into_iter()
        .find(|binary| is_available(binary))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_support::{env_guard, set_env_var};

    #[test]
    fn test_default_environment() {
        let _g = env_guard();
        let _client = set_env_var("SKRILLS_CLIENT", Some("codex"));
        assert_eq!(detect_cli_environment(), CliEnvironment::CodexCli);
        assert_eq!(detect_cli_environment().to_string(), "codex");

        let _client = set_env_var("SKRILLS_CLIENT", Some("CLAUDE"));
        assert_eq!(detect_cli_environment(), CliEnvironment::ClaudeCode);
    }

    /// IN-24: only argv[0]'s basename decides; arguments and directories that
    /// merely contain the words do not.
    #[test]
    fn classify_parent_cmdline_uses_argv0_basename() {
        assert_eq!(
            classify_parent_cmdline("/usr/bin/skrills\0sync\0--from\0codex\0"),
            CliEnvironment::Unknown
        );
        assert_eq!(
            classify_parent_cmdline("/home/claude/bin/skrills\0serve\0"),
            CliEnvironment::Unknown
        );
        assert_eq!(
            classify_parent_cmdline("/usr/local/bin/claude\0--resume\0"),
            CliEnvironment::ClaudeCode
        );
        assert_eq!(classify_parent_cmdline("codex\0"), CliEnvironment::CodexCli);
        assert_eq!(classify_parent_cmdline(""), CliEnvironment::Unknown);
    }

    #[test]
    fn test_get_cli_binary() {
        assert_eq!(get_cli_binary(CliEnvironment::ClaudeCode), "claude");
        assert_eq!(get_cli_binary(CliEnvironment::CodexCli), "codex");
        assert_eq!(get_cli_binary(CliEnvironment::Unknown), "claude");
    }

    #[test]
    fn test_is_cli_available_common_binary() {
        // Test with a binary that should exist on most systems
        #[cfg(unix)]
        {
            assert!(is_cli_available("ls"));
            assert!(!is_cli_available("nonexistent_binary_12345"));
        }
        #[cfg(windows)]
        {
            assert!(is_cli_available("cmd"));
            assert!(!is_cli_available("nonexistent_binary_12345"));
        }
    }

    #[test]
    fn test_get_available_cli() {
        let only_codex = |b: &str| b == "codex";
        // Preferred binary present.
        assert_eq!(pick_available_cli("codex", only_codex), Some("codex"));
        // Preferred binary missing: falls back to the one that exists.
        assert_eq!(pick_available_cli("claude", only_codex), Some("codex"));
        assert_eq!(
            pick_available_cli("codex", |b| b == "claude"),
            Some("claude")
        );
        // Neither present.
        assert_eq!(pick_available_cli("claude", |_| false), None);
        // The public entry point runs without panicking whatever is installed.
        let _ = get_available_cli();
    }
}
