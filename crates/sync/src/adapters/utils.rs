//! Shared utility functions for agent adapters.

use crate::common::ModuleFile;
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::{debug, warn};
use walkdir::WalkDir;

/// Returns true if the name starts with a dot (hidden file/directory).
pub fn is_hidden_component(name: &str) -> bool {
    name.starts_with('.')
}

/// Returns true if any path component is hidden (starts with a dot).
pub fn is_hidden_path(path: &Path) -> bool {
    path.components().any(|c| match c {
        std::path::Component::Normal(s) => is_hidden_component(&s.to_string_lossy()),
        _ => false,
    })
}

/// Checks whether `target_path` stays within `base_dir` after resolution.
///
/// Returns `true` if the path is contained, `false` if it escapes (path traversal).
/// For files that don't exist yet, canonicalizes the base directory and checks
/// that the relative path has no `..` components that escape the base.
pub fn is_path_contained(target_path: &Path, base_dir: &Path) -> bool {
    // Fast path: if both exist on disk, use canonical resolution
    if let (Ok(resolved), Ok(canonical_base)) =
        (target_path.canonicalize(), base_dir.canonicalize())
    {
        return resolved.starts_with(&canonical_base);
    }

    // Target doesn't exist yet — try canonicalizing the parent chain
    let resolved = target_path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.join(target_path.file_name().unwrap_or_default()));

    if let Some(resolved) = resolved {
        let canonical_base = base_dir
            .canonicalize()
            .unwrap_or_else(|_| base_dir.to_path_buf());
        return resolved.starts_with(&canonical_base);
    }

    // Neither target nor its parent exist — check if base_dir exists and
    // verify that the relative path between them has no traversal
    let canonical_base = match base_dir.canonicalize() {
        Ok(b) => b,
        Err(_) => return false, // Base doesn't exist — deny
    };

    // If target_path starts with base_dir, strip the prefix and check for ..
    if let Ok(relative) = target_path.strip_prefix(base_dir) {
        // No component should be ".."
        return !relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    }

    // If target_path is absolute but doesn't start with base_dir, try the
    // canonical base
    if let Ok(relative) = target_path.strip_prefix(&canonical_base) {
        return !relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    }

    false // Cannot determine containment — deny (fail-closed)
}

/// Computes a SHA-256 hash of the given content, returning a lowercase hex string.
pub fn hash_content(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

/// Sanitizes a name by filtering to safe characters: alphanumeric, hyphens, and underscores.
///
/// Prevents path traversal by stripping dots, slashes, and other special characters.
/// Use `sanitize_name_segments` for names that may contain legitimate path separators.
pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Sanitizes a name while preserving forward-slash path segments.
///
/// Each segment is individually filtered to `[a-zA-Z0-9_-]`.
/// Empty segments and traversal segments (`.` and `..`) are removed.
pub fn sanitize_name_segments(name: &str) -> String {
    name.split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .map(|segment| {
            segment
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Collects companion files from a skill directory (files other than SKILL.md).
pub fn collect_module_files(skill_dir: &Path) -> Vec<ModuleFile> {
    let mut modules = Vec::new();

    for entry in WalkDir::new(skill_dir)
        .min_depth(1)
        .max_depth(10)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                debug!(
                    error = %e,
                    path = ?e.path(),
                    "Skipping directory entry due to traversal error"
                );
                continue;
            }
        };

        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        if path.file_name().is_some_and(|n| n == "SKILL.md") {
            continue;
        }

        if let Ok(rel_path) = path.strip_prefix(skill_dir) {
            if is_hidden_path(rel_path) {
                continue;
            }

            let content = match std::fs::read(path) {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "Skipping unreadable module file"
                    );
                    continue;
                }
            };
            let hash = hash_content(&content);
            modules.push(ModuleFile {
                relative_path: rel_path.to_path_buf(),
                content,
                hash,
            });
        }
    }

    modules
}

/// Sanitizes a name to kebab-case suitable for file/directory names.
///
/// Lowercases, maps `_`/` `/`.` to hyphens, strips other non-alphanumeric characters,
/// and trims leading/trailing hyphens. Used by the Cursor adapter since Cursor
/// conventions use kebab-case file names.
///
/// Note: a round-trip through a kebab-case adapter will normalize names
/// (e.g., `My_Skill` → `my-skill`), so the returned name may differ from the input.
pub fn sanitize_name_kebab(name: &str) -> String {
    let raw: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else if c == '_' || c == ' ' || c == '.' {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|&c| c != '\0')
        .collect();
    // Collapse consecutive hyphens and trim
    let mut result = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c == '-' && result.ends_with('-') {
            continue;
        }
        result.push(c);
    }
    result.trim_matches('-').to_string()
}

/// Test helper functions for adapter tests.
#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::common::Command;
    use std::path::PathBuf;
    use std::time::SystemTime;

    /// Create a Command with minimal boilerplate. Uses deterministic defaults
    /// for source_path, modified, and hash so tests are reproducible.
    pub fn make_command(name: &str, content: &str) -> Command {
        let mut cmd = Command::new(
            name.to_string(),
            content.as_bytes().to_vec(),
            PathBuf::from(format!("/test/{name}.md")),
        );
        cmd.modified = SystemTime::UNIX_EPOCH;
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hidden_component() {
        assert!(is_hidden_component(".git"));
        assert!(is_hidden_component(".hidden"));
        assert!(!is_hidden_component("visible"));
        assert!(!is_hidden_component(""));
    }

    #[test]
    fn test_is_hidden_path() {
        assert!(is_hidden_path(Path::new(".git/config")));
        assert!(is_hidden_path(Path::new("foo/.hidden/bar")));
        assert!(!is_hidden_path(Path::new("foo/bar/baz")));
        assert!(!is_hidden_path(Path::new("visible.txt")));
    }

    #[test]
    fn test_hash_content() {
        let hash = hash_content(b"hello");
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex chars
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sanitize_name_removes_path_traversal() {
        assert_eq!(sanitize_name("../../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_name("valid-name_123"), "valid-name_123");
        assert_eq!(sanitize_name("../../malicious"), "malicious");
        assert_eq!(sanitize_name("normal"), "normal");
        assert_eq!(sanitize_name("with spaces"), "withspaces");
        assert_eq!(sanitize_name("has/slashes"), "hasslashes");
    }

    #[test]
    fn sanitize_name_segments_handles_traversal() {
        assert_eq!(sanitize_name_segments("../../../etc/passwd"), "etc/passwd");
        assert_eq!(sanitize_name_segments("foo/../bar"), "foo/bar");
        assert_eq!(sanitize_name_segments("foo/./bar"), "foo/bar");
        assert_eq!(sanitize_name_segments(".."), "");
        assert_eq!(sanitize_name_segments("."), "");
        assert_eq!(
            sanitize_name_segments("category/my-skill"),
            "category/my-skill"
        );
        assert_eq!(sanitize_name_segments("foo///bar"), "foo/bar");
        assert_eq!(sanitize_name_segments("my skill!@#"), "myskill");
    }

    #[test]
    fn sanitize_name_kebab_converts_to_kebab_case() {
        assert_eq!(sanitize_name_kebab("My Skill Name"), "my-skill-name");
        assert_eq!(
            sanitize_name_kebab("skill_with_underscores"),
            "skill-with-underscores"
        );
        assert_eq!(sanitize_name_kebab("Already-Kebab"), "already-kebab");
        assert_eq!(sanitize_name_kebab("file.name.ext"), "file-name-ext");
        assert_eq!(sanitize_name_kebab("CLAUDE.md"), "claude-md");
        // Slashes are stripped (not alphanumeric, not in map)
        assert_eq!(sanitize_name_kebab("../../../etc/passwd"), "etcpasswd");
    }

    #[test]
    fn collect_module_files_from_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let modules = collect_module_files(dir.path());
        assert!(modules.is_empty());
    }

    #[test]
    fn collect_module_files_skips_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SKILL.md"), "main skill").unwrap();
        std::fs::write(dir.path().join("helper.py"), "# helper").unwrap();
        let modules = collect_module_files(dir.path());
        assert_eq!(modules.len(), 1);
        assert_eq!(
            modules[0].relative_path,
            std::path::PathBuf::from("helper.py")
        );
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn sanitize_name_never_contains_path_traversal(s in "\\PC*") {
                let sanitized = sanitize_name(&s);
                prop_assert!(!sanitized.contains(".."), "sanitized name contained ..: {}", sanitized);
                prop_assert!(!sanitized.contains('/'), "sanitized name contained /: {}", sanitized);
                prop_assert!(!sanitized.contains('\\'), "sanitized name contained \\: {}", sanitized);
            }

            #[test]
            fn sanitize_name_segments_never_contains_traversal(s in "\\PC*") {
                let sanitized = sanitize_name_segments(&s);
                // No segment should be ".." after sanitization
                for segment in sanitized.split('/') {
                    prop_assert!(segment != "..", "segment was ..: {}", sanitized);
                    prop_assert!(segment != ".", "segment was .: {}", sanitized);
                }
                // Should never start with "/" (no absolute paths from arbitrary input)
                prop_assert!(!sanitized.starts_with('/'), "produced absolute path: {}", sanitized);
            }

            #[test]
            fn sanitize_name_only_contains_safe_chars(s in "\\PC*") {
                let sanitized = sanitize_name(&s);
                for c in sanitized.chars() {
                    prop_assert!(
                        c.is_alphanumeric() || c == '-' || c == '_',
                        "unexpected char {:?} in sanitized name: {}", c, sanitized
                    );
                }
            }

            #[test]
            fn sanitize_name_kebab_never_contains_path_separators(s in "\\PC*") {
                let sanitized = sanitize_name_kebab(&s);
                prop_assert!(!sanitized.contains('/'), "kebab name contained /: {}", sanitized);
                prop_assert!(!sanitized.contains('\\'), "kebab name contained \\: {}", sanitized);
                prop_assert!(!sanitized.contains(".."), "kebab name contained ..: {}", sanitized);
                // ASCII characters should be lowercased (function uses to_ascii_lowercase)
                for c in sanitized.chars() {
                    if c.is_ascii() {
                        prop_assert!(
                            !c.is_ascii_uppercase(),
                            "ASCII char {:?} was not lowercased in: {}", c, sanitized
                        );
                    }
                }
                // No consecutive hyphens
                prop_assert!(
                    !sanitized.contains("--"),
                    "kebab name contained consecutive hyphens: {}", sanitized
                );
            }

            #[test]
            fn hash_content_never_panics(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
                let hash = hash_content(&data);
                prop_assert_eq!(hash.len(), 64, "hash was not 64 hex chars");
                prop_assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "hash contained non-hex chars");
            }
        }
    }
}
