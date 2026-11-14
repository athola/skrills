//! `skrills sync-all` snapshots what it changes and `skrills sync-rollback`
//! undoes it. Every run gets its own `HOME`, so no test reads or writes the
//! real `~/.skrills`.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn skrills(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skrills"))
        .env("HOME", home)
        .env_remove("SKRILLS_SNAPSHOT_RETENTION_DAYS")
        .args(args)
        .output()
        .expect("run skrills")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn seed_codex_skill(home: &Path) {
    let dir = home.join(".codex/skills/cli-test");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        "---\nname: cli-test\ndescription: CLI test skill\n---\n# CLI Test\n",
    )
    .unwrap();
}

fn snapshot_names(home: &Path) -> Vec<String> {
    let root = home.join(".skrills/snapshots");
    let Ok(reader) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = reader
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn sync_all_to_every_target_takes_one_snapshot_for_the_invocation() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());

    let output = skrills(home.path(), &["sync-all", "--from", "codex"]);
    assert!(output.status.success(), "{}", stderr(&output));

    assert!(home
        .path()
        .join(".claude/skills/cli-test/SKILL.md")
        .exists());
    assert!(home
        .path()
        .join(".copilot/skills/cli-test/SKILL.md")
        .exists());
    assert_eq!(snapshot_names(home.path()).len(), 1);
}

#[test]
fn sync_all_dry_run_takes_no_snapshot() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());

    let output = skrills(home.path(), &["sync-all", "--from", "codex", "--dry-run"]);
    assert!(output.status.success(), "{}", stderr(&output));

    assert!(!home.path().join(".skrills/snapshots").exists());
}

#[test]
fn sync_rollback_deletes_what_the_last_sync_all_created() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());
    skrills(home.path(), &["sync-all", "--from", "codex"]);
    let synced = home.path().join(".claude/skills/cli-test/SKILL.md");
    assert!(synced.exists());

    let output = skrills(home.path(), &["sync-rollback"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("Rolled back"),
        "{}",
        stdout(&output)
    );
    assert!(!synced.exists());
    assert!(home.path().join(".codex/skills/cli-test/SKILL.md").exists());
}

#[test]
fn sync_rollback_restores_a_file_the_sync_overwrote() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());
    let existing = home.path().join(".claude/skills/cli-test/SKILL.md");
    fs::create_dir_all(existing.parent().unwrap()).unwrap();
    fs::write(
        &existing,
        "---\nname: cli-test\ndescription: mine\n---\n# Mine\n",
    )
    .unwrap();
    let output = skrills(
        home.path(),
        &["sync-all", "--from", "codex", "--to", "claude"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(fs::read_to_string(&existing)
        .unwrap()
        .contains("CLI test skill"));

    let output = skrills(home.path(), &["sync-rollback"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(fs::read_to_string(&existing).unwrap().contains("# Mine"));
}

#[test]
fn sync_rollback_list_shows_each_snapshot_with_its_timestamp() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());
    skrills(home.path(), &["sync-all", "--from", "codex"]);
    let names = snapshot_names(home.path());

    let output = skrills(home.path(), &["sync-rollback", "--list"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let listing = stdout(&output);
    assert!(listing.contains(&names[0]), "{listing}");
    assert!(listing.contains(" UTC"), "{listing}");
    assert!(listing.contains("files"), "{listing}");
}

#[test]
fn sync_rollback_list_with_no_snapshots_says_so() {
    let home = tempfile::tempdir().unwrap();
    let output = skrills(home.path(), &["sync-rollback", "--list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("No snapshots"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn sync_rollback_with_no_snapshots_fails() {
    let home = tempfile::tempdir().unwrap();
    let output = skrills(home.path(), &["sync-rollback"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no snapshots"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn sync_rollback_can_restore_a_named_snapshot() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());
    skrills(home.path(), &["sync-all", "--from", "codex"]);
    let name = snapshot_names(home.path()).remove(0);

    let output = skrills(home.path(), &["sync-rollback", &name]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains(&name));
    assert!(!home
        .path()
        .join(".claude/skills/cli-test/SKILL.md")
        .exists());
}

#[test]
fn sync_rollback_rejects_an_unknown_snapshot_name() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());
    skrills(home.path(), &["sync-all", "--from", "codex"]);

    let output = skrills(home.path(), &["sync-rollback", "../../etc"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no snapshot named"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn sync_rollback_list_and_a_snapshot_name_conflict() {
    let home = tempfile::tempdir().unwrap();
    let output = skrills(
        home.path(),
        &["sync-rollback", "--list", "20260101T000000Z"],
    );
    assert!(!output.status.success());
}

#[test]
fn sync_all_prunes_snapshots_older_than_the_retention_period() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());
    let old = home.path().join(".skrills/snapshots/20200101T000000Z");
    fs::create_dir_all(&old).unwrap();

    let output = skrills(home.path(), &["sync-all", "--from", "codex"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!old.exists());
    assert_eq!(snapshot_names(home.path()).len(), 1);
}

#[test]
fn retention_days_env_keeps_snapshots_newer_than_the_setting() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_skill(home.path());
    let old = home.path().join(".skrills/snapshots/20200101T000000Z");
    fs::create_dir_all(&old).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_skrills"))
        .env("HOME", home.path())
        .env("SKRILLS_SNAPSHOT_RETENTION_DAYS", "36500")
        .args(["sync-all", "--from", "codex"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(old.exists());
}
