//! Smart skill recommendations combining multiple signals.

pub mod comparative;
mod explainer;
mod scorer;
pub mod similarity;

pub use comparative::{
    compute_deviation_score, compute_effectiveness, get_baseline_expectations,
    infer_skill_category, DeviationEvidence, DeviationScore, EffectivenessMetric, ExpectedOutcome,
    OutcomeMetrics, SkillCategory,
};
pub use explainer::{generate_explanation, summarize_recommendations};
pub use scorer::{RecommendationScorer, Scorer};
pub use similarity::{
    compute_similarity, find_similar_skills, has_similar_skill, match_skill, MatchedField,
    SkillInfo, SkillMatch, DEFAULT_THRESHOLD,
};

use serde::{Deserialize, Serialize};

/// Enhanced recommendation with multiple signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartRecommendation {
    /// Skill URI.
    pub uri: String,
    /// Skill name.
    pub name: String,
    /// Combined score (0.0 - 10.0).
    pub score: f64,
    /// Breakdown of score components.
    pub score_breakdown: ScoreBreakdown,
    /// Human-readable explanation.
    pub explanation: String,
    /// Recommendation source signals.
    pub signals: Vec<RecommendationSignal>,
}

/// Breakdown of recommendation score components.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    /// Score from dependency relationships.
    pub dependency_score: f64,
    /// Score from usage patterns.
    pub usage_score: f64,
    /// Score from project context match.
    pub context_score: f64,
    /// Score from quality metrics.
    pub quality_score: f64,
}

impl ScoreBreakdown {
    /// Calculate total score.
    pub fn total(&self) -> f64 {
        self.dependency_score + self.usage_score + self.context_score + self.quality_score
    }
}

/// Signals that contribute to a recommendation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecommendationSignal {
    /// Skill is a dependency of the target.
    Dependency,
    /// Skill depends on the target.
    Dependent,
    /// Skill shares dependencies (sibling).
    Sibling,
    /// Skill frequently used together.
    CoUsed {
        /// Number of co-occurrences.
        count: u64,
    },
    /// Skill matches project language/framework.
    ProjectMatch {
        /// Matched technologies.
        matched: Vec<String>,
    },
    /// Skill recently used in this project.
    RecentlyUsed {
        /// Last used timestamp.
        last_used: u64,
    },
    /// Skill matches prompt keywords.
    PromptMatch {
        /// Matched keywords.
        keywords: Vec<String>,
    },
    /// Skill has high quality score.
    HighQuality {
        /// Quality score (0.0 - 1.0).
        score: f64,
    },
    /// Skill matches via trigram similarity.
    SimilarityMatch {
        /// Query that was matched.
        query: String,
        /// Similarity score (0.0 - 1.0).
        similarity: f64,
    },
}

impl RecommendationSignal {
    /// Get a short label for this signal.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Dependency => "dependency",
            Self::Dependent => "dependent",
            Self::Sibling => "sibling",
            Self::CoUsed { .. } => "co-used",
            Self::ProjectMatch { .. } => "project-match",
            Self::RecentlyUsed { .. } => "recently-used",
            Self::PromptMatch { .. } => "prompt-match",
            Self::HighQuality { .. } => "high-quality",
            Self::SimilarityMatch { .. } => "similarity-match",
        }
    }
}

/// Suggestions for new skills to create.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillGapAnalysis {
    /// Identified gaps based on project context.
    pub gaps: Vec<SkillGap>,
    /// Suggested skill names/topics.
    pub suggestions: Vec<String>,
}

/// An identified gap in the skill library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGap {
    /// Area where skills are missing.
    pub area: String,
    /// Evidence supporting this gap.
    pub evidence: Vec<String>,
    /// Priority (high/medium/low).
    pub priority: String,
}
