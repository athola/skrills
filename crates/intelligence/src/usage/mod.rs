//! Usage analytics from Claude Code and Codex CLI session data.

mod analytics;
mod claude_parser;
mod codex_parser;

pub use analytics::{build_analytics, get_cooccurring_skills, recency_score};
pub use claude_parser::{parse_claude_command_history, parse_claude_sessions};
pub use codex_parser::{
    parse_codex_command_history, parse_codex_sessions, parse_codex_skills_history,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Aggregated skill usage statistics from session history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageAnalytics {
    /// Skill usage frequency (skill_path -> count).
    pub frequency: HashMap<String, u64>,
    /// Skill recency scores (skill_path -> last_used_timestamp).
    pub recency: HashMap<String, u64>,
    /// Skills commonly used together (skill_a -> (skill_b -> co-occurrence_count)).
    pub cooccurrence: HashMap<String, HashMap<String, u64>>,
    /// Prompt text -> skill affinities (for semantic matching).
    pub prompt_affinities: Vec<PromptAffinity>,
    /// User command history entries.
    pub command_history: Vec<CommandEntry>,
    /// Total sessions analyzed.
    pub sessions_analyzed: usize,
    /// Time range of analyzed data.
    pub time_range: Option<TimeRange>,
}

/// Prompt-to-skill affinity mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptAffinity {
    /// Keywords/trigrams extracted from prompts.
    pub keywords: Vec<String>,
    /// Skills invoked after this prompt type.
    pub associated_skills: Vec<String>,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f64,
}

/// A user command entered in the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEntry {
    /// The command text entered by the user.
    pub text: String,
    /// Unix timestamp when command was entered.
    pub timestamp: u64,
    /// Session ID this command belongs to.
    pub session_id: String,
    /// Project path if available.
    pub project: Option<String>,
}

/// Time range for analytics data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: u64,
    pub end: u64,
}

/// A single skill usage event extracted from session data.
#[derive(Debug, Clone)]
pub struct SkillUsageEvent {
    /// Unix timestamp of the event.
    pub timestamp: u64,
    /// Skill path or URI.
    pub skill_path: String,
    /// Session ID this event belongs to.
    pub session_id: String,
    /// Prompt context that led to this skill being used.
    pub prompt_context: Option<String>,
}
