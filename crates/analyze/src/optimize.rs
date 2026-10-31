//! Optimization suggestions for skills.
//!
//! Analyzes skills and provides actionable suggestions to:
//! - Reduce token usage
//! - Improve structure
//! - Fix common issues

use serde::{Deserialize, Serialize};

use crate::deps::{DependencyAnalysis, DependencyType};
use crate::fence::FenceTracker;
use crate::tokens::{frontmatter_lines, TokenBreakdown, TokenCategory};

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
    /// Which condition the suggestion reports, when `quality_score`
    /// deducts for that condition directly. Matching on this, not on the
    /// message text, keeps a reworded message from changing the score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<SuggestionKind>,
}

/// Conditions that `quality_score` deducts for directly, so suggestions
/// reporting them are not counted a second time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestionKind {
    /// The skill is in the very-large token category.
    VeryLarge,
    /// The skill is in the large token category.
    Large,
    /// Referenced local files are missing.
    MissingFiles,
}

impl Suggestion {
    pub fn high(opt_type: OptimizationType, message: impl Into<String>) -> Self {
        Self {
            priority: Priority::High,
            opt_type,
            message: message.into(),
            token_savings: None,
            action: None,
            kind: None,
        }
    }

    pub fn medium(opt_type: OptimizationType, message: impl Into<String>) -> Self {
        Self {
            priority: Priority::Medium,
            opt_type,
            message: message.into(),
            token_savings: None,
            action: None,
            kind: None,
        }
    }

    pub fn low(opt_type: OptimizationType, message: impl Into<String>) -> Self {
        Self {
            priority: Priority::Low,
            opt_type,
            message: message.into(),
            token_savings: None,
            action: None,
            kind: None,
        }
    }

    pub fn with_kind(mut self, kind: SuggestionKind) -> Self {
        self.kind = Some(kind);
        self
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
                .with_kind(SuggestionKind::VeryLarge)
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
                .with_kind(SuggestionKind::Large)
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
            .with_kind(SuggestionKind::MissingFiles)
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
    let mut fence = FenceTracker::default();
    let mut block_lines = 0;
    let mut large_blocks = 0;

    for line in content.lines() {
        let was_open = fence.in_block();
        if !fence.is_code(line) {
            continue;
        }
        if !was_open {
            // Opening fence
            block_lines = 0;
        } else if !fence.in_block() {
            // Closing fence
            if block_lines > 50 {
                large_blocks += 1;
            }
        } else {
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
        let prefix = format!("{dir}/");
        let has_refs = deps.dependencies.iter().any(|d| {
            d.dep_type != DependencyType::ExternalUrl
                && d.target.trim_start_matches("./").starts_with(&prefix)
        });
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

/// Lines outside the frontmatter and fenced code blocks, with their
/// 1-based line numbers. Fence lines themselves are dropped.
fn prose_lines(content: &str) -> Vec<(usize, &str)> {
    let frontmatter = frontmatter_lines(content);
    let mut fence = FenceTracker::default();
    let mut out = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if frontmatter.as_ref().is_some_and(|r| r.contains(&i)) {
            continue;
        }
        if !fence.is_code(line) {
            out.push((i + 1, line));
        }
    }
    out
}

fn check_content_patterns(content: &str, suggestions: &mut Vec<Suggestion>) {
    // Code blocks hold comments like `# Example` and minified lines;
    // neither is a markdown heading or paragraph.
    let prose = prose_lines(content);

    // Check for duplicate headings: same level and text, so `# Usage`
    // and `## Usage` in different sections are not duplicates.
    let mut headings: Vec<(usize, &str)> = Vec::new();
    for &(_, line) in &prose {
        if line.starts_with('#') {
            let text = line.trim_start_matches('#');
            let level = line.len() - text.len();
            let heading = (level, text.trim());
            if headings.contains(&heading) {
                suggestions.push(
                    Suggestion::low(
                        OptimizationType::ImproveStructure,
                        format!("Duplicate heading found: '{}'", heading.1),
                    )
                    .with_action("Rename or consolidate duplicate sections"),
                );
                break; // Only report once
            }
            headings.push(heading);
        }
    }

    // Check for very long paragraphs (no line breaks)
    for &(line_number, line) in &prose {
        // Skip frontmatter delimiters and headings
        if line.starts_with('#') || line.trim() == "---" {
            continue;
        }

        let chars = line.chars().count();
        if chars > 500 {
            suggestions.push(
                Suggestion::low(
                    OptimizationType::ImproveStructure,
                    format!(
                        "Very long paragraph at line {} ({} chars)",
                        line_number, chars
                    ),
                )
                .with_action("Break into shorter paragraphs for readability"),
            );
            break; // Only report once
        }
    }

    // Check for TODO/FIXME markers: whole words in prose, so `TODOS`
    // and a TODO inside a code example do not count.
    let todo_count: usize = prose
        .iter()
        .map(|&(_, line)| {
            line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|w| *w == "TODO" || *w == "FIXME")
                .count()
        })
        .sum();
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

/// Whether `s` reports a condition `quality_score` already deducts for
/// directly (skill size, missing referenced files).
fn mirrors_direct_deduction(s: &Suggestion) -> bool {
    s.kind.is_some()
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

    // Deduct for the remaining suggestions. The size and missing-file
    // suggestions only restate the two deductions above, so counting
    // them again would penalise the same problem twice.
    let independent = || suggestions.iter().filter(|s| !mirrors_direct_deduction(s));

    // Deduct for high-priority suggestions
    let high_count = independent()
        .filter(|s| s.priority == Priority::High)
        .count();
    score -= 0.1 * (high_count as f64).min(3.0);

    // Deduct for medium-priority suggestions
    let medium_count = independent()
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
    fn code_block_contents_are_not_headings_or_paragraphs() {
        // IN-71: `# Example` comments in two bash blocks were reported
        // as a duplicate heading, and a long line in a code block as a
        // long paragraph.
        let long_line = "x".repeat(600);
        let content = format!(
            "# Title\n\n```bash\n# Example\necho a\n```\n\n```bash\n# Example\n{long_line}\n```\n"
        );
        let suggestions = suggest_optimizations(
            &content,
            &TokenBreakdown::default(),
            &DependencyAnalysis::default(),
        );
        assert!(
            !suggestions
                .iter()
                .any(|s| s.message.contains("Duplicate heading")
                    || s.message.contains("long paragraph")),
            "{:?}",
            suggestions.iter().map(|s| &s.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn size_and_missing_files_are_each_penalised_once() {
        // IN-72: the size and missing-file deductions were applied
        // again through the High suggestions that report them.
        let tokens = TokenBreakdown {
            total: 12_000,
            prose: 12_000,
            ..Default::default()
        };
        let mut deps = DependencyAnalysis::default();
        for i in 0..5 {
            deps.missing.push(crate::deps::Dependency {
                dep_type: crate::deps::DependencyType::Reference,
                target: format!("references/missing{i}.md"),
                line: Some(1),
                exists: Some(false),
            });
        }
        let suggestions = suggest_optimizations("# Skill\n", &tokens, &deps);
        let score = quality_score(&tokens, &deps, &suggestions);
        // 1.0 - 0.3 (very large) - 0.2 (five missing files).
        assert!((score - 0.5).abs() < 1e-9, "score {score}");

        let large = TokenBreakdown {
            total: 3_000,
            prose: 3_000,
            ..Default::default()
        };
        let suggestions =
            suggest_optimizations("# Skill\n", &large, &DependencyAnalysis::default());
        let score = quality_score(&large, &DependencyAnalysis::default(), &suggestions);
        assert!((score - 0.85).abs() < 1e-9, "score {score}");
    }

    fn messages(content: &str, deps: &DependencyAnalysis) -> Vec<String> {
        suggest_optimizations(content, &TokenBreakdown::default(), deps)
            .into_iter()
            .map(|s| s.message)
            .collect()
    }

    /// M299: a URL that merely contains `scripts/` is not a reference to
    /// the skill's scripts directory.
    #[test]
    fn external_url_is_not_a_reference_to_a_local_directory() {
        let mut deps = DependencyAnalysis::default();
        deps.directories.push("scripts".into());
        deps.total_dep_size = 10;
        deps.dependencies.push(crate::deps::Dependency {
            dep_type: crate::deps::DependencyType::ExternalUrl,
            target: "https://example.com/scripts/x".into(),
            line: Some(1),
            exists: None,
        });
        let msgs = messages("# Skill\n", &deps);
        assert!(
            msgs.iter().any(|m| m.contains("Directory 'scripts/'")),
            "{msgs:?}"
        );
    }

    /// M300: frontmatter is not prose; heading duplicates compare level;
    /// paragraph length counts characters.
    #[test]
    fn frontmatter_heading_levels_and_char_lengths() {
        let desc = "d".repeat(600);
        let content = format!(
            "---\n# comment\ndescription: {desc}\n# comment\n---\n# Usage\n## Usage\n{}\n",
            "é".repeat(300)
        );
        let msgs = messages(&content, &DependencyAnalysis::default());
        assert!(
            !msgs
                .iter()
                .any(|m| m.contains("Duplicate heading") || m.contains("long paragraph")),
            "{msgs:?}"
        );
    }

    /// M301: TODO markers count as words in prose only.
    #[test]
    fn todo_markers_count_whole_words_outside_code() {
        let content =
            "# Skill\nTODOS are listed elsewhere.\n```\n// TODO: example\n```\nTODO: real one\n";
        let msgs = messages(content, &DependencyAnalysis::default());
        assert!(
            msgs.iter().any(|m| m == "Found 1 TODO/FIXME marker(s)"),
            "{msgs:?}"
        );
    }

    /// M302: deductions follow the suggestion kind, not its wording.
    #[test]
    fn quality_score_ignores_message_wording() {
        let tokens = TokenBreakdown {
            total: 12_000,
            prose: 12_000,
            ..Default::default()
        };
        let deps = DependencyAnalysis::default();
        let mut suggestions = suggest_optimizations("# Skill\n", &tokens, &deps);
        for s in &mut suggestions {
            s.message = format!("reworded: {}", s.message);
        }
        let score = quality_score(&tokens, &deps, &suggestions);
        assert!((score - 0.7).abs() < 1e-9, "score {score}");
    }

    /// M303: a `~~~` fence hides its contents like a backtick fence.
    #[test]
    fn tilde_fenced_code_is_not_prose() {
        let content = "# Title\n~~~\n# Title\n~~~\n";
        let msgs = messages(content, &DependencyAnalysis::default());
        assert!(
            !msgs.iter().any(|m| m.contains("Duplicate heading")),
            "{msgs:?}"
        );
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
