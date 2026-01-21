//! Shared utility functions for agent adapters.

use sha2::{Digest, Sha256};
use std::path::Path;

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

/// Computes a SHA-256 hash of the given content, returning a lowercase hex string.
pub fn hash_content(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
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
}
