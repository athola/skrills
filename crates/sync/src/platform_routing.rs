//! Platform-routing helpers for sync operations.

/// Returns the default target platform for a given source.
///
/// Used when a sync tool needs to infer the target from only the source name.
#[must_use]
pub fn default_target_for(from: &str) -> &'static str {
    match from {
        "claude" => "codex",
        "codex" => "claude",
        "copilot" => "claude",
        "cursor" => "claude",
        _ => "codex",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_target_for_all_platforms() {
        assert_eq!(default_target_for("claude"), "codex");
        assert_eq!(default_target_for("codex"), "claude");
        assert_eq!(default_target_for("copilot"), "claude");
        assert_eq!(default_target_for("cursor"), "claude");
        assert_eq!(default_target_for("unknown"), "codex");
    }
}
