//! Integration tests for the Cursor adapter.

use super::*;
use crate::adapters::traits::AgentAdapter;
use crate::adapters::utils::test_helpers::make_command;
use crate::common::{McpServer, McpTransport, ModuleFile};
use tempfile::TempDir;

/// Cursor refuses to mirror a plugin directory with no manifest, so every
/// asset-placement test ships the manifest its plugin would really carry.
fn plugin_manifest(plugin: &str) -> crate::common::PluginAsset {
    crate::common::PluginAsset::new(
        plugin.to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(".claude-plugin/plugin.json"),
        format!("{{\"name\": \"{plugin}\"}}").into_bytes(),
        false,
    )
}

/// Marks a `plugins/local` directory as one skrills mirrored, which is what
/// makes it eligible for pruning.
fn mark_as_mirrored(plugin_dir: &std::path::Path) {
    std::fs::create_dir_all(plugin_dir).unwrap();
    std::fs::write(plugin_dir.join(super::MIRROR_MARKER), b"marker").unwrap();
}

/// The items as Cursor sees them on disk, without the trailing frontmatter
/// stash the reader turns back into frontmatter.
fn as_written(mut items: Vec<Command>) -> Vec<Command> {
    for item in &mut items {
        let text = std::fs::read_to_string(&item.source_path).unwrap();
        let body = match super::utils::take_stash(&text) {
            Some((body, _)) => format!("{body}\n"),
            None => text,
        };
        item.content = body.into_bytes();
    }
    items
}

fn make_skill_with_frontmatter(name: &str) -> Command {
    let content = format!(
        "---\nname: {}\ndescription: A test skill\ncategory: testing\ntags:\n  - test\n---\n\n# {} Skill\n\nDo the thing.\n",
        name, name
    );
    make_command(name, &content)
}

/// The manifest is written under the sanitized plugin name while the skill
/// body used the raw one, so any plugin name the sanitizer rewrites put the
/// two in different directories and Cursor found a plugin with no skills.
#[test]
fn write_skills_puts_manifest_and_skill_in_the_same_plugin_dir() {
    let tmp = TempDir::new().unwrap();
    let mut skill = make_skill_with_frontmatter("deep-work");
    skill.plugin_origin = Some(crate::common::PluginOrigin {
        plugin_name: "my.plugin".to_string(),
        publisher: "market".to_string(),
        version: "1.0.0".to_string(),
    });

    super::skills::write_skills(tmp.path(), &[skill]).unwrap();

    let local = tmp.path().join("plugins/local");
    let manifest_dirs: Vec<_> = std::fs::read_dir(&local)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    assert_eq!(
        manifest_dirs.len(),
        1,
        "manifest and skill body should share one plugin directory, got {manifest_dirs:?}"
    );
    let plugin_dir = local.join(&manifest_dirs[0]);
    assert!(
        plugin_dir.join(".cursor-plugin/plugin.json").exists(),
        "manifest missing from {}",
        plugin_dir.display()
    );
    assert!(
        plugin_dir.join("skills/deep-work/SKILL.md").exists(),
        "skill body missing from {}",
        plugin_dir.display()
    );
}

/// A plugin name is upstream metadata, so it must not be able to steer a write
/// out of `plugins/local/`.
#[test]
fn write_skills_refuses_traversal_in_plugin_name() {
    let tmp = TempDir::new().unwrap();
    let mut skill = make_skill_with_frontmatter("deep-work");
    skill.plugin_origin = Some(crate::common::PluginOrigin {
        plugin_name: "../../escaped".to_string(),
        publisher: "market".to_string(),
        version: "1.0.0".to_string(),
    });

    super::skills::write_skills(tmp.path(), &[skill]).unwrap();

    // `plugins/local/../../escaped` resolves to `<root>/escaped`: inside the
    // temp dir, but outside the directory the adapter is allowed to write.
    assert!(
        !tmp.path().join("escaped").exists(),
        "plugin name must not escape plugins/local"
    );
    // A sanitizer that returned an empty string, or one that dropped the skill,
    // would also leave `<root>/escaped` absent, so name the destination.
    let plugin_dir = tmp.path().join("plugins/local/escaped");
    assert!(
        plugin_dir.join("skills/deep-work/SKILL.md").exists(),
        "the skill body belongs in {}",
        plugin_dir.display()
    );
    assert!(
        plugin_dir.join(".cursor-plugin/plugin.json").exists(),
        "the manifest belongs beside it in {}",
        plugin_dir.display()
    );
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

/// The frontmatter fields of a written Cursor skill, parsed as YAML, plus the
/// body that follows them.
fn cursor_skill_frontmatter(content: &str) -> (serde_norway::Mapping, String) {
    let (yaml, body, _) = skrills_validate::frontmatter::split_frontmatter(content);
    let yaml = yaml.unwrap_or_else(|| panic!("Cursor skill has no frontmatter:\n{content}"));
    let fields: serde_norway::Mapping = serde_norway::from_str(&yaml)
        .unwrap_or_else(|e| panic!("frontmatter is not valid YAML ({e}):\n{content}"));
    (fields, body.to_string())
}

fn yaml_str<'a>(fields: &'a serde_norway::Mapping, key: &str) -> Option<&'a str> {
    fields.get(key).and_then(|v| v.as_str())
}

/// Cursor loads a skill only when SKILL.md has `name` and `description`
/// frontmatter (cursor.com/docs/context/skills); 0.9.0 stripped it, so every
/// flat-synced skill was written in a form Cursor does not load.
#[test]
fn write_skills_keeps_name_and_description_frontmatter() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let skills = vec![make_skill_with_frontmatter("test-skill")];
    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    let read_back = as_written(adapter.read_skills().unwrap());
    assert_eq!(read_back.len(), 1);
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (fields, body) = cursor_skill_frontmatter(&content);
    assert_eq!(yaml_str(&fields, "name"), Some("test-skill"), "{content}");
    assert_eq!(
        yaml_str(&fields, "description"),
        Some("A test skill"),
        "{content}"
    );
    assert!(body.contains("# test-skill Skill"), "{content}");
    assert_eq!(content.matches("A test skill").count(), 1, "{content}");
}

/// Cursor requires `name` to equal the skill's folder name.
#[test]
fn write_skills_sets_name_to_the_folder_name() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: Deep Work Helper\ndescription: Focus\n---\n\n# Body\n";
    adapter
        .write_skills(&[make_command("Deep_Work", content)])
        .unwrap();

    let on_disk = std::fs::read_to_string(tmp.path().join("skills/deep-work/SKILL.md")).unwrap();
    let (fields, _) = cursor_skill_frontmatter(&on_disk);
    assert_eq!(yaml_str(&fields, "name"), Some("deep-work"), "{on_disk}");
}

#[test]
fn write_skills_keeps_disable_model_invocation() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content =
        "---\nname: manual\ndescription: Run by hand\ndisable-model-invocation: true\n---\n\n# Body\n";
    adapter
        .write_skills(&[make_command("manual", content)])
        .unwrap();

    let on_disk = std::fs::read_to_string(tmp.path().join("skills/manual/SKILL.md")).unwrap();
    let (fields, _) = cursor_skill_frontmatter(&on_disk);
    assert_eq!(
        fields
            .get("disable-model-invocation")
            .and_then(|v| v.as_bool()),
        Some(true),
        "{on_disk}"
    );
}

/// A description YAML would read as a flow sequence, alias or mapping must
/// still come back as the same string.
#[test]
fn write_skills_quotes_a_description_yaml_would_misread() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let description = "[draft] *Use when*: the \"plan\" is #1";
    let content = format!(
        "---\nname: tricky\ndescription: '{}'\n---\n\n# Body\n",
        description.replace('\'', "''")
    );
    adapter
        .write_skills(&[make_command("tricky", &content)])
        .unwrap();

    let on_disk = std::fs::read_to_string(tmp.path().join("skills/tricky/SKILL.md")).unwrap();
    let (fields, _) = cursor_skill_frontmatter(&on_disk);
    assert_eq!(
        yaml_str(&fields, "description"),
        Some(description),
        "{on_disk}"
    );
}

#[test]
fn write_skills_without_a_description_warns_cursor_will_not_load_it() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: bare\n---\n\n# Body\n";
    let report = adapter
        .write_skills(&[make_command("bare", content)])
        .unwrap();

    assert_eq!(report.written, 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("bare") && w.contains("description")),
        "{:?}",
        report.warnings
    );
    let on_disk = std::fs::read_to_string(tmp.path().join("skills/bare/SKILL.md")).unwrap();
    let (fields, _) = cursor_skill_frontmatter(&on_disk);
    assert_eq!(yaml_str(&fields, "name"), Some("bare"), "{on_disk}");
}

/// Cursor loads a skill only when `name` matches its folder and it has a
/// description, so a source with no frontmatter used to be written as a skill
/// Cursor silently ignores.
#[test]
fn write_skills_gives_a_skill_without_frontmatter_a_name_and_warns() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let report = adapter
        .write_skills(&[make_command("plain", "# Plain\nbody\n")])
        .unwrap();

    assert_eq!(report.written, 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("plain") && w.contains("description")),
        "{:?}",
        report.warnings
    );
    let on_disk = std::fs::read_to_string(tmp.path().join("skills/plain/SKILL.md")).unwrap();
    let (fields, body) = cursor_skill_frontmatter(&on_disk);
    assert_eq!(yaml_str(&fields, "name"), Some("plain"), "{on_disk}");
    assert!(body.contains("# Plain"), "{on_disk}");
}

#[test]
fn skills_model_hint_follows_the_frontmatter() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: paradigms\ndescription: Compare architecture patterns\nmodel_hint: standard\n---\n\n# Paradigms\n\nContent.\n";
    let skills = vec![make_command("paradigms", content)];

    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    let read_back = as_written(adapter.read_skills().unwrap());
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (fields, body) = cursor_skill_frontmatter(&content);
    assert_eq!(
        yaml_str(&fields, "description"),
        Some("Compare architecture patterns")
    );
    assert!(
        body.trim_start()
            .starts_with("<!-- model_hint: standard -->\n"),
        "Model hint should open the body, got: {content}"
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
    let read_back = as_written(adapter.read_skills().unwrap());
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (fields, _) = cursor_skill_frontmatter(&content);
    assert_eq!(
        yaml_str(&fields, "description"),
        Some("A quoted description"),
        "{content}"
    );
}

#[test]
fn skills_block_scalar_description() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: research\ndescription: >-\n  Search GitHub for implementations.\n  Use when the user wants code.\nversion: 1.0\n---\n\n# Body\n";
    let skills = vec![make_command("research", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = as_written(adapter.read_skills().unwrap());
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (fields, _) = cursor_skill_frontmatter(&content);
    assert_eq!(
        yaml_str(&fields, "description"),
        Some("Search GitHub for implementations. Use when the user wants code."),
        "{content}"
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
    let read_back = as_written(adapter.read_skills().unwrap());
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (fields, _) = cursor_skill_frontmatter(&content);
    assert_eq!(
        yaml_str(&fields, "description"),
        Some("A single-quoted desc"),
        "{content}"
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
    let read_back = as_written(adapter.read_skills().unwrap());
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (_, body) = cursor_skill_frontmatter(&content);
    assert!(
        !content.contains("model_hint"),
        "No model_hint comment when field is absent"
    );
    assert!(
        body.trim_start().starts_with("# Content"),
        "Body should follow the frontmatter, got: {content}"
    );
}

#[test]
fn skills_model_hint_only_no_description() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: hinted\nmodel_hint: fast\n---\n\n# Content\n\nBody here.\n";
    let skills = vec![make_command("hinted", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = as_written(adapter.read_skills().unwrap());
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (_, body) = cursor_skill_frontmatter(&content);
    assert!(
        body.trim_start().starts_with("<!-- model_hint: fast -->\n"),
        "Model hint should open the body when no description, got: {content}"
    );
}

#[test]
fn skills_multiline_quoted_description() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "---\nname: modular-monolith\ndescription: 'Single deployable with enforced module\n  boundaries for team autonomy.'\n---\n\n# Content\n";
    let skills = vec![make_command("modular-monolith", content)];

    adapter.write_skills(&skills).unwrap();
    let read_back = as_written(adapter.read_skills().unwrap());
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (fields, _) = cursor_skill_frontmatter(&content);
    assert_eq!(
        yaml_str(&fields, "description"),
        Some("Single deployable with enforced module boundaries for team autonomy."),
        "{content}"
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

    let read_back = as_written(adapter.read_agents().unwrap());
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

/// Cursor agents have no tool list, so a Claude agent limited to `Read` and
/// `Bash` became unrestricted without a word in the report. (One with no
/// write tool becomes `readonly` instead; see
/// `agent_without_write_tools_is_written_readonly`.)
#[test]
fn dropping_a_tool_restriction_is_reported() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let agents = vec![
        make_command(
            "reader",
            "---\nname: reader\ntools: Read, Bash\n---\nRead and run.\n",
        ),
        make_command("free", "---\nname: free\n---\nAnything.\n"),
    ];

    let report = adapter.write_agents(&agents).unwrap();

    assert_eq!(report.written, 2);
    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    assert!(report.warnings[0].contains("reader"));
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

/// SY-44: the module docs said CLAUDE.md maps to `claude-md.mdc`; the Claude
/// reader names it `CLAUDE`, which the kebab sanitiser turns into `claude`.
#[test]
fn the_claude_instruction_is_written_to_claude_mdc() {
    let tmp = TempDir::new().unwrap();
    let source = crate::adapters::ClaudeAdapter::with_root(tmp.path().join("claude"));
    std::fs::create_dir_all(tmp.path().join("claude")).unwrap();
    std::fs::write(
        tmp.path().join("claude/CLAUDE.md"),
        "# Rules\n\nBe brief.\n",
    )
    .unwrap();
    let instructions = source.read_instructions().unwrap();

    let adapter = CursorAdapter::with_root(tmp.path().join("cursor"));
    adapter.write_instructions(&instructions).unwrap();

    let rule = std::fs::read_to_string(tmp.path().join("cursor/rules/claude.mdc")).unwrap();
    assert!(rule.contains("alwaysApply: true"), "{rule}");
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

    let read_back = as_written(adapter.read_skills().unwrap());
    assert_eq!(read_back.len(), 1);
    let content = String::from_utf8_lossy(&read_back[0].content);
    let (fields, _) = cursor_skill_frontmatter(&content);
    assert_eq!(
        yaml_str(&fields, "description"),
        Some("Has no body"),
        "{content}"
    );
}

/// A skill without frontmatter is not in Cursor format (Cursor needs `name`),
/// so it gains a `name` line; its body is kept exactly.
#[test]
fn skills_no_frontmatter_keeps_the_body_verbatim() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let content = "# My Skill\n\nDo the thing.\n";
    let skills = vec![make_command("plain-skill", content)];

    let report = adapter.write_skills(&skills).unwrap();
    assert_eq!(report.written, 1);

    let read_back = adapter.read_skills().unwrap();
    assert_eq!(read_back.len(), 1);
    let text = String::from_utf8_lossy(&read_back[0].content);
    let (fields, body) = cursor_skill_frontmatter(&text);
    assert_eq!(yaml_str(&fields, "name"), Some("plain-skill"), "{text}");
    assert_eq!(
        body.trim_start_matches('\n'),
        content,
        "the body without frontmatter should be preserved exactly"
    );
}

#[test]
fn skills_module_path_traversal_blocked() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let mut skill = make_skill_with_frontmatter("safe-skill");
    // Use ../malicious.md, the parent (.../skills/) exists after skill dir is created,
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

    // Verify the names, currently both flatten to "foo-bar" which is a known
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

    let read_back = as_written(adapter.read_commands(false).unwrap());
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

    let read_back = as_written(adapter.read_skills().unwrap());
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
    // The traversal components are stripped, file lands inside plugins/local/
    assert!(
        !tmp.path().join("tmp/evil").exists(),
        "Path traversal must not escape plugins/local/"
    );
    // The sanitized name should be under plugins/local/
    let local_dir = tmp.path().join("plugins/local");
    assert!(local_dir.exists(), "plugins/local should exist");
}

/// Runtime files (scripts, binaries) land under the plugin directory so
/// skills that shell out to them resolve at their documented relative path.
#[test]
fn plugin_assets_writes_script_to_local_plugin_dir() {
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

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), script])
        .unwrap();

    assert_eq!(report.written, 2, "Manifest and script should be written");
    let expected = tmp.path().join("plugins/local/my-plugin/scripts/helper.py");
    assert_eq!(
        std::fs::read(&expected).unwrap(),
        b"# helper\n".to_vec(),
        "Script should exist with source content at {}",
        expected.display()
    );
}

/// Skill bodies reach Cursor through the plugin mirror. The orchestrator drops
/// the plugin skills from the flat `~/.cursor/skills` copy when the batch is a
/// full mirror, so this is the path that carries them then.
#[test]
fn plugin_assets_writes_skill_body_to_local_plugin_dir() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let skill = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("skills/deep-work/SKILL.md"),
        b"---\nname: deep-work\n---\nbody\n".to_vec(),
        false,
    );

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), skill])
        .unwrap();

    assert_eq!(report.written, 2, "Manifest and skill should be written");
    assert!(
        tmp.path()
            .join("plugins/local/my-plugin/skills/deep-work/SKILL.md")
            .exists(),
        "Skill body should exist under the plugin's skills directory"
    );
}

/// A second run with identical content must not rewrite files, otherwise every
/// sync churns thousands of mtimes.
#[test]
fn plugin_assets_skips_unchanged_asset() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let asset = || {
        PluginAsset::new(
            "my-plugin".to_string(),
            "market".to_string(),
            "1.0.0".to_string(),
            std::path::PathBuf::from("scripts/helper.py"),
            b"# helper\n".to_vec(),
            false,
        )
    };

    adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), asset()])
        .unwrap();
    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), asset()])
        .unwrap();

    assert_eq!(report.written, 0, "Unchanged asset should not be rewritten");
    assert!(
        report
            .skipped
            .iter()
            .any(|reason| matches!(reason, crate::report::SkipReason::Unchanged { .. })),
        "Unchanged asset should be reported as skipped, got: {:?}",
        report.skipped
    );
}

/// Cursor reads its manifest from `.cursor-plugin/`; the Claude-named
/// directory must not survive the copy or Cursor sees two manifests.
#[test]
fn plugin_assets_rewrites_claude_plugin_dir_to_cursor_plugin() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let manifest = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(".claude-plugin/plugin.json"),
        b"{\"name\": \"my-plugin\"}".to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[manifest]).unwrap();

    assert_eq!(report.written, 1, "Manifest should be written once");
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");
    assert!(
        plugin_dir.join(".cursor-plugin/plugin.json").exists(),
        "Manifest should land in .cursor-plugin/"
    );
    assert!(
        !plugin_dir.join(".claude-plugin").exists(),
        "Claude-named manifest directory should not be copied verbatim"
    );
}

/// A relative path escaping the plugin directory is refused, not written
/// somewhere else under the user's home.
#[test]
fn plugin_assets_relative_path_traversal_refused() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let escaping = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("../../evil.sh"),
        b"rm -rf /\n".to_vec(),
        false,
    );

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), escaping])
        .unwrap();

    assert_eq!(report.written, 1, "Only the manifest should be written");
    assert!(
        !tmp.path().join("evil.sh").exists() && !tmp.path().join("plugins/evil.sh").exists(),
        "Escaping asset must not land outside the plugin directory"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("evil.sh") && w.contains("my-plugin")),
        "The refusal must be reported, not silently skipped: {:?}",
        report.warnings
    );
}

/// Executable assets stay executable, otherwise skills that invoke them
/// fail with a permission error on Cursor's side.
#[cfg(unix)]
#[test]
fn plugin_assets_preserve_executable_bit() {
    use crate::common::PluginAsset;
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let binary = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("bin/runner"),
        b"#!/bin/sh\n".to_vec(),
        true,
    );

    adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), binary])
        .unwrap();

    let mode = std::fs::metadata(tmp.path().join("plugins/local/my-plugin/bin/runner"))
        .unwrap()
        .permissions()
        .mode();
    assert!(
        mode & 0o111 != 0,
        "Executable asset should keep its executable bit, mode was {:o}",
        mode
    );
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

    // Pre-create a stale plugin from a prior sync, marked as skrills-managed
    let stale = local_dir.join("old-plugin/.cursor-plugin");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("plugin.json"), b"{\"name\": \"old-plugin\"}").unwrap();
    mark_as_mirrored(&local_dir.join("old-plugin"));

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

    // Also create a stale plugin that a previous sync mirrored
    let stale = local_dir.join("removed-plugin/.cursor-plugin");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("plugin.json"), b"{\"name\": \"removed\"}").unwrap();
    mark_as_mirrored(&local_dir.join("removed-plugin"));

    // Sync same content, should be skipped (unchanged) but stale should be pruned
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
    mark_as_mirrored(&local_dir.join("stale-a"));
    mark_as_mirrored(&local_dir.join("stale-b"));

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

    // Pre-create a stale plugin that a previous sync mirrored
    let stale = local_dir.join("old-plugin/.cursor-plugin");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("plugin.json"), b"{}").unwrap();
    mark_as_mirrored(&local_dir.join("old-plugin"));

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
    // The README file should survive, it's not a plugin directory so
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

// --- Plugin mirror hardening ---

/// A plugin upgrade can turn `scripts/helper` from a file into a directory.
/// `create_dir_all` then fails, and the abort used to discard the report for
/// everything already written and to fail again on every later sync.
#[test]
fn plugin_assets_replaces_a_file_where_a_directory_is_now_needed() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");
    // A mirror an earlier run wrote, so skrills owns what is in the way.
    mark_as_mirrored(&plugin_dir);
    std::fs::create_dir_all(plugin_dir.join("scripts")).unwrap();
    std::fs::write(plugin_dir.join("scripts/helper"), b"v1 was a file\n").unwrap();

    let upgraded = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "2.0.0".to_string(),
        std::path::PathBuf::from("scripts/helper/main.py"),
        b"print('v2')\n".to_vec(),
        false,
    );

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), upgraded])
        .unwrap();

    assert_eq!(report.written, 2, "warnings were {:?}", report.warnings);
    assert_eq!(
        std::fs::read(plugin_dir.join("scripts/helper/main.py")).unwrap(),
        b"print('v2')\n".to_vec()
    );
}

/// The mirror image: a directory sits where the new version ships a file.
#[test]
fn plugin_assets_replaces_a_directory_where_a_file_is_now_needed() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");
    // A mirror an earlier run wrote, so skrills owns what is in the way.
    mark_as_mirrored(&plugin_dir);
    std::fs::create_dir_all(plugin_dir.join("scripts/helper")).unwrap();
    std::fs::write(plugin_dir.join("scripts/helper/main.py"), b"v1\n").unwrap();

    let downgraded = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "2.0.0".to_string(),
        std::path::PathBuf::from("scripts/helper"),
        b"v2 is a file\n".to_vec(),
        false,
    );

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), downgraded])
        .unwrap();

    assert_eq!(report.written, 2, "warnings were {:?}", report.warnings);
    assert_eq!(
        std::fs::read(plugin_dir.join("scripts/helper")).unwrap(),
        b"v2 is a file\n".to_vec()
    );
}

/// `plugins/local/<name>` existing as a plain file is the user's doing, so the
/// writer says so and mirrors the rest of the batch instead of failing.
#[test]
fn plugin_assets_warns_instead_of_failing_when_the_plugin_dir_is_a_file() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");
    std::fs::create_dir_all(&local_dir).unwrap();
    std::fs::write(local_dir.join("blocked"), b"not a directory\n").unwrap();

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("blocked"), plugin_manifest("fine")])
        .unwrap();

    assert_eq!(report.written, 1, "the other plugin should still mirror");
    assert!(local_dir.join("fine/.cursor-plugin/plugin.json").exists());
    assert_eq!(
        std::fs::read(local_dir.join("blocked")).unwrap(),
        b"not a directory\n".to_vec(),
        "the user's file must be left alone"
    );
    assert!(
        report.warnings.iter().any(|w| w.contains("blocked")),
        "the refusal must be reported: {:?}",
        report.warnings
    );
}

/// The writer only ever added and overwrote, so a plugin upgrade that dropped a
/// skill left Cursor loading a skill that no longer exists upstream.
#[test]
fn plugin_assets_removes_files_the_plugin_no_longer_ships() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");

    let old_skill = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("skills/retired/SKILL.md"),
        b"# Retired\n".to_vec(),
        false,
    );
    adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), old_skill])
        .unwrap();
    assert!(plugin_dir.join("skills/retired/SKILL.md").exists());

    let new_skill = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "2.0.0".to_string(),
        std::path::PathBuf::from("skills/current/SKILL.md"),
        b"# Current\n".to_vec(),
        false,
    );
    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), new_skill])
        .unwrap();

    assert!(plugin_dir.join("skills/current/SKILL.md").exists());
    assert!(
        !plugin_dir.join("skills/retired").exists(),
        "the dropped skill should be gone, warnings were {:?}",
        report.warnings
    );
    assert!(
        plugin_dir.join(".cursor-plugin/plugin.json").exists(),
        "the manifest this run skipped as unchanged must survive the sweep"
    );
}

fn plugin_skill(plugin: &str, skill: &str) -> crate::common::PluginAsset {
    crate::common::PluginAsset::new(
        plugin.to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(format!("skills/{skill}/SKILL.md")),
        format!("---\nname: {skill}\ndescription: d\n---\n# {skill}\n").into_bytes(),
        false,
    )
}

/// M21: a skill directory inside a mirrored plugin carries the same marker as
/// a flat skill copy, so cleanup can tell it from a skill the user made.
#[test]
fn plugin_assets_mark_each_mirrored_skill_directory() {
    use crate::adapters::utils::SYNCED_MARKER;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");

    let report = adapter
        .write_plugin_assets(&[
            plugin_manifest("my-plugin"),
            plugin_skill("my-plugin", "alpha"),
        ])
        .unwrap();

    assert!(plugin_dir
        .join("skills/alpha")
        .join(SYNCED_MARKER)
        .is_file());
    assert!(
        !plugin_dir
            .join(".cursor-plugin")
            .join(SYNCED_MARKER)
            .exists(),
        "only skill directories are marked"
    );
    assert_eq!(report.written, 2, "the marker is not counted as an asset");
}

#[test]
fn plugin_assets_remirror_keeps_the_skill_marker_and_reports_unchanged() {
    use crate::adapters::utils::SYNCED_MARKER;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");
    let batch = [
        plugin_manifest("my-plugin"),
        plugin_skill("my-plugin", "alpha"),
    ];

    adapter.write_plugin_assets(&batch).unwrap();
    let report = adapter.write_plugin_assets(&batch).unwrap();

    assert!(plugin_dir
        .join("skills/alpha")
        .join(SYNCED_MARKER)
        .is_file());
    assert_eq!(report.written, 0);
    assert_eq!(report.skipped.len(), 2, "{:?}", report.skipped);
    assert!(
        !report.warnings.iter().any(|w| w.contains("Removed")),
        "the marker was swept: {:?}",
        report.warnings
    );

    let preview = adapter.preview_plugin_assets(&batch).unwrap();
    assert_eq!(preview.written, 2, "a preview counts assets, not markers");
}

#[test]
fn plugin_assets_remove_a_dropped_skill_directory_with_its_marker() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");

    adapter
        .write_plugin_assets(&[
            plugin_manifest("my-plugin"),
            plugin_skill("my-plugin", "alpha"),
            plugin_skill("my-plugin", "beta"),
        ])
        .unwrap();
    let report = adapter
        .write_plugin_assets(&[
            plugin_manifest("my-plugin"),
            plugin_skill("my-plugin", "alpha"),
        ])
        .unwrap();

    assert!(
        !plugin_dir.join("skills/beta").exists(),
        "{:?}",
        report.warnings
    );
    assert!(plugin_dir.join("skills/alpha/SKILL.md").is_file());
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.starts_with("Removed 1 file(s)")),
        "only the dropped SKILL.md counts as removed: {:?}",
        report.warnings
    );
}

/// A partial batch (the `sync-skills` tool sends one) does not describe the
/// whole plugin, so it must not read an absent file as "upstream dropped it".
#[test]
fn plugin_assets_does_not_sweep_when_the_batch_carries_no_manifest() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");

    // A manifest already on disk, as `write_skills` leaves one.
    std::fs::create_dir_all(plugin_dir.join(".cursor-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join(".cursor-plugin/plugin.json"),
        b"{\"name\": \"my-plugin\"}",
    )
    .unwrap();
    std::fs::create_dir_all(plugin_dir.join("skills/kept")).unwrap();
    std::fs::write(plugin_dir.join("skills/kept/SKILL.md"), b"# Kept\n").unwrap();

    let script = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("scripts/helper.py"),
        b"# helper\n".to_vec(),
        false,
    );
    let report = adapter.write_plugin_assets(&[script]).unwrap();

    assert_eq!(report.written, 1, "warnings were {:?}", report.warnings);
    assert!(
        plugin_dir.join("skills/kept/SKILL.md").exists(),
        "a supplementary batch must not sweep what it does not describe"
    );
}

/// `sanitize_name` strips dots and slashes, so `my.plugin` and `myplugin` both
/// resolve to `plugins/local/myplugin`. Merging their trees makes the on-disk
/// winner flip every run, so the batch is refused instead.
#[test]
fn plugin_assets_refuses_two_plugins_that_map_to_one_directory() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let report = adapter
        .write_plugin_assets(&[
            plugin_manifest("my.plugin"),
            plugin_manifest("myplugin"),
            plugin_manifest("unaffected"),
        ])
        .unwrap();

    assert_eq!(report.written, 1, "only the unambiguous plugin is mirrored");
    assert!(!tmp.path().join("plugins/local/myplugin").exists());
    assert!(tmp.path().join("plugins/local/unaffected").exists());
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("my.plugin") && w.contains("myplugin")),
        "the refusal must name both plugins: {:?}",
        report.warnings
    );
}

/// The content hash covers bytes only, so a file that stopped being executable
/// upstream used to keep `0o755` forever.
#[cfg(unix)]
#[test]
fn plugin_assets_clears_the_exec_bit_when_upstream_dropped_it() {
    use crate::common::PluginAsset;
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let dest = tmp.path().join("plugins/local/my-plugin/bin/runner");

    let asset = |executable| {
        PluginAsset::new(
            "my-plugin".to_string(),
            "market".to_string(),
            "1.0.0".to_string(),
            std::path::PathBuf::from("bin/runner"),
            b"#!/bin/sh\n".to_vec(),
            executable,
        )
    };

    adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), asset(true)])
        .unwrap();
    assert!(
        std::fs::metadata(&dest).unwrap().permissions().mode() & 0o111 != 0,
        "the first write should set the bit"
    );

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), asset(false)])
        .unwrap();

    let mode = std::fs::metadata(&dest).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o111,
        0,
        "the bit should be cleared although the bytes did not change, mode was {mode:o}, \
         report {report:?}"
    );
}

/// The directory used to be created before the containment check, so a batch
/// whose assets were all refused still left an empty directory behind, and the
/// prune loop then treated it as an installed plugin.
#[test]
fn plugin_assets_creates_no_directory_for_a_fully_refused_plugin() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let escaping = PluginAsset::new(
        "no-manifest".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("scripts/helper.py"),
        b"# helper\n".to_vec(),
        false,
    );

    let report = adapter.write_plugin_assets(&[escaping]).unwrap();

    assert_eq!(report.written, 0);
    assert!(
        !tmp.path().join("plugins/local/no-manifest").exists(),
        "a refused plugin must not leave a directory behind"
    );
}

/// Containment is resolved against the canonicalized plugin directory, so a
/// symlink planted at `plugins/local/<p>` would redirect the mirror out of
/// `~/.cursor`.
#[cfg(unix)]
#[test]
fn plugin_assets_refuses_a_symlinked_plugin_directory() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");
    std::fs::create_dir_all(&local_dir).unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, local_dir.join("hijacked")).unwrap();

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("hijacked")])
        .unwrap();

    assert_eq!(report.written, 0);
    assert!(
        !outside.join(".cursor-plugin").exists(),
        "the write must not follow the symlink"
    );
    assert!(
        report.warnings.iter().any(|w| w.contains("symlink")),
        "the refusal must be reported: {:?}",
        report.warnings
    );
}

/// A name made only of characters the sanitizer strips would write straight
/// into `plugins/local/` itself.
#[test]
fn plugin_assets_skips_a_plugin_name_that_sanitizes_to_nothing() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("...")])
        .unwrap();

    assert_eq!(report.written, 0);
    assert!(
        !tmp.path().join("plugins/local/.cursor-plugin").exists(),
        "nothing should be written into plugins/local itself"
    );
    assert!(
        report.warnings.iter().any(|w| w.contains("empty")),
        "the refusal must be reported: {:?}",
        report.warnings
    );
}

/// `plugins/local/` is also where a Cursor user puts hand-made plugins, and a
/// mirror written before the marker existed must not be deleted either.
#[test]
fn plugin_assets_never_prunes_a_directory_without_the_mirror_marker() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");

    let handmade = local_dir.join("hand-made/.cursor-plugin");
    std::fs::create_dir_all(&handmade).unwrap();
    std::fs::write(handmade.join("plugin.json"), b"{\"name\": \"hand-made\"}").unwrap();

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("mirrored")])
        .unwrap();

    assert!(
        handmade.join("plugin.json").exists(),
        "an unmarked directory must survive, warnings were {:?}",
        report.warnings
    );
}

/// The marker is what makes a mirror prunable later, so every plugin this run
/// mirrored has to carry one.
#[test]
fn plugin_assets_marks_every_mirrored_plugin_so_a_later_run_can_prune_it() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");

    adapter
        .write_plugin_assets(&[plugin_manifest("first"), plugin_manifest("second")])
        .unwrap();
    assert!(local_dir.join("first").join(super::MIRROR_MARKER).exists());
    assert!(local_dir.join("second").join(super::MIRROR_MARKER).exists());

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("first")])
        .unwrap();

    assert!(
        !local_dir.join("second").exists(),
        "a marked mirror that left the batch should be pruned, warnings were {:?}",
        report.warnings
    );
    assert!(local_dir.join("first/.cursor-plugin/plugin.json").exists());
}

/// A dry run has to name what would be deleted before it happens, and delete
/// nothing itself.
#[test]
fn preview_plugin_assets_lists_prune_targets_without_touching_disk() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");

    adapter
        .write_plugin_assets(&[plugin_manifest("stays"), plugin_manifest("goes")])
        .unwrap();

    let report = adapter
        .preview_plugin_assets(&[plugin_manifest("stays")])
        .unwrap();

    assert_eq!(report.written, 1, "the batch holds one asset");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("Would prune") && w.contains("goes")),
        "the preview must name the prune target: {:?}",
        report.warnings
    );
    assert!(
        local_dir.join("goes/.cursor-plugin/plugin.json").exists(),
        "a preview must not delete anything"
    );
}

/// A hand-made Cursor plugin can share its directory name with a Claude plugin.
/// The mirror then writes into a directory skrills never marked, and the sweep
/// used to delete every file there that the Claude plugin does not ship, which
/// is all of the user's own work.
#[test]
fn plugin_assets_never_sweeps_files_from_an_unmarked_directory() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/shared-name");

    std::fs::create_dir_all(plugin_dir.join("skills/mine")).unwrap();
    std::fs::write(plugin_dir.join("skills/mine/SKILL.md"), b"# Mine\n").unwrap();

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("shared-name")])
        .unwrap();

    assert!(
        plugin_dir.join("skills/mine/SKILL.md").exists(),
        "a file in an unmarked directory must survive, warnings were {:?}",
        report.warnings
    );
    assert!(
        !plugin_dir.join(super::MIRROR_MARKER).exists(),
        "a directory holding files the plugin does not ship must not be adopted, \
         or the next run would sweep them"
    );
    assert!(
        report.warnings.iter().any(|w| w.contains("shared-name")),
        "the user must be told the directory was left unmanaged: {:?}",
        report.warnings
    );
}

/// A mirror written before the marker existed holds exactly what the plugin
/// ships, so it is adopted on its first complete mirror and prunable after.
#[test]
fn plugin_assets_adopts_an_unmarked_mirror_holding_only_plugin_files() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/legacy");

    std::fs::create_dir_all(plugin_dir.join(".cursor-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join(".cursor-plugin/plugin.json"),
        b"{\"name\": \"old\"}",
    )
    .unwrap();

    adapter
        .write_plugin_assets(&[plugin_manifest("legacy")])
        .unwrap();

    assert!(plugin_dir.join(super::MIRROR_MARKER).exists());
}

/// SY-25: a Claude skill written to Cursor loses its frontmatter there, so a
/// sync back out of Cursor (to Codex, say) used to produce a SKILL.md with no
/// `name:`/`description:`, which Codex does not load. The frontmatter now
/// rides along in a trailing comment and the Cursor reader puts it back.
#[test]
fn skill_frontmatter_survives_a_trip_through_cursor() {
    let tmp = TempDir::new().unwrap();
    let mut skill = make_skill_with_frontmatter("deep-work");
    skill.content = b"---\nname: deep-work\ndescription: A test skill\nmodel_hint: deep\ncategory: testing\n---\n\n# Deep Skill\n\nDo the thing.\n".to_vec();

    super::skills::write_skills(tmp.path(), std::slice::from_ref(&skill)).unwrap();
    let back = super::skills::read_skills(tmp.path()).unwrap();
    let back = String::from_utf8(back[0].content.clone()).unwrap();

    assert!(
        back.starts_with("---\nname: deep-work\ndescription: A test skill\nmodel_hint: deep\ncategory: testing\n---\n"),
        "frontmatter not restored:\n{back}"
    );
    assert!(back.contains("# Deep Skill\n\nDo the thing."), "{back}");
    assert!(!back.contains("skrills:frontmatter"), "{back}");
    assert!(!back.contains("model_hint: deep -->"), "{back}");
    assert_eq!(back.matches("A test skill").count(), 1, "{back}");

    // Writing the restored copy back to Cursor changes nothing.
    let restored = make_command("deep-work", &back);
    let again = super::skills::write_skills(tmp.path(), &[restored]).unwrap();
    assert_eq!(again.written, 0, "{again:?}");
}

/// A Cursor skill the user wrote by hand carries no stash and reads back as is.
#[test]
fn a_native_cursor_skill_reads_back_unchanged() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("skills/native");
    std::fs::create_dir_all(&dir).unwrap();
    let text = "# Native\n\nBody <!-- note -->\n";
    std::fs::write(dir.join("SKILL.md"), text).unwrap();

    let back = super::skills::read_skills(tmp.path()).unwrap();
    assert_eq!(back[0].content, text.as_bytes());
}

#[test]
fn command_frontmatter_survives_a_trip_through_cursor() {
    let tmp = TempDir::new().unwrap();
    let original = "---\ndescription: Review code\nallowed-tools: Read, Grep\n---\n\n# Review\n\nLook closely.\n";
    super::commands::write_commands(tmp.path(), &[make_command("review", original)]).unwrap();

    let on_disk = std::fs::read_to_string(tmp.path().join("commands/review.md")).unwrap();
    assert!(
        on_disk.starts_with("# Review"),
        "Cursor copy must not start with frontmatter:\n{on_disk}"
    );

    let back = super::commands::read_commands(tmp.path()).unwrap();
    let back = String::from_utf8(back[0].content.clone()).unwrap();
    assert_eq!(back, original);

    let again =
        super::commands::write_commands(tmp.path(), &[make_command("review", &back)]).unwrap();
    assert_eq!(again.written, 0);
}

/// `tools:` and `isolation:` have no Cursor field; they are kept aside so a
/// sync back restores the restriction, and dropping `isolation` is reported.
#[test]
fn agent_tool_restriction_survives_a_trip_through_cursor() {
    let tmp = TempDir::new().unwrap();
    let original = "---\nname: reader\ntools:\n  - Read\nisolation: worktree\nmodel: opus\n---\n\nRead only.\n";
    let report =
        super::agents::write_agents(tmp.path(), &[make_command("reader", original)]).unwrap();
    assert!(
        report.warnings.iter().any(|w| w.contains("isolation")),
        "{:?}",
        report.warnings
    );

    let back = super::agents::read_agents(tmp.path()).unwrap();
    let back = String::from_utf8(back[0].content.clone()).unwrap();
    assert!(back.contains("tools:\n  - Read\n"), "{back}");
    assert!(back.contains("isolation: worktree\n"), "{back}");
    assert!(back.contains("model: opus\n"), "{back}");
    assert!(back.trim_end().ends_with("Read only."), "{back}");
    assert!(!back.contains("skrills:frontmatter"), "{back}");

    let again = super::agents::write_agents(tmp.path(), &[make_command("reader", &back)]).unwrap();
    assert_eq!(again.written, 0, "{again:?}");
}

/// Invalid UTF-8 used to be decoded lossily, writing U+FFFD into the copy.
#[test]
fn invalid_utf8_is_skipped_not_mangled() {
    let tmp = TempDir::new().unwrap();
    let mut bad = make_command("bad", "x");
    bad.content = vec![b'#', b' ', 0xff, 0xfe, b'\n'];

    let skills = super::skills::write_skills(tmp.path(), std::slice::from_ref(&bad)).unwrap();
    let commands = super::commands::write_commands(tmp.path(), std::slice::from_ref(&bad)).unwrap();
    let agents = super::agents::write_agents(tmp.path(), std::slice::from_ref(&bad)).unwrap();

    for report in [&skills, &commands, &agents] {
        assert_eq!(report.written, 0);
        assert!(
            matches!(
                report.skipped.as_slice(),
                [crate::report::SkipReason::ParseError { .. }]
            ),
            "{:?}",
            report.skipped
        );
    }
}

// --- Plugin hooks/hooks.json ---

/// A plugin's `hooks/hooks.json` asset with the given body.
fn plugin_hooks_asset(plugin: &str, body: &str) -> crate::common::PluginAsset {
    crate::common::PluginAsset::new(
        plugin.to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("hooks/hooks.json"),
        body.as_bytes().to_vec(),
        false,
    )
}

/// Mirrors `body` as the plugin's hooks file and returns what Cursor gets.
fn mirrored_plugin_hooks(tmp: &TempDir, body: &str) -> (serde_json::Value, WriteReport) {
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let report = adapter
        .write_plugin_assets(&[
            plugin_manifest("hooked"),
            plugin_hooks_asset("hooked", body),
        ])
        .unwrap();
    let path = tmp.path().join("plugins/local/hooked/hooks/hooks.json");
    let on_disk = std::fs::read_to_string(path).unwrap();
    let json = serde_json::from_str(&on_disk)
        .unwrap_or_else(|e| panic!("mirrored hooks.json is not JSON ({e}):\n{on_disk}"));
    (json, report)
}

const CLAUDE_PLUGIN_HOOKS: &str = r#"{
  "hooks": {
    "PostToolUse": [
      { "matcher": "Skill", "hooks": [
        { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/hooks/watch.py", "timeout": 2 }
      ] }
    ],
    "SessionStart": [
      { "hooks": [ { "type": "command", "command": "$CLAUDE_PLUGIN_ROOT/hooks/start.sh" } ] }
    ]
  }
}"#;

/// Cursor plugin hooks are flat camelCase entries
/// (cursor.com/docs/reference/plugins); the Claude nested PascalCase file was
/// copied verbatim, so no plugin hook ever ran in Cursor.
#[test]
fn plugin_hooks_json_is_translated_to_cursor_events() {
    let tmp = TempDir::new().unwrap();
    let (hooks, _) = mirrored_plugin_hooks(&tmp, CLAUDE_PLUGIN_HOOKS);

    assert_eq!(hooks["version"], 1, "{hooks:#}");
    let post = hooks["hooks"]["postToolUse"].as_array().unwrap();
    assert_eq!(post.len(), 1, "{hooks:#}");
    assert_eq!(post[0]["matcher"], "Skill", "{hooks:#}");
    assert_eq!(post[0]["type"], "command", "{hooks:#}");
    assert_eq!(post[0]["timeout"], 2, "{hooks:#}");
    assert!(post[0].get("hooks").is_none(), "{hooks:#}");
    assert!(hooks["hooks"]["sessionStart"].is_array(), "{hooks:#}");
    assert!(hooks["hooks"].get("PostToolUse").is_none(), "{hooks:#}");
}

/// Cursor documents plugin-root expansion only for `mcp.json`, so hook
/// commands get the mirror's absolute path.
#[test]
fn plugin_hooks_json_points_commands_at_the_mirrored_plugin() {
    let tmp = TempDir::new().unwrap();
    let (hooks, _) = mirrored_plugin_hooks(&tmp, CLAUDE_PLUGIN_HOOKS);

    let root = tmp.path().join("plugins/local/hooked");
    assert_eq!(
        hooks["hooks"]["postToolUse"][0]["command"],
        format!("{}/hooks/watch.py", root.display()),
        "{hooks:#}"
    );
    assert_eq!(
        hooks["hooks"]["sessionStart"][0]["command"],
        format!("{}/hooks/start.sh", root.display()),
        "{hooks:#}"
    );
}

#[test]
fn plugin_hooks_json_drops_events_cursor_lacks() {
    let tmp = TempDir::new().unwrap();
    let body = r#"{ "hooks": {
        "Notification": [ { "hooks": [ { "type": "command", "command": "notify" } ] } ],
        "Stop": [ { "hooks": [ { "type": "command", "command": "done" } ] } ]
    } }"#;
    let (hooks, report) = mirrored_plugin_hooks(&tmp, body);

    assert!(hooks["hooks"].get("Notification").is_none(), "{hooks:#}");
    assert!(hooks["hooks"].get("notification").is_none(), "{hooks:#}");
    assert_eq!(hooks["hooks"]["stop"][0]["command"], "done", "{hooks:#}");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("hooked") && w.contains("Notification")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn plugin_hooks_json_already_in_cursor_form_is_kept() {
    let tmp = TempDir::new().unwrap();
    let body = r#"{ "version": 1, "hooks": {
        "afterFileEdit": [ { "command": "./scripts/format.sh", "matcher": "Write" } ]
    } }"#;
    let (hooks, report) = mirrored_plugin_hooks(&tmp, body);

    assert_eq!(
        hooks["hooks"]["afterFileEdit"],
        serde_json::json!([{ "command": "./scripts/format.sh", "matcher": "Write" }]),
        "{hooks:#}"
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn plugin_hooks_json_that_does_not_parse_is_copied_unchanged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let body = "{ \"hooks\": { ,,, } }";
    let report = adapter
        .write_plugin_assets(&[
            plugin_manifest("hooked"),
            plugin_hooks_asset("hooked", body),
        ])
        .unwrap();

    let on_disk =
        std::fs::read_to_string(tmp.path().join("plugins/local/hooked/hooks/hooks.json")).unwrap();
    assert_eq!(on_disk, body);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("hooked") && w.contains("hooks.json")),
        "{:?}",
        report.warnings
    );
}

/// The unchanged check must compare against the translated file, or every
/// sync rewrites it.
#[test]
fn plugin_hooks_json_is_unchanged_on_a_second_sync() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let assets = [
        plugin_manifest("hooked"),
        plugin_hooks_asset("hooked", CLAUDE_PLUGIN_HOOKS),
    ];
    adapter.write_plugin_assets(&assets).unwrap();

    let again = adapter.write_plugin_assets(&assets).unwrap();
    assert_eq!(again.written, 0, "{again:?}");
}

// --- Agent tool restrictions ---

fn written_agent(tmp: &TempDir, name: &str, frontmatter: &str) -> (String, WriteReport) {
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let content = format!("---\nname: {name}\n{frontmatter}---\n\nDo the work.\n");
    let report = adapter
        .write_agents(&[make_command(name, &content)])
        .unwrap();
    let on_disk = std::fs::read_to_string(tmp.path().join(format!("agents/{name}.md"))).unwrap();
    (on_disk, report)
}

/// Cursor agents have no tool list; `readonly` is the only restriction
/// (cursor.com/docs/context/subagents). An agent Claude kept from writing got
/// full write access in Cursor.
#[test]
fn agent_without_write_tools_is_written_readonly() {
    let tmp = TempDir::new().unwrap();
    let (on_disk, report) = written_agent(&tmp, "auditor", "tools: Read, Grep, Glob, WebFetch\n");

    let (fields, _) = cursor_skill_frontmatter(&on_disk);
    assert_eq!(
        fields.get("readonly").and_then(|v| v.as_bool()),
        Some(true),
        "{on_disk}"
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn agent_with_a_block_list_of_read_tools_is_written_readonly() {
    let tmp = TempDir::new().unwrap();
    let (on_disk, _) = written_agent(&tmp, "reader", "tools:\n  - Read\n  - Grep\n");

    let (fields, _) = cursor_skill_frontmatter(&on_disk);
    assert_eq!(
        fields.get("readonly").and_then(|v| v.as_bool()),
        Some(true),
        "{on_disk}"
    );
}

#[test]
fn agent_with_a_write_tool_is_not_readonly() {
    let tmp = TempDir::new().unwrap();
    let (on_disk, report) = written_agent(&tmp, "committer", "tools: [Read, Bash(git:*)]\n");

    let (fields, _) = cursor_skill_frontmatter(&on_disk);
    assert!(fields.get("readonly").is_none(), "{on_disk}");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("committer") && w.contains("tool")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn agent_without_a_tools_list_is_not_readonly() {
    let tmp = TempDir::new().unwrap();
    let (on_disk, report) = written_agent(&tmp, "generalist", "model: opus\n");

    let (fields, _) = cursor_skill_frontmatter(&on_disk);
    assert!(fields.get("readonly").is_none(), "{on_disk}");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

/// The `readonly` added for Cursor stands in for `tools`; a sync back to
/// Claude restores `tools` and must not also carry `readonly`.
#[test]
fn agent_readonly_added_for_cursor_does_not_reach_claude() {
    let tmp = TempDir::new().unwrap();
    written_agent(&tmp, "auditor", "tools: Read, Grep\n");

    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let back = adapter.read_agents().unwrap();
    let back = String::from_utf8(back[0].content.clone()).unwrap();
    assert!(back.contains("tools: Read, Grep"), "{back}");
    assert!(!back.contains("readonly"), "{back}");
}

/// A plugin refused this run (here: a name collision) is still installed, so
/// its existing mirror must survive the prune that the rest of the batch runs.
#[test]
fn plugin_assets_does_not_prune_the_mirror_of_a_plugin_refused_this_run() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let local_dir = tmp.path().join("plugins/local");
    mark_as_mirrored(&local_dir.join("myplugin"));
    std::fs::write(local_dir.join("myplugin/keep.txt"), b"keep").unwrap();

    let batch = [
        plugin_manifest("my.plugin"),
        plugin_manifest("myplugin"),
        plugin_manifest("unaffected"),
    ];
    let preview = adapter.preview_plugin_assets(&batch).unwrap();
    assert!(
        !preview.warnings.iter().any(|w| w.contains("prune")),
        "a dry run must not announce pruning a refused plugin: {:?}",
        preview.warnings
    );

    let report = adapter.write_plugin_assets(&batch).unwrap();
    assert!(
        local_dir.join("myplugin/keep.txt").exists(),
        "the refused plugin's mirror was pruned, warnings were {:?}",
        report.warnings
    );
}

/// An unmarked directory holding the user's own files is not ours. The
/// ownership check used to run after the writes, so the user's file at a path
/// the plugin also ships was already overwritten by then.
#[test]
fn plugin_assets_writes_nothing_into_an_unmarked_directory_holding_user_files() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/mine");
    std::fs::create_dir_all(plugin_dir.join("scripts")).unwrap();
    std::fs::write(plugin_dir.join("scripts/helper.py"), b"# mine\n").unwrap();
    std::fs::write(plugin_dir.join("notes.md"), b"my notes\n").unwrap();

    let helper = PluginAsset::new(
        "mine".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("scripts/helper.py"),
        b"# upstream\n".to_vec(),
        false,
    );
    let report = adapter
        .write_plugin_assets(&[plugin_manifest("mine"), helper])
        .unwrap();

    assert_eq!(
        std::fs::read(plugin_dir.join("scripts/helper.py")).unwrap(),
        b"# mine\n",
        "the user's file was overwritten, warnings were {:?}",
        report.warnings
    );
    assert!(!plugin_dir.join(".cursor-plugin").exists());
    assert!(!plugin_dir.join(super::MIRROR_MARKER).exists());
    assert_eq!(report.written, 0);
}

/// Clearing a conflicting path deletes whatever sits there. In a directory
/// skrills does not own, that is the user's data.
#[test]
fn plugin_assets_never_removes_a_directory_inside_an_unmarked_plugin_dir() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/mine");
    std::fs::create_dir_all(plugin_dir.join(".cursor-plugin")).unwrap();
    std::fs::write(plugin_dir.join(".cursor-plugin/plugin.json"), b"{}").unwrap();
    std::fs::create_dir_all(plugin_dir.join("scripts/helper")).unwrap();
    std::fs::write(plugin_dir.join("scripts/helper/data.txt"), b"mine\n").unwrap();

    let helper = PluginAsset::new(
        "mine".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("scripts/helper"),
        b"#!/bin/sh\n".to_vec(),
        false,
    );
    let report = adapter.write_plugin_assets(&[helper]).unwrap();

    assert!(
        plugin_dir.join("scripts/helper/data.txt").exists(),
        "the user's directory was removed, warnings were {:?}",
        report.warnings
    );
}

/// A subtree the walk cannot read may hold the user's files; counting it as
/// empty adopted the directory, and the next run swept it.
#[cfg(unix)]
#[test]
fn plugin_assets_does_not_adopt_an_unmarked_directory_it_cannot_fully_read() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/mine");
    let private = plugin_dir.join("private");
    std::fs::create_dir_all(&private).unwrap();
    std::fs::write(private.join("secret.txt"), b"mine\n").unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&private).is_ok() {
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
        return; // running as root: permissions do not apply
    }

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("mine")])
        .unwrap();
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        !plugin_dir.join(super::MIRROR_MARKER).exists(),
        "an unreadable subtree must count as foreign, warnings were {:?}",
        report.warnings
    );
}

/// `write_skills` creates a plugin directory of its own for a plugin-origin
/// skill. A later full mirror of the same plugin must be able to take it over
/// even when the skill folder names differ, rather than refusing it as a
/// directory the user made.
#[test]
fn plugin_assets_takes_over_a_plugin_dir_that_write_skills_created() {
    use crate::common::PluginAsset;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let mut skill = make_skill_with_frontmatter("Deep_Work");
    skill.plugin_origin = Some(crate::common::PluginOrigin {
        plugin_name: "my-plugin".to_string(),
        publisher: "market".to_string(),
        version: "1.0.0".to_string(),
    });
    super::skills::write_skills(tmp.path(), &[skill]).unwrap();
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");
    assert!(plugin_dir.join(".cursor-plugin/plugin.json").exists());

    let shipped = PluginAsset::new(
        "my-plugin".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from("skills/Deep_Work/SKILL.md"),
        b"---\nname: Deep_Work\ndescription: d\n---\nbody\n".to_vec(),
        false,
    );
    let report = adapter
        .write_plugin_assets(&[plugin_manifest("my-plugin"), shipped])
        .unwrap();

    assert!(
        plugin_dir.join("skills/Deep_Work/SKILL.md").exists(),
        "the full mirror was refused, warnings were {:?}",
        report.warnings
    );
    assert!(plugin_dir.join(super::MIRROR_MARKER).exists());
}

/// `atomic_write` resolves a symlink and replaces its target, so a link
/// planted at `.skrills-mirror` had the file it points to overwritten with
/// the marker on every sync.
#[cfg(unix)]
#[test]
fn plugin_assets_does_not_write_the_marker_through_a_symlink() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, b"precious").unwrap();
    let plugin_dir = tmp.path().join("plugins/local/linked");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::os::unix::fs::symlink(&outside, plugin_dir.join(super::MIRROR_MARKER)).unwrap();

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("linked")])
        .unwrap();

    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"precious",
        "warnings were {:?}",
        report.warnings
    );
}

/// A plugin that ships a file at the marker's path makes the symlinked marker
/// count as one of its own files, so the directory is adopted and the marker
/// step itself has to refuse the link.
#[cfg(unix)]
#[test]
fn plugin_assets_warns_and_leaves_the_target_when_an_adopted_dir_has_a_symlinked_marker() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, b"precious").unwrap();
    let plugin_dir = tmp.path().join("plugins/local/linked");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::os::unix::fs::symlink(&outside, plugin_dir.join(super::MIRROR_MARKER)).unwrap();
    let shipped_marker = crate::common::PluginAsset::new(
        "linked".to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(super::MIRROR_MARKER),
        b"from the plugin".to_vec(),
        false,
    );

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("linked"), shipped_marker])
        .unwrap();

    assert_eq!(std::fs::read(&outside).unwrap(), b"precious");
    assert!(
        report.warnings.iter().any(|w| w.contains(&format!(
            "plugins/local/linked/{} is a symlink",
            super::MIRROR_MARKER
        ))),
        "warnings were {:?}",
        report.warnings
    );
}

#[cfg(unix)]
#[test]
fn plugin_assets_warns_and_leaves_the_target_when_a_skill_marker_is_a_symlink() {
    use crate::adapters::utils::SYNCED_MARKER;

    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, b"precious").unwrap();
    let plugin_dir = tmp.path().join("plugins/local/my-plugin");
    mark_as_mirrored(&plugin_dir);
    let skill_dir = plugin_dir.join("skills/alpha");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::os::unix::fs::symlink(&outside, skill_dir.join(SYNCED_MARKER)).unwrap();

    let report = adapter
        .write_plugin_assets(&[
            plugin_manifest("my-plugin"),
            plugin_skill("my-plugin", "alpha"),
        ])
        .unwrap();

    assert_eq!(std::fs::read(&outside).unwrap(), b"precious");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("is a symlink, so the skill directory")),
        "warnings were {:?}",
        report.warnings
    );
}

/// A symlinked `plugins/local` entry (a dev checkout) is the user's, even
/// when its target carries a marker from an earlier mirror.
#[cfg(unix)]
#[test]
fn plugin_assets_never_prunes_a_symlinked_plugin_entry() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let checkout = tmp.path().join("checkout");
    mark_as_mirrored(&checkout);
    let local_dir = tmp.path().join("plugins/local");
    std::fs::create_dir_all(&local_dir).unwrap();
    std::os::unix::fs::symlink(&checkout, local_dir.join("dev")).unwrap();

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("other")])
        .unwrap();

    assert!(
        std::fs::symlink_metadata(local_dir.join("dev")).is_ok(),
        "the link was pruned, warnings were {:?}",
        report.warnings
    );
}

/// A plugin skill in a Claude cache under `root`, with its manifest.
fn cached_plugin_skill(claude_root: &std::path::Path, version: &str) -> Command {
    let version_dir = claude_root.join(format!("plugins/cache/market/my-plugin/{version}"));
    std::fs::create_dir_all(version_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        version_dir.join(".claude-plugin/plugin.json"),
        format!("{{\"name\": \"my-plugin\", \"version\": \"{version}\"}}"),
    )
    .unwrap();
    let mut skill = make_skill_with_frontmatter("deep-work");
    skill.source_path = version_dir.join("skills/deep-work/SKILL.md");
    skill.plugin_origin = Some(crate::common::PluginOrigin {
        plugin_name: "my-plugin".to_string(),
        publisher: "market".to_string(),
        version: version.to_string(),
    });
    skill
}

/// The manifest was looked up in the real `~/.claude`, whatever the source
/// root was, and written once and never updated after a version bump.
#[test]
fn write_skills_takes_the_plugin_manifest_from_the_skills_own_cache_and_refreshes_it() {
    let tmp = TempDir::new().unwrap();
    let claude = tmp.path().join("claude");
    let cursor = tmp.path().join("cursor");
    let manifest = cursor.join("plugins/local/my-plugin/.cursor-plugin/plugin.json");

    super::skills::write_skills(&cursor, &[cached_plugin_skill(&claude, "1.0.0")]).unwrap();
    assert!(std::fs::read_to_string(&manifest)
        .unwrap()
        .contains("1.0.0"));

    super::skills::write_skills(&cursor, &[cached_plugin_skill(&claude, "2.0.0")]).unwrap();
    assert!(
        std::fs::read_to_string(&manifest)
            .unwrap()
            .contains("2.0.0"),
        "{}",
        std::fs::read_to_string(&manifest).unwrap()
    );
}

/// A symlinked plugin directory (a dev checkout) got the manifest written into
/// its target before the skill writer refused the link.
#[cfg(unix)]
#[test]
fn write_skills_writes_no_manifest_through_a_symlinked_plugin_dir() {
    let tmp = TempDir::new().unwrap();
    let cursor = tmp.path().join("cursor");
    let checkout = tmp.path().join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::create_dir_all(cursor.join("plugins/local")).unwrap();
    std::os::unix::fs::symlink(&checkout, cursor.join("plugins/local/my-plugin")).unwrap();

    super::skills::write_skills(
        &cursor,
        &[cached_plugin_skill(&tmp.path().join("claude"), "1.0.0")],
    )
    .unwrap();

    assert!(!checkout.join(".cursor-plugin").exists());
}

fn shipped(plugin: &str, path: &str) -> crate::common::PluginAsset {
    crate::common::PluginAsset::new(
        plugin.to_string(),
        "market".to_string(),
        "1.0.0".to_string(),
        std::path::PathBuf::from(path),
        format!("# {path}\n").into_bytes(),
        false,
    )
}

/// memory-palace hooks write session files into their own mirror. The sweep
/// removed every file the new batch lacked, so an upgrade deleted them.
#[test]
fn plugin_assets_sweep_keeps_files_the_mirror_never_wrote() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/p");
    adapter
        .write_plugin_assets(&[plugin_manifest("p"), shipped("p", "skills/old/SKILL.md")])
        .unwrap();
    std::fs::create_dir_all(plugin_dir.join("data/sessions")).unwrap();
    std::fs::write(plugin_dir.join("data/sessions/s.json"), b"{}").unwrap();

    let report = adapter
        .write_plugin_assets(&[plugin_manifest("p")])
        .unwrap();

    assert!(
        !plugin_dir.join("skills/old").exists(),
        "{:?}",
        report.warnings
    );
    assert!(
        plugin_dir.join("data/sessions/s.json").exists(),
        "{:?}",
        report.warnings
    );
}

#[test]
fn plugin_assets_keep_versions_keeps_files_a_newer_version_dropped() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let old = tmp.path().join("plugins/local/p/skills/old/SKILL.md");
    adapter
        .write_plugin_assets(&[plugin_manifest("p"), shipped("p", "skills/old/SKILL.md")])
        .unwrap();

    let keep = crate::adapters::MirrorOptions {
        keep_versions: true,
    };
    adapter
        .write_plugin_assets_with(&[plugin_manifest("p")], &keep)
        .unwrap();
    assert!(old.exists(), "--keep-versions must keep the dropped file");

    adapter
        .write_plugin_assets(&[plugin_manifest("p")])
        .unwrap();
    assert!(!old.exists(), "a later default sync must still remove it");
}

/// Mirrors written before the marker existed hold files of older plugin
/// versions, so the foreign-file check refused them and they never got the
/// latest version.
#[test]
fn plugin_assets_adopts_a_legacy_mirror_whose_manifest_names_the_plugin() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let plugin_dir = tmp.path().join("plugins/local/legacy");
    std::fs::create_dir_all(plugin_dir.join(".cursor-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join(".cursor-plugin/plugin.json"),
        b"{\"name\": \"legacy\", \"version\": \"0.9.0\"}",
    )
    .unwrap();
    std::fs::write(plugin_dir.join("leftover.md"), b"from 0.9.0\n").unwrap();

    adapter
        .write_plugin_assets(&[
            plugin_manifest("legacy"),
            shipped("legacy", "scripts/run.sh"),
        ])
        .unwrap();
    assert!(plugin_dir.join("scripts/run.sh").exists());
    assert!(plugin_dir.join(super::MIRROR_MARKER).exists());

    adapter
        .write_plugin_assets(&[plugin_manifest("legacy")])
        .unwrap();
    assert!(
        !plugin_dir.join("scripts/run.sh").exists(),
        "dropped upstream"
    );
    assert!(
        plugin_dir.join("leftover.md").exists(),
        "never written by sync"
    );
}

#[test]
fn plugin_assets_prune_keeps_files_a_ledgered_mirror_never_wrote() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf());
    let gone = tmp.path().join("plugins/local/gone");
    adapter
        .write_plugin_assets(&[plugin_manifest("gone"), plugin_manifest("stays")])
        .unwrap();
    std::fs::write(gone.join("notes.md"), b"mine\n").unwrap();

    adapter
        .write_plugin_assets(&[plugin_manifest("stays")])
        .unwrap();

    assert!(!gone.join(".cursor-plugin/plugin.json").exists());
    assert!(gone.join("notes.md").exists());
}

/// Cursor names its tools `Shell` and `Write` (cursor.com/docs/hooks), so a
/// mirrored `Bash` or `Edit|MultiEdit` matcher never fired in a plugin hook.
#[test]
fn plugin_hooks_json_maps_claude_tool_matchers_to_cursor_tools() {
    let tmp = TempDir::new().unwrap();
    let body = r#"{"hooks": {
      "PreToolUse": [
        {"matcher": "Write|Edit|MultiEdit", "hooks": [{"type": "command", "command": "a"}]},
        {"matcher": "Bash", "hooks": [{"type": "command", "command": "b"}]},
        {"matcher": "Read|Grep|Glob", "hooks": [{"type": "command", "command": "c"}]},
        {"matcher": "mcp__.*", "hooks": [{"type": "command", "command": "d"}]}
      ],
      "SessionStart": [
        {"matcher": "startup|clear", "hooks": [{"type": "command", "command": "e"}]}
      ]
    }}"#;
    let (hooks, _) = mirrored_plugin_hooks(&tmp, body);

    let pre = hooks["hooks"]["preToolUse"].as_array().unwrap();
    let matchers: Vec<&str> = pre.iter().map(|e| e["matcher"].as_str().unwrap()).collect();
    assert_eq!(
        matchers,
        ["Write", "Shell", "Read|Grep|Glob", "mcp__.*"],
        "{hooks:#}"
    );
    assert_eq!(
        hooks["hooks"]["sessionStart"][0]["matcher"],
        "startup|clear"
    );
}
