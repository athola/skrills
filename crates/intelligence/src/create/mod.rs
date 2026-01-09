//! Skill creation via GitHub search or LLM generation.

mod cli_detector;
pub mod empirical;
mod github_search;
mod llm_generator;

pub use cli_detector::{
    detect_cli_environment, get_available_cli, get_cli_binary, is_cli_available, CliEnvironment,
};
pub use empirical::{
    cluster_sessions, format_as_skill_md, generate_skill_from_cluster, ClusteredBehavior,
    ClusteringResult, EmpiricalSkillContent, FailurePattern, SuccessPattern,
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
    /// Generate from empirical session patterns.
    Empirical,
    /// Default: both GitHub search and LLM in sequence.
    #[default]
    Both,
}

impl std::fmt::Display for CreationMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GitHubSearch => write!(f, "github"),
            Self::LLMGenerate => write!(f, "llm"),
            Self::Empirical => write!(f, "empirical"),
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
            "empirical" | "pattern" | "patterns" => Ok(Self::Empirical),
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

#[cfg(test)]
mod tests {
    use super::*;

    mod creation_method {
        use super::*;

        #[test]
        fn default_is_both() {
            assert_eq!(CreationMethod::default(), CreationMethod::Both);
        }

        #[test]
        fn display_github_search() {
            assert_eq!(CreationMethod::GitHubSearch.to_string(), "github");
        }

        #[test]
        fn display_llm_generate() {
            assert_eq!(CreationMethod::LLMGenerate.to_string(), "llm");
        }

        #[test]
        fn display_empirical() {
            assert_eq!(CreationMethod::Empirical.to_string(), "empirical");
        }

        #[test]
        fn display_both() {
            assert_eq!(CreationMethod::Both.to_string(), "both");
        }

        #[test]
        fn from_str_github_variants() {
            assert_eq!(
                "github".parse::<CreationMethod>().unwrap(),
                CreationMethod::GitHubSearch
            );
            assert_eq!(
                "search".parse::<CreationMethod>().unwrap(),
                CreationMethod::GitHubSearch
            );
            assert_eq!(
                "GITHUB".parse::<CreationMethod>().unwrap(),
                CreationMethod::GitHubSearch
            );
        }

        #[test]
        fn from_str_llm_variants() {
            assert_eq!(
                "llm".parse::<CreationMethod>().unwrap(),
                CreationMethod::LLMGenerate
            );
            assert_eq!(
                "generate".parse::<CreationMethod>().unwrap(),
                CreationMethod::LLMGenerate
            );
            assert_eq!(
                "LLM".parse::<CreationMethod>().unwrap(),
                CreationMethod::LLMGenerate
            );
        }

        #[test]
        fn from_str_empirical_variants() {
            assert_eq!(
                "empirical".parse::<CreationMethod>().unwrap(),
                CreationMethod::Empirical
            );
            assert_eq!(
                "pattern".parse::<CreationMethod>().unwrap(),
                CreationMethod::Empirical
            );
            assert_eq!(
                "patterns".parse::<CreationMethod>().unwrap(),
                CreationMethod::Empirical
            );
        }

        #[test]
        fn from_str_both_variants() {
            assert_eq!(
                "both".parse::<CreationMethod>().unwrap(),
                CreationMethod::Both
            );
            assert_eq!(
                "all".parse::<CreationMethod>().unwrap(),
                CreationMethod::Both
            );
        }

        #[test]
        fn from_str_invalid() {
            let result = "invalid".parse::<CreationMethod>();
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                "Unknown creation method: invalid".to_string()
            );
        }

        #[test]
        fn serialize_deserialize_roundtrip() {
            let methods = [
                CreationMethod::GitHubSearch,
                CreationMethod::LLMGenerate,
                CreationMethod::Empirical,
                CreationMethod::Both,
            ];

            for method in methods {
                let json = serde_json::to_string(&method).unwrap();
                let parsed: CreationMethod = serde_json::from_str(&json).unwrap();
                assert_eq!(method, parsed);
            }
        }

        #[test]
        fn clone_and_debug() {
            let method = CreationMethod::LLMGenerate;
            let cloned = method.clone();
            assert_eq!(method, cloned);

            let debug = format!("{:?}", method);
            assert!(debug.contains("LLMGenerate"));
        }
    }

    mod create_skill_request {
        use super::*;

        #[test]
        fn new_sets_name_and_description() {
            let request = CreateSkillRequest::new("test-skill", "A test skill");

            assert_eq!(request.name, "test-skill");
            assert_eq!(request.description, "A test skill");
        }

        #[test]
        fn new_has_default_values() {
            let request = CreateSkillRequest::new("name", "desc");

            assert!(request.target_dir.is_none());
            assert_eq!(request.method, CreationMethod::Both);
            assert!(request.project_context.is_none());
            assert!(!request.dry_run);
        }

        #[test]
        fn with_target_dir() {
            let request =
                CreateSkillRequest::new("name", "desc").with_target_dir("/path/to/skills");

            assert_eq!(request.target_dir, Some("/path/to/skills".to_string()));
        }

        #[test]
        fn with_method() {
            let request =
                CreateSkillRequest::new("name", "desc").with_method(CreationMethod::GitHubSearch);

            assert_eq!(request.method, CreationMethod::GitHubSearch);
        }

        #[test]
        fn dry_run_enables_flag() {
            let request = CreateSkillRequest::new("name", "desc").dry_run();

            assert!(request.dry_run);
        }

        #[test]
        fn builder_chaining() {
            let request = CreateSkillRequest::new("chained-skill", "Full builder test")
                .with_target_dir("/skills")
                .with_method(CreationMethod::LLMGenerate)
                .dry_run();

            assert_eq!(request.name, "chained-skill");
            assert_eq!(request.description, "Full builder test");
            assert_eq!(request.target_dir, Some("/skills".to_string()));
            assert_eq!(request.method, CreationMethod::LLMGenerate);
            assert!(request.dry_run);
        }

        #[test]
        fn serialize_deserialize_roundtrip() {
            let request = CreateSkillRequest::new("serialization-test", "Testing JSON")
                .with_target_dir("/test")
                .with_method(CreationMethod::Empirical)
                .dry_run();

            let json = serde_json::to_string(&request).unwrap();
            let parsed: CreateSkillRequest = serde_json::from_str(&json).unwrap();

            assert_eq!(request.name, parsed.name);
            assert_eq!(request.description, parsed.description);
            assert_eq!(request.target_dir, parsed.target_dir);
            assert_eq!(request.method, parsed.method);
            assert_eq!(request.dry_run, parsed.dry_run);
        }

        #[test]
        fn clone_and_debug() {
            let request = CreateSkillRequest::new("debug-test", "Testing debug");
            let cloned = request.clone();

            assert_eq!(request.name, cloned.name);

            let debug = format!("{:?}", request);
            assert!(debug.contains("debug-test"));
        }
    }

    mod create_skill_result {
        use super::*;

        #[test]
        fn success_factory() {
            let result = CreateSkillResult::success(
                CreationMethod::LLMGenerate,
                "# Skill content".to_string(),
                Some("/path/skill.md".to_string()),
            );

            assert!(result.success);
            assert_eq!(result.method_used, CreationMethod::LLMGenerate);
            assert_eq!(result.content, Some("# Skill content".to_string()));
            assert_eq!(result.path, Some("/path/skill.md".to_string()));
            assert!(result.github_results.is_none());
            assert!(result.error.is_none());
        }

        #[test]
        fn success_without_path() {
            let result =
                CreateSkillResult::success(CreationMethod::Both, "content".to_string(), None);

            assert!(result.success);
            assert!(result.path.is_none());
            assert!(result.content.is_some());
        }

        #[test]
        fn failure_factory() {
            let result =
                CreateSkillResult::failure(CreationMethod::GitHubSearch, "API rate limit exceeded");

            assert!(!result.success);
            assert_eq!(result.method_used, CreationMethod::GitHubSearch);
            assert_eq!(result.error, Some("API rate limit exceeded".to_string()));
            assert!(result.path.is_none());
            assert!(result.content.is_none());
            assert!(result.github_results.is_none());
        }

        #[test]
        fn with_github_results() {
            let github_result = GitHubSkillResult {
                repo_url: "https://github.com/owner/repo".to_string(),
                skill_path: "SKILL.md".to_string(),
                file_url: "https://github.com/owner/repo/blob/main/SKILL.md".to_string(),
                description: Some("A skill".to_string()),
                stars: 42,
                last_updated: "2024-01-01T00:00:00Z".to_string(),
                raw_url: Some(
                    "https://raw.githubusercontent.com/owner/repo/main/SKILL.md".to_string(),
                ),
            };

            let result = CreateSkillResult::success(
                CreationMethod::GitHubSearch,
                "content".to_string(),
                None,
            )
            .with_github_results(vec![github_result.clone()]);

            assert!(result.github_results.is_some());
            let results = result.github_results.unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].skill_path, "SKILL.md");
            assert_eq!(results[0].stars, 42);
        }

        #[test]
        fn serialize_deserialize_roundtrip() {
            let result = CreateSkillResult::success(
                CreationMethod::Empirical,
                "Empirical content".to_string(),
                Some("/empirical/skill.md".to_string()),
            );

            let json = serde_json::to_string(&result).unwrap();
            let parsed: CreateSkillResult = serde_json::from_str(&json).unwrap();

            assert_eq!(result.success, parsed.success);
            assert_eq!(result.method_used, parsed.method_used);
            assert_eq!(result.content, parsed.content);
            assert_eq!(result.path, parsed.path);
        }

        #[test]
        fn clone_and_debug() {
            let result = CreateSkillResult::failure(CreationMethod::Both, "test error");
            let cloned = result.clone();

            assert_eq!(result.success, cloned.success);
            assert_eq!(result.error, cloned.error);

            let debug = format!("{:?}", result);
            assert!(debug.contains("test error"));
        }
    }
}
