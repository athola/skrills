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

/// Runs `skrills sync-all` with `HOME` pointing at `home` and returns stdout
/// and stderr for the failure message.
fn run_sync_all(home: &std::path::Path, args: &[&str]) -> Result<(String, String)> {
    let output = Command::new(env!("CARGO_BIN_EXE_skrills"))
        .env("HOME", home)
        .arg("sync-all")
        .args(args)
        .output()
        .context("Failed to execute sync-all command")?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "sync-all {args:?} should succeed\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
    );
    Ok((stdout, stderr))
}

/// Seeds `~/.claude/plugins/cache/market/my-plugin/1.0.0` with one skill and the
/// plugin manifest.
fn seed_claude_plugin_skill(home: &std::path::Path) -> Result<()> {
    let version_dir = home.join(".claude/plugins/cache/market/my-plugin/1.0.0");
    let skill_dir = version_dir.join("skills/deep-work");
    fs::create_dir_all(&skill_dir)?;
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: deep-work\ndescription: From a plugin\n---\n# Deep work\n",
    )?;
    let manifest_dir = version_dir.join(".claude-plugin");
    fs::create_dir_all(&manifest_dir)?;
    fs::write(
        manifest_dir.join("plugin.json"),
        "{\"name\": \"my-plugin\", \"version\": \"1.0.0\"}\n",
    )?;
    Ok(())
}

/// Cursor loads plugin skills from `plugins/local/<plugin>/skills/`, so the flat
/// `~/.cursor/skills` copy of the same body is a duplicate. Before the fix this
/// path wrote both.
#[test]
fn given_only_plugin_skills_when_sync_all_to_cursor_then_bodies_ride_the_plugin_mirror(
) -> Result<()> {
    let _g = skrills_test_utils::env_guard();
    let tmp = tempfile::tempdir()?;
    let home = tmp.path();
    let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));

    seed_claude_plugin_skill(home)?;

    let (stdout, stderr) = run_sync_all(home, &["--from", "claude", "--to", "cursor"])?;

    assert!(
        home.join(".cursor/plugins/local/my-plugin/skills/deep-work/SKILL.md")
            .exists(),
        "the plugin mirror should carry the skill body\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
    );
    assert!(
        !home.join(".cursor/skills").exists(),
        "no skill without a plugin origin exists, so the flat directory should not be created"
    );
    Ok(())
}

/// A skill in `~/.claude/skills` has no plugin origin, so it is absent from the
/// mirror and still needs the flat copy. Turning the flat sync off for every
/// Cursor target dropped these entirely.
#[test]
fn given_a_skill_without_plugin_origin_when_sync_all_to_cursor_then_it_lands_in_cursor_skills(
) -> Result<()> {
    let _g = skrills_test_utils::env_guard();
    let tmp = tempfile::tempdir()?;
    let home = tmp.path();
    let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));

    seed_claude_plugin_skill(home)?;
    let local_skill = home.join(".claude/skills/hand-written");
    fs::create_dir_all(&local_skill)?;
    fs::write(
        local_skill.join("SKILL.md"),
        "---\nname: hand-written\ndescription: Not from a plugin\n---\n# Hand written\n",
    )?;

    let (stdout, stderr) = run_sync_all(home, &["--from", "claude", "--to", "cursor"])?;

    assert!(
        home.join(".cursor/skills/hand-written/SKILL.md").exists(),
        "a skill with no plugin origin needs the flat copy\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
    );
    assert!(
        !home.join(".cursor/skills/deep-work").exists(),
        "the plugin skill must not be duplicated into the flat directory"
    );
    assert!(
        home.join(".cursor/plugins/local/my-plugin/skills/deep-work/SKILL.md")
            .exists(),
        "the plugin skill still belongs in the mirror"
    );
    Ok(())
}

/// Codex has no plugin cache, so nothing can ride the mirror and the flat copy
/// is the only way its skills reach Cursor. This direction used to report
/// "Skills: 0 synced" and exit 0.
#[test]
fn given_a_codex_skill_when_sync_all_to_cursor_then_it_lands_in_cursor_skills() -> Result<()> {
    let _g = skrills_test_utils::env_guard();
    let tmp = tempfile::tempdir()?;
    let home = tmp.path();
    let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));

    let skill_dir = home.join(".codex/skills/codex-only");
    fs::create_dir_all(&skill_dir)?;
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: codex-only\ndescription: Lives in codex\n---\n# Codex only\n",
    )?;

    let (stdout, stderr) = run_sync_all(home, &["--from", "codex", "--to", "cursor"])?;

    assert!(
        home.join(".cursor/skills/codex-only/SKILL.md").exists(),
        "a Codex source has no plugin cache, so the flat copy must still run\n\
         STDOUT:\n{stdout}\nSTDERR:\n{stderr}"
    );
    Ok(())
}
