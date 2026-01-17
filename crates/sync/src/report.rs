//! Sync reporting types for tracking what was synced and what was skipped.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Reasons why an item was skipped during sync.
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[must_use]
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

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================
    // SkipReason Tests (BDD style)
    // ==========================================

    mod skip_reason {
        use super::*;

        #[test]
        fn given_unsupported_field_when_description_then_includes_field_and_source() {
            let reason = SkipReason::UnsupportedField {
                field: "hooks".to_string(),
                source_agent: "codex".to_string(),
                suggestion: "Remove hooks field".to_string(),
            };

            let desc = reason.description();
            assert!(desc.contains("hooks"));
            assert!(desc.contains("codex"));
        }

        #[test]
        fn given_path_not_found_when_description_then_includes_path() {
            let reason = SkipReason::PathNotFound {
                path: PathBuf::from("/some/missing/path.md"),
                context: "skill reference".to_string(),
            };

            let desc = reason.description();
            assert!(desc.contains("/some/missing/path.md"));
        }

        #[test]
        fn given_agent_specific_when_description_then_includes_item_and_feature() {
            let reason = SkipReason::AgentSpecificFeature {
                item: "my-skill".to_string(),
                feature: "codex-only-field".to_string(),
                suggestion: "Remove or adapt".to_string(),
            };

            let desc = reason.description();
            assert!(desc.contains("my-skill"));
            assert!(desc.contains("codex-only-field"));
        }

        #[test]
        fn given_excluded_by_config_when_description_then_includes_pattern() {
            let reason = SkipReason::ExcludedByConfig {
                item: "test-skill".to_string(),
                pattern: "*.test".to_string(),
            };

            let desc = reason.description();
            assert!(desc.contains("test-skill"));
            assert!(desc.contains("*.test"));
        }

        #[test]
        fn given_unchanged_when_description_then_includes_item() {
            let reason = SkipReason::Unchanged {
                item: "stable-skill".to_string(),
            };

            let desc = reason.description();
            assert!(desc.contains("stable-skill"));
            assert!(desc.contains("unchanged") || desc.contains("hash"));
        }

        #[test]
        fn given_parse_error_when_description_then_includes_item_and_error() {
            let reason = SkipReason::ParseError {
                item: "broken-skill".to_string(),
                error: "invalid YAML".to_string(),
            };

            let desc = reason.description();
            assert!(desc.contains("broken-skill"));
            assert!(desc.contains("invalid YAML"));
        }

        #[test]
        fn given_would_overwrite_when_description_then_includes_item() {
            let reason = SkipReason::WouldOverwrite {
                item: "existing-cmd".to_string(),
            };

            let desc = reason.description();
            assert!(desc.contains("existing-cmd"));
            assert!(desc.contains("exists") || desc.contains("overwrite"));
        }

        #[test]
        fn given_unsupported_field_when_guidance_then_returns_suggestion() {
            let reason = SkipReason::UnsupportedField {
                field: "hooks".to_string(),
                source_agent: "codex".to_string(),
                suggestion: "Remove hooks".to_string(),
            };

            assert_eq!(reason.guidance(), Some("Remove hooks"));
        }

        #[test]
        fn given_unchanged_when_guidance_then_returns_none() {
            let reason = SkipReason::Unchanged {
                item: "skill".to_string(),
            };

            assert!(reason.guidance().is_none());
        }

        #[test]
        fn given_would_overwrite_when_guidance_then_returns_skip_flag_hint() {
            let reason = SkipReason::WouldOverwrite {
                item: "cmd".to_string(),
            };

            let guidance = reason
                .guidance()
                .expect("WouldOverwrite should have guidance");
            assert!(guidance.contains("skip-existing"));
        }

        #[test]
        fn given_path_not_found_when_guidance_then_returns_path_update_suggestion() {
            let reason = SkipReason::PathNotFound {
                path: std::path::PathBuf::from("/missing/skill.md"),
                context: "skill definition".to_string(),
            };

            let guidance = reason
                .guidance()
                .expect("PathNotFound should have guidance");
            assert!(
                guidance.contains("path") || guidance.contains("exclude"),
                "guidance should suggest path update or exclusion"
            );
        }

        #[test]
        fn given_agent_specific_feature_when_guidance_then_returns_custom_suggestion() {
            let reason = SkipReason::AgentSpecificFeature {
                item: "mcp-server".to_string(),
                feature: "stdio transport".to_string(),
                suggestion: "Use HTTP transport instead".to_string(),
            };

            let guidance = reason
                .guidance()
                .expect("AgentSpecificFeature should have guidance");
            assert_eq!(guidance, "Use HTTP transport instead");
        }

        #[test]
        fn given_excluded_by_config_when_guidance_then_returns_intentional_message() {
            let reason = SkipReason::ExcludedByConfig {
                item: "deprecated-skill".to_string(),
                pattern: "deprecated-*".to_string(),
            };

            let guidance = reason
                .guidance()
                .expect("ExcludedByConfig should have guidance");
            assert!(
                guidance.to_lowercase().contains("intentional")
                    || guidance.to_lowercase().contains("no action"),
                "guidance should indicate intentional exclusion"
            );
        }

        #[test]
        fn given_parse_error_when_guidance_then_returns_fix_syntax_suggestion() {
            let reason = SkipReason::ParseError {
                item: "broken-skill.md".to_string(),
                error: "invalid YAML frontmatter".to_string(),
            };

            let guidance = reason.guidance().expect("ParseError should have guidance");
            assert!(
                guidance.to_lowercase().contains("fix")
                    || guidance.to_lowercase().contains("syntax"),
                "guidance should suggest fixing syntax"
            );
        }
    }

    // ==========================================
    // WriteReport Tests
    // ==========================================

    mod write_report {
        use super::*;

        #[test]
        fn given_default_when_created_then_empty() {
            let report = WriteReport::default();

            assert_eq!(report.written, 0);
            assert!(report.skipped.is_empty());
            assert!(report.warnings.is_empty());
        }

        #[test]
        fn when_fields_set_then_values_retained() {
            let report = WriteReport {
                written: 5,
                skipped: vec![SkipReason::Unchanged {
                    item: "x".to_string(),
                }],
                warnings: vec!["Warning 1".to_string()],
            };

            assert_eq!(report.written, 5);
            assert_eq!(report.skipped.len(), 1);
            assert_eq!(report.warnings.len(), 1);
        }
    }

    // ==========================================
    // SyncReport Tests
    // ==========================================

    mod sync_report {
        use super::*;

        #[test]
        fn given_new_when_created_then_success_and_empty() {
            let report = SyncReport::new();

            assert!(report.success);
            assert_eq!(report.total_synced(), 0);
            assert_eq!(report.total_skipped(), 0);
        }

        #[test]
        fn given_report_with_synced_items_when_total_synced_then_sums_all() {
            let mut report = SyncReport::new();
            report.skills.written = 3;
            report.commands.written = 2;
            report.mcp_servers.written = 1;
            report.preferences.written = 4;

            assert_eq!(report.total_synced(), 10);
        }

        #[test]
        fn given_report_with_skipped_items_when_total_skipped_then_sums_all() {
            let mut report = SyncReport::new();
            report.skills.skipped = vec![
                SkipReason::Unchanged {
                    item: "a".to_string(),
                },
                SkipReason::Unchanged {
                    item: "b".to_string(),
                },
            ];
            report.commands.skipped = vec![SkipReason::WouldOverwrite {
                item: "c".to_string(),
            }];

            assert_eq!(report.total_skipped(), 3);
        }

        #[test]
        fn given_report_when_format_summary_then_includes_all_sections() {
            let mut report = SyncReport::new();
            report.skills.written = 5;
            report.commands.written = 3;
            report.mcp_servers.written = 1;
            report.preferences.written = 2;
            report.skills.skipped = vec![SkipReason::Unchanged {
                item: "x".to_string(),
            }];

            let summary = report.format_summary("codex", "claude");

            assert!(summary.contains("codex"));
            assert!(summary.contains("claude"));
            assert!(summary.contains("Skills"));
            assert!(summary.contains("Commands"));
            assert!(summary.contains("MCP Servers"));
            assert!(summary.contains("Preferences"));
            assert!(summary.contains("5 synced"));
            assert!(summary.contains("1 skipped"));
        }

        #[test]
        fn given_empty_report_when_format_summary_then_shows_zeros() {
            let report = SyncReport::new();
            let summary = report.format_summary("src", "dest");

            assert!(summary.contains("0 synced"));
            assert!(summary.contains("0 skipped"));
        }
    }

    // ==========================================
    // Serialization Tests (ensure serde works)
    // ==========================================

    mod serialization {
        use super::*;

        #[test]
        fn skip_reason_serializes_with_type_tag() {
            let reason = SkipReason::Unchanged {
                item: "test".to_string(),
            };

            let json = serde_json::to_string(&reason).unwrap();
            assert!(json.contains("\"type\":\"Unchanged\""));
            assert!(json.contains("\"item\":\"test\""));
        }

        #[test]
        fn skip_reason_deserializes_from_json() {
            let json = r#"{"type":"Unchanged","item":"test"}"#;
            let reason: SkipReason = serde_json::from_str(json).unwrap();

            match reason {
                SkipReason::Unchanged { item } => assert_eq!(item, "test"),
                _ => panic!("Wrong variant"),
            }
        }

        #[test]
        fn sync_report_roundtrips_through_json() {
            let mut report = SyncReport::new();
            report.skills.written = 5;
            report.success = true;
            report.summary = "Test summary".to_string();

            let json = serde_json::to_string(&report).unwrap();
            let restored: SyncReport = serde_json::from_str(&json).unwrap();

            assert_eq!(restored.skills.written, 5);
            assert!(restored.success);
            assert_eq!(restored.summary, "Test summary");
        }
    }
}
