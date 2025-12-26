use crate::cli::OutputFormat;
use crate::discovery::merge_extra_dirs;
use anyhow::Result;
use skrills_discovery::{discover_skills, extra_skill_roots};

/// Handle the `validate` command.
pub(crate) fn handle_validate_command(
    skill_dirs: Vec<std::path::PathBuf>,
    target: crate::cli::ValidationTarget,
    autofix: bool,
    backup: bool,
    format: OutputFormat,
    errors_only: bool,
) -> Result<()> {
    use skrills_validate::{
        validate_skill, AutofixOptions, ValidationSummary, ValidationTarget as VT,
    };

    let validation_target = match target {
        crate::cli::ValidationTarget::Claude => VT::Claude,
        crate::cli::ValidationTarget::Codex => VT::Codex,
        crate::cli::ValidationTarget::Both => VT::Both,
    };

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    if skills.is_empty() {
        println!("No skills found to validate.");
        return Ok(());
    }

    let mut results = Vec::new();
    let mut fixed_count = 0;
    let mut skipped_files: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut autofix_failures: Vec<(std::path::PathBuf, String)> = Vec::new();

    for meta in skills.iter() {
        let content = match std::fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(e) => {
                skipped_files.push((meta.path.clone(), e.to_string()));
                continue;
            }
        };

        let mut result = validate_skill(&meta.path, &content, validation_target);

        if autofix && !result.codex_valid && validation_target != VT::Claude {
            use skrills_validate::autofix_frontmatter;
            let opts = AutofixOptions {
                create_backup: backup,
                write_changes: true,
                suggested_name: Some(meta.name.clone()),
                suggested_description: None,
            };
            match autofix_frontmatter(&meta.path, &content, &opts) {
                Ok(fix_result) => {
                    if fix_result.modified {
                        fixed_count += 1;
                        let new_content = std::fs::read_to_string(&meta.path)?;
                        result = validate_skill(&meta.path, &new_content, validation_target);
                    }
                }
                Err(e) => {
                    autofix_failures.push((meta.path.clone(), e.to_string()));
                }
            }
        }

        if !errors_only || result.has_errors() {
            results.push(result);
        }
    }

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        let summary = ValidationSummary::from_results(&results);
        println!(
            "Validated {} skills: {} Claude-valid, {} Codex-valid, {} both-valid",
            summary.total, summary.claude_valid, summary.codex_valid, summary.both_valid
        );
        if fixed_count > 0 {
            println!("Auto-fixed {} skills", fixed_count);
        }
        if !autofix_failures.is_empty() {
            eprintln!(
                "\nWarning: {} skill(s) failed to auto-fix:",
                autofix_failures.len()
            );
            for (path, error) in &autofix_failures {
                eprintln!("  {}: {}", path.display(), error);
            }
        }
        if !skipped_files.is_empty() {
            eprintln!(
                "\nWarning: {} skill(s) could not be read:",
                skipped_files.len()
            );
            for (path, error) in &skipped_files {
                eprintln!("  {}: {}", path.display(), error);
            }
        }
        if summary.error_count > 0 {
            println!("\nErrors ({}):", summary.error_count);
            for result in &results {
                for issue in &result.issues {
                    if issue.severity == skrills_validate::Severity::Error {
                        let location = match issue.line {
                            Some(line) => format!("{}:{}", result.path.display(), line),
                            None => result.path.display().to_string(),
                        };
                        let target = match issue.target {
                            VT::Claude => "Claude",
                            VT::Codex => "Codex",
                            VT::Both => "Claude & Codex",
                        };
                        let suggestion = issue
                            .suggestion
                            .as_ref()
                            .map(|s| format!(" Suggestion: {}", s))
                            .unwrap_or_default();
                        println!(
                            "  {} ({}): {} [target: {}]{}",
                            result.name, location, issue.message, target, suggestion
                        );
                    }
                }
            }
        }
    }

    Ok(())
}
