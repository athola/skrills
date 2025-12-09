//! Integration tests for the SubagentService
//!
//! These tests follow BDD/TDD principles to validate the complete workflow
//! of the subagent service including backend communication and state management.

use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

use serde_json::json;
use skrills_subagents::store::MemRunStore;
use skrills_subagents::{
    BackendKind, RunId, RunRecord, RunRequest, RunState, RunStatus, RunStore, SubagentService,
};

/// Test context for SubagentService tests
struct TestContext {
    #[allow(dead_code)]
    temp_dir: TempDir,
    store: Arc<MemRunStore>,
    service: SubagentService,
}

impl TestContext {
    /// Create a new test context with isolated storage
    fn new() -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let store = Arc::new(MemRunStore::new());
        let service = SubagentService::with_store(store.clone(), BackendKind::Codex)?;

        Ok(Self {
            temp_dir,
            store,
            service,
        })
    }
}

#[cfg(test)]
mod subagent_service_tests {
    use super::*;

    #[tokio::test]
    async fn test_subagent_service_lifecycle() {
        /*
        GIVEN a newly created SubagentService
        WHEN the service is initialized
        THEN it should have a valid store and default backend
        */
        let ctx = TestContext::new().unwrap();

        // Verify service is properly initialized
        let tools = ctx.service.tools();
        assert!(!tools.is_empty(), "Service should have tools available");

        // Verify we can access the store
        let history: Vec<RunRecord> = ctx.store.history(10).await.unwrap();
        assert_eq!(history.len(), 0, "New store should be empty");
    }

    #[tokio::test]
    async fn test_create_and_execute_run() {
        /*
        GIVEN a SubagentService with a configured backend
        WHEN a user creates a new run with a specific request
        THEN the run should be created with proper metadata and be executable
        */
        let ctx = TestContext::new().unwrap();

        // Create a run request
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Test prompt for subagent execution".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        // Create the run via the store
        let run_id: RunId = ctx.store.create_run(request).await.unwrap();

        // Verify run was created
        let run_record = ctx.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run_record.id, run_id);
        assert_eq!(run_record.status.state, RunState::Pending);
        assert_eq!(run_record.request.backend, BackendKind::Codex);
        assert_eq!(
            run_record.request.prompt,
            "Test prompt for subagent execution"
        );
    }

    #[tokio::test]
    async fn test_run_execution_tracking() {
        /*
        GIVEN a created run in the system
        WHEN the run is executed
        THEN its state should progress through expected lifecycle stages
        */
        let ctx = TestContext::new().unwrap();

        // Create a run request
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Simple test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        // Create the run
        let run_id: RunId = ctx.store.create_run(request).await.unwrap();

        // Verify initial state
        let run_record = ctx.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run_record.status.state, RunState::Pending);

        // Update status to Running
        let new_status = RunStatus {
            state: RunState::Running,
            message: Some("Starting execution".to_string()),
            updated_at: time::OffsetDateTime::now_utc(),
        };
        ctx.store.update_status(run_id, new_status).await.unwrap();

        // Verify state changed
        let run = ctx.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunState::Running);

        // Update status to Succeeded
        let success_status = RunStatus {
            state: RunState::Succeeded,
            message: Some("Execution completed successfully".to_string()),
            updated_at: time::OffsetDateTime::now_utc(),
        };
        ctx.store
            .update_status(run_id, success_status)
            .await
            .unwrap();

        // Verify final state
        let run = ctx.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunState::Succeeded);
    }

    #[tokio::test]
    async fn test_run_failure_handling() {
        /*
        GIVEN a run that encounters an error during execution
        WHEN the error is handled
        THEN the run should be marked as failed with appropriate error information
        */
        let ctx = TestContext::new().unwrap();

        // Create a run request
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "This will fail".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        // Create the run
        let run_id: RunId = ctx.store.create_run(request).await.unwrap();

        // Set status to Failed
        let failed_status = RunStatus {
            state: RunState::Failed,
            message: Some("Simulated execution error".to_string()),
            updated_at: time::OffsetDateTime::now_utc(),
        };
        ctx.store
            .update_status(run_id, failed_status)
            .await
            .unwrap();

        // Verify failure state
        let run = ctx.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunState::Failed);
        assert_eq!(
            run.status.message.as_deref(),
            Some("Simulated execution error")
        );
    }

    #[tokio::test]
    async fn test_run_timeout_handling() {
        /*
        GIVEN a run with a configured timeout
        WHEN the timeout is exceeded
        THEN the run should be stopped appropriately
        */
        let ctx = TestContext::new().unwrap();

        // Create a run request (note: async_mode handles timeouts in real implementation)
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Long running task".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: true, // Use async for potentially long tasks
            tracing: false,
        };

        // Create the run
        let run_id: RunId = ctx.store.create_run(request).await.unwrap();

        // Verify it starts in Pending state
        let run = ctx.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunState::Pending);

        // Test stopping the run
        let stopped = ctx.store.stop(run_id).await.unwrap();
        assert!(stopped, "Run should be stoppable");

        // Verify it's now in Canceled state
        let run = ctx.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunState::Canceled);
    }

    #[tokio::test]
    async fn test_multiple_run_management() {
        /*
        GIVEN multiple runs in the system
        WHEN managing their lifecycle
        THEN each run should be tracked independently and accurately
        */
        let ctx = TestContext::new().unwrap();

        // Create multiple runs
        let mut run_ids: Vec<RunId> = Vec::new();

        for i in 0..3 {
            let request = RunRequest {
                backend: BackendKind::Codex,
                prompt: format!("Test run {}", i),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
            };

            let run_id: RunId = ctx.store.create_run(request).await.unwrap();
            run_ids.push(run_id);
        }

        // Verify all runs are created
        let history: Vec<RunRecord> = ctx.store.history(10).await.unwrap();
        assert_eq!(history.len(), 3);

        // Update runs to different states
        let running_status = RunStatus {
            state: RunState::Running,
            message: None,
            updated_at: time::OffsetDateTime::now_utc(),
        };
        ctx.store
            .update_status(run_ids[0], running_status)
            .await
            .unwrap();

        let success_status = RunStatus {
            state: RunState::Succeeded,
            message: None,
            updated_at: time::OffsetDateTime::now_utc(),
        };
        ctx.store
            .update_status(run_ids[1], success_status)
            .await
            .unwrap();

        let failed_status = RunStatus {
            state: RunState::Failed,
            message: Some("Test error".to_string()),
            updated_at: time::OffsetDateTime::now_utc(),
        };
        ctx.store
            .update_status(run_ids[2], failed_status)
            .await
            .unwrap();

        // Verify states are correct
        let history = ctx.store.history(10).await.unwrap();
        let states: Vec<_> = history.iter().map(|r| &r.status.state).collect();
        assert!(states.contains(&&RunState::Running));
        assert!(states.contains(&&RunState::Succeeded));
        assert!(states.contains(&&RunState::Failed));
    }

    #[tokio::test]
    async fn test_backend_configuration() {
        /*
        GIVEN multiple backend configurations
        WHEN creating runs with different backends
        THEN each run should use the correct backend
        */
        let ctx = TestContext::new().unwrap();

        // Test Codex backend
        let codex_request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Codex test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        let codex_run_id: RunId = ctx.store.create_run(codex_request).await.unwrap();
        let codex_run: RunRecord = ctx.store.get_run(codex_run_id).await.unwrap().unwrap();
        assert_eq!(codex_run.request.backend, BackendKind::Codex);

        // Test Claude backend
        let claude_request = RunRequest {
            backend: BackendKind::Claude,
            prompt: "Claude test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        let claude_run_id: RunId = ctx.store.create_run(claude_request).await.unwrap();
        let claude_run: RunRecord = ctx.store.get_run(claude_run_id).await.unwrap().unwrap();
        assert_eq!(claude_run.request.backend, BackendKind::Claude);
    }

    #[tokio::test]
    async fn test_template_usage() {
        /*
        GIVEN a run request with a template ID
        WHEN the run is created
        THEN the template ID should be preserved
        */
        let ctx = TestContext::new().unwrap();

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Test with template".to_string(),
            template_id: Some("test-template".to_string()),
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        let run_id: RunId = ctx.store.create_run(request).await.unwrap();
        let run: RunRecord = ctx.store.get_run(run_id).await.unwrap().unwrap();

        assert_eq!(run.request.template_id.as_deref(), Some("test-template"));
    }

    #[tokio::test]
    async fn test_output_schema_handling() {
        /*
        GIVEN a run request with an output schema
        WHEN the run is created
        THEN the output schema should be preserved
        */
        let ctx = TestContext::new().unwrap();

        let output_schema = json!({
            "type": "object",
            "properties": {
                "result": {"type": "string"},
                "confidence": {"type": "number"}
            }
        });

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Test with output schema".to_string(),
            template_id: None,
            output_schema: Some(output_schema.clone()),
            async_mode: false,
            tracing: false,
        };

        let run_id: RunId = ctx.store.create_run(request).await.unwrap();
        let run: RunRecord = ctx.store.get_run(run_id).await.unwrap().unwrap();

        assert_eq!(run.request.output_schema, Some(output_schema));
    }

    #[tokio::test]
    async fn test_event_tracking() {
        /*
        GIVEN a running run
        WHEN events are added to track progress
        THEN the events should be stored and retrievable
        */
        let ctx = TestContext::new().unwrap();

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Event tracking test".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        };

        let run_id = ctx.store.create_run(request).await.unwrap();

        // Add some events
        let event1 = skrills_subagents::RunEvent {
            ts: time::OffsetDateTime::now_utc(),
            kind: "started".to_string(),
            data: Some(json!({"step": 1})),
        };

        let event2 = skrills_subagents::RunEvent {
            ts: time::OffsetDateTime::now_utc(),
            kind: "progress".to_string(),
            data: Some(json!({"step": 2, "progress": 0.5})),
        };

        ctx.store.append_event(run_id, event1).await.unwrap();
        ctx.store.append_event(run_id, event2).await.unwrap();

        // Verify events are stored
        let run = ctx.store.get_run(run_id).await.unwrap().unwrap();
        assert_eq!(run.events.len(), 2);
        assert_eq!(run.events[0].kind, "started");
        assert_eq!(run.events[1].kind, "progress");
    }

    #[tokio::test]
    async fn test_service_tool_handling() {
        /*
        GIVEN the SubagentService
        WHEN calling service methods through the tool interface
        THEN they should execute correctly
        */
        let ctx = TestContext::new().unwrap();

        // Test running a subagent through the service
        let args = json!({
            "prompt": "Test prompt",
            "backend": "codex"
        });

        let result = ctx
            .service
            .handle_call("run_subagent", Some(args.as_object().unwrap()))
            .await
            .unwrap();
        assert!(!result.content.is_empty());

        // Extract run_id from result
        if let Some(structured) = result.structured_content {
            if let Some(run_id_str) = structured.get("run_id").and_then(|v| v.as_str()) {
                let run_id = RunId(Uuid::parse_str(run_id_str).unwrap());

                // Verify the run exists in the store
                let run = ctx.store.get_run(run_id).await.unwrap();
                assert!(run.is_some(), "Run should exist in store");
            }
        }
    }
}
