//! Project context analysis for skill recommendations.

mod dependencies;
mod detector;
mod git_context;

pub use dependencies::{parse_cargo_toml, parse_package_json, parse_pyproject_toml};
pub use detector::{
    analyze_project, analyze_project_with_options, detect_frameworks, detect_languages,
    AnalyzeProjectOptions,
};
pub use git_context::extract_git_keywords;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Comprehensive project context profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectProfile {
    /// Detected programming languages with file counts.
    pub languages: HashMap<String, LanguageInfo>,
    /// Project dependencies by ecosystem.
    pub dependencies: HashMap<String, Vec<DependencyInfo>>,
    /// Project description extracted from README.
    pub description: Option<String>,
    /// Keywords extracted from README and docs.
    pub keywords: Vec<String>,
    /// Recent commit message keywords.
    pub git_keywords: Vec<String>,
    /// Detected frameworks and tools.
    pub frameworks: Vec<String>,
    /// Project type classification.
    pub project_type: ProjectType,
    /// Root directory analyzed.
    pub root: PathBuf,
}

/// Information about a detected programming language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageInfo {
    /// Number of files in this language.
    pub file_count: usize,
    /// File extensions associated with this language.
    pub extensions: Vec<String>,
    /// Whether this is the primary language.
    pub primary: bool,
}

/// Information about a project dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyInfo {
    /// Dependency name.
    pub name: String,
    /// Version constraint if available.
    pub version: Option<String>,
    /// Whether this is a dev dependency.
    pub dev: bool,
}

/// Classification of project type.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum ProjectType {
    /// Unknown project type.
    #[default]
    Unknown,
    /// A library/package.
    Library,
    /// An application.
    Application,
    /// A plugin or extension.
    Plugin,
    /// A monorepo with multiple projects.
    Monorepo,
    /// A service or API.
    Service,
}
