//! `skrills prune` finds skills that session history says nobody uses and
//! removes them through a snapshot that `skrills sync-rollback` restores.
//! Every run gets its own `HOME` with fixture skills and session logs, so no
//! test reads or writes the real `~/.claude` or `~/.skrills`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const DAY: u64 = 86_400;

fn skrills(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_skrills"))
        .env("HOME", home)
        .env_remove("SKRILLS_SKILL_DIR")
        .env_remove("SKRILLS_INCLUDE_CLAUDE")
        .env_remove("SKRILLS_INCLUDE_MARKETPLACE")
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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn rfc3339(secs: u64) -> String {
    ::time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .unwrap()
        .format(&::time::format_description::well_known::Rfc3339)
        .unwrap()
}

fn date(secs: u64) -> String {
    rfc3339(secs)[..10].to_string()
}

/// `<home>/.claude/skills`, the root the tests prune.
fn skills_root(home: &Path) -> PathBuf {
    home.join(".claude/skills")
}

fn seed_skill_in(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("references")).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Fixture skill {name}\n---\n# {name}\n"),
    )
    .unwrap();
    fs::write(
        dir.join("references/notes.md"),
        format!("notes for {name}\n"),
    )
    .unwrap();
    dir
}

fn seed_skill(home: &Path, name: &str) -> PathBuf {
    seed_skill_in(&skills_root(home), name)
}

/// One Claude session log in which each `(skill, age in days)` was invoked.
fn seed_sessions(home: &Path, uses: &[(&str, u64)]) {
    let dir = home.join(".claude/projects/fixture-project");
    fs::create_dir_all(&dir).unwrap();
    let lines: Vec<String> = uses
        .iter()
        .map(|(skill, age_days)| {
            format!(
                r#"{{"timestamp":"{}","message":{{"role":"assistant","content":[{{"type":"tool_use","name":"Skill","input":{{"skill":"{skill}"}}}}]}}}}"#,
                rfc3339(now() - age_days * DAY)
            )
        })
        .collect();
    fs::write(dir.join("session.jsonl"), lines.join("\n")).unwrap();
}

fn prune(home: &Path, flags: &[&str]) -> Output {
    let root = skills_root(home);
    let mut args = vec!["prune", "--skill-dir", root.to_str().unwrap()];
    args.extend_from_slice(flags);
    skrills(home, &args)
}

fn snapshot_names(home: &Path) -> Vec<String> {
    let Ok(reader) = fs::read_dir(home.join(".skrills/snapshots")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = reader
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn suggest_lists_stale_and_never_used_skills_but_not_fresh_ones() {
    let home = tempfile::tempdir().unwrap();
    for name in ["stale-skill", "fresh-skill", "unused-skill"] {
        seed_skill(home.path(), name);
    }
    seed_sessions(home.path(), &[("stale-skill", 200), ("fresh-skill", 5)]);

    let output = prune(home.path(), &["--suggest"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let report = stdout(&output);
    assert!(report.contains("stale-skill"), "{report}");
    assert!(report.contains(&date(now() - 200 * DAY)), "{report}");
    assert!(report.contains("unused-skill"), "{report}");
    assert!(report.contains("never used in mined sessions"), "{report}");
    assert!(!report.contains("fresh-skill"), "{report}");
}

#[test]
fn suggest_does_not_modify_skills_or_take_a_snapshot() {
    let home = tempfile::tempdir().unwrap();
    let stale = seed_skill(home.path(), "stale-skill");
    seed_skill(home.path(), "unused-skill");
    seed_sessions(home.path(), &[("stale-skill", 200)]);

    let output = prune(home.path(), &["--suggest"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stale.join("SKILL.md").exists());
    assert!(skills_root(home.path()).join("unused-skill").exists());
    assert!(snapshot_names(home.path()).is_empty());
    assert!(
        !home.path().join(".skrills/analytics_cache.json").exists(),
        "suggest must not write the analytics cache"
    );
}

#[test]
fn suggest_json_gives_null_last_used_for_a_never_used_skill() {
    let home = tempfile::tempdir().unwrap();
    seed_skill(home.path(), "stale-skill");
    seed_skill(home.path(), "unused-skill");
    seed_sessions(home.path(), &[("stale-skill", 200)]);

    let output = prune(home.path(), &["--suggest", "--format", "json"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let report: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(report["older_than_days"], 90);
    let candidates = report["candidates"].as_array().unwrap();
    let by_name = |name: &str| {
        candidates
            .iter()
            .find(|candidate| candidate["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from {report}"))
    };
    assert_eq!(
        by_name("stale-skill")["last_used"].as_str().unwrap()[..10],
        date(now() - 200 * DAY)
    );
    assert!(by_name("unused-skill")["last_used"].is_null());
}

#[test]
fn suggest_fails_without_usage_data_instead_of_marking_everything_stale() {
    let home = tempfile::tempdir().unwrap();
    let skill = seed_skill(home.path(), "some-skill");

    let output = prune(home.path(), &["--suggest"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no skill usage"),
        "{}",
        stderr(&output)
    );
    assert!(
        !stdout(&output).contains("some-skill"),
        "{}",
        stdout(&output)
    );
    assert!(skill.join("SKILL.md").exists());
}

#[test]
fn apply_fails_without_usage_data_and_deletes_nothing() {
    let home = tempfile::tempdir().unwrap();
    let skill = seed_skill(home.path(), "some-skill");

    let output = prune(home.path(), &["--apply", "--yes"]);

    assert!(!output.status.success());
    assert!(skill.join("SKILL.md").exists());
    assert!(snapshot_names(home.path()).is_empty());
}

#[test]
fn threshold_flag_moves_the_stale_line() {
    let home = tempfile::tempdir().unwrap();
    seed_skill(home.path(), "month-old");
    seed_sessions(home.path(), &[("month-old", 40)]);

    let default_run = prune(home.path(), &["--suggest"]);
    let strict_run = prune(home.path(), &["--suggest", "--older-than-days", "30"]);

    assert!(default_run.status.success(), "{}", stderr(&default_run));
    assert!(!stdout(&default_run).contains("month-old"));
    assert!(strict_run.status.success(), "{}", stderr(&strict_run));
    assert!(stdout(&strict_run).contains("month-old"));
}

#[test]
fn a_zero_day_threshold_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    seed_skill(home.path(), "some-skill");
    seed_sessions(home.path(), &[("some-skill", 1)]);

    let output = prune(home.path(), &["--suggest", "--older-than-days", "0"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--older-than-days"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn prune_requires_exactly_one_of_suggest_and_apply() {
    let home = tempfile::tempdir().unwrap();

    let neither = prune(home.path(), &[]);
    let both = prune(home.path(), &["--suggest", "--apply"]);

    assert!(!neither.status.success());
    assert!(
        stderr(&neither).contains("--suggest") && stderr(&neither).contains("--apply"),
        "{}",
        stderr(&neither)
    );
    assert!(!both.status.success());
    assert!(
        stderr(&both).contains("cannot be used with"),
        "{}",
        stderr(&both)
    );
}

#[test]
fn yes_without_apply_is_rejected() {
    let home = tempfile::tempdir().unwrap();

    let output = prune(home.path(), &["--suggest", "--yes"]);

    assert!(!output.status.success());
    // A parse error, not the later "no skill usage" failure this test used
    // to pass on.
    assert!(
        stderr(&output).contains("cannot be used with") && stderr(&output).contains("'--yes'"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn prune_apply_refuses_plugin_cache_skills_found_through_an_extra_skill_dir() {
    let home = tempfile::tempdir().unwrap();
    let plugins = home.path().join(".claude/plugins");
    let cached = seed_skill_in(&plugins.join("cache/acme/plugin/skills"), "plugin-skill");
    let market = seed_skill_in(&plugins.join("marketplaces/acme/skills"), "market-skill");
    seed_sessions(home.path(), &[("other-skill", 200)]);
    let root = plugins.to_str().unwrap();

    let suggest = skrills(
        home.path(),
        &[
            "prune",
            "--skill-dir",
            root,
            "--suggest",
            "--format",
            "json",
        ],
    );
    let apply = skrills(
        home.path(),
        &["prune", "--skill-dir", root, "--apply", "--yes"],
    );

    assert!(suggest.status.success(), "{}", stderr(&suggest));
    let report: serde_json::Value = serde_json::from_str(&stdout(&suggest)).unwrap();
    assert_eq!(report["candidates"], serde_json::json!([]), "{report}");
    let skipped = report["skipped"].as_array().unwrap();
    for name in ["plugin-skill", "market-skill"] {
        let entry = skipped
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("{name} not skipped: {report}"));
        assert!(
            entry["reason"].as_str().unwrap().contains("plugin"),
            "{entry}"
        );
    }
    assert!(apply.status.success(), "{}", stderr(&apply));
    assert!(cached.join("SKILL.md").exists());
    assert!(market.join("SKILL.md").exists());
}

#[test]
fn a_mirror_skill_found_through_an_extra_skill_dir_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let mirrored = seed_skill_in(&home.path().join(".codex/skills-mirror"), "mirror-skill");
    seed_sessions(home.path(), &[("other-skill", 200)]);
    let root = home.path().join(".codex");

    let apply = skrills(
        home.path(),
        &[
            "prune",
            "--skill-dir",
            root.to_str().unwrap(),
            "--apply",
            "--yes",
        ],
    );

    assert!(apply.status.success(), "{}", stderr(&apply));
    assert!(mirrored.join("SKILL.md").exists());
}

#[test]
fn apply_refuses_when_session_history_is_shorter_than_the_cutoff() {
    let home = tempfile::tempdir().unwrap();
    let unused = seed_skill(home.path(), "unused-skill");
    seed_sessions(home.path(), &[("fresh-skill", 10), ("fresh-skill", 2)]);

    let output = prune(home.path(), &["--apply", "--yes"]);

    assert!(!output.status.success(), "{}", stdout(&output));
    let err = stderr(&output);
    assert!(err.contains(&date(now() - 10 * DAY)), "{err}");
    assert!(err.contains("--ignore-short-history"), "{err}");
    assert!(unused.join("SKILL.md").exists());
    assert!(snapshot_names(home.path()).is_empty());
}

#[test]
fn ignore_short_history_deletes_and_still_prints_the_caveat() {
    let home = tempfile::tempdir().unwrap();
    let unused = seed_skill(home.path(), "unused-skill");
    seed_sessions(home.path(), &[("fresh-skill", 10)]);

    let output = prune(home.path(), &["--apply", "--yes", "--ignore-short-history"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let shown = format!("{}{}", stdout(&output), stderr(&output));
    assert!(shown.contains("only starts"), "{shown}");
    assert!(!unused.exists());
}

#[test]
fn ignore_short_history_is_refused_with_suggest() {
    let home = tempfile::tempdir().unwrap();

    let output = prune(home.path(), &["--suggest", "--ignore-short-history"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot be used with")
            && stderr(&output).contains("'--ignore-short-history'"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn apply_names_the_snapshot_it_took() {
    let home = tempfile::tempdir().unwrap();
    seed_skill(home.path(), "stale-skill");
    seed_sessions(home.path(), &[("stale-skill", 200)]);

    let output = prune(home.path(), &["--apply", "--yes"]);

    assert!(output.status.success(), "{}", stderr(&output));
    let names = snapshot_names(home.path());
    assert_eq!(names.len(), 1);
    assert!(stdout(&output).contains(&names[0]), "{}", stdout(&output));
}

#[test]
fn apply_deletes_nothing_when_no_snapshot_can_be_taken() {
    let home = tempfile::tempdir().unwrap();
    let stale = seed_skill(home.path(), "stale-skill");
    seed_sessions(home.path(), &[("stale-skill", 200)]);
    // A file where the snapshot directory has to go.
    fs::write(home.path().join(".skrills"), "not a directory").unwrap();

    let output = prune(home.path(), &["--apply", "--yes"]);

    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(
        err.contains("snapshot") && err.contains("nothing deleted"),
        "{err}"
    );
    assert!(stale.join("SKILL.md").exists());
}

#[test]
fn apply_with_yes_deletes_stale_skills_and_keeps_fresh_ones() {
    let home = tempfile::tempdir().unwrap();
    let stale = seed_skill(home.path(), "stale-skill");
    let unused = seed_skill(home.path(), "unused-skill");
    let fresh = seed_skill(home.path(), "fresh-skill");
    seed_sessions(home.path(), &[("stale-skill", 200), ("fresh-skill", 5)]);

    let output = prune(home.path(), &["--apply", "--yes"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stale.exists());
    assert!(!unused.exists());
    assert!(fresh.join("SKILL.md").exists());
    assert_eq!(snapshot_names(home.path()).len(), 1);
}

#[test]
fn sync_rollback_restores_every_file_of_a_pruned_skill() {
    let home = tempfile::tempdir().unwrap();
    let stale = seed_skill(home.path(), "stale-skill");
    let skill_md = fs::read(stale.join("SKILL.md")).unwrap();
    let notes = fs::read(stale.join("references/notes.md")).unwrap();
    seed_sessions(home.path(), &[("stale-skill", 200)]);
    let pruned = prune(home.path(), &["--apply", "--yes"]);
    assert!(pruned.status.success(), "{}", stderr(&pruned));
    assert!(!stale.exists());

    let rollback = skrills(home.path(), &["sync-rollback"]);

    assert!(rollback.status.success(), "{}", stderr(&rollback));
    assert_eq!(fs::read(stale.join("SKILL.md")).unwrap(), skill_md);
    assert_eq!(fs::read(stale.join("references/notes.md")).unwrap(), notes);
}

#[test]
fn apply_without_a_terminal_or_yes_fails_before_deleting_anything() {
    let home = tempfile::tempdir().unwrap();
    let stale = seed_skill(home.path(), "stale-skill");
    seed_sessions(home.path(), &[("stale-skill", 200)]);

    let output = prune(home.path(), &["--apply"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--yes"),
        "the error should name the flag that skips the prompt: {}",
        stderr(&output)
    );
    assert!(stale.join("SKILL.md").exists());
    assert!(snapshot_names(home.path()).is_empty());
}

#[test]
fn apply_with_nothing_stale_changes_nothing_and_takes_no_snapshot() {
    let home = tempfile::tempdir().unwrap();
    let fresh = seed_skill(home.path(), "fresh-skill");
    seed_sessions(home.path(), &[("fresh-skill", 5)]);

    let output = prune(home.path(), &["--apply", "--yes"]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(fresh.join("SKILL.md").exists());
    assert!(snapshot_names(home.path()).is_empty());
}

#[test]
fn plugin_cache_skills_are_never_suggested_or_deleted() {
    let home = tempfile::tempdir().unwrap();
    let cache_root = home
        .path()
        .join(".claude/plugins/cache/vendor/plugin/skills");
    let cached = seed_skill_in(&cache_root, "plugin-skill");
    seed_skill(home.path(), "stale-skill");
    seed_sessions(home.path(), &[("stale-skill", 200)]);
    let user_root = skills_root(home.path());
    let roots = [user_root.to_str().unwrap(), cache_root.to_str().unwrap()];

    let suggest = skrills(
        home.path(),
        &[
            "prune",
            "--skill-dir",
            roots[0],
            "--skill-dir",
            roots[1],
            "--suggest",
        ],
    );
    let apply = skrills(
        home.path(),
        &[
            "prune",
            "--skill-dir",
            roots[0],
            "--skill-dir",
            roots[1],
            "--apply",
            "--yes",
        ],
    );

    assert!(suggest.status.success(), "{}", stderr(&suggest));
    // Named only under the protected heading, never as a candidate.
    let report = stdout(&suggest);
    let (candidates, protected) = report.split_once("Stale but protected").unwrap();
    assert!(!candidates.contains("plugin-skill"), "{report}");
    assert!(protected.contains("plugin-skill"), "{report}");
    assert!(apply.status.success(), "{}", stderr(&apply));
    assert!(cached.join("SKILL.md").exists());
    assert!(!user_root.join("stale-skill").exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_skill_directory_is_never_followed_or_deleted() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let target = seed_skill_in(elsewhere.path(), "linked-skill");
    let link = skills_root(home.path()).join("linked-skill");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    seed_skill(home.path(), "stale-skill");
    seed_sessions(home.path(), &[("stale-skill", 200)]);

    let suggest = prune(home.path(), &["--suggest"]);
    let apply = prune(home.path(), &["--apply", "--yes"]);

    assert!(suggest.status.success(), "{}", stderr(&suggest));
    assert!(
        !stdout(&suggest).contains("linked-skill"),
        "{}",
        stdout(&suggest)
    );
    assert!(apply.status.success(), "{}", stderr(&apply));
    assert!(link.symlink_metadata().is_ok(), "the symlink was removed");
    assert!(
        target.join("SKILL.md").exists(),
        "the link target was deleted"
    );
}
