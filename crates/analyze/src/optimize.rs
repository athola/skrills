//! Optimization suggestions for skills.
//!
//! Analyzes skills and provides actionable suggestions to:
//! - Reduce token usage
//! - Improve structure
//! - Fix common issues

use serde::{Deserialize, Serialize};

use crate::deps::DependencyAnalysis;
use crate::tokens::{TokenBreakdown, TokenCategory};

/// Priority level for optimization suggestions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    /// High priority - significant impact.
    High,
    /// Medium priority - moderate impact.
    Medium,
    /// Low priority - minor improvement.
    Low,
}

/// Category of optimization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptimizationType {
    /// Reduce overall size.
    ReduceSize,
    /// Improve structure.
    ImproveStructure,
    /// Fix potential issues.
    FixIssue,
    /// Improve compatibility.
    Compatibility,
}

/// A single optimization suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    /// Priority of this suggestion.
    pub priority: Priority,
    /// Type of optimization.
    pub opt_type: OptimizationType,
    /// Human-readable description.
    pub message: String,
    /// Estimated token savings (if applicable).
    pub token_savings: Option<usize>,
    /// Specific action to take.
    pub action: Option<String>,
}

impl Suggestion {
    pub fn high(opt_type: OptimizationType, message: impl Into<String>) -> Self {
        Self {
            priority: Priority::High,
            opt_type,
            message: message.into(),
            token_savings: None,
            action: None,
        }
    }

    pub fn medium(opt_type: OptimizationType, message: impl Into<String>) -> Self {
        Self {
            priority: Priority::Medium,
            opt_type,
            message: message.into(),
            token_savings: None,
            action: None,
        }
    }

    pub fn low(opt_type: OptimizationType, message: impl Into<String>) -> Self {
        Self {
            priority: Priority::Low,
            opt_type,
            message: message.into(),
            token_savings: None,
            action: None,
        }
    }

    pub fn with_savings(mut self, tokens: usize) -> Self {
        self.token_savings = Some(tokens);
        self
    }

    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }
}

/// Generate optimization suggestions for a skill.
pub fn suggest_optimizations(
    content: &str,
    tokens: &TokenBreakdown,
    deps: &DependencyAnalysis,
) -> Vec<Suggestion> {
    let mut suggestions = Vec::new();

    // Check overall size
    let category = TokenCategory::from_count(tokens.total);
    match category {
        TokenCategory::VeryLarge => {
            suggestions.push(
                Suggestion::high(
                    OptimizationType::ReduceSize,
                    format!(
                        "Skill is very large ({} tokens). Consider splitting into smaller skills.",
                        tokens.total
                    ),
                )
                .with_action("Split into focused, single-responsibility skills"),
            );
        }
        TokenCategory::Large => {
            suggestions.push(
                Suggestion::medium(
                    OptimizationType::ReduceSize,
                    format!(
                        "Skill is large ({} tokens). Review for unnecessary content.",
                        tokens.total
                    ),
                )
                .with_action("Remove redundant examples or verbose explanations"),
            );
        }
        _ => {}
    }

    // Check code vs prose ratio
    if tokens.code > 0 && tokens.prose > 0 {
        let code_ratio = tokens.code as f64 / tokens.total as f64;
        if code_ratio > 0.7 {
            suggestions.push(
                Suggestion::medium(
                    OptimizationType::ImproveStructure,
                    format!(
                        "Code blocks comprise {:.0}% of skill. Consider extracting to separate files.",
                        code_ratio * 100.0
                    ),
                )
                .with_savings(tokens.code / 2)
                .with_action("Move large code blocks to scripts/ or references/ directory"),
            );
        }
    }

    // Check for large code blocks
    check_large_code_blocks(content, &mut suggestions);

    // Check for missing dependencies
    if !deps.missing.is_empty() {
        suggestions.push(
            Suggestion::high(
                OptimizationType::FixIssue,
                format!("{} referenced files are missing", deps.missing.len()),
            )
            .with_action("Create missing files or update references"),
        );
    }

    // Check for unused directories
    check_directory_usage(deps, &mut suggestions);

    // Check content patterns
    check_content_patterns(content, &mut suggestions);

    // Sort by priority
    suggestions.sort_by(|a, b| {
        let priority_order = |p: &Priority| match p {
            Priority::High => 0,
            Priority::Medium => 1,
            Priority::Low => 2,
        };
        priority_order(&a.priority).cmp(&priority_order(&b.priority))
    });

    suggestions
}

fn check_large_code_blocks(content: &str, suggestions: &mut Vec<Suggestion>) {
    let mut in_code_block = false;
    let mut block_lines = 0;
    let mut large_blocks = 0;

    for line in content.lines() {
        if line.trim().starts_with("```") {
            if in_code_block {
                // End of block
                if block_lines > 50 {
                    large_blocks += 1;
                }
                block_lines = 0;
            }
            in_code_block = !in_code_block;
        } else if in_code_block {
            block_lines += 1;
        }
    }

    if large_blocks > 0 {
        suggestions.push(
            Suggestion::medium(
                OptimizationType::ReduceSize,
                format!(
                    "Found {} large code block(s) (>50 lines). Consider using file references.",
                    large_blocks
                ),
            )
            .with_action("Extract large code blocks to separate files in scripts/ or references/"),
        );
    }
}

fn check_directory_usage(deps: &DependencyAnalysis, suggestions: &mut Vec<Suggestion>) {
    // Check if directories exist but aren't referenced
    for dir in &deps.directories {
        let has_refs = deps.dependencies.iter().any(|d| d.target.contains(dir));
        if !has_refs && deps.total_dep_size > 0 {
            suggestions.push(
                Suggestion::low(
                    OptimizationType::ImproveStructure,
                    format!(
                        "Directory '{}/' exists but no references found in skill content",
                        dir
                    ),
                )
                .with_action("Add references to files or remove unused directory"),
            );
        }
    }
}

fn check_content_patterns(content: &str, suggestions: &mut Vec<Suggestion>) {
    // Check for duplicate headings
    let mut headings: Vec<&str> = Vec::new();
    for line in content.lines() {
        if line.starts_with('#') {
            let heading = line.trim_start_matches('#').trim();
            if headings.contains(&heading) {
                suggestions.push(
                    Suggestion::low(
                        OptimizationType::ImproveStructure,
                        format!("Duplicate heading found: '{}'", heading),
                    )
                    .with_action("Rename or consolidate duplicate sections"),
                );
                break; // Only report once
            }
            headings.push(heading);
        }
    }

    // Check for very long paragraphs (no line breaks)
    let lines: Vec<&str> = content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        // Skip code blocks, frontmatter, headings
        if line.starts_with("```") || line.starts_with('#') || line.trim() == "---" {
            continue;
        }

        if line.len() > 500 {
            suggestions.push(
                Suggestion::low(
                    OptimizationType::ImproveStructure,
                    format!(
                        "Very long paragraph at line {} ({} chars)",
                        i + 1,
                        line.len()
                    ),
                )
                .with_action("Break into shorter paragraphs for readability"),
            );
            break; // Only report once
        }
    }

    // Check for TODO/FIXME markers
    let todo_count = content.matches("TODO").count() + content.matches("FIXME").count();
    if todo_count > 0 {
        suggestions.push(
            Suggestion::low(
                OptimizationType::FixIssue,
                format!("Found {} TODO/FIXME marker(s)", todo_count),
            )
            .with_action("Address or remove TODO/FIXME comments"),
        );
    }
}

/// Calculate a quality score (0.0 - 1.0) for a skill.
pub fn quality_score(
    tokens: &TokenBreakdown,
    deps: &DependencyAnalysis,
    suggestions: &[Suggestion],
) -> f64 {
    let mut score = 1.0;

    // Deduct for size issues
    match TokenCategory::from_count(tokens.total) {
        TokenCategory::VeryLarge => score -= 0.3,
        TokenCategory::Large => score -= 0.15,
        _ => {}
    }

    // Deduct for missing dependencies
    if !deps.missing.is_empty() {
        score -= 0.2 * (deps.missing.len() as f64 / 5.0).min(1.0);
    }

    // Deduct for high-priority suggestions
    let high_count = suggestions
        .iter()
        .filter(|s| s.priority == Priority::High)
        .count();
    score -= 0.1 * (high_count as f64).min(3.0);

    // Deduct for medium-priority suggestions
    let medium_count = suggestions
        .iter()
        .filter(|s| s.priority == Priority::Medium)
        .count();
    score -= 0.05 * (medium_count as f64).min(4.0);

    score.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suggest_for_large_skill() {
        let content = "a".repeat(50000); // Very large
        let tokens = TokenBreakdown {
            total: 12500,
            prose: 12500,
            ..Default::default()
        };
        let deps = DependencyAnalysis::default();

        let suggestions = suggest_optimizations(&content, &tokens, &deps);

        assert!(!suggestions.is_empty());
        assert!(suggestions
            .iter()
            .any(|s| s.priority == Priority::High && s.message.contains("very large")));
    }

    #[test]
    fn test_quality_score() {
        let tokens = TokenBreakdown {
            total: 500,
            ..Default::default()
        };
        let deps = DependencyAnalysis::default();
        let suggestions = Vec::new();

        let score = quality_score(&tokens, &deps, &suggestions);
        assert!(score > 0.9); // Good score for small skill with no issues
    }

    #[test]
    fn test_quality_score_with_issues() {
        let tokens = TokenBreakdown {
            total: 10000, // Very large
            ..Default::default()
        };
        let mut deps = DependencyAnalysis::default();
        deps.missing.push(crate::deps::Dependency {
            dep_type: crate::deps::DependencyType::Reference,
            target: "missing.md".to_string(),
            line: Some(1),
            exists: Some(false),
        });
        let suggestions = vec![Suggestion::high(OptimizationType::ReduceSize, "Test issue")];

        let score = quality_score(&tokens, &deps, &suggestions);
        assert!(score < 0.6); // Lower score due to issues
    }
}
