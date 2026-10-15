//! CLI-based backend adapter for subprocess execution.
//!
//! This adapter spawns CLI tools (like `codex` or `claude`) as subprocesses
//! to execute agent prompts with tool capabilities.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use thiserror::Error;
use time::OffsetDateTime;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex, Semaphore};

use crate::backend::{spawn_run, AdapterCapabilities, BackendAdapter};
use crate::cli_detection::{default_cli_binary, normalize_cli_binary};
use crate::store::{
    BackendKind, RunEvent, RunId, RunRequest, RunState, RunStatus, RunStore, SubagentTemplate,
};

/// Errors that can occur during CLI subprocess execution.
#[derive(Error, Debug)]
pub enum CliError {
    /// Failed to spawn the subprocess.
    #[error("failed to spawn CLI process '{binary}': {source}")]
    SpawnFailed {
        binary: String,
        #[source]
        source: io::Error,
    },

    /// The process exited with a non-zero exit code.
    ///
    /// Currently process failures are recorded via [`RunStore::update_status`]
    /// rather than returned as errors, but this variant is exposed for:
    /// - API consumers who want to construct or match on this error type
    /// - Test code that verifies error display formatting
    #[error("CLI process exited with code {exit_code:?}: {stderr}")]
    ProcessFailed {
        exit_code: Option<i32>,
        stderr: String,
    },

    /// Failed to wait for the process to complete.
    #[error("failed to wait for CLI process: {0}")]
    WaitFailed(#[source] io::Error),
}

/// Default timeout for CLI subprocess execution (5 minutes).
const DEFAULT_TIMEOUT_MS: u64 = 300_000;

/// Configuration for CLI-based adapter.
#[derive(Debug, Clone)]
pub struct CliConfig {
    /// Path to the CLI binary (e.g., "codex", "claude", or absolute path).
    pub binary: String,
    /// Working directory for subprocess execution.
    pub working_dir: Option<PathBuf>,
    /// Environment variables to set for the subprocess.
    pub env_vars: HashMap<String, String>,
    /// Timeout for the subprocess.
    pub timeout: Duration,
    /// Whether to run in non-interactive mode.
    pub non_interactive: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            binary: default_cli_binary(),
            working_dir: None,
            env_vars: HashMap::new(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            non_interactive: true,
        }
    }
}

impl CliConfig {
    /// Create a new CLI config with the specified binary.
    pub fn new(binary: impl Into<String>) -> Self {
        Self {
            binary: binary.into(),
            ..Default::default()
        }
    }

    /// Create configuration from environment variables.
    ///
    /// Looks for:
    /// - SKRILLS_CLI_BINARY: Path to the CLI binary ("auto" uses current client)
    /// - SKRILLS_CLI_WORKING_DIR: Working directory
    /// - SKRILLS_CLI_TIMEOUT_MS: Timeout in milliseconds
    pub fn from_env() -> Self {
        let binary = normalize_cli_binary(std::env::var("SKRILLS_CLI_BINARY").ok())
            .unwrap_or_else(default_cli_binary);
        let working_dir = std::env::var("SKRILLS_CLI_WORKING_DIR")
            .ok()
            .map(PathBuf::from);
        let timeout_ms = crate::backend::config::parse_timeout_ms(
            "SKRILLS_CLI_TIMEOUT_MS",
            std::env::var("SKRILLS_CLI_TIMEOUT_MS").ok(),
            DEFAULT_TIMEOUT_MS,
        );

        Self {
            binary,
            working_dir,
            env_vars: HashMap::new(),
            timeout: Duration::from_millis(timeout_ms),
            non_interactive: true,
        }
    }

    /// Set the working directory.
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }

    /// Add an environment variable.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Set the timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Disable non-interactive mode (for testing with simple commands).
    pub fn without_non_interactive(mut self) -> Self {
        self.non_interactive = false;
        self
    }
}

/// Most bytes of stdout kept for the `completion` event; the rest is drained
/// and dropped so a chatty child cannot grow server memory without bound.
const MAX_CAPTURED_STDOUT: usize = 1024 * 1024;

/// Most bytes of stderr kept for the failure message.
const MAX_CAPTURED_STDERR: usize = 64 * 1024;

/// How long output is still read after the child exits, for a pipe that a
/// background process inherited and keeps open.
const OUTPUT_DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Most bytes of one output line held in memory and kept in its `stream`
/// event; the rest of the line is read and dropped.
const MAX_STREAM_LINE: usize = 16 * 1024;

/// Environment variables a CLI child inherits from the server.
///
/// The child gets a cleared environment plus these names, so server secrets
/// such as `SKRILLS_*_API_KEY` never reach it. The provider keys the `claude`
/// and `codex` CLIs read themselves are kept, as are the locale, proxy and
/// certificate settings they need to reach their APIs.
const INHERITED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "TZ",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "CLAUDE_CONFIG_DIR",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    // Other Claude Code sign-ins and providers (code.claude.com/docs/en/env-vars,
    // amazon-bedrock, google-vertex-ai).
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_PROFILE",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "CLOUD_ML_REGION",
    "CODEX_HOME",
    // Codex CLI API-key sign-in (learn.chatgpt.com/docs/auth).
    "CODEX_API_KEY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    // Windows needs these to start most programs at all.
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
];

/// Which known CLI a binary path names, judged by its file stem so that
/// `/opt/bin/claude` and `claude.exe` count while `my-claude-wrapper` does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnownCli {
    Claude,
    Codex,
}

impl KnownCli {
    pub(crate) fn from_binary(binary: &str) -> Option<Self> {
        let stem = std::path::Path::new(binary)
            .file_stem()?
            .to_str()?
            .to_ascii_lowercase();
        match stem.as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// Tracks a running CLI subprocess.
///
/// The child itself stays owned by the task that runs it; the map only holds
/// the cancel signal, so no lock is held while the child is awaited.
pub(crate) struct CliProcess {
    cancel: oneshot::Sender<()>,
}

/// Cancel signals for live CLI children, keyed by run.
///
/// Shared by every adapter the service builds, so `stop-run` can reach a child
/// that was spawned by an adapter built for an earlier call.
pub(crate) type CliProcessMap = Arc<Mutex<HashMap<RunId, CliProcess>>>;

/// Signals the child of `run_id` to be killed. Returns whether one was live.
pub(crate) async fn cancel_cli_process(processes: &CliProcessMap, run_id: RunId) -> bool {
    let process = processes.lock().await.remove(&run_id);
    match process {
        Some(process) => {
            // The receiver is gone only when the run already finished.
            let _ = process.cancel.send(());
            true
        }
        None => false,
    }
}

/// How a CLI child's run ended.
enum Outcome {
    Exited(io::Result<std::process::ExitStatus>),
    StreamFailed(anyhow::Error),
    Canceled,
    TimedOut,
}

/// Appends `chunk` to `buf` while `buf` stays under `cap` bytes.
fn push_capped(buf: &mut String, chunk: &str, cap: usize, truncated: &mut bool) {
    if buf.len() + chunk.len() <= cap {
        buf.push_str(chunk);
        return;
    }
    let mut end = cap.saturating_sub(buf.len()).min(chunk.len());
    while end > 0 && !chunk.is_char_boundary(end) {
        end -= 1;
    }
    buf.push_str(&chunk[..end]);
    *truncated = true;
}

/// Reads one line, through its `\n` or EOF, into `buf`, keeping at most
/// `cap` bytes and reading past the rest, so a child that writes no newline
/// cannot grow `buf` without bound. Returns `None` at EOF, otherwise whether
/// the line was cut.
async fn read_line_capped<R>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
) -> io::Result<Option<bool>>
where
    R: AsyncBufRead + Unpin,
{
    buf.clear();
    let mut read_any = false;
    let mut truncated = false;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(read_any.then_some(truncated));
        }
        read_any = true;
        let (len, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        let keep = len.min(cap.saturating_sub(buf.len()));
        truncated |= keep < len;
        buf.extend_from_slice(&available[..keep]);
        reader.consume(len);
        if done {
            return Ok(Some(truncated));
        }
    }
}

/// Reads a pipe to EOF, decoding each line lossily so one invalid UTF-8
/// sequence cannot end the read early, and keeps at most `cap` bytes.
async fn drain_capped<R>(pipe: R, cap: usize) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = BufReader::new(pipe);
    let mut buf = Vec::new();
    let mut out = String::new();
    let mut truncated = false;
    loop {
        match read_line_capped(&mut reader, &mut buf, cap).await {
            Ok(None) => break,
            Ok(Some(_)) => push_capped(
                &mut out,
                &String::from_utf8_lossy(&buf),
                cap,
                &mut truncated,
            ),
            Err(e) => {
                tracing::warn!(error = %e, "failed to read CLI stderr");
                break;
            }
        }
    }
    out
}

/// CLI-based adapter that spawns subprocesses for agent execution.
///
/// This adapter is designed for agents that require tool capabilities,
/// spawning CLI tools like `codex` or `claude` as subprocesses. Despite the
/// name it serves any CLI; [`BackendAdapter::backend`] reports the backend the
/// configured binary belongs to.
pub struct CodexCliAdapter {
    config: CliConfig,
    /// Cancel signals for active processes, indexed by run_id.
    processes: CliProcessMap,
    /// One permit per live CLI child; shared by every adapter the service
    /// builds, so a looping caller cannot start unbounded processes.
    slots: Arc<Semaphore>,
}

/// Most CLI children one service runs at once.
pub(crate) const MAX_CONCURRENT_CLI_RUNS: usize = 8;

impl CodexCliAdapter {
    /// Create a new CLI adapter with default configuration.
    pub fn new() -> Self {
        Self::with_config(CliConfig::default())
    }

    /// Create a CLI adapter with the specified configuration.
    pub fn with_config(config: CliConfig) -> Self {
        Self::with_shared_processes(config, Arc::new(Mutex::new(HashMap::new())))
    }

    /// Create a CLI adapter that registers its children in `processes`.
    pub(crate) fn with_shared_processes(config: CliConfig, processes: CliProcessMap) -> Self {
        Self::with_shared_state(
            config,
            processes,
            Arc::new(Semaphore::new(MAX_CONCURRENT_CLI_RUNS)),
        )
    }

    /// Create a CLI adapter that registers its children in `processes` and
    /// takes a permit from `slots` for each run.
    pub(crate) fn with_shared_state(
        config: CliConfig,
        processes: CliProcessMap,
        slots: Arc<Semaphore>,
    ) -> Self {
        Self {
            config,
            processes,
            slots,
        }
    }

    /// Create a CLI adapter from environment variables.
    pub fn from_env() -> Self {
        Self::with_config(CliConfig::from_env())
    }

    /// Returns a reference to the adapter's configuration.
    ///
    /// This is primarily useful for testing to verify the correct binary was selected.
    #[cfg(test)]
    pub fn config(&self) -> &CliConfig {
        &self.config
    }

    /// Whether the prompt goes to the child on stdin rather than in argv.
    fn prompt_on_stdin(&self) -> bool {
        self.config.non_interactive && KnownCli::from_binary(&self.config.binary).is_some()
    }

    /// Build the command arguments for the CLI.
    ///
    /// Each known CLI gets its real headless form and reads the prompt from
    /// stdin: `claude --print` and `codex exec -`. In argv a prompt is visible
    /// to other users through `ps`, and past about 128 KiB the spawn fails.
    /// Any other binary gets `-- <prompt>`; the `--` ends option parsing, so a
    /// prompt that starts with `-` cannot be read as a flag such as one that
    /// disables permission checks.
    fn build_command_args(&self, prompt: &str) -> Vec<String> {
        if !self.config.non_interactive {
            return Vec::new();
        }
        match KnownCli::from_binary(&self.config.binary) {
            Some(KnownCli::Claude) => vec!["--print".to_string()],
            Some(KnownCli::Codex) => vec!["exec".to_string(), "-".to_string()],
            None => vec!["--".to_string(), prompt.to_string()],
        }
    }

    /// Build the child command: cleared environment plus [`INHERITED_ENV`] and
    /// the configured variables, stdin piped for the prompt or closed, both
    /// outputs piped, and the child killed if its handle is dropped.
    fn build_command(&self, args: &[String]) -> Command {
        let mut cmd = Command::new(&self.config.binary);
        cmd.args(args);
        if let Some(ref dir) = self.config.working_dir {
            cmd.current_dir(dir);
        }
        cmd.env_clear();
        for name in INHERITED_ENV {
            if let Some(value) = std::env::var_os(name) {
                cmd.env(name, value);
            }
        }
        for (key, value) in &self.config.env_vars {
            cmd.env(key, value);
        }
        cmd.stdin(if self.prompt_on_stdin() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        // Its own process group, so stop and timeout can kill the tool
        // processes the CLI starts along with it.
        #[cfg(unix)]
        cmd.process_group(0);
        cmd
    }

    /// Execute the CLI subprocess and capture output.
    async fn execute_run(
        &self,
        run_id: RunId,
        request: RunRequest,
        store: Arc<dyn RunStore>,
    ) -> Result<()> {
        tracing::info!(
            run_id = %run_id,
            binary = %self.config.binary,
            "Starting CLI subprocess execution"
        );

        // The run id reached the caller before this task started, so a
        // `stop-run` may already have come. Register the cancel signal first,
        // then check: a stop after the check finds the signal, and one before
        // it left the run terminal, so no child is spawned.
        let (cancel_tx, cancel_rx) = oneshot::channel();
        self.processes
            .lock()
            .await
            .insert(run_id, CliProcess { cancel: cancel_tx });
        let finished = store
            .status(run_id)
            .await
            .map(|status| status.is_none_or(|s| s.state.is_terminal()));
        if !matches!(finished, Ok(false)) {
            self.processes.lock().await.remove(&run_id);
            tracing::debug!(run_id = %run_id, "run finished before its CLI child was spawned");
            return finished.map(|_| ());
        }

        let args = self.build_command_args(&request.prompt);
        let mut cmd = self.build_command(&args);
        // Arguments may hold the prompt, which is not logged.
        tracing::debug!(
            run_id = %run_id,
            arg_count = args.len(),
            prompt_bytes = request.prompt.len(),
            working_dir = ?self.config.working_dir,
            "Spawning CLI process"
        );
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.processes.lock().await.remove(&run_id);
                return Err(CliError::SpawnFailed {
                    binary: self.config.binary.clone(),
                    source: e,
                }
                .into());
            }
        };

        // Written on its own task: a prompt larger than the pipe buffer would
        // otherwise block until the child reads it, while nothing drains the
        // child's output. Dropping the handle closes stdin, ending the prompt.
        if let Some(mut stdin) = child.stdin.take() {
            let prompt = request.prompt.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt as _;
                if let Err(e) = stdin.write_all(prompt.as_bytes()).await {
                    tracing::warn!(%run_id, error = %e, "failed to write the prompt to CLI stdin");
                }
            });
        }

        let outcome = self
            .supervise(run_id, &mut child, cancel_rx, store.clone())
            .await;

        // Whatever happened, the child must not outlive this function.
        self.processes.lock().await.remove(&run_id);
        let (output, error_output, status, output_truncated) = match outcome {
            (Outcome::Exited(Ok(status)), output, error_output, truncated) => {
                (output, error_output, status, truncated)
            }
            (Outcome::Exited(Err(e)), ..) => return Err(CliError::WaitFailed(e).into()),
            (Outcome::StreamFailed(e), ..) => {
                Self::kill(run_id, &mut child).await;
                return Err(e);
            }
            (Outcome::Canceled, ..) => {
                Self::kill(run_id, &mut child).await;
                tracing::debug!(run_id = %run_id, "CLI process stopped by user");
                return Ok(());
            }
            (Outcome::TimedOut, ..) => {
                Self::kill(run_id, &mut child).await;
                let millis = self.config.timeout.as_millis();
                tracing::warn!(run_id = %run_id, timeout_ms = %millis, "CLI subprocess timed out");
                store
                    .append_event(
                        run_id,
                        RunEvent {
                            ts: OffsetDateTime::now_utc(),
                            kind: "error".into(),
                            data: Some(json!({ "timeout_ms": millis })),
                        },
                    )
                    .await?;
                store
                    .update_status(
                        run_id,
                        RunStatus {
                            state: RunState::Failed,
                            message: Some(format!("CLI timed out after {millis} ms")),
                            updated_at: OffsetDateTime::now_utc(),
                        },
                    )
                    .await?;
                return Ok(());
            }
        };

        // Update status based on exit code
        if status.success() {
            tracing::info!(
                run_id = %run_id,
                "CLI subprocess completed successfully"
            );
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: "completion".into(),
                        data: Some(json!({ "text": output.trim(), "truncated": output_truncated })),
                    },
                )
                .await?;

            store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Succeeded,
                        message: Some(if output_truncated {
                            format!(
                                "completed (truncated: output over {MAX_CAPTURED_STDOUT} bytes)"
                            )
                        } else {
                            "completed".into()
                        }),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await?;
        } else {
            let exit_code = status.code();
            tracing::warn!(
                run_id = %run_id,
                exit_code = ?exit_code,
                "CLI subprocess failed"
            );
            let msg = if error_output.is_empty() {
                format!("CLI exited with code {:?}", exit_code)
            } else {
                format!(
                    "CLI exited with code {:?}: {}",
                    exit_code,
                    error_output.trim()
                )
            };

            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: "error".into(),
                        data: Some(json!({
                            "exit_code": exit_code,
                            "stderr": error_output.trim(),
                        })),
                    },
                )
                .await?;

            store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Failed,
                        message: Some(msg),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await?;
        }

        Ok(())
    }

    /// Streams stdout as events while stderr drains on its own task, then waits
    /// for the child, racing all of it against the cancel signal and the
    /// configured timeout.
    async fn supervise(
        &self,
        run_id: RunId,
        child: &mut Child,
        cancel_rx: oneshot::Receiver<()>,
        store: Arc<dyn RunStore>,
    ) -> (Outcome, String, String, bool) {
        store
            .append_event(
                run_id,
                RunEvent {
                    ts: OffsetDateTime::now_utc(),
                    kind: "start".into(),
                    data: Some(json!({
                        "binary": self.config.binary,
                        "working_dir": self.config.working_dir,
                        "pid": child.id(),
                    })),
                },
            )
            .await
            .unwrap_or_else(
                |e| tracing::warn!(error = %e, %run_id, "failed to record start event"),
            );

        // Both pipes drain at once: reading stdout to EOF before touching
        // stderr deadlocks once the child fills the stderr pipe buffer.
        let stdout = child.stdout.take();
        let stderr_task = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(drain_capped(stderr, MAX_CAPTURED_STDERR)));

        let mut output = String::new();
        // Whether stdout went past MAX_CAPTURED_STDOUT and was cut.
        let mut truncated = false;
        // Streams stdout to EOF; returns the failure that should end the run.
        let read_stdout = async {
            if let Some(stdout) = stdout {
                let mut reader = BufReader::new(stdout);
                let mut buf = Vec::new();
                loop {
                    let line_cut =
                        match read_line_capped(&mut reader, &mut buf, MAX_STREAM_LINE).await {
                            Ok(None) => break,
                            Ok(Some(cut)) => cut,
                            Err(e) => {
                                tracing::warn!(error = %e, %run_id, "failed to read CLI stdout");
                                break;
                            }
                        };
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']);
                    push_capped(&mut output, line, MAX_CAPTURED_STDOUT, &mut truncated);
                    push_capped(&mut output, "\n", MAX_CAPTURED_STDOUT, &mut truncated);
                    let data = if line_cut {
                        json!({ "line": line, "truncated": true })
                    } else {
                        json!({ "line": line })
                    };
                    if let Err(e) = store
                        .append_event(
                            run_id,
                            RunEvent {
                                ts: OffsetDateTime::now_utc(),
                                kind: "stream".into(),
                                data: Some(data),
                            },
                        )
                        .await
                    {
                        return Some(Outcome::StreamFailed(e));
                    }
                }
            }
            None
        };
        // The child's exit, not stdout EOF, ends the run: a background process
        // that inherited stdout can hold it open long after the child is gone.
        // Output still in the pipe is drained for a short grace period.
        let run = async {
            tokio::pin!(read_stdout);
            tokio::select! {
                failed = &mut read_stdout => match failed {
                    Some(outcome) => outcome,
                    None => Outcome::Exited(child.wait().await),
                },
                status = child.wait() => {
                    match tokio::time::timeout(OUTPUT_DRAIN_GRACE, &mut read_stdout).await {
                        Ok(Some(outcome)) => outcome,
                        Ok(None) | Err(_) => Outcome::Exited(status),
                    }
                }
            }
        };

        let outcome = tokio::select! {
            outcome = run => outcome,
            _ = cancel_rx => Outcome::Canceled,
            _ = tokio::time::sleep(self.config.timeout) => Outcome::TimedOut,
        };

        // A killed or exited child closes stderr, so this join ends promptly;
        // after a cancel or timeout the caller kills the child first, so the
        // drain is aborted here instead of awaited.
        let error_output = match stderr_task {
            // Bounded: a grandchild that inherited stderr can hold it open
            // after the child itself has exited.
            // On timeout the drain is aborted: dropping its handle would
            // detach it, leaking a task and the pipe for as long as the
            // grandchild lives.
            Some(mut task) if matches!(outcome, Outcome::Exited(_)) => {
                match tokio::time::timeout(OUTPUT_DRAIN_GRACE, &mut task).await {
                    Ok(Ok(stderr)) => stderr,
                    Ok(Err(e)) => {
                        tracing::warn!(error = %e, %run_id, "CLI stderr drain task failed");
                        String::new()
                    }
                    Err(_) => {
                        task.abort();
                        String::new()
                    }
                }
            }
            Some(task) => {
                task.abort();
                String::new()
            }
            None => String::new(),
        };
        (outcome, output, error_output, truncated)
    }

    /// Kills the child and, on unix, every process in its group: `claude`
    /// and `codex` start shells, builds and MCP servers that would otherwise
    /// keep running. The crate forbids `unsafe`, so the group is signalled
    /// through the POSIX `kill` utility rather than `killpg`.
    async fn kill(run_id: RunId, child: &mut Child) {
        #[cfg(unix)]
        if let Some(pid) = child.id() {
            let group = format!("-{pid}");
            match Command::new("kill")
                .args(["-KILL", "--", &group])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
            {
                Ok(status) if status.success() => {}
                Ok(status) => tracing::warn!(
                    run_id = %run_id,
                    %status,
                    "failed to kill the CLI process group"
                ),
                Err(e) => tracing::warn!(
                    run_id = %run_id,
                    error = %e,
                    "failed to run kill for the CLI process group"
                ),
            }
        }
        if let Err(e) = child.kill().await {
            tracing::warn!(
                run_id = %run_id,
                error = %e,
                "Failed to kill subprocess - process may still be running"
            );
        }
    }
}

impl Default for CodexCliAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BackendAdapter for CodexCliAdapter {
    fn backend(&self) -> BackendKind {
        match KnownCli::from_binary(&self.config.binary) {
            Some(KnownCli::Claude) => BackendKind::Claude,
            Some(KnownCli::Codex) => BackendKind::Codex,
            None => BackendKind::Other("cli".into()),
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            supports_schema: false, // CLI doesn't support structured output schema
            supports_async: true,   // Subprocess runs asynchronously
            supports_tracing: false,
            supports_secure_transcript: false,
        }
    }

    async fn list_templates(&self) -> Result<Vec<SubagentTemplate>> {
        // CLI adapter provides a single template representing CLI-based execution
        Ok(vec![SubagentTemplate {
            id: "cli-default".into(),
            name: format!("{} CLI Agent", self.config.binary),
            description: Some(format!(
                "CLI-based agent using {} subprocess",
                self.config.binary
            )),
            backend: self.backend(),
            capabilities: vec!["tools".into(), "subprocess".into()],
        }])
    }

    async fn run(&self, mut request: RunRequest, store: Arc<dyn RunStore>) -> Result<RunId> {
        request.backend = self.backend();
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            anyhow::anyhow!(
                "too many concurrent CLI subagent runs; wait for one to finish or stop one"
            )
        })?;
        let adapter = CodexCliAdapter::with_shared_state(
            self.config.clone(),
            self.processes.clone(),
            self.slots.clone(),
        );
        spawn_run(
            request,
            store,
            "spawning CLI process",
            "CLI",
            move |run_id, request, store| async move {
                // Held until the child is gone.
                let _permit = permit;
                adapter.execute_run(run_id, request, store).await
            },
        )
        .await
    }

    async fn stop(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<bool> {
        let stopped = store.stop(run_id).await?;
        cancel_cli_process(&self.processes, run_id).await;
        Ok(stopped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemRunStore;
    use crate::store::RunRecord;
    use std::sync::LazyLock;
    use tokio::sync::Mutex;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    async fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }

    fn env_guard_blocking() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.blocking_lock()
    }

    use skrills_test_utils::{set_env_var, EnvVarGuard};

    fn set_cli_env(client: Option<&str>, cli_binary: Option<&str>) -> Vec<EnvVarGuard> {
        vec![
            set_env_var("SKRILLS_CLIENT", client),
            set_env_var("SKRILLS_CLI_BINARY", cli_binary),
            set_env_var("CLAUDE_CODE_SESSION", None),
            set_env_var("CLAUDE_CLI", None),
            set_env_var("__CLAUDE_MCP_SERVER", None),
            set_env_var("CLAUDE_CODE_ENTRYPOINT", None),
            set_env_var("CODEX_SESSION_ID", None),
            set_env_var("CODEX_CLI", None),
            set_env_var("CODEX_HOME", None),
        ]
    }

    /// Wait for a run to complete (not in Running state) with polling.
    /// This is more reliable than a fixed sleep, especially in CI environments.
    async fn wait_for_completion(
        store: &Arc<dyn RunStore>,
        run_id: RunId,
        timeout: Duration,
    ) -> Option<RunStatus> {
        // Yield to let the spawned task get scheduled before we start polling.
        // This is crucial in CI environments where CPU time is limited.
        tokio::task::yield_now().await;

        let start = std::time::Instant::now();
        // Use 50ms poll interval to reduce scheduler contention in CI
        let poll_interval = Duration::from_millis(50);

        loop {
            if let Ok(Some(status)) = store.status(run_id).await {
                if status.state != RunState::Running && status.state != RunState::Pending {
                    return Some(status);
                }
            }

            if start.elapsed() > timeout {
                // Return whatever state we have
                return store.status(run_id).await.ok().flatten();
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    #[test]
    fn test_cli_config_default() {
        let _guard = env_guard_blocking();
        let _env_guards = set_cli_env(None, None);
        let config = CliConfig::default();
        assert_eq!(config.binary, "claude");
        assert!(config.working_dir.is_none());
        assert!(config.env_vars.is_empty());
        assert_eq!(config.timeout, Duration::from_millis(DEFAULT_TIMEOUT_MS));
        assert!(config.non_interactive);
    }

    #[test]
    fn test_cli_config_from_env_auto_uses_client_hint() {
        let _guard = env_guard_blocking();
        let _env_guards = set_cli_env(Some("codex"), Some("auto"));
        let config = CliConfig::from_env();
        assert_eq!(config.binary, "codex");
    }

    #[test]
    fn test_cli_config_new() {
        let config = CliConfig::new("claude");
        assert_eq!(config.binary, "claude");
    }

    #[test]
    fn test_cli_config_builder() {
        let config = CliConfig::new("codex")
            .with_working_dir(PathBuf::from("/tmp"))
            .with_env("FOO", "bar")
            .with_timeout(Duration::from_secs(60));

        assert_eq!(config.binary, "codex");
        assert_eq!(config.working_dir, Some(PathBuf::from("/tmp")));
        assert_eq!(config.env_vars.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(config.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_codex_cli_adapter_new() {
        let _guard = env_guard_blocking();
        let _env_guards = set_cli_env(Some("codex"), None);
        let adapter = CodexCliAdapter::new();
        assert_eq!(adapter.config.binary, "codex");
    }

    /// M319: `SKRILLS_CLI_TIMEOUT_MS=0` falls back to the default.
    #[test]
    fn cli_timeout_env_zero_uses_the_default() {
        let _guard = env_guard_blocking();
        let _t = set_env_var("SKRILLS_CLI_TIMEOUT_MS", Some("0"));
        assert_eq!(
            CliConfig::from_env().timeout,
            Duration::from_millis(DEFAULT_TIMEOUT_MS)
        );
    }

    #[test]
    fn test_codex_cli_adapter_with_config() {
        let config = CliConfig::new("custom-cli");
        let adapter = CodexCliAdapter::with_config(config);
        assert_eq!(adapter.config.binary, "custom-cli");
    }

    #[test]
    fn test_codex_cli_adapter_backend() {
        let _guard = env_guard_blocking();
        let _env_guards = set_cli_env(Some("codex"), None);
        let adapter = CodexCliAdapter::new();
        assert_eq!(adapter.backend(), BackendKind::Codex);
    }

    #[test]
    fn test_codex_cli_adapter_capabilities() {
        let _guard = env_guard_blocking();
        let _env_guards = set_cli_env(Some("codex"), None);
        let adapter = CodexCliAdapter::new();
        let caps = adapter.capabilities();

        assert!(!caps.supports_schema);
        assert!(caps.supports_async);
        assert!(!caps.supports_tracing);
        assert!(!caps.supports_secure_transcript);
    }

    #[tokio::test]
    async fn test_codex_cli_adapter_list_templates() {
        let _guard = env_guard().await;
        let _env_guards = set_cli_env(Some("codex"), None);
        let adapter = CodexCliAdapter::new();
        let templates = adapter.list_templates().await.unwrap();

        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].id, "cli-default");
        assert!(templates[0].name.contains("codex"));
        assert!(templates[0].capabilities.contains(&"tools".to_string()));
        assert!(templates[0]
            .capabilities
            .contains(&"subprocess".to_string()));
    }

    #[test]
    fn test_build_command_args_codex() {
        let _guard = env_guard_blocking();
        let _env_guards = set_cli_env(Some("codex"), None);
        let adapter = CodexCliAdapter::new();
        let args = adapter.build_command_args("test prompt");

        // `codex exec -` reads the prompt from stdin (M256: argv is visible
        // to other users and size-limited).
        assert_eq!(args, vec!["exec", "-"]);
    }

    #[test]
    fn test_build_command_args_claude() {
        let config = CliConfig::new("claude");
        let adapter = CodexCliAdapter::with_config(config);
        let args = adapter.build_command_args("test prompt");

        // `claude --print` with no positional prompt reads it from stdin.
        assert_eq!(args, vec!["--print"]);
    }

    #[test]
    fn test_build_command_args_matches_on_file_stem_not_substring() {
        let by_path = CodexCliAdapter::with_config(CliConfig::new("/opt/tools/claude"));
        assert_eq!(by_path.build_command_args("p")[0], "--print");

        // A binary whose name merely contains "codex" is not the codex CLI.
        let wrapper = CodexCliAdapter::with_config(CliConfig::new("/usr/bin/not-codex-wrapper"));
        assert_eq!(wrapper.build_command_args("p"), vec!["--", "p"]);
    }

    #[test]
    fn test_prompt_starting_with_dash_is_not_parsed_as_a_flag() {
        let adapter = CodexCliAdapter::with_config(CliConfig::new("my-agent-cli"));
        let args = adapter.build_command_args("--dangerously-skip-permissions");
        let sep = args.iter().position(|a| a == "--").expect("separator");
        assert_eq!(args[sep + 1], "--dangerously-skip-permissions");
    }

    #[test]
    fn test_backend_follows_the_configured_binary() {
        let claude = CodexCliAdapter::with_config(CliConfig::new("claude"));
        assert_eq!(claude.backend(), BackendKind::Claude);
        let codex = CodexCliAdapter::with_config(CliConfig::new("/usr/local/bin/codex"));
        assert_eq!(codex.backend(), BackendKind::Codex);
        let other = CodexCliAdapter::with_config(CliConfig::new("sh"));
        assert_eq!(other.backend(), BackendKind::Other("cli".into()));
    }

    #[test]
    fn test_build_command_args_non_interactive_disabled() {
        let config = CliConfig::new("custom").without_non_interactive();
        let adapter = CodexCliAdapter::with_config(config);
        let args = adapter.build_command_args("test prompt");

        // When non_interactive is false, no args are added
        assert!(args.is_empty());
    }

    #[tokio::test]
    async fn test_run_creates_record_in_store() {
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());

        // Use a binary that doesn't exist to test record creation
        // The spawn will fail but the record should be created
        let config = CliConfig::new("nonexistent-binary-12345");
        let adapter = CodexCliAdapter::with_config(config);

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "test prompt".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Status should be Running initially (before spawn fails)
        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Running);

        // Give time for the spawned task to attempt execution
        tokio::time::sleep(Duration::from_millis(100)).await;

        // After spawn fails, status should be Failed
        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Failed);
    }

    #[tokio::test]
    async fn test_run_with_true_succeeds() {
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());

        // Use 'true' command which always succeeds with no args
        let config = CliConfig::new("true").without_non_interactive();
        let adapter = CodexCliAdapter::with_config(config);

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Wait for the process to complete with polling (more reliable in CI)
        let status = wait_for_completion(&store, run_id, Duration::from_secs(5))
            .await
            .expect("run should have a status");
        assert_eq!(status.state, RunState::Succeeded);
    }

    #[tokio::test]
    async fn test_run_with_false_fails() {
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());

        // Use 'false' command which always fails
        let config = CliConfig::new("false").without_non_interactive();
        let adapter = CodexCliAdapter::with_config(config);

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Wait for the process to complete with polling (more reliable in CI)
        let status = wait_for_completion(&store, run_id, Duration::from_secs(5))
            .await
            .expect("run should have a status");
        assert_eq!(status.state, RunState::Failed);
    }

    #[tokio::test]
    async fn test_status_returns_run_status() {
        let _guard = env_guard().await;
        let _env_guards = set_cli_env(Some("codex"), None);
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let adapter = CodexCliAdapter::new();

        // Create a run directly in the store
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
        };
        let run_id = store.create_run(request).await.unwrap();

        let status = adapter.status(run_id, store.clone()).await.unwrap();
        assert!(status.is_some());
        assert_eq!(status.unwrap().state, RunState::Pending);
    }

    #[tokio::test]
    async fn test_stop_cancels_run() {
        let _guard = env_guard().await;
        let _env_guards = set_cli_env(Some("codex"), None);
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let adapter = CodexCliAdapter::new();

        // A run with no live process: only the store changes.
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
        };
        let run_id = store.create_run(request).await.unwrap();

        let stopped = adapter.stop(run_id, store.clone()).await.unwrap();
        assert!(stopped);

        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Canceled);
    }

    #[tokio::test]
    async fn test_history_returns_runs() {
        let _guard = env_guard().await;
        let _env_guards = set_cli_env(Some("codex"), None);
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let adapter = CodexCliAdapter::new();

        // Create some runs
        for i in 0..3 {
            let request = RunRequest {
                backend: BackendKind::Codex,
                prompt: format!("test {}", i),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            };
            store.create_run(request).await.unwrap();
        }

        let history = adapter.history(10, store.clone()).await.unwrap();
        assert_eq!(history.len(), 3);
    }

    #[tokio::test]
    async fn test_run_with_working_dir() {
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());

        // Use 'pwd' without arguments to test working directory
        let config = CliConfig::new("pwd")
            .with_working_dir(PathBuf::from("/tmp"))
            .without_non_interactive();
        let adapter = CodexCliAdapter::with_config(config);

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Wait for completion with polling (more reliable in CI)
        let status = wait_for_completion(&store, run_id, Duration::from_secs(5))
            .await
            .expect("run should have a status");
        assert_eq!(
            status.state,
            RunState::Succeeded,
            "pwd should succeed, got: {:?}",
            status
        );

        // Check for completion event
        let run = store.run(run_id).await.unwrap().unwrap();
        assert!(
            run.events.iter().any(|e| e.kind == "completion"),
            "should have completion event, got events: {:?}",
            run.events
        );

        // Verify output contains /tmp
        let completion = run
            .events
            .iter()
            .find(|e| e.kind == "completion")
            .and_then(|e| e.data.as_ref())
            .and_then(|d| d.get("text"))
            .and_then(|t| t.as_str());
        assert!(
            completion.is_some_and(|t| t.contains("tmp")),
            "completion should contain tmp: {:?}",
            completion
        );
    }

    #[tokio::test]
    async fn test_run_with_env_vars() {
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());

        let config = CliConfig {
            binary: "sh".to_string(),
            working_dir: None,
            env_vars: {
                let mut env = HashMap::new();
                env.insert("TEST_VAR".to_string(), "test_value".to_string());
                env
            },
            timeout: Duration::from_secs(10),
            non_interactive: false,
        };
        let adapter = CodexCliAdapter::with_config(config);

        // Verify the adapter can be configured with env vars
        assert_eq!(
            adapter.config.env_vars.get("TEST_VAR"),
            Some(&"test_value".to_string())
        );

        // Use store to avoid unused variable warning
        let _ = store.history(1).await;
    }

    #[tokio::test]
    async fn test_run_captures_stdout() {
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());

        // Use 'echo' to test stdout capture
        let config = CliConfig::new("echo").without_non_interactive();
        let adapter = CodexCliAdapter::with_config(config);

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Wait for the process to complete with polling (more reliable in CI)
        let status = wait_for_completion(&store, run_id, Duration::from_secs(5))
            .await
            .expect("run should have a status");
        assert_eq!(status.state, RunState::Succeeded);

        // `echo` with no arguments prints one empty line: one stream event
        // carrying it, and an empty completion text.
        let run = store.run(run_id).await.unwrap().unwrap();
        assert!(run.events.iter().any(|e| e.kind == "start"));
        let lines: Vec<&serde_json::Value> = run
            .events
            .iter()
            .filter(|e| e.kind == "stream")
            .filter_map(|e| e.data.as_ref())
            .collect();
        assert_eq!(lines, [&serde_json::json!({ "line": "" })]);
        let completion = run
            .events
            .iter()
            .find(|e| e.kind == "completion")
            .and_then(|e| e.data.as_ref())
            .expect("completion event");
        assert_eq!(completion["text"], "");
    }

    /// Child-process tests. Each one runs a throwaway `/bin/sh` script, never a
    /// real `claude` or `codex` binary.
    #[cfg(unix)]
    mod process_tests {
        use super::*;

        /// A run of `body` as a shell script. The adapter passes the prompt as
        /// `/bin/sh -- <prompt>`, so the prompt is the script path and the
        /// file is read by `sh` rather than exec'd, which avoids the ETXTBSY
        /// race of exec'ing a file another test thread may still hold open.
        /// The temp dir must outlive the run.
        struct Script {
            _dir: tempfile::TempDir,
            path: String,
        }

        fn script_config(body: &str) -> (Script, CliConfig) {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("fake-cli.sh");
            std::fs::write(&path, format!("{body}\n")).unwrap();
            let path = path.to_str().unwrap().to_string();
            (Script { _dir: dir, path }, CliConfig::new("/bin/sh"))
        }

        fn request_for(script: &Script) -> RunRequest {
            RunRequest {
                backend: BackendKind::Codex,
                prompt: script.path.clone(),
                template_id: None,
                output_schema: None,
                async_mode: true,
                tracing: false,
                model: None,
            }
        }

        fn process_alive(pid: u32) -> bool {
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }

        async fn wait_until_gone(pid: u32) -> bool {
            for _ in 0..100 {
                if !process_alive(pid) {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            false
        }

        /// Polls the run's `start` event for the child pid.
        async fn child_pid(store: &Arc<dyn RunStore>, run_id: RunId) -> u32 {
            for _ in 0..100 {
                if let Some(run) = store.run(run_id).await.unwrap() {
                    if let Some(pid) = run
                        .events
                        .iter()
                        .find(|e| e.kind == "start")
                        .and_then(|e| e.data.as_ref())
                        .and_then(|d| d.get("pid"))
                        .and_then(|p| p.as_u64())
                    {
                        return pid as u32;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("run never recorded a start event with a pid");
        }

        fn completion_text(run: &RunRecord) -> String {
            run.events
                .iter()
                .find(|e| e.kind == "completion")
                .and_then(|e| e.data.as_ref())
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or_default()
                .to_string()
        }

        /// RT-2 / RT-28: stop must kill the child, not only flip the status.
        #[tokio::test]
        async fn stop_kills_the_child_process() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("exec sleep 30");
            let adapter = CodexCliAdapter::with_config(config);

            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let pid = child_pid(&store, run_id).await;
            assert!(process_alive(pid));

            assert!(adapter.stop(run_id, store.clone()).await.unwrap());

            assert!(wait_until_gone(pid).await, "child {pid} survived stop");
            let status = store.status(run_id).await.unwrap().unwrap();
            assert_eq!(status.state, RunState::Canceled);
        }

        /// M255: a run stopped before its child was spawned never spawns it.
        #[tokio::test]
        async fn run_stopped_before_spawn_never_starts_the_child() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let marker_dir = tempfile::tempdir().unwrap();
            let marker = marker_dir.path().join("spawned");
            let (script, config) = script_config(&format!("touch '{}'", marker.display()));
            let adapter = CodexCliAdapter::with_config(config);
            let run_id = store.create_run(request_for(&script)).await.unwrap();
            assert!(store.stop(run_id).await.unwrap());

            adapter
                .execute_run(run_id, request_for(&script), store.clone())
                .await
                .unwrap();

            assert!(!marker.exists(), "a stopped run spawned its child");
            let run = store.run(run_id).await.unwrap().unwrap();
            assert_eq!(run.status.state, RunState::Canceled);
            assert!(run.events.iter().all(|e| e.kind != "start"));
        }

        /// M252: one huge newline-free line is cut to MAX_STREAM_LINE in its
        /// stream event and marked truncated, not held whole.
        #[tokio::test]
        async fn oversized_stdout_line_is_capped_in_its_stream_event() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) =
                script_config("head -c 2000000 /dev/zero | tr '\\0' a; echo; echo done");
            let adapter =
                CodexCliAdapter::with_config(config.with_timeout(Duration::from_secs(20)));
            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let status = wait_for_completion(&store, run_id, Duration::from_secs(25))
                .await
                .unwrap();
            assert_eq!(status.state, RunState::Succeeded, "{status:?}");

            let run = store.run(run_id).await.unwrap().unwrap();
            let streams: Vec<&serde_json::Value> = run
                .events
                .iter()
                .filter(|e| e.kind == "stream")
                .filter_map(|e| e.data.as_ref())
                .collect();
            let long = streams[0];
            assert_eq!(long["line"].as_str().unwrap().len(), MAX_STREAM_LINE);
            assert_eq!(long["truncated"], true);
            let last = streams.last().unwrap();
            assert_eq!(last["line"], "done");
            assert!(last.get("truncated").is_none());
        }

        /// M313: output past the capture cap is reported as truncated.
        #[tokio::test]
        async fn completion_over_the_capture_cap_is_marked_truncated() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("yes aaaaaaa | head -c 1200000");
            let adapter =
                CodexCliAdapter::with_config(config.with_timeout(Duration::from_secs(30)));
            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let status = wait_for_completion(&store, run_id, Duration::from_secs(35))
                .await
                .unwrap();
            assert_eq!(status.state, RunState::Succeeded, "{status:?}");
            assert!(
                status
                    .message
                    .as_deref()
                    .unwrap_or("")
                    .contains("truncated"),
                "{status:?}"
            );
            let run = store.run(run_id).await.unwrap().unwrap();
            let completion = run
                .events
                .iter()
                .find(|e| e.kind == "completion")
                .and_then(|e| e.data.clone())
                .unwrap();
            assert_eq!(completion["truncated"], true);
        }

        /// M253: a background process that inherits stdout must not turn a
        /// finished run into a timeout.
        #[tokio::test]
        async fn grandchild_holding_stdout_does_not_time_out_the_run() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("(sleep 8 &); echo hi");
            let adapter = CodexCliAdapter::with_config(config.with_timeout(Duration::from_secs(5)));
            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let status = wait_for_completion(&store, run_id, Duration::from_secs(10))
                .await
                .unwrap();

            assert_eq!(status.state, RunState::Succeeded, "{status:?}");
            let run = store.run(run_id).await.unwrap().unwrap();
            assert_eq!(completion_text(&run), "hi");
        }

        /// M254: stop kills the child's whole process group, not only the
        /// direct child, so tool subprocesses do not outlive the run.
        #[tokio::test]
        async fn stop_kills_the_childs_background_processes() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("sleep 30 & echo $!; wait");
            let adapter = CodexCliAdapter::with_config(config);
            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let mut grandchild = None;
            for _ in 0..100 {
                let run = store.run(run_id).await.unwrap().unwrap();
                grandchild = run
                    .events
                    .iter()
                    .find(|e| e.kind == "stream")
                    .and_then(|e| e.data.as_ref()?.get("line")?.as_str()?.parse::<u32>().ok());
                if grandchild.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let grandchild = grandchild.expect("script printed its background pid");
            assert!(process_alive(grandchild));

            assert!(adapter.stop(run_id, store.clone()).await.unwrap());

            assert!(
                wait_until_gone(grandchild).await,
                "background process {grandchild} survived stop"
            );
        }

        /// M259: a run past the concurrent-child limit is refused rather than
        /// spawning another CLI process; the slot frees when a run ends.
        #[tokio::test]
        async fn run_past_the_child_limit_is_refused_until_a_slot_frees() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("exec sleep 30");
            let adapter = CodexCliAdapter::with_shared_state(
                config,
                Arc::new(Mutex::new(HashMap::new())),
                Arc::new(tokio::sync::Semaphore::new(1)),
            );
            let first = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            child_pid(&store, first).await;

            let err = adapter
                .run(request_for(&script), store.clone())
                .await
                .expect_err("second concurrent run must be refused");
            assert!(err.to_string().contains("concurrent"), "{err}");

            assert!(adapter.stop(first, store.clone()).await.unwrap());
            let mut freed = false;
            for _ in 0..100 {
                if let Ok(run_id) = adapter.run(request_for(&script), store.clone()).await {
                    adapter.stop(run_id, store.clone()).await.unwrap();
                    freed = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(freed, "the slot never freed after the first run ended");
        }

        /// M256: a known CLI gets its prompt on stdin, not in argv.
        #[tokio::test]
        async fn known_cli_receives_the_prompt_on_stdin() {
            use std::os::unix::fs::PermissionsExt as _;
            let dir = tempfile::tempdir().unwrap();
            let fake = dir.path().join("claude");
            std::fs::write(&fake, "#!/bin/sh\necho \"args:$*\"\ncat\n").unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
            let adapter = CodexCliAdapter::with_config(CliConfig::new(fake.to_str().unwrap()));
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let mut request = RunRequest {
                backend: BackendKind::Claude,
                prompt: "secret prompt text".into(),
                template_id: None,
                output_schema: None,
                async_mode: true,
                tracing: false,
                model: None,
            };
            request.prompt.push_str(" end");
            let run_id = adapter.run(request, store.clone()).await.unwrap();
            let status = wait_for_completion(&store, run_id, Duration::from_secs(10))
                .await
                .unwrap();
            assert_eq!(status.state, RunState::Succeeded, "{status:?}");
            let run = store.run(run_id).await.unwrap().unwrap();
            assert_eq!(
                completion_text(&run),
                "args:--print\nsecret prompt text end"
            );
        }

        /// RT-2: an adapter built later, sharing the process map, reaches a
        /// child spawned by an earlier adapter.
        #[tokio::test]
        async fn stop_through_a_second_adapter_sharing_the_map_kills_the_child() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let processes: CliProcessMap = Arc::new(Mutex::new(HashMap::new()));
            let (script, config) = script_config("exec sleep 30");
            let first = CodexCliAdapter::with_shared_processes(config.clone(), processes.clone());
            let run_id = first
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let pid = child_pid(&store, run_id).await;

            let second = CodexCliAdapter::with_shared_processes(config, processes);
            assert!(second.stop(run_id, store.clone()).await.unwrap());
            assert!(wait_until_gone(pid).await, "child {pid} survived stop");
        }

        /// RT-3: the configured timeout kills a hung child and fails the run.
        #[tokio::test]
        async fn timeout_kills_a_hung_child_and_fails_the_run() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("exec sleep 30");
            let adapter =
                CodexCliAdapter::with_config(config.with_timeout(Duration::from_millis(300)));

            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let pid = child_pid(&store, run_id).await;
            let status = wait_for_completion(&store, run_id, Duration::from_secs(10))
                .await
                .unwrap();

            assert_eq!(status.state, RunState::Failed);
            assert!(
                status
                    .message
                    .as_deref()
                    .unwrap_or("")
                    .contains("timed out"),
                "{status:?}"
            );
            assert!(
                wait_until_gone(pid).await,
                "child {pid} survived the timeout"
            );
        }

        /// RT-4: a child that fills the stderr pipe before closing stdout must
        /// not deadlock the reader.
        #[tokio::test]
        async fn large_stderr_before_stdout_does_not_deadlock() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            // 256 KiB of stderr, well past a 64 KiB pipe buffer, then stdout.
            let (script, config) = script_config(
                "i=0; while [ $i -lt 4096 ]; do \
                 echo 0123456789012345678901234567890123456789012345678901234567890123 >&2; \
                 i=$((i+1)); done; echo finished",
            );
            let adapter =
                CodexCliAdapter::with_config(config.with_timeout(Duration::from_secs(20)));

            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let status = wait_for_completion(&store, run_id, Duration::from_secs(25))
                .await
                .unwrap();

            assert_eq!(status.state, RunState::Succeeded, "{status:?}");
            let run = store.run(run_id).await.unwrap().unwrap();
            assert_eq!(completion_text(&run), "finished");
        }

        /// RT-5: the child must not share the server's stdin, which carries the
        /// JSON-RPC stream on stdio transport.
        #[cfg(target_os = "linux")]
        #[tokio::test]
        async fn child_stdin_is_null() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("readlink /proc/self/fd/0");
            let adapter = CodexCliAdapter::with_config(config);

            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            wait_for_completion(&store, run_id, Duration::from_secs(10)).await;
            let run = store.run(run_id).await.unwrap().unwrap();
            assert_eq!(completion_text(&run), "/dev/null");
        }

        /// RT-11: one invalid UTF-8 sequence must not end the read early.
        #[tokio::test]
        async fn invalid_utf8_does_not_truncate_output() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("printf 'a\\377b\\nsecond line\\n'");
            let adapter = CodexCliAdapter::with_config(config);

            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let status = wait_for_completion(&store, run_id, Duration::from_secs(10))
                .await
                .unwrap();
            assert_eq!(status.state, RunState::Succeeded);
            let run = store.run(run_id).await.unwrap().unwrap();
            let text = completion_text(&run);
            assert!(text.contains("second line"), "output was cut: {text:?}");
            assert!(text.starts_with('a'), "{text:?}");
        }

        /// RT-12: server secrets must not reach the child's environment.
        #[tokio::test]
        async fn server_secrets_are_not_inherited() {
            let _guard = env_guard().await;
            let _secret = skrills_test_utils::set_env_var("SKRILLS_CODEX_API_KEY", Some("leak"));
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) =
                script_config("echo \"key=${SKRILLS_CODEX_API_KEY:-unset} path=${PATH:+set}\"");
            let adapter = CodexCliAdapter::with_config(config.with_env("EXPLICIT", "yes"));

            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            wait_for_completion(&store, run_id, Duration::from_secs(10)).await;
            let run = store.run(run_id).await.unwrap().unwrap();
            assert_eq!(completion_text(&run), "key=unset path=set");
        }

        /// M324: credentials the `claude` and `codex` CLIs document reach
        /// the child.
        #[tokio::test]
        async fn cli_auth_variables_are_inherited() {
            let _guard = env_guard().await;
            let _oauth = skrills_test_utils::set_env_var("CLAUDE_CODE_OAUTH_TOKEN", Some("oauth"));
            let _bedrock = skrills_test_utils::set_env_var("CLAUDE_CODE_USE_BEDROCK", Some("1"));
            let _codex = skrills_test_utils::set_env_var("CODEX_API_KEY", Some("ck"));
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config(
                "echo \"${CLAUDE_CODE_OAUTH_TOKEN:-unset} ${CLAUDE_CODE_USE_BEDROCK:-unset} ${CODEX_API_KEY:-unset}\"",
            );
            let adapter = CodexCliAdapter::with_config(config);
            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            wait_for_completion(&store, run_id, Duration::from_secs(10)).await;
            let run = store.run(run_id).await.unwrap().unwrap();
            assert_eq!(completion_text(&run), "oauth 1 ck");
        }

        /// A store whose `stream` writes fail, to drive the error path.
        struct FailingStreamStore(MemRunStore);

        #[async_trait]
        impl RunStore for FailingStreamStore {
            async fn create_run(&self, request: RunRequest) -> Result<RunId> {
                self.0.create_run(request).await
            }
            async fn update_status(&self, run_id: RunId, status: RunStatus) -> Result<()> {
                self.0.update_status(run_id, status).await
            }
            async fn append_event(&self, run_id: RunId, event: RunEvent) -> Result<()> {
                if event.kind == "stream" {
                    anyhow::bail!("disk full");
                }
                self.0.append_event(run_id, event).await
            }
            async fn run(&self, run_id: RunId) -> Result<Option<RunRecord>> {
                self.0.run(run_id).await
            }
            async fn status(&self, run_id: RunId) -> Result<Option<RunStatus>> {
                self.0.status(run_id).await
            }
            async fn history(&self, limit: usize) -> Result<Vec<RunRecord>> {
                self.0.history(limit).await
            }
            async fn stop(&self, run_id: RunId) -> Result<bool> {
                self.0.stop(run_id).await
            }
        }

        /// RT-10: a store error while streaming must not orphan the child.
        #[tokio::test]
        async fn store_error_while_streaming_kills_the_child() {
            let store: Arc<dyn RunStore> = Arc::new(FailingStreamStore(MemRunStore::new()));
            let (script, config) = script_config("echo first; exec sleep 30");
            let adapter = CodexCliAdapter::with_config(config);

            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let pid = child_pid(&store, run_id).await;
            let status = wait_for_completion(&store, run_id, Duration::from_secs(10))
                .await
                .unwrap();

            assert_eq!(status.state, RunState::Failed);
            assert!(wait_until_gone(pid).await, "child {pid} was orphaned");
        }

        /// RT-9: stop must not wait behind a child that closed its pipes but
        /// keeps running.
        #[tokio::test]
        async fn stop_does_not_block_behind_a_child_with_closed_pipes() {
            let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
            let (script, config) = script_config("exec sleep 30 >/dev/null 2>&1");
            let adapter = CodexCliAdapter::with_config(config);

            let run_id = adapter
                .run(request_for(&script), store.clone())
                .await
                .unwrap();
            let pid = child_pid(&store, run_id).await;
            // Let the reader reach EOF and start waiting on the child.
            tokio::time::sleep(Duration::from_millis(200)).await;

            tokio::time::timeout(Duration::from_secs(2), adapter.stop(run_id, store.clone()))
                .await
                .expect("stop blocked on the process map")
                .unwrap();
            assert!(wait_until_gone(pid).await, "child {pid} survived stop");
        }

        /// RT-33: captured stdout is capped.
        #[test]
        fn push_capped_stops_at_the_cap_on_a_char_boundary() {
            let mut buf = String::new();
            let mut truncated = false;
            push_capped(&mut buf, "ab", 3, &mut truncated);
            push_capped(&mut buf, "é", 3, &mut truncated);
            assert_eq!(buf, "ab");
            assert!(truncated);
        }
    }

    // CliError tests
    mod cli_error_tests {
        use super::*;

        #[test]
        fn test_spawn_failed_error_display() {
            let err = CliError::SpawnFailed {
                binary: "nonexistent".to_string(),
                source: io::Error::new(io::ErrorKind::NotFound, "No such file or directory"),
            };
            let msg = err.to_string();
            assert!(msg.contains("nonexistent"));
            assert!(msg.contains("failed to spawn"));
        }

        #[test]
        fn test_process_failed_error_display() {
            let err = CliError::ProcessFailed {
                exit_code: Some(1),
                stderr: "command failed".to_string(),
            };
            let msg = err.to_string();
            assert!(msg.contains("1"));
            assert!(msg.contains("command failed"));
        }

        #[test]
        fn test_process_failed_error_no_exit_code() {
            let err = CliError::ProcessFailed {
                exit_code: None,
                stderr: "killed by signal".to_string(),
            };
            let msg = err.to_string();
            assert!(msg.contains("None"));
            assert!(msg.contains("killed by signal"));
        }

        #[test]
        fn test_wait_failed_error_display() {
            let err = CliError::WaitFailed(io::Error::new(
                io::ErrorKind::Interrupted,
                "wait interrupted",
            ));
            let msg = err.to_string();
            assert!(msg.contains("failed to wait"));
        }

        #[test]
        fn test_cli_error_is_std_error() {
            // Verify CliError implements std::error::Error
            fn assert_error<E: std::error::Error>() {}
            assert_error::<CliError>();
        }

        #[test]
        fn test_cli_error_source_chain() {
            use std::error::Error;

            let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
            let err = CliError::SpawnFailed {
                binary: "test".to_string(),
                source: io_err,
            };

            // The error should have a source
            assert!(err.source().is_some());
        }

        #[test]
        fn test_cli_error_converts_to_anyhow() {
            let err = CliError::SpawnFailed {
                binary: "test".to_string(),
                source: io::Error::new(io::ErrorKind::NotFound, "not found"),
            };

            // Should be convertible to anyhow::Error
            let anyhow_err: anyhow::Error = err.into();
            assert!(anyhow_err.to_string().contains("test"));
        }
    }
}
