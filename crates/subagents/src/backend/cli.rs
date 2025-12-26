//! CLI-based backend adapter for subprocess execution.
//!
//! This adapter spawns CLI tools (like `codex` or `claude`) as subprocesses
//! to execute agent prompts with tool capabilities.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use thiserror::Error;
use time::OffsetDateTime;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::backend::{AdapterCapabilities, BackendAdapter};
use crate::cli_detection::{default_cli_binary, normalize_cli_binary};
use crate::store::{
    BackendKind, RunEvent, RunId, RunRecord, RunRequest, RunState, RunStatus, RunStore,
    SubagentTemplate,
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
        let timeout_ms = match std::env::var("SKRILLS_CLI_TIMEOUT_MS") {
            Ok(v) => match v.parse::<u64>() {
                Ok(ms) => ms,
                Err(_) => {
                    tracing::warn!(
                        value = %v,
                        default = DEFAULT_TIMEOUT_MS,
                        "Invalid SKRILLS_CLI_TIMEOUT_MS value, using default"
                    );
                    DEFAULT_TIMEOUT_MS
                }
            },
            Err(_) => DEFAULT_TIMEOUT_MS,
        };

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

/// Tracks a running CLI subprocess.
struct CliProcess {
    child: Child,
}

/// CLI-based adapter that spawns subprocesses for agent execution.
///
/// This adapter is designed for agents that require tool capabilities,
/// spawning CLI tools like `codex` or `claude` as subprocesses.
pub struct CodexCliAdapter {
    config: CliConfig,
    /// Active processes indexed by run_id.
    processes: Arc<Mutex<HashMap<RunId, CliProcess>>>,
}

impl CodexCliAdapter {
    /// Create a new CLI adapter with default configuration.
    pub fn new() -> Self {
        Self::with_config(CliConfig::default())
    }

    /// Create a CLI adapter with the specified configuration.
    pub fn with_config(config: CliConfig) -> Self {
        Self {
            config,
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a CLI adapter from environment variables.
    pub fn from_env() -> Self {
        Self::with_config(CliConfig::from_env())
    }

    /// Build the command arguments for the CLI.
    fn build_command_args(&self, prompt: &str) -> Vec<String> {
        let mut args = Vec::new();

        // Only add prompt arguments if non_interactive is enabled
        // (real CLI tools like codex/claude need these flags)
        if self.config.non_interactive {
            args.push("--prompt".to_string());
            args.push(prompt.to_string());

            // Different CLIs have different flags
            if self.config.binary.contains("codex") {
                args.push("--non-interactive".to_string());
            } else if self.config.binary.contains("claude") {
                args.push("--print".to_string());
            }
        }

        args
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

        // Record start event
        store
            .append_event(
                run_id,
                RunEvent {
                    ts: OffsetDateTime::now_utc(),
                    kind: "start".into(),
                    data: Some(json!({
                        "binary": self.config.binary,
                        "working_dir": self.config.working_dir,
                    })),
                },
            )
            .await?;

        // Build the command
        let mut cmd = Command::new(&self.config.binary);
        let args = self.build_command_args(&request.prompt);
        cmd.args(&args);

        // Set working directory if configured
        if let Some(ref dir) = self.config.working_dir {
            cmd.current_dir(dir);
        }

        // Set environment variables
        for (key, value) in &self.config.env_vars {
            cmd.env(key, value);
        }

        // Capture stdout and stderr
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Spawn the process
        tracing::debug!(
            run_id = %run_id,
            args = ?args,
            working_dir = ?self.config.working_dir,
            "Spawning CLI process"
        );
        let mut child = cmd.spawn().map_err(|e| CliError::SpawnFailed {
            binary: self.config.binary.clone(),
            source: e,
        })?;

        // Get stdout handle before storing the child
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Store the process handle for potential cancellation
        {
            let mut processes = self.processes.lock().await;
            processes.insert(run_id, CliProcess { child });
        }

        // Create output accumulator
        let mut output = String::new();
        let mut error_output = String::new();

        // Read stdout line by line, emitting stream events
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                output.push_str(&line);
                output.push('\n');

                store
                    .append_event(
                        run_id,
                        RunEvent {
                            ts: OffsetDateTime::now_utc(),
                            kind: "stream".into(),
                            data: Some(json!({ "line": line })),
                        },
                    )
                    .await?;
            }
        }

        // Capture stderr
        if let Some(stderr) = stderr {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                error_output.push_str(&line);
                error_output.push('\n');
            }
        }

        // Wait for process to complete - get child back from map
        let status = {
            let mut processes = self.processes.lock().await;
            if let Some(mut process) = processes.remove(&run_id) {
                process.child.wait().await.map_err(CliError::WaitFailed)?
            } else {
                // Process was already removed (e.g., by stop())
                tracing::debug!(
                    run_id = %run_id,
                    "Process already removed from tracking - likely stopped by user"
                );
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
                        data: Some(json!({ "text": output.trim() })),
                    },
                )
                .await?;

            store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Succeeded,
                        message: Some("completed".into()),
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
}

impl Default for CodexCliAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BackendAdapter for CodexCliAdapter {
    fn backend(&self) -> BackendKind {
        // Use Codex kind since this is primarily for codex CLI
        // Could also use BackendKind::Other("cli".into()) for more generic use
        BackendKind::Codex
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
            backend: BackendKind::Codex,
            capabilities: vec!["tools".into(), "subprocess".into()],
        }])
    }

    async fn run(&self, request: RunRequest, store: Arc<dyn RunStore>) -> Result<RunId> {
        // Create the run record
        let run_id = store.create_run(request.clone()).await?;

        // Update status to Running
        store
            .update_status(
                run_id,
                RunStatus {
                    state: RunState::Running,
                    message: Some("spawning CLI process".into()),
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await?;

        // Clone self for the spawned task
        let config = self.config.clone();
        let processes = self.processes.clone();
        let adapter = CodexCliAdapter { config, processes };

        // Spawn the execution in a background task
        let store_clone = store.clone();
        tokio::spawn(async move {
            if let Err(err) = adapter
                .execute_run(run_id, request, store_clone.clone())
                .await
            {
                tracing::error!("CLI execution failed: {}", err);
                if let Err(store_err) = store_clone
                    .append_event(
                        run_id,
                        RunEvent {
                            ts: OffsetDateTime::now_utc(),
                            kind: "error".into(),
                            data: Some(json!({"message": err.to_string()})),
                        },
                    )
                    .await
                {
                    tracing::error!(
                        run_id = %run_id,
                        original_error = %err,
                        store_error = %store_err,
                        "Failed to record error event in store - error details may be lost"
                    );
                }
                if let Err(store_err) = store_clone
                    .update_status(
                        run_id,
                        RunStatus {
                            state: RunState::Failed,
                            message: Some(err.to_string()),
                            updated_at: OffsetDateTime::now_utc(),
                        },
                    )
                    .await
                {
                    tracing::error!(
                        run_id = %run_id,
                        store_error = %store_err,
                        "Failed to update run status to Failed - run may appear stuck"
                    );
                }
            }
        });

        Ok(run_id)
    }

    async fn status(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<Option<RunStatus>> {
        store.status(run_id).await
    }

    async fn stop(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<bool> {
        // Try to kill the subprocess if it's still running
        {
            let mut processes = self.processes.lock().await;
            if let Some(mut process) = processes.remove(&run_id) {
                // Attempt to kill the process
                if let Err(kill_err) = process.child.kill().await {
                    tracing::warn!(
                        run_id = %run_id,
                        error = %kill_err,
                        "Failed to kill subprocess - process may still be running"
                    );
                }
            }
        }

        // Update the store
        store.stop(run_id).await
    }

    async fn history(&self, limit: usize, store: Arc<dyn RunStore>) -> Result<Vec<RunStatus>> {
        let runs: Vec<RunRecord> = store.history(limit).await?;
        Ok(runs.into_iter().map(|r| r.status).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemRunStore;
    use std::env;
    use std::sync::LazyLock;
    use tokio::sync::Mutex;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    async fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }

    fn env_guard_blocking() -> tokio::sync::MutexGuard<'static, ()> {
        ENV_LOCK.blocking_lock()
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.previous {
                env::set_var(self.key, v);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = env::var(key).ok();
        if let Some(v) = value {
            env::set_var(key, v);
        } else {
            env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

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

        assert!(args.contains(&"--prompt".to_string()));
        assert!(args.contains(&"test prompt".to_string()));
        assert!(args.contains(&"--non-interactive".to_string()));
    }

    #[test]
    fn test_build_command_args_claude() {
        let config = CliConfig::new("claude");
        let adapter = CodexCliAdapter::with_config(config);
        let args = adapter.build_command_args("test prompt");

        assert!(args.contains(&"--prompt".to_string()));
        assert!(args.contains(&"test prompt".to_string()));
        assert!(args.contains(&"--print".to_string()));
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
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Wait for the process to complete
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = store.status(run_id).await.unwrap().unwrap();
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
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Wait for the process to complete
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = store.status(run_id).await.unwrap().unwrap();
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

        // Create a run in the store
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };
        let run_id = store.create_run(request).await.unwrap();

        // Stop the run
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
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Wait for completion
        tokio::time::sleep(Duration::from_millis(200)).await;

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
        };

        let run_id = adapter.run(request, store.clone()).await.unwrap();

        // Wait for the process to complete
        tokio::time::sleep(Duration::from_millis(200)).await;

        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Succeeded);

        // Check that we have events
        let run = store.run(run_id).await.unwrap().unwrap();
        assert!(run.events.iter().any(|e| e.kind == "start"));
        assert!(run.events.iter().any(|e| e.kind == "completion"));
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
