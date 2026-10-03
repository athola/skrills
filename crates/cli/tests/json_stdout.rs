//! Under `--format json`, stdout carries the JSON document and nothing else.
//!
//! The tracing subscriber wrote to stdout and the first-run banner was
//! printed there too, so a consumer had to scan for where the document
//! started. Every notice now goes to stderr, and stdout parses as it is.
//!
//! The same runs pin SA-5: with no `--skill-dir` and no `SKRILLS_SKILL_DIR`,
//! these commands said "No skills found" although their help promises "all
//! discovered skills".

use std::path::Path;
use std::process::Command;

const SKILL: &str = "---\nname: alpha\ndescription: A skill that validates for every target\n---\n# Alpha\n\nUse this skill to check the JSON output.\n";

/// Runs the binary against an unconfigured temp HOME with logging forced on,
/// which is the noisiest case: the first-run check fires and every
/// `tracing::info!` is emitted.
fn run_json(home: &Path, args: &[&str]) -> (serde_json::Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args(args)
        .env("HOME", home)
        .env("RUST_LOG", "debug")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("SKRILLS_SKILL_DIR")
        .output()
        .expect("spawn skrills");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{args:?} exited {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status
    );
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("{args:?}: stdout is not one JSON document ({e}):\n{stdout}\nstderr: {stderr}")
    });
    (value, stderr)
}

fn home_with_one_skill() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let skill = home.path().join(".claude/skills/alpha/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(skill, SKILL).unwrap();
    home
}

#[test]
fn analyze_json_stdout_parses_and_finds_default_skills() {
    let home = home_with_one_skill();
    let (value, stderr) = run_json(home.path(), &["analyze", "--format", "json"]);
    assert_eq!(
        value.as_array().map(Vec::len),
        Some(1),
        "the ~/.claude/skills skill is discovered by default: {value}"
    );
    assert!(
        stderr.contains("DEBUG") || stderr.contains("INFO"),
        "logging was on, so the log lines must have gone to stderr: {stderr}"
    );
}

#[test]
fn validate_json_stdout_parses_and_finds_default_skills() {
    let home = home_with_one_skill();
    let (value, _) = run_json(home.path(), &["validate", "--format", "json"]);
    assert_eq!(value.as_array().map(Vec::len), Some(1), "{value}");
}

#[test]
fn metrics_json_stdout_parses_and_finds_default_skills() {
    let home = home_with_one_skill();
    let (value, _) = run_json(home.path(), &["metrics", "--format", "json"]);
    assert_eq!(value["total_skills"], 1, "{value}");
}

#[test]
fn skill_catalog_json_stdout_parses_and_finds_default_skills() {
    let home = home_with_one_skill();
    let (value, _) = run_json(home.path(), &["skill-catalog", "--format", "json"]);
    assert_eq!(value["total_skills"], 1, "{value}");
}
