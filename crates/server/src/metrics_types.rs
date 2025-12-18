//! Metrics types for skill statistics.
//!
//! These types are used by both the CLI metrics command and the MCP skill-metrics tool.

use serde::Serialize;
use std::collections::HashMap;

/// Aggregate statistics about discovered skills.
#[derive(Debug, Clone, Serialize)]
pub struct SkillMetrics {
    /// Total number of discovered skills.
    pub total_skills: usize,
    /// Count of skills by source (claude, codex, marketplace, etc.).
    pub by_source: HashMap<String, usize>,
    /// Distribution of skills by quality score.
    pub by_quality: QualityDistribution,
    /// Dependency graph statistics.
    pub dependency_stats: DependencyStats,
    /// Token usage statistics.
    pub token_stats: TokenStats,
    /// Optional validation summary (requires extra computation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_summary: Option<MetricsValidationSummary>,
}

/// Distribution of skills by quality score buckets.
#[derive(Debug, Clone, Serialize)]
pub struct QualityDistribution {
    /// Skills with quality >= 0.8.
    pub high: usize,
    /// Skills with quality >= 0.5 and < 0.8.
    pub medium: usize,
    /// Skills with quality < 0.5.
    pub low: usize,
}

/// Statistics about skill dependencies.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyStats {
    /// Total dependency edges across all skills.
    pub total_dependencies: usize,
    /// Average dependencies per skill.
    pub avg_per_skill: f64,
    /// Skills with no dependencies and no dependents.
    pub orphan_count: usize,
    /// Skills with the most dependents (hub skills).
    pub hub_skills: Vec<HubSkill>,
}

/// A hub skill with its dependent count.
#[derive(Debug, Clone, Serialize)]
pub struct HubSkill {
    /// Skill URI.
    pub uri: String,
    /// Number of skills that depend on this one.
    pub dependent_count: usize,
}

/// Token usage statistics.
#[derive(Debug, Clone, Serialize)]
pub struct TokenStats {
    /// Total tokens across all skills.
    pub total_tokens: usize,
    /// Average tokens per skill.
    pub avg_per_skill: usize,
    /// The skill with the most tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub largest_skill: Option<SkillTokenInfo>,
}

/// Token info for a single skill.
#[derive(Debug, Clone, Serialize)]
pub struct SkillTokenInfo {
    /// Skill URI.
    pub uri: String,
    /// Token count.
    pub tokens: usize,
}

/// Validation summary for metrics.
#[derive(Debug, Clone, Serialize)]
pub struct MetricsValidationSummary {
    /// Skills passing validation.
    pub passing: usize,
    /// Skills with errors.
    pub with_errors: usize,
    /// Skills with warnings only.
    pub with_warnings: usize,
}
