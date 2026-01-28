use anyhow::{bail, Context, Result};
use skrills_discovery::{discover_skills, extra_skill_roots};
use std::path::PathBuf;

use crate::cli::ValidationTarget;
use crate::discovery::merge_extra_dirs;

/// Handle the pre-commit-validate command.
pub(crate) fn handle_pre_commit_validate_command(
    staged: bool,
    target: ValidationTarget,
    skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    use skrills_validate::{validate_skill, ValidationTarget as VT};
    use std::process::Command;

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);

    let validation_target = match target {
        ValidationTarget::Claude => VT::Claude,
        ValidationTarget::Codex => VT::Codex,
        ValidationTarget::Copilot => VT::Copilot,
        ValidationTarget::All => VT::All,
        ValidationTarget::Both => VT::Both,
    };

    let skill_files: Vec<PathBuf> = if staged {
        let output = Command::new("git")
            .args(["diff", "--cached", "--name-only", "--diff-filter=ACM"])
            .output()
            .with_context(|| "Failed to run git diff")?;

        if !output.status.success() {
            bail!(
                "Git command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|f| f.ends_with(".md") || f.ends_with(".skill"))
            .map(PathBuf::from)
            .collect()
    } else {
        discover_skills(&roots, None)?
            .into_iter()
            .map(|s| s.path)
            .collect()
    };

    if skill_files.is_empty() {
        println!("No skill files to validate.");
        return Ok(());
    }

    let mut errors_found = false;
    let mut validated = 0;

    for path in &skill_files {
        if !path.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                errors_found = true;
                eprintln!("✗ {} (read error: {})", path.display(), e);
                continue;
            }
        };

        let result = validate_skill(path, &content, validation_target);

        if result.has_errors() {
            errors_found = true;
            eprintln!("✗ {}", path.display());
            for issue in &result.issues {
                if issue.severity == skrills_validate::Severity::Error {
                    eprintln!("  - {}", issue.message);
                }
            }
        } else {
            validated += 1;
        }
    }

    if errors_found {
        eprintln!();
        eprintln!("Validation failed. Fix errors before committing.");
        std::process::exit(1);
    }

    println!("✓ {} skill file(s) validated successfully", validated);
    Ok(())
}
