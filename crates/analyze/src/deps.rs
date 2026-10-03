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
            // Only probe paths inside the skill directory. A target
            // that is absolute or climbs out with `..` stays unchecked
            // (`exists: None`) so skill content cannot use the
            // analyzer to test for arbitrary files.
            if !is_confined(&dep.target) {
                continue;
            }
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

        // Find markdown images first: LINK_REGEX also matches the
        // `[alt](path)` tail of an image, and whichever pass records a
        // path first decides its type.
        for cap in IMAGE_REGEX.captures_iter(line) {
            let Some(path) = local_link_target(&cap[2]) else {
                continue;
            };
            if seen_paths.insert(path.to_string()) {
                analysis.dependencies.push(Dependency {
                    dep_type: DependencyType::Asset,
                    target: path.to_string(),
                    line: Some(line_number),
                    exists: None,
                });
            }
        }

        // Find markdown links (excluding URLs and images)
        for cap in LINK_REGEX.captures_iter(line) {
            let start = cap.get(0).map_or(0, |m| m.start());
            if line[..start].ends_with('!') {
                continue;
            }
            let Some(path) = local_link_target(&cap[2]) else {
                continue;
            };
            if seen_paths.insert(path.to_string()) {
                analysis.dependencies.push(Dependency {
                    dep_type: classify_path(path),
                    target: path.to_string(),
                    line: Some(line_number),
                    exists: None,
                });
            }
        }
    }
}

/// Reduce a markdown link destination to the local file path it names,
/// or `None` when it names no local file.
///
/// Drops an optional title (`path "title"`), angle brackets, and any
/// `#fragment` or `?query`. Returns `None` for a same-page anchor
/// (`#usage`) and for any destination with a URI scheme (`https:`,
/// `mailto:`, `tel:`, ...).
fn local_link_target(raw: &str) -> Option<&str> {
    let dest = raw.trim();
    let dest = dest.split_whitespace().next().unwrap_or("");
    let dest = dest
        .strip_prefix('<')
        .and_then(|d| d.strip_suffix('>'))
        .unwrap_or(dest);
    if dest.starts_with('#') || has_uri_scheme(dest) {
        return None;
    }
    let path = dest.split(['#', '?']).next().unwrap_or("");
    (!path.is_empty()).then_some(path)
}

/// RFC 3986 scheme: a letter, then letters, digits, `+`, `-` or `.`,
/// then `:`. A single letter is treated as a Windows drive, not a
/// scheme.
fn has_uri_scheme(dest: &str) -> bool {
    let Some((scheme, _)) = dest.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    scheme.len() > 1
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Whether `target` stays inside the skill directory when joined to
/// it: relative, with no `..`, root or prefix component.
fn is_confined(target: &str) -> bool {
    Path::new(target).components().all(|c| {
        matches!(
            c,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    })
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
    fn anchors_schemes_fragments_and_titles_are_not_missing_files() {
        // IN-21: `[Usage](#usage)` was existence-checked as
        // `skill_dir/#usage` and reported missing.
        let tmp = tempfile::TempDir::new().unwrap();
        let skill = tmp.path().join("SKILL.md");
        std::fs::create_dir_all(tmp.path().join("references")).unwrap();
        std::fs::write(tmp.path().join("references/guide.md"), "g").unwrap();
        let content = "\
[Usage](#usage)
[Mail](mailto:someone@example.com)
[Section](references/guide.md#install)
[Query](references/guide.md?raw=1)
[Titled](references/guide.md \"The guide\")
";
        let analysis = analyze_dependencies(&skill, content);
        assert!(
            analysis.missing.is_empty(),
            "nothing here is a missing file: {:?}",
            analysis.missing
        );
        let targets: Vec<_> = analysis
            .dependencies
            .iter()
            .map(|d| d.target.as_str())
            .collect();
        assert_eq!(targets, ["references/guide.md"]);
    }

    #[test]
    fn link_targets_outside_the_skill_dir_are_not_probed() {
        // IN-67: `../../etc/shadow` or `/etc/passwd` must not turn the
        // analyzer into an existence oracle for arbitrary paths.
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(tmp.path().join("outside.md"), "x").unwrap();
        let content = "[up](../outside.md)\n[abs](/etc/passwd)\n";
        let analysis = analyze_dependencies(&skill_dir.join("SKILL.md"), content);
        for dep in &analysis.dependencies {
            assert_eq!(dep.exists, None, "{} was probed", dep.target);
        }
        assert!(analysis.missing.is_empty());
    }

    #[test]
    fn image_links_are_classified_as_assets() {
        // IN-68: the link pass matched `[d](diagram.dot)` inside the
        // image syntax first and classified it as a Reference.
        let mut analysis = DependencyAnalysis::default();
        extract_content_dependencies(&mut analysis, Path::new("."), "![d](diagram.dot)");
        assert_eq!(analysis.dependencies.len(), 1);
        assert_eq!(analysis.dependencies[0].dep_type, DependencyType::Asset);
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
