//! Dependency analysis for skills.
//!
//! Analyzes skill dependencies including:
//! - Local file references (modules/, references/, scripts/, assets/)
//! - External links and URLs
//! - Cross-skill references

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

/// Types of dependencies a skill can have.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
            for entry in WalkDir::new(&dir_path).into_iter().filter_map(|e| e.ok()) {
                if entry.path().is_file() {
                    if let Ok(meta) = entry.metadata() {
                        analysis.total_dep_size += meta.len();
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

static URL_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://[^\s\)\]>]+").unwrap());
static LINK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").unwrap());
static IMAGE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").unwrap());

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

/// Get all files in a skill's dependency directories.
pub fn list_dependency_files(skill_path: &Path) -> Vec<PathBuf> {
    let skill_dir = skill_path.parent().unwrap_or(Path::new("."));
    let mut files = Vec::new();

    for subdir in &["modules", "references", "scripts", "assets"] {
        let dir_path = skill_dir.join(subdir);
        if dir_path.exists() {
            for entry in WalkDir::new(&dir_path).into_iter().filter_map(|e| e.ok()) {
                if entry.path().is_file() {
                    files.push(entry.path().to_path_buf());
                }
            }
        }
    }

    files
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
}
