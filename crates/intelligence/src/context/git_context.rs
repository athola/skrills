//! Extract keywords from git commit history.

use anyhow::Result;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Extract keywords from recent git commit messages.
///
/// Analyzes the most recent commits in a git repository to identify frequently
/// mentioned keywords. This helps with intelligent skill recommendations based on
/// what the user has been working on.
///
/// # Arguments
/// * `root` - The root directory of the git repository
/// * `commit_limit` - Maximum number of commits to analyze
///
/// # Returns
/// A vector of up to 30 keywords sorted by frequency (most common first).
/// Keywords are normalized to lowercase and filtered to exclude:
/// - Words shorter than 3 characters
/// - Common stop words (the, and, for, etc.)
/// - Commit action words (add, fix, update, remove, etc.)
/// - Conventional commit prefixes (feat:, fix:, docs:, etc.)
///
/// # Errors
/// Returns an error if git is not available or the directory is not a git repository.
///
/// # Example
/// ```no_run
/// use std::path::Path;
/// use skrills_intelligence::context::extract_git_keywords;
///
/// let keywords = extract_git_keywords(Path::new("."), 50)?;
/// // keywords might contain: ["authentication", "database", "api", ...]
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn extract_git_keywords(root: &Path, commit_limit: usize) -> Result<Vec<String>> {
    let stdout = run_git_log("git".as_ref(), root, commit_limit, GIT_LOG_TIMEOUT)?;
    Ok(extract_keywords_from_commits(&stdout))
}

/// How long `git log` may run before it is killed.
const GIT_LOG_TIMEOUT: Duration = Duration::from_secs(10);

/// Run `git log` for commit subjects in `root`, killing it after `timeout`.
///
/// `root` may be any repository a caller names, so the invocation does not
/// trust its config: no pager, no fsmonitor hook, and no signature
/// verification (which would run the repository's `gpg.program`).
fn run_git_log(
    git: &std::ffi::OsStr,
    root: &Path,
    commit_limit: usize,
    timeout: Duration,
) -> Result<String> {
    let mut child = Command::new(git)
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "log.showSignature=false",
            "log",
            "--no-show-signature",
            "-n",
            &commit_limit.to_string(),
            "--format=%s",
        ])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Drain both pipes on their own threads so a large log cannot fill a
    // pipe buffer and stall the child while we wait on it.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(crate::IntelligenceError::GitLogFailed(format!(
                "git log timed out after {}s",
                timeout.as_secs_f64()
            ))
            .into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(crate::IntelligenceError::GitLogFailed(
            String::from_utf8_lossy(&stderr).into_owned(),
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// Extract meaningful keywords from commit messages.
fn extract_keywords_from_commits(commits: &str) -> Vec<String> {
    let mut word_counts: HashMap<String, usize> = HashMap::new();

    for line in commits.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Skip conventional commit prefixes
        let line = strip_conventional_prefix(line);

        // Extract words
        for word in line.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
            let word = word.to_lowercase();
            if word.len() >= 3 && !is_commit_stop_word(&word) {
                *word_counts.entry(word).or_insert(0) += 1;
            }
        }
    }

    // Sort by frequency and take top keywords
    let mut keywords: Vec<_> = word_counts.into_iter().collect();
    keywords.sort_by_key(|b| std::cmp::Reverse(b.1));

    keywords
        .into_iter()
        .take(30)
        .map(|(word, _)| word)
        .collect()
}

/// Strip conventional commit prefixes like "feat:", "fix:", etc.
fn strip_conventional_prefix(line: &str) -> &str {
    const PREFIXES: &[&str] = &[
        "feat:",
        "fix:",
        "docs:",
        "style:",
        "refactor:",
        "perf:",
        "test:",
        "build:",
        "ci:",
        "chore:",
        "revert:",
        "feat(",
        "fix(",
        "docs(",
        "style(",
        "refactor(",
        "perf(",
        "test(",
        "build(",
        "ci(",
        "chore(",
        "revert(",
    ];

    for prefix in PREFIXES {
        if line.to_lowercase().starts_with(prefix) {
            let rest = &line[prefix.len()..];
            // Handle scope like "feat(scope): message"
            if let Some(idx) = rest.find("):") {
                return rest[idx + 2..].trim_start();
            }
            return rest.trim_start_matches(':').trim();
        }
    }

    line
}

/// Check if a word is a common stop word in commit messages.
fn is_commit_stop_word(word: &str) -> bool {
    const STOP_WORDS: &[&str] = &[
        // Common words
        "the",
        "and",
        "for",
        "that",
        "this",
        "with",
        "are",
        "was",
        "were",
        "been",
        "have",
        "has",
        "had",
        "not",
        "but",
        "can",
        "could",
        "would",
        "should",
        "may",
        "might",
        "will",
        "shall",
        "from",
        "into",
        "about",
        "than",
        "then",
        "when",
        "where",
        "what",
        "which",
        "who",
        "how",
        "all",
        "each",
        "every",
        "both",
        "few",
        "more",
        "most",
        "other",
        "some",
        "such",
        "only",
        "same",
        "just",
        "also",
        "very",
        "even",
        "back",
        "after",
        "before",
        "between",
        "now",
        "new",
        "use",
        "using",
        "used",
        // Commit-specific words
        "add",
        "added",
        "adding",
        "update",
        "updated",
        "updates",
        "updating",
        "fix",
        "fixed",
        "fixes",
        "fixing",
        "remove",
        "removed",
        "removes",
        "removing",
        "change",
        "changed",
        "changes",
        "changing",
        "move",
        "moved",
        "moves",
        "moving",
        "rename",
        "renamed",
        "renames",
        "renaming",
        "refactor",
        "refactored",
        "refactors",
        "refactoring",
        "merge",
        "merged",
        "merges",
        "merging",
        "bump",
        "bumped",
        "bumps",
        "version",
        "release",
        "released",
        "wip",
        "todo",
        "tmp",
        "temp",
    ];

    STOP_WORDS.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a fake `git` that records its arguments to `<dir>/args`, then
    /// either prints one subject or hangs.
    #[cfg(unix)]
    fn fake_git(dir: &Path, hang: bool) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let tail = if hang {
            "exec sleep 30"
        } else {
            "echo 'refactor authentication layer'"
        };
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n{tail}\n",
            dir.join("args").display()
        );
        let git = dir.join(if hang { "git-hang" } else { "git" });
        std::fs::write(&git, script).unwrap();
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
        git
    }

    /// IN-26: a hung `git log` is killed after the timeout, and the
    /// invocation disables the repository-config hooks it does not need.
    /// The fake binary is passed by path, so PATH is never touched.
    #[cfg(unix)]
    #[test]
    fn git_log_times_out_and_runs_hardened() {
        let bin = tempfile::tempdir().unwrap();

        let git = fake_git(bin.path(), false);
        let out = run_git_log(git.as_os_str(), bin.path(), 5, Duration::from_secs(10)).unwrap();
        assert_eq!(out.trim(), "refactor authentication layer");
        let args = std::fs::read_to_string(bin.path().join("args")).unwrap();
        for expected in [
            "--no-pager",
            "core.fsmonitor=false",
            "log.showSignature=false",
            "--no-show-signature",
        ] {
            assert!(
                args.lines().any(|a| a == expected),
                "missing {expected}: {args}"
            );
        }

        let hanging = fake_git(bin.path(), true);
        let started = Instant::now();
        let err = run_git_log(
            hanging.as_os_str(),
            bin.path(),
            5,
            Duration::from_millis(300),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("timed out"), "unexpected error: {err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn test_strip_conventional_prefix() {
        assert_eq!(
            strip_conventional_prefix("feat: add new feature"),
            "add new feature"
        );
        assert_eq!(
            strip_conventional_prefix("fix(auth): resolve login issue"),
            "resolve login issue"
        );
        assert_eq!(
            strip_conventional_prefix("regular commit message"),
            "regular commit message"
        );
    }

    #[test]
    fn test_extract_keywords() {
        let commits = r#"feat: implement user authentication
fix: resolve database connection issue
docs: update API documentation
feat(api): add new endpoint for users
chore: update dependencies"#;

        let keywords = extract_keywords_from_commits(commits);

        // Should contain meaningful words, not stop words
        assert!(
            keywords.contains(&"implement".to_string())
                || keywords.contains(&"authentication".to_string())
                || keywords.contains(&"database".to_string())
                || keywords.contains(&"api".to_string())
        );

        // Should not contain stop words
        assert!(!keywords.contains(&"the".to_string()));
        assert!(!keywords.contains(&"add".to_string())); // commit stop word
    }

    #[test]
    fn test_is_commit_stop_word() {
        assert!(is_commit_stop_word("add"));
        assert!(is_commit_stop_word("update"));
        assert!(is_commit_stop_word("the"));
        assert!(!is_commit_stop_word("authentication"));
        assert!(!is_commit_stop_word("database"));
    }
}
