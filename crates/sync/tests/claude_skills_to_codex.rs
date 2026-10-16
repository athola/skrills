//! The Claude -> Codex skills path that `sync`, `mirror`, `setup`, the TUI
//! and the `sync-from-claude` MCP tool share. It used to live in
//! `skrills-server` beside a second, tree-copying engine.

use std::fs;
use std::path::Path;

use skrills_sync::sync_claude_skills_to_codex;
use tempfile::tempdir;

fn write_skill(dir: &Path, name: &str) {
    let dir = dir.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: test skill\n---\nbody\n"),
    )
    .unwrap();
}

#[test]
fn claude_skills_land_in_the_shared_dir_codex_loads() {
    let tmp = tempdir().unwrap();
    let home = tmp.path();
    let claude_root = home.join(".claude");
    write_skill(&claude_root.join("skills"), "demo");

    let report = sync_claude_skills_to_codex(&claude_root, home, false, false).unwrap();

    assert_eq!(report.skills.written, 1);
    assert!(home.join(".agents/skills/demo/SKILL.md").exists());
    assert!(!home.join(".codex/skills/demo").exists());
}

/// Codex loads nothing without `[features] skills = true`, so a config the
/// flag cannot be written to must be reported, not swallowed.
#[test]
fn unwritable_codex_config_is_reported_as_a_skill_warning() {
    let tmp = tempdir().unwrap();
    let home = tmp.path();
    let claude_root = home.join(".claude");
    write_skill(&claude_root.join("skills"), "demo");
    // A directory where the file belongs makes every read of it fail.
    fs::create_dir_all(home.join(".codex/config.toml")).unwrap();

    let report = sync_claude_skills_to_codex(&claude_root, home, false, false).unwrap();

    assert!(home.join(".agents/skills/demo/SKILL.md").exists());
    assert!(
        report
            .skills
            .warnings
            .iter()
            .any(|w| w.contains("codex skills feature")),
        "{:?}",
        report.skills.warnings
    );
}

#[test]
fn claude_to_codex_skill_dry_run_writes_nothing() {
    let tmp = tempdir().unwrap();
    let home = tmp.path();
    let claude_root = home.join(".claude");
    write_skill(&claude_root.join("skills"), "demo");

    let report = sync_claude_skills_to_codex(&claude_root, home, false, true).unwrap();

    assert!(report.success);
    assert!(!home.join(".agents").exists());
    assert!(!home.join(".codex").exists());
}

/// The old copier honoured `include_marketplace` for skills; the adapter path
/// that replaced it read no marketplace skills at all.
#[test]
fn marketplace_skills_sync_only_when_included() {
    let tmp = tempdir().unwrap();
    let home = tmp.path();
    let claude_root = home.join(".claude");
    write_skill(
        &claude_root.join("plugins/marketplaces/a/plugins/b/skills"),
        "market",
    );

    let without = sync_claude_skills_to_codex(&claude_root, home, false, false).unwrap();
    assert_eq!(without.skills.written, 0);
    assert!(!home.join(".agents/skills/market").exists());

    let with = sync_claude_skills_to_codex(&claude_root, home, true, false).unwrap();
    assert_eq!(with.skills.written, 1);
    assert!(home.join(".agents/skills/market/SKILL.md").exists());
}

/// A user skill keeps its name when a marketplace plugin ships one alike.
#[test]
fn a_local_skill_wins_over_a_marketplace_skill_of_the_same_name() {
    let tmp = tempdir().unwrap();
    let home = tmp.path();
    let claude_root = home.join(".claude");
    write_skill(&claude_root.join("skills"), "demo");
    let market = claude_root.join("plugins/marketplaces/a/plugins/b/skills/demo");
    fs::create_dir_all(&market).unwrap();
    fs::write(
        market.join("SKILL.md"),
        "---\nname: demo\ndescription: marketplace copy\n---\nother\n",
    )
    .unwrap();

    let report = sync_claude_skills_to_codex(&claude_root, home, true, false).unwrap();

    assert_eq!(report.skills.written, 1);
    let written = fs::read_to_string(home.join(".agents/skills/demo/SKILL.md")).unwrap();
    assert!(written.contains("test skill"), "{written}");
}

/// `sync-all --include-marketplace` and the MCP sync tools route through
/// `sync_between`, which built the Claude source without marketplace skills.
#[test]
fn sync_between_honours_include_marketplace_for_skills() {
    let _lock = skrills_test_utils::env_guard();
    let fixture = skrills_test_utils::TestFixture::new().unwrap();
    let _home = fixture.home_guard();
    let home = fixture.home_path();
    write_skill(
        &home.join(".claude/plugins/marketplaces/a/plugins/b/skills"),
        "market",
    );
    let skills_only = skrills_sync::SyncParams {
        sync_commands: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_hooks: false,
        sync_instructions: false,
        sync_plugin_assets: false,
        include_marketplace: true,
        ..Default::default()
    };

    let report = skrills_sync::orchestrator::sync_between(
        skrills_sync::Platform::Claude,
        skrills_sync::Platform::Codex,
        &skills_only,
    )
    .unwrap();

    assert_eq!(report.skills.written, 1, "{:?}", report.skills);
    assert!(home.join(".agents/skills/market/SKILL.md").exists());
}
