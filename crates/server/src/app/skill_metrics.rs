//! Aggregate skill metrics.
//!
//! The one computation behind the `skill-metrics` MCP tool
//! (`SkillService::compute_metrics`) and `skrills metrics`. Both used to
//! carry their own copy of it (SA-44).

use crate::metrics_types::{
    DependencyStats, HubSkill, MetricsValidationSummary, QualityDistribution, SkillMetrics,
    SkillTokenInfo, TokenStats,
};
use skrills_analyze::{analyze_skill, RelationshipGraph};
use skrills_discovery::SkillMeta;
use skrills_validate::{validate_skill, ValidationTarget};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fs;

/// Aggregates the metrics for `skills`, reading dependency edges from
/// `graph` (see [`crate::app::build_dependency_graph`]).
///
/// A skill that cannot be read is logged and left out of every count,
/// `total_skills` and `orphan_count` included.
pub fn compute_skill_metrics(
    skills: &[SkillMeta],
    graph: &RelationshipGraph,
    include_validation: bool,
) -> SkillMetrics {
    let mut by_source: HashMap<String, usize> = HashMap::new();
    let mut quality_high = 0usize;
    let mut quality_medium = 0usize;
    let mut quality_low = 0usize;
    let mut total_tokens = 0usize;
    let mut largest_skill: Option<SkillTokenInfo> = None;

    // Validation counters (only computed if requested)
    let mut passing = 0usize;
    let mut with_errors = 0usize;
    let mut with_warnings = 0usize;

    let mut counted: HashSet<String> = HashSet::new();
    for meta in skills {
        // Read skill content (before counting to ensure consistent totals)
        let content = match fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %meta.path.display(), error = %e, "Failed to read skill file");
                continue;
            }
        };

        let skill_uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
        counted.insert(skill_uri.clone());
        *by_source
            .entry(meta.source.label().to_string())
            .or_default() += 1;

        // Analyze for quality and tokens
        let analysis = analyze_skill(&meta.path, &content);

        // Quality buckets
        if analysis.quality_score >= 0.8 {
            quality_high += 1;
        } else if analysis.quality_score >= 0.5 {
            quality_medium += 1;
        } else {
            quality_low += 1;
        }

        // Token stats
        total_tokens += analysis.tokens.total;
        if largest_skill
            .as_ref()
            .is_none_or(|s| analysis.tokens.total > s.tokens)
        {
            largest_skill = Some(SkillTokenInfo {
                uri: skill_uri,
                tokens: analysis.tokens.total,
            });
        }

        // Optional validation
        if include_validation {
            let result = validate_skill(&meta.path, &content, ValidationTarget::Both);
            if result.claude_valid && result.codex_valid {
                passing += 1;
            } else if result.has_errors() {
                with_errors += 1;
            } else {
                with_warnings += 1;
            }
        }
    }
    let skill_count = by_source.values().sum::<usize>();

    // Dependency stats over the skills counted above, in graph order.
    let mut total_dependencies = 0usize;
    let mut orphan_count = 0usize;
    let mut hub_counts: Vec<(String, usize)> = Vec::new();
    for skill_uri in graph.skills().iter().filter(|u| counted.contains(*u)) {
        let deps = graph.dependencies(skill_uri);
        let dependents = graph.dependents(skill_uri);

        total_dependencies += deps.len();

        if deps.is_empty() && dependents.is_empty() {
            orphan_count += 1;
        }

        if !dependents.is_empty() {
            hub_counts.push((skill_uri.to_string(), dependents.len()));
        }
    }

    // Sort hubs by dependent count (descending) and take top 5
    hub_counts.sort_by_key(|b| Reverse(b.1));
    let hub_skills: Vec<HubSkill> = hub_counts
        .into_iter()
        .take(5)
        .map(|(uri, count)| HubSkill {
            uri,
            dependent_count: count,
        })
        .collect();

    let avg_deps = if skill_count > 0 {
        total_dependencies as f64 / skill_count as f64
    } else {
        0.0
    };

    let avg_tokens = total_tokens.checked_div(skill_count).unwrap_or(0);

    let validation_summary = include_validation.then_some(MetricsValidationSummary {
        passing,
        with_errors,
        with_warnings,
    });

    SkillMetrics {
        total_skills: skill_count,
        by_source,
        by_quality: QualityDistribution {
            high: quality_high,
            medium: quality_medium,
            low: quality_low,
        },
        dependency_stats: DependencyStats {
            total_dependencies,
            avg_per_skill: avg_deps,
            orphan_count,
            hub_skills,
        },
        token_stats: TokenStats {
            total_tokens,
            avg_per_skill: avg_tokens,
            largest_skill,
        },
        validation_summary,
    }
}
