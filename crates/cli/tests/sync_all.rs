//! CLI integration test for `skrills sync-all --from codex`.
//!
//! Verifies end-to-end argument plumbing copies Codex skills into Claude.

use std::fs;
use std::process::Command;

use anyhow::{Context, Result};

#[test]
fn given_codex_skill_when_sync_all_from_codex_then_skill_is_copied_into_claude() -> Result<()> {
    let _g = skrills_test_utils::env_guard();

    // GIVEN a Codex skill exists under ~/.codex/skills
    // Isolate filesystem side effects - tempdir will auto-clean on drop
    let tmp = tempfile::tempdir()?;

    // Set HOME to temp directory (restored automatically on guard drop)
    let _home_guard = skrills_test_utils::set_env_var("HOME", Some(tmp.path().to_str().unwrap()));

    // Seed a Codex skill
    let codex_skills = tmp.path().join(".codex/skills");
    fs::create_dir_all(&codex_skills)?;
    let skill_dir = codex_skills.join("cli-test");
    fs::create_dir_all(&skill_dir)?;
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: cli-test\ndescription: CLI test skill\n---\n# CLI Test\n",
    )?;

    // WHEN the user runs `skrills sync-all --from codex`
    let bin_path = env!("CARGO_BIN_EXE_skrills");
    let output = Command::new(bin_path)
        // HOME already set in environment
        .args(["sync-all", "--from", "codex"])
        .output()
        .context("Failed to execute sync-all command")?;

    // Capture output for debugging
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // In debug builds, always show output
    if cfg!(debug_assertions) {
        eprintln!("sync-all stdout:\n{}", stdout);
        eprintln!("sync-all stderr:\n{}", stderr);
    }

    assert!(
        output.status.success(),
        "sync-all command should succeed\n\
         Status: {:?}\n\
         STDOUT:\n{}\n\
         STDERR:\n{}",
        output.status,
        stdout,
        stderr
    );

    // THEN the skill is copied into ~/.claude/skills
    let claude_skill = tmp.path().join(".claude/skills/cli-test/SKILL.md");
    assert!(
        claude_skill.exists(),
        "Claude skills directory should receive synced skill"
    );

    // AND legacy ~/.codex/skills-mirror is not created as a side effect
    assert!(
        !tmp.path().join(".codex/skills-mirror").exists(),
        "sync-all should not create ~/.codex/skills-mirror"
    );

    Ok(())
}
