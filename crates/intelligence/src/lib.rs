//! Skill recommendations based on usage patterns and project context.
//!
//! This crate provides:
//! - Usage analytics from Claude Code and Codex CLI session data
//! - Project context analysis (languages, dependencies, frameworks)
//! - Recommendations combining multiple signals
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
    fetch_skill_content, generate_skill_sync, generate_skill_with_llm, get_available_cli,
    is_cli_available, search_github_skills, search_skills_advanced, CliEnvironment,
    CreateSkillRequest, CreateSkillResult, CreationMethod, GitHubSkillResult,
};
pub use recommend::{
    compute_similarity, find_similar_skills, has_similar_skill, match_skill,
    summarize_recommendations, MatchedField, RecommendationSignal, ScoreBreakdown, SkillGap,
    SkillGapAnalysis, SkillInfo, SkillMatch, SmartRecommendation, DEFAULT_THRESHOLD,
};
pub use usage::{
    build_analytics, get_cooccurring_skills, parse_claude_command_history, parse_claude_sessions,
    recency_score, CommandEntry, PromptAffinity, SkillUsageEvent, TimeRange, UsageAnalytics,
};
