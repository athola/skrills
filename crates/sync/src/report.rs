//! Sync reporting types for tracking what was synced and what was skipped.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Reasons why an item was skipped during sync.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SkipReason {
    /// Field exists in source but not supported by target agent
    UnsupportedField {
        field: String,
        source_agent: String,
        suggestion: String,
    },
    /// References a path that doesn't exist
    PathNotFound { path: PathBuf, context: String },
    /// Uses agent-specific features
    AgentSpecificFeature {
        item: String,
        feature: String,
        suggestion: String,
    },
    /// Excluded by user configuration
    ExcludedByConfig { item: String, pattern: String },
    /// Hash unchanged, no sync needed
    Unchanged { item: String },
    /// Parse error in source file
    ParseError { item: String, error: String },
    /// Would overwrite an existing item on the target side
    WouldOverwrite { item: String },
}

#[allow(dead_code)]
impl SkipReason {
    /// Returns a human-readable description of the skip reason.
    pub fn description(&self) -> String {
        match self {
            Self::UnsupportedField {
                field,
                source_agent,
                ..
            } => {
                format!(
                    "Field '{}' from {} not supported by target",
                    field, source_agent
                )
            }
            Self::PathNotFound { path, .. } => {
                format!("Path not found: {}", path.display())
            }
            Self::AgentSpecificFeature { item, feature, .. } => {
                format!("{} uses agent-specific feature: {}", item, feature)
            }
            Self::ExcludedByConfig { item, pattern } => {
                format!("{} excluded by pattern: {}", item, pattern)
            }
            Self::Unchanged { item } => {
                format!("{} unchanged (same hash)", item)
            }
            Self::ParseError { item, error } => {
                format!("Failed to parse {}: {}", item, error)
            }
            Self::WouldOverwrite { item } => {
                format!("{} already exists on target (would overwrite)", item)
            }
        }
    }

    /// Returns actionable guidance for the user.
    pub fn guidance(&self) -> Option<&str> {
        match self {
            Self::UnsupportedField { suggestion, .. } => Some(suggestion),
            Self::PathNotFound { .. } => Some("Update path in source config or exclude this item"),
            Self::AgentSpecificFeature { suggestion, .. } => Some(suggestion),
            Self::ExcludedByConfig { .. } => Some("Intentional exclusion, no action needed"),
            Self::Unchanged { .. } => None,
            Self::ParseError { .. } => Some("Fix the source file syntax"),
            Self::WouldOverwrite { .. } => Some("Use --skip-existing-commands to keep target copy"),
        }
    }
}

/// Report for a write operation on a single artifact type.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WriteReport {
    /// Number of items successfully written
    pub written: usize,
    /// Items that were skipped with reasons
    pub skipped: Vec<SkipReason>,
    /// Non-fatal warnings
    pub warnings: Vec<String>,
}

/// Complete sync report across all artifact types.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub skills: WriteReport,
    pub commands: WriteReport,
    pub mcp_servers: WriteReport,
    pub preferences: WriteReport,
    /// Overall success status
    pub success: bool,
    /// Summary message
    pub summary: String,
}

#[allow(dead_code)]
impl SyncReport {
    /// Creates a new empty report.
    pub fn new() -> Self {
        Self {
            success: true,
            ..Default::default()
        }
    }

    /// Returns total items synced across all types.
    pub fn total_synced(&self) -> usize {
        self.skills.written
            + self.commands.written
            + self.mcp_servers.written
            + self.preferences.written
    }

    /// Returns total items skipped across all types.
    pub fn total_skipped(&self) -> usize {
        self.skills.skipped.len()
            + self.commands.skipped.len()
            + self.mcp_servers.skipped.len()
            + self.preferences.skipped.len()
    }

    /// Generates a formatted summary for display.
    pub fn format_summary(&self, source: &str, target: &str) -> String {
        let mut out = String::new();
        out.push_str(&format!("Sync Complete: {} → {}\n", source, target));
        out.push_str(&format!(
            "  Skills:      {} synced, {} skipped\n",
            self.skills.written,
            self.skills.skipped.len()
        ));
        out.push_str(&format!(
            "  Commands:    {} synced, {} skipped\n",
            self.commands.written,
            self.commands.skipped.len()
        ));
        out.push_str(&format!(
            "  MCP Servers: {} synced, {} skipped\n",
            self.mcp_servers.written,
            self.mcp_servers.skipped.len()
        ));
        out.push_str(&format!(
            "  Preferences: {} synced, {} skipped\n",
            self.preferences.written,
            self.preferences.skipped.len()
        ));
        out
    }
}
