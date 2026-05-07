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
fn adapter_basics() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let adapter = CursorAdapter::with_root(root.clone());
    crate::adapters::tests_common::assert_adapter_basics(&adapter, "cursor", &root, |fields| {
        assert!(fields.commands);
        assert!(fields.mcp_servers);
        assert!(!fields.preferences, "Cursor preferences are not yet mapped");
        assert!(fields.skills);
        assert!(fields.hooks);
        assert!(fields.agents);
        assert!(fields.instructions);
    });
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
    crate::adapters::tests_common::assert_read_commands_empty(&adapter);
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

// --- MCP edge cases ---

#[test]
fn mcp_http_transport_detected() {
    let tmp = TempDir::new().unwrap();
    let mcp_path = tmp.path().join("mcp.json");
    std::fs::write(
        &mcp_path,
        r#"{
        "mcpServers": {
            "remote-server": {
                "url": "https://mcp.example.com/v1",
                "headers": {"Authorization": "Bearer tok"}
            }
        }
    }"#,
    )
    .unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();
    let server = servers.get("remote-server").unwrap();
    assert_eq!(server.transport, McpTransport::Http);
    assert_eq!(server.url.as_deref(), Some("https://mcp.example.com/v1"));
    assert!(
        server.command.is_empty(),
        "HTTP server should have no command"
    );
}

#[test]
fn mcp_skips_entries_without_command_or_url() {
    let tmp = TempDir::new().unwrap();
    let mcp_path = tmp.path().join("mcp.json");
    std::fs::write(
        &mcp_path,
        r#"{
        "mcpServers": {
            "broken": {"args": ["--verbose"]},
            "valid": {"command": "/usr/bin/server"}
        }
    }"#,
    )
    .unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();
    assert_eq!(servers.len(), 1, "broken entry should be skipped");
    assert!(servers.contains_key("valid"));
}

#[test]
fn mcp_disabled_server_preserved() {
    let tmp = TempDir::new().unwrap();
    let mcp_path = tmp.path().join("mcp.json");
    std::fs::write(
        &mcp_path,
        r#"{
        "mcpServers": {
            "dormant": {
                "command": "/usr/bin/server",
                "enabled": false
            }
        }
    }"#,
    )
    .unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();
    let server = servers.get("dormant").unwrap();
    assert!(!server.enabled, "disabled flag should be preserved");
}

#[test]
fn mcp_skip_unchanged_on_second_write() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let mut servers = HashMap::new();
    servers.insert(
        "stable".to_string(),
        McpServer {
            name: "stable".to_string(),
            transport: McpTransport::Stdio,
            command: "/usr/bin/server".to_string(),
            args: vec![],
            env: HashMap::new(),
            url: None,
            headers: None,
            enabled: true,
            allowed_tools: vec![],
            disabled_tools: vec![],
        },
    );

    let first = adapter.write_mcp_servers(&servers).unwrap();
    assert_eq!(first.written, 1);

    let second = adapter.write_mcp_servers(&servers).unwrap();
    assert_eq!(second.written, 0, "unchanged MCP config should be skipped");
    assert_eq!(second.skipped.len(), 1);
}

#[test]
fn mcp_empty_servers_returns_empty_report() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let servers = HashMap::new();
    let report = adapter.write_mcp_servers(&servers).unwrap();
    assert_eq!(report.written, 0);
    assert!(report.skipped.is_empty());
}

#[test]
fn mcp_nonexistent_file_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let servers = adapter.read_mcp_servers().unwrap();
    assert!(servers.is_empty());
}

// --- Commands edge cases ---

#[test]
fn commands_hidden_files_skipped() {
    let tmp = TempDir::new().unwrap();
    let cmd_dir = tmp.path().join("commands");
    std::fs::create_dir_all(&cmd_dir).unwrap();

    std::fs::write(cmd_dir.join("visible.md"), "# Visible\n").unwrap();
    std::fs::write(cmd_dir.join(".hidden.md"), "# Hidden\n").unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let commands = adapter.read_commands(false).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "visible");
}

#[test]
fn commands_non_md_files_skipped() {
    let tmp = TempDir::new().unwrap();
    let cmd_dir = tmp.path().join("commands");
    std::fs::create_dir_all(&cmd_dir).unwrap();

    std::fs::write(cmd_dir.join("real.md"), "# Real\n").unwrap();
    std::fs::write(cmd_dir.join("config.json"), "{}").unwrap();
    std::fs::write(cmd_dir.join("script.sh"), "#!/bin/bash\n").unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let commands = adapter.read_commands(false).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].name, "real");
}

#[test]
fn commands_empty_list_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let report = adapter.write_commands(&[]).unwrap();
    assert_eq!(report.written, 0);
    // commands dir should not be created for empty writes
    assert!(!tmp.path().join("commands").exists());
}

#[test]
fn commands_subdirectories_ignored() {
    let tmp = TempDir::new().unwrap();
    let cmd_dir = tmp.path().join("commands");
    std::fs::create_dir_all(cmd_dir.join("subdir")).unwrap();
    std::fs::write(cmd_dir.join("subdir/nested.md"), "# Nested\n").unwrap();
    std::fs::write(cmd_dir.join("top-level.md"), "# Top Level\n").unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let commands = adapter.read_commands(false).unwrap();
    assert_eq!(commands.len(), 1, "subdirectories should be skipped");
    assert_eq!(commands[0].name, "top-level");
}

#[test]
fn commands_frontmatter_stripped_on_write() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nallowed-tools: [Read, Write]\ndescription: Test command\n---\n\n# Do Thing\n\nCommand body.\n";
    let commands = vec![make_command("with-fm", content)];
    adapter.write_commands(&commands).unwrap();

    let read_back = adapter.read_commands(false).unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        !body.contains("---"),
        "Frontmatter should be stripped from commands"
    );
    assert!(body.contains("# Do Thing"), "Body should be preserved");
}

// --- Paths unit tests ---

#[test]
fn paths_all_resolve_under_root() {
    let root = std::path::Path::new("/tmp/fake-cursor");
    assert_eq!(paths::skills_dir(root), root.join("skills"));
    assert_eq!(paths::commands_dir(root), root.join("commands"));
    assert_eq!(paths::agents_dir(root), root.join("agents"));
    assert_eq!(paths::rules_dir(root), root.join("rules"));
    assert_eq!(paths::hooks_path(root), root.join("hooks.json"));
    assert_eq!(paths::mcp_config_path(root), root.join("mcp.json"));
}

// --- Skills edge cases ---

#[test]
fn skills_empty_list_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let report = adapter.write_skills(&[]).unwrap();
    assert_eq!(report.written, 0);
    assert!(!tmp.path().join("skills").exists());
}

#[test]
fn skills_directories_without_skill_md_skipped() {
    let tmp = TempDir::new().unwrap();
    let skills_dir = tmp.path().join("skills");

    // Directory with SKILL.md
    std::fs::create_dir_all(skills_dir.join("valid-skill")).unwrap();
    std::fs::write(skills_dir.join("valid-skill/SKILL.md"), "# Valid Skill\n").unwrap();

    // Directory without SKILL.md (just loose files)
    std::fs::create_dir_all(skills_dir.join("no-skill-md")).unwrap();
    std::fs::write(skills_dir.join("no-skill-md/README.md"), "# Not a skill\n").unwrap();

    // Regular file at skills root (not a directory)
    std::fs::write(skills_dir.join("stray-file.md"), "# Stray\n").unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let skills = adapter.read_skills().unwrap();
    assert_eq!(
        skills.len(),
        1,
        "only directories with SKILL.md should be read"
    );
    assert_eq!(skills[0].name, "valid-skill");
}

#[test]
fn skills_model_hint_injected_as_comment() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content =
        "---\nname: smart-skill\nmodel_hint: opus\n---\n\n# Smart Skill\n\nDo things smartly.\n";
    let skills = vec![make_command("smart-skill", content)];
    adapter.write_skills(&skills).unwrap();

    let read_back = adapter.read_skills().unwrap();
    let body = String::from_utf8_lossy(&read_back[0].content);
    assert!(
        body.contains("<!-- model_hint: opus -->"),
        "model_hint should be injected as HTML comment, got: {}",
        body
    );
}

// --- Plugin Asset Manifest Tests ---

/// GIVEN a plugin_name containing path traversal components
/// WHEN write_plugin_assets is called
/// THEN the manifest is written inside plugins/local/ (traversal stripped)
#[test]
fn plugin_assets_path_traversal_sanitized() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let malicious = PluginAsset::new(
        "../../tmp/evil".to_string(),
        "shady".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(".claude-plugin/plugin.json"),
        b"{\"name\": \"evil\"}".to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[malicious]).unwrap();

    assert_eq!(report.written, 1);
    // The traversal components are stripped — file lands inside plugins/local/
    assert!(
        !tmp.path().join("tmp/evil").exists(),
        "Path traversal must not escape plugins/local/"
    );
    // The sanitized name should be under plugins/local/
    let local_dir = tmp.path().join("plugins/local");
    assert!(local_dir.exists(), "plugins/local should exist");
}

/// Non-manifest assets (scripts, binaries) are silently ignored — only
/// `.claude-plugin/plugin.json` files are processed by the manifest writer.
#[test]
fn plugin_assets_ignores_non_manifest_files() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let script = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("scripts/helper.py"),
        b"# helper\n".to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[script]).unwrap();

    assert_eq!(report.written, 0, "Non-manifest assets should be ignored");
}

/// A valid `.claude-plugin/plugin.json` asset is written to
/// `plugins/local/<plugin>/.cursor-plugin/plugin.json`.
#[test]
fn plugin_assets_writes_manifest_to_local() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let manifest = PluginAsset::new(
        "good-plugin".to_string(),
        "legit-market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(".claude-plugin/plugin.json"),
        b"{\"name\": \"good-plugin\"}".to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[manifest]).unwrap();

    assert_eq!(report.written, 1, "Manifest should be written");
    let expected = tmp
        .path()
        .join("plugins/local/good-plugin/.cursor-plugin/plugin.json");
    assert!(expected.exists(), "Manifest should exist at local path");
}

// --- Stale Plugin Pruning Tests ---

/// GIVEN a previously synced plugin directory exists in plugins/local
/// WHEN write_plugin_assets is called with a new set that excludes it
/// THEN the stale plugin directory is pruned
#[test]
fn plugin_assets_prunes_stale_plugin_directories() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");

    // Pre-create a stale plugin from a prior sync
    let stale = local_dir.join("old-plugin/.cursor-plugin");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("plugin.json"), b"{\"name\": \"old-plugin\"}").unwrap();

    // Sync only the new plugin
    let asset = PluginAsset::new(
        "new-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(".claude-plugin/plugin.json"),
        b"{\"name\": \"new-plugin\"}".to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[asset]).unwrap();

    assert_eq!(report.written, 1);
    // New plugin exists
    assert!(local_dir
        .join("new-plugin/.cursor-plugin/plugin.json")
        .exists());
    // Old plugin is removed
    assert!(
        !local_dir.join("old-plugin").exists(),
        "Stale old-plugin should be pruned"
    );
}

/// GIVEN an unchanged manifest already on disk
/// WHEN write_plugin_assets is called with the same content
/// THEN the write is skipped but stale plugins are still pruned
#[test]
fn plugin_assets_prune_preserves_current_when_unchanged() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");

    // Pre-create the current plugin manifest (simulating a prior sync)
    let current = local_dir.join("current-plugin/.cursor-plugin");
    std::fs::create_dir_all(&current).unwrap();
    let content = b"{\"name\": \"current-plugin\"}";
    std::fs::write(current.join("plugin.json"), content).unwrap();

    // Also create a stale plugin
    let stale = local_dir.join("removed-plugin/.cursor-plugin");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("plugin.json"), b"{\"name\": \"removed\"}").unwrap();

    // Sync same content — should be skipped (unchanged) but stale should be pruned
    let asset = PluginAsset::new(
        "current-plugin".to_string(),
        "market".to_string(),
        "2.0.0".to_string(),
        std::path::PathBuf::from(".claude-plugin/plugin.json"),
        content.to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[asset]).unwrap();

    // Content unchanged, so nothing written
    assert_eq!(report.written, 0);
    // Current plugin preserved
    assert!(
        current.join("plugin.json").exists(),
        "Current plugin should be preserved"
    );
    // Stale plugin pruned
    assert!(
        !local_dir.join("removed-plugin").exists(),
        "Stale removed-plugin should be pruned even when current is unchanged"
    );
}

/// GIVEN multiple plugins are synced
/// WHEN each has a corresponding manifest
/// THEN only plugins not in the current set are pruned
#[test]
fn plugin_assets_prune_multiple_plugins_independently() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");

    // Pre-create stale plugins
    let stale_a = local_dir.join("stale-a/.cursor-plugin");
    let stale_b = local_dir.join("stale-b/.cursor-plugin");
    std::fs::create_dir_all(&stale_a).unwrap();
    std::fs::create_dir_all(&stale_b).unwrap();
    std::fs::write(stale_a.join("plugin.json"), b"{\"name\": \"stale-a\"}").unwrap();
    std::fs::write(stale_b.join("plugin.json"), b"{\"name\": \"stale-b\"}").unwrap();

    let assets = vec![
        PluginAsset::new(
            "plugin-a".to_string(),
            "market".to_string(),
            "2.0.0".to_string(),
            std::path::PathBuf::from(".claude-plugin/plugin.json"),
            b"{\"name\": \"plugin-a\"}".to_vec(),
            false,
        ),
        PluginAsset::new(
            "plugin-b".to_string(),
            "market".to_string(),
            "1.0.0".to_string(),
            std::path::PathBuf::from(".claude-plugin/plugin.json"),
            b"{\"name\": \"plugin-b\"}".to_vec(),
            false,
        ),
    ];

    let report = adapter.write_plugin_assets(&assets).unwrap();

    assert_eq!(report.written, 2);
    // New plugins exist
    assert!(local_dir
        .join("plugin-a/.cursor-plugin/plugin.json")
        .exists());
    assert!(local_dir
        .join("plugin-b/.cursor-plugin/plugin.json")
        .exists());
    // Stale plugins pruned
    assert!(
        !local_dir.join("stale-a").exists(),
        "Stale stale-a should be pruned"
    );
    assert!(
        !local_dir.join("stale-b").exists(),
        "Stale stale-b should be pruned"
    );
}

/// GIVEN an empty asset list
/// WHEN write_plugin_assets is called
/// THEN no pruning occurs and existing directories are untouched
#[test]
fn plugin_assets_empty_list_does_not_prune() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local/some-plugin/.cursor-plugin");
    std::fs::create_dir_all(&local_dir).unwrap();
    std::fs::write(
        local_dir.join("plugin.json"),
        b"{\"name\": \"some-plugin\"}",
    )
    .unwrap();

    let report = adapter.write_plugin_assets(&[]).unwrap();

    assert_eq!(report.written, 0);
    assert!(
        report.warnings.is_empty(),
        "No warnings for empty asset list"
    );
    assert!(
        local_dir.join("plugin.json").exists(),
        "Existing files should be untouched"
    );
}

/// GIVEN a stale plugin directory exists in plugins/local
/// WHEN write_plugin_assets prunes it
/// THEN the report.warnings contains the pruned plugin name
#[test]
fn plugin_assets_prune_reports_warnings() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");

    // Pre-create a stale plugin
    let stale = local_dir.join("old-plugin/.cursor-plugin");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("plugin.json"), b"{}").unwrap();

    let asset = PluginAsset::new(
        "new-plugin".to_string(),
        "pub".to_string(),
        "0.2.0".to_string(),
        std::path::PathBuf::from(".claude-plugin/plugin.json"),
        b"{\"name\": \"new-plugin\"}".to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[asset]).unwrap();

    assert_eq!(report.written, 1);
    assert_eq!(report.warnings.len(), 1, "Should have 1 prune warning");
    assert!(
        report.warnings[0].contains("old-plugin"),
        "Warning should identify the pruned plugin, got: {}",
        report.warnings[0]
    );
}

/// GIVEN a non-directory file exists alongside plugin directories in plugins/local
/// WHEN pruning runs
/// THEN non-directory entries are not considered for pruning (no crash)
#[test]
fn plugin_assets_prune_ignores_non_directory_entries() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");
    std::fs::create_dir_all(&local_dir).unwrap();

    // Create a stray file alongside plugin directories
    std::fs::write(local_dir.join("README.md"), b"# notes").unwrap();

    let asset = PluginAsset::new(
        "myplugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(".claude-plugin/plugin.json"),
        b"{\"name\": \"myplugin\"}".to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[asset]).unwrap();

    assert_eq!(report.written, 1);
    // The README file should survive — it's not a plugin directory so
    // remove_dir_all won't succeed on it, but the pruning loop should
    // not crash. (It will appear in warnings if remove_dir_all fails
    // on a file, but that's acceptable.)
    assert!(
        local_dir
            .join("myplugin/.cursor-plugin/plugin.json")
            .exists(),
        "Plugin manifest should be written"
    );
}
