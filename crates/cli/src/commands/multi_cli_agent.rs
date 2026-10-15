//! Handler for the `multi-cli-agent` command.
//!
//! Routes agent execution across available CLI backends (Claude, Codex)
//! with automatic fallback when the primary backend is unavailable.

use super::agent::{codex_args, codex_sandbox, run_with_codex, AgentPath};
use crate::cli::AgentBackend;
use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use skrills_discovery::{collect_agents, merge_extra_dirs, resolve_agent};
use skrills_subagents::launch::{claude_print_args, display_command, CodexSandbox};
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

/// The arguments the Claude CLI is run with for the agent at `agent_path`.
fn claude_args(agent_path: &AgentPath) -> Vec<String> {
    claude_print_args(&agent_path.prompt())
}

/// The command line `backend` runs, for `--dry-run`: the Codex sandbox
/// flags versus Claude's `--print` are the part worth seeing.
pub(super) fn dry_run_command(
    backend: AgentBackend,
    bin: &str,
    agent_path: &AgentPath,
    sandbox: CodexSandbox,
) -> String {
    let args = match backend {
        AgentBackend::Claude => claude_args(agent_path),
        AgentBackend::Codex => codex_args(agent_path, sandbox),
        AgentBackend::Auto => unreachable!("select_backend never returns Auto as a backend"),
    };
    display_command(bin, &args)
}

/// `--dangerously-bypass-sandbox` is a Codex flag. Running Claude instead
/// would drop it without a word, so that combination is refused.
fn check_bypass_backend(backend: AgentBackend, sandbox: CodexSandbox) -> Result<()> {
    if sandbox == CodexSandbox::Bypass && backend != AgentBackend::Codex {
        return Err(anyhow!(
            "--dangerously-bypass-sandbox applies only to the Codex backend, \
             but '{}' was selected; pass --backend codex",
            backend.as_str()
        ));
    }
    Ok(())
}

/// Launch an agent via the Claude CLI.
fn run_with_claude(bin: &str, agent_path: &AgentPath) -> Result<()> {
    let status = Command::new(bin)
        .args(claude_args(agent_path))
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
/// chosen for its permission prompts, to a Codex run is not a substitution
/// to make behind a warning.
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
    dangerously_bypass_sandbox: bool,
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
    let sandbox = codex_sandbox(dangerously_bypass_sandbox);
    check_bypass_backend(backend_kind, sandbox)?;

    let backend_label = backend_kind.as_str();
    println!("Backend: {backend_label} ({bin})");

    if dry_run {
        println!("Agent path: {}", agent_path.as_str());
        println!(
            "Would run: {}",
            dry_run_command(backend_kind, bin, &agent_path, sandbox)
        );
        return Ok(());
    }

    match backend_kind {
        AgentBackend::Claude => run_with_claude(bin, &agent_path),
        AgentBackend::Codex => run_with_codex(bin, &agent_path, sandbox),
        AgentBackend::Auto => unreachable!("select_backend never returns Auto as a backend"),
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

    #[cfg(unix)]
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

    #[cfg(unix)]
    #[test]
    fn is_available_returns_true_for_sh() {
        assert!(is_available("sh"));
    }

    #[test]
    fn find_binary_returns_none_for_empty_candidates() {
        let result = find_binary(&[]);
        assert!(result.is_none());
    }

    #[cfg(unix)]
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

    /// SA-48: `--backend claude` with Claude missing ran Codex without
    /// sandbox or approvals after a warning.
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

    #[test]
    fn dry_run_shows_each_backends_full_command() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agent.md");
        std::fs::write(&file, "x").unwrap();
        let path = AgentPath::new(&file).unwrap();

        let codex = dry_run_command(
            AgentBackend::Codex,
            "codex",
            &path,
            CodexSandbox::WorkspaceWrite,
        );
        let claude = dry_run_command(
            AgentBackend::Claude,
            "claude",
            &path,
            CodexSandbox::WorkspaceWrite,
        );

        // Rewritten for M453: this pinned `codex --yolo exec --timeout_ms`;
        // the default is now a sandboxed run, and Codex has no timeout flag.
        assert!(
            codex.starts_with("codex exec --sandbox workspace-write \""),
            "{codex}"
        );
        assert!(
            !codex.contains("--yolo") && !codex.contains("timeout"),
            "{codex}"
        );
        assert!(claude.starts_with("claude --print \""), "{claude}");
    }

    #[test]
    fn dry_run_shows_the_bypass_only_when_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agent.md");
        std::fs::write(&file, "x").unwrap();
        let path = AgentPath::new(&file).unwrap();

        let codex = dry_run_command(AgentBackend::Codex, "codex", &path, CodexSandbox::Bypass);

        assert!(
            codex.starts_with("codex exec --dangerously-bypass-approvals-and-sandbox \""),
            "{codex}"
        );
    }

    #[test]
    fn bypass_with_the_claude_backend_is_refused() {
        let err = check_bypass_backend(AgentBackend::Claude, CodexSandbox::Bypass)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--backend codex"), "{err}");
        assert!(check_bypass_backend(AgentBackend::Codex, CodexSandbox::Bypass).is_ok());
        assert!(check_bypass_backend(AgentBackend::Claude, CodexSandbox::WorkspaceWrite).is_ok());
    }
}
