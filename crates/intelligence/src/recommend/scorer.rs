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
const SIMILARITY_WEIGHT: f64 = 2.5;

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
                    // Use log2(count + 1) to ensure: count=1 → 1.0, count=3 → 2.0, count=7 → 3.0
                    // This avoids the edge case where count=0 and count=1 produce identical scores
                    if *count > 0 {
                        breakdown.usage_score +=
                            COUSED_WEIGHT * ((*count + 1) as f64).log2().max(1.0);
                    }
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
                RecommendationSignal::SimilarityMatch { similarity, .. } => {
                    // Similarity is 0.0-1.0, scale by weight
                    breakdown.context_score += SIMILARITY_WEIGHT * similarity;
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
    use crate::context::LanguageInfo;

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

    // -------------------------------------------------------------------------
    // Edge Case Tests for Mathematical Correctness
    // -------------------------------------------------------------------------

    #[test]
    fn test_coused_count_zero_produces_no_score() {
        let scorer = RecommendationScorer::new();
        let signals = vec![RecommendationSignal::CoUsed { count: 0 }];
        let rec = scorer.score("skill://test/skill", signals);

        // count=0 should NOT contribute to usage score
        assert_eq!(
            rec.score_breakdown.usage_score, 0.0,
            "count=0 should produce no usage score"
        );
    }

    #[test]
    fn test_coused_count_one_different_from_zero() {
        let scorer = RecommendationScorer::new();

        let rec_zero = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::CoUsed { count: 0 }],
        );
        let rec_one = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::CoUsed { count: 1 }],
        );

        // count=1 should produce higher score than count=0
        assert!(
            rec_one.score > rec_zero.score,
            "count=1 ({}) should produce higher score than count=0 ({})",
            rec_one.score,
            rec_zero.score
        );
    }

    #[test]
    fn test_coused_logarithmic_scaling() {
        let scorer = RecommendationScorer::new();

        let rec_1 = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::CoUsed { count: 1 }],
        );
        let rec_3 = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::CoUsed { count: 3 }],
        );
        let rec_7 = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::CoUsed { count: 7 }],
        );

        // log2(2) = 1.0, log2(4) = 2.0, log2(8) = 3.0
        // So count=1→1.0, count=3→2.0, count=7→3.0 (using log2(count+1))
        assert!(
            rec_3.score > rec_1.score,
            "count=3 should score higher than count=1"
        );
        assert!(
            rec_7.score > rec_3.score,
            "count=7 should score higher than count=3"
        );

        // Verify logarithmic growth (not linear)
        let growth_1_to_3 = rec_3.score - rec_1.score;
        let growth_3_to_7 = rec_7.score - rec_3.score;

        // log2(4) - log2(2) = 1.0, log2(8) - log2(4) = 1.0
        // Growth should be approximately equal (both ~COUSED_WEIGHT)
        assert!(
            (growth_1_to_3 - growth_3_to_7).abs() < 0.1,
            "Logarithmic scaling should show equal growth increments"
        );
    }

    #[test]
    fn test_similarity_score_bounds() {
        let scorer = RecommendationScorer::new();

        // Test similarity at bounds
        let rec_zero = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::SimilarityMatch {
                query: "test".to_string(),
                similarity: 0.0,
            }],
        );
        let rec_one = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::SimilarityMatch {
                query: "test".to_string(),
                similarity: 1.0,
            }],
        );

        assert_eq!(
            rec_zero.score_breakdown.context_score, 0.0,
            "similarity=0.0 should produce 0.0 context score"
        );
        assert!(
            (rec_one.score_breakdown.context_score - SIMILARITY_WEIGHT).abs() < 0.001,
            "similarity=1.0 should produce SIMILARITY_WEIGHT context score"
        );
    }

    // -------------------------------------------------------------------------
    // Signal Type Tests - Test each signal type individually
    // -------------------------------------------------------------------------

    #[test]
    fn test_dependent_scoring() {
        let scorer = RecommendationScorer::new();
        let signals = vec![RecommendationSignal::Dependent];
        let rec = scorer.score("skill://test/skill", signals);

        assert_eq!(rec.score_breakdown.dependency_score, DEPENDENT_WEIGHT);
        assert_eq!(rec.score, DEPENDENT_WEIGHT);
    }

    #[test]
    fn test_sibling_scoring() {
        let scorer = RecommendationScorer::new();
        let signals = vec![RecommendationSignal::Sibling];
        let rec = scorer.score("skill://test/skill", signals);

        assert_eq!(rec.score_breakdown.dependency_score, SIBLING_WEIGHT);
        assert_eq!(rec.score, SIBLING_WEIGHT);
    }

    #[test]
    fn test_recently_used_scoring() {
        let scorer = RecommendationScorer::new();
        let signals = vec![RecommendationSignal::RecentlyUsed {
            last_used: 1700000000,
        }];
        let rec = scorer.score("skill://test/skill", signals);

        assert_eq!(rec.score_breakdown.usage_score, RECENCY_WEIGHT);
        assert_eq!(rec.score, RECENCY_WEIGHT);
    }

    #[test]
    fn test_prompt_match_scoring() {
        let scorer = RecommendationScorer::new();

        // Test with 1 keyword
        let rec_1 = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::PromptMatch {
                keywords: vec!["test".to_string()],
            }],
        );
        assert!(
            (rec_1.score_breakdown.context_score - PROMPT_MATCH_WEIGHT * 1.0).abs() < 0.001,
            "1 keyword should give PROMPT_MATCH_WEIGHT"
        );

        // Test with 3 keywords (max)
        let rec_3 = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::PromptMatch {
                keywords: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            }],
        );
        assert!(
            (rec_3.score_breakdown.context_score - PROMPT_MATCH_WEIGHT * 3.0).abs() < 0.001,
            "3 keywords should give PROMPT_MATCH_WEIGHT * 3.0"
        );

        // Test with more than 3 keywords (capped at 3)
        let rec_5 = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::PromptMatch {
                keywords: vec![
                    "a".to_string(),
                    "b".to_string(),
                    "c".to_string(),
                    "d".to_string(),
                    "e".to_string(),
                ],
            }],
        );
        assert!(
            (rec_5.score_breakdown.context_score - PROMPT_MATCH_WEIGHT * 3.0).abs() < 0.001,
            "5 keywords should be capped at PROMPT_MATCH_WEIGHT * 3.0"
        );
    }

    #[test]
    fn test_prompt_match_empty_keywords() {
        let scorer = RecommendationScorer::new();
        let rec = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::PromptMatch { keywords: vec![] }],
        );
        assert_eq!(
            rec.score_breakdown.context_score, 0.0,
            "Empty keywords should produce 0.0 context score"
        );
    }

    #[test]
    fn test_high_quality_scoring() {
        let scorer = RecommendationScorer::new();

        // Test quality score of 0.5
        let rec_half = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::HighQuality { score: 0.5 }],
        );
        assert!(
            (rec_half.score_breakdown.quality_score - QUALITY_WEIGHT * 0.5).abs() < 0.001,
            "quality=0.5 should give QUALITY_WEIGHT * 0.5"
        );

        // Test quality score of 1.0
        let rec_full = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::HighQuality { score: 1.0 }],
        );
        assert!(
            (rec_full.score_breakdown.quality_score - QUALITY_WEIGHT).abs() < 0.001,
            "quality=1.0 should give QUALITY_WEIGHT"
        );
    }

    #[test]
    fn test_project_match_multiple_technologies() {
        let scorer = RecommendationScorer::new();
        let signals = vec![RecommendationSignal::ProjectMatch {
            matched: vec!["Rust".to_string(), "Tokio".to_string(), "Serde".to_string()],
        }];
        let rec = scorer.score("skill://test/skill", signals);

        assert!(
            (rec.score_breakdown.context_score - CONTEXT_MATCH_WEIGHT * 3.0).abs() < 0.001,
            "3 project matches should give CONTEXT_MATCH_WEIGHT * 3.0"
        );
    }

    // -------------------------------------------------------------------------
    // Builder Pattern Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_with_usage() {
        let mut usage = UsageAnalytics::default();
        usage
            .recency
            .insert("skill://test/skill".to_string(), 1700000000);

        let scorer = RecommendationScorer::new().with_usage(usage);

        let recency = scorer.get_recency("skill://test/skill");
        assert_eq!(recency, Some(1700000000));
    }

    #[test]
    fn test_with_context() {
        let mut profile = ProjectProfile::default();
        profile.languages.insert(
            "Python".to_string(),
            LanguageInfo {
                file_count: 5,
                extensions: vec!["py".to_string()],
                primary: true,
            },
        );

        let scorer = RecommendationScorer::new().with_context(profile);
        let matches = scorer.get_project_matches("python-script");
        assert!(matches.contains(&"Python".to_string()));
    }

    #[test]
    fn test_with_quality_scores() {
        let mut quality_scores = HashMap::new();
        quality_scores.insert("skill://test/skill".to_string(), 0.85);

        let scorer = RecommendationScorer::new().with_quality_scores(quality_scores);

        // Score without explicit HighQuality signal should use quality_scores
        let rec = scorer.score("skill://test/skill", vec![]);
        assert!(
            (rec.score_breakdown.quality_score - QUALITY_WEIGHT * 0.85).abs() < 0.001,
            "Should use quality_scores when no HighQuality signal present"
        );
    }

    #[test]
    fn test_quality_score_not_overwritten_when_signal_present() {
        let mut quality_scores = HashMap::new();
        quality_scores.insert("skill://test/skill".to_string(), 0.5);

        let scorer = RecommendationScorer::new().with_quality_scores(quality_scores);

        // Score with explicit HighQuality signal should use signal, not quality_scores
        let rec = scorer.score(
            "skill://test/skill",
            vec![RecommendationSignal::HighQuality { score: 0.9 }],
        );
        assert!(
            (rec.score_breakdown.quality_score - QUALITY_WEIGHT * 0.9).abs() < 0.001,
            "Should use signal quality when HighQuality signal present"
        );
    }

    // -------------------------------------------------------------------------
    // Cooccurrence Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_cooccurrence() {
        let mut usage = UsageAnalytics::default();
        let mut inner = HashMap::new();
        inner.insert("skill://test/skill-b".to_string(), 5);
        usage
            .cooccurrence
            .insert("skill://test/skill-a".to_string(), inner);

        let scorer = RecommendationScorer::new().with_usage(usage);

        assert_eq!(
            scorer.get_cooccurrence("skill://test/skill-a", "skill://test/skill-b"),
            5
        );
        // Non-existent pairs should return 0
        assert_eq!(
            scorer.get_cooccurrence("skill://test/skill-a", "skill://test/nonexistent"),
            0
        );
        assert_eq!(
            scorer.get_cooccurrence("skill://test/nonexistent", "skill://test/skill-b"),
            0
        );
    }

    #[test]
    fn test_get_cooccurrence_no_usage() {
        let scorer = RecommendationScorer::new();
        assert_eq!(
            scorer.get_cooccurrence("skill://test/a", "skill://test/b"),
            0
        );
    }

    // -------------------------------------------------------------------------
    // Recency Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_get_recency() {
        let mut usage = UsageAnalytics::default();
        usage
            .recency
            .insert("skill://test/skill".to_string(), 1700000000);

        let scorer = RecommendationScorer::new().with_usage(usage);

        assert_eq!(scorer.get_recency("skill://test/skill"), Some(1700000000));
        assert_eq!(scorer.get_recency("skill://test/nonexistent"), None);
    }

    #[test]
    fn test_get_recency_no_usage() {
        let scorer = RecommendationScorer::new();
        assert_eq!(scorer.get_recency("skill://test/skill"), None);
    }

    // -------------------------------------------------------------------------
    // Enhance Signals Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_enhance_signals_adds_project_match() {
        let mut profile = ProjectProfile::default();
        profile.languages.insert(
            "Rust".to_string(),
            LanguageInfo {
                file_count: 10,
                extensions: vec!["rs".to_string()],
                primary: true,
            },
        );

        let scorer = RecommendationScorer::new().with_context(profile);
        let enhanced = scorer.enhance_signals("skill://test/rust-helper", vec![]);

        assert_eq!(enhanced.len(), 1);
        match &enhanced[0] {
            RecommendationSignal::ProjectMatch { matched } => {
                assert!(matched.contains(&"Rust".to_string()));
            }
            _ => panic!("Expected ProjectMatch signal"),
        }
    }

    #[test]
    fn test_enhance_signals_adds_recency() {
        let mut usage = UsageAnalytics::default();
        usage
            .recency
            .insert("skill://test/skill".to_string(), 1700000000);

        let scorer = RecommendationScorer::new().with_usage(usage);
        let enhanced = scorer.enhance_signals("skill://test/skill", vec![]);

        assert_eq!(enhanced.len(), 1);
        match &enhanced[0] {
            RecommendationSignal::RecentlyUsed { last_used } => {
                assert_eq!(*last_used, 1700000000);
            }
            _ => panic!("Expected RecentlyUsed signal"),
        }
    }

    #[test]
    fn test_enhance_signals_adds_high_quality() {
        let mut quality_scores = HashMap::new();
        quality_scores.insert("skill://test/skill".to_string(), 0.85);

        let scorer = RecommendationScorer::new().with_quality_scores(quality_scores);
        let enhanced = scorer.enhance_signals("skill://test/skill", vec![]);

        assert_eq!(enhanced.len(), 1);
        match &enhanced[0] {
            RecommendationSignal::HighQuality { score } => {
                assert!((score - 0.85).abs() < 0.001);
            }
            _ => panic!("Expected HighQuality signal"),
        }
    }

    #[test]
    fn test_enhance_signals_does_not_add_low_quality() {
        let mut quality_scores = HashMap::new();
        quality_scores.insert("skill://test/skill".to_string(), 0.5); // Below 0.7 threshold

        let scorer = RecommendationScorer::new().with_quality_scores(quality_scores);
        let enhanced = scorer.enhance_signals("skill://test/skill", vec![]);

        assert!(
            enhanced.is_empty(),
            "Quality below 0.7 should not add HighQuality signal"
        );
    }

    #[test]
    fn test_enhance_signals_does_not_add_zero_recency() {
        let mut usage = UsageAnalytics::default();
        usage.recency.insert("skill://test/skill".to_string(), 0); // Zero timestamp

        let scorer = RecommendationScorer::new().with_usage(usage);
        let enhanced = scorer.enhance_signals("skill://test/skill", vec![]);

        assert!(
            enhanced.is_empty(),
            "Zero recency should not add RecentlyUsed signal"
        );
    }

    #[test]
    fn test_enhance_signals_preserves_existing() {
        let mut profile = ProjectProfile::default();
        profile.languages.insert(
            "Rust".to_string(),
            LanguageInfo {
                file_count: 10,
                extensions: vec!["rs".to_string()],
                primary: true,
            },
        );

        let scorer = RecommendationScorer::new().with_context(profile);
        let existing = vec![RecommendationSignal::Dependency];
        let enhanced = scorer.enhance_signals("skill://test/rust-skill", existing);

        assert_eq!(enhanced.len(), 2);
        assert!(matches!(enhanced[0], RecommendationSignal::Dependency));
        assert!(matches!(
            enhanced[1],
            RecommendationSignal::ProjectMatch { .. }
        ));
    }

    #[test]
    fn test_enhance_signals_combines_all_sources() {
        let mut profile = ProjectProfile::default();
        profile.languages.insert(
            "Rust".to_string(),
            LanguageInfo {
                file_count: 10,
                extensions: vec!["rs".to_string()],
                primary: true,
            },
        );

        let mut usage = UsageAnalytics::default();
        usage
            .recency
            .insert("skill://test/rust-skill".to_string(), 1700000000);

        let mut quality_scores = HashMap::new();
        quality_scores.insert("skill://test/rust-skill".to_string(), 0.9);

        let scorer = RecommendationScorer::new()
            .with_context(profile)
            .with_usage(usage)
            .with_quality_scores(quality_scores);

        let enhanced = scorer.enhance_signals("skill://test/rust-skill", vec![]);

        assert_eq!(enhanced.len(), 3);
        // Should have ProjectMatch, RecentlyUsed, and HighQuality
        let has_project_match = enhanced
            .iter()
            .any(|s| matches!(s, RecommendationSignal::ProjectMatch { .. }));
        let has_recency = enhanced
            .iter()
            .any(|s| matches!(s, RecommendationSignal::RecentlyUsed { .. }));
        let has_quality = enhanced
            .iter()
            .any(|s| matches!(s, RecommendationSignal::HighQuality { .. }));

        assert!(has_project_match, "Should have ProjectMatch signal");
        assert!(has_recency, "Should have RecentlyUsed signal");
        assert!(has_quality, "Should have HighQuality signal");
    }

    // -------------------------------------------------------------------------
    // Project Matching Edge Cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_project_matches_no_context() {
        let scorer = RecommendationScorer::new();
        let matches = scorer.get_project_matches("rust-async-tokio");
        assert!(matches.is_empty());
    }

    #[test]
    fn test_project_matches_case_insensitive() {
        let mut profile = ProjectProfile::default();
        profile.languages.insert(
            "Rust".to_string(),
            LanguageInfo {
                file_count: 10,
                extensions: vec!["rs".to_string()],
                primary: true,
            },
        );

        let scorer = RecommendationScorer::new().with_context(profile);

        // Should match regardless of case
        let matches_lower = scorer.get_project_matches("rust-helper");
        let matches_upper = scorer.get_project_matches("RUST-helper");
        let matches_mixed = scorer.get_project_matches("RuSt-helper");

        assert!(matches_lower.contains(&"Rust".to_string()));
        assert!(matches_upper.contains(&"Rust".to_string()));
        assert!(matches_mixed.contains(&"Rust".to_string()));
    }

    #[test]
    fn test_project_matches_dependencies() {
        use crate::context::DependencyInfo;

        let mut profile = ProjectProfile::default();
        let deps = vec![
            DependencyInfo {
                name: "serde".to_string(),
                version: Some("1.0".to_string()),
                dev: false,
            },
            DependencyInfo {
                name: "tokio".to_string(),
                version: Some("1.0".to_string()),
                dev: false,
            },
        ];
        profile.dependencies.insert("rust".to_string(), deps);

        let scorer = RecommendationScorer::new().with_context(profile);
        let matches = scorer.get_project_matches("serde-json-helper");

        assert!(matches.contains(&"serde".to_string()));
    }

    #[test]
    fn test_project_matches_no_match() {
        let mut profile = ProjectProfile::default();
        profile.languages.insert(
            "Python".to_string(),
            LanguageInfo {
                file_count: 10,
                extensions: vec!["py".to_string()],
                primary: true,
            },
        );

        let scorer = RecommendationScorer::new().with_context(profile);
        let matches = scorer.get_project_matches("rust-skill");

        assert!(matches.is_empty());
    }

    // -------------------------------------------------------------------------
    // Score Breakdown and Total Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_score_total_equals_components_sum() {
        let scorer = RecommendationScorer::new();
        let signals = vec![
            RecommendationSignal::Dependency,
            RecommendationSignal::CoUsed { count: 4 },
            RecommendationSignal::ProjectMatch {
                matched: vec!["Rust".to_string(), "Tokio".to_string()],
            },
            RecommendationSignal::HighQuality { score: 0.8 },
        ];
        let rec = scorer.score("skill://test/skill", signals);

        let expected_total = rec.score_breakdown.dependency_score
            + rec.score_breakdown.usage_score
            + rec.score_breakdown.context_score
            + rec.score_breakdown.quality_score;

        assert!(
            (rec.score - expected_total).abs() < 0.001,
            "Total score should equal sum of breakdown components"
        );
    }

    #[test]
    fn test_all_signals_combined() {
        let scorer = RecommendationScorer::new();
        let signals = vec![
            RecommendationSignal::Dependency,
            RecommendationSignal::Dependent,
            RecommendationSignal::Sibling,
            RecommendationSignal::CoUsed { count: 3 },
            RecommendationSignal::ProjectMatch {
                matched: vec!["Rust".to_string()],
            },
            RecommendationSignal::RecentlyUsed {
                last_used: 1700000000,
            },
            RecommendationSignal::PromptMatch {
                keywords: vec!["test".to_string(), "example".to_string()],
            },
            RecommendationSignal::HighQuality { score: 0.9 },
            RecommendationSignal::SimilarityMatch {
                query: "test query".to_string(),
                similarity: 0.75,
            },
        ];
        let rec = scorer.score("skill://test/skill", signals);

        // Verify all breakdown components have values
        assert!(
            rec.score_breakdown.dependency_score > 0.0,
            "dependency_score should be positive"
        );
        assert!(
            rec.score_breakdown.usage_score > 0.0,
            "usage_score should be positive"
        );
        assert!(
            rec.score_breakdown.context_score > 0.0,
            "context_score should be positive"
        );
        assert!(
            rec.score_breakdown.quality_score > 0.0,
            "quality_score should be positive"
        );

        // Verify total is reasonable
        assert!(rec.score > 0.0, "Total score should be positive");
    }

    // -------------------------------------------------------------------------
    // Extract Skill Name Edge Cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_skill_name_edge_cases() {
        assert_eq!(extract_skill_name(""), "");
        assert_eq!(extract_skill_name("/"), "");
        assert_eq!(extract_skill_name("//"), "");
        assert_eq!(extract_skill_name("name"), "name");
        assert_eq!(extract_skill_name("skill://a/b/c/name"), "name");
    }

    // -------------------------------------------------------------------------
    // Multiple Signals of Same Type
    // -------------------------------------------------------------------------

    #[test]
    fn test_multiple_dependency_signals() {
        let scorer = RecommendationScorer::new();
        let signals = vec![
            RecommendationSignal::Dependency,
            RecommendationSignal::Dependency,
        ];
        let rec = scorer.score("skill://test/skill", signals);

        assert!(
            (rec.score_breakdown.dependency_score - DEPENDENCY_WEIGHT * 2.0).abs() < 0.001,
            "Multiple Dependency signals should accumulate"
        );
    }

    #[test]
    fn test_multiple_coused_signals() {
        let scorer = RecommendationScorer::new();
        let signals = vec![
            RecommendationSignal::CoUsed { count: 3 },
            RecommendationSignal::CoUsed { count: 7 },
        ];
        let rec = scorer.score("skill://test/skill", signals);

        // Both should contribute to usage_score
        // log2(4) = 2.0, log2(8) = 3.0
        let expected = COUSED_WEIGHT * 2.0 + COUSED_WEIGHT * 3.0;
        assert!(
            (rec.score_breakdown.usage_score - expected).abs() < 0.001,
            "Multiple CoUsed signals should accumulate"
        );
    }

    // -------------------------------------------------------------------------
    // SmartRecommendation Structure Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_recommendation_structure() {
        let scorer = RecommendationScorer::new();
        let signals = vec![RecommendationSignal::Dependency];
        let rec = scorer.score("skill://namespace/path/my-skill", signals.clone());

        assert_eq!(rec.uri, "skill://namespace/path/my-skill");
        assert_eq!(rec.name, "my-skill");
        assert!(
            !rec.explanation.is_empty(),
            "Explanation should not be empty"
        );
        assert_eq!(rec.signals.len(), 1);
    }

    // -------------------------------------------------------------------------
    // Weight Constants Verification
    // -------------------------------------------------------------------------

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_weight_constants_are_positive() {
        assert!(DEPENDENCY_WEIGHT > 0.0);
        assert!(DEPENDENT_WEIGHT > 0.0);
        assert!(SIBLING_WEIGHT > 0.0);
        assert!(COUSED_WEIGHT > 0.0);
        assert!(CONTEXT_MATCH_WEIGHT > 0.0);
        assert!(RECENCY_WEIGHT > 0.0);
        assert!(PROMPT_MATCH_WEIGHT > 0.0);
        assert!(QUALITY_WEIGHT > 0.0);
        assert!(SIMILARITY_WEIGHT > 0.0);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_dependency_weights_ordering() {
        // Dependency should be highest, then Dependent, then Sibling
        assert!(
            DEPENDENCY_WEIGHT > DEPENDENT_WEIGHT,
            "Dependency weight should be higher than Dependent"
        );
        assert!(
            DEPENDENT_WEIGHT > SIBLING_WEIGHT,
            "Dependent weight should be higher than Sibling"
        );
    }
}
