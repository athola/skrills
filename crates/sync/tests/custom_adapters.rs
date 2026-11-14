//! Config-registered adapters: the `~/.skrills/adapters/<name>.toml` schema,
//! its validation, and a sync into and out of a custom layout.

use skrills_sync::adapters::traits::AgentAdapter;
use skrills_sync::{
    adapters_dir, create_endpoint_adapter, load_adapter_files, sync_between_endpoints,
    sync_between_endpoints_with_decider, AdapterConfig, ConfigAdapter, ConfigProblem, Decision,
    ItemChange, Platform, SkipReason, SyncEndpoint, SyncParams,
};
use std::fs;
use std::path::Path;
use std::sync::Arc;

const WINDSURF: &str = r#"
name = "windsurf"
root = "~/.windsurf"
skills_dir = "skills"
commands_dir = "workflows"
agents_dir = "agents"
instructions_file = "rules.md"

[mcp]
path = "mcp_config.json"
servers_key = "mcpServers"
"#;

fn put(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn parse(toml: &str, home: &Path) -> Result<AdapterConfig, Vec<ConfigProblem>> {
    AdapterConfig::from_toml(toml, home)
}

fn problems(toml: &str, home: &Path) -> Vec<ConfigProblem> {
    parse(toml, home).expect_err("config should be rejected")
}

fn write_adapter_file(home: &Path, file: &str, body: &str) {
    put(&adapters_dir(home).join(file), body);
}

// --- schema ---

#[test]
fn a_full_config_resolves_every_path_under_the_expanded_root() {
    let home = tempfile::tempdir().unwrap();

    let config = parse(WINDSURF, home.path()).unwrap();

    let root = home.path().join(".windsurf");
    assert_eq!(config.name, "windsurf");
    assert_eq!(config.root, root);
    assert_eq!(config.skills_dir, Some(root.join("skills")));
    assert_eq!(config.commands_dir, Some(root.join("workflows")));
    assert_eq!(config.agents_dir, Some(root.join("agents")));
    assert_eq!(config.instructions_file, Some(root.join("rules.md")));
    let mcp = config.mcp.unwrap();
    assert_eq!(mcp.path, root.join("mcp_config.json"));
    assert_eq!(mcp.servers_key, "mcpServers");
}

#[test]
fn invalid_toml_is_a_parse_problem() {
    let home = tempfile::tempdir().unwrap();

    let found = problems("name = ", home.path());

    assert!(
        matches!(found.as_slice(), [ConfigProblem::Parse(_)]),
        "{found:?}"
    );
}

#[test]
fn an_unknown_key_is_a_parse_problem_so_typos_are_not_silently_ignored() {
    let home = tempfile::tempdir().unwrap();

    let found = problems(
        "name = \"x\"\nroot = \"/tmp/x\"\nskill_dir = \"skills\"\n",
        home.path(),
    );

    assert!(
        matches!(found.as_slice(), [ConfigProblem::Parse(_)]),
        "{found:?}"
    );
}

#[test]
fn missing_name_and_root_are_both_reported() {
    let home = tempfile::tempdir().unwrap();

    let found = problems("skills_dir = \"skills\"\n", home.path());

    assert!(
        found.contains(&ConfigProblem::MissingKey("name")),
        "{found:?}"
    );
    assert!(
        found.contains(&ConfigProblem::MissingKey("root")),
        "{found:?}"
    );
}

#[test]
fn an_mcp_table_needs_both_path_and_servers_key() {
    let home = tempfile::tempdir().unwrap();

    let no_key = problems(
        "name = \"x\"\nroot = \"/tmp/x\"\n[mcp]\npath = \"m.json\"\n",
        home.path(),
    );
    let no_path = problems(
        "name = \"x\"\nroot = \"/tmp/x\"\n[mcp]\nservers_key = \"mcpServers\"\n",
        home.path(),
    );

    assert_eq!(no_key, vec![ConfigProblem::MissingKey("mcp.servers_key")]);
    assert_eq!(no_path, vec![ConfigProblem::MissingKey("mcp.path")]);
}

#[test]
fn a_root_that_is_not_absolute_after_tilde_expansion_is_rejected() {
    let home = tempfile::tempdir().unwrap();

    let found = problems("name = \"x\"\nroot = \"relative/dir\"\n", home.path());

    assert!(
        matches!(found.as_slice(), [ConfigProblem::RootNotAbsolute(_)]),
        "{found:?}"
    );
}

#[test]
fn a_bare_tilde_root_is_the_home_directory() {
    let home = tempfile::tempdir().unwrap();

    let config = parse("name = \"x\"\nroot = \"~\"\n", home.path()).unwrap();

    assert_eq!(config.root, home.path());
}

#[test]
fn names_must_be_lowercase_words() {
    let home = tempfile::tempdir().unwrap();

    for bad in ["", "Wind Surf", "../evil", "UPPER", "-lead"] {
        let found = problems(
            &format!("name = \"{bad}\"\nroot = \"/tmp/x\"\n"),
            home.path(),
        );
        assert!(
            found
                .iter()
                .any(|p| matches!(p, ConfigProblem::InvalidName(_))),
            "{bad:?}: {found:?}"
        );
    }
}

// --- categories ---

/// The configured flag first, then the four that must be off.
type SupportView = fn(&skrills_sync::FieldSupport) -> [bool; 5];

/// Each category's key alone turns on exactly that category, so the
/// orchestrator skips the rest with `UnsupportedField`.
#[test]
fn only_configured_categories_are_supported_in_both_directions() {
    let home = tempfile::tempdir().unwrap();
    let base = "name = \"x\"\nroot = \"/tmp/x\"\n";
    let cases: [(&str, SupportView); 5] = [
        ("skills_dir = \"s\"\n", |f| {
            [
                f.skills,
                f.commands,
                f.agents,
                f.instructions,
                f.mcp_servers,
            ]
        }),
        ("commands_dir = \"c\"\n", |f| {
            [
                f.commands,
                f.skills,
                f.agents,
                f.instructions,
                f.mcp_servers,
            ]
        }),
        ("agents_dir = \"a\"\n", |f| {
            [
                f.agents,
                f.skills,
                f.commands,
                f.instructions,
                f.mcp_servers,
            ]
        }),
        ("instructions_file = \"i.md\"\n", |f| {
            [
                f.instructions,
                f.skills,
                f.commands,
                f.agents,
                f.mcp_servers,
            ]
        }),
        ("[mcp]\npath = \"m.json\"\nservers_key = \"k\"\n", |f| {
            [
                f.mcp_servers,
                f.skills,
                f.commands,
                f.agents,
                f.instructions,
            ]
        }),
    ];
    for (extra, view) in cases {
        let adapter = ConfigAdapter::new(parse(&format!("{base}{extra}"), home.path()).unwrap());
        for support in [adapter.read_support(), adapter.write_support()] {
            let flags = view(&support);
            assert!(flags[0], "{extra}: the configured category is on");
            assert!(flags[1..].iter().all(|on| !on), "{extra}: the rest are off");
            assert!(
                !support.preferences && !support.hooks && !support.plugin_assets,
                "{extra}"
            );
        }
    }
}

#[test]
fn a_config_with_no_categories_supports_nothing() {
    let home = tempfile::tempdir().unwrap();
    let adapter =
        ConfigAdapter::new(parse("name = \"x\"\nroot = \"/tmp/x\"\n", home.path()).unwrap());

    let support = adapter.write_support();

    assert!(!support.skills && !support.commands && !support.agents);
    assert!(!support.instructions && !support.mcp_servers);
}

#[test]
fn categories_lists_the_configured_ones_by_name() {
    let home = tempfile::tempdir().unwrap();

    let all = parse(WINDSURF, home.path()).unwrap();
    let some = parse(
        "name = \"x\"\nroot = \"/tmp/x\"\nskills_dir = \"s\"\n",
        home.path(),
    )
    .unwrap();

    assert_eq!(
        all.categories(),
        vec![
            "skills",
            "commands",
            "agents",
            "instructions",
            "mcp_servers"
        ]
    );
    assert_eq!(some.categories(), vec!["skills"]);
}

// --- containment ---

#[test]
fn a_relative_path_that_climbs_out_of_the_root_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    let cases = [
        ("skills_dir = \"../outside\"", "skills_dir"),
        ("commands_dir = \"a/../../outside\"", "commands_dir"),
        ("agents_dir = \"..\"", "agents_dir"),
        ("instructions_file = \"../AGENTS.md\"", "instructions_file"),
        (
            "[mcp]\npath = \"../m.json\"\nservers_key = \"k\"",
            "mcp.path",
        ),
    ];
    for (line, key) in cases {
        let found = problems(
            &format!("name = \"x\"\nroot = \"/tmp/x\"\n{line}\n"),
            home.path(),
        );
        assert!(
            found
                .iter()
                .any(|p| matches!(p, ConfigProblem::PathEscapesRoot { key: k, .. } if *k == key)),
            "{line}: {found:?}"
        );
    }
}

#[test]
fn an_absolute_path_where_a_relative_one_is_expected_is_rejected() {
    let home = tempfile::tempdir().unwrap();

    let found = problems(
        "name = \"x\"\nroot = \"/tmp/x\"\nskills_dir = \"/etc\"\n",
        home.path(),
    );

    assert!(
        matches!(
            found.as_slice(),
            [ConfigProblem::PathEscapesRoot {
                key: "skills_dir",
                ..
            }]
        ),
        "{found:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_category_dir_that_is_a_symlink_out_of_the_root_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    let outside = home.path().join("elsewhere");
    let root = home.path().join("tool");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("skills")).unwrap();

    let found = problems(
        &format!(
            "name = \"x\"\nroot = \"{}\"\nskills_dir = \"skills\"\n",
            root.display()
        ),
        home.path(),
    );

    assert!(
        matches!(
            found.as_slice(),
            [ConfigProblem::PathEscapesRoot {
                key: "skills_dir",
                ..
            }]
        ),
        "{found:?}"
    );
}

// --- registry ---

#[test]
fn a_missing_adapters_directory_is_an_empty_registry() {
    let home = tempfile::tempdir().unwrap();

    assert!(load_adapter_files(home.path()).unwrap().is_empty());
}

#[test]
fn only_toml_files_are_adapters_and_each_keeps_its_path() {
    let home = tempfile::tempdir().unwrap();
    write_adapter_file(home.path(), "windsurf.toml", WINDSURF);
    write_adapter_file(home.path(), "notes.txt", "not an adapter");

    let files = load_adapter_files(home.path()).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].path,
        adapters_dir(home.path()).join("windsurf.toml")
    );
    assert_eq!(files[0].outcome.as_ref().unwrap().name, "windsurf");
}

#[test]
fn a_name_equal_to_a_built_in_is_rejected_whatever_its_case() {
    let home = tempfile::tempdir().unwrap();
    for builtin in ["claude", "codex", "copilot", "cursor"] {
        write_adapter_file(
            home.path(),
            &format!("{builtin}.toml"),
            &format!("name = \"{builtin}\"\nroot = \"/tmp/x\"\n"),
        );
    }

    let files = load_adapter_files(home.path()).unwrap();

    assert_eq!(files.len(), 4);
    for file in files {
        let found = file.outcome.expect_err("built-in name must be rejected");
        assert!(
            found
                .iter()
                .any(|p| matches!(p, ConfigProblem::BuiltinNameClash(_))),
            "{}: {found:?}",
            file.path.display()
        );
    }
}

#[test]
fn two_files_with_one_name_are_both_rejected_naming_the_other() {
    let home = tempfile::tempdir().unwrap();
    let body = "name = \"dup\"\nroot = \"/tmp/x\"\n";
    write_adapter_file(home.path(), "a.toml", body);
    write_adapter_file(home.path(), "b.toml", body);

    let files = load_adapter_files(home.path()).unwrap();

    let dir = adapters_dir(home.path());
    assert_eq!(
        files[0].outcome,
        Err(vec![ConfigProblem::DuplicateName {
            name: "dup".into(),
            other: dir.join("b.toml")
        }])
    );
    assert_eq!(
        files[1].outcome,
        Err(vec![ConfigProblem::DuplicateName {
            name: "dup".into(),
            other: dir.join("a.toml")
        }])
    );
}

// --- endpoints ---

#[test]
fn a_built_in_name_parses_as_the_platform_whatever_its_case() {
    assert_eq!(
        "CLAUDE".parse::<SyncEndpoint>(),
        Ok(SyncEndpoint::Builtin(Platform::Claude))
    );
    assert_eq!(
        "cursor".parse::<SyncEndpoint>(),
        Ok(SyncEndpoint::Builtin(Platform::Cursor))
    );
}

#[test]
fn any_other_name_parses_as_a_custom_endpoint_to_be_resolved_later() {
    assert_eq!(
        "windsurf".parse::<SyncEndpoint>(),
        Ok(SyncEndpoint::Custom("windsurf".to_string()))
    );
    assert_eq!(
        SyncEndpoint::Custom("windsurf".into()).to_string(),
        "windsurf"
    );
}

#[test]
fn a_platform_converts_to_its_endpoint() {
    assert_eq!(
        SyncEndpoint::from(Platform::Codex),
        SyncEndpoint::Builtin(Platform::Codex)
    );
}

#[test]
fn an_unknown_name_lists_the_built_ins_and_the_configured_adapters() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    write_adapter_file(home.path(), "windsurf.toml", WINDSURF);

    let err = create_endpoint_adapter(&SyncEndpoint::Custom("nope".into()))
        .err()
        .expect("unknown name")
        .to_string();

    for expected in ["nope", "claude", "codex", "copilot", "cursor", "windsurf"] {
        assert!(err.contains(expected), "{expected} missing from: {err}");
    }
}

#[test]
fn an_unknown_name_says_so_when_no_adapter_is_configured() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));

    let err = create_endpoint_adapter(&SyncEndpoint::Custom("nope".into()))
        .err()
        .expect("unknown name")
        .to_string();

    assert!(err.contains("none configured"), "{err}");
}

#[test]
fn a_configured_name_whose_file_is_invalid_names_the_file_and_the_problem() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    write_adapter_file(
        home.path(),
        "windsurf.toml",
        "name = \"windsurf\"\nroot = \"/tmp/x\"\nskills_dir = \"../out\"\n",
    );

    let err = create_endpoint_adapter(&SyncEndpoint::Custom("windsurf".into()))
        .err()
        .expect("invalid config")
        .to_string();

    assert!(err.contains("windsurf.toml"), "{err}");
    assert!(err.contains("skills_dir"), "{err}");
}

#[test]
fn a_built_in_endpoint_creates_the_built_in_adapter() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));

    let adapter = create_endpoint_adapter(&SyncEndpoint::Builtin(Platform::Cursor)).unwrap();

    assert_eq!(adapter.name(), "cursor");
}

// --- the adapter ---

#[test]
fn skills_with_modules_commands_and_agents_round_trip_through_the_custom_layout() {
    let home = tempfile::tempdir().unwrap();
    let adapter = ConfigAdapter::new(parse(WINDSURF, home.path()).unwrap());
    let root = home.path().join(".windsurf");
    let skill_source = tempfile::tempdir().unwrap();
    let source = skrills_sync::ClaudeAdapter::with_root(skill_source.path().to_path_buf());
    put(
        &skill_source.path().join("skills/alpha/SKILL.md"),
        "---\nname: alpha\ndescription: A\n---\nBody\n",
    );
    put(
        &skill_source.path().join("skills/alpha/notes.md"),
        "notes\n",
    );
    put(&skill_source.path().join("commands/hello.md"), "# Hello\n");
    put(
        &skill_source.path().join("agents/rev.md"),
        "---\nname: rev\n---\nBody\n",
    );

    let skills = source.read_skills().unwrap();
    let skill_report = adapter.write_skills(&skills).unwrap();
    let command_report = adapter
        .write_commands(&source.read_commands(false).unwrap())
        .unwrap();
    let agent_report = adapter
        .write_agents(&source.read_agents().unwrap())
        .unwrap();

    assert_eq!(skill_report.written, 1);
    assert_eq!(command_report.written, 1);
    assert_eq!(agent_report.written, 1);
    assert!(root.join("skills/alpha/SKILL.md").is_file());
    assert!(root.join("skills/alpha/notes.md").is_file());
    assert_eq!(
        fs::read_to_string(root.join("workflows/hello.md")).unwrap(),
        "# Hello\n"
    );
    assert!(root.join("agents/rev.md").is_file());

    let back = adapter.read_skills().unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].name, "alpha");
    assert_eq!(back[0].modules.len(), 1);
    assert_eq!(adapter.read_commands(false).unwrap()[0].name, "hello");
    assert_eq!(adapter.read_agents().unwrap()[0].name, "rev");
}

#[test]
fn readers_of_unconfigured_or_absent_directories_return_nothing() {
    let home = tempfile::tempdir().unwrap();
    let bare = ConfigAdapter::new(
        parse("name = \"x\"\nroot = \"/tmp/never-made\"\n", home.path()).unwrap(),
    );
    let absent = ConfigAdapter::new(parse(WINDSURF, home.path()).unwrap());

    for adapter in [&bare, &absent] {
        assert!(adapter.read_skills().unwrap().is_empty());
        assert!(adapter.read_commands(false).unwrap().is_empty());
        assert!(adapter.read_agents().unwrap().is_empty());
        assert!(adapter.read_instructions().unwrap().is_empty());
        assert!(adapter.read_mcp_servers().unwrap().is_empty());
        assert!(adapter.read_hooks().unwrap().is_empty());
    }
}

#[test]
fn mcp_servers_merge_into_the_configured_key_and_keep_other_keys() {
    let home = tempfile::tempdir().unwrap();
    let config = "name = \"x\"\nroot = \"~/.x\"\n[mcp]\npath = \"cfg/servers.json\"\nservers_key = \"servers\"\n";
    let adapter = ConfigAdapter::new(parse(config, home.path()).unwrap());
    let path = home.path().join(".x/cfg/servers.json");
    put(
        &path,
        r#"{"theme": "dark", "servers": {"mine": {"command": "/bin/mine"}}}"#,
    );
    let source = tempfile::tempdir().unwrap();
    put(
        &source.path().join(".claude.json"),
        r#"{"mcpServers": {"fresh": {"command": "/bin/fresh", "args": ["a"]}}}"#,
    );
    let servers = skrills_sync::ClaudeAdapter::with_root(source.path().to_path_buf())
        .read_mcp_servers()
        .unwrap();

    let report = adapter.write_mcp_servers(&servers).unwrap();

    assert_eq!(report.written, 1);
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["theme"], "dark");
    assert_eq!(written["servers"]["mine"]["command"], "/bin/mine");
    assert_eq!(written["servers"]["fresh"]["command"], "/bin/fresh");
    assert!(
        written.get("mcpServers").is_none(),
        "the default key must not be added"
    );
    let read_back = adapter.read_mcp_servers().unwrap();
    assert_eq!(read_back["fresh"].args, vec!["a".to_string()]);
    assert!(read_back.contains_key("mine"));
}

#[test]
fn an_mcp_file_whose_servers_key_is_not_an_object_is_an_error_naming_the_file() {
    let home = tempfile::tempdir().unwrap();
    let config =
        "name = \"x\"\nroot = \"~/.x\"\n[mcp]\npath = \"m.json\"\nservers_key = \"servers\"\n";
    let adapter = ConfigAdapter::new(parse(config, home.path()).unwrap());
    let path = home.path().join(".x/m.json");
    put(&path, r#"{"servers": ["a"]}"#);

    let err = adapter.read_mcp_servers().unwrap_err().to_string();

    assert!(err.contains("not an object"), "{err}");
    assert!(err.contains("m.json"), "{err}");
}

#[test]
fn malformed_sse_and_incomplete_mcp_entries_are_skipped_and_the_rest_kept() {
    let home = tempfile::tempdir().unwrap();
    let config =
        "name = \"x\"\nroot = \"~/.x\"\n[mcp]\npath = \"m.json\"\nservers_key = \"mcpServers\"\n";
    let adapter = ConfigAdapter::new(parse(config, home.path()).unwrap());
    put(
        &home.path().join(".x/m.json"),
        r#"{"mcpServers": {
            "good": {"command": "/bin/good", "disabled": true},
            "remote": {"url": "https://example.com/mcp", "headers": {"a": "b"}},
            "stream": {"type": "sse", "url": "https://example.com/sse"},
            "empty": {},
            "bad-env": {"command": "/bin/x", "env": {"PORT": 80}}
        }}"#,
    );

    let servers = adapter.read_mcp_servers().unwrap();

    let mut names: Vec<&str> = servers.keys().map(String::as_str).collect();
    names.sort();
    assert_eq!(names, vec!["good", "remote"]);
    assert!(!servers["good"].enabled);
    assert_eq!(
        servers["remote"].url.as_deref(),
        Some("https://example.com/mcp")
    );
}

#[test]
fn instructions_land_in_a_managed_block_and_are_not_read_back_as_the_users_own() {
    let home = tempfile::tempdir().unwrap();
    let adapter = ConfigAdapter::new(parse(WINDSURF, home.path()).unwrap());
    let path = home.path().join(".windsurf/rules.md");
    put(&path, "# My rules\n");
    let source = tempfile::tempdir().unwrap();
    put(&source.path().join("CLAUDE.md"), "# From Claude\n");
    let instructions = skrills_sync::ClaudeAdapter::with_root(source.path().to_path_buf())
        .read_instructions()
        .unwrap();

    let report = adapter.write_instructions(&instructions).unwrap();

    assert_eq!(report.written, 1);
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# My rules\n"), "{text}");
    assert!(text.contains("# From Claude"), "{text}");
    let read_back = adapter.read_instructions().unwrap();
    assert_eq!(read_back.len(), 1);
    assert_eq!(
        String::from_utf8_lossy(&read_back[0].content),
        "# My rules\n"
    );
    // A second identical write changes nothing.
    assert_eq!(
        adapter.write_instructions(&instructions).unwrap().written,
        0
    );
}

// --- end to end ---

fn seed_claude(home: &Path) {
    put(&home.join(".claude/commands/hello.md"), "# Hello\n");
    put(
        &home.join(".claude/skills/alpha/SKILL.md"),
        "---\nname: alpha\ndescription: A\n---\n\nBody\n",
    );
    put(&home.join(".claude/skills/alpha/notes.md"), "notes\n");
    put(
        &home.join(".claude/agents/rev.md"),
        "---\nname: rev\ndescription: R\n---\nBody\n",
    );
}

const SKILLS_AND_COMMANDS_ONLY: &str = r#"
name = "windsurf"
root = "~/.windsurf"
skills_dir = "skills"
commands_dir = "workflows"
"#;

fn claude() -> SyncEndpoint {
    SyncEndpoint::Builtin(Platform::Claude)
}

fn windsurf() -> SyncEndpoint {
    SyncEndpoint::Custom("windsurf".to_string())
}

fn sync_params() -> SyncParams {
    SyncParams {
        from: Some("claude".to_string()),
        ..Default::default()
    }
}

#[test]
fn sync_from_claude_writes_skills_and_commands_and_skips_unsupported_categories() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    seed_claude(home.path());
    write_adapter_file(home.path(), "windsurf.toml", SKILLS_AND_COMMANDS_ONLY);

    let report = sync_between_endpoints(&claude(), &windsurf(), &sync_params()).unwrap();

    let root = home.path().join(".windsurf");
    assert_eq!(report.skills.written, 1);
    assert_eq!(report.commands.written, 1);
    assert!(root.join("skills/alpha/SKILL.md").is_file());
    assert!(root.join("skills/alpha/notes.md").is_file());
    assert_eq!(
        fs::read_to_string(root.join("workflows/hello.md")).unwrap(),
        "# Hello\n"
    );
    assert!(!root.join("agents").exists(), "agents are not configured");
    assert!(
        report
            .agents
            .skipped
            .iter()
            .any(|reason| matches!(reason, SkipReason::UnsupportedField { .. })),
        "{:?}",
        report.agents.skipped
    );
}

#[test]
fn a_dry_run_into_a_custom_adapter_writes_nothing() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    seed_claude(home.path());
    write_adapter_file(home.path(), "windsurf.toml", SKILLS_AND_COMMANDS_ONLY);
    let params = SyncParams {
        dry_run: true,
        ..sync_params()
    };

    let report = sync_between_endpoints(&claude(), &windsurf(), &params).unwrap();

    assert_eq!(report.skills.written, 1);
    assert!(!home.path().join(".windsurf").exists());
}

#[test]
fn a_second_sync_into_a_custom_adapter_writes_nothing_new() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    seed_claude(home.path());
    write_adapter_file(home.path(), "windsurf.toml", SKILLS_AND_COMMANDS_ONLY);
    let _report = sync_between_endpoints(&claude(), &windsurf(), &sync_params()).unwrap();

    let again = sync_between_endpoints(&claude(), &windsurf(), &sync_params()).unwrap();

    assert_eq!(again.skills.written, 0);
    assert_eq!(again.commands.written, 0);
}

#[test]
fn rolling_back_a_sync_into_a_custom_adapter_removes_what_it_created() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    seed_claude(home.path());
    write_adapter_file(home.path(), "windsurf.toml", SKILLS_AND_COMMANDS_ONLY);
    // A file the target already had must come back as it was.
    let existing = home.path().join(".windsurf/workflows/hello.md");
    put(&existing, "# Mine\n");
    let _report = sync_between_endpoints(&claude(), &windsurf(), &sync_params()).unwrap();
    assert_eq!(fs::read_to_string(&existing).unwrap(), "# Hello\n");

    let snapshots = skrills_sync::snapshot::default_root().unwrap();
    let rolled = skrills_sync::snapshot::rollback(&snapshots, None).unwrap();

    assert!(rolled.restored >= 1 && rolled.removed >= 1, "{rolled:?}");
    assert_eq!(fs::read_to_string(&existing).unwrap(), "# Mine\n");
    assert!(!home.path().join(".windsurf/skills/alpha/SKILL.md").exists());
    assert!(!home.path().join(".windsurf/skills/alpha/notes.md").exists());
}

#[test]
fn a_custom_adapter_can_be_the_source_of_a_sync() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    write_adapter_file(home.path(), "windsurf.toml", SKILLS_AND_COMMANDS_ONLY);
    put(&home.path().join(".windsurf/workflows/ship.md"), "# Ship\n");
    let params = SyncParams {
        from: Some("windsurf".to_string()),
        ..Default::default()
    };

    let report = sync_between_endpoints(&windsurf(), &claude(), &params).unwrap();

    assert_eq!(report.commands.written, 1);
    assert_eq!(
        fs::read_to_string(home.path().join(".claude/commands/ship.md")).unwrap(),
        "# Ship\n"
    );
}

/// Interactive decisions use the default whole-category scope: a rejection
/// skips the category and leaves the target's bytes alone.
#[test]
fn rejecting_every_change_interactively_leaves_a_custom_target_untouched() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    seed_claude(home.path());
    write_adapter_file(home.path(), "windsurf.toml", SKILLS_AND_COMMANDS_ONLY);
    let existing = home.path().join(".windsurf/workflows/hello.md");
    put(&existing, "# Mine\n");
    let params = SyncParams {
        interactive: true,
        ..sync_params()
    };
    let decider: Arc<dyn skrills_sync::ItemDecider> =
        Arc::new(|_: &ItemChange| Ok(Decision::Reject));

    let _report =
        sync_between_endpoints_with_decider(&claude(), &windsurf(), &params, decider).unwrap();

    assert_eq!(fs::read_to_string(&existing).unwrap(), "# Mine\n");
    assert!(!home.path().join(".windsurf/skills").exists());
}

#[test]
fn accepting_every_change_interactively_writes_to_a_custom_target() {
    let _guard = skrills_test_utils::env_guard();
    let home = tempfile::tempdir().unwrap();
    let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
    seed_claude(home.path());
    write_adapter_file(home.path(), "windsurf.toml", SKILLS_AND_COMMANDS_ONLY);
    let params = SyncParams {
        interactive: true,
        ..sync_params()
    };
    let decider: Arc<dyn skrills_sync::ItemDecider> =
        Arc::new(|_: &ItemChange| Ok(Decision::Accept));

    let _report =
        sync_between_endpoints_with_decider(&claude(), &windsurf(), &params, decider).unwrap();

    assert!(home
        .path()
        .join(".windsurf/skills/alpha/SKILL.md")
        .is_file());
}
