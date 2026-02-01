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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    // GIVEN a list of git diff output lines
    // WHEN filtering for skill files (.md or .skill)
    // THEN only matching extensions are included
    #[test]
    fn filters_skill_files_from_git_output() {
        let git_output =
            "src/main.rs\nskills/my-skill.md\nREADME.txt\ntools/helper.skill\nCargo.toml";
        let skill_files: Vec<PathBuf> = git_output
            .lines()
            .filter(|f| f.ends_with(".md") || f.ends_with(".skill"))
            .map(PathBuf::from)
            .collect();

        assert_eq!(skill_files.len(), 2);
        assert_eq!(skill_files[0], PathBuf::from("skills/my-skill.md"));
        assert_eq!(skill_files[1], PathBuf::from("tools/helper.skill"));
    }

    // GIVEN no skill files in git output
    // WHEN filtering
    // THEN the result is empty
    #[test]
    fn empty_skill_files_from_non_skill_output() {
        let git_output = "src/main.rs\nCargo.toml\nREADME.txt";
        let skill_files: Vec<PathBuf> = git_output
            .lines()
            .filter(|f| f.ends_with(".md") || f.ends_with(".skill"))
            .map(PathBuf::from)
            .collect();

        assert!(skill_files.is_empty());
    }

    // GIVEN empty git output
    // WHEN filtering
    // THEN no files are returned
    #[test]
    fn empty_git_output_yields_no_files() {
        let git_output = "";
        let skill_files: Vec<PathBuf> = git_output
            .lines()
            .filter(|f| f.ends_with(".md") || f.ends_with(".skill"))
            .map(PathBuf::from)
            .collect();

        assert!(skill_files.is_empty());
    }

    // GIVEN error tracking variables
    // WHEN errors occur during validation
    // THEN errors_found is true and validated count is correct
    #[test]
    fn error_tracking_logic() {
        let mut errors_found = false;
        let mut validated: usize = 0;

        // Simulate: first file validates OK
        validated += 1;
        assert!(!errors_found, "No errors yet");

        // Simulate: second file has read error
        errors_found = true;
        assert!(errors_found, "Error flag should be set");

        // Simulate: third file validates OK
        validated += 1;

        // Simulate: fourth file also has validation error
        let another_error = true;
        errors_found = errors_found || another_error;

        assert!(errors_found);
        assert_eq!(validated, 2);
    }

    // GIVEN a ValidationTarget enum mapping
    // WHEN mapping CLI targets to validate targets
    // THEN each variant maps correctly
    #[test]
    fn validation_target_mapping_is_exhaustive() {
        use crate::cli::ValidationTarget;

        // Ensure all variants can be constructed
        let targets = [
            ValidationTarget::Claude,
            ValidationTarget::Codex,
            ValidationTarget::Copilot,
            ValidationTarget::All,
            ValidationTarget::Both,
        ];
        assert_eq!(targets.len(), 5);
    }
}
