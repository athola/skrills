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
