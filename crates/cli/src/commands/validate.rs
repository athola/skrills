use crate::cli::OutputFormat;
use anyhow::{bail, Result};
use skrills_discovery::{discover_skills, SkillRoot};
use skrills_validate::{
    autofix_frontmatter, validate_skill, AutofixOptions, ValidationResult, ValidationSummary,
    ValidationTarget as VT,
};
use std::path::PathBuf;

/// How `run_validation` treats a skill that fails the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Autofix {
    /// Validate only.
    Off,
    /// Rewrite the file in place, with a backup when `backup` is set.
    Write { backup: bool },
    /// Compute the fix and validate the fixed text, writing nothing.
    Preview,
}

/// What validating a set of skill roots found.
#[derive(Debug, Default)]
pub(crate) struct ValidationRun {
    /// One result per readable skill, after any autofix.
    pub results: Vec<ValidationResult>,
    /// Skills the autofix changed (or would change, under `Autofix::Preview`).
    pub fixed: Vec<PathBuf>,
    /// Skills that could not be read.
    pub unreadable: Vec<(PathBuf, String)>,
    /// Skills the autofix could not repair.
    pub autofix_failures: Vec<(PathBuf, String)>,
}

impl ValidationRun {
    /// Whether anything is left that should stop a caller: an error-level
    /// issue, an unreadable skill, or a failed autofix.
    pub(crate) fn failed(&self) -> bool {
        self.results.iter().any(ValidationResult::has_errors)
            || !self.unreadable.is_empty()
            || !self.autofix_failures.is_empty()
    }

    /// Human-readable lines naming each failure, for stderr.
    pub(crate) fn failure_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for result in self.results.iter().filter(|r| r.has_errors()) {
            for issue in &result.issues {
                if issue.severity == skrills_validate::Severity::Error {
                    lines.push(format!(
                        "  {} ({}): {}",
                        result.name,
                        result.path.display(),
                        issue.message
                    ));
                }
            }
        }
        for (path, error) in &self.unreadable {
            lines.push(format!("  {}: could not be read: {error}", path.display()));
        }
        for (path, error) in &self.autofix_failures {
            lines.push(format!("  {}: autofix failed: {error}", path.display()));
        }
        lines
    }
}

/// Validates every skill under `roots` against `target`.
pub(crate) fn run_validation(
    roots: &[SkillRoot],
    target: VT,
    autofix: Autofix,
) -> Result<ValidationRun> {
    let skills = discover_skills(roots, None)?;
    let mut run = ValidationRun::default();

    for meta in &skills {
        let content = match std::fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(e) => {
                run.unreadable.push((meta.path.clone(), e.to_string()));
                continue;
            }
        };

        let mut result = validate_skill(&meta.path, &content, target);

        if autofix != Autofix::Off && !result.codex_valid && target != VT::Claude {
            let opts = AutofixOptions {
                create_backup: matches!(autofix, Autofix::Write { backup: true }),
                write_changes: matches!(autofix, Autofix::Write { .. }),
                // The directory name, not the discovery key `bad/SKILL.md`.
                suggested_name: Some(super::skill::skill_dir_name(meta)),
                suggested_description: None,
            };
            match autofix_frontmatter(&meta.path, &content, &opts) {
                Ok(fix) if fix.modified => {
                    run.fixed.push(meta.path.clone());
                    result = validate_skill(&meta.path, &fix.content, target);
                }
                Ok(_) => {}
                Err(e) => run.autofix_failures.push((meta.path.clone(), e)),
            }
        }

        run.results.push(result);
    }

    Ok(run)
}

/// Handle the `validate` command.
///
/// Exits non-zero when a skill has an error-level issue, cannot be read, or
/// could not be auto-fixed, so a script or CI step can gate on the exit code.
pub(crate) fn handle_validate_command(
    skill_dirs: Vec<std::path::PathBuf>,
    target: crate::cli::ValidationTarget,
    autofix: bool,
    backup: bool,
    format: OutputFormat,
    errors_only: bool,
) -> Result<()> {
    let validation_target: VT = target.into();
    let roots = crate::commands::skill_roots_for(&skill_dirs);
    let mode = if autofix {
        Autofix::Write { backup }
    } else {
        Autofix::Off
    };
    let run = run_validation(&roots, validation_target, mode)?;

    if run.results.is_empty() && run.unreadable.is_empty() {
        if format.is_json() {
            println!("[]");
        } else {
            println!("No skills found to validate.");
        }
        return Ok(());
    }

    let shown: Vec<&ValidationResult> = run
        .results
        .iter()
        .filter(|r| !errors_only || r.has_errors())
        .collect();

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&shown)?);
    } else {
        // The summary counts every validated skill, not only the rows
        // `--errors-only` keeps.
        let summary = ValidationSummary::from_results(&run.results);
        println!(
            "Validated {} skills: {} Claude-valid, {} Codex-valid, {} Copilot-valid, {} all-valid",
            summary.total,
            summary.claude_valid,
            summary.codex_valid,
            summary.copilot_valid,
            summary.all_valid
        );
        if !run.fixed.is_empty() {
            println!("Auto-fixed {} skills", run.fixed.len());
        }
        if summary.error_count > 0 {
            println!("\nErrors ({}):", summary.error_count);
            for result in &shown {
                for issue in &result.issues {
                    if issue.severity == skrills_validate::Severity::Error {
                        let location = match issue.line {
                            Some(line) => format!("{}:{}", result.path.display(), line),
                            None => result.path.display().to_string(),
                        };
                        let target = match issue.target {
                            VT::Claude => "Claude",
                            VT::Codex => "Codex",
                            VT::Copilot => "Copilot",
                            VT::Both => "Claude & Codex",
                            VT::All => "Claude, Codex & Copilot",
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

    // Unreadable files and failed fixes have no row in the JSON array, so
    // they are reported on stderr in both formats rather than dropped.
    if !run.autofix_failures.is_empty() {
        eprintln!(
            "\nWarning: {} skill(s) failed to auto-fix:",
            run.autofix_failures.len()
        );
        for (path, error) in &run.autofix_failures {
            eprintln!("  {}: {}", path.display(), error);
        }
    }
    if !run.unreadable.is_empty() {
        eprintln!(
            "\nWarning: {} skill(s) could not be read:",
            run.unreadable.len()
        );
        for (path, error) in &run.unreadable {
            eprintln!("  {}: {}", path.display(), error);
        }
    }

    if run.failed() {
        let errors = run.results.iter().filter(|r| r.has_errors()).count();
        bail!(
            "validation failed: {errors} skill(s) with errors, {} unreadable, {} autofix failure(s)",
            run.unreadable.len(),
            run.autofix_failures.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::SkillSource;

    fn root(dir: &std::path::Path) -> Vec<SkillRoot> {
        vec![SkillRoot {
            root: dir.to_path_buf(),
            source: SkillSource::Extra(0),
        }]
    }

    fn write_skill(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name).join("SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn a_skill_without_frontmatter_fails_codex_validation() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "bare", "# Bare\nNo frontmatter.\n");

        let run = run_validation(&root(dir.path()), VT::Codex, Autofix::Off).unwrap();

        assert!(run.failed());
        assert!(!run.failure_lines().is_empty());
    }

    #[test]
    fn preview_autofix_validates_the_fix_without_writing_it() {
        let dir = tempfile::tempdir().unwrap();
        let body = "# Bare\nNo frontmatter, but a body long enough to describe.\n";
        let path = write_skill(dir.path(), "bare", body);

        let run = run_validation(&root(dir.path()), VT::Codex, Autofix::Preview).unwrap();

        assert_eq!(run.fixed, vec![path.clone()]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
    }

    #[test]
    fn write_autofix_rewrites_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let body = "# Bare\nNo frontmatter, but a body long enough to describe.\n";
        let path = write_skill(dir.path(), "bare", body);

        let run = run_validation(
            &root(dir.path()),
            VT::Codex,
            Autofix::Write { backup: false },
        )
        .unwrap();

        assert_eq!(run.fixed, vec![path.clone()]);
        assert_ne!(std::fs::read_to_string(&path).unwrap(), body);
    }
}
