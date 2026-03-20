//! Handler for the `multi-cli-agent` command.
//!
//! Routes agent execution across available CLI backends (Claude, Codex)
//! with automatic fallback when the primary backend is unavailable.

use crate::cli::AgentBackend;
use crate::discovery::{collect_agents, merge_extra_dirs, resolve_agent};
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;

/// CLI binary names to probe for availability.
const CLAUDE_BINS: &[&str] = &["claude"];
const CODEX_BINS: &[&str] = &["codex"];

/// Check whether a CLI binary is available on `$PATH`.
fn is_available(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Find the first available binary from a list of candidates.
fn find_binary<'a>(candidates: &'a [&'a str]) -> Option<&'a str> {
    candidates.iter().copied().find(|bin| is_available(bin))
}

/// Determine the ordered list of backends to try based on the user's preference.
fn resolve_backends(preference: AgentBackend) -> Vec<(&'static str, &'static [&'static str])> {
    match preference {
        AgentBackend::Claude => vec![("claude", CLAUDE_BINS), ("codex", CODEX_BINS)],
        AgentBackend::Codex => vec![("codex", CODEX_BINS), ("claude", CLAUDE_BINS)],
        AgentBackend::Auto => {
            // Prefer whichever is available, Claude first
            vec![("claude", CLAUDE_BINS), ("codex", CODEX_BINS)]
        }
    }
}

/// Launch an agent via the Claude CLI.
fn run_with_claude(bin: &str, agent_path: &str) -> Result<()> {
    let prompt = format!(
        "Load agent spec at {} and execute its instructions",
        agent_path
    );
    let status = Command::new(bin).args(["--print", &prompt]).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "claude agent exited with status {:?}",
            status.code()
        ))
    }
}

/// Launch an agent via the Codex CLI.
fn run_with_codex(bin: &str, agent_path: &str) -> Result<()> {
    let prompt = format!(
        "Load agent spec at {} and execute its instructions",
        agent_path
    );
    let status = Command::new(bin)
        .args(["--yolo", "exec", "--timeout_ms", "1800000"])
        .arg(&prompt)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "codex agent exited with status {:?}",
            status.code()
        ))
    }
}

pub(crate) fn handle_multi_cli_agent_command(
    agent_spec: String,
    backend: AgentBackend,
    skill_dirs: Vec<PathBuf>,
    dry_run: bool,
) -> Result<()> {
    let agents = collect_agents(&merge_extra_dirs(&skill_dirs))?;
    let agent = resolve_agent(&agent_spec, &agents)?;
    let agent_path = agent.path.display().to_string();

    println!(
        "Agent: {} (source: {}, path: {})",
        agent.name,
        agent.source.label(),
        agent.path.display()
    );

    let backends = resolve_backends(backend);

    // Find the first available backend
    let mut selected = None;
    for (name, candidates) in &backends {
        if let Some(bin) = find_binary(candidates) {
            selected = Some((*name, bin));
            break;
        }
    }

    let (backend_name, bin) = selected.ok_or_else(|| {
        anyhow!(
            "no CLI backend available (tried: {})",
            backends
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    println!("Backend: {backend_name} ({bin})");

    if dry_run {
        println!("Agent path: {agent_path}");
        println!("Would run with: {backend_name}");
        return Ok(());
    }

    match backend_name {
        "claude" => run_with_claude(bin, &agent_path),
        "codex" => run_with_codex(bin, &agent_path),
        _ => Err(anyhow!("unknown backend: {backend_name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_backends_claude_prefers_claude_first() {
        let backends = resolve_backends(AgentBackend::Claude);
        assert_eq!(backends[0].0, "claude");
        assert_eq!(backends[1].0, "codex");
    }

    #[test]
    fn resolve_backends_codex_prefers_codex_first() {
        let backends = resolve_backends(AgentBackend::Codex);
        assert_eq!(backends[0].0, "codex");
        assert_eq!(backends[1].0, "claude");
    }

    #[test]
    fn resolve_backends_auto_defaults_to_claude_first() {
        let backends = resolve_backends(AgentBackend::Auto);
        assert_eq!(backends[0].0, "claude");
        assert_eq!(backends[1].0, "codex");
    }

    #[test]
    fn resolve_backends_always_returns_two_entries() {
        for variant in [
            AgentBackend::Auto,
            AgentBackend::Claude,
            AgentBackend::Codex,
        ] {
            let backends = resolve_backends(variant);
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
}
