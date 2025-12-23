//! Intelligent skill recommendations based on usage patterns and project context.
//!
//! This crate provides:
//! - Usage analytics from Claude Code and Codex CLI session data
//! - Project context analysis (languages, dependencies, frameworks)
//! - Smart skill recommendations combining multiple signals
//! - Skill creation via GitHub search or LLM generation

pub mod context;
pub mod create;
pub mod recommend;
pub mod usage;

pub use context::{
    analyze_project, analyze_project_with_options, AnalyzeProjectOptions, DependencyInfo,
    LanguageInfo, ProjectProfile, ProjectType,
};
pub use create::{
    generate_skill_sync, search_github_skills, CliEnvironment, CreateSkillRequest,
    CreateSkillResult, CreationMethod, GitHubSkillResult,
};
pub use recommend::{
    RecommendationSignal, ScoreBreakdown, SkillGap, SkillGapAnalysis, SmartRecommendation,
};
pub use usage::{
    build_analytics, CommandEntry, PromptAffinity, SkillUsageEvent, TimeRange, UsageAnalytics,
};
