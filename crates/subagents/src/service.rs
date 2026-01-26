use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rmcp::model::{CallToolResult, Content, Tool};
use serde_json::{json, Map as JsonMap, Value};

use crate::backend::BackendAdapter;
use crate::backend::{
    claude::ClaudeAdapter,
    cli::{CliConfig, CodexCliAdapter},
    codex::CodexAdapter,
};
use crate::cli_detection::{
    cli_binary_from_client_env, cli_binary_from_exe_path, normalize_cli_binary, DEFAULT_CLI_BINARY,
};
use crate::registry::AgentRegistry;
use crate::settings::{backend_from_str, load_file_config, ExecutionMode, SubagentsFileConfig};
use crate::store::{default_store_path, BackendKind, RunId, RunRequest, RunStore, StateRunStore};
use crate::tool_schemas;

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
    default_backend: BackendKind,
    default_execution_mode: ExecutionMode,
    cli_binary: Option<String>,
    registry: Arc<AgentRegistry>,
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
            Arc::new(CodexAdapter::new("gpt-5-codex".into())?),
        );
        adapters.insert(
            BackendKind::Claude,
            Arc::new(ClaudeAdapter::new("claude-code".into())?),
        );

        let default_execution_mode = file_config
            .execution_mode
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        let cli_binary = normalize_cli_binary(file_config.cli_binary);

        Ok(Self {
            store,
            adapters,
            default_backend,
            default_execution_mode,
            cli_binary,
            registry,
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
    /// 3. Backend hint (`Codex` -> "codex", `Claude` -> "claude")
    /// 4. Default CLI binary from service configuration/environment detection
    fn cli_adapter_for(
        &self,
        cli_binary_override: Option<String>,
        backend_hint: Option<BackendKind>,
    ) -> Arc<CodexCliAdapter> {
        let mut config = CliConfig::from_env();

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

        Arc::new(CodexCliAdapter::with_config(config))
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

    fn default_backend_from_env(&self) -> BackendKind {
        std::env::var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND")
            .ok()
            .as_deref()
            .map(backend_from_str)
            .unwrap_or_else(|| self.default_backend.clone())
    }

    fn default_cli_binary(&self) -> String {
        self.cli_binary
            .clone()
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
        let mut cli_templates = self.cli_adapter_for(None, None).list_templates().await?;
        templates.append(&mut cli_templates);
        Ok(CallToolResult {
            content: vec![Content::text("listed subagents")],
            structured_content: Some(json!({"templates": templates})),
            is_error: Some(false),
            meta: None,
        })
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

        Ok(CallToolResult {
            content: vec![Content::text(format!("found {} agents", agents.len()))],
            structured_content: Some(json!({"agents": agents})),
            is_error: Some(false),
            meta: None,
        })
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
            .map(|s| s.to_string());

        // Determine backend from args (used for both CLI binary selection and API routing)
        let backend = args
            .get("backend")
            .and_then(|v| v.as_str())
            .map(backend_from_str)
            .unwrap_or_else(|| self.default_backend_from_env());

        // Smart routing: if agent_id is specified, use agent-based routing
        let adapter: Arc<dyn BackendAdapter> = if let Some(agent_name) = agent_id {
            self.route_for_agent(
                agent_name,
                execution_mode,
                cli_binary_override.clone(),
                Some(backend.clone()),
            )?
        } else if matches!(execution_mode, ExecutionMode::Cli) {
            self.cli_adapter_for(cli_binary_override, Some(backend))
        } else {
            // API mode: use backend from args or default.
            self.adapter_for(Some(backend))?
        };

        let request = RunRequest {
            backend: adapter.backend(),
            prompt,
            template_id,
            output_schema,
            async_mode: stream,
            tracing,
        };
        let run_id = adapter.run(request, self.store.clone()).await?;
        let status = adapter.status(run_id, self.store.clone()).await?;
        Ok(CallToolResult {
            content: vec![Content::text(format!("run_id={run_id}"))],
            structured_content: Some(json!({
                "run_id": run_id,
                "status": status,
                "events": self.store.run(run_id).await?.map(|r| r.events).unwrap_or_default()
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    /// Route to appropriate adapter based on agent configuration.
    ///
    /// Returns:
    /// - CLI adapter if agent requires tools (spawns CLI subprocess)
    /// - API adapter if agent doesn't require tools
    /// - Error if agent not found
    ///
    /// When routing to CLI, the `backend_hint` takes precedence over model-based detection
    /// for determining which CLI binary to spawn.
    fn route_for_agent(
        &self,
        agent_name: &str,
        execution_mode: ExecutionMode,
        cli_binary_override: Option<String>,
        backend_hint: Option<BackendKind>,
    ) -> Result<Arc<dyn BackendAdapter>> {
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
            // Use backend hint to determine CLI binary, falling back to model-based detection
            let cli_backend = backend_hint
                .unwrap_or_else(|| self.backend_for_model(agent.config.model.as_deref()));
            return Ok(self.cli_adapter_for(cli_binary_override, Some(cli_backend)));
        }

        // Agent doesn't require tools - use API adapter
        // Determine which API backend to use based on agent's model
        let backend = self.backend_for_model(agent.config.model.as_deref());
        self.adapter_for(Some(backend))
    }

    /// Determine the backend kind based on the model name.
    fn backend_for_model(&self, model: Option<&str>) -> BackendKind {
        match model {
            Some(m)
                if m.contains("claude")
                    || m.contains("sonnet")
                    || m.contains("opus")
                    || m.contains("haiku") =>
            {
                BackendKind::Claude
            }
            Some(m)
                if m.contains("gpt")
                    || m.contains("codex")
                    || m.contains("o1")
                    || m.contains("o3") =>
            {
                BackendKind::Codex
            }
            // Default to the service's default backend
            _ => self.default_backend_from_env(),
        }
    }

    async fn handle_status(&self, args: Option<&JsonMap<String, Value>>) -> Result<CallToolResult> {
        let args = args.ok_or_else(|| anyhow!("arguments required"))?;
        let run_id_val = args
            .get("run_id")
            .ok_or_else(|| anyhow!("run_id is required"))?;
        let run_id = run_id_from_value(run_id_val)?;
        let status = self.store.status(run_id).await?;
        Ok(CallToolResult {
            content: vec![Content::text("status")],
            structured_content: Some(json!({
                "run_id": run_id,
                "status": status,
                "events": self.store.run(run_id).await?.map(|r| r.events).unwrap_or_default()
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    async fn handle_stop(&self, args: Option<&JsonMap<String, Value>>) -> Result<CallToolResult> {
        let args = args.ok_or_else(|| anyhow!("arguments required"))?;
        let run_id = run_id_from_value(
            args.get("run_id")
                .ok_or_else(|| anyhow!("run_id is required"))?,
        )?;
        let stopped = self.store.stop(run_id).await?;
        Ok(CallToolResult {
            content: vec![Content::text("stopped")],
            structured_content: Some(json!({"run_id": run_id, "stopped": stopped})),
            is_error: Some(false),
            meta: None,
        })
    }

    async fn handle_history(
        &self,
        args: Option<&JsonMap<String, Value>>,
    ) -> Result<CallToolResult> {
        let limit = args
            .and_then(|m| m.get("limit"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(20);
        let runs = self.store.history(limit).await?;
        Ok(CallToolResult {
            content: vec![Content::text("history")],
            structured_content: Some(json!({"runs": runs})),
            is_error: Some(false),
            meta: None,
        })
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
            .map(|v| v as usize);

        // Fetch the run record
        let record = match self.store.run(run_id).await? {
            Some(r) => r,
            None => {
                return Ok(CallToolResult {
                    content: vec![Content::text(format!("run not found: {}", run_id))],
                    structured_content: Some(json!({
                        "error": format!("run not found: {}", run_id),
                        "run_id": run_id.to_string()
                    })),
                    is_error: Some(true),
                    meta: None,
                });
            }
        };

        let total_count = record.events.len();

        // Determine the slice of events to return
        let (events_to_return, start_index) = match since_index {
            Some(idx) => {
                // Return events after the given index
                let start = idx + 1;
                if start >= total_count {
                    (Vec::new(), start)
                } else {
                    (record.events[start..].to_vec(), start)
                }
            }
            None => {
                // Return all events
                (record.events.clone(), 0)
            }
        };

        // Format events with their indices
        let events_json: Vec<Value> = events_to_return
            .iter()
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

        let has_more = false; // For now, we return all matching events

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "events: {} of {} total",
                events_json.len(),
                total_count
            ))],
            structured_content: Some(json!({
                "run_id": run_id.to_string(),
                "events": events_json,
                "total_count": total_count,
                "has_more": has_more
            })),
            is_error: Some(false),
            meta: None,
        })
    }

    async fn handle_transcript(&self) -> Result<CallToolResult> {
        Ok(CallToolResult {
            content: vec![Content::text("secure transcripts are not yet implemented")],
            structured_content: Some(json!({"status": "unimplemented"})),
            is_error: Some(false),
            meta: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MemRunStore, RunEvent, RunRequest, RunState};
    use skrills_discovery::{SkillRoot, SkillSource};
    use std::env;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::tempdir;
    use time::OffsetDateTime;

    fn create_agent_file(dir: &std::path::Path, name: &str, content: &str) {
        let agents_dir = dir.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(agents_dir.join(name), content).unwrap();
    }

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
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

    #[tokio::test]
    async fn tools_include_core_and_extended() {
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let tools = service.tools();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"run-subagent"));
        assert!(names.contains(&"run-subagent-async"));
        assert!(names.contains(&"download-transcript-secure"));
    }

    #[tokio::test]
    async fn tools_include_list_agents() {
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
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

    #[tokio::test]
    async fn run_and_status_round_trip() {
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
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
        let status = service.store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Running);
    }

    #[tokio::test]
    async fn snake_case_aliases_are_supported() {
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let args = json!({"prompt": "hello", "backend": "codex"})
            .as_object()
            .cloned();
        let result = service
            .handle_call("run_subagent", args.as_ref())
            .await
            .unwrap();
        assert!(result.structured_content.is_some());
    }

    // =============================================================
    // Tests for smart routing (Task 4)
    // =============================================================

    #[tokio::test]
    async fn run_subagent_tool_schema_includes_agent_id() {
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
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

        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
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

        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        assert_eq!(service.default_cli_binary(), "claude");
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

        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
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

        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
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
                SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex)
                    .unwrap();
            service.cli_adapter_for(None, Some(BackendKind::Codex))
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

        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

        // When backend hint is Codex, CLI binary should be "codex"
        let codex_adapter = service.cli_adapter_for(None, Some(BackendKind::Codex));
        assert_eq!(
            codex_adapter.config().binary,
            "codex",
            "Backend hint Codex should select 'codex' binary"
        );

        // When backend hint is Claude, CLI binary should be "claude"
        let claude_adapter = service.cli_adapter_for(None, Some(BackendKind::Claude));
        assert_eq!(
            claude_adapter.config().binary,
            "claude",
            "Backend hint Claude should select 'claude' binary"
        );

        // When no backend hint, should fall back to default detection
        let default_adapter = service.cli_adapter_for(None, None);
        // Default is "claude" when no env vars are set (DEFAULT_CLI_BINARY)
        assert_eq!(
            default_adapter.config().binary,
            "claude",
            "No backend hint should fall back to default"
        );

        // When backend hint is Copilot (via Other), should fall back to default
        // (Copilot CLI doesn't support subagent execution)
        let copilot_adapter =
            service.cli_adapter_for(None, Some(BackendKind::Other("copilot".to_string())));
        assert_eq!(
            copilot_adapter.config().binary,
            "claude",
            "Backend hint Copilot should fall back to default (unsupported)"
        );
    }

    #[tokio::test]
    async fn run_without_agent_id_defaults_to_cli_mode() {
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let args = json!({"prompt": "hi"}).as_object().cloned();
        let result = service.handle_run(false, args.as_ref()).await.unwrap();

        // Should succeed using CLI mode by default.
        assert!(result.structured_content.is_some());
        let content = result.structured_content.unwrap();
        assert!(content.get("run_id").is_some());
        let message = content
            .get("status")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str());
        assert_eq!(message, Some("spawning CLI process"));
    }

    #[tokio::test]
    async fn run_with_execution_mode_api_uses_api_adapter() {
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let args = json!({"prompt": "hi", "execution_mode": "api", "backend": "codex"})
            .as_object()
            .cloned();
        let result = service.handle_run(false, args.as_ref()).await.unwrap();

        let content = result.structured_content.unwrap();
        let message = content
            .get("status")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str());
        assert_eq!(message, Some("dispatched"));
    }

    #[tokio::test]
    async fn run_with_agent_id_no_tools_routes_to_api() {
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

        // Should succeed - routed to API adapter
        assert!(result.structured_content.is_some());
        let content = result.structured_content.unwrap();
        assert!(content.get("run_id").is_some());
        let message = content
            .get("status")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str());
        assert_eq!(message, Some("dispatched"));
    }

    #[tokio::test]
    async fn run_with_agent_id_with_tools_routes_to_cli() {
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

        // Should succeed - routed to CLI adapter (though spawn may fail if codex isn't installed)
        // The important thing is that routing works and returns a run_id
        assert!(result.is_ok(), "should route to CLI adapter: {:?}", result);
        let content = result.unwrap().structured_content.unwrap();
        assert!(content.get("run_id").is_some(), "should have run_id");
    }

    #[tokio::test]
    async fn run_with_agent_id_with_tools_execution_mode_api_still_uses_cli() {
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

        let content = result.structured_content.unwrap();
        let message = content
            .get("status")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str());
        assert_eq!(message, Some("spawning CLI process"));
    }

    #[tokio::test]
    async fn run_with_nonexistent_agent_id_errors() {
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
    }

    #[tokio::test]
    async fn run_with_agent_id_ignores_backend_param() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Create an agent without tools
        create_agent_file(
            &home.join(".codex"),
            "my-agent.md",
            r#"---
name: my-agent
description: Test agent
model: claude
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

        // Even with explicit backend=codex, agent_id takes precedence
        let args = json!({"prompt": "hi", "agent_id": "my-agent", "backend": "codex"})
            .as_object()
            .cloned();
        let result = service.handle_run(false, args.as_ref()).await.unwrap();

        // Should succeed - agent_id route takes priority
        assert!(result.structured_content.is_some());
    }

    #[tokio::test]
    async fn run_async_with_agent_id_routes_correctly() {
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
    }

    #[tokio::test]
    async fn handle_call_with_unknown_tool_returns_error() {
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

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
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

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
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();
        let tools = service.tools();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            names.contains(&"get-run-events"),
            "should have get-run-events tool"
        );
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

        let service = SubagentService::with_store(store, BackendKind::Codex).unwrap();
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

        let service = SubagentService::with_store(store, BackendKind::Codex).unwrap();

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
            })
            .await
            .unwrap();

        let service = SubagentService::with_store(store, BackendKind::Codex).unwrap();
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
        let service =
            SubagentService::with_store(Arc::new(MemRunStore::new()), BackendKind::Codex).unwrap();

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

        let service = SubagentService::with_store(store, BackendKind::Codex).unwrap();

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
            })
            .await
            .unwrap();

        let service = SubagentService::with_store(store, BackendKind::Codex).unwrap();
        let args = json!({"run_id": run_id.0.to_string()}).as_object().cloned();

        // Should work with both naming conventions
        let result_dash = service.handle_call("get-run-events", args.as_ref()).await;
        let result_underscore = service.handle_call("get_run_events", args.as_ref()).await;

        assert!(result_dash.is_ok());
        assert!(result_underscore.is_ok());
    }
}
