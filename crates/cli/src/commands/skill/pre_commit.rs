use anyhow::{bail, Context, Result};
use skrills_discovery::discover_skills;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::ValidationTarget;

/// Handle the pre-commit-validate command.
pub(crate) fn handle_pre_commit_validate_command(
    staged: bool,
    target: ValidationTarget,
    skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    let skill_files = if staged {
        staged_skill_files(Path::new("."))?
    } else {
        let roots = crate::commands::skill_roots_for(&skill_dirs);
        discover_skills(&roots, None)?
            .into_iter()
            .map(|s| {
                let content = std::fs::read_to_string(&s.path).map_err(|e| e.to_string());
                (s.path, content)
            })
            .collect()
    };

    if skill_files.is_empty() {
        println!("No skill files to validate.");
        return Ok(());
    }

    let outcome = validate_files(&skill_files, target.into());
    for line in &outcome.failures {
        eprintln!("{line}");
    }
    if !outcome.failures.is_empty() {
        eprintln!();
        bail!("Validation failed. Fix errors before committing.");
    }

    println!(
        "✓ {} skill file(s) validated successfully",
        outcome.validated
    );
    Ok(())
}

/// Whether a path names a file the hook validates.
fn is_hook_skill_file(path: &str) -> bool {
    path.ends_with(".md") || path.ends_with(".skill")
}

/// The staged skill files in the repository that contains `dir`, with the
/// staged blob of each rather than the worktree copy.
///
/// Paths come from `git diff -z`, so a name git would otherwise quote (any
/// non-ASCII byte) arrives intact, and they are resolved against the
/// repository top level rather than the current directory. A blob that
/// cannot be read is returned as an error for the caller to report, never
/// dropped: the hook must not pass a file it did not look at.
fn staged_skill_files(dir: &Path) -> Result<Vec<(PathBuf, Result<String, String>)>> {
    let top = git_stdout(dir, &["rev-parse", "--show-toplevel"])?;
    let top = PathBuf::from(String::from_utf8_lossy(&top).trim_end_matches('\n'));

    let names = git_stdout(
        &top,
        &["diff", "--cached", "--name-only", "-z", "--diff-filter=ACM"],
    )?;

    let mut files = Vec::new();
    for raw in names.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let rel = String::from_utf8_lossy(raw).into_owned();
        if !is_hook_skill_file(&rel) {
            continue;
        }
        let content = git_stdout(&top, &["show", &format!(":{rel}")])
            .map_err(|e| e.to_string())
            .and_then(|blob| String::from_utf8(blob).map_err(|e| e.to_string()));
        files.push((top.join(&rel), content));
    }
    Ok(files)
}

fn git_stdout(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("Failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// What validating a set of skill files found.
#[derive(Debug, Default)]
struct HookOutcome {
    validated: usize,
    /// One human-readable line per failure, for stderr.
    failures: Vec<String>,
}

fn validate_files(
    files: &[(PathBuf, Result<String, String>)],
    target: skrills_validate::ValidationTarget,
) -> HookOutcome {
    let mut outcome = HookOutcome::default();
    for (path, content) in files {
        let content = match content {
            Ok(c) => c,
            Err(e) => {
                outcome
                    .failures
                    .push(format!("✗ {} (read error: {e})", path.display()));
                continue;
            }
        };
        let result = skrills_validate::validate_skill(path, content, target);
        if result.has_errors() {
            outcome.failures.push(format!("✗ {}", path.display()));
            for issue in &result.issues {
                if issue.severity == skrills_validate::Severity::Error {
                    outcome.failures.push(format!("  - {}", issue.message));
                }
            }
        } else {
            outcome.validated += 1;
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_validate::ValidationTarget as VT;

    const VALID: &str =
        "---\nname: good\ndescription: A skill that passes validation\n---\n# Good\n";
    const INVALID: &str = "no frontmatter at all\n";

    fn git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
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
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {args:?}");
    }

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    #[test]
    fn hook_skill_file_filter_keeps_md_and_skill() {
        assert!(is_hook_skill_file("skills/my-skill.md"));
        assert!(is_hook_skill_file("tools/helper.skill"));
        assert!(!is_hook_skill_file("src/main.rs"));
        assert!(!is_hook_skill_file("README.txt"));
    }

    #[test]
    fn validate_files_counts_valid_and_reports_invalid_and_unreadable() {
        let files = vec![
            (PathBuf::from("a/SKILL.md"), Ok(VALID.to_string())),
            (PathBuf::from("b/SKILL.md"), Ok(INVALID.to_string())),
            (PathBuf::from("c/SKILL.md"), Err("gone".to_string())),
        ];

        let outcome = validate_files(&files, VT::Codex);

        assert_eq!(outcome.validated, 1);
        assert!(outcome.failures.iter().any(|l| l.contains("b/SKILL.md")));
        assert!(outcome
            .failures
            .iter()
            .any(|l| l.contains("c/SKILL.md") && l.contains("read error")));
    }

    /// SB-17: a staged non-ASCII name came back quoted from git, failed
    /// `exists()`, and was skipped; and the worktree copy was validated
    /// instead of the staged blob.
    #[test]
    fn staged_files_use_the_staged_blob_and_survive_non_ascii_names() {
        if !git_available() {
            eprintln!("skipped: git is not on PATH");
            return;
        }
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &["init", "-q"]);
        let dir = repo.path().join("skïll");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("SKILL.md");
        std::fs::write(&file, INVALID).unwrap();
        git(repo.path(), &["add", "."]);
        // The worktree copy is fixed but the fix is not staged.
        std::fs::write(&file, VALID).unwrap();

        // Run from a subdirectory: paths must resolve from the top level.
        let files = staged_skill_files(&dir).unwrap();

        assert_eq!(files.len(), 1, "{files:?}");
        assert!(files[0].0.ends_with("skïll/SKILL.md"), "{:?}", files[0].0);
        assert_eq!(files[0].1.as_deref(), Ok(INVALID));
        let outcome = validate_files(&files, VT::Codex);
        assert_eq!(outcome.validated, 0);
        assert!(!outcome.failures.is_empty());
    }

    #[test]
    fn staged_files_outside_a_repository_is_an_error() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // GIT_CEILING_DIRECTORIES stops git walking up into an enclosing repo.
        let _g = skrills_test_utils::env_guard();
        let _c = skrills_test_utils::set_env_var(
            "GIT_CEILING_DIRECTORIES",
            Some(dir.path().parent().unwrap().to_str().unwrap()),
        );
        assert!(staged_skill_files(dir.path()).is_err());
    }
}
