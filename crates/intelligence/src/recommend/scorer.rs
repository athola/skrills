//! Multi-signal recommendation scoring.

use super::{explainer, RecommendationSignal, ScoreBreakdown, SmartRecommendation};
use crate::context::ProjectProfile;
use crate::usage::UsageAnalytics;
use std::collections::HashMap;

/// Weights for different recommendation signals.
const DEPENDENCY_WEIGHT: f64 = 3.0;
const DEPENDENT_WEIGHT: f64 = 2.0;
const SIBLING_WEIGHT: f64 = 1.0;
const COUSED_WEIGHT: f64 = 2.5;
const CONTEXT_MATCH_WEIGHT: f64 = 2.0;
const RECENCY_WEIGHT: f64 = 1.5;
const PROMPT_MATCH_WEIGHT: f64 = 1.5;
const QUALITY_WEIGHT: f64 = 1.0;

/// Trait for computing recommendation scores.
pub trait Scorer {
    /// Score a skill based on available signals.
    fn score(&self, uri: &str, signals: Vec<RecommendationSignal>) -> SmartRecommendation;
}

/// Recommendation scorer that combines multiple signals.
#[derive(Debug, Default)]
pub struct RecommendationScorer {
    /// Usage analytics data.
    usage: Option<UsageAnalytics>,
    /// Project context data.
    context: Option<ProjectProfile>,
    /// Quality scores by skill URI.
    quality_scores: HashMap<String, f64>,
}

impl RecommendationScorer {
    /// Create a new scorer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add usage analytics data.
    pub fn with_usage(mut self, usage: UsageAnalytics) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Add project context data.
    pub fn with_context(mut self, context: ProjectProfile) -> Self {
        self.context = Some(context);
        self
    }

    /// Add quality scores for skills.
    pub fn with_quality_scores(mut self, scores: HashMap<String, f64>) -> Self {
        self.quality_scores = scores;
        self
    }

    /// Get co-occurrence count for two skills.
    pub fn get_cooccurrence(&self, skill_a: &str, skill_b: &str) -> u64 {
        self.usage
            .as_ref()
            .and_then(|u| u.cooccurrence.get(skill_a))
            .and_then(|coocs| coocs.get(skill_b))
            .copied()
            .unwrap_or(0)
    }

    /// Get recency timestamp for a skill.
    pub fn get_recency(&self, skill: &str) -> Option<u64> {
        self.usage
            .as_ref()
            .and_then(|u| u.recency.get(skill))
            .copied()
    }

    /// Check if skill matches project technologies.
    pub fn get_project_matches(&self, skill_name: &str) -> Vec<String> {
        let mut matches = Vec::new();

        if let Some(ref ctx) = self.context {
            let skill_lower = skill_name.to_lowercase();

            // Check language matches
            for lang in ctx.languages.keys() {
                if skill_lower.contains(&lang.to_lowercase()) {
                    matches.push(lang.clone());
                }
            }

            // Check framework matches
            for framework in &ctx.frameworks {
                if skill_lower.contains(&framework.to_lowercase()) {
                    matches.push(framework.clone());
                }
            }

            // Check dependency name matches
            for deps in ctx.dependencies.values() {
                for dep in deps {
                    if skill_lower.contains(&dep.name.to_lowercase()) {
                        matches.push(dep.name.clone());
                    }
                }
            }
        }

        matches
    }

    /// Enhance signals with usage and context data.
    pub fn enhance_signals(
        &self,
        uri: &str,
        mut signals: Vec<RecommendationSignal>,
    ) -> Vec<RecommendationSignal> {
        let skill_name = extract_skill_name(uri);

        // Add project match signal if applicable
        let project_matches = self.get_project_matches(&skill_name);
        if !project_matches.is_empty() {
            signals.push(RecommendationSignal::ProjectMatch {
                matched: project_matches,
            });
        }

        // Add recency signal if applicable
        if let Some(last_used) = self.get_recency(uri) {
            if last_used > 0 {
                signals.push(RecommendationSignal::RecentlyUsed { last_used });
            }
        }

        // Add quality signal if high
        if let Some(&quality) = self.quality_scores.get(uri) {
            if quality >= 0.7 {
                signals.push(RecommendationSignal::HighQuality { score: quality });
            }
        }

        signals
    }
}

impl Scorer for RecommendationScorer {
    fn score(&self, uri: &str, signals: Vec<RecommendationSignal>) -> SmartRecommendation {
        let mut breakdown = ScoreBreakdown::default();

        for signal in &signals {
            match signal {
                RecommendationSignal::Dependency => {
                    breakdown.dependency_score += DEPENDENCY_WEIGHT;
                }
                RecommendationSignal::Dependent => {
                    breakdown.dependency_score += DEPENDENT_WEIGHT;
                }
                RecommendationSignal::Sibling => {
                    breakdown.dependency_score += SIBLING_WEIGHT;
                }
                RecommendationSignal::CoUsed { count } => {
                    // Logarithmic scaling for co-occurrence
                    breakdown.usage_score += COUSED_WEIGHT * (*count as f64).log2().max(1.0);
                }
                RecommendationSignal::ProjectMatch { matched } => {
                    breakdown.context_score += CONTEXT_MATCH_WEIGHT * matched.len() as f64;
                }
                RecommendationSignal::RecentlyUsed { .. } => {
                    breakdown.usage_score += RECENCY_WEIGHT;
                }
                RecommendationSignal::PromptMatch { keywords } => {
                    breakdown.context_score +=
                        PROMPT_MATCH_WEIGHT * (keywords.len() as f64).min(3.0);
                }
                RecommendationSignal::HighQuality { score } => {
                    breakdown.quality_score += QUALITY_WEIGHT * score;
                }
            }
        }

        // Add base quality score if available
        if let Some(&quality) = self.quality_scores.get(uri) {
            if breakdown.quality_score == 0.0 {
                breakdown.quality_score = quality * QUALITY_WEIGHT;
            }
        }

        let total_score = breakdown.total();
        let explanation = explainer::generate_explanation(&signals);

        SmartRecommendation {
            uri: uri.to_string(),
            name: extract_skill_name(uri),
            score: total_score,
            score_breakdown: breakdown,
            explanation,
            signals,
            quality_score: self.quality_scores.get(uri).copied(),
        }
    }
}

/// Extract skill name from URI.
fn extract_skill_name(uri: &str) -> String {
    uri.rsplit('/').next().unwrap_or(uri).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_signals() {
        let scorer = RecommendationScorer::new();
        let rec = scorer.score("skill://test/skill", vec![]);

        assert_eq!(rec.uri, "skill://test/skill");
        assert_eq!(rec.name, "skill");
        assert_eq!(rec.score, 0.0);
    }

    #[test]
    fn test_dependency_scoring() {
        let scorer = RecommendationScorer::new();
        let signals = vec![RecommendationSignal::Dependency];
        let rec = scorer.score("skill://test/skill", signals);

        assert_eq!(rec.score_breakdown.dependency_score, DEPENDENCY_WEIGHT);
        assert_eq!(rec.score, DEPENDENCY_WEIGHT);
    }

    #[test]
    fn test_combined_signals() {
        let scorer = RecommendationScorer::new();
        let signals = vec![
            RecommendationSignal::Dependency,
            RecommendationSignal::CoUsed { count: 4 }, // log2(4) = 2
            RecommendationSignal::ProjectMatch {
                matched: vec!["Rust".to_string()],
            },
        ];
        let rec = scorer.score("skill://test/rust-skill", signals);

        assert!(rec.score > 0.0);
        assert!(rec.score_breakdown.dependency_score > 0.0);
        assert!(rec.score_breakdown.usage_score > 0.0);
        assert!(rec.score_breakdown.context_score > 0.0);
    }

    #[test]
    fn test_extract_skill_name() {
        assert_eq!(extract_skill_name("skill://source/path/name"), "name");
        assert_eq!(extract_skill_name("simple-name"), "simple-name");
    }

    #[test]
    fn test_project_matches() {
        let mut profile = ProjectProfile::default();
        profile.languages.insert(
            "Rust".to_string(),
            crate::context::LanguageInfo {
                file_count: 10,
                extensions: vec!["rs".to_string()],
                primary: true,
            },
        );
        profile.frameworks.push("Tokio".to_string());

        let scorer = RecommendationScorer::new().with_context(profile);

        let matches = scorer.get_project_matches("rust-async-tokio");
        assert!(matches.contains(&"Rust".to_string()));
        assert!(matches.contains(&"Tokio".to_string()));
    }
}
