//! A flag the binary parses has to either act or refuse.
//!
//! `sync-all --validate`, `sync-all --autofix` and `validate --watch` were
//! once accepted, documented and ignored: the command ran without them and
//! exited 0, so a script asking for a validated sync got an unvalidated one.
//! `validate --watch` now runs its loop (#208) and refuses the flags it
//! cannot honor. The two `sync-all` flags now act,
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
        home.path().join(".agents/skills/good/SKILL.md").exists(),
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

/// `--watch` prints results as files change, so a one-shot JSON document or
/// an autofix pass has nowhere to go; refusing beats silently dropping them.
#[cfg(feature = "watch")]
#[test]
fn validate_watch_refuses_flags_it_cannot_honor() {
    let home = tempfile::tempdir().unwrap();
    for extra in ["--format=json", "--autofix"] {
        let output = run_skrills(home.path(), &["validate", "--watch", extra]);
        assert_refused(&output, "--watch");
    }
}

/// Reads the child's stdout on a thread so the test can wait for a line
/// with a deadline instead of sleeping.
#[cfg(all(feature = "watch", unix))]
fn wait_for_line(
    lines: &std::sync::mpsc::Receiver<String>,
    needle: &str,
    seen: &mut Vec<String>,
) -> bool {
    wait_for_line_within(lines, needle, seen, std::time::Duration::from_secs(30))
}

#[cfg(all(feature = "watch", unix))]
fn wait_for_line_within(
    lines: &std::sync::mpsc::Receiver<String>,
    needle: &str,
    seen: &mut Vec<String>,
    within: std::time::Duration,
) -> bool {
    let deadline = std::time::Instant::now() + within;
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match lines.recv_timeout(left) {
            Ok(line) => {
                let hit = line.contains(needle);
                seen.push(line);
                if hit {
                    return true;
                }
            }
            Err(_) => return false,
        }
    }
    false
}

#[cfg(all(feature = "watch", unix))]
#[test]
fn validate_watch_revalidates_a_changed_skill_and_stops_on_sigint() {
    use std::io::BufRead;

    let home = tempfile::tempdir().unwrap();
    let skills = home.path().join("watched");
    std::fs::create_dir_all(skills.join("late")).unwrap();
    std::fs::create_dir_all(skills.join("tidy")).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args([
            "validate",
            "--watch",
            "--errors-only",
            "--debounce-ms",
            "50",
            "--skill-dir",
        ])
        .arg(&skills)
        .env("HOME", home.path())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("SKRILLS_SKILL_DIR")
        .env_remove("SKRILLS_MIRROR_SOURCE")
        .env_remove("RUST_LOG")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn skrills");
    let stdout = child.stdout.take().unwrap();
    let (tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut seen = Vec::new();
    let ready = wait_for_line(&lines, "Press Ctrl+C", &mut seen);
    // The watcher can be listening before FSEvents delivers its first event,
    // so the write is repeated (with new bytes each time) until it is seen.
    // A valid skill changes alongside; `--errors-only` must leave it out.
    let mut revalidated = false;
    for attempt in 0..20 {
        if !ready || revalidated {
            break;
        }
        std::fs::write(
            skills.join("tidy/SKILL.md"),
            format!("---\nname: tidy\ndescription: A valid skill\n---\n# Tidy {attempt}\n"),
        )
        .unwrap();
        std::fs::write(
            skills.join("late/SKILL.md"),
            format!("{BARE_SKILL}\n{attempt}\n"),
        )
        .unwrap();
        revalidated = wait_for_line_within(
            &lines,
            "Revalidating",
            &mut seen,
            std::time::Duration::from_millis(1500),
        );
    }
    // The result lines for the batch follow the header.
    wait_for_line_within(&lines, "late", &mut seen, std::time::Duration::from_secs(5));

    let pid = child.id().to_string();
    let killed = Command::new("kill").args(["-INT", &pid]).status().unwrap();
    let status = child.wait().unwrap();
    let stopped = wait_for_line(&lines, "Watch mode stopped", &mut seen);

    assert!(ready, "watcher never reported ready: {seen:?}");
    assert!(revalidated, "change was not revalidated: {seen:?}");
    assert!(
        seen.iter().any(|line| line.contains("late")),
        "the failing skill was not reported: {seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.contains("tidy")),
        "--errors-only printed a valid skill: {seen:?}"
    );
    assert!(killed.success());
    assert!(status.success(), "Ctrl+C should exit 0, got {status:?}");
    assert!(stopped, "no clean stop message: {seen:?}");
}

#[cfg(not(feature = "dashboard"))]
#[test]
fn dashboard_without_the_feature_exits_non_zero() {
    let home = tempfile::tempdir().unwrap();
    let output = run_skrills(home.path(), &["dashboard"]);
    assert_refused(&output, "dashboard feature not enabled");
}
