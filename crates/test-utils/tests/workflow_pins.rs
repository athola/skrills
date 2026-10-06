//! Supply-chain guard for the GitHub Actions workflows.
//!
//! A tag such as `@v2` can be moved by the action's owner, which changes what
//! runs in CI with no diff in this repository. Every third-party action is
//! therefore pinned to a full commit SHA, with the version in a trailing
//! comment for Dependabot (`.github/dependabot.yml`) to update.
//!
//! GitHub-owned `actions/*` and local `./` actions are exempt.

use std::fs;
use std::path::{Path, PathBuf};

fn workflows_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root exists above crates/test-utils")
        .join(".github")
        .join("workflows")
}

/// The action reference on a `uses:` line, without any trailing comment.
fn uses_target(line: &str) -> Option<&str> {
    let trimmed = line.trim_start().trim_start_matches("- ").trim_start();
    let rest = trimmed.strip_prefix("uses:")?;
    let rest = rest.split('#').next().unwrap_or("").trim();
    Some(rest.trim_matches(|c| c == '"' || c == '\''))
}

fn is_full_sha(reference: &str) -> bool {
    reference.len() == 40 && reference.bytes().all(|b| b.is_ascii_hexdigit())
}

#[test]
fn third_party_actions_are_pinned_to_a_commit_sha() {
    let mut unpinned = Vec::new();
    let mut checked = 0;
    let mut entries: Vec<_> = fs::read_dir(workflows_dir())
        .expect("read .github/workflows")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml"))
        .collect();
    entries.sort();

    for path in entries {
        let text = fs::read_to_string(&path).expect("read workflow");
        for (n, line) in text.lines().enumerate() {
            let Some(target) = uses_target(line) else {
                continue;
            };
            if target.starts_with("./") || target.starts_with("actions/") {
                continue;
            }
            checked += 1;
            let reference = target.rsplit_once('@').map_or("", |(_, r)| r);
            if !is_full_sha(reference) {
                unpinned.push(format!(
                    "{}:{}: {target}",
                    path.file_name().unwrap().to_string_lossy(),
                    n + 1
                ));
            }
        }
    }

    assert!(
        checked > 0,
        "found no third-party actions; is the path right?"
    );
    assert!(
        unpinned.is_empty(),
        "pin these actions to a 40-character commit SHA:\n  {}",
        unpinned.join("\n  ")
    );
}
