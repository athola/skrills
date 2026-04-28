//! Legacy platform-routing types (v0.7.0 deprecation locality).
//!
//! Houses the deprecated [`SyncDirection`] enum and its companion
//! [`parse_direction`] helper. The non-deprecated replacement is the
//! string-based [`crate::orchestrator::sync_between`] paired with
//! [`default_target_for`]. Co-located here so the deprecated surface
//! is a single grep target — once the deprecated items are removed in
//! a future release, this file can be deleted in one move.
//!
//! Why the migration: each new platform added 2 enum variants and
//! 2 match arms in callers; switching to string platform names made
//! that linear in adapters rather than quadratic in pairs.

use crate::Result;
use anyhow::bail;
use serde::{Deserialize, Serialize};

/// Source platform for sync operation.
///
/// **Deprecated**: Use [`crate::orchestrator::sync_between`] with
/// string platform names instead. This enum grows quadratically with
/// the number of platforms and will be removed in a future release.
#[deprecated(
    since = "0.7.0",
    note = "Use sync_between(from, to, params) with string platform names instead"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncDirection {
    /// Claude Code → Codex CLI.
    ClaudeToCodex,
    /// Codex CLI → Claude Code.
    CodexToClaude,
    /// Claude Code → GitHub Copilot.
    ClaudeToCopilot,
    /// GitHub Copilot → Claude Code.
    CopilotToClaude,
    /// Claude Code → Cursor.
    ClaudeToCursor,
    /// Cursor → Claude Code.
    CursorToClaude,
}

/// Determines sync direction from string input (legacy API).
///
/// **Deprecated**: Use [`crate::orchestrator::sync_between`] with
/// [`default_target_for`] instead.
///
/// ```
/// #[allow(deprecated)]
/// use skrills_sync::{parse_direction, SyncDirection};
///
/// #[allow(deprecated)]
/// {
///     assert_eq!(parse_direction("claude").unwrap(), SyncDirection::ClaudeToCodex);
///     assert_eq!(parse_direction("codex").unwrap(), SyncDirection::CodexToClaude);
///     assert!(parse_direction("invalid").is_err());
/// }
/// ```
#[deprecated(
    since = "0.7.0",
    note = "Use sync_between(from, to, params) with default_target_for(from) instead"
)]
#[allow(deprecated)]
pub fn parse_direction(from: &str) -> Result<SyncDirection> {
    match from.to_lowercase().as_str() {
        "claude" => Ok(SyncDirection::ClaudeToCodex),
        "codex" => Ok(SyncDirection::CodexToClaude),
        "copilot" => Ok(SyncDirection::CopilotToClaude),
        "cursor" => Ok(SyncDirection::CursorToClaude),
        _ => bail!(
            "Unknown source '{}'. Use 'claude', 'codex', 'copilot', or 'cursor'",
            from
        ),
    }
}

/// Returns the default target platform for a given source.
///
/// Used when a sync tool needs to infer the target from only the source name.
/// Not deprecated — this is the recommended companion to `sync_between`.
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
    #[allow(deprecated)]
    fn parse_direction_claude() {
        let dir = parse_direction("claude").unwrap();
        assert_eq!(dir, SyncDirection::ClaudeToCodex);
    }

    #[test]
    #[allow(deprecated)]
    fn parse_direction_codex() {
        let dir = parse_direction("codex").unwrap();
        assert_eq!(dir, SyncDirection::CodexToClaude);
    }

    #[test]
    #[allow(deprecated)]
    fn parse_direction_cursor() {
        let dir = parse_direction("cursor").unwrap();
        assert_eq!(dir, SyncDirection::CursorToClaude);
    }

    #[test]
    #[allow(deprecated)]
    fn parse_direction_invalid() {
        let result = parse_direction("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn default_target_for_all_platforms() {
        assert_eq!(default_target_for("claude"), "codex");
        assert_eq!(default_target_for("codex"), "claude");
        assert_eq!(default_target_for("copilot"), "claude");
        assert_eq!(default_target_for("cursor"), "claude");
        assert_eq!(default_target_for("unknown"), "codex");
    }
}
