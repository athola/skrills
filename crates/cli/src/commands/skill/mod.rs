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

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use skrills_discovery::SkillMeta;
use std::path::PathBuf;

/// Escape a string for safe embedding in YAML double-quoted values.
///
/// Newlines and tabs are escaped too: a raw newline inside a double-quoted
/// scalar would be folded or, before a `key:` line, end the value.
pub(super) fn escape_yaml_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// The name a user would type for a skill: its directory name, which is the
/// skill's name by convention (`review/SKILL.md` is `review`).
pub(super) fn skill_dir_name(meta: &SkillMeta) -> String {
    meta.path
        .parent()
        .and_then(|p| p.file_name())
        .or_else(|| meta.path.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| meta.name.clone())
}

/// Finds the one discovered skill that `query` names.
///
/// A skill matches when `query` equals, ignoring ASCII case, its discovery
/// key (`review/SKILL.md`), its directory name (`review`) or its frontmatter
/// `name`. Commands that rewrite the file call this, so there is no substring
/// fallback and no first-hit pick: no match and several matches are both
/// errors, the latter listing every candidate.
pub(super) fn find_skill<'a>(skills: &'a [SkillMeta], query: &str) -> Result<&'a SkillMeta> {
    let matches: Vec<&SkillMeta> = skills
        .iter()
        .filter(|s| {
            s.name.eq_ignore_ascii_case(query)
                || skill_dir_name(s).eq_ignore_ascii_case(query)
                || s.frontmatter_name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(query))
        })
        .collect();
    match matches.as_slice() {
        [] => bail!("Skill '{query}' not found in discovered skills"),
        [one] => Ok(one),
        many => bail!(
            "Skill name '{query}' is ambiguous; it matches {} skills:\n{}\nPass --skill-dir to narrow the search.",
            many.len(),
            many.iter()
                .map(|s| format!("  {}", s.path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
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

    fn meta(root: &str, rel: &str, fm_name: Option<&str>) -> SkillMeta {
        SkillMeta {
            name: rel.to_string(),
            path: PathBuf::from(root).join(rel),
            source: skrills_discovery::SkillSource::Extra(0),
            root: PathBuf::from(root),
            hash: String::new(),
            description: None,
            frontmatter_name: fm_name.map(str::to_string),
        }
    }

    /// SA-3 / SB-18: `test` used to match every skill whose absolute path
    /// contained "test" and the first one was rewritten.
    #[test]
    fn find_skill_ignores_substrings_of_the_path() {
        let skills = vec![
            meta("/home/u/test-projects/skills", "alpha/SKILL.md", None),
            meta("/home/u/test-projects/skills", "beta/SKILL.md", None),
        ];

        let err = find_skill(&skills, "test").unwrap_err().to_string();

        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn find_skill_matches_directory_name_key_and_frontmatter_name() {
        let skills = vec![
            meta("/r", "alpha/SKILL.md", None),
            meta("/r", "beta/SKILL.md", Some("Beta-Skill")),
        ];

        assert_eq!(find_skill(&skills, "ALPHA").unwrap().name, "alpha/SKILL.md");
        assert_eq!(
            find_skill(&skills, "alpha/SKILL.md").unwrap().name,
            "alpha/SKILL.md"
        );
        assert_eq!(
            find_skill(&skills, "beta-skill").unwrap().name,
            "beta/SKILL.md"
        );
    }

    #[test]
    fn find_skill_refuses_an_ambiguous_name_and_lists_the_candidates() {
        let skills = vec![
            meta("/plugin-a/skills", "review/SKILL.md", None),
            meta("/plugin-b/skills", "nested/review/SKILL.md", None),
        ];

        let err = find_skill(&skills, "review").unwrap_err().to_string();

        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("/plugin-a/skills/review/SKILL.md"), "{err}");
        assert!(
            err.contains("/plugin-b/skills/nested/review/SKILL.md"),
            "{err}"
        );
    }

    #[test]
    fn skill_dir_name_is_the_parent_directory() {
        let m = meta("/r", "group/review/SKILL.md", None);
        assert_eq!(skill_dir_name(&m), "review");
    }

    /// SA-46: a newline in `--message` ended the YAML scalar.
    #[test]
    fn escape_yaml_string_escapes_line_breaks_and_tabs() {
        assert_eq!(escape_yaml_string("a\nb\r\tc"), r"a\nb\r\tc");
    }
}
