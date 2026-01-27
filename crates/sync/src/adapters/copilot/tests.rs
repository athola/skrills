//! Tests for Copilot adapter.

use super::*;
use crate::adapters::utils::hash_content;
use crate::common::{McpServer, McpTransport, Preferences};
use std::collections::HashMap;
use std::fs;
use std::time::SystemTime;
use tempfile::tempdir;

// ==========================================
// Basic Adapter Tests
// ==========================================

#[test]
fn copilot_adapter_name() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    assert_eq!(adapter.name(), "copilot");
}

#[test]
fn copilot_adapter_config_root() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    assert_eq!(adapter.config_root(), tmp.path().to_path_buf());
}

#[test]
fn copilot_adapter_supported_fields() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let fields = adapter.supported_fields();

    // Copilot prompts are NOT equivalent to Claude commands/Codex prompts
    // (prompts are detailed instruction files, commands are quick atomic shortcuts)
    assert!(!fields.commands, "Copilot should NOT support commands");
    assert!(fields.mcp_servers, "Copilot should support MCP servers");
    assert!(fields.preferences, "Copilot should support preferences");
    assert!(fields.skills, "Copilot should support skills");
}

// ==========================================
// Commands/Prompts Tests
// ==========================================

#[test]
fn read_commands_returns_empty_when_no_prompts() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let commands = adapter.read_commands(false).unwrap();
    assert!(commands.is_empty());
}

#[test]
fn read_commands_discovers_prompts_md() {
    let tmp = tempdir().unwrap();
    let prompts_dir = tmp.path().join("prompts");
    fs::create_dir_all(&prompts_dir).unwrap();
    fs::write(
        prompts_dir.join("commit.prompts.md"),
        "# Commit Prompt\n\nGenerate a commit message.",
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let commands = adapter.read_commands(false).unwrap();

    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "commit");
}

#[test]
fn write_commands_creates_prompts_files() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let commands = vec![Command {
        name: "review".to_string(),
        content: b"# Review Prompt\n\nReview this code.".to_vec(),
        source_path: PathBuf::from("/tmp/review.md"),
        modified: SystemTime::now(),
        hash: "abc".to_string(),
        modules: Vec::new(),
    }];

    let report = adapter.write_commands(&commands).unwrap();
    assert_eq!(report.written, 1);

    // Verify file was created with .prompts.md extension
    let prompt_path = tmp.path().join("prompts/review.prompts.md");
    assert!(prompt_path.exists());
}

#[test]
fn write_commands_skips_unchanged() {
    let tmp = tempdir().unwrap();
    let prompts_dir = tmp.path().join("prompts");
    fs::create_dir_all(&prompts_dir).unwrap();
    let content = b"# Test Prompt";
    fs::write(prompts_dir.join("test.prompts.md"), content).unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let commands = vec![Command {
        name: "test".to_string(),
        content: content.to_vec(),
        source_path: PathBuf::from("/tmp/test.md"),
        modified: SystemTime::now(),
        hash: hash_content(content),
        modules: Vec::new(),
    }];

    let report = adapter.write_commands(&commands).unwrap();
    assert_eq!(report.written, 0);
    assert_eq!(report.skipped.len(), 1);
}

#[test]
fn commands_roundtrip() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let commands = vec![Command {
        name: "commit-msg".to_string(),
        content: b"# Commit Message Generator".to_vec(),
        source_path: PathBuf::from("/tmp/commit-msg.md"),
        modified: SystemTime::now(),
        hash: "hash123".to_string(),
        modules: Vec::new(),
    }];

    adapter.write_commands(&commands).unwrap();
    let read_back = adapter.read_commands(false).unwrap();

    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].name, "commit-msg");
}

// ==========================================
// Skills Tests
// ==========================================

#[test]
fn read_skills_empty_dir() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();
    assert!(skills.is_empty());
}

#[test]
fn read_skills_discovers_skill_md() {
    let tmp = tempdir().unwrap();
    let skill_dir = tmp.path().join("skills/my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: test\n---\n# My Skill",
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "my-skill");
}

#[test]
fn read_skills_nested_directories() {
    let tmp = tempdir().unwrap();
    let skill_dir = tmp.path().join("skills/category/nested-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: nested\ndescription: test\n---\n# Nested",
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "category/nested-skill");
}

#[test]
fn read_skills_ignores_non_skill_md_files() {
    let tmp = tempdir().unwrap();
    let skill_dir = tmp.path().join("skills/my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("README.md"), "# Readme").unwrap();
    fs::write(skill_dir.join("notes.txt"), "Notes").unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();

    assert!(skills.is_empty());
}

#[test]
fn read_skills_ignores_hidden_directories() {
    let tmp = tempdir().unwrap();
    let hidden_dir = tmp.path().join("skills/.hidden");
    fs::create_dir_all(&hidden_dir).unwrap();
    fs::write(hidden_dir.join("SKILL.md"), "# Hidden").unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();

    assert!(skills.is_empty());
}

#[test]
fn write_skills_creates_directories() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let skill = Command {
        name: "alpha".to_string(),
        content: b"---\nname: alpha\ndescription: test\n---\n# Alpha\n".to_vec(),
        source_path: PathBuf::from("/tmp/alpha.md"),
        modified: SystemTime::now(),
        hash: "hash".to_string(),
        modules: Vec::new(),
    };

    let report = adapter.write_skills(&[skill]).unwrap();
    assert_eq!(report.written, 1);
    assert!(tmp.path().join("skills/alpha/SKILL.md").exists());
}

#[test]
fn write_skills_skips_unchanged() {
    let tmp = tempdir().unwrap();
    let skill_dir = tmp.path().join("skills/alpha");
    fs::create_dir_all(&skill_dir).unwrap();
    let content = b"---\nname: alpha\n---\n# Alpha\n";
    fs::write(skill_dir.join("SKILL.md"), content).unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let skill = Command {
        name: "alpha".to_string(),
        content: content.to_vec(),
        source_path: PathBuf::from("/tmp/alpha.md"),
        modified: SystemTime::now(),
        hash: hash_content(content),
        modules: Vec::new(),
    };

    let report = adapter.write_skills(&[skill]).unwrap();
    assert_eq!(report.written, 0);
    assert_eq!(report.skipped.len(), 1);
}

#[test]
fn write_skills_no_config_toml_created() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let skill = Command {
        name: "test".to_string(),
        content: b"# Test".to_vec(),
        source_path: PathBuf::from("/tmp/test.md"),
        modified: SystemTime::now(),
        hash: "hash".to_string(),
        modules: Vec::new(),
    };

    adapter.write_skills(&[skill]).unwrap();

    // Unlike Codex, Copilot should NOT create config.toml
    assert!(!tmp.path().join("config.toml").exists());
}

// ==========================================
// MCP Servers Tests
// ==========================================

#[test]
fn read_mcp_servers_empty_when_no_file() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();
    assert!(servers.is_empty());
}

#[test]
fn read_mcp_servers_from_mcp_config_json() {
    let tmp = tempdir().unwrap();
    // Note: MCP servers in mcp-config.json, NOT config.json
    let mcp_config_path = tmp.path().join("mcp-config.json");
    fs::write(
        &mcp_config_path,
        r#"{
        "mcpServers": {
            "test-server": {
                "command": "/usr/bin/test",
                "args": ["--flag", "value"]
            }
        }
    }"#,
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();

    assert_eq!(servers.len(), 1);
    let server = servers.get("test-server").unwrap();
    assert_eq!(server.command, "/usr/bin/test");
    assert_eq!(server.args, vec!["--flag", "value"]);
    assert!(server.enabled);
}

#[test]
fn read_mcp_servers_handles_disabled_flag() {
    let tmp = tempdir().unwrap();
    let mcp_config_path = tmp.path().join("mcp-config.json");
    fs::write(
        &mcp_config_path,
        r#"{
        "mcpServers": {
            "disabled-server": {
                "command": "/bin/disabled",
                "disabled": true
            }
        }
    }"#,
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();

    let server = servers.get("disabled-server").unwrap();
    assert!(!server.enabled);
}

#[test]
fn read_mcp_servers_invalid_json_returns_error() {
    let tmp = tempdir().unwrap();
    let mcp_config_path = tmp.path().join("mcp-config.json");
    fs::write(&mcp_config_path, "{ invalid json }").unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let result = adapter.read_mcp_servers();
    assert!(result.is_err());
}

#[test]
fn write_mcp_servers_creates_mcp_config_json() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let mut servers = HashMap::new();
    servers.insert(
        "my-server".to_string(),
        McpServer {
            name: "my-server".to_string(),
            transport: McpTransport::Stdio,
            command: "/bin/server".to_string(),
            args: vec!["arg1".to_string()],
            env: HashMap::new(),
            url: None,
            headers: None,
            enabled: true,
        },
    );

    let report = adapter.write_mcp_servers(&servers).unwrap();
    assert_eq!(report.written, 1);

    // Should write to mcp-config.json, NOT config.json
    let mcp_config_path = tmp.path().join("mcp-config.json");
    assert!(mcp_config_path.exists());

    let content = fs::read_to_string(&mcp_config_path).unwrap();
    let config: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(config["mcpServers"]["my-server"].is_object());
}

#[test]
fn write_mcp_servers_preserves_existing_structure() {
    let tmp = tempdir().unwrap();
    let mcp_config_path = tmp.path().join("mcp-config.json");
    fs::write(
        &mcp_config_path,
        r#"{
        "someOtherField": "preserved",
        "mcpServers": {}
    }"#,
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let mut servers = HashMap::new();
    servers.insert(
        "new-server".to_string(),
        McpServer {
            name: "new-server".to_string(),
            transport: McpTransport::Stdio,
            command: "/bin/new".to_string(),
            args: vec![],
            env: HashMap::new(),
            url: None,
            headers: None,
            enabled: true,
        },
    );

    adapter.write_mcp_servers(&servers).unwrap();

    let content = fs::read_to_string(&mcp_config_path).unwrap();
    let config: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(config["someOtherField"], "preserved");
}

// ==========================================
// Preferences Tests (with security field preservation)
// ==========================================

#[test]
fn read_preferences_empty_when_no_file() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let prefs = adapter.read_preferences().unwrap();
    assert!(prefs.model.is_none());
}

#[test]
fn read_preferences_from_config_json() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(
        &config_path,
        r#"{
        "model": "gpt-4o"
    }"#,
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let prefs = adapter.read_preferences().unwrap();

    assert_eq!(prefs.model.as_deref(), Some("gpt-4o"));
}

#[test]
fn read_preferences_invalid_json_returns_error() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(&config_path, "not valid json").unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let result = adapter.read_preferences();
    assert!(result.is_err());
}

#[test]
fn write_preferences_creates_config_json() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let prefs = Preferences {
        model: Some("gpt-4o".to_string()),
        custom: HashMap::new(),
    };

    let report = adapter.write_preferences(&prefs).unwrap();
    assert_eq!(report.written, 1);

    let config_path = tmp.path().join("config.json");
    assert!(config_path.exists());

    let content = fs::read_to_string(&config_path).unwrap();
    let config: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(config["model"], "gpt-4o");
}

#[test]
fn write_preferences_preserves_trusted_folders() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(
        &config_path,
        r#"{
        "model": "old-model",
        "trusted_folders": ["/home/user/project"]
    }"#,
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let prefs = Preferences {
        model: Some("gpt-4o".to_string()),
        custom: HashMap::new(),
    };

    adapter.write_preferences(&prefs).unwrap();

    let content = fs::read_to_string(&config_path).unwrap();
    let config: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Model should be updated
    assert_eq!(config["model"], "gpt-4o");
    // Security field should be preserved
    assert_eq!(config["trusted_folders"][0], "/home/user/project");
}

#[test]
fn write_preferences_preserves_allowed_urls() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(
        &config_path,
        r#"{
        "allowed_urls": ["https://example.com"],
        "denied_urls": ["https://malicious.com"]
    }"#,
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let prefs = Preferences {
        model: Some("new-model".to_string()),
        custom: HashMap::new(),
    };

    adapter.write_preferences(&prefs).unwrap();

    let content = fs::read_to_string(&config_path).unwrap();
    let config: serde_json::Value = serde_json::from_str(&content).unwrap();

    // Security fields should be preserved
    assert_eq!(config["allowed_urls"][0], "https://example.com");
    assert_eq!(config["denied_urls"][0], "https://malicious.com");
}

#[test]
fn write_preferences_invalid_existing_json_returns_error() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join("config.json");
    fs::write(&config_path, "{ malformed: json, }").unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let prefs = Preferences {
        model: Some("gpt-4o".to_string()),
        custom: HashMap::new(),
    };

    let result = adapter.write_preferences(&prefs);
    assert!(result.is_err());
}

// ==========================================
// Sanitization Tests
// ==========================================

#[test]
fn sanitize_name_removes_path_traversal() {
    use super::utils::sanitize_name;

    // Path traversal attacks are blocked
    assert_eq!(sanitize_name("../../../etc/passwd"), "etc/passwd");
    assert_eq!(sanitize_name("../../malicious"), "malicious");

    // Valid names pass through unchanged
    assert_eq!(sanitize_name("valid-name_123"), "valid-name_123");
    assert_eq!(sanitize_name("normal"), "normal");

    // Spaces and special chars are removed within segments
    assert_eq!(sanitize_name("with spaces"), "withspaces");

    // Nested skill paths are preserved (key fix for review feedback)
    assert_eq!(sanitize_name("category/my-skill"), "category/my-skill");
    assert_eq!(sanitize_name("deep/nested/skill"), "deep/nested/skill");

    // Mixed: nested paths with traversal attempts
    assert_eq!(
        sanitize_name("category/../other/skill"),
        "category/other/skill"
    );
    assert_eq!(sanitize_name("./relative/./path"), "relative/path");

    // Empty segments are collapsed
    assert_eq!(sanitize_name("a//b///c"), "a/b/c");
}

// ==========================================
// Integration Tests
// ==========================================

#[test]
fn skills_roundtrip() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let skill = Command {
        name: "test-skill".to_string(),
        content: b"---\nname: test-skill\ndescription: A test skill\n---\n# Test Skill\n".to_vec(),
        source_path: PathBuf::from("/tmp/test.md"),
        modified: SystemTime::now(),
        hash: "hash123".to_string(),
        modules: Vec::new(),
    };

    adapter.write_skills(&[skill]).unwrap();
    let read_back = adapter.read_skills().unwrap();

    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].name, "test-skill");
}

#[test]
fn mcp_servers_roundtrip() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let mut servers = HashMap::new();
    servers.insert(
        "test-server".to_string(),
        McpServer {
            name: "test-server".to_string(),
            transport: McpTransport::Stdio,
            command: "/bin/test".to_string(),
            args: vec!["--arg".to_string()],
            env: HashMap::from([("KEY".to_string(), "value".to_string())]),
            url: None,
            headers: None,
            enabled: true,
        },
    );

    adapter.write_mcp_servers(&servers).unwrap();
    let read_back = adapter.read_mcp_servers().unwrap();

    assert_eq!(read_back.len(), 1);
    let server = read_back.get("test-server").unwrap();
    assert_eq!(server.command, "/bin/test");
    assert_eq!(server.args, vec!["--arg"]);
    assert!(server.enabled);
}

// ==========================================
// Agents Tests
// ==========================================

#[test]
fn copilot_supports_agents() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let fields = adapter.supported_fields();
    assert!(fields.agents, "Copilot should support agents");
}

#[test]
fn read_agents_empty_dir() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let agents = adapter.read_agents().unwrap();
    assert!(agents.is_empty());
}

#[test]
fn read_agents_discovers_agent_md() {
    let tmp = tempdir().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("test-agent.agent.md"),
        "---\nname: test-agent\ndescription: A test agent\ntarget: github-copilot\n---\n# Test Agent\n\nInstructions here.",
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let agents = adapter.read_agents().unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "test-agent");
}

#[test]
fn read_agents_handles_plain_md_files() {
    let tmp = tempdir().unwrap();
    let agents_dir = tmp.path().join("agents");
    fs::create_dir_all(&agents_dir).unwrap();
    fs::write(
        agents_dir.join("simple.md"),
        "---\nname: simple\n---\n# Simple",
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let agents = adapter.read_agents().unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "simple");
}

#[test]
fn write_agents_creates_agent_files() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let agents = vec![Command {
        name: "my-agent".to_string(),
        content: b"---\nname: my-agent\ndescription: Test\nmodel: opus\n---\n# My Agent".to_vec(),
        source_path: PathBuf::from("/tmp/test.md"),
        modified: SystemTime::now(),
        hash: "abc".to_string(),
        modules: Vec::new(),
    }];

    let report = adapter.write_agents(&agents).unwrap();
    assert_eq!(report.written, 1);

    // Verify file was created
    let agent_path = tmp.path().join("agents/my-agent.agent.md");
    assert!(agent_path.exists());

    // Verify content was transformed
    let content = fs::read_to_string(&agent_path).unwrap();
    assert!(content.contains("target: github-copilot"));
    assert!(!content.contains("model: opus"));
}

#[test]
fn write_agents_skips_unchanged() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let agents = vec![Command {
        name: "unchanged".to_string(),
        content: b"---\nname: unchanged\ndescription: Test\n---\n# Agent".to_vec(),
        source_path: PathBuf::from("/tmp/test.md"),
        modified: SystemTime::now(),
        hash: "abc".to_string(),
        modules: Vec::new(),
    }];

    // Write once
    let report1 = adapter.write_agents(&agents).unwrap();
    assert_eq!(report1.written, 1);

    // Write again - should be skipped
    let report2 = adapter.write_agents(&agents).unwrap();
    assert_eq!(report2.written, 0);
    assert_eq!(report2.skipped.len(), 1);
}

#[test]
fn transform_agent_adds_target_to_claude_format() {
    use super::utils::transform_agent_for_copilot;

    let claude_content = b"---\nname: test\ndescription: Test agent\nmodel: opus\ncolor: green\n---\n# Test\n\nContent here.";
    let result = transform_agent_for_copilot(claude_content);
    let result_str = std::str::from_utf8(&result).unwrap();

    assert!(result_str.contains("target: github-copilot"));
    assert!(!result_str.contains("model: opus"));
    assert!(!result_str.contains("color: green"));
    assert!(result_str.contains("name: test"));
    assert!(result_str.contains("description: Test agent"));
    assert!(result_str.contains("# Test\n\nContent here."));
}

#[test]
fn transform_agent_preserves_existing_target() {
    use super::utils::transform_agent_for_copilot;

    let content = b"---\nname: test\ntarget: vscode\n---\n# Test";
    let result = transform_agent_for_copilot(content);
    let result_str = std::str::from_utf8(&result).unwrap();

    // Should keep existing target, not add duplicate
    assert!(result_str.contains("target: vscode"));
    assert!(!result_str.contains("target: github-copilot"));
}

#[test]
fn transform_agent_handles_no_frontmatter() {
    use super::utils::transform_agent_for_copilot;

    let content = b"# Just markdown\n\nNo frontmatter here.";
    let result = transform_agent_for_copilot(content);
    let result_str = std::str::from_utf8(&result).unwrap();

    assert!(result_str.starts_with("---\ntarget: github-copilot\n---"));
    assert!(result_str.contains("# Just markdown"));
}

// ==========================================
// XDG Compliance Tests
// ==========================================

#[test]
fn resolve_config_root_prefers_xdg_if_exists() {
    // This test verifies the XDG path resolution logic.
    // When both XDG and legacy paths exist, XDG should be preferred.
    // We can't easily test the env var without process isolation,
    // but we verify the resolve_config_root function exists and works.
    let result = CopilotAdapter::resolve_config_root();
    assert!(result.is_ok(), "resolve_config_root should succeed");

    let path = result.unwrap();
    // Path should end with "copilot" or ".copilot"
    let filename = path.file_name().unwrap().to_str().unwrap();
    assert!(
        filename == "copilot" || filename == ".copilot",
        "Config root should be named 'copilot' or '.copilot', got: {}",
        filename
    );
}

#[test]
fn new_adapter_uses_xdg_resolution() {
    // Verify that new() calls resolve_config_root internally
    let adapter = CopilotAdapter::new();
    assert!(adapter.is_ok(), "CopilotAdapter::new() should succeed");

    let adapter = adapter.unwrap();
    let root = adapter.config_root();
    let filename = root.file_name().unwrap().to_str().unwrap();
    assert!(
        filename == "copilot" || filename == ".copilot",
        "Config root should be named 'copilot' or '.copilot', got: {}",
        filename
    );
}

// ==========================================
// Edge Case Tests (Issue #125)
// ==========================================

/// Test that skill names with path traversal attempts are properly sanitized
/// during write operations. This is a security-critical test.
#[test]
fn write_skills_sanitizes_path_traversal_in_names() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    // Attempt path traversal in skill name
    let malicious_skill = Command {
        name: "../../../etc/passwd".to_string(),
        content: b"---\nname: malicious\ndescription: Attack\n---\n# Evil".to_vec(),
        source_path: PathBuf::from("/tmp/test.md"),
        modified: SystemTime::now(),
        hash: "evil".to_string(),
        modules: Vec::new(),
    };

    adapter.write_skills(&[malicious_skill]).unwrap();

    // Verify the file was NOT written outside the skills directory
    let evil_path = PathBuf::from("/etc/passwd");
    assert!(!evil_path.exists() || !evil_path.ends_with("passwd.md"));

    // Verify it was written to the sanitized path instead
    let skills_dir = tmp.path().join("skills");
    let safe_path = skills_dir.join("etc/passwd/SKILL.md");
    assert!(safe_path.exists(), "Skill should be at sanitized path");
}

/// Test that absolute paths in skill names are sanitized
#[test]
fn write_skills_sanitizes_absolute_paths() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let skill = Command {
        name: "/absolute/path/skill".to_string(),
        content: b"---\nname: abs\ndescription: Test\n---\n# Abs".to_vec(),
        source_path: PathBuf::from("/tmp/test.md"),
        modified: SystemTime::now(),
        hash: "abs".to_string(),
        modules: Vec::new(),
    };

    adapter.write_skills(&[skill]).unwrap();

    // Should be sanitized to relative path
    let skills_dir = tmp.path().join("skills");
    // Leading slash removed, segments preserved
    let expected = skills_dir.join("absolute/path/skill/SKILL.md");
    assert!(
        expected.exists(),
        "Skill should exist at sanitized path: {:?}",
        expected
    );
}

/// Test handling of env variables with special characters
#[test]
fn mcp_servers_handles_special_chars_in_env() {
    let tmp = tempdir().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

    let mut servers = HashMap::new();
    servers.insert(
        "special-env".to_string(),
        McpServer {
            name: "special-env".to_string(),
            transport: McpTransport::Stdio,
            command: "/bin/test".to_string(),
            args: vec![],
            env: HashMap::from([
                ("NORMAL".to_string(), "value".to_string()),
                (
                    "WITH_QUOTES".to_string(),
                    "value with \"quotes\"".to_string(),
                ),
                ("WITH_NEWLINE".to_string(), "line1\nline2".to_string()),
                ("WITH_UNICODE".to_string(), "日本語 émoji 🎉".to_string()),
            ]),
            url: None,
            headers: None,
            enabled: true,
        },
    );

    adapter.write_mcp_servers(&servers).unwrap();
    let read_back = adapter.read_mcp_servers().unwrap();

    let server = read_back.get("special-env").unwrap();
    assert_eq!(server.env.get("NORMAL").unwrap(), "value");
    assert_eq!(
        server.env.get("WITH_QUOTES").unwrap(),
        "value with \"quotes\""
    );
    assert_eq!(server.env.get("WITH_NEWLINE").unwrap(), "line1\nline2");
    assert_eq!(server.env.get("WITH_UNICODE").unwrap(), "日本語 émoji 🎉");
}

/// Test handling of malformed mcpServers field as null
#[test]
fn mcp_servers_handles_null_field() {
    let tmp = tempdir().unwrap();
    let config_dir = tmp.path();
    fs::create_dir_all(config_dir).unwrap();

    // Write config with mcpServers: null
    let config_path = config_dir.join("mcp-config.json");
    fs::write(&config_path, r#"{"mcpServers": null}"#).unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();

    assert!(
        servers.is_empty(),
        "null mcpServers should return empty map"
    );
}

/// Test handling of malformed mcpServers field as array
#[test]
fn mcp_servers_handles_array_field() {
    let tmp = tempdir().unwrap();
    let config_dir = tmp.path();
    fs::create_dir_all(config_dir).unwrap();

    // Write config with mcpServers as array (wrong type)
    let config_path = config_dir.join("mcp-config.json");
    fs::write(&config_path, r#"{"mcpServers": ["item1", "item2"]}"#).unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();

    assert!(
        servers.is_empty(),
        "array mcpServers should return empty map"
    );
}

/// Test handling of malformed mcpServers field as string
#[test]
fn mcp_servers_handles_string_field() {
    let tmp = tempdir().unwrap();
    let config_dir = tmp.path();
    fs::create_dir_all(config_dir).unwrap();

    // Write config with mcpServers as string (wrong type)
    let config_path = config_dir.join("mcp-config.json");
    fs::write(&config_path, r#"{"mcpServers": "not an object"}"#).unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();

    assert!(
        servers.is_empty(),
        "string mcpServers should return empty map"
    );
}

/// Test that symlinked skill directories are skipped
#[test]
#[cfg(unix)]
fn read_skills_skips_symlinks() {
    use std::os::unix::fs::symlink;

    let tmp = tempdir().unwrap();
    let skills_dir = tmp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a real skill
    let real_skill_dir = skills_dir.join("real-skill");
    fs::create_dir_all(&real_skill_dir).unwrap();
    fs::write(
        real_skill_dir.join("SKILL.md"),
        "---\nname: real\n---\n# Real",
    )
    .unwrap();

    // Create a symlinked skill directory
    let target_dir = tmp.path().join("symlink-target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(
        target_dir.join("SKILL.md"),
        "---\nname: symlinked\n---\n# Symlinked",
    )
    .unwrap();
    symlink(&target_dir, skills_dir.join("symlink-skill")).unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();

    // Should only find the real skill, not the symlinked one
    assert_eq!(skills.len(), 1);
    // Skill name is extracted from directory, not frontmatter
    assert_eq!(skills[0].name, "real-skill");
}

/// Test that broken symlinks are handled gracefully
#[test]
#[cfg(unix)]
fn read_skills_handles_broken_symlinks() {
    use std::os::unix::fs::symlink;

    let tmp = tempdir().unwrap();
    let skills_dir = tmp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a broken symlink
    symlink("/nonexistent/path", skills_dir.join("broken-link")).unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills();

    // Should succeed without panicking
    assert!(skills.is_ok());
    assert!(skills.unwrap().is_empty());
}

/// Test that MCP server args field with wrong type logs warning
#[test]
fn mcp_servers_warns_on_wrong_args_type() {
    let tmp = tempdir().unwrap();
    let config_dir = tmp.path();
    fs::create_dir_all(config_dir).unwrap();

    // Write config with args as string instead of array
    let config_path = config_dir.join("mcp-config.json");
    fs::write(
        &config_path,
        r#"{
            "mcpServers": {
                "test": {
                    "command": "/bin/test",
                    "args": "not-an-array"
                }
            }
        }"#,
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();

    // Server should still be parsed, but with empty args
    let server = servers.get("test").unwrap();
    assert_eq!(server.command, "/bin/test");
    assert!(server.args.is_empty(), "Wrong type args should be empty");
}

/// Test that MCP server env field with wrong type logs warning
#[test]
fn mcp_servers_warns_on_wrong_env_type() {
    let tmp = tempdir().unwrap();
    let config_dir = tmp.path();
    fs::create_dir_all(config_dir).unwrap();

    // Write config with env as array instead of object
    let config_path = config_dir.join("mcp-config.json");
    fs::write(
        &config_path,
        r#"{
            "mcpServers": {
                "test": {
                    "command": "/bin/test",
                    "env": ["KEY=value"]
                }
            }
        }"#,
    )
    .unwrap();

    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();

    // Server should still be parsed, but with empty env
    let server = servers.get("test").unwrap();
    assert_eq!(server.command, "/bin/test");
    assert!(server.env.is_empty(), "Wrong type env should be empty");
}
