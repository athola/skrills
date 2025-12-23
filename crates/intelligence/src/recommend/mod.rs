//! Smart skill recommendations combining multiple signals.

mod explainer;
mod scorer;

pub use explainer::generate_explanation;
pub use scorer::{RecommendationScorer, Scorer};

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
    /// Quality score if available.
    pub quality_score: Option<f64>,
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
