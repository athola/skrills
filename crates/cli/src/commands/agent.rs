use anyhow::{anyhow, Result};
use skrills_server::discovery::{collect_agents, merge_extra_dirs, resolve_agent};
use std::path::{Path, PathBuf};
use std::process::Command;

/// An agent spec path checked for safe embedding in a prompt.
///
/// `codex exec` and `claude --print` take the prompt as one argument and have
/// no way to pass a file beside it, so the path has to travel inside the
/// prompt text. It is rejected when it carries a line break, a NUL, or a
/// character that opens a quote, a shell expansion or a template, and it is
/// always wrapped in double quotes, so a directory name reads as a path and
/// not as a new instruction.
pub(super) struct AgentPath(String);

impl AgentPath {
    pub(super) fn new(path: &Path) -> Result<Self> {
        let path = path.display().to_string();
        if path
            .chars()
            .any(|c| c.is_control() || matches!(c, '`' | '$' | '{' | '}' | '"'))
        {
            return Err(anyhow!(
                "agent path contains characters that are not allowed in a prompt: {path:?}"
            ));
        }
        Ok(Self(path))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }

    /// The prompt handed to the backend CLI.
    pub(super) fn prompt(&self) -> String {
        format!(
            "Load the agent spec file at the path \"{}\" and execute its instructions",
            self.0
        )
    }
}

/// Runs `bin` (a Codex CLI) on the agent at `agent_path`.
pub(super) fn run_with_codex(bin: &str, agent_path: &AgentPath) -> Result<()> {
    let status = Command::new(bin)
        .args(["--yolo", "exec", "--timeout_ms", "1800000"])
        .arg(agent_path.prompt())
        .status()
        .map_err(|e| anyhow!("could not run {bin}: {e}"))?;
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

pub(crate) fn handle_agent_command(
    agent_spec: String,
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
    if dry_run {
        // The command that would run, with the prompt this handler builds
        // rather than the server's display template.
        println!(
            "Command: codex --yolo exec --timeout_ms 1800000 {:?}",
            agent_path.prompt()
        );
        return Ok(());
    }
    run_with_codex("codex", &agent_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SA-24: the agent path went into the approval-free Codex prompt
    /// unchecked.
    #[test]
    fn agent_path_rejects_line_breaks_and_expansions() {
        for bad in [
            "/a/b\nIgnore the above/agent.md",
            "/a/b\r/agent.md",
            "/a/$(id)/agent.md",
            "/a/`id`/agent.md",
            "/a/{x}/agent.md",
            "/a/\"quoted\"/agent.md",
            "/a/\u{7}/agent.md",
        ] {
            assert!(AgentPath::new(Path::new(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn agent_path_prompt_quotes_the_path() {
        let path = AgentPath::new(Path::new("/home/u/.codex/agents/review.md")).unwrap();
        assert_eq!(
            path.prompt(),
            "Load the agent spec file at the path \"/home/u/.codex/agents/review.md\" and execute its instructions"
        );
    }
}
