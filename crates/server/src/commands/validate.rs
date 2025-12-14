use crate::discovery::merge_extra_dirs;
use anyhow::Result;
use skrills_discovery::{discover_skills, extra_skill_roots};

/// Handle the `validate` command.
pub(crate) fn handle_validate_command(
    skill_dirs: Vec<std::path::PathBuf>,
    target: crate::cli::ValidationTarget,
    autofix: bool,
    backup: bool,
    format: String,
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

    for meta in skills.iter() {
        let content = match std::fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(_) => continue,
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
            if let Ok(fix_result) = autofix_frontmatter(&meta.path, &content, &opts) {
                if fix_result.modified {
                    fixed_count += 1;
                    let new_content = std::fs::read_to_string(&meta.path)?;
                    result = validate_skill(&meta.path, &new_content, validation_target);
                }
            }
        }

        if !errors_only || result.has_errors() {
            results.push(result);
        }
    }

    if format == "json" {
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
        if summary.error_count > 0 {
            println!("\nErrors ({}):", summary.error_count);
            for result in &results {
                for issue in &result.issues {
                    if issue.severity == skrills_validate::Severity::Error {
                        println!(
                            "  {} ({}): {}",
                            result.name,
                            result.path.display(),
                            issue.message
                        );
                    }
                }
            }
        }
    }

    Ok(())
}
