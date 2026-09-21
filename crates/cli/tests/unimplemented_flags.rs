//! A flag the binary parses has to either act or refuse.
//!
//! `sync-all --validate`, `sync-all --autofix` and `validate --watch` were
//! accepted, documented and ignored: the command ran without them and exited
//! 0, so a script asking for a validated sync got an unvalidated one. The same
//! shape hid `skrills dashboard` in a build without the `dashboard` feature,
//! which logged an error and still exited 0.

use std::process::{Command, Output};

fn run_skrills(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args(args)
        .env("HOME", home)
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

#[test]
fn sync_all_validate_is_refused_not_ignored() {
    let home = tempfile::tempdir().unwrap();
    let output = run_skrills(home.path(), &["sync-all", "--dry-run", "--validate"]);
    assert_refused(&output, "--validate");
}

#[test]
fn sync_all_autofix_is_refused_not_ignored() {
    let home = tempfile::tempdir().unwrap();
    let output = run_skrills(home.path(), &["sync-all", "--dry-run", "--autofix"]);
    assert_refused(&output, "--autofix");
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
