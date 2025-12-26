//! Skill creation via GitHub search or LLM generation.

mod cli_detector;
mod github_search;
mod llm_generator;

pub use cli_detector::{
    detect_cli_environment, get_available_cli, get_cli_binary, is_cli_available, CliEnvironment,
};
pub use github_search::{
    fetch_skill_content, search_github_skills, search_skills_advanced, GitHubSkillResult,
};
pub use llm_generator::{generate_skill_sync, generate_skill_with_llm};

use crate::context::ProjectProfile;
use serde::{Deserialize, Serialize};

/// Method for creating a new skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum CreationMethod {
    /// Search GitHub for existing skills.
    GitHubSearch,
    /// Generate using LLM.
    LLMGenerate,
    /// Default: both in sequence.
    #[default]
    Both,
}

impl std::fmt::Display for CreationMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHubSearch => write!(f, "github"),
            Self::LLMGenerate => write!(f, "llm"),
            Self::Both => write!(f, "both"),
        }
    }
}

impl std::str::FromStr for CreationMethod {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "github" | "search" => Ok(Self::GitHubSearch),
            "llm" | "generate" => Ok(Self::LLMGenerate),
            "both" | "all" => Ok(Self::Both),
            _ => Err(format!("Unknown creation method: {}", s)),
        }
    }
}

/// Request to create a new skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSkillRequest {
    /// Skill name or topic.
    pub name: String,
    /// Detailed description of what the skill should do.
    pub description: String,
    /// Target directory for the skill.
    pub target_dir: Option<String>,
    /// Creation method preference.
    pub method: CreationMethod,
    /// Project context to inform generation.
    pub project_context: Option<ProjectProfile>,
    /// Whether to preview without creating files.
    pub dry_run: bool,
}

impl CreateSkillRequest {
    /// Create a new request with defaults.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            target_dir: None,
            method: CreationMethod::Both,
            project_context: None,
            dry_run: false,
        }
    }

    /// Set the target directory.
    pub fn with_target_dir(mut self, dir: impl Into<String>) -> Self {
        self.target_dir = Some(dir.into());
        self
    }

    /// Set the creation method.
    pub fn with_method(mut self, method: CreationMethod) -> Self {
        self.method = method;
        self
    }

    /// Set project context.
    pub fn with_context(mut self, context: ProjectProfile) -> Self {
        self.project_context = Some(context);
        self
    }

    /// Enable dry run mode.
    pub fn dry_run(mut self) -> Self {
        self.dry_run = true;
        self
    }
}

/// Result of skill creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSkillResult {
    /// Whether creation succeeded.
    pub success: bool,
    /// Path to created skill.
    pub path: Option<String>,
    /// Creation method used.
    pub method_used: CreationMethod,
    /// Content generated/found.
    pub content: Option<String>,
    /// GitHub results if searched.
    pub github_results: Option<Vec<GitHubSkillResult>>,
    /// Error message if failed.
    pub error: Option<String>,
}

impl CreateSkillResult {
    /// Create a success result.
    pub fn success(method: CreationMethod, content: String, path: Option<String>) -> Self {
        Self {
            success: true,
            path,
            method_used: method,
            content: Some(content),
            github_results: None,
            error: None,
        }
    }

    /// Create a failure result.
    pub fn failure(method: CreationMethod, error: impl Into<String>) -> Self {
        Self {
            success: false,
            path: None,
            method_used: method,
            content: None,
            github_results: None,
            error: Some(error.into()),
        }
    }

    /// Create a result with GitHub search results.
    pub fn with_github_results(mut self, results: Vec<GitHubSkillResult>) -> Self {
        self.github_results = Some(results);
        self
    }
}
