//! MCP server maps are merged into the target by name, never replaced.
//!
//! Every writer used to assign the source's map over the target's, so a server
//! that only the target knew about was deleted on each sync, and a source with
//! no servers emptied the target.

use skrills_sync::adapters::{
    AgentAdapter, ClaudeAdapter, CodexAdapter, CopilotAdapter, CursorAdapter,
};
use skrills_sync::common::{McpServer, McpTransport};
use std::collections::HashMap;
use std::path::Path;
use tempfile::TempDir;

fn stdio(name: &str) -> McpServer {
    McpServer {
        name: name.to_string(),
        transport: McpTransport::Stdio,
        command: format!("/bin/{name}"),
        args: Vec::new(),
        env: HashMap::new(),
        url: None,
        headers: None,
        enabled: true,
        allowed_tools: Vec::new(),
        disabled_tools: Vec::new(),
    }
}

/// Each target with the file its MCP map lives in.
fn targets(root: &Path) -> Vec<(Box<dyn AgentAdapter>, std::path::PathBuf)> {
    vec![
        (
            Box::new(ClaudeAdapter::with_root(root.join("claude"))),
            root.join("claude/settings.json"),
        ),
        (
            Box::new(CodexAdapter::with_root(root.join("codex"))),
            root.join("codex/config.json"),
        ),
        (
            Box::new(CopilotAdapter::with_root(root.join("copilot"))),
            root.join("copilot/mcp-config.json"),
        ),
        (
            Box::new(CursorAdapter::with_root(root.join("cursor"))),
            root.join("cursor/mcp.json"),
        ),
    ]
}

fn seed(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        r#"{"theme": "dark", "mcpServers": {"a": {"command": "/bin/a"}, "b": {"command": "/bin/b", "timeout": 5}}}"#,
    )
    .unwrap();
}

fn read(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn target_only_servers_and_other_keys_survive_a_sync() {
    let tmp = TempDir::new().unwrap();
    let source = HashMap::from([("c".to_string(), stdio("c"))]);

    for (adapter, path) in targets(tmp.path()) {
        seed(&path);
        adapter.write_mcp_servers(&source).unwrap();

        let config = read(&path);
        let servers = config["mcpServers"].as_object().unwrap();
        assert!(
            servers.contains_key("a") && servers.contains_key("b") && servers.contains_key("c"),
            "{}: expected a, b and c, got {servers:?}",
            adapter.name()
        );
        assert_eq!(config["theme"], "dark", "{}", adapter.name());
        assert_eq!(
            config["mcpServers"]["b"]["timeout"],
            5,
            "{}",
            adapter.name()
        );
    }
}

#[test]
fn a_source_with_no_servers_leaves_the_target_untouched() {
    let tmp = TempDir::new().unwrap();

    for (adapter, path) in targets(tmp.path()) {
        seed(&path);
        let before = std::fs::read(&path).unwrap();
        adapter.write_mcp_servers(&HashMap::new()).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "{}: an empty source must not rewrite the file",
            adapter.name()
        );
    }
}

/// `value["key"] = ...` on a JSON array panics; the writers must return an
/// error instead of taking down the MCP server process.
#[test]
fn a_non_object_config_root_is_an_error_not_a_panic() {
    let tmp = TempDir::new().unwrap();
    let source = HashMap::from([("c".to_string(), stdio("c"))]);

    for (adapter, path) in targets(tmp.path()) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[]").unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            adapter.write_mcp_servers(&source)
        }));
        let result = result.unwrap_or_else(|_| panic!("{} panicked", adapter.name()));
        assert!(result.is_err(), "{}: expected an error", adapter.name());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]");
    }
}

/// The writer's own output was compared as a string built from a `HashMap`,
/// whose order changes per process, so an unchanged map was rewritten.
#[test]
fn a_second_identical_sync_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let source: HashMap<_, _> = ["a", "b", "c", "d"]
        .iter()
        .map(|n| (n.to_string(), stdio(n)))
        .collect();

    for (adapter, _path) in targets(tmp.path()) {
        adapter.write_mcp_servers(&source).unwrap();
        let second = adapter.write_mcp_servers(&source).unwrap();
        assert_eq!(second.written, 0, "{}", adapter.name());
    }
}

/// Preferences carry only `model`. With no model the writers used to
/// re-serialize the user's file anyway, and create it when it was missing.
#[test]
fn preferences_without_a_model_touch_nothing() {
    let tmp = TempDir::new().unwrap();
    let prefs = skrills_sync::common::Preferences::default();

    let claude = ClaudeAdapter::with_root(tmp.path().join("claude"));
    let codex = CodexAdapter::with_root(tmp.path().join("codex"));
    let copilot = CopilotAdapter::with_root(tmp.path().join("copilot"));
    let adapters: [(&dyn AgentAdapter, &str); 3] = [
        (&claude, "claude/settings.json"),
        (&codex, "codex/config.json"),
        (&copilot, "copilot/config.json"),
    ];

    for (adapter, rel) in adapters {
        adapter.write_preferences(&prefs).unwrap();
        assert!(
            !tmp.path().join(rel).exists(),
            "{}: no model, no file",
            adapter.name()
        );
    }
}

#[test]
fn preferences_on_a_non_object_root_are_an_error_not_a_panic() {
    let tmp = TempDir::new().unwrap();
    let prefs = skrills_sync::common::Preferences {
        model: Some("sonnet".to_string()),
        ..Default::default()
    };
    let path = tmp.path().join("copilot/config.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "\"just a string\"").unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().join("copilot"));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        adapter.write_preferences(&prefs)
    }))
    .expect("must not panic");
    assert!(result.is_err());
}
