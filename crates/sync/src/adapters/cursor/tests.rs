//! Integration tests for the Cursor adapter.

use super::*;
use crate::adapters::traits::AgentAdapter;
use crate::adapters::utils::test_helpers::make_command;
use crate::common::{McpServer, McpTransport, ModuleFile};
use tempfile::TempDir;

fn make_skill_with_frontmatter(name: &str) -> Command {
    let content = format!(
        "---\nname: {}\ndescription: A test skill\ncategory: testing\ntags:\n  - test\n---\n\n# {} Skill\n\nDo the thing.\n",
        name, name
    );
    make_command(name, &content)
}

fn make_skill_with_modules(name: &str) -> Command {
    let mut cmd = make_skill_with_frontmatter(name);
    cmd.modules = vec![ModuleFile {
        relative_path: std::path::PathBuf::from("modules/helper.md"),
        content: b"# Helper\n\nHelper content.\n".to_vec(),
        hash: "module-hash".to_string(),
    }];
    cmd
}

// --- Adapter basics ---

#[test]
fn adapter_name_is_cursor() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    assert_eq!(adapter.name(), "cursor");
}

#[test]
fn adapter_config_root() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    assert_eq!(adapter.config_root(), tmp.path().to_path_buf());
}

#[test]
fn adapter_supports_expected_fields() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let fields = adapter.supported_fields();
    assert!(fields.commands);
    assert!(fields.mcp_servers);
    assert!(!fields.preferences, "Cursor preferences are not yet mapped");
    assert!(fields.skills);
    assert!(fields.hooks);
    assert!(fields.agents);
    assert!(fields.instructions);
}

// --- Commands ---

#[test]
fn commands_round_trip() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let commands = vec![
        make_command("review-code", "# Review Code\n\nReview the code.\n"),
        make_command("deploy", "# Deploy\n\nDeploy to staging.\n"),
    ];

    let report = adapter.write_commands(&commands).unwrap();
    assert_eq!(report.written, 2);

    let read_back = adapter.read_commands(false).unwrap();
    assert_eq!(read_back.len(), 2);
    assert_eq!(read_back[0].name, "deploy");
    assert_eq!(read_back[1].name, "review-code");
}

#[test]
fn commands_empty_directory() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let commands = adapter.read_commands(false).unwrap();
    assert!(commands.is_empty());
}

// --- Skills ---

#[test]
fn skills_strip_frontmatter_on_write() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let skills = vec![make_skill_with_frontmatter("test-skill")];
    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    // Read back and verify frontmatter was stripped
    let read_back = adapter.read_skills().unwrap();
    assert_eq!(read_back.len(), 1);
    let content = String::from_utf8_lossy(&read_back[0].content);
    assert!(!content.contains("---"), "Frontmatter should be stripped");
    assert!(
        content.contains("# test-skill Skill"),
        "Body should be preserved"
    );
}

#[test]
fn skills_preserve_modules() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let skills = vec![make_skill_with_modules("modular-skill")];
    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    // Verify module file was written
    let module_path = tmp.path().join("skills/modular-skill/modules/helper.md");
    assert!(module_path.exists());
}

#[test]
fn skills_empty_directory() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();
    assert!(skills.is_empty());
}

// --- Agents ---

#[test]
fn agents_translate_frontmatter_on_write() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let agent_content = "---\nname: reviewer\nbackground: true\ntools: [Read, Write]\nisolation: worktree\nmodel: claude-sonnet-4-6\n---\n\nReview code thoroughly.\n";
    let agents = vec![make_command("reviewer", agent_content)];

    let report = adapter.write_agents(&agents).unwrap();
    assert_eq!(report.written, 1);

    let read_back = adapter.read_agents().unwrap();
    assert_eq!(read_back.len(), 1);
    let content = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        content.contains("is_background: true"),
        "background should be renamed"
    );
    assert!(!content.contains("tools:"), "tools should be stripped");
    assert!(
        !content.contains("isolation:"),
        "isolation should be stripped"
    );
}

// --- Hooks ---

#[test]
fn hooks_event_name_translation() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let hook_entries = r#"[{"command": "./hooks/lint.sh", "type": "command", "timeout": 5}]"#;
    let hooks = vec![make_command("PreToolUse", hook_entries)];

    let report = adapter.write_hooks(&hooks).unwrap();
    assert_eq!(report.written, 1);

    // Verify the file uses camelCase
    let hooks_file = tmp.path().join("hooks.json");
    let content = std::fs::read_to_string(&hooks_file).unwrap();
    assert!(content.contains("preToolUse"), "Event should be camelCase");
    assert!(
        !content.contains("PreToolUse"),
        "Should not have PascalCase"
    );
}

#[test]
fn hooks_skip_unmappable_events() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let hooks = vec![make_command(
        "Notification",
        r#"[{"command": "./notify.sh"}]"#,
    )];

    let report = adapter.write_hooks(&hooks).unwrap();
    assert_eq!(report.written, 0);
    assert_eq!(report.skipped.len(), 1);
    assert!(matches!(
        &report.skipped[0],
        crate::report::SkipReason::AgentSpecificFeature { .. }
    ));
}

// --- Rules ---

#[test]
fn rules_claude_md_becomes_always_apply() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let instructions = vec![make_command(
        "CLAUDE.md",
        "# Project Instructions\n\nDo things.\n",
    )];

    let report = adapter.write_instructions(&instructions).unwrap();
    assert_eq!(report.written, 1);

    let rule_path = tmp.path().join("rules/claude-md.mdc");
    assert!(rule_path.exists());
    let content = std::fs::read_to_string(&rule_path).unwrap();
    assert!(content.contains("alwaysApply: true"));
}

#[test]
fn rules_with_globs_preserved() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nglobs: \"**/*.test.ts\"\ndescription: Test guidelines\n---\n\n# Testing\n\nWrite good tests.\n";
    let instructions = vec![make_command("testing", content)];

    let report = adapter.write_instructions(&instructions).unwrap();
    assert_eq!(report.written, 1);

    let read_back = adapter.read_instructions().unwrap();
    assert_eq!(read_back.len(), 1);
    let content = String::from_utf8_lossy(&read_back[0].content);
    assert!(content.contains("globs:"));
    assert!(content.contains("alwaysApply: false"));
}

// --- Skills edge cases ---

#[test]
fn skills_frontmatter_only_content_produces_empty_body() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    // Skill with frontmatter but no body content
    let content = "---\nname: empty-body\ndescription: Has no body\n---\n";
    let skills = vec![make_command("empty-body", content)];

    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    let read_back = adapter.read_skills().unwrap();
    assert_eq!(read_back.len(), 1);
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(!body.contains("---"), "Frontmatter should be stripped");
    // Body may be empty or whitespace-only
    assert!(
        body.trim().is_empty(),
        "Body should be empty after stripping frontmatter-only content"
    );
}

#[test]
fn skills_no_frontmatter_preserved_verbatim() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    // Skill without frontmatter (already in Cursor format)
    let content = "# My Skill\n\nDo the thing.\n";
    let skills = vec![make_command("plain-skill", content)];

    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    let read_back = adapter.read_skills().unwrap();
    assert_eq!(read_back.len(), 1);
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert_eq!(
        body, content,
        "Content without frontmatter should be preserved exactly"
    );
}

#[test]
fn skills_module_path_traversal_blocked() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let mut skill = make_skill_with_frontmatter("safe-skill");
    // Use ../malicious.md — the parent (.../skills/) exists after skill dir is created,
    // so is_path_contained can canonicalize the parent and detect the escape.
    skill.modules = vec![ModuleFile {
        relative_path: std::path::PathBuf::from("../malicious.md"),
        content: b"should not be written".to_vec(),
        hash: "bad-hash".to_string(),
    }];

    let skills = vec![skill];
    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    // The malicious module should NOT be written outside the skill directory
    let malicious_path = tmp.path().join("skills/malicious.md");
    assert!(!malicious_path.exists(), "Path traversal should be blocked");
}

#[test]
fn skills_hidden_directories_skipped_on_read() {
    let tmp = TempDir::new().unwrap();
    let skills_dir = tmp.path().join("skills");

    // Create a hidden skill directory
    std::fs::create_dir_all(skills_dir.join(".hidden-skill")).unwrap();
    std::fs::write(
        skills_dir.join(".hidden-skill/SKILL.md"),
        "# Hidden\n\nShould be skipped.\n",
    )
    .unwrap();

    // Create a visible skill directory
    std::fs::create_dir_all(skills_dir.join("visible-skill")).unwrap();
    std::fs::write(
        skills_dir.join("visible-skill/SKILL.md"),
        "# Visible\n\nShould be found.\n",
    )
    .unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();
    assert_eq!(skills.len(), 1, "Hidden directories should be skipped");
    assert_eq!(skills[0].name, "visible-skill");
}

// S5: Verify read_agents returns empty vec when agents directory doesn't exist
#[test]
fn agents_empty_directory() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let agents = adapter.read_agents().unwrap();
    assert!(
        agents.is_empty(),
        "read_agents should return empty vec when agents directory doesn't exist"
    );
}

// S6: Verify alwaysApply: true is preserved in a round-trip through rules
#[test]
fn rules_always_apply_true_passthrough() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content =
        "---\nalwaysApply: true\ndescription: Important rule\n---\n\n# Always Active\n\nThis rule always applies.\n";
    let instructions = vec![make_command("always-on", content)];

    let report = adapter.write_instructions(&instructions).unwrap();
    assert_eq!(report.written, 1);

    let read_back = adapter.read_instructions().unwrap();
    assert_eq!(read_back.len(), 1);
    let read_content = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        read_content.contains("alwaysApply: true"),
        "alwaysApply: true should be preserved in round-trip, got: {}",
        read_content
    );
}

// --- Skip-unchanged tests ---

#[test]
fn commands_skip_unchanged_on_second_write() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let commands = vec![make_command("greet", "# Greet\n\nSay hello.\n")];

    let first = adapter.write_commands(&commands).unwrap();
    assert_eq!(first.written, 1);
    assert!(first.skipped.is_empty());

    let second = adapter.write_commands(&commands).unwrap();
    assert_eq!(second.written, 0, "unchanged command should be skipped");
    assert_eq!(second.skipped.len(), 1);
}

#[test]
fn commands_overwrite_when_content_changes() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let v1 = vec![make_command("greet", "# V1\n")];
    let first = adapter.write_commands(&v1).unwrap();
    assert_eq!(first.written, 1);

    let v2 = vec![make_command("greet", "# V2\n")];
    let second = adapter.write_commands(&v2).unwrap();
    assert_eq!(second.written, 1, "changed command should be written");
    assert!(second.skipped.is_empty());
}

#[test]
fn skills_skip_unchanged_on_second_write() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let skills = vec![make_skill_with_frontmatter("repeat-skill")];

    let first = adapter.write_skills(&skills).unwrap();
    assert_eq!(first.written, 1);

    let second = adapter.write_skills(&skills).unwrap();
    assert_eq!(second.written, 0, "unchanged skill should be skipped");
    assert_eq!(second.skipped.len(), 1);
}

#[test]
fn agents_skip_unchanged_on_second_write() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let agent_content = "---\nname: helper\nmodel: opus\n---\n\nHelp out.\n";
    let agents = vec![make_command("helper", agent_content)];

    let first = adapter.write_agents(&agents).unwrap();
    assert_eq!(first.written, 1);

    let second = adapter.write_agents(&agents).unwrap();
    assert_eq!(second.written, 0, "unchanged agent should be skipped");
    assert_eq!(second.skipped.len(), 1);
}

#[test]
fn rules_skip_unchanged_on_second_write() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let instructions = vec![make_command(
        "CLAUDE.md",
        "# Instructions\n\nDo things.\n",
    )];

    let first = adapter.write_instructions(&instructions).unwrap();
    assert_eq!(first.written, 1);

    let second = adapter.write_instructions(&instructions).unwrap();
    assert_eq!(second.written, 0, "unchanged rule should be skipped");
    assert_eq!(second.skipped.len(), 1);
}

// --- MCP ---

#[test]
fn mcp_round_trip() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let mut servers = HashMap::new();
    servers.insert(
        "test-server".to_string(),
        McpServer {
            name: "test-server".to_string(),
            transport: McpTransport::Stdio,
            command: "/usr/bin/test-server".to_string(),
            args: vec!["--port".to_string(), "3000".to_string()],
            env: HashMap::new(),
            url: None,
            headers: None,
            enabled: true,
        },
    );

    let report = adapter.write_mcp_servers(&servers).unwrap();
    assert_eq!(report.written, 1);

    let read_back = adapter.read_mcp_servers().unwrap();
    assert_eq!(read_back.len(), 1);
    let server = read_back.get("test-server").unwrap();
    assert_eq!(server.command, "/usr/bin/test-server");
    assert_eq!(server.args, vec!["--port", "3000"]);
}
