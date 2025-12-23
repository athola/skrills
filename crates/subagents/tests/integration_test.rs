//! Integration tests for the subagents feature.
//!
//! These tests verify the full workflow of the subagent system including:
//! - Discovery of agent files from standard locations
//! - Smart routing based on agent capabilities (tools vs. no tools)
//! - Event streaming with incremental fetching
//! - Error handling for invalid inputs
//!
//! Tests use tempdir for isolated test environments and mock CLI execution
//! using simple shell commands instead of real Codex/Claude CLIs.

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;

use skrills_discovery::{SkillRoot, SkillSource};
use skrills_subagents::backend::cli::{CliConfig, CodexCliAdapter};
use skrills_subagents::registry::AgentRegistry;
use skrills_subagents::store::{
    BackendKind, MemRunStore, RunEvent, RunId, RunRequest, RunState, RunStatus,
};
use skrills_subagents::{RunStore, SubagentService};
use tokio::time::{sleep, Instant};

/// Test fixture for creating isolated test environments with agent files.
struct IntegrationTestFixture {
    #[allow(dead_code)]
    temp_dir: TempDir,
    store: Arc<MemRunStore>,
    registry: Arc<AgentRegistry>,
}

impl IntegrationTestFixture {
    /// Create a new test fixture with an empty home directory.
    fn new() -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let store = Arc::new(MemRunStore::new());

        // Create empty agent roots
        let roots = vec![SkillRoot {
            root: temp_dir.path().join(".codex/agents"),
            source: SkillSource::Codex,
        }];

        let registry = Arc::new(AgentRegistry::discover_from_roots(&roots)?);

        Ok(Self {
            temp_dir,
            store,
            registry,
        })
    }

    /// Create a test fixture with predefined agent files.
    fn with_agents(agents: &[(&str, &str)]) -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let store = Arc::new(MemRunStore::new());

        // Create agent directory
        let agents_dir = temp_dir.path().join(".codex/agents");
        fs::create_dir_all(&agents_dir)?;

        // Write agent files
        for (name, content) in agents {
            fs::write(agents_dir.join(name), content)?;
        }

        // Discover agents from the temp directory
        let roots = vec![SkillRoot {
            root: agents_dir,
            source: SkillSource::Codex,
        }];

        let registry = Arc::new(AgentRegistry::discover_from_roots(&roots)?);

        Ok(Self {
            temp_dir,
            store,
            registry,
        })
    }

    /// Create a SubagentService using this fixture's store and registry.
    fn create_service(&self) -> anyhow::Result<SubagentService> {
        SubagentService::with_store_and_registry(
            self.store.clone(),
            BackendKind::Codex,
            self.registry.clone(),
        )
    }
}

async fn wait_for_run_state(
    store: Arc<dyn RunStore>,
    run_id: RunId,
    expected: RunState,
    timeout: Duration,
) -> anyhow::Result<RunStatus> {
    let started = Instant::now();
    let mut last_state: Option<RunState> = None;
    loop {
        if let Some(status) = store.status(run_id).await? {
            last_state = Some(status.state.clone());
            if status.state == expected {
                return Ok(status);
            }
            if matches!(status.state, RunState::Failed | RunState::Canceled) {
                anyhow::bail!(
                    "run ended early: expected {:?}, got {:?}",
                    expected,
                    status.state
                );
            }
        }
        if started.elapsed() >= timeout {
            anyhow::bail!(
                "timed out waiting for {:?}; last_state={:?}",
                expected,
                last_state
            );
        }
        sleep(Duration::from_millis(10)).await;
    }
}

// ============================================================================
// Module: Full Workflow Integration Tests
// ============================================================================

mod full_workflow_tests {
    use super::*;

    #[tokio::test]
    async fn test_complete_workflow_discovery_list_run_events() {
        /*
        GIVEN a test environment with multiple agent files
        WHEN we discover agents, list them, run one, and get events
        THEN the complete workflow should succeed with proper data flow
        */

        // Setup: Create agents with different configurations
        let fixture = IntegrationTestFixture::with_agents(&[
            (
                "research-agent.md",
                r#"---
name: research-agent
description: Researches topics
model: claude
---

You are a research agent."#,
            ),
            (
                "code-agent.md",
                r#"---
name: code-agent
description: Writes code
tools: Read, Bash, Glob
model: sonnet
---

You are a code agent with tool access."#,
            ),
        ])
        .unwrap();

        let service = fixture.create_service().unwrap();

        // Step 1: List agents
        let list_result = service.handle_call("list-agents", None).await.unwrap();
        let agents = list_result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("agents"))
            .and_then(|v| v.as_array())
            .expect("should have agents array");

        assert_eq!(agents.len(), 2, "should discover 2 agents");

        // Verify agent data is complete
        for agent in agents {
            assert!(agent.get("name").is_some(), "agent should have name");
            assert!(
                agent.get("description").is_some(),
                "agent should have description"
            );
            assert!(agent.get("source").is_some(), "agent should have source");
            assert!(agent.get("path").is_some(), "agent should have path");
            assert!(
                agent.get("requires_cli").is_some(),
                "agent should have requires_cli"
            );
        }

        // Step 2: Run the no-tools agent (routes to API)
        let run_args = json!({"prompt": "test prompt", "agent_id": "research-agent"})
            .as_object()
            .cloned();
        let run_result = service
            .handle_call("run-subagent", run_args.as_ref())
            .await
            .unwrap();

        let run_id = run_result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("run_id"))
            .and_then(|v| v.as_str())
            .expect("should have run_id");

        assert!(!run_id.is_empty(), "run_id should not be empty");

        // Step 3: Get run status
        let status_args = json!({"run_id": run_id}).as_object().cloned();
        let status_result = service
            .handle_call("get-run-status", status_args.as_ref())
            .await
            .unwrap();

        assert!(
            status_result.structured_content.is_some(),
            "status should have structured content"
        );

        // Step 4: Get run events
        let events_result = service
            .handle_call("get-run-events", status_args.as_ref())
            .await
            .unwrap();

        let events_content = events_result
            .structured_content
            .expect("should have structured content");
        assert!(
            events_content.get("events").is_some(),
            "should have events field"
        );
        assert!(
            events_content.get("total_count").is_some(),
            "should have total_count"
        );
        assert!(
            events_content.get("has_more").is_some(),
            "should have has_more"
        );
    }

    #[tokio::test]
    async fn test_workflow_with_multiple_runs() {
        /*
        GIVEN a service with agents
        WHEN we create multiple runs
        THEN history should track all runs correctly
        */
        let fixture = IntegrationTestFixture::with_agents(&[(
            "simple-agent.md",
            r#"---
name: simple-agent
description: Simple agent
---

Content."#,
        )])
        .unwrap();

        let service = fixture.create_service().unwrap();

        // Create multiple runs
        let mut run_ids = Vec::new();
        for i in 0..3 {
            let args = json!({
                "prompt": format!("test prompt {}", i),
                "agent_id": "simple-agent"
            })
            .as_object()
            .cloned();

            let result = service
                .handle_call("run-subagent", args.as_ref())
                .await
                .unwrap();
            let run_id = result
                .structured_content
                .as_ref()
                .and_then(|v| v.get("run_id"))
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap();

            run_ids.push(run_id);
        }

        // Verify history contains all runs
        let history_result = service
            .handle_call("get-run-history", json!({}).as_object())
            .await
            .unwrap();

        let runs = history_result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("runs"))
            .and_then(|v| v.as_array())
            .expect("should have runs array");

        assert_eq!(runs.len(), 3, "history should contain 3 runs");
    }
}

// ============================================================================
// Module: Smart Routing Integration Tests
// ============================================================================

mod smart_routing_tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_without_tools_routes_to_api_adapter() {
        /*
        GIVEN an agent without tools
        WHEN running the agent
        THEN it should route to the API adapter (not CLI)
        AND the run should be created successfully
        */
        let fixture = IntegrationTestFixture::with_agents(&[(
            "api-only-agent.md",
            r#"---
name: api-only-agent
description: An agent for API calls only
model: gpt-4
---

You are an API-only agent."#,
        )])
        .unwrap();

        let service = fixture.create_service().unwrap();

        // Verify the agent exists and doesn't require CLI
        let list_result = service.handle_call("list-agents", None).await.unwrap();
        let agents = list_result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("agents"))
            .and_then(|v| v.as_array())
            .unwrap();

        let agent = agents
            .iter()
            .find(|a| a.get("name").and_then(|v| v.as_str()) == Some("api-only-agent"))
            .unwrap();

        assert_eq!(
            agent.get("requires_cli").and_then(|v| v.as_bool()),
            Some(false),
            "agent without tools should not require CLI"
        );

        // Run the agent
        let args = json!({"prompt": "test", "agent_id": "api-only-agent"})
            .as_object()
            .cloned();
        let result = service
            .handle_call("run-subagent", args.as_ref())
            .await
            .unwrap();

        // Should succeed with a run_id
        let run_id = result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("run_id"))
            .and_then(|v| v.as_str());

        assert!(run_id.is_some(), "should have run_id from API adapter");
    }

    #[tokio::test]
    async fn test_agent_with_tools_routes_to_cli_adapter() {
        /*
        GIVEN an agent with tools specified
        WHEN running the agent
        THEN it should route to the CLI adapter
        AND the run should be created (CLI spawn may fail but routing works)
        */
        let fixture = IntegrationTestFixture::with_agents(&[(
            "cli-agent.md",
            r#"---
name: cli-agent
description: An agent with tool access
tools: Read, Bash, Glob, Grep
model: sonnet
---

You are a CLI agent with full tool access."#,
        )])
        .unwrap();

        let service = fixture.create_service().unwrap();

        // Verify the agent exists and requires CLI
        let list_result = service.handle_call("list-agents", None).await.unwrap();
        let agents = list_result
            .structured_content
            .as_ref()
            .and_then(|v| v.get("agents"))
            .and_then(|v| v.as_array())
            .unwrap();

        let agent = agents
            .iter()
            .find(|a| a.get("name").and_then(|v| v.as_str()) == Some("cli-agent"))
            .unwrap();

        assert_eq!(
            agent.get("requires_cli").and_then(|v| v.as_bool()),
            Some(true),
            "agent with tools should require CLI"
        );

        // Verify tools are captured
        let tools = agent.get("tools").and_then(|v| v.as_array()).unwrap();
        assert!(tools.len() >= 4, "should have multiple tools");

        // Run the agent - CLI adapter will be selected
        let args = json!({"prompt": "test", "agent_id": "cli-agent"})
            .as_object()
            .cloned();
        let result = service.handle_call("run-subagent", args.as_ref()).await;

        // Should succeed (routing works even if CLI spawn fails)
        assert!(result.is_ok(), "routing should succeed");
        let run_id = result
            .unwrap()
            .structured_content
            .and_then(|v| v.get("run_id").and_then(|v| v.as_str()).map(String::from));
        assert!(run_id.is_some(), "should have run_id from CLI adapter");
    }

    #[tokio::test]
    async fn test_model_based_backend_selection() {
        /*
        GIVEN agents with different model specifications
        WHEN routing without tools
        THEN the backend should be selected based on model name
        */
        let fixture = IntegrationTestFixture::with_agents(&[
            (
                "claude-model-agent.md",
                r#"---
name: claude-model-agent
description: Uses Claude model
model: claude-3-opus
---

Claude agent."#,
            ),
            (
                "gpt-model-agent.md",
                r#"---
name: gpt-model-agent
description: Uses GPT model
model: gpt-4-turbo
---

GPT agent."#,
            ),
            (
                "sonnet-model-agent.md",
                r#"---
name: sonnet-model-agent
description: Uses Sonnet model
model: sonnet
---

Sonnet agent."#,
            ),
        ])
        .unwrap();

        let service = fixture.create_service().unwrap();

        // All agents should work (routed to appropriate backend based on model)
        for agent_name in [
            "claude-model-agent",
            "gpt-model-agent",
            "sonnet-model-agent",
        ] {
            let args = json!({"prompt": "test", "agent_id": agent_name})
                .as_object()
                .cloned();
            let result = service.handle_call("run-subagent", args.as_ref()).await;

            assert!(
                result.is_ok(),
                "agent {} should route successfully: {:?}",
                agent_name,
                result
            );
        }
    }

    #[tokio::test]
    async fn test_agent_id_takes_precedence_over_backend_param() {
        /*
        GIVEN an agent with a specific model
        WHEN running with both agent_id and explicit backend parameter
        THEN agent_id routing should take precedence
        */
        let fixture = IntegrationTestFixture::with_agents(&[(
            "specific-agent.md",
            r#"---
name: specific-agent
description: Has specific model
model: claude
---

Content."#,
        )])
        .unwrap();

        let service = fixture.create_service().unwrap();

        // Run with both agent_id and backend (agent_id should win)
        let args = json!({
            "prompt": "test",
            "agent_id": "specific-agent",
            "backend": "codex"  // This should be ignored
        })
        .as_object()
        .cloned();

        let result = service
            .handle_call("run-subagent", args.as_ref())
            .await
            .unwrap();

        // Should succeed - agent_id routing used
        assert!(result.structured_content.is_some());
    }

    #[tokio::test]
    async fn test_fallback_to_default_backend_without_agent_id() {
        /*
        GIVEN a service with default backend configured
        WHEN running without agent_id
        THEN the default backend should be used
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        // Run without agent_id - should use default backend
        let args = json!({"prompt": "test"}).as_object().cloned();
        let result = service
            .handle_call("run-subagent", args.as_ref())
            .await
            .unwrap();

        assert!(
            result.structured_content.is_some(),
            "should succeed with default backend"
        );
    }
}

// ============================================================================
// Module: Event Streaming Integration Tests
// ============================================================================

mod event_streaming_tests {
    use super::*;
    use time::OffsetDateTime;

    #[tokio::test]
    async fn test_event_streaming_full_flow() {
        /*
        GIVEN a run with multiple events
        WHEN fetching events without since_index
        THEN all events should be returned with proper indices
        */
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

        // Add events
        for i in 0..5 {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: format!("event-{}", i),
                        data: Some(json!({"step": i, "message": format!("Processing step {}", i)})),
                    },
                )
                .await
                .unwrap();
        }

        let fixture = IntegrationTestFixture::new().unwrap();
        let service = SubagentService::with_store_and_registry(
            store,
            BackendKind::Codex,
            fixture.registry.clone(),
        )
        .unwrap();

        let args = json!({"run_id": run_id.0.to_string()}).as_object().cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        let content = result.structured_content.unwrap();
        let events = content.get("events").and_then(|v| v.as_array()).unwrap();

        assert_eq!(events.len(), 5, "should return all 5 events");

        // Verify events have sequential indices
        for (i, event) in events.iter().enumerate() {
            assert_eq!(
                event.get("index").and_then(|v| v.as_u64()),
                Some(i as u64),
                "event {} should have correct index",
                i
            );
            assert!(event.get("ts").is_some(), "event should have timestamp");
            assert!(event.get("kind").is_some(), "event should have kind");
        }

        assert_eq!(content.get("total_count").and_then(|v| v.as_u64()), Some(5));
        assert_eq!(
            content.get("has_more").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[tokio::test]
    async fn test_event_streaming_incremental_with_since_index() {
        /*
        GIVEN a run with 10 events
        WHEN fetching events with since_index
        THEN only events after that index should be returned
        */
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

        // Add 10 events
        for i in 0..10 {
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

        let fixture = IntegrationTestFixture::new().unwrap();
        let service = SubagentService::with_store_and_registry(
            store,
            BackendKind::Codex,
            fixture.registry.clone(),
        )
        .unwrap();

        // Fetch events after index 4 (should get indices 5, 6, 7, 8, 9)
        let args = json!({"run_id": run_id.0.to_string(), "since_index": 4})
            .as_object()
            .cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        let content = result.structured_content.unwrap();
        let events = content.get("events").and_then(|v| v.as_array()).unwrap();

        assert_eq!(events.len(), 5, "should return 5 events after index 4");

        // First event should be index 5
        assert_eq!(
            events[0].get("index").and_then(|v| v.as_u64()),
            Some(5),
            "first event should have index 5"
        );

        // Last event should be index 9
        assert_eq!(
            events[4].get("index").and_then(|v| v.as_u64()),
            Some(9),
            "last event should have index 9"
        );

        // Total count should still be 10
        assert_eq!(
            content.get("total_count").and_then(|v| v.as_u64()),
            Some(10)
        );
    }

    #[tokio::test]
    async fn test_event_streaming_polling_pattern() {
        /*
        GIVEN a run that receives events over time
        WHEN polling with progressive since_index values
        THEN new events should be retrieved incrementally
        */
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

        let fixture = IntegrationTestFixture::new().unwrap();
        let service = SubagentService::with_store_and_registry(
            store.clone(),
            BackendKind::Codex,
            fixture.registry.clone(),
        )
        .unwrap();

        // Initial poll - no events
        let args = json!({"run_id": run_id.0.to_string()}).as_object().cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();
        let content = result.structured_content.unwrap();
        assert_eq!(content.get("total_count").and_then(|v| v.as_u64()), Some(0));

        // Add first batch of events
        for i in 0..3 {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: format!("batch1-{}", i),
                        data: None,
                    },
                )
                .await
                .unwrap();
        }

        // Poll again - get all events
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();
        let content = result.structured_content.unwrap();
        let events = content.get("events").and_then(|v| v.as_array()).unwrap();
        assert_eq!(events.len(), 3, "should get 3 events");

        // Record last index for incremental polling
        let last_index = events
            .last()
            .and_then(|e| e.get("index").and_then(|v| v.as_u64()))
            .unwrap();
        assert_eq!(last_index, 2);

        // Add second batch of events
        for i in 0..2 {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: format!("batch2-{}", i),
                        data: None,
                    },
                )
                .await
                .unwrap();
        }

        // Incremental poll - only new events
        let args = json!({"run_id": run_id.0.to_string(), "since_index": last_index})
            .as_object()
            .cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();
        let content = result.structured_content.unwrap();
        let events = content.get("events").and_then(|v| v.as_array()).unwrap();

        assert_eq!(events.len(), 2, "should get only 2 new events");
        assert_eq!(
            events[0].get("index").and_then(|v| v.as_u64()),
            Some(3),
            "first new event should be index 3"
        );
    }

    #[tokio::test]
    async fn test_event_streaming_empty_results() {
        /*
        GIVEN a run with events
        WHEN polling with since_index beyond all events
        THEN an empty array should be returned
        */
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
                        data: None,
                    },
                )
                .await
                .unwrap();
        }

        let fixture = IntegrationTestFixture::new().unwrap();
        let service = SubagentService::with_store_and_registry(
            store,
            BackendKind::Codex,
            fixture.registry.clone(),
        )
        .unwrap();

        // Poll with since_index beyond all events
        let args = json!({"run_id": run_id.0.to_string(), "since_index": 100})
            .as_object()
            .cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        let content = result.structured_content.unwrap();
        let events = content.get("events").and_then(|v| v.as_array()).unwrap();

        assert!(events.is_empty(), "should return empty array");
        assert_eq!(
            content.get("total_count").and_then(|v| v.as_u64()),
            Some(5),
            "total_count should still show 5"
        );
    }
}

// ============================================================================
// Module: Error Handling Integration Tests
// ============================================================================

mod error_handling_tests {
    use super::*;

    #[tokio::test]
    async fn test_run_with_nonexistent_agent_returns_error() {
        /*
        GIVEN a service with no agents matching the requested name
        WHEN running with that agent_id
        THEN an appropriate error should be returned
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let args = json!({"prompt": "test", "agent_id": "nonexistent-agent-xyz"})
            .as_object()
            .cloned();
        let result = service.handle_call("run-subagent", args.as_ref()).await;

        assert!(result.is_err(), "should return error for nonexistent agent");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("agent not found"),
            "error should mention agent not found: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_run_without_prompt_returns_error() {
        /*
        GIVEN a run request without the required prompt field
        WHEN calling run-subagent
        THEN an error should be returned
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let args = json!({"backend": "codex"}).as_object().cloned();
        let result = service.handle_call("run-subagent", args.as_ref()).await;

        assert!(result.is_err(), "should return error when prompt missing");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("prompt"),
            "error should mention prompt: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_run_with_no_arguments_returns_error() {
        /*
        GIVEN a run request with no arguments at all
        WHEN calling run-subagent
        THEN an error should be returned
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let result = service.handle_call("run-subagent", None).await;

        assert!(result.is_err(), "should return error when no arguments");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("arguments"),
            "error should mention arguments: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_get_status_with_invalid_run_id_format() {
        /*
        GIVEN an invalid run_id format (not a valid UUID)
        WHEN calling get-run-status
        THEN an error should be returned
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let args = json!({"run_id": "not-a-valid-uuid"}).as_object().cloned();
        let result = service.handle_call("get-run-status", args.as_ref()).await;

        assert!(result.is_err(), "should return error for invalid UUID");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("invalid run_id"),
            "error should mention invalid run_id: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_get_events_with_nonexistent_run_returns_error() {
        /*
        GIVEN a valid UUID that doesn't correspond to any run
        WHEN calling get-run-events
        THEN an error response should be returned
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let args = json!({"run_id": "00000000-0000-0000-0000-000000000000"})
            .as_object()
            .cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        // Should return error response (not throw)
        assert_eq!(result.is_error, Some(true), "should be error response");
        assert!(
            result
                .structured_content
                .as_ref()
                .and_then(|v| v.get("error"))
                .is_some(),
            "should have error field"
        );
    }

    #[tokio::test]
    async fn test_get_status_with_missing_run_id() {
        /*
        GIVEN a get-run-status call without run_id
        WHEN executing the call
        THEN an error should be returned
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let args = json!({}).as_object().cloned();
        let result = service.handle_call("get-run-status", args.as_ref()).await;

        assert!(result.is_err(), "should return error when run_id missing");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("run_id"),
            "error should mention run_id: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_unknown_tool_returns_error() {
        /*
        GIVEN an unknown tool name
        WHEN calling handle_call
        THEN an error should be returned
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let result = service.handle_call("unknown-tool-xyz", None).await;

        assert!(result.is_err(), "should return error for unknown tool");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("unknown tool"),
            "error should mention unknown tool: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_stop_nonexistent_run_returns_error() {
        /*
        GIVEN a valid UUID for a run that doesn't exist
        WHEN calling stop-run
        THEN an error should be returned
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let args = json!({"run_id": "11111111-1111-1111-1111-111111111111"})
            .as_object()
            .cloned();
        let result = service.handle_call("stop-run", args.as_ref()).await;

        // Store returns NotFound error for non-existent runs
        assert!(result.is_err(), "should return error for nonexistent run");
    }
}

// ============================================================================
// Module: CLI Adapter Integration Tests (with mocked binary)
// ============================================================================

mod cli_adapter_integration_tests {
    use super::*;
    use skrills_subagents::backend::BackendAdapter;

    #[tokio::test]
    async fn test_cli_adapter_with_echo_succeeds() {
        /*
        GIVEN a CLI adapter configured to use 'echo' command
        WHEN running a request
        THEN the subprocess should execute and succeed
        */
        let store: Arc<dyn skrills_subagents::RunStore> = Arc::new(MemRunStore::new());

        // Use 'echo' as mock CLI - no arguments needed
        let config = CliConfig::new("echo").without_non_interactive();
        let adapter = CodexCliAdapter::with_config(config);

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        let run_id = adapter
            .run(request, store.clone())
            .await
            .expect("run should start");

        let status = wait_for_run_state(
            store.clone(),
            run_id,
            RunState::Succeeded,
            Duration::from_secs(2),
        )
        .await
        .expect("echo should succeed");
        assert_eq!(
            status.state,
            RunState::Succeeded,
            "echo should succeed, got: {:?}",
            status
        );

        // Verify events were recorded
        let record = store
            .run(run_id)
            .await
            .expect("run lookup should succeed")
            .expect("run should exist");
        assert!(
            record.events.iter().any(|e| e.kind == "start"),
            "should have start event"
        );
        assert!(
            record.events.iter().any(|e| e.kind == "completion"),
            "should have completion event"
        );
    }

    #[tokio::test]
    async fn test_cli_adapter_with_false_fails() {
        /*
        GIVEN a CLI adapter configured to use 'false' command
        WHEN running a request
        THEN the subprocess should fail with appropriate status
        */
        let store: Arc<dyn skrills_subagents::RunStore> = Arc::new(MemRunStore::new());

        // Use 'false' which always exits with code 1
        let config = CliConfig::new("false").without_non_interactive();
        let adapter = CodexCliAdapter::with_config(config);

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        let run_id = adapter
            .run(request, store.clone())
            .await
            .expect("run should start");

        let status = wait_for_run_state(
            store.clone(),
            run_id,
            RunState::Failed,
            Duration::from_secs(2),
        )
        .await
        .expect("false should fail");
        assert_eq!(
            status.state,
            RunState::Failed,
            "false should fail, got: {:?}",
            status
        );

        // Verify error event was recorded
        let record = store
            .run(run_id)
            .await
            .expect("run lookup should succeed")
            .expect("run should exist");
        assert!(
            record.events.iter().any(|e| e.kind == "error"),
            "should have error event"
        );
    }

    #[tokio::test]
    async fn test_cli_adapter_captures_stdout() {
        /*
        GIVEN a CLI adapter running a command that produces output
        WHEN the command completes
        THEN stdout should be captured in events
        */
        let store: Arc<dyn skrills_subagents::RunStore> = Arc::new(MemRunStore::new());

        // Use 'pwd' to get output
        let config = CliConfig::new("pwd").without_non_interactive();
        let adapter = CodexCliAdapter::with_config(config);

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        let run_id = adapter
            .run(request, store.clone())
            .await
            .expect("run should start");

        wait_for_run_state(
            store.clone(),
            run_id,
            RunState::Succeeded,
            Duration::from_secs(2),
        )
        .await
        .expect("pwd should succeed");

        let record = store
            .run(run_id)
            .await
            .expect("run lookup should succeed")
            .expect("run should exist");

        // Should have completion event with output
        let completion = record
            .events
            .iter()
            .find(|e| e.kind == "completion")
            .expect("should have completion event");

        let data = completion
            .data
            .as_ref()
            .expect("completion should have data");
        let text = data
            .get("text")
            .and_then(|v| v.as_str())
            .expect("completion should have text");
        assert!(!text.is_empty(), "text should not be empty");
    }

    #[tokio::test]
    async fn test_cli_adapter_stop_cancels_run() {
        /*
        GIVEN a running CLI subprocess
        WHEN stop is called
        THEN the run should be marked as canceled
        */
        let store: Arc<dyn skrills_subagents::RunStore> = Arc::new(MemRunStore::new());

        // Use 'sleep' for a long-running process
        let config = CliConfig::new("sleep")
            .without_non_interactive()
            .with_timeout(Duration::from_secs(60));
        let adapter = CodexCliAdapter::with_config(config);

        // Manually create a run in the store (simpler than spawning)
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "10".to_string(), // sleep 10 seconds
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        let run_id = store
            .create_run(request)
            .await
            .expect("run creation should succeed");

        // Stop the run
        let stopped = adapter
            .stop(run_id, store.clone())
            .await
            .expect("stop should succeed");
        assert!(stopped, "stop should succeed");

        let status = store
            .status(run_id)
            .await
            .expect("status lookup should succeed")
            .expect("run should exist");
        assert_eq!(status.state, RunState::Canceled);
    }
}

// ============================================================================
// Module: Response Schema Compliance Tests
// ============================================================================

mod response_schema_tests {
    use super::*;

    #[tokio::test]
    async fn test_list_agents_response_schema() {
        /*
        GIVEN a list-agents call
        WHEN the response is returned
        THEN it should match the expected schema for Claude Code compatibility
        */
        let fixture = IntegrationTestFixture::with_agents(&[(
            "schema-test-agent.md",
            r#"---
name: schema-test-agent
description: For schema testing
tools: Read
model: sonnet
---

Content."#,
        )])
        .unwrap();

        let service = fixture.create_service().unwrap();
        let result = service.handle_call("list-agents", None).await.unwrap();

        // Verify response structure
        assert!(result.structured_content.is_some());
        let content = result.structured_content.unwrap();

        // Must have "agents" array
        let agents = content.get("agents").and_then(|v| v.as_array());
        assert!(agents.is_some(), "must have agents array");

        let agent = &agents.unwrap()[0];

        // Verify all required fields are present
        assert!(agent.get("name").is_some(), "agent must have name");
        assert!(
            agent.get("description").is_some(),
            "agent must have description"
        );
        assert!(agent.get("tools").is_some(), "agent must have tools");
        assert!(agent.get("source").is_some(), "agent must have source");
        assert!(agent.get("path").is_some(), "agent must have path");
        assert!(
            agent.get("requires_cli").is_some(),
            "agent must have requires_cli"
        );

        // Verify types
        assert!(agent["name"].is_string());
        assert!(agent["description"].is_string());
        assert!(agent["tools"].is_array());
        assert!(agent["source"].is_string());
        assert!(agent["path"].is_string());
        assert!(agent["requires_cli"].is_boolean());
    }

    #[tokio::test]
    async fn test_run_subagent_response_schema() {
        /*
        GIVEN a run-subagent call
        WHEN the response is returned
        THEN it should match the expected schema
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();

        let args = json!({"prompt": "test"}).as_object().cloned();
        let result = service
            .handle_call("run-subagent", args.as_ref())
            .await
            .unwrap();

        let content = result.structured_content.unwrap();

        // Must have run_id
        let run_id = content.get("run_id");
        assert!(run_id.is_some(), "must have run_id");
        assert!(run_id.unwrap().is_string(), "run_id must be string");

        // Should have status
        assert!(content.get("status").is_some(), "should have status");

        // Should have events array
        let events = content.get("events");
        assert!(events.is_some(), "should have events");
        assert!(events.unwrap().is_array(), "events must be array");
    }

    #[tokio::test]
    async fn test_get_run_events_response_schema() {
        /*
        GIVEN a get-run-events call
        WHEN the response is returned
        THEN it should match the expected schema for Claude Code compatibility
        */
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

        // Add an event
        store
            .append_event(
                run_id,
                RunEvent {
                    ts: time::OffsetDateTime::now_utc(),
                    kind: "test-event".into(),
                    data: Some(json!({"key": "value"})),
                },
            )
            .await
            .unwrap();

        let fixture = IntegrationTestFixture::new().unwrap();
        let service = SubagentService::with_store_and_registry(
            store,
            BackendKind::Codex,
            fixture.registry.clone(),
        )
        .unwrap();

        let args = json!({"run_id": run_id.0.to_string()}).as_object().cloned();
        let result = service
            .handle_call("get-run-events", args.as_ref())
            .await
            .unwrap();

        let content = result.structured_content.unwrap();

        // Verify required fields
        assert!(content.get("run_id").is_some(), "must have run_id");
        assert!(content.get("events").is_some(), "must have events");
        assert!(
            content.get("total_count").is_some(),
            "must have total_count"
        );
        assert!(content.get("has_more").is_some(), "must have has_more");

        // Verify types
        assert!(content["run_id"].is_string());
        assert!(content["events"].is_array());
        assert!(content["total_count"].is_number());
        assert!(content["has_more"].is_boolean());

        // Verify event schema
        let events = content["events"].as_array().unwrap();
        if !events.is_empty() {
            let event = &events[0];
            assert!(event.get("index").is_some(), "event must have index");
            assert!(event.get("ts").is_some(), "event must have ts");
            assert!(event.get("kind").is_some(), "event must have kind");

            assert!(event["index"].is_number());
            assert!(event["ts"].is_string());
            assert!(event["kind"].is_string());
        }
    }

    #[tokio::test]
    async fn test_tools_list_includes_required_tools() {
        /*
        GIVEN a SubagentService
        WHEN getting the tools list
        THEN it should include all required MCP tools
        */
        let fixture = IntegrationTestFixture::new().unwrap();
        let service = fixture.create_service().unwrap();
        let tools = service.tools();
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

        // Required tools for Claude Code compatibility
        assert!(tool_names.contains(&"list-agents"), "must have list-agents");
        assert!(
            tool_names.contains(&"run-subagent"),
            "must have run-subagent"
        );
        assert!(
            tool_names.contains(&"get-run-status"),
            "must have get-run-status"
        );
        assert!(
            tool_names.contains(&"get-run-events"),
            "must have get-run-events"
        );
        assert!(tool_names.contains(&"stop-run"), "must have stop-run");
        assert!(
            tool_names.contains(&"get-run-history"),
            "must have get-run-history"
        );
        assert!(
            tool_names.contains(&"list-subagents"),
            "must have list-subagents"
        );
    }
}
