//! Offline mode (issue #205): local commands work with `SKRILLS_OFFLINE=1`,
//! and network-backed ones say why they cannot run instead of trying.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const SKILL: &str =
    "---\nname: alpha\ndescription: A skill that validates for every target\n---\n# Alpha\n\nUse this skill offline.\n";

fn home_with_one_skill() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let skill = home.path().join(".claude/skills/alpha/SKILL.md");
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    fs::write(skill, SKILL).unwrap();
    home
}

/// Runs the binary with HOME in a tempdir. `SKRILLS_OFFLINE` is set only when
/// `offline_env` is true, so the `--offline` flag can be tested on its own.
fn run(home: &Path, offline_env: bool, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_skrills"));
    command
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CACHE_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("SKRILLS_SKILL_DIR")
        .env("SKRILLS_INCLUDE_CLAUDE", "1")
        .env_remove("SKRILLS_OFFLINE");
    if offline_env {
        command.env("SKRILLS_OFFLINE", "1");
    }
    command.output().expect("spawn skrills")
}

fn stdout_and_stderr(output: &Output) -> String {
    format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn validate_works_with_skrills_offline_set() {
    let home = home_with_one_skill();
    let output = run(home.path(), true, &["validate", "--format", "json"]);
    assert!(output.status.success(), "{}", stdout_and_stderr(&output));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_array().map(Vec::len), Some(1), "{report}");
}

#[test]
fn analyze_works_with_skrills_offline_set() {
    let home = home_with_one_skill();
    let output = run(home.path(), true, &["analyze", "--format", "json"]);
    assert!(output.status.success(), "{}", stdout_and_stderr(&output));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.as_array().map(Vec::len), Some(1), "{report}");
}

#[test]
fn local_sync_between_environments_works_with_skrills_offline_set() {
    let home = home_with_one_skill();
    let output = run(home.path(), true, &["sync-all", "--from", "claude"]);
    assert!(output.status.success(), "{}", stdout_and_stderr(&output));
    assert!(
        home.path().join(".agents/skills/alpha/SKILL.md").exists(),
        "the Claude skill should reach the Codex skills directory: {}",
        stdout_and_stderr(&output)
    );
}

#[test]
fn github_skill_search_offline_without_cache_names_offline_mode() {
    let home = tempfile::tempdir().unwrap();
    let output = run(home.path(), true, &["search-skills-github", "rust"]);
    assert!(!output.status.success(), "{}", stdout_and_stderr(&output));
    let text = stdout_and_stderr(&output);
    assert!(text.contains("offline mode"), "{text}");
    assert!(text.contains("no cached data"), "{text}");
}

#[test]
fn the_offline_flag_acts_like_the_environment_switch() {
    let home = tempfile::tempdir().unwrap();
    let output = run(
        home.path(),
        false,
        &["--offline", "search-skills-github", "rust"],
    );
    assert!(!output.status.success(), "{}", stdout_and_stderr(&output));
    assert!(
        stdout_and_stderr(&output).contains("no cached data"),
        "{}",
        stdout_and_stderr(&output)
    );
}
