use codex_mcp_skills_server::runtime::{reset_runtime_cache_for_tests, runtime_overrides_cached};
use rmcp::transport::TokioChildProcess;
use rmcp::{model::CallToolRequestParam, service::serve_client};
use serde_json::json;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[tokio::test]
async fn runtime_tools_round_trip_over_mcp() -> anyhow::Result<()> {
    reset_runtime_cache_for_tests();
    let tmp = tempfile::tempdir()?;
    std::env::set_var("HOME", tmp.path());
    std::fs::create_dir_all(tmp.path().join(".codex/skills"))?;

    // Build path to the already-compiled CLI binary to avoid spawning a nested cargo build.
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
        "codex-mcp-skills.exe"
    } else {
        "codex-mcp-skills"
    });
    let cargo_home = std::env::var("CARGO_HOME").unwrap_or_else(|_| {
        workspace_root
            .join(".cargo-home")
            .to_string_lossy()
            .into_owned()
    });
    let status = Command::new("cargo")
        .args(["build", "-p", "codex-mcp-skills"])
        .env("CARGO_HOME", &cargo_home)
        .status()
        .await?;
    assert!(
        status.success(),
        "cargo build failed to produce codex-mcp-skills"
    );

    // Start the real CLI server over stdio and talk to it via MCP client transport.
    let mut command = Command::new(&binary_path);
    command.arg("serve");
    command.env("HOME", tmp.path());

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

    // Ensure the runtime tools are advertised to clients.
    let tools = peer.list_all_tools().await?;
    assert!(tools.iter().any(|t| t.name == "runtime-status"));
    assert!(tools.iter().any(|t| t.name == "set-runtime-options"));

    // Update runtime options through the MCP tool.
    let args = json!({ "manifest_first": false, "render_mode_log": true, "manifest_minimal": true });
    let result = peer
        .call_tool(CallToolRequestParam {
            name: "set-runtime-options".into(),
            arguments: args.as_object().cloned(),
        })
        .await?;

    let structured = result
        .structured_content
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("expected structured_content"))?;
    assert_eq!(
        structured.get("manifest_first").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        structured.get("render_mode_log").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        structured
            .get("manifest_minimal")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        structured
            .get("overrides")
            .and_then(|o| o.get("manifest_first"))
            .and_then(|v| v.as_bool()),
        Some(false)
    );

    // Fetch status to verify persistence and shape of response.
    let status = peer
        .call_tool(CallToolRequestParam {
            name: "runtime-status".into(),
            arguments: None,
        })
        .await?;
    let status_obj = status
        .structured_content
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("expected structured_content"))?;
    assert_eq!(
        status_obj.get("manifest_first").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        status_obj
            .get("overrides")
            .and_then(|o| o.get("render_mode_log"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        status_obj
            .get("overrides")
            .and_then(|o| o.get("manifest_minimal"))
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(status_obj.get("env").is_some());

    // Cache should now reflect persisted override file.
    reset_runtime_cache_for_tests();
    let persisted = runtime_overrides_cached();
    assert!(!persisted.manifest_first());
    assert!(persisted.render_mode_log());
    assert!(persisted.manifest_minimal());

    client.cancel().await?;

    // Drain stderr if present so the child can exit cleanly.
    if let Some(mut err) = stderr {
        let mut buf = String::new();
        let _ = err.read_to_string(&mut buf).await;
        assert!(!buf.to_lowercase().contains("error"));
    }

    Ok(())
}
