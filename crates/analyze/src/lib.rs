//! Skill analysis: token counting, dependencies, and optimization.
//!
//! This crate provides comprehensive analysis of SKILL.md files:
//! - Token estimation with section breakdown
//! - Dependency analysis (local files, external URLs)
//! - Optimization suggestions
//! - Quality scoring
//!
//! # Example
//!
//! ```rust
//! use skrills_analyze::{analyze_skill, SkillAnalysis};
//! use std::path::Path;
//!
//! let content = r#"---
//! name: my-skill
//! description: A helpful skill
//! ---
//! # My Skill
//! See [reference](references/guide.md) for details.
//! "#;
//!
//! let analysis = analyze_skill(Path::new("skill.md"), content);
//! println!("Tokens: {}", analysis.tokens.total);
//! println!("Quality: {:.0}%", analysis.quality_score * 100.0);
//! ```

#![deny(unsafe_code)]

pub mod deps;
pub mod graph;
pub mod optimize;
pub mod resolve;
pub mod tokens;

pub use deps::{analyze_dependencies, Dependency, DependencyAnalysis, DependencyType};
pub use graph::DependencyGraph;
pub use optimize::{quality_score, suggest_optimizations, OptimizationType, Priority, Suggestion};
pub use resolve::{
    DependencyGraph as ResolveDependencyGraph, DependencyResolver, GraphBuilder, InMemoryRegistry,
    ResolutionResult, ResolveError, ResolveOptions, ResolvedDependency, SkillInfo, SkillRegistry,
};
// Re-export SkillSource for users of the resolve API
pub use skrills_discovery::SkillSource;
pub use tokens::{count_tokens, estimate_tokens, TokenBreakdown, TokenCategory};

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Complete analysis of a skill file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAnalysis {
    /// Skill name (from filename or frontmatter).
    pub name: String,
    /// Token breakdown.
    pub tokens: TokenBreakdown,
    /// Token category.
    pub category: TokenCategory,
    /// Dependency analysis.
    pub dependencies: DependencyAnalysis,
    /// Optimization suggestions.
    pub suggestions: Vec<Suggestion>,
    /// Quality score (0.0 - 1.0).
    pub quality_score: f64,
}

/// Analyze a single skill file.
pub fn analyze_skill(path: &Path, content: &str) -> SkillAnalysis {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| {
            if s.eq_ignore_ascii_case("SKILL") {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("skill")
                    .to_string()
            } else {
                s.to_string()
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    let tokens = count_tokens(content);
    let category = TokenCategory::from_count(tokens.total);
    let dependencies = analyze_dependencies(path, content);
    let suggestions = suggest_optimizations(content, &tokens, &dependencies);
    let quality = quality_score(&tokens, &dependencies, &suggestions);

    SkillAnalysis {
        name,
        tokens,
        category,
        dependencies,
        suggestions,
        quality_score: quality,
    }
}

/// Summary of multiple skill analyses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisSummary {
    /// Total number of skills analyzed.
    pub total_skills: usize,
    /// Total tokens across all skills.
    pub total_tokens: usize,
    /// Skills by category.
    pub by_category: CategoryCounts,
    /// Total optimization suggestions.
    pub total_suggestions: usize,
    /// High-priority suggestions count.
    pub high_priority_count: usize,
    /// Average quality score.
    pub avg_quality: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategoryCounts {
    pub small: usize,
    pub medium: usize,
    pub large: usize,
    pub very_large: usize,
}

impl AnalysisSummary {
    pub fn from_analyses(analyses: &[SkillAnalysis]) -> Self {
        let mut summary = AnalysisSummary {
            total_skills: analyses.len(),
            ..Default::default()
        };

        let mut total_quality = 0.0;

        for analysis in analyses {
            summary.total_tokens += analysis.tokens.total;
            summary.total_suggestions += analysis.suggestions.len();
            summary.high_priority_count += analysis
                .suggestions
                .iter()
                .filter(|s| s.priority == Priority::High)
                .count();
            total_quality += analysis.quality_score;

            match analysis.category {
                TokenCategory::Small => summary.by_category.small += 1,
                TokenCategory::Medium => summary.by_category.medium += 1,
                TokenCategory::Large => summary.by_category.large += 1,
                TokenCategory::VeryLarge => summary.by_category.very_large += 1,
            }
        }

        if !analyses.is_empty() {
            summary.avg_quality = total_quality / analyses.len() as f64;
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_skill() {
        let content = "---\nname: test\ndescription: A test\n---\n# Test\nSome content here.";
        let analysis = analyze_skill(Path::new("test.md"), content);

        assert_eq!(analysis.name, "test");
        assert!(analysis.tokens.total > 0);
        assert!(analysis.quality_score > 0.0);
    }

    #[test]
    fn test_skill_naming_from_directory() {
        let content = "# Content";
        let analysis = analyze_skill(Path::new("/skills/my-skill/SKILL.md"), content);

        assert_eq!(analysis.name, "my-skill");
    }

    #[test]
    fn test_analysis_summary() {
        let analyses = vec![
            SkillAnalysis {
                name: "a".to_string(),
                tokens: TokenBreakdown {
                    total: 100,
                    ..Default::default()
                },
                category: TokenCategory::Small,
                dependencies: DependencyAnalysis::default(),
                suggestions: vec![],
                quality_score: 0.9,
            },
            SkillAnalysis {
                name: "b".to_string(),
                tokens: TokenBreakdown {
                    total: 5000,
                    ..Default::default()
                },
                category: TokenCategory::Large,
                dependencies: DependencyAnalysis::default(),
                suggestions: vec![Suggestion::high(OptimizationType::ReduceSize, "Test")],
                quality_score: 0.6,
            },
        ];

        let summary = AnalysisSummary::from_analyses(&analyses);

        assert_eq!(summary.total_skills, 2);
        assert_eq!(summary.total_tokens, 5100);
        assert_eq!(summary.by_category.small, 1);
        assert_eq!(summary.by_category.large, 1);
        assert_eq!(summary.high_priority_count, 1);
        assert!((summary.avg_quality - 0.75).abs() < 0.01);
    }
}
