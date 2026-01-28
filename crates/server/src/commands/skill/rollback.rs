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
