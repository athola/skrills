//! `sync-all`, `mirror` and `sync-status` through the real binary: a failed
//! sync phase is a non-zero exit with its warning on screen, not a green run.
//! Every run gets its own `HOME`, and the variables that could point a CLI
//! at a real config directory are cleared.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn skrills(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("COPILOT_HOME")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("SKRILLS_MIRROR_SOURCE")
        .env_remove("SKRILLS_SKILL_DIR")
        .env_remove("SKRILLS_INCLUDE_CLAUDE")
        .env_remove("SKRILLS_INCLUDE_MARKETPLACE")
        .stdin(Stdio::null())
        .output()
        .expect("run skrills")
}

fn both(output: &Output) -> String {
    format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn seed_claude_skill(home: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let skill = home.join(format!(".claude/skills/{name}/SKILL.md"));
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    fs::write(&skill, body).unwrap();
    skill
}

/// A Claude MCP server to sync, and a Codex config.toml that does not parse,
/// so the MCP phase fails while the others run.
fn home_whose_codex_mcp_phase_fails() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    seed_claude_skill(
        home.path(),
        "alpha",
        "---\nname: alpha\ndescription: A fixture skill\n---\n# Alpha\n",
    );
    fs::write(
        home.path().join(".claude.json"),
        r#"{"mcpServers":{"fixture":{"command":"echo","args":["hi"]}}}"#,
    )
    .unwrap();
    fs::create_dir_all(home.path().join(".codex")).unwrap();
    fs::write(
        home.path().join(".codex/config.toml"),
        "this is = = not toml [",
    )
    .unwrap();
    home
}

#[test]
fn sync_all_exits_non_zero_when_a_sync_phase_fails() {
    let home = home_whose_codex_mcp_phase_fails();

    let output = skrills(
        home.path(),
        &["sync-all", "--from", "claude", "--to", "codex"],
    );

    assert!(!output.status.success(), "{}", both(&output));
    let shown = both(&output);
    assert!(shown.contains("sync failed"), "{shown}");
    assert!(shown.contains("codex: failed"), "{shown}");
}

#[test]
fn mirror_exits_non_zero_and_prints_the_phase_warning_when_a_phase_fails() {
    let home = home_whose_codex_mcp_phase_fails();

    let output = skrills(home.path(), &["mirror", "--skip-existing-commands"]);

    assert!(!output.status.success(), "{}", both(&output));
    let shown = both(&output);
    assert!(shown.contains("sync failed"), "{shown}");
    assert!(!shown.contains("mirror complete"), "{shown}");
}

/// `--autofix` rewrites source skills before the snapshot starts, so
/// `sync-rollback` cannot undo it; a run that then fails says so.
#[test]
fn a_failed_sync_after_autofix_names_the_rewritten_source_skills() {
    let home = home_whose_codex_mcp_phase_fails();
    // No frontmatter: Codex validation fails and autofix adds it.
    let skill = seed_claude_skill(home.path(), "needs-fix", "# Needs fix\n\nBody text.\n");

    let output = skrills(
        home.path(),
        &["sync-all", "--from", "claude", "--to", "codex", "--autofix"],
    );

    assert!(!output.status.success(), "{}", both(&output));
    let shown = both(&output);
    assert!(shown.contains("sync-rollback does not restore"), "{shown}");
    assert!(shown.contains(&skill.display().to_string()), "{shown}");
}

#[test]
fn sync_all_interactive_without_a_terminal_fails_and_writes_nothing() {
    let home = tempfile::tempdir().unwrap();
    seed_claude_skill(
        home.path(),
        "alpha",
        "---\nname: alpha\ndescription: A fixture skill\n---\n# Alpha\n",
    );

    let output = skrills(
        home.path(),
        &[
            "sync-all",
            "--from",
            "claude",
            "--to",
            "codex",
            "--interactive",
        ],
    );

    assert!(!output.status.success(), "{}", both(&output));
    assert!(both(&output).contains("terminal"), "{}", both(&output));
    assert!(!home.path().join(".agents").exists());
    assert!(!home.path().join(".codex").exists());
    assert!(!home.path().join(".skrills/snapshots").exists());
}

#[test]
fn sync_status_to_a_custom_adapter_previews_without_writing() {
    let home = tempfile::tempdir().unwrap();
    seed_claude_skill(
        home.path(),
        "alpha",
        "---\nname: alpha\ndescription: A fixture skill\n---\n# Alpha\n",
    );
    let adapters = home.path().join(".skrills/adapters");
    fs::create_dir_all(&adapters).unwrap();
    fs::write(
        adapters.join("demo.toml"),
        "name = \"demo\"\nroot = \"~/.demo\"\nskills_dir = \"skills\"\n",
    )
    .unwrap();

    let output = skrills(
        home.path(),
        &["sync-status", "--from", "claude", "--to", "demo"],
    );

    assert!(output.status.success(), "{}", both(&output));
    assert!(both(&output).contains("demo"), "{}", both(&output));
    assert!(
        !home.path().join(".demo").exists(),
        "a preview wrote the target"
    );
}
