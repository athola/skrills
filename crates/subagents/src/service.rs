use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rmcp::model::{object, JsonObject};
use rmcp::model::{CallToolResult, Content, Tool};
use serde_json::{json, Map as JsonMap, Value};

use crate::backend::BackendAdapter;
use crate::backend::{claude::ClaudeAdapter, codex::CodexAdapter};
use crate::registry::AgentRegistry;
use crate::store::{default_store_path, BackendKind, RunId, RunRequest, RunStore, StateRunStore};

fn backend_from_str(raw: &str) -> BackendKind {
    if raw.eq_ignore_ascii_case("codex")
        || raw.eq_ignore_ascii_case("gpt")
        || raw.eq_ignore_ascii_case("openai")
    {
        BackendKind::Codex
    } else if raw.eq_ignore_ascii_case("claude") || raw.eq_ignore_ascii_case("anthropic") {
        BackendKind::Claude
    } else {
        BackendKind::Other(raw.to_string())
    }
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
    default_backend: BackendKind,
    registry: Arc<AgentRegistry>,
}

impl SubagentService {
    pub fn new() -> Result<Self> {
        let store = Arc::new(StateRunStore::new(default_store_path()?)?);
        let default_backend = std::env::var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND")
            .ok()
            .as_deref()
            .map(backend_from_str)
            .unwrap_or(BackendKind::Codex);
        Self::with_store(store, default_backend)
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
        let mut adapters: HashMap<BackendKind, Arc<dyn BackendAdapter>> = HashMap::new();
        adapters.insert(
            BackendKind::Codex,
            Arc::new(CodexAdapter::new("gpt-5-codex".into())?),
        );
        adapters.insert(
            BackendKind::Claude,
            Arc::new(ClaudeAdapter::new("claude-code".into())?),
        );
        Ok(Self {
            store,
            adapters,
            default_backend,
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

    pub fn tools(&self) -> Vec<Tool> {
        let run_schema: Arc<JsonObject> = Arc::new(object(json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {"type": "string", "description": "User instruction"},
                "backend": {"type": "string", "description": "codex|claude|other"},
                "template_id": {"type": "string"},
                "output_schema": {"type": "object"},
                "tracing": {"type": "boolean"},
                "stream": {"type": "boolean"},
                "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 300000}
            }
        })));
        let run_id_schema: Arc<JsonObject> = Arc::new(object(json!({
            "type": "object",
            "required": ["run_id"],
            "properties": {"run_id": {"type": "string"}}
        })));
        let history_schema: Arc<JsonObject> = Arc::new(object(json!({
            "type": "object",
            "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 50}},
        })));

        let run_output_schema: Arc<JsonObject> = Arc::new(object(json!({
            "type": "object",
            "required": ["run_id"],
            "properties": {
                "run_id": {"type": "string"},
                "status": {"type": "object"},
                "events": {"type": "array", "items": {"type": "object"}}
            }
        })));
        let list_output_schema: Arc<JsonObject> = Arc::new(object(json!({
            "type": "object",
            "properties": {"templates": {"type": "array", "items": {"type": "object"}}}
        })));
        let agents_output_schema: Arc<JsonObject> = Arc::new(object(json!({
            "type": "object",
            "properties": {
                "agents": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "description": {"type": "string"},
                            "tools": {"type": "array", "items": {"type": "string"}},
                            "model": {"type": "string"},
                            "source": {"type": "string"},
                            "path": {"type": "string"},
                            "requires_cli": {"type": "boolean"}
                        }
                    }
                }
            }
        })));

        let mut tools = vec![
            Tool {
                name: "list-subagents".into(),
                title: Some("List subagent templates".into()),
                description: Some("List available subagent templates and capabilities".into()),
                input_schema: Arc::new(JsonObject::default()),
                output_schema: Some(list_output_schema),
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "list-agents".into(),
                title: Some("List discovered agents".into()),
                description: Some(
                    "List all discovered agent definitions from standard locations".into(),
                ),
                input_schema: Arc::new(JsonObject::default()),
                output_schema: Some(agents_output_schema),
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "run-subagent".into(),
                title: Some("Run a subagent".into()),
                description: Some("Run a subagent with optional backend/template selection".into()),
                input_schema: run_schema.clone(),
                output_schema: Some(run_output_schema.clone()),
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "get-run-status".into(),
                title: Some("Get subagent run status".into()),
                description: Some("Fetch status for a run".into()),
                input_schema: run_id_schema.clone(),
                output_schema: Some(run_output_schema.clone()),
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "stop-run".into(),
                title: Some("Stop a running subagent".into()),
                description: Some("Attempt to cancel a running subagent".into()),
                input_schema: run_id_schema.clone(),
                output_schema: Some(run_output_schema.clone()),
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "get-run-history".into(),
                title: Some("Recent runs".into()),
                description: Some("Return recent subagent runs".into()),
                input_schema: history_schema.clone(),
                output_schema: Some(run_output_schema.clone()),
                annotations: None,
                icons: None,
                meta: None,
            },
        ];

        // Codex-only extended tools
        tools.push(Tool {
            name: "run-subagent-async".into(),
            title: Some("Run subagent asynchronously".into()),
            description: Some("Start background run (Codex-capable backends).".into()),
            input_schema: run_schema,
            output_schema: Some(run_output_schema.clone()),
            annotations: None,
            icons: None,
            meta: None,
        });
        tools.push(Tool {
            name: "get-async-status".into(),
            title: Some("Status for async run".into()),
            description: Some("Fetch status for async runs".into()),
            input_schema: run_id_schema,
            output_schema: Some(run_output_schema.clone()),
            annotations: None,
            icons: None,
            meta: None,
        });
        tools.push(Tool {
            name: "download-transcript-secure".into(),
            title: Some("Download secure transcript".into()),
            description: Some("Fetch encrypted reasoning transcript (Codex only)".into()),
            input_schema: Arc::new(JsonObject::default()),
            output_schema: None,
            annotations: None,
            icons: None,
            meta: None,
        });
        tools
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
        let backend = args
            .get("backend")
            .and_then(|v| v.as_str())
            .map(backend_from_str);
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

        let adapter = self.adapter_for(backend)?;
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
    use crate::store::{MemRunStore, RunState};
    use skrills_discovery::{SkillRoot, SkillSource};
    use std::fs;
    use tempfile::tempdir;

    fn create_agent_file(dir: &std::path::Path, name: &str, content: &str) {
        let agents_dir = dir.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(agents_dir.join(name), content).unwrap();
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
        let args = json!({"prompt": "hi", "backend": "codex"})
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
}
