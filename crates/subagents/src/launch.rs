//! Command lines that run an agent spec through Codex or Claude Code
//! non-interactively.
//!
//! `skrills agent`, `skrills multi-cli-agent`, the server's
//! `DEFAULT_AGENT_RUN_TEMPLATE`, the MCP subagent CLI backend and skill
//! generation build their Codex argv here, so the sandbox policy cannot drift
//! between them. Codex flags were checked against the Codex CLI
//! source (`codex-rs/utils/cli/src/shared_options.rs`,
//! `codex-rs/exec/src/cli.rs`): `exec` takes `--sandbox <mode>` and
//! `--dangerously-bypass-approvals-and-sandbox` (alias `--yolo`), always runs
//! without approval prompts, and has no timeout flag.

/// How a Codex run is confined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CodexSandbox {
    /// `--sandbox workspace-write`: the agent may write only inside the
    /// working directory. Codex also refuses to start outside a git
    /// repository unless the directory is trusted.
    #[default]
    WorkspaceWrite,
    /// `--sandbox read-only`: the agent may read but not write.
    ReadOnly,
    /// No sandbox flag: the user's Codex config (`sandbox_mode`) decides. The
    /// MCP subagent backend runs this way, as it always has.
    FromCodexConfig,
    /// `--dangerously-bypass-approvals-and-sandbox`: no sandbox and no
    /// approvals. Only when the user asks for it explicitly; agent files can
    /// come from third-party plugin caches.
    Bypass,
}

/// The prompt that tells a CLI to load and run the agent spec at `path`.
///
/// Neither CLI takes a file beside the prompt, so the path travels inside
/// the prompt text, quoted so a directory name reads as a path.
pub fn agent_prompt(path: &str) -> String {
    format!("Load the agent spec file at the path \"{path}\" and execute its instructions")
}

/// Where `codex exec` gets its prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexPrompt<'a> {
    /// As the last argument.
    Arg(&'a str),
    /// From stdin, through the `-` placeholder. Keeps the prompt out of `ps`
    /// and clear of the argv size limit.
    Stdin,
}

/// One non-interactive `codex exec` run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexExec<'a> {
    pub prompt: CodexPrompt<'a>,
    pub sandbox: CodexSandbox,
    /// `--skip-git-repo-check`: let Codex start outside a git repository.
    pub skip_git_repo_check: bool,
}

impl CodexExec<'_> {
    /// The argv after the `codex` binary.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec!["exec".to_string()];
        if self.skip_git_repo_check {
            args.push("--skip-git-repo-check".to_string());
        }
        let confinement: &[&str] = match self.sandbox {
            CodexSandbox::WorkspaceWrite => &["--sandbox", "workspace-write"],
            CodexSandbox::ReadOnly => &["--sandbox", "read-only"],
            CodexSandbox::FromCodexConfig => &[],
            CodexSandbox::Bypass => &["--dangerously-bypass-approvals-and-sandbox"],
        };
        args.extend(confinement.iter().map(|a| a.to_string()));
        args.push(match self.prompt {
            CodexPrompt::Arg(prompt) => prompt.to_string(),
            CodexPrompt::Stdin => "-".to_string(),
        });
        args
    }
}

/// Arguments for `codex exec <prompt>` under `sandbox`.
pub fn codex_exec_args(prompt: &str, sandbox: CodexSandbox) -> Vec<String> {
    CodexExec {
        prompt: CodexPrompt::Arg(prompt),
        sandbox,
        skip_git_repo_check: false,
    }
    .args()
}

/// Arguments for `claude --print <prompt>`.
pub fn claude_print_args(prompt: &str) -> Vec<String> {
    vec!["--print".to_string(), prompt.to_string()]
}

/// `bin` and `args` as one line for display, with arguments that contain a
/// space quoted.
pub fn display_command(bin: &str, args: &[String]) -> String {
    std::iter::once(bin.to_string())
        .chain(args.iter().map(|a| {
            if a.contains(' ') {
                format!("{a:?}")
            } else {
                a.clone()
            }
        }))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROMPT: &str =
        "Load the agent spec file at the path \"/a/agent.md\" and execute its instructions";

    /// M453: agents found in plugin caches ran under `codex --yolo`.
    #[test]
    fn default_codex_launch_is_sandboxed_without_bypass() {
        let args = codex_exec_args(PROMPT, CodexSandbox::default());
        assert_eq!(args, ["exec", "--sandbox", "workspace-write", PROMPT]);
        assert!(!args
            .iter()
            .any(|a| a.contains("dangerously") || a == "--yolo"));
    }

    #[test]
    fn bypass_is_added_only_when_asked_for() {
        let args = codex_exec_args(PROMPT, CodexSandbox::Bypass);
        assert_eq!(
            args,
            ["exec", "--dangerously-bypass-approvals-and-sandbox", PROMPT]
        );
    }

    /// `--timeout_ms` is not a Codex flag; current Codex rejects the argv.
    #[test]
    fn codex_args_carry_no_timeout_flag() {
        for sandbox in [CodexSandbox::WorkspaceWrite, CodexSandbox::Bypass] {
            let args = codex_exec_args(PROMPT, sandbox);
            assert!(!args.iter().any(|a| a.contains("timeout")), "{args:?}");
        }
    }

    #[test]
    fn display_command_quotes_arguments_with_spaces() {
        let args = codex_exec_args(&agent_prompt("/a/agent.md"), CodexSandbox::default());
        assert_eq!(
            display_command("codex", &args),
            "codex exec --sandbox workspace-write \"Load the agent spec file at the path \\\"/a/agent.md\\\" and execute its instructions\""
        );
    }

    #[test]
    fn codex_exec_puts_an_argument_prompt_last() {
        let args = CodexExec {
            prompt: CodexPrompt::Arg(PROMPT),
            sandbox: CodexSandbox::WorkspaceWrite,
            skip_git_repo_check: false,
        }
        .args();
        assert_eq!(args, codex_exec_args(PROMPT, CodexSandbox::WorkspaceWrite));
    }

    /// `codex exec -` reads the prompt from stdin, keeping it out of `ps`.
    #[test]
    fn codex_exec_reads_a_stdin_prompt_from_dash() {
        let args = CodexExec {
            prompt: CodexPrompt::Stdin,
            sandbox: CodexSandbox::WorkspaceWrite,
            skip_git_repo_check: false,
        }
        .args();
        assert_eq!(args, ["exec", "--sandbox", "workspace-write", "-"]);
    }

    #[test]
    fn codex_exec_read_only_sandbox() {
        let args = CodexExec {
            prompt: CodexPrompt::Arg(PROMPT),
            sandbox: CodexSandbox::ReadOnly,
            skip_git_repo_check: false,
        }
        .args();
        assert_eq!(args, ["exec", "--sandbox", "read-only", PROMPT]);
    }

    /// No sandbox flag: the user's Codex config decides.
    #[test]
    fn codex_exec_from_codex_config_passes_no_sandbox_flag() {
        let args = CodexExec {
            prompt: CodexPrompt::Stdin,
            sandbox: CodexSandbox::FromCodexConfig,
            skip_git_repo_check: false,
        }
        .args();
        assert_eq!(args, ["exec", "-"]);
    }

    #[test]
    fn codex_exec_skips_the_git_repo_check_only_when_asked() {
        let args = CodexExec {
            prompt: CodexPrompt::Arg(PROMPT),
            sandbox: CodexSandbox::ReadOnly,
            skip_git_repo_check: true,
        }
        .args();
        assert_eq!(
            args,
            [
                "exec",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                PROMPT
            ]
        );
    }

    #[test]
    fn claude_runs_the_prompt_with_print() {
        assert_eq!(claude_print_args(PROMPT), ["--print", PROMPT]);
    }
}
