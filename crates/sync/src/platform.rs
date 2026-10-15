//! The coding tools sync reads from and writes to.

use serde::{Deserialize, Serialize};

/// A tool skrills syncs. Its lowercase name (`claude`, `codex`, `copilot`,
/// `cursor`) is the spelling used on the command line, in MCP tool
/// arguments and in [`crate::SyncParams::from`].
///
/// ```
/// use skrills_sync::Platform;
///
/// let cursor: Platform = "Cursor".parse().unwrap();
/// assert_eq!(cursor, Platform::Cursor);
/// assert_eq!(cursor.as_str(), "cursor");
/// assert!("vscode".parse::<Platform>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Claude Code CLI.
    #[default]
    Claude,
    /// OpenAI Codex CLI.
    Codex,
    /// GitHub Copilot CLI.
    Copilot,
    /// Cursor IDE.
    Cursor,
}

impl Platform {
    /// Every platform, in a fixed order.
    pub const ALL: [Platform; 4] = [
        Platform::Claude,
        Platform::Codex,
        Platform::Copilot,
        Platform::Cursor,
    ];

    /// The lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
        }
    }

    /// The target a sync from `self` goes to when none is given:
    /// Claude to Codex, everything else to Claude.
    pub fn default_target(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex | Self::Copilot | Self::Cursor => Self::Claude,
        }
    }

    /// Every other platform, the targets of a sync with no `--to`.
    pub fn other_targets(self) -> Vec<Self> {
        Self::ALL.into_iter().filter(|p| *p != self).collect()
    }

    /// Whether this is Claude.
    pub fn is_claude(self) -> bool {
        self == Self::Claude
    }

    /// Whether this is Codex.
    pub fn is_codex(self) -> bool {
        self == Self::Codex
    }

    /// Whether this is Copilot.
    pub fn is_copilot(self) -> bool {
        self == Self::Copilot
    }

    /// Whether this is Cursor.
    pub fn is_cursor(self) -> bool {
        self == Self::Cursor
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name that is not one of [`Platform::ALL`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Unknown platform '{0}'. Use 'claude', 'codex', 'copilot', or 'cursor'")]
pub struct UnknownPlatform(pub String);

impl std::str::FromStr for Platform {
    type Err = UnknownPlatform;

    /// Case-insensitive, as the MCP tools always accepted.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str().eq_ignore_ascii_case(name))
            .ok_or_else(|| UnknownPlatform(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_through_from_str() {
        for platform in Platform::ALL {
            assert_eq!(platform.as_str().parse::<Platform>(), Ok(platform));
            assert_eq!(platform.to_string(), platform.as_str());
        }
        assert_eq!("CLAUDE".parse::<Platform>(), Ok(Platform::Claude));
        assert!("vscode".parse::<Platform>().is_err());
    }

    #[test]
    fn default_target_sends_claude_to_codex_and_the_rest_to_claude() {
        assert_eq!(Platform::Claude.default_target(), Platform::Codex);
        for p in [Platform::Codex, Platform::Copilot, Platform::Cursor] {
            assert_eq!(p.default_target(), Platform::Claude);
        }
    }

    #[test]
    fn other_targets_are_the_three_other_platforms() {
        for platform in Platform::ALL {
            let others = platform.other_targets();
            assert_eq!(others.len(), 3);
            assert!(!others.contains(&platform));
        }
    }

    #[test]
    fn predicates_match_only_their_own_platform() {
        for platform in Platform::ALL {
            assert_eq!(platform.is_claude(), platform == Platform::Claude);
            assert_eq!(platform.is_codex(), platform == Platform::Codex);
            assert_eq!(platform.is_copilot(), platform == Platform::Copilot);
            assert_eq!(platform.is_cursor(), platform == Platform::Cursor);
        }
    }

    #[test]
    fn serde_uses_the_lowercase_name() {
        assert_eq!(
            serde_json::to_string(&Platform::Copilot).unwrap(),
            "\"copilot\""
        );
    }
}
