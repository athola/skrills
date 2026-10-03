use anyhow::{bail, Context, Result};
use skrills_discovery::discover_skills;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::OutputFormat;

use super::{find_skill, RollbackResult, SkillVersion};

/// Handle the skill-rollback command.
pub(crate) fn handle_skill_rollback_command(
    name: String,
    version: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    let roots = crate::commands::skill_roots_for(&skill_dirs);
    let skills = discover_skills(&roots, None)?;
    let skill = find_skill(&skills, &name)?;
    let skill_path = &skill.path;

    let available_versions = skill_history(skill_path)?;

    if available_versions.is_empty() {
        if format.is_json() {
            let result = RollbackResult {
                skill_name: skill.name.clone(),
                skill_path: skill_path.clone(),
                rolled_back: false,
                from_version: None,
                to_version: None,
                available_versions: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!(
                "No git history found for skill '{}' at {}",
                skill.name,
                skill_path.display()
            );
            println!("Skill rollback requires the skill to be under git version control.");
        }
        return Ok(());
    }

    match version {
        Some(target_version) => {
            checkout_version(skill_path, &target_version)?;

            let result = RollbackResult {
                skill_name: skill.name.clone(),
                skill_path: skill_path.clone(),
                rolled_back: true,
                from_version: available_versions.first().map(|v| v.hash.clone()),
                to_version: Some(target_version.clone()),
                available_versions: vec![],
            };

            if format.is_json() {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("Rolled back '{}' to version {}", skill.name, target_version);
                println!("  Path: {}", skill_path.display());
            }
        }
        None => {
            let result = RollbackResult {
                skill_name: skill.name.clone(),
                skill_path: skill_path.clone(),
                rolled_back: false,
                from_version: None,
                to_version: None,
                available_versions: available_versions.clone(),
            };

            if format.is_json() {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "Available versions for '{}' ({}):",
                    skill.name,
                    skill_path.display()
                );
                println!();
                for (i, v) in available_versions.iter().enumerate() {
                    let current = if i == 0 { " (current)" } else { "" };
                    println!(
                        "  {} - {}{}",
                        v.hash,
                        v.date.split_whitespace().next().unwrap_or(&v.date),
                        current
                    );
                    println!("        {}", v.message);
                }
                println!();
                println!(
                    "To rollback: skrills skill-rollback {} --version <hash>",
                    name
                );
            }
        }
    }

    Ok(())
}

/// Runs git in the skill file's directory.
fn git_in(skill_path: &Path, args: &[&str]) -> Result<std::process::Output> {
    let dir = skill_path
        .parent()
        .context("Skill has no parent directory")?;
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("Could not execute git for '{}'", skill_path.display()))
}

fn path_arg(skill_path: &Path) -> Result<&str> {
    skill_path
        .to_str()
        .with_context(|| format!("skill path is not UTF-8: {}", skill_path.display()))
}

/// The last ten commits that touched `skill_path`, newest first.
fn skill_history(skill_path: &Path) -> Result<Vec<SkillVersion>> {
    let output = git_in(
        skill_path,
        &[
            "log",
            "--pretty=format:%h|%ai|%s",
            "-n",
            "10",
            "--",
            path_arg(skill_path)?,
        ],
    )?;
    if !output.status.success() {
        bail!(
            "Git log failed for '{}': {}",
            skill_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(parse_git_log(&String::from_utf8_lossy(&output.stdout)))
}

/// Parses `%h|%ai|%s` lines; the subject may itself contain `|`.
fn parse_git_log(stdout: &str) -> Vec<SkillVersion> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '|');
            Some(SkillVersion {
                hash: parts.next()?.to_string(),
                date: parts.next()?.to_string(),
                message: parts.next()?.to_string(),
            })
        })
        .collect()
}

/// A version argument is an abbreviated or full hex commit id, which also
/// keeps option-like and shell-like strings away from `git checkout`.
fn is_valid_version_hash(version: &str) -> bool {
    (4..=40).contains(&version.len()) && version.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Restores `skill_path` to its content at `version`.
///
/// Refuses when the file has uncommitted changes, staged or not, because
/// `git checkout <rev> -- <file>` overwrites them with no way back.
fn checkout_version(skill_path: &Path, version: &str) -> Result<()> {
    if !is_valid_version_hash(version) {
        bail!(
            "Invalid version hash '{}'. Expected 4-40 hexadecimal characters (e.g., 'abc1234' or full SHA).",
            version
        );
    }

    let status = git_in(
        skill_path,
        &["status", "--porcelain", "--", path_arg(skill_path)?],
    )?;
    if !status.status.success() {
        bail!(
            "git status failed for '{}': {}",
            skill_path.display(),
            String::from_utf8_lossy(&status.stderr).trim()
        );
    }
    if !status.stdout.is_empty() {
        bail!(
            "'{}' has uncommitted changes; commit or stash them before rolling back",
            skill_path.display()
        );
    }

    let checkout = git_in(
        skill_path,
        &["checkout", version, "--", path_arg(skill_path)?],
    )?;
    if !checkout.status.success() {
        bail!(
            "Git checkout failed: {}",
            String::from_utf8_lossy(&checkout.stderr)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(repo: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(repo)
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    /// A repository with two commits of `alpha/SKILL.md`; returns the first
    /// commit's hash.
    fn repo_with_two_versions(repo: &Path) -> (PathBuf, String) {
        git(repo, &["init", "-q"]);
        let file = repo.join("alpha/SKILL.md");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "v1\n").unwrap();
        git(repo, &["add", "."]);
        git(repo, &["commit", "-q", "-m", "first | with pipe"]);
        let first = git(repo, &["rev-parse", "--short", "HEAD"]);
        std::fs::write(&file, "v2\n").unwrap();
        git(repo, &["commit", "-qam", "second"]);
        (file, first)
    }

    #[test]
    fn parse_git_log_keeps_pipes_in_the_subject_and_drops_malformed_lines() {
        let versions = parse_git_log(
            "abc1234|2024-01-15 10:30:00 -0500|fix: handle | in message\nno_pipes\ndef|only-one\n",
        );
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].hash, "abc1234");
        assert_eq!(versions[0].message, "fix: handle | in message");
    }

    #[test]
    fn version_hash_validation() {
        for ok in ["abcd", "abc1234", "ABCDEF", &"a".repeat(40)] {
            assert!(is_valid_version_hash(ok), "{ok}");
        }
        for bad in [
            "",
            "abc",
            "--help",
            "-",
            "abc123; echo pwned",
            "$(whoami)",
            &"a".repeat(41),
        ] {
            assert!(!is_valid_version_hash(bad), "{bad}");
        }
    }

    #[test]
    fn history_and_checkout_restore_an_earlier_version() {
        if !git_available() {
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        let (file, first) = repo_with_two_versions(repo.path());

        let history = skill_history(&file).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].message, "first | with pipe");

        checkout_version(&file, &first).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "v1\n");
    }

    /// SB-18: rollback overwrote uncommitted edits.
    #[test]
    fn checkout_refuses_a_file_with_uncommitted_changes() {
        if !git_available() {
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        let (file, first) = repo_with_two_versions(repo.path());
        std::fs::write(&file, "local edit\n").unwrap();

        let err = checkout_version(&file, &first).unwrap_err().to_string();

        assert!(err.contains("uncommitted changes"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "local edit\n");
    }

    #[test]
    fn rollback_result_serializes_available_versions() {
        let result = RollbackResult {
            skill_name: "s".to_string(),
            skill_path: PathBuf::from("/tmp/s/SKILL.md"),
            rolled_back: false,
            from_version: None,
            to_version: None,
            available_versions: parse_git_log("abc1|2024-01-01|m"),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["available_versions"][0]["hash"], "abc1");
    }
}
