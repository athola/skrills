//! Handler for the `multi-cli-agent` command.
//!
//! Routes agent execution across available CLI backends (Claude, Codex)
//! with automatic fallback when the primary backend is unavailable.

use crate::cli::AgentBackend;
use crate::discovery::{collect_agents, merge_extra_dirs, resolve_agent};
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;

/// A validated agent path that is safe for embedding in LLM prompts.
///
/// Rejects characters that could enable prompt injection when the path
/// is interpolated into a prompt string.
struct AgentPath(String);

impl AgentPath {
    /// Create a new `AgentPath`, validating that it contains no prompt-injection characters.
    fn new(path: String) -> Result<Self> {
        if path
            .chars()
            .any(|c| matches!(c, '\n' | '\r' | '\0' | '`' | '$' | '{' | '}'))
        {
            return Err(anyhow!("agent path contains invalid characters: {}", path));
        }
        Ok(Self(path))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Check whether a CLI binary is available on `$PATH`.
fn is_available(bin: &str) -> bool {
    match Command::new("which")
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(s) => s.success(),
        Err(e) => {
            eprintln!("warning: failed to run `which {bin}`: {e}");
            false
        }
    }
}

/// Find the first available binary from a list of candidates.
fn find_binary<'a>(candidates: &'a [&'a str]) -> Option<&'a str> {
    candidates.iter().copied().find(|bin| is_available(bin))
}

/// Launch an agent via the Claude CLI.
fn run_with_claude(bin: &str, agent_path: &AgentPath) -> Result<()> {
    let prompt = format!(
        "Load agent spec at {} and execute its instructions",
        agent_path.as_str()
    );
    let status = Command::new(bin).args(["--print", &prompt]).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "claude agent exited with {}",
            status
                .code()
                .map(|c| format!("code {c}"))
                .unwrap_or_else(|| "signal (killed)".to_string())
        ))
    }
}

/// Launch an agent via the Codex CLI.
fn run_with_codex(bin: &str, agent_path: &AgentPath) -> Result<()> {
    let prompt = format!(
        "Load agent spec at {} and execute its instructions",
        agent_path.as_str()
    );
    let status = Command::new(bin)
        .args(["--yolo", "exec", "--timeout_ms", "1800000"])
        .arg(&prompt)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "codex agent exited with {}",
            status
                .code()
                .map(|c| format!("code {c}"))
                .unwrap_or_else(|| "signal (killed)".to_string())
        ))
    }
}

/// Run an agent across available CLI backends with automatic fallback.
///
/// Resolves the agent spec, probes for available backends in priority order
/// (determined by the `backend` preference), and dispatches execution.
/// When an explicit backend is requested but unavailable, a warning is emitted
/// before falling back to the next available backend.
pub(crate) fn handle_multi_cli_agent_command(
    agent_spec: String,
    backend: AgentBackend,
    skill_dirs: Vec<PathBuf>,
    dry_run: bool,
) -> Result<()> {
    let agents = collect_agents(&merge_extra_dirs(&skill_dirs))?;
    let agent = resolve_agent(&agent_spec, &agents)?;
    let agent_path = AgentPath::new(agent.path.display().to_string())?;

    println!(
        "Agent: {} (source: {}, path: {})",
        agent.name,
        agent.source.label(),
        agent_path.as_str()
    );

    let backends = backend.backends();
    let preferred = *backends.keys().next().unwrap();

    // Find the first available backend
    let mut selected: Option<(AgentBackend, &str)> = None;
    for (backend_kind, candidates) in &backends {
        if let Some(bin) = find_binary(candidates) {
            selected = Some((*backend_kind, bin));
            break;
        }
    }

    let (backend_kind, bin) = selected.ok_or_else(|| {
        anyhow!(
            "no CLI backend available (tried: {})",
            backends
                .iter()
                .map(|(b, _)| b.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    // Warn when an explicit backend preference was overridden by fallback
    if !matches!(backend, AgentBackend::Auto) && backend_kind != preferred {
        eprintln!(
            "warning: requested backend '{}' is not available, falling back to '{}'",
            preferred.as_str(),
            backend_kind.as_str()
        );
    }

    let backend_label = backend_kind.as_str();
    println!("Backend: {backend_label} ({bin})");

    if dry_run {
        println!("Agent path: {}", agent_path.as_str());
        println!("Would run with: {backend_label}");
        return Ok(());
    }

    match backend_kind {
        AgentBackend::Claude => run_with_claude(bin, &agent_path),
        AgentBackend::Codex => run_with_codex(bin, &agent_path),
        AgentBackend::Auto => unreachable!("resolve_backends never returns Auto as a backend"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_backends_claude_prefers_claude_first() {
        let backends = AgentBackend::Claude.backends();
        let keys: Vec<_> = backends.keys().collect();
        assert_eq!(keys, vec![&AgentBackend::Claude, &AgentBackend::Codex]);
    }

    #[test]
    fn resolve_backends_codex_prefers_codex_first() {
        let backends = AgentBackend::Codex.backends();
        let keys: Vec<_> = backends.keys().collect();
        assert_eq!(keys, vec![&AgentBackend::Codex, &AgentBackend::Claude]);
    }

    #[test]
    fn resolve_backends_auto_defaults_to_claude_first() {
        let backends = AgentBackend::Auto.backends();
        let keys: Vec<_> = backends.keys().collect();
        assert_eq!(keys, vec![&AgentBackend::Claude, &AgentBackend::Codex]);
    }

    #[test]
    fn resolve_backends_always_returns_two_entries() {
        for variant in [
            AgentBackend::Auto,
            AgentBackend::Claude,
            AgentBackend::Codex,
        ] {
            let backends = variant.backends();
            assert_eq!(backends.len(), 2, "should always have two backend entries");
        }
    }

    #[test]
    fn find_binary_returns_none_for_nonexistent() {
        let result = find_binary(&["absolutely-nonexistent-binary-12345"]);
        assert!(result.is_none());
    }

    #[test]
    fn find_binary_returns_first_available() {
        // "sh" should exist on any Unix system
        let result = find_binary(&["absolutely-nonexistent-binary-12345", "sh"]);
        assert_eq!(result, Some("sh"));
    }

    #[test]
    fn is_available_returns_false_for_nonexistent() {
        assert!(!is_available("absolutely-nonexistent-binary-12345"));
    }

    #[test]
    fn is_available_returns_true_for_sh() {
        assert!(is_available("sh"));
    }

    #[test]
    fn find_binary_returns_none_for_empty_candidates() {
        let result = find_binary(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn find_binary_returns_first_when_all_available() {
        // Both "sh" and "bash" should exist — first one wins
        let result = find_binary(&["sh", "bash"]);
        assert_eq!(result, Some("sh"));
    }

    #[test]
    fn resolve_backends_each_entry_has_nonempty_candidates() {
        for variant in [
            AgentBackend::Auto,
            AgentBackend::Claude,
            AgentBackend::Codex,
        ] {
            for (backend_kind, candidates) in &variant.backends() {
                assert!(
                    !candidates.is_empty(),
                    "backend '{}' should have at least one candidate binary",
                    backend_kind.as_str()
                );
            }
        }
    }

    #[test]
    fn agent_backend_default_is_auto() {
        assert!(matches!(AgentBackend::default(), AgentBackend::Auto));
    }
}
