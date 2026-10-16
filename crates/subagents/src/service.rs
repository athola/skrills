use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rmcp::model::{CallToolResult, ContentBlock, Tool};
use serde_json::{json, Map as JsonMap, Value};

use crate::backend::{abort_run_task, BackendAdapter};
use crate::backend::{
    claude::ClaudeAdapter,
    cli::{cancel_cli_process, CliConfig, CliProcessMap, CodexCliAdapter, MAX_CONCURRENT_CLI_RUNS},
    codex::CodexAdapter,
};
use crate::cli_detection::{
    cli_binary_from_client_env, cli_binary_from_exe_path, normalize_cli_binary, DEFAULT_CLI_BINARY,
};
use crate::registry::AgentRegistry;
use crate::settings::{backend_from_str, load_file_config, ExecutionMode, SubagentsFileConfig};
use crate::store::{
    default_store_path, BackendKind, RunId, RunRequest, RunStatus, RunStore, StateRunStore,
};
use crate::tool_schemas;

/// Builds a successful tool result with text content and a structured payload.
///
/// rmcp marks `CallToolResult` `#[non_exhaustive]`, so it can only be built
/// through its constructors, and none of them takes structured content
/// alongside caller-chosen text content. The fields stay public, so setting
/// `structured_content` after construction is the supported route.
///
/// This duplicates `skrills-server`'s `mcp_result` helpers because
/// `skrills-subagents` sits below that crate in the dependency graph.
fn tool_ok(content: Vec<ContentBlock>, structured_content: Option<Value>) -> CallToolResult {
    let mut result = CallToolResult::success(content);
    result.structured_content = structured_content;
    result
}

/// Builds a tool-level error result with text content and a structured payload.
fn tool_err(content: Vec<ContentBlock>, structured_content: Option<Value>) -> CallToolResult {
    let mut result = CallToolResult::error(content);
    result.structured_content = structured_content;
    result
}

/// Model an API-mode Claude run asks for unless `SKRILLS_CLAUDE_MODEL` or an
/// agent's model says otherwise. `claude-code` was not a Messages API model.
const DEFAULT_CLAUDE_API_MODEL: &str = "claude-opus-5-5";

/// Model an API-mode Codex run asks for. The adapter calls
/// `chat/completions`, which `gpt-5-codex` does not serve (Responses only).
const DEFAULT_CODEX_API_MODEL: &str = "gpt-6-astra";

/// Runs `get-run-history` returns when no `limit` is given.
const DEFAULT_HISTORY_LIMIT: usize = 20;

/// Largest `get-run-history` limit; matches the tool schema.
const MAX_HISTORY_LIMIT: usize = 50;

/// Most events one `get-run-events` call returns; matches the tool schema.
const MAX_EVENTS_PAGE: usize = 1_000;

/// Largest `timeout_ms` a caller may ask for; matches the tool schema.
const MAX_TIMEOUT_MS: u64 = 300_000;

/// Timeout a synchronous run waits for when the caller gave none. Matches the
/// CLI adapter's default and covers the API adapters' 120 s default.
const DEFAULT_RUN_TIMEOUT_MS: u64 = 300_000;

/// Extra time a synchronous call waits beyond the run's own timeout, so the
/// adapter's timeout, not this wait, decides the outcome.
const SYNC_WAIT_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// Prepends an agent's instructions to the caller's prompt.
///
/// A run request carries one prompt, so a named agent's system prompt travels
/// in it; without this, running an agent executed a bare prompt.
fn compose_agent_prompt(agent_instructions: &str, prompt: &str) -> String {
    let instructions = agent_instructions.trim();
    if instructions.is_empty() {
        prompt.to_string()
    } else {
        format!("{instructions}\n\n---\n\n{prompt}")
    }
}

/// The model id an API-mode agent run asks for, from the agent's `model`.
///
/// Claude Code agent files name models by alias (`opus`, `sonnet`, `haiku`)
/// or `inherit`; those are not API model ids, so they leave the adapter's
/// configured model in place. Any other value is passed through.
fn api_model_for_agent(model: Option<&str>) -> Option<String> {
    let model = model?.trim();
    let is_alias = ["opus", "sonnet", "haiku", "inherit"]
        .iter()
        .any(|alias| model.eq_ignore_ascii_case(alias));
    (!model.is_empty() && !is_alias).then(|| model.to_string())
}

/// An API adapter's HTTP timeout is fixed when the service starts, so a
/// per-call `timeout_ms` cannot reach it; accepting one left the call waiting
/// past it on the adapter's own timeout.
fn api_timeout_error() -> anyhow::Error {
    anyhow!(
        "timeout_ms applies only to CLI runs; an API run uses SKRILLS_CODEX_TIMEOUT_MS \
         or SKRILLS_CLAUDE_TIMEOUT_MS"
    )
}

/// The tool error every run-addressed tool returns for an unknown run.
fn run_not_found(run_id: RunId) -> CallToolResult {
    let message = format!("run not found: {run_id}");
    tool_err(
        vec![ContentBlock::text(message.clone())],
        Some(json!({"error": message, "run_id": run_id.to_string()})),
    )
}

fn run_id_from_value(val: &Value) -> Result<RunId> {
    let s = val
        .as_str()
        .ok_or_else(|| anyhow!("run_id must be a string"))?;
    let uuid = uuid::Uuid::parse_str(s).map_err(|e| anyhow!("invalid run_id: {e}"))?;
    Ok(RunId(uuid))
}

pub struct SubagentService {
    store: Arc<dyn RunStore>,
    adapters: HashMap<BackendKind, Arc<dyn BackendAdapter>>,
    /// Backend an API-mode run uses when neither the call nor an agent's
    /// model names one.
    default_backend: BackendKind,
    /// `default_backend` from the subagents config file, if set. Unlike the
    /// constructor's fallback it also picks the CLI binary.
    configured_backend: Option<BackendKind>,
    default_execution_mode: ExecutionMode,
    cli_binary: Option<String>,
    registry: Arc<AgentRegistry>,
    /// Cancel signals for every CLI child this service spawned, so `stop-run`
    /// reaches a child started by an earlier call.
    cli_processes: CliProcessMap,
    /// Permits for live CLI children, shared by every CLI adapter built.
    cli_slots: Arc<tokio::sync::Semaphore>,
}

impl SubagentService {
    pub fn new() -> Result<Self> {
        let store = Arc::new(StateRunStore::new(default_store_path()?)?);
        let file_config = load_file_config();
        let default_backend = std::env::var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND")
            .ok()
            .as_deref()
            .map(backend_from_str)
            .or_else(|| file_config.default_backend.as_deref().map(backend_from_str))
            .unwrap_or(BackendKind::Codex);
        let registry = Arc::new(AgentRegistry::discover()?);
        Self::with_store_and_registry_with_config(store, default_backend, registry, file_config)
    }

    pub fn with_store(store: Arc<dyn RunStore>, default_backend: BackendKind) -> Result<Self> {
        let registry = Arc::new(AgentRegistry::discover()?);
        Self::with_store_and_registry(store, default_backend, registry)
    }

    pub fn with_store_and_registry(
        store: Arc<dyn RunStore>,
        default_backend: BackendKind,
        registry: Arc<AgentRegistry>,
    ) -> Result<Self> {
        let file_config = load_file_config();
        Self::with_store_and_registry_with_config(store, default_backend, registry, file_config)
    }

    fn with_store_and_registry_with_config(
        store: Arc<dyn RunStore>,
        default_backend: BackendKind,
        registry: Arc<AgentRegistry>,
        file_config: SubagentsFileConfig,
    ) -> Result<Self> {
        let mut adapters: HashMap<BackendKind, Arc<dyn BackendAdapter>> = HashMap::new();
        adapters.insert(
            BackendKind::Codex,
            Arc::new(CodexAdapter::new(DEFAULT_CODEX_API_MODEL.into())?),
        );
        adapters.insert(
            BackendKind::Claude,
            Arc::new(ClaudeAdapter::new(DEFAULT_CLAUDE_API_MODEL.into())?),
        );

        let default_execution_mode = match file_config.execution_mode.as_deref() {
            None => ExecutionMode::default(),
            Some(raw) => raw.parse().unwrap_or_else(|_| {
                tracing::warn!(
                    value = %raw,
                    "invalid execution_mode in the subagents config (expected 'cli' or 'api'); using the default"
                );
                ExecutionMode::default()
            }),
        };
        let cli_binary = normalize_cli_binary(file_config.cli_binary);
        let configured_backend = file_config.default_backend.as_deref().map(backend_from_str);

        Ok(Self {
            store,
            adapters,
            default_backend,
            configured_backend,
            default_execution_mode,
            cli_binary,
            registry,
            cli_processes: Default::default(),
            cli_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CLI_RUNS)),
        })
    }

    fn adapter_for(&self, backend: Option<BackendKind>) -> Result<Arc<dyn BackendAdapter>> {
        let key = backend.unwrap_or_else(|| self.default_backend.clone());
        self.adapters
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow!("backend not configured: {key:?}"))
    }

    /// Constructs a CLI adapter with the appropriate binary selection.
    ///
    /// Binary selection precedence (highest to lowest):
    /// 1. Explicit `cli_binary_override` parameter
    /// 2. `SKRILLS_CLI_BINARY` environment variable (handled by `CliConfig::from_env()`)
    /// 3. Backend hint (`Codex` -> "codex", `Claude` -> "claude"): the
    ///    call's `backend` argument or an agent's model
    /// 4. [`Self::default_cli_binary`]: `cli_binary` from the subagents
    ///    config, then a configured default backend, then client detection
    ///
    /// Pass `None` as the hint when the call named no backend: the service's
    /// API fallback backend must not pick the CLI ahead of the operator's
    /// configuration and the detected client.
    fn cli_adapter_for(
        &self,
        cli_binary_override: Option<String>,
        backend_hint: Option<BackendKind>,
        timeout: Option<std::time::Duration>,
    ) -> Arc<CodexCliAdapter> {
        let mut config = CliConfig::from_env();
        if let Some(timeout) = timeout {
            config.timeout = timeout;
        }

        // Only apply backend hint if env var is not set (from_env handles env var internally,
        // but we check here to determine if we should override with backend hint)
        let env_binary = normalize_cli_binary(std::env::var("SKRILLS_CLI_BINARY").ok());
        if env_binary.is_none() {
            config.binary = match backend_hint {
                Some(BackendKind::Codex) => "codex".into(),
                Some(BackendKind::Claude) => "claude".into(),
                Some(BackendKind::Other(ref name)) if name.eq_ignore_ascii_case("copilot") => {
                    tracing::warn!(
                        "Copilot CLI does not support subagent execution; using default binary"
                    );
                    self.default_cli_binary()
                }
                Some(BackendKind::Other(_)) | None => self.default_cli_binary(),
            };
        }

        // Explicit override takes highest precedence
        if let Some(binary) = cli_binary_override {
            config.binary = binary;
        }

        Arc::new(CodexCliAdapter::with_shared_state(
            config,
            self.cli_processes.clone(),
            self.cli_slots.clone(),
        ))
    }

    /// Accepts a per-call `cli_binary` only when it names a known CLI by bare
    /// name or matches the binary the operator configured.
    ///
    /// The argument comes from the model calling the tool, so an arbitrary
    /// path here would let a prompt-injected model run any program.
    fn checked_cli_binary(&self, requested: &str) -> Result<String> {
        let requested = requested.trim();
        // Exact names only: matching the file stem let `claude.sh` or
        // `CODEX.bat` through, which resolve to other programs on PATH.
        let known_names: &[&str] = if cfg!(windows) {
            &["claude", "codex", "claude.exe", "codex.exe"]
        } else {
            &["claude", "codex"]
        };
        let bare_known = known_names.contains(&requested);
        let operator_configured = normalize_cli_binary(std::env::var("SKRILLS_CLI_BINARY").ok())
            .into_iter()
            .chain(self.cli_binary.clone())
            .any(|configured| configured == requested);
        if bare_known || operator_configured {
            Ok(requested.to_string())
        } else {
            Err(anyhow!(
                "cli_binary {requested:?} is not allowed: use \"claude\" or \"codex\", \
                 or set the binary with SKRILLS_CLI_BINARY or cli_binary in the subagents config"
            ))
        }
    }

    fn execution_mode_from_env(&self) -> Option<ExecutionMode> {
        match std::env::var("SKRILLS_SUBAGENTS_EXECUTION_MODE") {
            Ok(raw) => match raw.parse() {
                Ok(mode) => Some(mode),
                Err(_) => {
                    tracing::warn!(
                        value = %raw,
                        "invalid SKRILLS_SUBAGENTS_EXECUTION_MODE (expected 'cli' or 'api')"
                    );
                    None
                }
            },
            Err(_) => None,
        }
    }

    fn default_execution_mode(&self) -> ExecutionMode {
        self.execution_mode_from_env()
            .unwrap_or(self.default_execution_mode)
    }

    /// The default backend the operator set, through
    /// `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` or the subagents config file.
    fn configured_default_backend(&self) -> Option<BackendKind> {
        std::env::var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND")
            .ok()
            .as_deref()
            .map(backend_from_str)
            .or_else(|| self.configured_backend.clone())
    }

    fn default_backend_from_env(&self) -> BackendKind {
        self.configured_default_backend()
            .unwrap_or_else(|| self.default_backend.clone())
    }

    fn default_cli_binary(&self) -> String {
        let configured_backend_binary = || match self.configured_default_backend() {
            Some(BackendKind::Codex) => Some("codex".to_string()),
            Some(BackendKind::Claude) => Some("claude".to_string()),
            Some(BackendKind::Other(_)) | None => None,
        };
        self.cli_binary
            .clone()
            .or_else(configured_backend_binary)
            .or_else(cli_binary_from_client_env)
            .or_else(cli_binary_from_exe_path)
            .unwrap_or_else(|| DEFAULT_CLI_BINARY.to_string())
    }

    pub fn tools(&self) -> Vec<Tool> {
        tool_schemas::all_tools()
    }

    pub async fn handle_call(
        &self,
        name: &str,
        args: Option<&JsonMap<String, Value>>,
    ) -> Result<CallToolResult> {
        match name {
            "list-subagents" | "list_subagents" => self.handle_list_subagents().await,
            "list-agents" | "list_agents" => self.handle_list_agents().await,
            "run-subagent" | "run_subagent" => self.handle_run(false, args).await,
            "run-subagent-async" | "run_subagent_async" => self.handle_run(true, args).await,
            "get-run-status" | "get_async_status" | "get_run_status" | "get-async-status" => {
                self.handle_status(args).await
            }
            "stop-run" | "stop_run" => self.handle_stop(args).await,
            "get-run-history" | "get_run_history" => self.handle_history(args).await,
            "get-run-events" | "get_run_events" => self.handle_get_events(args).await,
            "download-transcript-secure" | "download_transcript_secure" => {
                self.handle_transcript().await
            }
            other => Err(anyhow!("unknown tool: {other}")),
        }
    }

    async fn handle_list_subagents(&self) -> Result<CallToolResult> {
        let mut templates = Vec::new();
        for adapter in self.adapters.values() {
            let mut t = adapter.list_templates().await?;
            templates.append(&mut t);
        }
        let mut cli_templates = self
            .cli_adapter_for(None, None, None)
            .list_templates()
            .await?;
        templates.append(&mut cli_templates);
        Ok(tool_ok(
            vec![ContentBlock::text("listed subagents")],
            Some(json!({"templates": templates})),
        ))
    }

    async fn handle_list_agents(&self) -> Result<CallToolResult> {
        let agents: Vec<Value> = self
            .registry
            .list()
            .iter()
            .map(|agent| {
                let requires_cli = agent.config.tools.as_ref().is_some_and(|t| !t.is_empty());

                json!({
                    "name": agent.config.name,
                    "description": agent.config.description,
                    "tools": agent.config.tools.clone().unwrap_or_default(),
                    "model": agent.config.model.clone(),
                    "source": agent.meta.source.label(),
                    "path": agent.meta.path.to_string_lossy(),
                    "requires_cli": requires_cli
                })
            })
            .collect();

        Ok(tool_ok(
            vec![ContentBlock::text(format!("found {} agents", agents.len()))],
            Some(json!({"agents": agents})),
        ))
    }

    async fn handle_run(
        &self,
        async_mode: bool,
        args: Option<&JsonMap<String, Value>>,
    ) -> Result<CallToolResult> {
        let args = args.ok_or_else(|| anyhow!("arguments required"))?;
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("prompt is required"))?
            .to_string();
        let agent_id = args.get("agent_id").and_then(|v| v.as_str());
        let template_id = args
            .get("template_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let output_schema = args.get("output_schema").cloned();
        let tracing = args
            .get("tracing")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let stream = args
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(async_mode);
        let execution_mode = args
            .get("execution_mode")
            .and_then(|v| v.as_str())
            .map(ExecutionMode::parse)
            .transpose()?
            .unwrap_or_else(|| self.default_execution_mode());
        let cli_binary_override = args
            .get("cli_binary")
            .and_then(|v| v.as_str())
            .map(|requested| self.checked_cli_binary(requested))
            .transpose()?;
        let timeout = match args.get("timeout_ms") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_u64() {
                Some(ms) if (1..=MAX_TIMEOUT_MS).contains(&ms) => {
                    Some(std::time::Duration::from_millis(ms))
                }
                _ => {
                    return Err(anyhow!(
                        "timeout_ms must be an integer from 1 to {MAX_TIMEOUT_MS}"
                    ))
                }
            },
        };

        // The caller's backend, if any. `None` lets an agent's model pick.
        let explicit_backend = args
            .get("backend")
            .and_then(|v| v.as_str())
            .map(backend_from_str);

        // Smart routing: if agent_id is specified, use agent-based routing
        let (adapter, prompt, model): (Arc<dyn BackendAdapter>, String, Option<String>) =
            if let Some(agent_name) = agent_id {
                let agent = self
                    .registry
                    .get(agent_name)
                    .ok_or_else(|| anyhow!("agent not found: {}", agent_name))?;
                let prompt = compose_agent_prompt(&agent.config.system_prompt, &prompt);
                let (adapter, model) = self.route_for_agent(
                    agent_name,
                    execution_mode,
                    cli_binary_override.clone(),
                    explicit_backend,
                    timeout,
                )?;
                (adapter, prompt, model)
            } else {
                let adapter = if matches!(execution_mode, ExecutionMode::Cli) {
                    self.cli_adapter_for(cli_binary_override, explicit_backend, timeout)
                } else {
                    if timeout.is_some() {
                        return Err(api_timeout_error());
                    }
                    // API mode: use backend from args or default.
                    let backend =
                        explicit_backend.unwrap_or_else(|| self.default_backend_from_env());
                    self.adapter_for(Some(backend))?
                };
                (adapter, prompt, None)
            };

        if output_schema.is_some() && !adapter.capabilities().supports_schema {
            return Err(anyhow!(
                "output_schema is not supported by a CLI run; use execution_mode \"api\""
            ));
        }
        if let Some(id) = template_id.as_deref() {
            let offered = adapter.list_templates().await?;
            if !offered.iter().any(|t| t.id == id) {
                let ids: Vec<&str> = offered.iter().map(|t| t.id.as_str()).collect();
                return Err(anyhow!(
                    "template_id {id:?} is not offered by the selected backend (available: {})",
                    ids.join(", ")
                ));
            }
        }

        let request = RunRequest {
            backend: adapter.backend(),
            prompt,
            template_id,
            output_schema,
            async_mode: stream,
            tracing,
            model,
        };
        let run_id = adapter.run(request, self.store.clone()).await?;
        let status = if async_mode {
            adapter.status(run_id, self.store.clone()).await?
        } else {
            // The synchronous tool returns the finished run, not a run that has
            // only just been spawned.
            let limit = timeout.unwrap_or(std::time::Duration::from_millis(DEFAULT_RUN_TIMEOUT_MS))
                + SYNC_WAIT_GRACE;
            self.wait_for_finish(run_id, limit).await?
        };
        Ok(tool_ok(
            vec![ContentBlock::text(format!("run_id={run_id}"))],
            Some(json!({
                "run_id": run_id,
                "status": status,
                "events": self.store.run(run_id).await?.map(|r| r.events).unwrap_or_default()
            })),
        ))
    }

    /// Polls the store until the run reaches a terminal state or `limit`
    /// passes, and returns the last status seen.
    async fn wait_for_finish(
        &self,
        run_id: RunId,
        limit: std::time::Duration,
    ) -> Result<Option<RunStatus>> {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let status = self.store.status(run_id).await?;
            let finished = status.as_ref().is_none_or(|s| s.state.is_terminal());
            if finished || tokio::time::Instant::now() >= deadline {
                return Ok(status);
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    /// Route to appropriate adapter based on agent configuration.
    ///
    /// Returns:
    /// - CLI adapter if agent requires tools (spawns CLI subprocess)
    /// - API adapter if agent doesn't require tools, with the model id the
    ///   request should ask for (see [`api_model_for_agent`])
    /// - Error if agent not found
    ///
    /// The `backend_hint` takes precedence over model-based detection, both
    /// for the CLI binary and for the API backend.
    fn route_for_agent(
        &self,
        agent_name: &str,
        execution_mode: ExecutionMode,
        cli_binary_override: Option<String>,
        backend_hint: Option<BackendKind>,
        timeout: Option<std::time::Duration>,
    ) -> Result<(Arc<dyn BackendAdapter>, Option<String>)> {
        let agent = self
            .registry
            .get(agent_name)
            .ok_or_else(|| anyhow!("agent not found: {}", agent_name))?;

        // Check if agent requires CLI execution (has tools)
        let requires_cli = agent.config.tools.as_ref().is_some_and(|t| !t.is_empty());
        let use_cli = matches!(execution_mode, ExecutionMode::Cli) || requires_cli;

        if use_cli {
            if matches!(execution_mode, ExecutionMode::Api) && requires_cli {
                tracing::debug!(
                    agent = agent_name,
                    "execution_mode=api requested but tools require CLI"
                );
            }
            tracing::debug!(
                agent = agent_name,
                tools = ?agent.config.tools,
                "routing to CLI adapter"
            );
            // Use backend hint to determine CLI binary, falling back to
            // model-based detection, then to the service's CLI default.
            let cli_backend = backend_hint.or_else(|| {
                Self::backend_for_model(agent.config.model.as_ref().map(|m| m.as_str()))
            });
            return Ok((
                self.cli_adapter_for(cli_binary_override, cli_backend, timeout),
                None,
            ));
        }

        // Agent doesn't require tools - use API adapter
        // Determine which API backend to use based on agent's model
        if timeout.is_some() {
            return Err(api_timeout_error());
        }
        let model = agent.config.model.as_ref().map(|m| m.as_str());
        let backend = backend_hint
            .or_else(|| Self::backend_for_model(model))
            .unwrap_or_else(|| self.default_backend_from_env());
        Ok((self.adapter_for(Some(backend))?, api_model_for_agent(model)))
    }

    /// The backend an agent's model name belongs to, if it names one.
    ///
    /// Case-insensitive and by known prefix or alias: substring matching
    /// sent `GPT-4` and `Claude-3-Opus` to the default backend and matched
    /// `o1`/`o3` inside unrelated ids. `inherit` names no backend.
    fn backend_for_model(model: Option<&str>) -> Option<BackendKind> {
        let model = model?.trim().to_ascii_lowercase();
        let openai_reasoning = model
            .strip_prefix('o')
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
        if model.starts_with("claude") || ["opus", "sonnet", "haiku"].contains(&model.as_str()) {
            Some(BackendKind::Claude)
        } else if model.starts_with("gpt") || model.starts_with("codex") || openai_reasoning {
            Some(BackendKind::Codex)
        } else {
            None
        }
    }

    async fn handle_status(&self, args: Option<&JsonMap<String, Value>>) -> Result<CallToolResult> {
        let args = args.ok_or_else(|| anyhow!("arguments required"))?;
        let run_id_val = args
            .get("run_id")
            .ok_or_else(|| anyhow!("run_id is required"))?;
        let run_id = run_id_from_value(run_id_val)?;
        let Some(record) = self.store.run(run_id).await? else {
            return Ok(run_not_found(run_id));
        };
        Ok(tool_ok(
            vec![ContentBlock::text("status")],
            Some(json!({
                "run_id": run_id,
                "status": record.status,
                "events": record.events
            })),
        ))
    }

    async fn handle_stop(&self, args: Option<&JsonMap<String, Value>>) -> Result<CallToolResult> {
        let args = args.ok_or_else(|| anyhow!("arguments required"))?;
        let run_id = run_id_from_value(
            args.get("run_id")
                .ok_or_else(|| anyhow!("run_id is required"))?,
        )?;
        if self.store.status(run_id).await?.is_none() {
            return Ok(run_not_found(run_id));
        }
        let stopped = self.store.stop(run_id).await?;
        // Kill the CLI child, if this run has one. API runs have no process;
        // their late result is ignored because the run is already terminal.
        // An HTTP run has no child: abort its request task instead.
        if !cancel_cli_process(&self.cli_processes, run_id).await {
            abort_run_task(run_id);
        }
        Ok(tool_ok(
            vec![ContentBlock::text("stopped")],
            Some(json!({"run_id": run_id, "stopped": stopped})),
        ))
    }

    async fn handle_history(
        &self,
        args: Option<&JsonMap<String, Value>>,
    ) -> Result<CallToolResult> {
        let limit = match args.and_then(|m| m.get("limit")) {
            None | Some(Value::Null) => DEFAULT_HISTORY_LIMIT,
            Some(v) => match v.as_u64() {
                Some(n) if (1..=MAX_HISTORY_LIMIT as u64).contains(&n) => n as usize,
                _ => {
                    return Err(anyhow!(
                        "limit must be an integer from 1 to {MAX_HISTORY_LIMIT}"
                    ))
                }
            },
        };
        let runs = self.store.history(limit).await?;
        Ok(tool_ok(
            vec![ContentBlock::text("history")],
            Some(json!({"runs": runs})),
        ))
    }

    async fn handle_get_events(
        &self,
        args: Option<&JsonMap<String, Value>>,
    ) -> Result<CallToolResult> {
        let args = args.ok_or_else(|| anyhow!("arguments required"))?;
        let run_id_val = args
            .get("run_id")
            .ok_or_else(|| anyhow!("run_id is required"))?;
        let run_id = run_id_from_value(run_id_val)?;

        // Optional since_index for incremental fetching
        let since_index = args
            .get("since_index")
            .and_then(|v| v.as_u64())
            .map(|v| usize::try_from(v).unwrap_or(usize::MAX));
        let limit = match args.get("limit") {
            None | Some(Value::Null) => MAX_EVENTS_PAGE,
            Some(v) => match v.as_u64() {
                Some(n) if (1..=MAX_EVENTS_PAGE as u64).contains(&n) => n as usize,
                _ => {
                    return Err(anyhow!(
                        "limit must be an integer from 1 to {MAX_EVENTS_PAGE}"
                    ))
                }
            },
        };

        // Fetch the run record
        let Some(record) = self.store.run(run_id).await? else {
            return Ok(run_not_found(run_id));
        };

        // Indices are absolute over the run's whole life. Past the per-run cap
        // the held events start with an `events_dropped` marker for the
        // dropped ones; it takes the index of the last dropped event, so held
        // event `p` has index `base + p` (RT-31).
        let base = crate::store::dropped_event_count(&record.events).saturating_sub(1);
        let total_count = base + record.events.len();

        // Determine the slice of events to return
        let (events_to_return, start_index) = match since_index {
            Some(idx) => {
                // Return events after the given index
                let start = idx.saturating_add(1);
                let position = start.saturating_sub(base);
                if start >= total_count {
                    (Vec::new(), start)
                } else {
                    (record.events[position..].to_vec(), base + position)
                }
            }
            None => {
                // Return all events
                (record.events.clone(), base)
            }
        };

        // One page; the rest is fetched with `since_index`.
        let has_more = events_to_return.len() > limit;
        // Format events with their indices
        let events_json: Vec<Value> = events_to_return
            .iter()
            .take(limit)
            .enumerate()
            .map(|(i, event)| {
                json!({
                    "index": start_index + i,
                    "ts": event.ts.to_string(),
                    "kind": event.kind,
                    "data": event.data
                })
            })
            .collect();

        Ok(tool_ok(
            vec![ContentBlock::text(format!(
                "events: {} of {} total",
                events_json.len(),
                total_count
            ))],
            Some(json!({
                "run_id": run_id.to_string(),
                "events": events_json,
                "total_count": total_count,
                "has_more": has_more
            })),
        ))
    }

    /// Reported as a tool-level error, not a success: the tool is no longer
    /// advertised, but older callers may still name it, and one that
    /// branches on `is_error` would otherwise treat "not implemented" as a
    /// transcript it can read.
    async fn handle_transcript(&self) -> Result<CallToolResult> {
        Ok(tool_err(
            vec![ContentBlock::text(
                "secure transcripts are not yet implemented",
            )],
            Some(json!({"status": "unimplemented"})),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MemRunStore, RunEvent, RunRequest, RunState};
    use skrills_discovery::{SkillRoot, SkillSource};
    use std::fs;
    use tempfile::tempdir;
    use time::OffsetDateTime;

    fn create_agent_file(dir: &std::path::Path, name: &str, content: &str) {
        let agents_dir = dir.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(agents_dir.join(name), content).unwrap();
    }

    /// A service built without reading the developer's HOME: an empty agent
    /// registry and no subagents config file (M322).
    fn hermetic_service(
        store: Arc<dyn RunStore>,
        default_backend: BackendKind,
    ) -> Result<SubagentService> {
        SubagentService::with_store_and_registry_with_config(
            store,
            default_backend,
            Arc::new(AgentRegistry::discover_from_roots(&[]).unwrap()),
            SubagentsFileConfig::default(),
        )
    }

    #[test]
    fn tool_ok_marks_the_result_as_not_an_error() {
        let result = tool_ok(vec![ContentBlock::text("done")], Some(json!({"runs": 0})));

        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content, Some(json!({"runs": 0})));
    }

    #[test]
    fn tool_err_marks_the_result_as_an_error() {
        let result = tool_err(vec![ContentBlock::text("failed")], None);

        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.structured_content, None);
    }

    use skrills_test_utils::{env_guard, set_env_var, EnvVarGuard};

    /// Runs `body` with the env lock held and a harmless environment: every
    /// CLI run goes to `true`, so a real `claude` or `codex` is never spawned;
    /// API keys are cleared so an API-mode run fails fast instead of calling a
    /// real endpoint; HOME points at an empty temp dir so the developer's agents
    /// and config are not read.
    ///
    /// The body runs on its own runtime inside this synchronous function, so
    /// the std env lock is never held across an `.await`.
    fn with_harmless_env<F, Fut>(body: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let _lock = env_guard();
        let home = tempdir().unwrap();
        let _guards: Vec<EnvVarGuard> = vec![
            set_env_var("HOME", Some(home.path().to_str().unwrap())),
            set_env_var("SKRILLS_CLI_BINARY", Some("true")),
            set_env_var("SKRILLS_CODEX_API_KEY", None),
            set_env_var("SKRILLS_CLAUDE_API_KEY", None),
            set_env_var("SKRILLS_SUBAGENTS_EXECUTION_MODE", None),
            set_env_var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND", None),
        ];
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(body());
    }

    fn event_kinds(content: &Value) -> Vec<String> {
        content
            .get("events")
            .and_then(|v| v.as_array())
            .map(|events| {
                events
                    .iter()
                    .filter_map(|e| e.get("kind").and_then(|k| k.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn status_message(content: &Value) -> String {
        content
            .get("status")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// Like [`with_harmless_env`], but `PATH` holds only fake `claude`,
    /// `codex` and `cursor` scripts that print `ran:<name>`, so a test can
    /// see which CLI a run spawned. Client detection variables are cleared;
    /// `env` then sets the ones the test needs. `body` gets the temp HOME.
    #[cfg(unix)]
    fn with_fake_clis<F, Fut>(env: &[(&'static str, Option<&str>)], body: F)
    where
        F: FnOnce(std::path::PathBuf) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _lock = env_guard();
        let home = tempdir().unwrap();
        let bin = home.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        for name in ["claude", "codex", "cursor"] {
            let script = bin.join(name);
            fs::write(&script, format!("#!/bin/sh\necho ran:{name}\n")).unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut guards: Vec<EnvVarGuard> = vec![
            set_env_var("HOME", Some(home.path().to_str().unwrap())),
            set_env_var("PATH", Some(bin.to_str().unwrap())),
            set_env_var("SKRILLS_CLI_BINARY", None),
            set_env_var("SKRILLS_CLI_TIMEOUT_MS", None),
            set_env_var("SKRILLS_CODEX_API_KEY", None),
            set_env_var("SKRILLS_CLAUDE_API_KEY", None),
            set_env_var("SKRILLS_SUBAGENTS_EXECUTION_MODE", None),
            set_env_var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND", None),
            set_env_var("SKRILLS_CLIENT", None),
            set_env_var("CLAUDE_CODE_SESSION", None),
            set_env_var("CLAUDE_CLI", None),
            set_env_var("__CLAUDE_MCP_SERVER", None),
            set_env_var("CLAUDE_CODE_ENTRYPOINT", None),
            set_env_var("CODEX_SESSION_ID", None),
            set_env_var("CODEX_CLI", None),
            set_env_var("CODEX_HOME", None),
            set_env_var("CURSOR_PROJECT_DIR", None),
            set_env_var("CURSOR_VERSION", None),
        ];
        for (key, value) in env {
            guards.push(set_env_var(key, *value));
        }
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(body(home.path().to_path_buf()));
    }

    /// The `text` of a run's `completion` event.
    fn completion_text(content: &Value) -> String {
        content["events"]
            .as_array()
            .and_then(|events| events.iter().find(|e| e["kind"] == "completion"))
            .and_then(|e| e["data"]["text"].as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// M251: with no `backend` argument and no configured default backend,
    /// the CLI follows client detection instead of a hardcoded Codex.
    #[cfg(unix)]
    #[test]
    fn run_without_backend_arg_follows_client_detection() {
        with_fake_clis(&[("CLAUDE_CODE_ENTRYPOINT", Some("cli"))], |_home| async {
            let service = SubagentService::new().unwrap();
            let args = json!({"prompt": "hi"}).as_object().cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();
            let content = result.structured_content.unwrap();
            assert_eq!(completion_text(&content), "ran:claude", "{content}");
        });
    }

    /// M251: the `cli_binary` in subagents.toml picks the CLI when the call
    /// names no backend.
    #[cfg(unix)]
    #[test]
    fn run_without_backend_arg_uses_the_configured_cli_binary() {
        with_fake_clis(&[("SKRILLS_CLIENT", Some("codex"))], |home| async move {
            fs::create_dir_all(home.join(".claude")).unwrap();
            fs::write(
                home.join(".claude/subagents.toml"),
                "cli_binary = \"claude\"\n",
            )
            .unwrap();
            let service = SubagentService::new().unwrap();
            let args = json!({"prompt": "hi"}).as_object().cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();
            let content = result.structured_content.unwrap();
            assert_eq!(completion_text(&content), "ran:claude", "{content}");
        });
    }

    #[tokio::test]
    async fn tools_include_core_and_extended() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let tools = service.tools();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"run-subagent"));
        assert!(names.contains(&"run-subagent-async"));
        // M320: unimplemented, so not advertised.
        assert!(!names.contains(&"download-transcript-secure"));
    }

    #[tokio::test]
    async fn tools_include_list_agents() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let tools = service.tools();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"list-agents"));
    }

    #[tokio::test]
    async fn list_agents_returns_empty_when_no_agents() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Create empty agent roots (no agents)
        let roots = vec![SkillRoot {
            root: home.join(".codex/agents"),
            source: SkillSource::Codex,
        }];

        let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
        let service = SubagentService::with_store_and_registry(
            Arc::new(MemRunStore::new()),
            BackendKind::Codex,
            registry,
        )
        .unwrap();

        let result = service.handle_call("list-agents", None).await.unwrap();
        let agents = result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("agents"))
            .and_then(|v| v.as_array())
            .expect("should have agents array");

        assert!(agents.is_empty());
    }

    #[tokio::test]
    async fn list_agents_returns_agent_data() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        create_agent_file(
            &home.join(".codex"),
            "test-agent.md",
            r#"---
name: test-agent
description: A test agent for listing
tools: Read, Bash
model: sonnet
---

You are a test agent."#,
        );

        let roots = vec![SkillRoot {
            root: home.join(".codex/agents"),
            source: SkillSource::Codex,
        }];

        let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
        let service = SubagentService::with_store_and_registry(
            Arc::new(MemRunStore::new()),
            BackendKind::Codex,
            registry,
        )
        .unwrap();

        let result = service.handle_call("list-agents", None).await.unwrap();
        let agents = result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("agents"))
            .and_then(|v| v.as_array())
            .expect("should have agents array");

        assert_eq!(agents.len(), 1);

        let agent = &agents[0];
        assert_eq!(
            agent.get("name").and_then(|v| v.as_str()),
            Some("test-agent")
        );
        assert_eq!(
            agent.get("description").and_then(|v| v.as_str()),
            Some("A test agent for listing")
        );
        assert_eq!(agent.get("model").and_then(|v| v.as_str()), Some("sonnet"));
        assert_eq!(agent.get("source").and_then(|v| v.as_str()), Some("codex"));
        assert!(agent.get("path").and_then(|v| v.as_str()).is_some());

        let tools = agent
            .get("tools")
            .and_then(|v| v.as_array())
            .expect("should have tools array");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].as_str(), Some("Read"));
        assert_eq!(tools[1].as_str(), Some("Bash"));
    }

    #[tokio::test]
    async fn list_agents_requires_cli_field_computed_correctly() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Agent with tools (requires CLI)
        create_agent_file(
            &home.join(".codex"),
            "tool-agent.md",
            r#"---
name: tool-agent
description: Has tools
tools: Read, Bash
---

Content."#,
        );

        // Agent without tools (does not require CLI)
        create_agent_file(
            &home.join(".codex"),
            "no-tool-agent.md",
            r#"---
name: no-tool-agent
description: No tools
---

Content."#,
        );

        let roots = vec![SkillRoot {
            root: home.join(".codex/agents"),
            source: SkillSource::Codex,
        }];

        let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
        let service = SubagentService::with_store_and_registry(
            Arc::new(MemRunStore::new()),
            BackendKind::Codex,
            registry,
        )
        .unwrap();

        let result = service.handle_call("list-agents", None).await.unwrap();
        let agents = result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("agents"))
            .and_then(|v| v.as_array())
            .expect("should have agents array");

        assert_eq!(agents.len(), 2);

        // Find agents by name and check requires_cli
        let tool_agent = agents
            .iter()
            .find(|a| a.get("name").and_then(|v| v.as_str()) == Some("tool-agent"))
            .expect("should find tool-agent");
        let no_tool_agent = agents
            .iter()
            .find(|a| a.get("name").and_then(|v| v.as_str()) == Some("no-tool-agent"))
            .expect("should find no-tool-agent");

        assert_eq!(
            tool_agent.get("requires_cli").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            no_tool_agent.get("requires_cli").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[tokio::test]
    async fn list_agents_snake_case_alias_works() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let roots = vec![SkillRoot {
            root: home.join(".codex/agents"),
            source: SkillSource::Codex,
        }];

        let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
        let service = SubagentService::with_store_and_registry(
            Arc::new(MemRunStore::new()),
            BackendKind::Codex,
            registry,
        )
        .unwrap();

        // Should work with both naming conventions
        let result_dash = service.handle_call("list-agents", None).await;
        let result_underscore = service.handle_call("list_agents", None).await;

        assert!(result_dash.is_ok());
        assert!(result_underscore.is_ok());
    }

    #[test]
    fn run_and_status_round_trip() {
        with_harmless_env(|| async {
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            let args = json!({"prompt": "hi", "backend": "codex", "execution_mode": "api"})
                .as_object()
                .cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();
            let run_id = result
                .structured_content
                .as_ref()
                .and_then(|v| v.get("run_id"))
                .and_then(|v| v.as_str())
                .map(|s| RunId(uuid::Uuid::parse_str(s).unwrap()))
                .unwrap();
            // RT-17: the synchronous tool returns a finished run. With no API
            // key the API adapter fails at once, which is the finish here.
            let status = service.store.status(run_id).await.unwrap().unwrap();
            assert_eq!(status.state, RunState::Failed);
            assert!(
                status
                    .message
                    .as_deref()
                    .unwrap_or("")
                    .contains("API key not set"),
                "{status:?}"
            );
        });
    }

    #[test]
    fn snake_case_aliases_are_supported() {
        with_harmless_env(|| async {
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            let args = json!({"prompt": "hello", "backend": "codex"})
                .as_object()
                .cloned();
            let result = service
                .handle_call("run_subagent", args.as_ref())
                .await
                .unwrap();
            assert!(result.structured_content.is_some());
        });
    }

    // =============================================================
    // Tests for smart routing (Task 4)
    // =============================================================

    #[tokio::test]
    async fn run_subagent_tool_schema_includes_agent_id() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let tools = service.tools();

        // Check run-subagent
        let run_tool = tools.iter().find(|t| t.name.as_ref() == "run-subagent");
        assert!(run_tool.is_some(), "run-subagent tool should exist");

        let schema = run_tool.unwrap().input_schema.as_ref();
        let props = schema.get("properties").expect("should have properties");
        assert!(
            props.get("agent_id").is_some(),
            "run-subagent should have agent_id property"
        );
        assert!(
            props.get("execution_mode").is_some(),
            "run-subagent should have execution_mode property"
        );
        assert!(
            props.get("cli_binary").is_some(),
            "run-subagent should have cli_binary property"
        );

        // Check run-subagent-async
        let async_tool = tools
            .iter()
            .find(|t| t.name.as_ref() == "run-subagent-async");
        assert!(async_tool.is_some(), "run-subagent-async tool should exist");

        let async_schema = async_tool.unwrap().input_schema.as_ref();
        let async_props = async_schema
            .get("properties")
            .expect("should have properties");
        assert!(
            async_props.get("agent_id").is_some(),
            "run-subagent-async should have agent_id property"
        );
        assert!(
            async_props.get("execution_mode").is_some(),
            "run-subagent-async should have execution_mode property"
        );
        assert!(
            async_props.get("cli_binary").is_some(),
            "run-subagent-async should have cli_binary property"
        );
    }

    #[tokio::test]
    async fn cli_binary_defaults_to_codex_when_client_env_codex() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let _home_guard = set_env_var("HOME", Some(temp.path().to_str().unwrap()));
        let _client_guard = set_env_var("SKRILLS_CLIENT", Some("codex"));
        let _cli_guard = set_env_var("SKRILLS_CLI_BINARY", None);
        let _claude_session = set_env_var("CLAUDE_CODE_SESSION", None);
        let _claude_cli = set_env_var("CLAUDE_CLI", None);
        let _claude_mcp = set_env_var("__CLAUDE_MCP_SERVER", None);
        let _claude_entry = set_env_var("CLAUDE_CODE_ENTRYPOINT", None);
        let _codex_session = set_env_var("CODEX_SESSION_ID", None);
        let _codex_cli = set_env_var("CODEX_CLI", None);
        let _codex_home = set_env_var("CODEX_HOME", None);

        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        assert_eq!(service.default_cli_binary(), "codex");
    }

    #[tokio::test]
    async fn cli_binary_defaults_to_claude_when_client_env_claude() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let _home_guard = set_env_var("HOME", Some(temp.path().to_str().unwrap()));
        let _client_guard = set_env_var("SKRILLS_CLIENT", Some("claude"));
        let _cli_guard = set_env_var("SKRILLS_CLI_BINARY", None);
        let _claude_session = set_env_var("CLAUDE_CODE_SESSION", None);
        let _claude_cli = set_env_var("CLAUDE_CLI", None);
        let _claude_mcp = set_env_var("__CLAUDE_MCP_SERVER", None);
        let _claude_entry = set_env_var("CLAUDE_CODE_ENTRYPOINT", None);
        let _codex_session = set_env_var("CODEX_SESSION_ID", None);
        let _codex_cli = set_env_var("CODEX_CLI", None);
        let _codex_home = set_env_var("CODEX_HOME", None);

        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        assert_eq!(service.default_cli_binary(), "claude");
    }

    #[tokio::test]
    async fn cli_binary_follows_configured_default_backend_over_client_env() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let _home_guard = set_env_var("HOME", Some(temp.path().to_str().unwrap()));
        let _cli_guard = set_env_var("SKRILLS_CLI_BINARY", None);
        let _claude_session = set_env_var("CLAUDE_CODE_SESSION", None);
        let _claude_cli = set_env_var("CLAUDE_CLI", None);
        let _claude_mcp = set_env_var("__CLAUDE_MCP_SERVER", None);
        let _claude_entry = set_env_var("CLAUDE_CODE_ENTRYPOINT", None);
        let _codex_session = set_env_var("CODEX_SESSION_ID", None);
        let _codex_cli = set_env_var("CODEX_CLI", None);
        let _codex_home = set_env_var("CODEX_HOME", None);

        for (backend, client, expected) in
            [("claude", "codex", "claude"), ("codex", "claude", "codex")]
        {
            let _backend_guard = set_env_var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND", Some(backend));
            let _client_guard = set_env_var("SKRILLS_CLIENT", Some(client));
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            assert_eq!(service.default_cli_binary(), expected, "backend={backend}");
        }
    }

    #[tokio::test]
    async fn cli_binary_defaults_to_codex_when_codex_session_env_present() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let _home_guard = set_env_var("HOME", Some(temp.path().to_str().unwrap()));
        let _client_guard = set_env_var("SKRILLS_CLIENT", None);
        let _codex_guard = set_env_var("CODEX_SESSION_ID", Some("session-123"));
        let _cli_guard = set_env_var("SKRILLS_CLI_BINARY", None);
        let _claude_session = set_env_var("CLAUDE_CODE_SESSION", None);
        let _claude_cli = set_env_var("CLAUDE_CLI", None);
        let _claude_mcp = set_env_var("__CLAUDE_MCP_SERVER", None);
        let _claude_entry = set_env_var("CLAUDE_CODE_ENTRYPOINT", None);

        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        assert_eq!(service.default_cli_binary(), "codex");
    }

    #[tokio::test]
    async fn cli_binary_defaults_to_claude_when_claude_session_env_present() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let _home_guard = set_env_var("HOME", Some(temp.path().to_str().unwrap()));
        let _client_guard = set_env_var("SKRILLS_CLIENT", None);
        let _claude_guard = set_env_var("CLAUDE_CODE_SESSION", Some("session-123"));
        let _cli_guard = set_env_var("SKRILLS_CLI_BINARY", None);
        let _codex_session = set_env_var("CODEX_SESSION_ID", None);
        let _codex_cli = set_env_var("CODEX_CLI", None);
        let _codex_home = set_env_var("CODEX_HOME", None);

        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        assert_eq!(service.default_cli_binary(), "claude");
    }

    #[tokio::test]
    async fn cli_binary_env_auto_uses_default() {
        let adapter = {
            let _guard = env_guard();
            let temp = tempdir().unwrap();
            let _home_guard = set_env_var("HOME", Some(temp.path().to_str().unwrap()));
            let _client_guard = set_env_var("SKRILLS_CLIENT", Some("codex"));
            let _cli_guard = set_env_var("SKRILLS_CLI_BINARY", Some("auto"));
            let _claude_session = set_env_var("CLAUDE_CODE_SESSION", None);
            let _claude_cli = set_env_var("CLAUDE_CLI", None);
            let _claude_mcp = set_env_var("__CLAUDE_MCP_SERVER", None);
            let _claude_entry = set_env_var("CLAUDE_CODE_ENTRYPOINT", None);
            let _codex_session = set_env_var("CODEX_SESSION_ID", None);
            let _codex_cli = set_env_var("CODEX_CLI", None);
            let _codex_home = set_env_var("CODEX_HOME", None);

            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            service.cli_adapter_for(None, Some(BackendKind::Codex), None)
        };

        let templates = adapter.list_templates().await.unwrap();
        let name = templates
            .first()
            .map(|t| t.name.to_lowercase())
            .unwrap_or_default();

        assert!(name.contains("codex"));
    }

    #[tokio::test]
    async fn cli_adapter_uses_backend_hint_for_binary_selection() {
        // Test that backend hint overrides default cli binary detection
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let _home_guard = set_env_var("HOME", Some(temp.path().to_str().unwrap()));
        // Clear all client detection env vars
        let _client_guard = set_env_var("SKRILLS_CLIENT", None);
        let _cli_guard = set_env_var("SKRILLS_CLI_BINARY", None);
        let _claude_session = set_env_var("CLAUDE_CODE_SESSION", None);
        let _claude_cli = set_env_var("CLAUDE_CLI", None);
        let _claude_mcp = set_env_var("__CLAUDE_MCP_SERVER", None);
        let _claude_entry = set_env_var("CLAUDE_CODE_ENTRYPOINT", None);
        let _codex_session = set_env_var("CODEX_SESSION_ID", None);
        let _codex_cli = set_env_var("CODEX_CLI", None);
        let _codex_home = set_env_var("CODEX_HOME", None);

        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

        // When backend hint is Codex, CLI binary should be "codex"
        let codex_adapter = service.cli_adapter_for(None, Some(BackendKind::Codex), None);
        assert_eq!(
            codex_adapter.config().binary,
            "codex",
            "Backend hint Codex should select 'codex' binary"
        );

        // When backend hint is Claude, CLI binary should be "claude"
        let claude_adapter = service.cli_adapter_for(None, Some(BackendKind::Claude), None);
        assert_eq!(
            claude_adapter.config().binary,
            "claude",
            "Backend hint Claude should select 'claude' binary"
        );

        // When no backend hint, should fall back to default detection
        let default_adapter = service.cli_adapter_for(None, None, None);
        // Default is "claude" when no env vars are set (DEFAULT_CLI_BINARY)
        assert_eq!(
            default_adapter.config().binary,
            "claude",
            "No backend hint should fall back to default"
        );

        // When backend hint is Copilot (via Other), should fall back to default
        // (Copilot CLI doesn't support subagent execution)
        let copilot_adapter =
            service.cli_adapter_for(None, Some(BackendKind::Other("copilot".to_string())), None);
        assert_eq!(
            copilot_adapter.config().binary,
            "claude",
            "Backend hint Copilot should fall back to default (unsupported)"
        );
    }

    #[test]
    fn run_without_agent_id_defaults_to_cli_mode() {
        with_harmless_env(|| async {
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            let args = json!({"prompt": "hi"}).as_object().cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();

            // Should succeed using CLI mode by default, and the synchronous
            // call returns once the child has exited.
            let content = result.structured_content.unwrap();
            assert!(content.get("run_id").is_some());
            assert_eq!(content["status"]["state"], "Succeeded", "{content}");
            assert!(event_kinds(&content).contains(&"completion".to_string()));
        });
    }

    #[test]
    fn run_with_execution_mode_api_uses_api_adapter() {
        with_harmless_env(|| async {
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            let args = json!({"prompt": "hi", "execution_mode": "api", "backend": "codex"})
                .as_object()
                .cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();

            // The Codex API adapter ran: it failed on the missing key, which a
            // CLI run would never report.
            let content = result.structured_content.unwrap();
            assert!(
                status_message(&content).contains("Codex API key not set"),
                "{content}"
            );
        });
    }

    #[test]
    fn run_with_agent_id_no_tools_routes_to_api() {
        with_harmless_env(|| async {
            let tmp = tempdir().unwrap();
            let home = tmp.path();

            // Create an agent without tools (API-capable)
            create_agent_file(
                &home.join(".codex"),
                "api-agent.md",
                r#"---
name: api-agent
description: An agent without tools
model: gpt-4
---

You are an API agent."#,
            );

            let roots = vec![SkillRoot {
                root: home.join(".codex/agents"),
                source: SkillSource::Codex,
            }];

            let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Codex,
                registry,
            )
            .unwrap();

            let args = json!({"prompt": "hi", "agent_id": "api-agent", "execution_mode": "api"})
                .as_object()
                .cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();

            // Routed to the Codex API adapter (model gpt-4), which fails on the
            // missing key: a CLI run would never report that.
            let content = result.structured_content.unwrap();
            assert!(content.get("run_id").is_some());
            assert!(
                status_message(&content).contains("Codex API key not set"),
                "{content}"
            );
        });
    }

    #[test]
    fn run_with_agent_id_with_tools_routes_to_cli() {
        with_harmless_env(|| async {
            let tmp = tempdir().unwrap();
            let home = tmp.path();

            // Create an agent WITH tools (requires CLI)
            create_agent_file(
                &home.join(".codex"),
                "cli-agent.md",
                r#"---
name: cli-agent
description: An agent with tools
tools: Read, Bash, Glob
model: sonnet
---

You are a CLI agent."#,
            );

            let roots = vec![SkillRoot {
                root: home.join(".codex/agents"),
                source: SkillSource::Codex,
            }];

            let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Codex,
                registry,
            )
            .unwrap();

            let args = json!({"prompt": "hi", "agent_id": "cli-agent"})
                .as_object()
                .cloned();
            let result = service.handle_run(false, args.as_ref()).await;

            // Routed to the CLI: the `true` child ran and completed.
            let result = result.expect("should route to CLI adapter");
            let content = result.structured_content.unwrap();
            assert_eq!(content["status"]["state"], "Succeeded", "{content}");
            assert!(event_kinds(&content).contains(&"completion".to_string()));
        });
    }

    #[test]
    fn run_with_agent_id_with_tools_execution_mode_api_still_uses_cli() {
        with_harmless_env(|| async {
            let tmp = tempdir().unwrap();
            let home = tmp.path();

            // Create an agent WITH tools (requires CLI)
            create_agent_file(
                &home.join(".codex"),
                "cli-agent.md",
                r#"---
name: cli-agent
description: An agent with tools
tools: Read, Bash, Glob
model: sonnet
---

You are a CLI agent."#,
            );

            let roots = vec![SkillRoot {
                root: home.join(".codex/agents"),
                source: SkillSource::Codex,
            }];

            let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Codex,
                registry,
            )
            .unwrap();

            let args = json!({"prompt": "hi", "agent_id": "cli-agent", "execution_mode": "api"})
                .as_object()
                .cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();

            // A CLI child ran to completion; an API run has no completion here.
            let content = result.structured_content.unwrap();
            assert_eq!(content["status"]["state"], "Succeeded", "{content}");
            assert!(event_kinds(&content).contains(&"completion".to_string()));
        });
    }

    #[test]
    fn run_with_nonexistent_agent_id_errors() {
        with_harmless_env(|| async {
            let tmp = tempdir().unwrap();
            let home = tmp.path();

            let roots = vec![SkillRoot {
                root: home.join(".codex/agents"),
                source: SkillSource::Codex,
            }];

            let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Codex,
                registry,
            )
            .unwrap();

            let args = json!({"prompt": "hi", "agent_id": "nonexistent-agent"})
                .as_object()
                .cloned();
            let result = service.handle_run(false, args.as_ref()).await;

            // Should error because agent doesn't exist
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.to_string().contains("agent not found"),
                "error should mention agent not found: {}",
                err
            );
        });
    }

    /// M261: in API mode the call's `backend` outranks the agent's model.
    #[test]
    fn api_agent_run_honours_the_backend_argument() {
        with_harmless_env(|| async {
            let tmp = tempdir().unwrap();
            let registry = agent_registry(
                tmp.path(),
                "my-agent.md",
                "---\nname: my-agent\ndescription: d\nmodel: opus\n---\n\nBody.",
            );
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Claude,
                registry,
            )
            .unwrap();

            let args = json!({
                "prompt": "hi",
                "agent_id": "my-agent",
                "backend": "codex",
                "execution_mode": "api"
            })
            .as_object()
            .cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();

            let content = result.structured_content.unwrap();
            assert!(
                status_message(&content).contains("Codex API key not set"),
                "{content}"
            );
        });
    }

    /// M261: an API-mode agent run asks the API for the agent's model, not
    /// the adapter's default.
    #[test]
    fn api_agent_run_sends_the_agent_model() {
        with_harmless_env(|| async {
            use wiremock::matchers::{body_partial_json, method};
            use wiremock::{Mock, MockServer, ResponseTemplate};
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_partial_json(json!({"model": "gpt-4o-mini"})))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}]
                })))
                .expect(1)
                .mount(&server)
                .await;
            let _key = set_env_var("SKRILLS_CODEX_API_KEY", Some("test-key"));
            let _base = set_env_var("SKRILLS_CODEX_BASE_URL", Some(&server.uri()));
            let tmp = tempdir().unwrap();
            let registry = agent_registry(
                tmp.path(),
                "mini.md",
                "---\nname: mini\ndescription: d\nmodel: gpt-4o-mini\n---\n\nBody.",
            );
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Codex,
                registry,
            )
            .unwrap();

            let args = json!({"prompt": "hi", "agent_id": "mini", "execution_mode": "api"})
                .as_object()
                .cloned();
            let result = service.handle_run(false, args.as_ref()).await.unwrap();

            let content = result.structured_content.unwrap();
            assert_eq!(content["status"]["state"], "Succeeded", "{content}");
        });
    }

    /// M316: model names are matched case-insensitively by known prefix,
    /// not by substring.
    #[test]
    fn backend_for_model_matches_known_prefixes_case_insensitively() {
        use BackendKind::{Claude, Codex};
        for (model, expected) in [
            ("GPT-4", Some(Codex)),
            ("gpt-6-astra", Some(Codex)),
            ("o1-preview", Some(Codex)),
            ("o4-mini", Some(Codex)),
            ("codex-mini", Some(Codex)),
            ("Claude-3-Opus", Some(Claude)),
            ("claude-opus-5-5", Some(Claude)),
            ("Sonnet", Some(Claude)),
            ("haiku", Some(Claude)),
            ("neo1-chat", None),
            ("pro3-model", None),
            ("inherit", None),
        ] {
            assert_eq!(
                SubagentService::backend_for_model(Some(model)),
                expected,
                "{model}"
            );
        }
    }

    #[test]
    fn claude_code_model_aliases_are_not_sent_as_api_model_ids() {
        for alias in ["opus", "Sonnet", "haiku", "inherit"] {
            assert_eq!(api_model_for_agent(Some(alias)), None, "{alias}");
        }
        assert_eq!(
            api_model_for_agent(Some("gpt-4o-mini")).as_deref(),
            Some("gpt-4o-mini")
        );
        assert_eq!(api_model_for_agent(None), None);
    }

    #[test]
    fn run_async_with_agent_id_routes_correctly() {
        with_harmless_env(|| async {
            let tmp = tempdir().unwrap();
            let home = tmp.path();

            create_agent_file(
                &home.join(".codex"),
                "async-agent.md",
                r#"---
name: async-agent
description: An async-capable agent
---

Content."#,
            );

            let roots = vec![SkillRoot {
                root: home.join(".codex/agents"),
                source: SkillSource::Codex,
            }];

            let registry = Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap());
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Codex,
                registry,
            )
            .unwrap();

            // Test run-subagent-async with agent_id
            let args = json!({"prompt": "hi", "agent_id": "async-agent"})
                .as_object()
                .cloned();
            let result = service.handle_run(true, args.as_ref()).await.unwrap();

            assert!(result.structured_content.is_some());
        });
    }

    #[tokio::test]
    async fn handle_call_with_unknown_tool_returns_error() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

        let result = service.handle_call("nonexistent-tool", None).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("unknown tool"),
            "expected 'unknown tool' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn handle_call_with_invalid_tool_name_returns_error() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

        // Test various invalid tool names
        for invalid_name in [
            "",
            "foo-bar-baz",
            "unknown_command",
            "definitely-not-a-tool",
        ] {
            let result = service.handle_call(invalid_name, None).await;
            assert!(
                result.is_err(),
                "expected error for tool name '{}', but got Ok",
                invalid_name
            );
        }
    }

    // =============================================================
    // Tests for get-run-events (Task 6)
    // =============================================================

    #[tokio::test]
    async fn tools_include_get_run_events() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let tools = service.tools();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            names.contains(&"get-run-events"),
            "should have get-run-events tool"
        );
    }

    /// RT-31: once the oldest events are dropped at the per-run cap, an
    /// index a client already holds must still name the same event.
    #[tokio::test]
    async fn get_run_events_indices_stay_stable_after_old_events_are_dropped() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "test".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();
        let total = 10_000 + 5;
        for i in 0..total {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: "tick".into(),
                        data: Some(json!({"n": i})),
                    },
                )
                .await
                .unwrap();
        }

        let service = hermetic_service(store, BackendKind::Codex).unwrap();
        let fetch = |since: Option<u64>| {
            let mut args = json!({"run_id": run_id.0.to_string()});
            if let Some(since) = since {
                args["since_index"] = json!(since);
            }
            let service = &service;
            async move {
                service
                    .handle_call("get-run-events", args.as_object())
                    .await
                    .unwrap()
                    .structured_content
                    .expect("structured content")
            }
        };

        // Index N is event N for every event still held.
        let after = fetch(Some(10_001)).await;
        let events = after["events"].as_array().unwrap();
        assert_eq!(events.len(), 3, "{after}");
        for event in events {
            assert_eq!(event["index"], event["data"]["n"], "{event}");
        }
        assert_eq!(after["total_count"], json!(total));

        // A client that fell behind the cap is told how many it missed.
        let behind = fetch(Some(2)).await;
        let first = &behind["events"][0];
        assert_eq!(first["kind"], "events_dropped", "{first}");
        let dropped = &first["data"]["count"];
        assert_eq!(&behind["events"][1]["data"]["n"], dropped);
        assert_eq!(&behind["events"][1]["index"], dropped);
    }

    #[tokio::test]
    async fn get_run_events_returns_all_events_without_since_index() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "test".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();

        // Add some events
        for i in 0..3 {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: format!("event-{}", i),
                        data: Some(json!({"index": i})),
                    },
                )
                .await
                .unwrap();
        }

        let service = hermetic_service(store, BackendKind::Codex).unwrap();
        let args = json!({"run_id": run_id.0.to_string()}).as_object().cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        let content = result
            .structured_content
            .expect("should have structured content");
        let events = content
            .get("events")
            .and_then(|v| v.as_array())
            .expect("should have events array");
        assert_eq!(events.len(), 3);

        // Check that events have proper index
        assert_eq!(events[0].get("index").and_then(|v| v.as_u64()), Some(0));
        assert_eq!(events[1].get("index").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(events[2].get("index").and_then(|v| v.as_u64()), Some(2));

        // Check total_count
        assert_eq!(content.get("total_count").and_then(|v| v.as_u64()), Some(3));
    }

    #[tokio::test]
    async fn get_run_events_with_since_index_returns_incremental() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "test".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();

        // Add 5 events
        for i in 0..5 {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: format!("event-{}", i),
                        data: Some(json!({"num": i})),
                    },
                )
                .await
                .unwrap();
        }

        let service = hermetic_service(store, BackendKind::Codex).unwrap();

        // Get events after index 2 (should return events at indices 3 and 4)
        let args = json!({"run_id": run_id.0.to_string(), "since_index": 2})
            .as_object()
            .cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        let content = result
            .structured_content
            .expect("should have structured content");
        let events = content
            .get("events")
            .and_then(|v| v.as_array())
            .expect("should have events array");
        assert_eq!(events.len(), 2, "should return 2 events after index 2");

        // Verify the indices start from 3
        assert_eq!(events[0].get("index").and_then(|v| v.as_u64()), Some(3));
        assert_eq!(events[1].get("index").and_then(|v| v.as_u64()), Some(4));

        // total_count should still be 5 (total events in run)
        assert_eq!(content.get("total_count").and_then(|v| v.as_u64()), Some(5));
    }

    #[tokio::test]
    async fn get_run_events_with_no_events_returns_empty_array() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "test".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();

        let service = hermetic_service(store, BackendKind::Codex).unwrap();
        let args = json!({"run_id": run_id.0.to_string()}).as_object().cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        let content = result
            .structured_content
            .expect("should have structured content");
        let events = content
            .get("events")
            .and_then(|v| v.as_array())
            .expect("should have events array");
        assert!(events.is_empty());
        assert_eq!(content.get("total_count").and_then(|v| v.as_u64()), Some(0));
    }

    #[tokio::test]
    async fn get_run_events_with_invalid_run_id_returns_error() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

        let args = json!({"run_id": "00000000-0000-0000-0000-000000000000"})
            .as_object()
            .cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        // Should return error response
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn get_run_events_rejects_limit_outside_one_to_max_page() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

        for limit in [json!(0), json!(MAX_EVENTS_PAGE + 1), json!(-1), json!("10")] {
            let args = json!({"run_id": "00000000-0000-0000-0000-000000000000", "limit": limit})
                .as_object()
                .cloned();
            let err = service
                .handle_call("get-run-events", args.as_ref())
                .await
                .expect_err("out-of-range limit must be rejected");
            assert!(
                err.to_string()
                    .contains("limit must be an integer from 1 to 1000"),
                "limit={limit}: {err}"
            );
        }
    }

    #[tokio::test]
    async fn get_run_events_since_index_beyond_events_returns_empty() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "test".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();

        // Add 3 events
        for i in 0..3 {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: format!("event-{}", i),
                        data: None,
                    },
                )
                .await
                .unwrap();
        }

        let service = hermetic_service(store, BackendKind::Codex).unwrap();

        // Request events after index 10 (beyond the 3 events we have)
        let args = json!({"run_id": run_id.0.to_string(), "since_index": 10})
            .as_object()
            .cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        let content = result
            .structured_content
            .expect("should have structured content");
        let events = content
            .get("events")
            .and_then(|v| v.as_array())
            .expect("should have events array");
        assert!(
            events.is_empty(),
            "should return empty array when since_index is beyond events"
        );
        assert_eq!(content.get("total_count").and_then(|v| v.as_u64()), Some(3));
        assert_eq!(
            content.get("has_more").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[tokio::test]
    async fn get_run_events_snake_case_alias_works() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "test".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();

        let service = hermetic_service(store, BackendKind::Codex).unwrap();
        let args = json!({"run_id": run_id.0.to_string()}).as_object().cloned();

        // Should work with both naming conventions
        let result_dash = service.handle_call("get-run-events", args.as_ref()).await;
        let result_underscore = service.handle_call("get_run_events", args.as_ref()).await;

        assert!(result_dash.is_ok());
        assert!(result_underscore.is_ok());
    }

    fn agent_registry(home: &std::path::Path, file: &str, content: &str) -> Arc<AgentRegistry> {
        create_agent_file(&home.join(".codex"), file, content);
        let roots = vec![SkillRoot {
            root: home.join(".codex/agents"),
            source: SkillSource::Codex,
        }];
        Arc::new(AgentRegistry::discover_from_roots(&roots).unwrap())
    }

    fn run_id_of(result: &CallToolResult) -> String {
        result.structured_content.as_ref().unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// RT-1: a model-supplied `cli_binary` cannot name an arbitrary program.
    #[test]
    fn run_rejects_an_arbitrary_cli_binary() {
        with_harmless_env(|| async {
            let store = Arc::new(MemRunStore::new());
            let service = hermetic_service(store.clone(), BackendKind::Codex).unwrap();

            for payload in [
                "/tmp/payload.sh",
                "/tmp/claude",
                "./codex",
                "sh",
                // M288: a known stem with another extension or case is a
                // different program found on PATH.
                "claude.sh",
                "CODEX.bat",
                "Claude",
            ] {
                let args = json!({"prompt": "hi", "cli_binary": payload})
                    .as_object()
                    .cloned();
                let err = service
                    .handle_call("run-subagent", args.as_ref())
                    .await
                    .expect_err(payload);
                assert!(err.to_string().contains("not allowed"), "{payload}: {err}");
            }
            assert!(
                store.history(10).await.unwrap().is_empty(),
                "a rejected binary must not create a run"
            );

            // Known CLIs by bare name, and the operator's own setting, pass.
            assert_eq!(service.checked_cli_binary("claude").unwrap(), "claude");
            assert_eq!(service.checked_cli_binary("codex").unwrap(), "codex");
            assert_eq!(service.checked_cli_binary("true").unwrap(), "true");
        });
    }

    /// RT-2: `stop-run` kills the child spawned by an earlier call.
    #[cfg(unix)]
    #[test]
    fn stop_run_kills_the_cli_child() {
        with_harmless_env(|| async {
            let _sh = set_env_var("SKRILLS_CLI_BINARY", Some("/bin/sh"));
            let dir = tempdir().unwrap();
            let script = dir.path().join("hang.sh");
            fs::write(&script, "exec sleep 30\n").unwrap();
            let store = Arc::new(MemRunStore::new());
            let service = hermetic_service(store.clone(), BackendKind::Codex).unwrap();

            let args = json!({"prompt": script.to_str().unwrap()})
                .as_object()
                .cloned();
            let result = service
                .handle_call("run-subagent-async", args.as_ref())
                .await
                .unwrap();
            let run_id = run_id_of(&result);
            let id = RunId(uuid::Uuid::parse_str(&run_id).unwrap());

            let mut pid = None;
            for _ in 0..100 {
                let run = store.run(id).await.unwrap().unwrap();
                pid = run
                    .events
                    .iter()
                    .find(|e| e.kind == "start")
                    .and_then(|e| e.data.as_ref()?.get("pid")?.as_u64());
                if pid.is_some() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            let pid = pid.expect("child pid").to_string();
            let alive = || {
                std::process::Command::new("kill")
                    .args(["-0", &pid])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            };
            assert!(alive());

            let args = json!({"run_id": run_id}).as_object().cloned();
            let stopped = service
                .handle_call("stop-run", args.as_ref())
                .await
                .unwrap();
            assert_eq!(stopped.structured_content.unwrap()["stopped"], true);

            let mut gone = false;
            for _ in 0..100 {
                if !alive() {
                    gone = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            assert!(gone, "stop-run left child {pid} running");
        });
    }

    /// RT-3: the `timeout_ms` argument bounds a CLI run.
    #[cfg(unix)]
    #[test]
    fn run_honours_timeout_ms() {
        with_harmless_env(|| async {
            let _sh = set_env_var("SKRILLS_CLI_BINARY", Some("/bin/sh"));
            let dir = tempdir().unwrap();
            let script = dir.path().join("hang.sh");
            fs::write(&script, "exec sleep 30\n").unwrap();
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

            let started = std::time::Instant::now();
            let args = json!({"prompt": script.to_str().unwrap(), "timeout_ms": 300})
                .as_object()
                .cloned();
            let result = service
                .handle_call("run-subagent", args.as_ref())
                .await
                .unwrap();
            let content = result.structured_content.unwrap();
            assert_eq!(content["status"]["state"], "Failed", "{content}");
            assert!(status_message(&content).contains("timed out"), "{content}");
            assert!(started.elapsed() < std::time::Duration::from_secs(10));

            for bad in [json!(0), json!(300_001), json!("soon")] {
                let args = json!({"prompt": "hi", "timeout_ms": bad})
                    .as_object()
                    .cloned();
                assert!(service
                    .handle_call("run-subagent", args.as_ref())
                    .await
                    .is_err());
            }
        });
    }

    /// M262: an API run cannot honour a per-call timeout, so asking for one
    /// is an error instead of a silently longer wait.
    #[test]
    fn api_run_rejects_timeout_ms() {
        with_harmless_env(|| async {
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            let args = json!({"prompt": "hi", "execution_mode": "api", "timeout_ms": 1000})
                .as_object()
                .cloned();
            let err = service
                .handle_call("run-subagent", args.as_ref())
                .await
                .expect_err("timeout_ms in API mode must be rejected");
            assert!(err.to_string().contains("timeout_ms"), "{err}");
        });
    }

    /// M263: a CLI run cannot honour `output_schema`, so it is an error
    /// rather than a run that silently returns free-form text.
    #[test]
    fn cli_run_rejects_output_schema() {
        with_harmless_env(|| async {
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            let args = json!({"prompt": "hi", "output_schema": {"type": "object"}})
                .as_object()
                .cloned();
            let err = service
                .handle_call("run-subagent", args.as_ref())
                .await
                .expect_err("output_schema in CLI mode must be rejected");
            assert!(err.to_string().contains("output_schema"), "{err}");
        });
    }

    /// M263: a `template_id` the chosen backend does not offer is an error.
    #[test]
    fn run_rejects_an_unknown_template_id() {
        with_harmless_env(|| async {
            let service =
                hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
            let args = json!({"prompt": "hi", "template_id": "no-such-template"})
                .as_object()
                .cloned();
            let err = service
                .handle_call("run-subagent", args.as_ref())
                .await
                .expect_err("unknown template must be rejected");
            assert!(err.to_string().contains("no-such-template"), "{err}");

            let args = json!({"prompt": "hi", "template_id": "cli-default"})
                .as_object()
                .cloned();
            service
                .handle_call("run-subagent", args.as_ref())
                .await
                .expect("the CLI adapter's own template is accepted");
        });
    }

    /// M265: every run-addressed tool reports an unknown run the same way.
    #[tokio::test]
    async fn unknown_run_is_the_same_tool_error_for_status_events_and_stop() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let id = "11111111-1111-1111-1111-111111111111";
        let args = json!({"run_id": id}).as_object().cloned();
        for tool in ["get-run-status", "get-run-events", "stop-run"] {
            let result = service
                .handle_call(tool, args.as_ref())
                .await
                .unwrap_or_else(|e| panic!("{tool}: protocol error {e}"));
            assert_eq!(result.is_error, Some(true), "{tool}");
            assert_eq!(
                result.structured_content,
                Some(json!({"error": format!("run not found: {id}"), "run_id": id})),
                "{tool}"
            );
        }
    }

    /// M275 / M276: with no model configured, each API backend asks for a
    /// model id its endpoint serves.
    #[test]
    fn api_runs_default_to_models_their_endpoints_serve() {
        for (backend, key, base, expected) in [
            (
                "claude",
                "SKRILLS_CLAUDE_API_KEY",
                "SKRILLS_CLAUDE_BASE_URL",
                "claude-opus-5-5",
            ),
            (
                "codex",
                "SKRILLS_CODEX_API_KEY",
                "SKRILLS_CODEX_BASE_URL",
                "gpt-6-astra",
            ),
        ] {
            with_harmless_env(|| async move {
                use wiremock::matchers::{body_partial_json, method};
                use wiremock::{Mock, MockServer, ResponseTemplate};
                let server = MockServer::start().await;
                Mock::given(method("POST"))
                    .and(body_partial_json(json!({ "model": expected })))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                        "content": [{"type": "text", "text": "ok"}],
                        "stop_reason": "end_turn",
                        "choices": [{"message": {"content": "ok"}, "finish_reason": "stop"}]
                    })))
                    .expect(1)
                    .mount(&server)
                    .await;
                let _key = set_env_var(key, Some("test-key"));
                let _base = set_env_var(base, Some(&server.uri()));
                let _model_c = set_env_var("SKRILLS_CLAUDE_MODEL", None);
                let _model_o = set_env_var("SKRILLS_CODEX_MODEL", None);
                let service =
                    hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
                let args = json!({"prompt": "hi", "execution_mode": "api", "backend": backend})
                    .as_object()
                    .cloned();
                let result = service.handle_run(false, args.as_ref()).await.unwrap();
                let content = result.structured_content.unwrap();
                assert_eq!(
                    content["status"]["state"], "Succeeded",
                    "{backend}: {content}"
                );
            });
        }
    }

    /// M280: `limit` follows the schema (1 to 50); other values are errors.
    #[tokio::test]
    async fn history_limit_outside_the_schema_range_is_an_error() {
        let service = hermetic_service(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        for bad in [json!(0), json!(51), json!(1_000_000), json!("ten")] {
            let args = json!({ "limit": bad }).as_object().cloned();
            assert!(
                service
                    .handle_call("get-run-history", args.as_ref())
                    .await
                    .is_err(),
                "{bad}"
            );
        }
        for good in [json!({}), json!({"limit": 1}), json!({"limit": 50})] {
            let args = good.as_object().cloned();
            service
                .handle_call("get-run-history", args.as_ref())
                .await
                .unwrap_or_else(|e| panic!("{good}: {e}"));
        }
    }

    /// M281: get-run-events pages with `limit` and reports `has_more`.
    #[tokio::test]
    async fn get_run_events_pages_with_limit_and_has_more() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "p".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();
        for n in 0..5 {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: "tick".into(),
                        data: Some(json!({ "n": n })),
                    },
                )
                .await
                .unwrap();
        }
        let service = hermetic_service(store, BackendKind::Codex).unwrap();
        let page = |args: Value| {
            let service = &service;
            async move {
                service
                    .handle_call("get-run-events", args.as_object())
                    .await
                    .unwrap()
                    .structured_content
                    .unwrap()
            }
        };
        let first = page(json!({"run_id": run_id.to_string(), "limit": 2})).await;
        assert_eq!(first["events"].as_array().unwrap().len(), 2);
        assert_eq!(first["has_more"], true);
        let last = page(json!({"run_id": run_id.to_string(), "since_index": 2, "limit": 2})).await;
        assert_eq!(last["events"].as_array().unwrap().len(), 2);
        assert_eq!(last["events"][1]["index"], 4);
        assert_eq!(last["has_more"], false);
    }

    /// RT-18: a named agent's instructions reach the run.
    #[test]
    fn run_with_agent_id_sends_the_agent_instructions() {
        with_harmless_env(|| async {
            let _echo = set_env_var("SKRILLS_CLI_BINARY", Some("echo"));
            let tmp = tempdir().unwrap();
            let registry = agent_registry(
                tmp.path(),
                "cli-agent.md",
                "---\nname: cli-agent\ndescription: An agent with tools\ntools: Read\n---\n\nYou are a careful reviewer.",
            );
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Codex,
                registry,
            )
            .unwrap();

            let args = json!({"prompt": "check this", "agent_id": "cli-agent"})
                .as_object()
                .cloned();
            let result = service
                .handle_call("run-subagent", args.as_ref())
                .await
                .unwrap();
            let content = result.structured_content.unwrap();
            let completion = content["events"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["kind"] == "completion")
                .map(|e| e["data"]["text"].as_str().unwrap_or_default().to_string())
                .expect("completion event");
            assert!(
                completion.contains("You are a careful reviewer."),
                "{completion}"
            );
            assert!(completion.contains("check this"), "{completion}");
        });
    }

    #[test]
    fn compose_agent_prompt_keeps_a_bare_prompt_without_instructions() {
        assert_eq!(compose_agent_prompt("  ", "hi"), "hi");
        let composed = compose_agent_prompt("Be brief.", "hi");
        assert!(composed.starts_with("Be brief."));
        assert!(composed.ends_with("hi"));
    }

    /// RT-19: with no `backend` argument, a tool-using agent's model picks
    /// the CLI.
    #[test]
    fn tool_agent_without_backend_arg_uses_the_model_to_pick_the_cli() {
        with_harmless_env(|| async {
            let _no_cli = set_env_var("SKRILLS_CLI_BINARY", None);
            let tmp = tempdir().unwrap();
            let registry = agent_registry(
                tmp.path(),
                "opus-agent.md",
                "---\nname: opus-agent\ndescription: d\ntools: Read\nmodel: opus\n---\n\nBody.",
            );
            let service = SubagentService::with_store_and_registry(
                Arc::new(MemRunStore::new()),
                BackendKind::Codex,
                registry,
            )
            .unwrap();

            // Routing only: nothing is spawned.
            let (adapter, _) = service
                .route_for_agent("opus-agent", ExecutionMode::Cli, None, None, None)
                .unwrap();
            assert_eq!(adapter.backend(), BackendKind::Claude);

            let (adapter, _) = service
                .route_for_agent(
                    "opus-agent",
                    ExecutionMode::Cli,
                    None,
                    Some(BackendKind::Codex),
                    None,
                )
                .unwrap();
            assert_eq!(adapter.backend(), BackendKind::Codex);
        });
    }

    /// RT-31: a huge `since_index` does not overflow.
    #[tokio::test]
    async fn get_run_events_with_max_since_index_returns_empty() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "p".into(),
                template_id: None,
                output_schema: None,
                async_mode: true,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();
        let service = SubagentService::with_store_and_registry_with_config(
            store,
            BackendKind::Codex,
            Arc::new(AgentRegistry::discover_from_roots(&[]).unwrap()),
            SubagentsFileConfig::default(),
        )
        .unwrap();
        let args = json!({"run_id": run_id.to_string(), "since_index": u64::MAX})
            .as_object()
            .cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();
        let content = result.structured_content.unwrap();
        assert_eq!(content["events"].as_array().unwrap().len(), 0);
    }

    /// RT-32: each handler's structured output has every key its advertised
    /// output schema requires, so a validating client accepts it.
    #[tokio::test]
    async fn stop_and_history_outputs_match_their_schemas() {
        let store = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "p".into(),
                template_id: None,
                output_schema: None,
                async_mode: true,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();
        let service = SubagentService::with_store_and_registry_with_config(
            store,
            BackendKind::Codex,
            Arc::new(AgentRegistry::discover_from_roots(&[]).unwrap()),
            SubagentsFileConfig::default(),
        )
        .unwrap();
        let tools = service.tools();

        for (tool, args) in [
            ("stop-run", json!({"run_id": run_id.to_string()})),
            ("get-run-history", json!({})),
        ] {
            let schema = tools
                .iter()
                .find(|t| t.name == tool)
                .and_then(|t| t.output_schema.clone())
                .unwrap_or_else(|| panic!("{tool} has an output schema"));
            let result = service.handle_call(tool, args.as_object()).await.unwrap();
            let output = result.structured_content.unwrap();
            for key in schema
                .get("required")
                .and_then(|r| r.as_array())
                .expect("required keys")
            {
                let key = key.as_str().unwrap();
                assert!(
                    output.get(key).is_some(),
                    "{tool} output lacks {key}: {output}"
                );
            }
        }
    }
}
