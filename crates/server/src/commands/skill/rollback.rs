use anyhow::{bail, Context, Result};
use skrills_discovery::{discover_skills, extra_skill_roots};
use std::path::PathBuf;

use crate::cli::OutputFormat;
use crate::discovery::merge_extra_dirs;

use super::{RollbackResult, SkillVersion};

/// Handle the skill-rollback command.
pub(crate) fn handle_skill_rollback_command(
    name: String,
    version: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    use std::process::Command;

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    let skill = skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(&name) || s.path.to_string_lossy().contains(&name))
        .with_context(|| format!("Skill '{}' not found in discovered skills", name))?;

    let skill_path = &skill.path;

    let parent_dir = skill_path
        .parent()
        .with_context(|| "Skill has no parent directory")?;

    let git_log = Command::new("git")
        .args([
            "log",
            "--pretty=format:%h|%ai|%s",
            "-n",
            "10",
            "--",
            skill_path.to_str().unwrap_or(""),
        ])
        .current_dir(parent_dir)
        .output();

    let available_versions: Vec<SkillVersion> = match git_log {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '|').collect();
                if parts.len() == 3 {
                    Some(SkillVersion {
                        hash: parts[0].to_string(),
                        date: parts[1].to_string(),
                        message: parts[2].to_string(),
                    })
                } else {
                    None
                }
            })
            .collect(),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            bail!(
                "Git log failed for '{}': {}",
                skill_path.display(),
                stderr.trim()
            );
        }
        Err(e) => {
            bail!(
                "Could not execute git for '{}': {}",
                skill_path.display(),
                e
            );
        }
    };

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
            let hash_pattern =
                regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").expect("Invalid regex pattern");
            if !hash_pattern.is_match(&target_version) {
                bail!(
                    "Invalid version hash '{}'. Expected 4-40 hexadecimal characters (e.g., 'abc1234' or full SHA).",
                    target_version
                );
            }

            let checkout = Command::new("git")
                .args([
                    "checkout",
                    &target_version,
                    "--",
                    skill_path.to_str().unwrap_or(""),
                ])
                .current_dir(parent_dir)
                .output()
                .with_context(|| "Failed to execute git checkout")?;

            if !checkout.status.success() {
                bail!(
                    "Git checkout failed: {}",
                    String::from_utf8_lossy(&checkout.stderr)
                );
            }

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

#[cfg(test)]
mod tests {
    use super::super::{RollbackResult, SkillVersion};
    use std::path::PathBuf;

    // GIVEN git log output in the expected format
    // WHEN parsing lines with splitn
    // THEN SkillVersion structs are correctly constructed
    #[test]
    fn parse_git_log_line() {
        let line = "abc1234|2024-01-15 10:30:00 -0500|Initial commit";
        let parts: Vec<&str> = line.splitn(3, '|').collect();

        assert_eq!(parts.len(), 3);
        let version = SkillVersion {
            hash: parts[0].to_string(),
            date: parts[1].to_string(),
            message: parts[2].to_string(),
        };
        assert_eq!(version.hash, "abc1234");
        assert_eq!(version.date, "2024-01-15 10:30:00 -0500");
        assert_eq!(version.message, "Initial commit");
    }

    // GIVEN a git log line with pipe characters in the message
    // WHEN using splitn(3, '|')
    // THEN the message preserves internal pipes
    #[test]
    fn parse_git_log_with_pipe_in_message() {
        let line = "def5678|2024-02-20 14:00:00 +0000|fix: handle | in commit message";
        let parts: Vec<&str> = line.splitn(3, '|').collect();

        assert_eq!(parts.len(), 3);
        assert_eq!(parts[2], "fix: handle | in commit message");
    }

    // GIVEN a malformed git log line (missing fields)
    // WHEN parsing
    // THEN it should be filtered out
    #[test]
    fn malformed_git_log_filtered() {
        let lines = vec![
            "abc1234|2024-01-15|Good commit",
            "bad_line_no_pipes",
            "def5678|only_one_pipe",
        ];
        let versions: Vec<SkillVersion> = lines
            .into_iter()
            .filter_map(|line| {
                let parts: Vec<&str> = line.splitn(3, '|').collect();
                if parts.len() == 3 {
                    Some(SkillVersion {
                        hash: parts[0].to_string(),
                        date: parts[1].to_string(),
                        message: parts[2].to_string(),
                    })
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].hash, "abc1234");
    }

    // GIVEN valid git version hashes
    // WHEN validated against the pattern
    // THEN they are accepted
    #[test]
    fn valid_version_hashes_accepted() {
        let hash_pattern = regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").unwrap();
        let valid = [
            "abcd",
            "abc1234",
            "ABCDEF",
            "1234567890abcdef1234567890abcdef12345678",
        ];
        for h in &valid {
            assert!(hash_pattern.is_match(h), "Expected '{}' to be valid", h);
        }
    }

    // GIVEN invalid or malicious version hashes
    // WHEN validated against the pattern
    // THEN they are rejected
    #[test]
    fn invalid_version_hashes_rejected() {
        let hash_pattern = regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").unwrap();
        let invalid = [
            "abc",               // too short
            "",                  // empty
            "; rm -rf /",        // injection
            "abc123\necho hack", // newline injection
            "--help",            // flag injection
            "xyz123",            // invalid hex chars
        ];
        for h in &invalid {
            assert!(!hash_pattern.is_match(h), "Expected '{}' to be rejected", h);
        }
    }

    // GIVEN an empty available_versions list
    // WHEN building RollbackResult
    // THEN rolled_back is false
    #[test]
    fn rollback_result_no_history() {
        let result = RollbackResult {
            skill_name: "test".to_string(),
            skill_path: PathBuf::from("/tmp/test.md"),
            rolled_back: false,
            from_version: None,
            to_version: None,
            available_versions: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"rolled_back\":false"));
        assert!(json.contains("\"available_versions\":[]"));
    }

    // GIVEN a successful rollback
    // WHEN building RollbackResult
    // THEN from_version and to_version are populated
    #[test]
    fn rollback_result_success() {
        let result = RollbackResult {
            skill_name: "my-skill".to_string(),
            skill_path: PathBuf::from("/skills/my-skill.md"),
            rolled_back: true,
            from_version: Some("abc1234".to_string()),
            to_version: Some("def5678".to_string()),
            available_versions: vec![],
        };
        assert!(result.rolled_back);
        assert_eq!(result.from_version.as_deref(), Some("abc1234"));
        assert_eq!(result.to_version.as_deref(), Some("def5678"));
    }

    // GIVEN SkillVersion struct
    // WHEN serialized and deserialized
    // THEN round-trip preserves data
    #[test]
    fn skill_version_round_trip() {
        let version = SkillVersion {
            hash: "fedcba9".to_string(),
            date: "2024-03-01 09:00:00 +0100".to_string(),
            message: "chore: cleanup".to_string(),
        };
        let json = serde_json::to_string(&version).unwrap();
        let parsed: SkillVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.hash, version.hash);
        assert_eq!(parsed.date, version.date);
        assert_eq!(parsed.message, version.message);
    }
}
