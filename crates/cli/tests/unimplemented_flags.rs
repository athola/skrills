//! A flag the binary parses has to either act or refuse.
//!
//! `sync-all --validate`, `sync-all --autofix` and `validate --watch` were
//! once accepted, documented and ignored: the command ran without them and
//! exited 0, so a script asking for a validated sync got an unvalidated one.
//! `validate --watch` still refuses (#208). The two `sync-all` flags now act,
//! fail-closed: the source skills are validated before any target is synced,
//! and a remaining error aborts every target with nothing written. The same
//! shape hid `skrills dashboard` in a build without the `dashboard` feature,
//! which logged an error and still exited 0.

use std::path::Path;
use std::process::{Command, Output};

/// A skill Codex accepts.
const VALID_SKILL: &str = "---\nname: good\ndescription: A skill that validates for every target\n---\n# Good\n\nUse this skill to check that validation passes.\n";
/// A skill with no frontmatter, which Codex rejects and autofix can repair.
const BARE_SKILL: &str =
    "# Bare\n\nThis skill has no frontmatter, so Codex will not load it as written.\n";

fn run_skrills(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("SKRILLS_SKILL_DIR")
        .env_remove("SKRILLS_MIRROR_SOURCE")
        .env_remove("RUST_LOG")
        .output()
        .expect("spawn skrills")
}

fn assert_refused(output: &Output, needle: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "expected a non-zero exit, got success. stderr: {stderr}"
    );
    assert!(
        stderr.contains(needle),
        "stderr should mention {needle:?}, got: {stderr}"
    );
}

fn assert_succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "expected success, got {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_claude_skill(home: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = home.join(".claude/skills").join(name).join("SKILL.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn sync_all_validate_with_an_invalid_source_aborts_before_any_target() {
    let home = tempfile::tempdir().unwrap();
    seed_claude_skill(home.path(), "good", VALID_SKILL);
    seed_claude_skill(home.path(), "bare", BARE_SKILL);

    let output = run_skrills(
        home.path(),
        &[
            "sync-all",
            "--from",
            "claude",
            "--to",
            "codex",
            "--validate",
        ],
    );

    assert_refused(&output, "aborted before syncing any target");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bare"),
        "the failing skill is named: {stderr}"
    );
    assert!(
        !home.path().join(".codex").exists(),
        "nothing may be written to the target when validation fails"
    );
}

#[test]
fn sync_all_validate_with_a_valid_source_syncs() {
    let home = tempfile::tempdir().unwrap();
    seed_claude_skill(home.path(), "good", VALID_SKILL);

    let output = run_skrills(
        home.path(),
        &[
            "sync-all",
            "--from",
            "claude",
            "--to",
            "codex",
            "--validate",
        ],
    );

    assert_succeeded(&output);
    assert!(
        home.path()
            .join(".codex/skills/skills/good/SKILL.md")
            .exists()
            || home.path().join(".codex/skills/good/SKILL.md").exists(),
        "the validated skill should reach the target"
    );
}

#[test]
fn sync_all_autofix_repairs_the_source_then_syncs() {
    let home = tempfile::tempdir().unwrap();
    let bare = seed_claude_skill(home.path(), "bare", BARE_SKILL);

    let output = run_skrills(
        home.path(),
        &["sync-all", "--from", "claude", "--to", "codex", "--autofix"],
    );

    assert_succeeded(&output);
    let fixed = std::fs::read_to_string(&bare).unwrap();
    assert!(
        fixed.starts_with("---\n") && fixed.contains("name: bare"),
        "autofix should add frontmatter named after the directory: {fixed}"
    );
    assert!(home.path().join(".codex").exists(), "the sync should run");
}

#[test]
fn sync_all_autofix_dry_run_rewrites_nothing() {
    let home = tempfile::tempdir().unwrap();
    let bare = seed_claude_skill(home.path(), "bare", BARE_SKILL);

    let output = run_skrills(
        home.path(),
        &[
            "sync-all",
            "--from",
            "claude",
            "--to",
            "codex",
            "--autofix",
            "--dry-run",
        ],
    );

    assert_succeeded(&output);
    assert_eq!(std::fs::read_to_string(&bare).unwrap(), BARE_SKILL);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("would fix"), "{stderr}");
}

#[cfg(feature = "watch")]
#[test]
fn validate_watch_is_refused_not_ignored() {
    let home = tempfile::tempdir().unwrap();
    let output = run_skrills(home.path(), &["validate", "--watch"]);
    assert_refused(&output, "--watch");
}

#[cfg(not(feature = "dashboard"))]
#[test]
fn dashboard_without_the_feature_exits_non_zero() {
    let home = tempfile::tempdir().unwrap();
    let output = run_skrills(home.path(), &["dashboard"]);
    assert_refused(&output, "dashboard feature not enabled");
}
