//! Per-item accept/reject decisions for an interactive sync.
//!
//! The orchestrator asks an [`ItemDecider`] about every item a run would
//! create or change, before any writer runs. The CLI answers with prompts;
//! tests answer with closures, so no terminal is needed.

use crate::Result;
use std::fmt;

/// The kind of artifact a decision is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Command,
    Skill,
    Agent,
    Hook,
    Instruction,
    McpServer,
    Preferences,
    PluginAssets,
}

impl fmt::Display for ItemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Command => "command",
            Self::Skill => "skill",
            Self::Agent => "agent",
            Self::Hook => "hook",
            Self::Instruction => "instruction",
            Self::McpServer => "MCP server",
            Self::Preferences => "preference",
            Self::PluginAssets => "plugin assets",
        })
    }
}

impl ItemKind {
    /// The label for a prompt that covers every item of this kind.
    pub fn plural(self) -> &'static str {
        match self {
            Self::Command => "commands",
            Self::Skill => "skills",
            Self::Agent => "agents",
            Self::Hook => "hooks",
            Self::Instruction => "instructions",
            Self::McpServer => "MCP servers",
            Self::Preferences => "preferences",
            Self::PluginAssets => "plugin assets",
        }
    }
}

/// How the target holds an item the run would write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// The target has no copy.
    New,
    /// The target has a copy that differs from the source.
    Changed,
}

/// Whether a prompt covers one item or every item of a category.
///
/// A category is asked about as a whole when its target writer does not
/// give back the bytes it read, so keeping the target's copy cannot be
/// expressed item by item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecisionScope {
    PerItem,
    #[default]
    WholeCategory,
}

/// One question for the decider.
///
/// Marked non-exhaustive so later fields do not break deciders that only
/// read it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ItemChange {
    pub kind: ItemKind,
    /// The item's name; for a whole-category prompt, the category label.
    pub name: String,
    pub change: ChangeKind,
    pub scope: DecisionScope,
    /// Unified diff from the target's copy to the source's for a per-item
    /// prompt (everything is an insertion for a new item); for a
    /// whole-category prompt, the names of the items that would change.
    pub diff: String,
    /// Both sides' text when the target's copy and the source's copy each
    /// changed since the last sync. Only set for a per-item prompt.
    pub conflict: Option<ConflictSides>,
    /// For a whole-category prompt, how many of its items are in conflict.
    pub conflicting_items: usize,
}

/// The two texts of a conflicting item, for a side-by-side view.
///
/// Secret values (MCP server environment and headers) are already masked, as
/// in [`ItemChange::diff`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSides {
    pub target: String,
    pub source: String,
}

impl ItemChange {
    /// A question about one item.
    pub fn per_item(
        kind: ItemKind,
        name: impl Into<String>,
        change: ChangeKind,
        diff: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            change,
            scope: DecisionScope::PerItem,
            diff: diff.into(),
            conflict: None,
            conflicting_items: 0,
        }
    }
}

/// The answer to an [`ItemChange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Reject,
}

/// Answers whether one change may be written.
///
/// Return `Err(SyncError::InteractiveAborted)` to stop the run with nothing
/// written; any other error also stops it.
pub trait ItemDecider: Send + Sync {
    fn decide(&self, change: &ItemChange) -> Result<Decision>;
}

impl<F> ItemDecider for F
where
    F: Fn(&ItemChange) -> Result<Decision> + Send + Sync,
{
    fn decide(&self, change: &ItemChange) -> Result<Decision> {
        self(change)
    }
}
