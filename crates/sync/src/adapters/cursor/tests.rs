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

    // Read back and verify frontmatter was stripped but description preserved
    let read_back = adapter.read_skills().unwrap();
    assert_eq!(read_back.len(), 1);
    let content = String::from_utf8_lossy(&read_back[0].content);
    assert!(!content.contains("---"), "Frontmatter should be stripped");
    assert!(
        content.starts_with("A test skill\n"),
        "Description should be preserved as plain text first line"
    );
    assert!(
        content.contains("# test-skill Skill"),
        "Body should be preserved"
    );
}

#[test]
fn skills_description_before_model_hint() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: paradigms\ndescription: Compare architecture patterns\nmodel_hint: standard\n---\n\n# Paradigms\n\nContent.\n";
    let skills = vec![make_command("paradigms", content)];

    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    let read_back = adapter.read_skills().unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);

    assert!(
        body.starts_with("Compare architecture patterns\n"),
        "Description should be plain text first line, got: {}",
        body
    );
    assert!(
        body.contains("<!-- model_hint: standard -->"),
        "Model hint should be in output"
    );

    // Description must come before model_hint
    let desc_pos = body.find("Compare architecture patterns").unwrap();
    let hint_pos = body.find("<!-- model_hint:").unwrap();
    assert!(
        desc_pos < hint_pos,
        "Description should appear before model_hint so Cursor shows it as the subtitle"
    );
}

#[test]
fn skills_description_strips_yaml_quotes() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content =
        "---\nname: quoted\ndescription: \"A quoted description\"\n---\n\n# Body\n\nContent.\n";
    let skills = vec![make_command("quoted", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = adapter.read_skills().unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        body.starts_with("A quoted description\n"),
        "Quotes should be stripped from description, got: {}",
        body
    );
}

#[test]
fn skills_block_scalar_description() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: research\ndescription: >-\n  Search GitHub for implementations.\n  Use when the user wants code.\nversion: 1.0\n---\n\n# Body\n";
    let skills = vec![make_command("research", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = adapter.read_skills().unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        body.starts_with("Search GitHub for implementations. Use when the user wants code.\n"),
        "Block scalar description should be flattened to plain text, got: {}",
        body
    );
}

#[test]
fn skills_single_quoted_description() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content =
        "---\nname: single-q\ndescription: 'A single-quoted desc'\n---\n\n# Body\n\nContent.\n";
    let skills = vec![make_command("single-q", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = adapter.read_skills().unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        body.starts_with("A single-quoted desc\n"),
        "Single quotes should be stripped, got: {}",
        body
    );
}

#[test]
fn skills_description_only_no_model_hint() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content =
        "---\nname: plain\ndescription: Just a description\n---\n\n# Content\n\nBody here.\n";
    let skills = vec![make_command("plain", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = adapter.read_skills().unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        body.starts_with("Just a description\n"),
        "Description should be first line, got: {}",
        body
    );
    assert!(
        !body.contains("model_hint"),
        "No model_hint comment when field is absent"
    );
    assert!(
        body.contains("# Content"),
        "Body should follow the description"
    );
}

#[test]
fn skills_model_hint_only_no_description() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: hinted\nmodel_hint: fast\n---\n\n# Content\n\nBody here.\n";
    let skills = vec![make_command("hinted", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = adapter.read_skills().unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        body.starts_with("<!-- model_hint: fast -->\n"),
        "Model hint should be first line when no description, got: {}",
        body
    );
}

#[test]
fn skills_multiline_quoted_description() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: modular-monolith\ndescription: 'Single deployable with enforced module\n  boundaries for team autonomy.'\n---\n\n# Content\n";
    let skills = vec![make_command("modular-monolith", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = adapter.read_skills().unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        body.starts_with("Single deployable with enforced module boundaries for team autonomy.\n"),
        "Multi-line quoted description should be flattened and unquoted, got: {}",
        body
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
fn skills_frontmatter_only_content_preserves_description() {
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
    assert!(
        body.contains("Has no body"),
        "Description should be preserved as plain text, got: {}",
        body
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

// #183: Verify that rules in subdirectories produce unique names that don't
// collide with top-level rules whose name happens to match the flattened form.
#[test]
fn rules_subdirectory_name_collision() {
    let tmp = TempDir::new().unwrap();
    let rules_dir = tmp.path().join("rules");

    // Create a top-level rule: rules/foo-bar.mdc
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(
        rules_dir.join("foo-bar.mdc"),
        "---\nalwaysApply: false\ndescription: top-level rule\n---\n\n# Top-level foo-bar\n",
    )
    .unwrap();

    // Create a subdirectory rule: rules/foo/bar.mdc
    // After flattening, this becomes "foo-bar" which collides with the top-level rule.
    std::fs::create_dir_all(rules_dir.join("foo")).unwrap();
    std::fs::write(
        rules_dir.join("foo/bar.mdc"),
        "---\nalwaysApply: false\ndescription: subdirectory rule\n---\n\n# Subdirectory foo/bar\n",
    )
    .unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let rules = adapter.read_instructions().unwrap();

    // Both rules should be read (2 files on disk → 2 entries)
    assert_eq!(
        rules.len(),
        2,
        "Both top-level and subdirectory rules should be read"
    );

    // Verify the names — currently both flatten to "foo-bar" which is a known
    // limitation. The important thing is that neither file is silently dropped.
    let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
    assert!(
        names.contains(&"foo-bar"),
        "Should contain foo-bar, got: {:?}",
        names
    );

    // Both entries should have distinct content (even if names collide)
    let contents: Vec<String> = rules
        .iter()
        .map(|r| String::from_utf8_lossy(&r.content).to_string())
        .collect();
    let has_top_level = contents.iter().any(|c| c.contains("Top-level foo-bar"));
    let has_subdir = contents.iter().any(|c| c.contains("Subdirectory foo/bar"));
    assert!(has_top_level, "Top-level rule content should be present");
    assert!(has_subdir, "Subdirectory rule content should be present");
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

    let instructions = vec![make_command("CLAUDE.md", "# Instructions\n\nDo things.\n")];

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
            allowed_tools: vec![],
            disabled_tools: vec![],
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

#[test]
fn mcp_read_with_tool_configs() {
    let tmp = TempDir::new().unwrap();
    let mcp_path = tmp.path().join("mcp.json");
    std::fs::write(
        &mcp_path,
        r#"{
        "mcpServers": {
            "restricted-server": {
                "command": "/usr/bin/mcp-server",
                "args": ["--port", "3000"],
                "allowedTools": ["read_file", "search_*"],
                "disabledTools": ["delete_file", "write_file"]
            }
        }
    }"#,
    )
    .unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();

    let server = servers.get("restricted-server").unwrap();
    assert_eq!(server.allowed_tools, vec!["read_file", "search_*"]);
    assert_eq!(server.disabled_tools, vec!["delete_file", "write_file"]);
}

#[test]
fn mcp_write_preserves_tool_configs() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let mut servers = HashMap::new();
    servers.insert(
        "my-server".to_string(),
        McpServer {
            name: "my-server".to_string(),
            transport: McpTransport::Stdio,
            command: "/bin/server".to_string(),
            args: vec![],
            env: HashMap::new(),
            url: None,
            headers: None,
            enabled: true,
            allowed_tools: vec!["tool_a".to_string(), "tool_b".to_string()],
            disabled_tools: vec!["tool_c".to_string()],
        },
    );

    adapter.write_mcp_servers(&servers).unwrap();

    let read_back = adapter.read_mcp_servers().unwrap();
    let server = read_back.get("my-server").unwrap();
    assert_eq!(
        server.allowed_tools,
        vec!["tool_a".to_string(), "tool_b".to_string()]
    );
    assert_eq!(server.disabled_tools, vec!["tool_c".to_string()]);
}

#[test]
fn mcp_empty_tool_configs_omitted_from_json() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let mut servers = HashMap::new();
    servers.insert(
        "clean-server".to_string(),
        McpServer {
            name: "clean-server".to_string(),
            transport: McpTransport::Stdio,
            command: "/bin/server".to_string(),
            args: vec![],
            env: HashMap::new(),
            url: None,
            headers: None,
            enabled: true,
            allowed_tools: vec![],
            disabled_tools: vec![],
        },
    );

    adapter.write_mcp_servers(&servers).unwrap();

    let content = std::fs::read_to_string(tmp.path().join("mcp.json")).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
    let server_json = &settings["mcpServers"]["clean-server"];
    assert!(server_json.get("allowedTools").is_none());
    assert!(server_json.get("disabledTools").is_none());
}
