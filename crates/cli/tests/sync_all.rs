//! CLI integration test for `skrills sync-all --from codex`.
//!
//! Verifies end-to-end argument plumbing copies Codex skills into Claude.

use std::fs;
use std::process::Command;

use anyhow::Result;

#[test]
fn sync_all_copies_skills_from_codex_to_claude() -> Result<()> {
    // Isolate filesystem side effects
    let tmp = tempfile::tempdir()?;

    // Seed a Codex skill
    let codex_skills = tmp.path().join(".codex/skills");
    fs::create_dir_all(&codex_skills)?;
    fs::write(codex_skills.join("cli-test.md"), "# CLI Test")?;

    // Use pre-built binary instead of cargo run (avoids 60+ second recompile)
    let bin_path = env!("CARGO_BIN_EXE_skrills");
    let status = Command::new(bin_path)
        .env("HOME", tmp.path())
        .args(["sync-all", "--from", "codex"])
        .output()?;

    assert!(
        status.status.success(),
        "sync-all command should succeed (status={:?}, stderr={})",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );

    let claude_skill = tmp.path().join(".claude/skills/cli-test.md");
    assert!(
        claude_skill.exists(),
        "Claude skills directory should receive synced skill"
    );

    Ok(())
}
