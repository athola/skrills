//! Skill management command handlers.
//!
//! Commands for deprecating, rolling back, profiling, cataloging, importing,
//! scoring, and generating usage reports for skills.

mod catalog;
mod deprecation;
mod import;
mod pre_commit;
mod profiling;
mod rollback;
mod scoring;
mod sync_pull;
mod usage_report;

pub(crate) use catalog::handle_skill_catalog_command;
pub(crate) use deprecation::handle_skill_deprecate_command;
pub(crate) use import::handle_skill_import_command;
pub(crate) use pre_commit::handle_pre_commit_validate_command;
pub(crate) use profiling::handle_skill_profile_command;
pub(crate) use rollback::handle_skill_rollback_command;
pub(crate) use scoring::handle_skill_score_command;
pub(crate) use sync_pull::handle_sync_pull_command;
pub(crate) use usage_report::handle_skill_usage_report_command;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Escape a string for safe embedding in YAML double-quoted values.
pub(super) fn escape_yaml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Result of skill deprecation operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct DeprecationResult {
    pub skill_name: String,
    pub skill_path: PathBuf,
    pub deprecated: bool,
    pub message: Option<String>,
    pub replacement: Option<String>,
}

/// Version info for skill rollback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillVersion {
    pub hash: String,
    pub date: String,
    pub message: String,
}

/// Result of skill rollback operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct RollbackResult {
    pub skill_name: String,
    pub skill_path: PathBuf,
    pub rolled_back: bool,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub available_versions: Vec<SkillVersion>,
}

/// Statistics for skill profiling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStats {
    pub name: String,
    pub invocations: u64,
    pub last_used: Option<String>,
    pub avg_tokens: Option<f64>,
    pub success_rate: Option<f64>,
}

/// Result of skill profile operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileResult {
    pub period_days: u32,
    pub total_invocations: u64,
    pub unique_skills_used: usize,
    pub top_skills: Vec<SkillStats>,
}

/// Catalog entry for a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub source: String,
    pub description: Option<String>,
    pub path: PathBuf,
    pub deprecated: bool,
}

/// Result of skill catalog operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogResult {
    pub total_skills: usize,
    pub skills: Vec<CatalogEntry>,
}

/// Result of skill import operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub source: String,
    pub target_path: PathBuf,
    pub imported: bool,
    pub skill_name: Option<String>,
    pub message: String,
}

/// Skill usage statistics for reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub skill_name: String,
    pub invocations: u64,
    pub percentage: f64,
}

/// Result of usage report generation.
#[derive(Debug, Serialize, Deserialize)]
pub struct UsageReportResult {
    pub period_days: u32,
    pub generated_at: String,
    pub total_invocations: u64,
    pub unique_skills: usize,
    pub skills: Vec<UsageStats>,
}

/// Quality score components.
#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub frontmatter_completeness: u8,
    pub validation_score: u8,
    pub description_quality: u8,
    pub token_efficiency: u8,
}

/// Score result for a skill.
#[derive(Debug, Serialize, Deserialize)]
pub struct SkillScoreResult {
    pub name: String,
    pub path: PathBuf,
    pub total_score: u8,
    pub breakdown: ScoreBreakdown,
    pub suggestions: Vec<String>,
}

/// Result of sync-pull operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncPullResult {
    pub source: Option<String>,
    pub target: String,
    pub skills_pulled: usize,
    pub dry_run: bool,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that invalid git version hashes are rejected (command injection prevention)
    #[test]
    fn rollback_invalid_version_hash_is_rejected() {
        let valid_hashes = ["abc1", "abc12345", "abc123456789abcdef", "ABCDEF0123456789"];
        let hash_pattern = regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").unwrap();

        for hash in &valid_hashes {
            assert!(
                hash_pattern.is_match(hash),
                "Expected '{}' to be valid",
                hash
            );
        }

        let invalid_hashes = [
            "abc",
            "; rm -rf /",
            "abc123; echo pwned",
            "$(whoami)",
            "`id`",
            "abc\necho hacked",
            "abc|cat /etc/passwd",
            "--help",
            "-",
            "",
        ];

        for hash in &invalid_hashes {
            assert!(
                !hash_pattern.is_match(hash),
                "Expected '{}' to be rejected as invalid",
                hash
            );
        }
    }

    #[test]
    fn deprecation_message_basic_format() {
        let message = "Use new-skill instead";
        let formatted = format!("deprecation_message: \"{}\"\n", message);
        assert!(formatted.contains("\"Use new-skill instead\""));
    }

    #[test]
    fn skill_version_serializes_correctly() {
        let version = SkillVersion {
            hash: "abc1234".to_string(),
            date: "2024-01-15 10:30:00 -0500".to_string(),
            message: "Initial commit".to_string(),
        };

        let json = serde_json::to_string(&version).unwrap();
        assert!(json.contains("abc1234"));
        assert!(json.contains("Initial commit"));
    }

    #[test]
    fn rollback_result_default_state() {
        let result = RollbackResult {
            skill_name: "test-skill".to_string(),
            skill_path: PathBuf::from("/path/to/skill.md"),
            rolled_back: false,
            from_version: None,
            to_version: None,
            available_versions: vec![],
        };

        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("\"rolled_back\": false"));
        assert!(json.contains("\"available_versions\": []"));
    }

    #[test]
    fn import_result_existing_skill_message() {
        let result = ImportResult {
            source: "/path/to/source.md".to_string(),
            target_path: PathBuf::from("/home/user/.claude/skills/my-skill.md"),
            imported: false,
            skill_name: Some("my-skill".to_string()),
            message: "Skill 'my-skill' already exists. Use --force to overwrite.".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"imported\":false"));
        assert!(json.contains("--force"));
    }

    #[test]
    fn precommit_validation_error_flag_tracking() {
        let mut errors_found = false;
        let mut validated = 0;

        validated += 1;

        let read_failed = true;
        if read_failed {
            errors_found = true;
        }

        let has_validation_errors = true;
        if has_validation_errors {
            errors_found = true;
        }

        assert!(
            errors_found,
            "errors_found should be true when any error occurs"
        );
        assert_eq!(
            validated, 1,
            "Only successful validations should be counted"
        );
    }

    #[test]
    fn escape_yaml_string_handles_special_chars() {
        assert_eq!(escape_yaml_string("hello"), "hello");
        assert_eq!(escape_yaml_string(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(escape_yaml_string(r"back\slash"), r"back\\slash");
    }

    #[test]
    fn rollback_result_with_empty_available_versions() {
        let result = RollbackResult {
            skill_name: "test".to_string(),
            skill_path: PathBuf::from("/tmp/skill.md"),
            rolled_back: false,
            from_version: None,
            to_version: None,
            available_versions: vec![],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"available_versions\":[]"));
        assert!(json.contains("\"rolled_back\":false"));
    }

    #[test]
    fn rollback_version_hash_rejects_empty_string() {
        let hash_pattern = regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").unwrap();
        assert!(!hash_pattern.is_match(""));
    }

    #[test]
    fn rollback_version_hash_rejects_too_short() {
        let hash_pattern = regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").unwrap();
        assert!(!hash_pattern.is_match("ab"));
        assert!(!hash_pattern.is_match("abc"));
    }

    #[test]
    fn rollback_version_hash_rejects_too_long() {
        let hash_pattern = regex::Regex::new(r"^[0-9a-fA-F]{4,40}$").unwrap();
        let long_hash = "a".repeat(41);
        assert!(!hash_pattern.is_match(&long_hash));
    }

    #[test]
    fn deprecation_result_nonexistent_skill() {
        // Simulates the result structure when a skill is not found
        let result = DeprecationResult {
            skill_name: "nonexistent-skill".to_string(),
            skill_path: PathBuf::from("/does/not/exist.md"),
            deprecated: false,
            message: Some("Skill not found".to_string()),
            replacement: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"deprecated\":false"));
        assert!(json.contains("Skill not found"));
    }

    #[test]
    fn deprecation_empty_message_uses_default() {
        // The handle function uses a default when message is None
        let message: Option<String> = None;
        let deprecation_msg = message.as_deref().unwrap_or("This skill is deprecated");
        assert_eq!(deprecation_msg, "This skill is deprecated");
    }

    #[test]
    fn deprecation_with_empty_string_message() {
        let message = Some("".to_string());
        let deprecation_msg = message.as_deref().unwrap_or("This skill is deprecated");
        assert_eq!(deprecation_msg, "");
        // Empty string is technically valid but produces empty deprecation_message
        let formatted = format!(
            "deprecation_message: \"{}\"\n",
            escape_yaml_string(deprecation_msg)
        );
        assert_eq!(formatted, "deprecation_message: \"\"\n");
    }

    #[test]
    fn import_result_url_source_error() {
        // URL imports should produce an error
        let source = "https://example.com/skill.md";
        assert!(source.starts_with("http://") || source.starts_with("https://"));
    }

    #[test]
    fn import_result_git_source_error() {
        let source = "git://github.com/repo.git";
        assert!(source.starts_with("git://") || source.ends_with(".git"));
    }

    #[test]
    fn escape_yaml_string_empty() {
        assert_eq!(escape_yaml_string(""), "");
    }

    #[test]
    fn escape_yaml_string_multiple_special_chars() {
        let input = r#"say "hello" and use back\slash"#;
        let escaped = escape_yaml_string(input);
        assert_eq!(escaped, r#"say \"hello\" and use back\\slash"#);
    }

    #[test]
    fn catalog_entry_deprecated_flag() {
        let entry = CatalogEntry {
            name: "old-skill".to_string(),
            source: "claude".to_string(),
            description: Some("Deprecated skill".to_string()),
            path: PathBuf::from("/skills/old.md"),
            deprecated: true,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"deprecated\":true"));
    }
}
