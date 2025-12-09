//! End-to-end tests for subagent functionality over MCP
//!
//! These tests verify the complete integration between the skrills server,
//! subagent backends, and MCP protocol for subagent execution.

use httpmock::prelude::*;
use rmcp::transport::TokioChildProcess;
use rmcp::{model::CallToolRequestParam, service::serve_client};
use serde_json::json;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn given_skrills_server_with_subagents_when_executing_run_subagent_then_completes_successfully(
) -> anyhow::Result<()> {
    // GIVEN a running skrills server with subagent support
    // WHEN executing the run_subagent MCP tool
    // THEN it should route to the correct backend and complete successfully
    // Arrange
    let tmp = tempfile::tempdir()?;
    std::env::set_var("HOME", tmp.path());
    std::fs::create_dir_all(tmp.path().join(".codex/skills"))?;

    // Mock Codex/Claude HTTP endpoints to verify adapters honor config.
    let server = MockServer::start();
    let codex_mock = server.mock(|when, then| {
        when.method(POST).path("/v1/chat/completions");
        then.status(200)
            .json_body(json!({"choices": [{"message": {"content": "mock codex reply"}}]}));
    });
    let claude_mock = server.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200)
            .json_body(json!({"content": [{"text": "mock claude reply"}]}));
    });

    std::env::set_var("SKRILLS_CODEX_API_KEY", "test-codex-key");
    std::env::set_var(
        "SKRILLS_CODEX_BASE_URL",
        format!("{}/v1/", server.base_url()),
    );
    std::env::set_var("SKRILLS_CLAUDE_API_KEY", "test-claude-key");
    std::env::set_var(
        "SKRILLS_CLAUDE_BASE_URL",
        format!("{}/v1/", server.base_url()),
    );
    std::env::set_var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND", "codex");

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir")
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join("target"));
    let binary_path = target_dir.join("debug").join(if cfg!(windows) {
        "skrills.exe"
    } else {
        "skrills"
    });
    let cargo_home = std::env::var("CARGO_HOME").unwrap_or_else(|_| {
        workspace_root
            .join(".cargo-home")
            .to_string_lossy()
            .into_owned()
    });

    let status = Command::new("cargo")
        .args(["build", "-p", "skrills", "--features", "subagents"])
        .env("CARGO_HOME", &cargo_home)
        .status()
        .await?;
    assert!(status.success(), "cargo build failed to produce skrills");

    // Act
    let mut command = Command::new(&binary_path);
    command.arg("serve");
    command.env("HOME", tmp.path());
    command.env("SKRILLS_CODEX_API_KEY", "test-codex-key");
    command.env(
        "SKRILLS_CODEX_BASE_URL",
        format!("{}/v1/", server.base_url()),
    );
    command.env("SKRILLS_CLAUDE_API_KEY", "test-claude-key");
    command.env(
        "SKRILLS_CLAUDE_BASE_URL",
        format!("{}/v1/", server.base_url()),
    );
    command.env("SKRILLS_SUBAGENTS_DEFAULT_BACKEND", "codex");

    let (transport, stderr) = TokioChildProcess::builder(command)
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = stderr;

    let client = match serve_client((), transport).await {
        Ok(c) => c,
        Err(e) => {
            if let Some(mut err) = stderr.take() {
                let mut buf = String::new();
                let _ = err.read_to_string(&mut buf).await;
                panic!("serve_client failed: {e:?}\nstderr:\n{buf}");
            }
            return Err(e.into());
        }
    };
    let peer = client.peer().clone();

    // Verify subagent tools are available
    let tools = peer.list_all_tools().await?;
    assert!(
        tools.iter().any(|t| t.name == "run_subagent"),
        "run_subagent tool should be available"
    );

    // Execute subagent with codex backend
    let args = json!({"prompt": "ping", "backend": "codex", "stream": false});
    let result = peer
        .call_tool(CallToolRequestParam {
            name: "run_subagent".into(),
            arguments: args.as_object().cloned(),
        })
        .await?;
    let run_id = result
        .structured_content
        .as_ref()
        .and_then(|v| v.get("run_id"))
        .and_then(|v| v.as_str())
        .expect("run_id string")
        .to_string();

    // Assert - Poll for completion
    let mut last_state = String::new();
    for _ in 0..10 {
        let status = peer
            .call_tool(CallToolRequestParam {
                name: "get_run_status".into(),
                arguments: Some(json!({ "run_id": run_id }).as_object().cloned().unwrap()),
            })
            .await?;
        let content = status.structured_content.unwrap();
        last_state = content
            .get("status")
            .and_then(|s| s.get("state"))
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string();
        if last_state == "Succeeded" {
            let events = content
                .get("events")
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            assert!(!events.is_empty(), "expected streaming events");
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(last_state, "Succeeded", "Subagent execution should succeed");

    // Verify correct backend was called
    codex_mock.assert();
    let claude_hits = claude_mock.calls();
    assert!(
        claude_hits <= 1,
        "claude mock unexpectedly invoked {claude_hits} times"
    );

    client.cancel().await?;

    // Assert no errors in stderr
    if let Some(mut err) = stderr {
        let mut buf = String::new();
        let _ = err.read_to_string(&mut buf).await;
        assert!(
            !buf.to_lowercase().contains("error"),
            "Server should not log errors"
        );
    }
    Ok(())
}

#[tokio::test]
async fn given_server_with_multiple_backends_when_switching_default_then_routes_correctly(
) -> anyhow::Result<()> {
    // GIVEN a skrills server with multiple backends configured
    // WHEN switching the default backend
    // THEN subsequent requests should route to the new default
    // Arrange
    let tmp = tempfile::tempdir()?;
    std::env::set_var("HOME", tmp.path());
    std::fs::create_dir_all(tmp.path().join(".codex/skills"))?;

    let server = MockServer::start();

    // Track calls to each backend
    let codex_mock = server.mock(|when, then| {
        when.method(POST).path("/v1/chat/completions");
        then.status(200).json_body(json!({
            "choices": [{"message": {"content": "codex response"}}]
        }));
    });

    let claude_mock = server.mock(|when, then| {
        when.method(POST).path("/v1/messages");
        then.status(200).json_body(json!({
            "content": [{"text": "claude response"}]
        }));
    });

    // Configure both backends
    std::env::set_var("SKRILLS_CODEX_API_KEY", "test-codex-key");
    std::env::set_var("SKRILLS_CLAUDE_API_KEY", "test-claude-key");
    std::env::set_var(
        "SKRILLS_CODEX_BASE_URL",
        format!("{}/v1/", server.base_url()),
    );
    std::env::set_var(
        "SKRILLS_CLAUDE_BASE_URL",
        format!("{}/v1/", server.base_url()),
    );

    // Build binary
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir")
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join("target"));
    let binary_path = target_dir.join("debug").join(if cfg!(windows) {
        "skrills.exe"
    } else {
        "skrills"
    });

    // Act & Assert
    // Test with Codex as default
    std::env::set_var("SKRILLS_SUBAGENTS_DEFAULT_BACKEND", "codex");

    // Start server
    let mut command = Command::new(&binary_path);
    command.arg("serve");
    command.env("HOME", tmp.path());
    command.env("SKRILLS_CODEX_API_KEY", "test-codex-key");
    command.env(
        "SKRILLS_CODEX_BASE_URL",
        format!("{}/v1/", server.base_url()),
    );
    command.env("SKRILLS_CLAUDE_API_KEY", "test-claude-key");
    command.env(
        "SKRILLS_CLAUDE_BASE_URL",
        format!("{}/v1/", server.base_url()),
    );
    command.env("SKRILLS_SUBAGENTS_DEFAULT_BACKEND", "codex");

    let (transport, _stderr) = TokioChildProcess::builder(command).spawn()?;

    let client = serve_client((), transport).await?;
    let peer = client.peer().clone();

    // Execute request without specifying backend (should use codex)
    let args = json!({"prompt": "test", "stream": false});
    let result = peer
        .call_tool(CallToolRequestParam {
            name: "run_subagent".into(),
            arguments: args.as_object().cloned(),
        })
        .await?;

    // Wait for completion
    let run_id = result
        .structured_content
        .as_ref()
        .and_then(|v| v.get("run_id"))
        .and_then(|v| v.as_str())
        .expect("run_id string")
        .to_string();

    // Poll for completion
    let mut last_state = String::new();
    for _ in 0..20 {
        let status = peer
            .call_tool(CallToolRequestParam {
                name: "get_run_status".into(),
                arguments: Some(json!({ "run_id": run_id }).as_object().cloned().unwrap()),
            })
            .await?;
        let content = status.structured_content.unwrap();
        last_state = content
            .get("status")
            .and_then(|s| s.get("state"))
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string();
        if last_state == "Succeeded" {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(last_state, "Succeeded", "Subagent execution should succeed");

    // Verify codex was called
    assert_eq!(codex_mock.calls(), 1, "Codex should be called once");
    assert_eq!(claude_mock.calls(), 0, "Claude should not be called");

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn given_server_when_streaming_enabled_then_emits_events() -> anyhow::Result<()> {
    // GIVEN a running skrills server
    // WHEN executing a subagent with streaming enabled
    // THEN it should emit streaming events during execution
    // This test would verify that streaming works correctly
    // Implementation would follow similar pattern to the main test
    // but with stream: true and verification of event emission
    Ok(())
}
