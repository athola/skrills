//! Handler for the `multi-cli-agent` command.
//!
//! Routes agent execution across available CLI backends (Claude, Codex)
//! with automatic fallback when the primary backend is unavailable.

use super::agent::{run_with_codex, AgentPath};
use crate::cli::AgentBackend;
use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use skrills_server::discovery::{collect_agents, merge_extra_dirs, resolve_agent};
use std::path::PathBuf;
use std::process::Command;

/// Whether `bin` names an executable file in one of the `$PATH` directories.
///
/// Searched in-process rather than by spawning `which`, which is not installed
/// everywhere and turned its own absence into "backend unavailable".
pub(crate) fn is_available(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable(&dir.join(bin)))
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file() || path.with_extension("exe").is_file()
}

/// Find the first available binary from a list of candidates.
fn find_binary<'a>(candidates: &'a [&'a str]) -> Option<&'a str> {
    candidates.iter().copied().find(|bin| is_available(bin))
}

/// Launch an agent via the Claude CLI.
fn run_with_claude(bin: &str, agent_path: &AgentPath) -> Result<()> {
    let status = Command::new(bin)
        .args(["--print", &agent_path.prompt()])
        .status()
        .map_err(|e| anyhow!("could not run {bin}: {e}"))?;
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

/// Picks the backend to run.
///
/// `--backend auto` takes the first available backend in priority order. An
/// explicit `--backend` is honoured or refused: falling back from `claude`,
/// chosen for its permission prompts, to an approval-free `codex --yolo` run
/// is not a substitution to make behind a warning.
fn select_backend<'a>(
    requested: AgentBackend,
    backends: &'a IndexMap<AgentBackend, &'a [&'a str]>,
    find: impl Fn(&'a [&'a str]) -> Option<&'a str>,
) -> Result<(AgentBackend, &'a str)> {
    if !matches!(requested, AgentBackend::Auto) {
        let candidates = backends.get(&requested).copied().unwrap_or(&[]);
        return find(candidates).map(|bin| (requested, bin)).ok_or_else(|| {
            anyhow!(
                "requested backend '{}' is not available (looked for: {}); \
                     install it or pass --backend auto",
                requested.as_str(),
                candidates.join(", ")
            )
        });
    }
    backends
        .iter()
        .find_map(|(kind, candidates)| find(candidates).map(|bin| (*kind, bin)))
        .ok_or_else(|| {
            anyhow!(
                "no CLI backend available (tried: {})",
                backends
                    .keys()
                    .map(|b| b.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// Run an agent on the requested CLI backend, or the first available one
/// under `--backend auto`.
pub(crate) fn handle_multi_cli_agent_command(
    agent_spec: String,
    backend: AgentBackend,
    skill_dirs: Vec<PathBuf>,
    dry_run: bool,
) -> Result<()> {
    let agents = collect_agents(&merge_extra_dirs(&skill_dirs))?;
    let agent = resolve_agent(&agent_spec, &agents)?;
    let agent_path = AgentPath::new(&agent.path)?;

    println!(
        "Agent: {} (source: {}, path: {})",
        agent.name,
        agent.source.label(),
        agent_path.as_str()
    );

    let backends = backend.backends();
    let (backend_kind, bin) = select_backend(backend, &backends, find_binary)?;

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
        // Both "sh" and "bash" should exist, first one wins
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

    fn only<'a>(present: &'a [&'a str]) -> impl Fn(&'a [&'a str]) -> Option<&'a str> {
        move |candidates: &'a [&'a str]| candidates.iter().copied().find(|c| present.contains(c))
    }

    /// SA-48: `--backend claude` with Claude missing ran `codex --yolo exec`
    /// after a warning.
    #[test]
    fn an_explicit_backend_that_is_missing_is_refused_not_replaced() {
        let backends = AgentBackend::Claude.backends();

        let err = select_backend(AgentBackend::Claude, &backends, only(&["codex"]))
            .unwrap_err()
            .to_string();

        assert!(err.contains("'claude' is not available"), "{err}");
    }

    #[test]
    fn an_explicit_backend_that_is_present_is_used() {
        let backends = AgentBackend::Codex.backends();
        let (kind, bin) =
            select_backend(AgentBackend::Codex, &backends, only(&["claude", "codex"])).unwrap();
        assert_eq!(kind, AgentBackend::Codex);
        assert_eq!(bin, "codex");
    }

    #[test]
    fn auto_falls_back_to_the_next_available_backend() {
        let backends = AgentBackend::Auto.backends();
        let (kind, _) = select_backend(AgentBackend::Auto, &backends, only(&["codex"])).unwrap();
        assert_eq!(kind, AgentBackend::Codex);
    }

    #[test]
    fn auto_with_nothing_installed_is_an_error() {
        let backends = AgentBackend::Auto.backends();
        assert!(select_backend(AgentBackend::Auto, &backends, only(&[])).is_err());
    }
}
