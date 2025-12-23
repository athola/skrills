//! Extract keywords from git commit history.

use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Extract keywords from recent git commit messages.
pub fn extract_git_keywords(root: &Path, commit_limit: usize) -> Result<Vec<String>> {
    // Run git log to get recent commit messages
    let output = Command::new("git")
        .args([
            "log",
            "--oneline",
            "-n",
            &commit_limit.to_string(),
            "--format=%s",
        ])
        .current_dir(root)
        .output()?;

    if !output.status.success() {
        anyhow::bail!("git log failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let keywords = extract_keywords_from_commits(&stdout);

    Ok(keywords)
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
    keywords.sort_by(|a, b| b.1.cmp(&a.1));

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
