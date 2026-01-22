//! Dependency analysis for skills.
//!
//! Analyzes skill dependencies including:
//! - Local file references (modules/, references/, scripts/, assets/)
//! - External links and URLs
//! - Cross-skill references

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

/// Severity level for warnings encountered during dependency analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WarningLevel {
    /// Informational note, not a problem.
    Info,
    /// Potential issue that may need attention.
    Warning,
    /// Problem that should be addressed.
    Error,
}

impl fmt::Display for WarningLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Info => write!(f, "info"),
            Self::Warning => write!(f, "warning"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Kind of warning encountered during dependency analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// Failed to read file metadata.
    MetadataAccessFailed,
    /// Failed to access directory entry during traversal.
    DirectoryEntryAccessFailed,
    /// Failed to read file contents.
    FileReadFailed,
}

impl fmt::Display for WarningKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MetadataAccessFailed => write!(f, "metadata_access_failed"),
            Self::DirectoryEntryAccessFailed => write!(f, "directory_entry_access_failed"),
            Self::FileReadFailed => write!(f, "file_read_failed"),
        }
    }
}

/// A structured warning encountered during dependency analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    /// Severity level of the warning.
    pub level: WarningLevel,
    /// Kind of warning.
    pub kind: WarningKind,
    /// Human-readable message describing the issue.
    pub message: String,
    /// Path context where the warning occurred, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<PathBuf>,
}

impl Warning {
    /// Creates a new warning with the given parameters.
    pub fn new(level: WarningLevel, kind: WarningKind, message: impl Into<String>) -> Self {
        Self {
            level,
            kind,
            message: message.into(),
            context: None,
        }
    }

    /// Adds path context to the warning.
    #[must_use]
    pub fn with_context(mut self, path: impl Into<PathBuf>) -> Self {
        self.context = Some(path.into());
        self
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Types of dependencies a skill can have.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DependencyType {
    /// Reference to a local module file.
    Module,
    /// Reference to a file in references/ directory.
    Reference,
    /// Reference to a script in scripts/ directory.
    Script,
    /// Reference to an asset file.
    Asset,
    /// External URL reference.
    ExternalUrl,
    /// Reference to another skill.
    Skill,
}

/// A single dependency found in a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// Type of dependency.
    pub dep_type: DependencyType,
    /// Path or URL to the dependency.
    pub target: String,
    /// Line number where found (1-indexed).
    pub line: Option<usize>,
    /// Whether the dependency exists (for local files).
    pub exists: Option<bool>,
}

/// Analysis result for skill dependencies.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyAnalysis {
    /// All dependencies found.
    pub dependencies: Vec<Dependency>,
    /// Local directories that exist.
    pub directories: Vec<String>,
    /// Missing local dependencies.
    pub missing: Vec<Dependency>,
    /// Total size of local dependencies in bytes.
    pub total_dep_size: u64,
    /// Structured warnings encountered during analysis (e.g., permission errors).
    pub warnings: Vec<Warning>,
}

impl DependencyAnalysis {
    /// Returns warnings as strings for backward compatibility.
    pub fn warning_messages(&self) -> Vec<String> {
        self.warnings.iter().map(|w| w.message.clone()).collect()
    }

    /// Filters warnings by severity level.
    pub fn warnings_by_level(&self, level: WarningLevel) -> Vec<&Warning> {
        self.warnings.iter().filter(|w| w.level == level).collect()
    }

    /// Filters warnings by kind.
    pub fn warnings_by_kind(&self, kind: WarningKind) -> Vec<&Warning> {
        self.warnings.iter().filter(|w| w.kind == kind).collect()
    }
}

impl DependencyAnalysis {
    /// Count dependencies by type.
    pub fn count_by_type(&self, dep_type: DependencyType) -> usize {
        self.dependencies
            .iter()
            .filter(|d| d.dep_type == dep_type)
            .count()
    }

    /// Get all external URLs.
    pub fn external_urls(&self) -> Vec<&str> {
        self.dependencies
            .iter()
            .filter(|d| d.dep_type == DependencyType::ExternalUrl)
            .map(|d| d.target.as_str())
            .collect()
    }
}

/// Analyze dependencies for a skill file.
pub fn analyze_dependencies(skill_path: &Path, content: &str) -> DependencyAnalysis {
    let mut analysis = DependencyAnalysis::default();
    let skill_dir = skill_path.parent().unwrap_or(Path::new("."));

    // Check for standard subdirectories
    for subdir in &["modules", "references", "scripts", "assets"] {
        let dir_path = skill_dir.join(subdir);
        if dir_path.exists() && dir_path.is_dir() {
            analysis.directories.push(subdir.to_string());

            // Calculate total size
            for entry in WalkDir::new(&dir_path).into_iter() {
                match entry {
                    Ok(entry) => {
                        if entry.path().is_file() {
                            match entry.metadata() {
                                Ok(meta) => {
                                    analysis.total_dep_size += meta.len();
                                }
                                Err(e) => {
                                    let msg = format!(
                                        "Could not read metadata for {}: {}",
                                        entry.path().display(),
                                        e
                                    );
                                    tracing::warn!(
                                        path = %entry.path().display(),
                                        error = %e,
                                        kind = "metadata_access_failed",
                                        "{}",
                                        msg
                                    );
                                    analysis.warnings.push(
                                        Warning::new(
                                            WarningLevel::Warning,
                                            WarningKind::MetadataAccessFailed,
                                            msg,
                                        )
                                        .with_context(entry.path()),
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let msg =
                            format!("failed to access entry in {}: {}", dir_path.display(), e);
                        tracing::warn!(
                            dir = %dir_path.display(),
                            error = %e,
                            kind = "directory_entry_access_failed",
                            "{}",
                            msg
                        );
                        analysis.warnings.push(
                            Warning::new(
                                WarningLevel::Warning,
                                WarningKind::DirectoryEntryAccessFailed,
                                msg,
                            )
                            .with_context(&dir_path),
                        );
                    }
                }
            }
        }
    }

    // Extract dependencies from content
    extract_content_dependencies(&mut analysis, skill_dir, content);

    // Check which dependencies exist
    for dep in &mut analysis.dependencies {
        if matches!(
            dep.dep_type,
            DependencyType::Module
                | DependencyType::Reference
                | DependencyType::Script
                | DependencyType::Asset
        ) {
            let path = skill_dir.join(&dep.target);
            dep.exists = Some(path.exists());

            if !path.exists() {
                analysis.missing.push(dep.clone());
            }
        }
    }

    analysis
}

// RATIONALE: These regex patterns are compile-time string literals that have been verified
// to be valid. The `.expect()` calls will never panic because:
// 1. Patterns are hardcoded constants, not user-provided
// 2. Each pattern has been tested and is syntactically correct
// 3. LazyLock ensures initialization happens only once at runtime
// (Using RATIONALE instead of SAFETY since this is safe code, not unsafe)
static URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://[^\s\)\]>]+").expect("URL_REGEX: compile-time constant")
});
static LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").expect("LINK_REGEX: compile-time constant")
});
static IMAGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").expect("IMAGE_REGEX: compile-time constant")
});

fn extract_content_dependencies(
    analysis: &mut DependencyAnalysis,
    _skill_dir: &Path,
    content: &str,
) {
    let mut seen_urls: HashSet<String> = HashSet::new();
    let mut seen_paths: HashSet<String> = HashSet::new();

    for (line_num, line) in content.lines().enumerate() {
        let line_number = line_num + 1;

        // Find external URLs
        for url_match in URL_REGEX.find_iter(line) {
            let url = url_match
                .as_str()
                .trim_end_matches(&['.', ',', ')', ']'][..]);
            if !seen_urls.contains(url) {
                seen_urls.insert(url.to_string());
                analysis.dependencies.push(Dependency {
                    dep_type: DependencyType::ExternalUrl,
                    target: url.to_string(),
                    line: Some(line_number),
                    exists: None,
                });
            }
        }

        // Find markdown links (excluding URLs)
        for cap in LINK_REGEX.captures_iter(line) {
            let path = &cap[2];
            if !path.starts_with("http://")
                && !path.starts_with("https://")
                && !seen_paths.contains(path)
            {
                seen_paths.insert(path.to_string());
                let dep_type = classify_path(path);
                analysis.dependencies.push(Dependency {
                    dep_type,
                    target: path.to_string(),
                    line: Some(line_number),
                    exists: None,
                });
            }
        }

        // Find markdown images (excluding URLs)
        for cap in IMAGE_REGEX.captures_iter(line) {
            let path = &cap[2];
            if !path.starts_with("http://")
                && !path.starts_with("https://")
                && !seen_paths.contains(path)
            {
                seen_paths.insert(path.to_string());
                analysis.dependencies.push(Dependency {
                    dep_type: DependencyType::Asset,
                    target: path.to_string(),
                    line: Some(line_number),
                    exists: None,
                });
            }
        }
    }
}

fn classify_path(path: &str) -> DependencyType {
    let path_lower = path.to_lowercase();

    if path_lower.starts_with("modules/") || path_lower.contains("/modules/") {
        DependencyType::Module
    } else if path_lower.starts_with("references/") || path_lower.contains("/references/") {
        DependencyType::Reference
    } else if path_lower.starts_with("scripts/") || path_lower.contains("/scripts/") {
        DependencyType::Script
    } else if path_lower.starts_with("assets/")
        || path_lower.contains("/assets/")
        || is_asset_extension(path)
    {
        DependencyType::Asset
    } else if path_lower.ends_with(".md") || path_lower.contains("skill") {
        DependencyType::Skill
    } else {
        DependencyType::Reference
    }
}

fn is_asset_extension(path: &str) -> bool {
    let extensions = [
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp", ".mp4", ".mp3", ".wav", ".pdf",
    ];
    let path_lower = path.to_lowercase();
    extensions.iter().any(|ext| path_lower.ends_with(ext))
}

/// Result of listing dependency files.
#[derive(Debug, Clone, Default)]
pub struct DependencyFileList {
    /// Files found in dependency directories.
    pub files: Vec<PathBuf>,
    /// Structured warnings encountered (e.g., permission errors).
    pub warnings: Vec<Warning>,
}

impl DependencyFileList {
    /// Returns warnings as strings for backward compatibility.
    pub fn warning_messages(&self) -> Vec<String> {
        self.warnings.iter().map(|w| w.message.clone()).collect()
    }
}

/// Get all files in a skill's dependency directories.
pub fn list_dependency_files(skill_path: &Path) -> DependencyFileList {
    let skill_dir = skill_path.parent().unwrap_or(Path::new("."));
    let mut result = DependencyFileList::default();

    for subdir in &["modules", "references", "scripts", "assets"] {
        let dir_path = skill_dir.join(subdir);
        if dir_path.exists() {
            for entry in WalkDir::new(&dir_path).into_iter() {
                match entry {
                    Ok(entry) => {
                        if entry.path().is_file() {
                            result.files.push(entry.path().to_path_buf());
                        }
                    }
                    Err(e) => {
                        let msg =
                            format!("failed to access entry in {}: {}", dir_path.display(), e);
                        tracing::warn!(
                            dir = %dir_path.display(),
                            error = %e,
                            kind = "directory_entry_access_failed",
                            "{}",
                            msg
                        );
                        result.warnings.push(
                            Warning::new(
                                WarningLevel::Warning,
                                WarningKind::DirectoryEntryAccessFailed,
                                msg,
                            )
                            .with_context(&dir_path),
                        );
                    }
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_urls() {
        let content = "Check out https://example.com for more info.";
        let mut analysis = DependencyAnalysis::default();
        extract_content_dependencies(&mut analysis, Path::new("."), content);

        assert_eq!(analysis.count_by_type(DependencyType::ExternalUrl), 1);
        assert!(analysis.dependencies[0].target.contains("example.com"));
    }

    #[test]
    fn test_extract_markdown_links() {
        let content = "See [docs](references/guide.md) for details.";
        let mut analysis = DependencyAnalysis::default();
        extract_content_dependencies(&mut analysis, Path::new("."), content);

        assert!(analysis
            .dependencies
            .iter()
            .any(|d| d.target == "references/guide.md"));
    }

    #[test]
    fn test_classify_path() {
        assert_eq!(classify_path("modules/core.md"), DependencyType::Module);
        assert_eq!(
            classify_path("references/api.md"),
            DependencyType::Reference
        );
        assert_eq!(classify_path("scripts/build.sh"), DependencyType::Script);
        assert_eq!(classify_path("assets/logo.png"), DependencyType::Asset);
        assert_eq!(classify_path("diagram.png"), DependencyType::Asset);
    }

    #[test]
    fn test_dependency_analysis_warnings_default_empty() {
        let analysis = DependencyAnalysis::default();
        assert!(analysis.warnings.is_empty());
    }

    #[test]
    fn test_analyze_dependencies_no_warnings_for_accessible_files() {
        // When analyzing a path with no subdirectories, there should be no warnings
        let analysis = analyze_dependencies(Path::new("/nonexistent/skill.md"), "");
        // No warnings expected since there are no directories to walk
        assert!(analysis.warnings.is_empty());
    }

    #[test]
    fn test_extract_markdown_images() {
        // Explicitly tests IMAGE_REGEX pattern extraction
        let content = r#"
Here's an image: ![alt text](images/diagram.png)
And another: ![logo](assets/logo.svg)
URL images should be skipped: ![remote](https://example.com/pic.jpg)
HTTP too: ![http](http://example.com/pic.jpg)
"#;
        let mut analysis = DependencyAnalysis::default();
        extract_content_dependencies(&mut analysis, Path::new("."), content);

        // Should extract the local image paths as Asset dependencies
        let assets: Vec<_> = analysis
            .dependencies
            .iter()
            .filter(|d| d.dep_type == DependencyType::Asset)
            .collect();

        assert_eq!(assets.len(), 2, "Should extract 2 local image paths");
        assert!(
            assets.iter().any(|d| d.target == "images/diagram.png"),
            "Should extract images/diagram.png"
        );
        assert!(
            assets.iter().any(|d| d.target == "assets/logo.svg"),
            "Should extract assets/logo.svg"
        );

        // URL images should be captured as ExternalUrl, not Asset
        let urls: Vec<_> = analysis
            .dependencies
            .iter()
            .filter(|d| d.dep_type == DependencyType::ExternalUrl)
            .collect();

        assert!(
            urls.iter()
                .any(|d| d.target.contains("example.com/pic.jpg")),
            "URL images should be extracted as ExternalUrl"
        );
    }

    #[test]
    fn test_image_regex_edge_cases() {
        // Additional edge cases for IMAGE_REGEX
        let content = r#"
Empty alt: ![](path/to/image.png)
With spaces in alt: ![my cool diagram](diagram.png)
Nested brackets should not confuse: ![text with [brackets]](file.png)
"#;
        let mut analysis = DependencyAnalysis::default();
        extract_content_dependencies(&mut analysis, Path::new("."), content);

        let assets: Vec<_> = analysis
            .dependencies
            .iter()
            .filter(|d| d.dep_type == DependencyType::Asset)
            .collect();

        assert!(
            assets.iter().any(|d| d.target == "path/to/image.png"),
            "Should handle empty alt text"
        );
        assert!(
            assets.iter().any(|d| d.target == "diagram.png"),
            "Should handle spaces in alt text"
        );
    }

    // ---- Warning type tests ----

    #[test]
    fn test_warning_level_display() {
        assert_eq!(WarningLevel::Info.to_string(), "info");
        assert_eq!(WarningLevel::Warning.to_string(), "warning");
        assert_eq!(WarningLevel::Error.to_string(), "error");
    }

    #[test]
    fn test_warning_kind_display() {
        assert_eq!(
            WarningKind::MetadataAccessFailed.to_string(),
            "metadata_access_failed"
        );
        assert_eq!(
            WarningKind::DirectoryEntryAccessFailed.to_string(),
            "directory_entry_access_failed"
        );
        assert_eq!(WarningKind::FileReadFailed.to_string(), "file_read_failed");
    }

    #[test]
    fn test_warning_new_and_display() {
        let w = Warning::new(
            WarningLevel::Warning,
            WarningKind::MetadataAccessFailed,
            "Could not read file",
        );
        assert_eq!(w.level, WarningLevel::Warning);
        assert_eq!(w.kind, WarningKind::MetadataAccessFailed);
        assert_eq!(w.message, "Could not read file");
        assert!(w.context.is_none());
        assert_eq!(w.to_string(), "Could not read file");
    }

    #[test]
    fn test_warning_with_context() {
        let w = Warning::new(
            WarningLevel::Error,
            WarningKind::FileReadFailed,
            "Read failed",
        )
        .with_context("/some/path");
        assert_eq!(w.context, Some(PathBuf::from("/some/path")));
    }

    #[test]
    fn test_warning_serde_roundtrip() {
        let w = Warning::new(
            WarningLevel::Info,
            WarningKind::MetadataAccessFailed,
            "test msg",
        )
        .with_context("/path/to/file");
        let json = serde_json::to_string(&w).expect("serialize");
        let parsed: Warning = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.level, w.level);
        assert_eq!(parsed.kind, w.kind);
        assert_eq!(parsed.message, w.message);
        assert_eq!(parsed.context, w.context);
    }

    #[test]
    fn test_dependency_analysis_warning_messages() {
        let mut analysis = DependencyAnalysis::default();
        analysis.warnings.push(Warning::new(
            WarningLevel::Warning,
            WarningKind::MetadataAccessFailed,
            "msg1",
        ));
        analysis.warnings.push(Warning::new(
            WarningLevel::Error,
            WarningKind::FileReadFailed,
            "msg2",
        ));
        let messages = analysis.warning_messages();
        assert_eq!(messages, vec!["msg1", "msg2"]);
    }

    #[test]
    fn test_dependency_analysis_warnings_by_level() {
        let mut analysis = DependencyAnalysis::default();
        analysis.warnings.push(Warning::new(
            WarningLevel::Info,
            WarningKind::MetadataAccessFailed,
            "info msg",
        ));
        analysis.warnings.push(Warning::new(
            WarningLevel::Warning,
            WarningKind::MetadataAccessFailed,
            "warning msg",
        ));
        analysis.warnings.push(Warning::new(
            WarningLevel::Error,
            WarningKind::FileReadFailed,
            "error msg",
        ));

        assert_eq!(analysis.warnings_by_level(WarningLevel::Info).len(), 1);
        assert_eq!(analysis.warnings_by_level(WarningLevel::Warning).len(), 1);
        assert_eq!(analysis.warnings_by_level(WarningLevel::Error).len(), 1);
    }

    #[test]
    fn test_dependency_analysis_warnings_by_kind() {
        let mut analysis = DependencyAnalysis::default();
        analysis.warnings.push(Warning::new(
            WarningLevel::Warning,
            WarningKind::MetadataAccessFailed,
            "meta1",
        ));
        analysis.warnings.push(Warning::new(
            WarningLevel::Warning,
            WarningKind::MetadataAccessFailed,
            "meta2",
        ));
        analysis.warnings.push(Warning::new(
            WarningLevel::Error,
            WarningKind::FileReadFailed,
            "read",
        ));

        assert_eq!(
            analysis
                .warnings_by_kind(WarningKind::MetadataAccessFailed)
                .len(),
            2
        );
        assert_eq!(
            analysis.warnings_by_kind(WarningKind::FileReadFailed).len(),
            1
        );
        assert_eq!(
            analysis
                .warnings_by_kind(WarningKind::DirectoryEntryAccessFailed)
                .len(),
            0
        );
    }
}
