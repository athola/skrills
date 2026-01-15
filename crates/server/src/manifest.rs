//! Skills manifest for caching and quick loading.
//!
//! This module provides types and utilities for managing a skills manifest,
//! which enables fast loading of skill metadata without re-scanning the filesystem.
//!
//! # Example
//!
//! ```no_run
//! use skrills_server::manifest::{SkillsManifest, SkillManifestEntry};
//! use std::path::PathBuf;
//!
//! let mut manifest = SkillsManifest::new();
//! manifest.skills.push(SkillManifestEntry {
//!     name: "my-skill".to_string(),
//!     description: Some("A useful skill".to_string()),
//!     path: PathBuf::from("/path/to/SKILL.md"),
//!     source: "codex".to_string(),
//! });
//!
//! let markdown = manifest.to_markdown();
//! assert!(markdown.contains("my-skill"));
//! ```

use serde::{Deserialize, Serialize};
use skrills_discovery::SkillMeta;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Skill entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillManifestEntry {
    /// The skill name.
    pub name: String,
    /// Optional description of the skill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Path to the SKILL.md file.
    pub path: PathBuf,
    /// The source where this skill was discovered (e.g., "codex", "claude").
    pub source: String,
}

/// Skills manifest for caching and quick loading.
///
/// The manifest stores a list of discovered skills with their metadata,
/// enabling faster startup by avoiding filesystem scans when the cache is valid.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SkillsManifest {
    /// Manifest version for compatibility checking.
    pub version: u32,
    /// List of skill entries.
    pub skills: Vec<SkillManifestEntry>,
    /// RFC 3339 timestamp when the manifest was generated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
}

impl SkillsManifest {
    /// Current manifest version.
    pub const CURRENT_VERSION: u32 = 1;

    /// Creates a new empty manifest with the current version.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            skills: Vec::new(),
            generated_at: None,
        }
    }

    /// Loads a manifest from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let manifest: Self = serde_json::from_str(&content)?;
        Ok(manifest)
    }

    /// Saves the manifest to a JSON file.
    ///
    /// Creates parent directories if they don't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Generates a markdown skill list for embedding.
    ///
    /// Each skill is formatted as a bullet point with optional description.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        for skill in &self.skills {
            md.push_str(&format!("- **{}**", skill.name));
            if let Some(desc) = &skill.description {
                md.push_str(&format!(": {}", desc));
            }
            md.push('\n');
        }
        md
    }

    /// Returns the number of skills in the manifest.
    #[must_use]
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Returns `true` if the manifest contains no skills.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

/// Generates a manifest from discovered skills.
///
/// This function converts a slice of `SkillMeta` into a `SkillsManifest`,
/// setting the generated timestamp to the current time.
#[must_use]
pub fn generate_manifest(skills: &[SkillMeta]) -> SkillsManifest {
    let entries = skills
        .iter()
        .map(|s| SkillManifestEntry {
            name: s.name.clone(),
            description: s.description.clone(),
            path: s.path.clone(),
            source: s.source.label(),
        })
        .collect();

    SkillsManifest {
        version: SkillsManifest::CURRENT_VERSION,
        skills: entries,
        generated_at: Some(format_rfc3339(SystemTime::now())),
    }
}

/// Default path for the skills manifest.
///
/// # Errors
///
/// Returns an error if the home directory cannot be determined.
pub fn default_manifest_path() -> anyhow::Result<PathBuf> {
    Ok(skrills_state::home_dir()?.join(".codex/cache/skills-manifest.json"))
}

/// Formats a `SystemTime` as an RFC 3339 timestamp.
///
/// This is a simple implementation that converts to seconds since UNIX epoch.
fn format_rfc3339(time: SystemTime) -> String {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => {
            let secs = duration.as_secs();
            // Simple UTC timestamp format: YYYY-MM-DDTHH:MM:SSZ
            // Calculate date components from epoch seconds
            let days_since_epoch = secs / 86400;
            let time_of_day = secs % 86400;

            let hours = time_of_day / 3600;
            let minutes = (time_of_day % 3600) / 60;
            let seconds = time_of_day % 60;

            // Approximate date calculation (not accounting for leap years perfectly)
            let (year, month, day) = days_to_ymd(days_since_epoch);

            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                year, month, day, hours, minutes, seconds
            )
        }
        Err(_) => "1970-01-01T00:00:00Z".to_string(),
    }
}

/// Converts days since UNIX epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Days since 1970-01-01
    let mut remaining = days as i64;
    let mut year = 1970;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let days_in_months: [i64; 12] = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1;
    for days_in_month in days_in_months {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }

    let day = remaining + 1;

    (year as u64, month, day as u64)
}

/// Returns true if the given year is a leap year.
fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::SkillSource;
    use tempfile::tempdir;

    #[test]
    fn test_manifest_new() {
        let manifest = SkillsManifest::new();
        assert_eq!(manifest.version, SkillsManifest::CURRENT_VERSION);
        assert!(manifest.skills.is_empty());
        assert!(manifest.generated_at.is_none());
    }

    #[test]
    fn test_manifest_default() {
        let manifest = SkillsManifest::default();
        assert_eq!(manifest.version, 0);
        assert!(manifest.skills.is_empty());
    }

    #[test]
    fn test_manifest_serialize_deserialize() {
        let mut manifest = SkillsManifest::new();
        manifest.skills.push(SkillManifestEntry {
            name: "test-skill".to_string(),
            description: Some("A test skill".to_string()),
            path: PathBuf::from("/path/to/SKILL.md"),
            source: "codex".to_string(),
        });
        manifest.generated_at = Some("2024-01-01T00:00:00Z".to_string());

        let json = serde_json::to_string(&manifest).unwrap();
        let deserialized: SkillsManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(manifest, deserialized);
    }

    #[test]
    fn test_manifest_save_and_load() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("cache/manifest.json");

        let mut manifest = SkillsManifest::new();
        manifest.skills.push(SkillManifestEntry {
            name: "saved-skill".to_string(),
            description: None,
            path: PathBuf::from("/skill/path"),
            source: "claude".to_string(),
        });

        manifest.save(&path).unwrap();
        assert!(path.exists());

        let loaded = SkillsManifest::load(&path).unwrap();
        assert_eq!(manifest.version, loaded.version);
        assert_eq!(manifest.skills.len(), loaded.skills.len());
        assert_eq!(manifest.skills[0].name, loaded.skills[0].name);
    }

    #[test]
    fn test_manifest_to_markdown() {
        let mut manifest = SkillsManifest::new();
        manifest.skills.push(SkillManifestEntry {
            name: "skill-one".to_string(),
            description: Some("First skill description".to_string()),
            path: PathBuf::from("/path/one"),
            source: "codex".to_string(),
        });
        manifest.skills.push(SkillManifestEntry {
            name: "skill-two".to_string(),
            description: None,
            path: PathBuf::from("/path/two"),
            source: "claude".to_string(),
        });

        let md = manifest.to_markdown();
        assert!(md.contains("- **skill-one**: First skill description"));
        assert!(md.contains("- **skill-two**\n"));
    }

    #[test]
    fn test_manifest_to_markdown_empty() {
        let manifest = SkillsManifest::new();
        let md = manifest.to_markdown();
        assert!(md.is_empty());
    }

    #[test]
    fn test_generate_manifest_from_skills() {
        let skills = vec![
            SkillMeta {
                name: "test/SKILL.md".to_string(),
                path: PathBuf::from("/home/user/.codex/skills/test/SKILL.md"),
                source: SkillSource::Codex,
                root: PathBuf::from("/home/user/.codex/skills"),
                hash: "abc123".to_string(),
                description: None,
            },
            SkillMeta {
                name: "another/SKILL.md".to_string(),
                path: PathBuf::from("/home/user/.claude/skills/another/SKILL.md"),
                source: SkillSource::Claude,
                root: PathBuf::from("/home/user/.claude/skills"),
                hash: "def456".to_string(),
                description: None,
            },
        ];

        let manifest = generate_manifest(&skills);

        assert_eq!(manifest.version, SkillsManifest::CURRENT_VERSION);
        assert_eq!(manifest.skills.len(), 2);
        assert!(manifest.generated_at.is_some());

        assert_eq!(manifest.skills[0].name, "test/SKILL.md");
        assert_eq!(manifest.skills[0].source, "codex");
        assert_eq!(manifest.skills[1].name, "another/SKILL.md");
        assert_eq!(manifest.skills[1].source, "claude");
    }

    #[test]
    fn test_generate_manifest_empty() {
        let manifest = generate_manifest(&[]);
        assert_eq!(manifest.version, SkillsManifest::CURRENT_VERSION);
        assert!(manifest.skills.is_empty());
        assert!(manifest.generated_at.is_some());
    }

    #[test]
    fn test_manifest_len_and_is_empty() {
        let mut manifest = SkillsManifest::new();
        assert!(manifest.is_empty());
        assert_eq!(manifest.len(), 0);

        manifest.skills.push(SkillManifestEntry {
            name: "skill".to_string(),
            description: None,
            path: PathBuf::from("/path"),
            source: "codex".to_string(),
        });

        assert!(!manifest.is_empty());
        assert_eq!(manifest.len(), 1);
    }

    #[test]
    fn test_format_rfc3339() {
        // Test epoch
        let epoch = SystemTime::UNIX_EPOCH;
        let formatted = format_rfc3339(epoch);
        assert_eq!(formatted, "1970-01-01T00:00:00Z");
    }

    #[test]
    fn test_format_rfc3339_known_date() {
        use std::time::Duration;

        // 2024-01-15T12:30:45Z = 1705321845 seconds since epoch
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1705321845);
        let formatted = format_rfc3339(time);
        assert_eq!(formatted, "2024-01-15T12:30:45Z");
    }

    #[test]
    fn test_days_to_ymd() {
        // 1970-01-01 = day 0
        assert_eq!(days_to_ymd(0), (1970, 1, 1));

        // 1970-12-31 = day 364
        assert_eq!(days_to_ymd(364), (1970, 12, 31));

        // 1971-01-01 = day 365
        assert_eq!(days_to_ymd(365), (1971, 1, 1));

        // 2000-03-01 (leap year test) = day 11017
        assert_eq!(days_to_ymd(11017), (2000, 3, 1));
    }

    #[test]
    fn test_is_leap_year() {
        assert!(!is_leap_year(1970));
        assert!(is_leap_year(2000)); // Divisible by 400
        assert!(!is_leap_year(1900)); // Divisible by 100 but not 400
        assert!(is_leap_year(2024)); // Divisible by 4
        assert!(!is_leap_year(2023));
    }

    #[test]
    fn test_manifest_load_nonexistent() {
        let result = SkillsManifest::load(Path::new("/nonexistent/path/manifest.json"));
        assert!(result.is_err());
    }

    #[test]
    fn test_manifest_load_invalid_json() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("invalid.json");
        std::fs::write(&path, "not valid json").unwrap();

        let result = SkillsManifest::load(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_skill_manifest_entry_serialization() {
        let entry = SkillManifestEntry {
            name: "my-skill".to_string(),
            description: None,
            path: PathBuf::from("/path/to/skill"),
            source: "codex".to_string(),
        };

        let json = serde_json::to_string(&entry).unwrap();
        // description should be omitted when None
        assert!(!json.contains("description"));

        let entry_with_desc = SkillManifestEntry {
            name: "my-skill".to_string(),
            description: Some("A description".to_string()),
            path: PathBuf::from("/path/to/skill"),
            source: "codex".to_string(),
        };

        let json = serde_json::to_string(&entry_with_desc).unwrap();
        assert!(json.contains("description"));
    }

    /// Tests that manifest save fails gracefully on read-only parent directory.
    /// This documents the error propagation behavior of create_dir_all failures.
    #[cfg(unix)]
    #[test]
    fn test_manifest_save_fails_on_readonly_parent() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir().unwrap();
        let readonly_dir = tmp.path().join("readonly");
        std::fs::create_dir(&readonly_dir).unwrap();

        // Make directory read-only (no write permission)
        std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o444)).unwrap();

        let manifest = SkillsManifest::new();
        // Try to save to a subdirectory that can't be created
        let path = readonly_dir.join("subdir/manifest.json");

        let result = manifest.save(&path);
        assert!(result.is_err(), "Expected error when parent is read-only");

        // Verify error message contains useful context
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Permission denied") || err_msg.contains("permission"),
            "Error should mention permission issue: {}",
            err_msg
        );

        // Cleanup: restore permissions for tempdir cleanup
        std::fs::set_permissions(&readonly_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
