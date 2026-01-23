//! CLI handler for the `metrics` command.

use crate::app::{
    DependencyStats, HubSkill, MetricsValidationSummary, QualityDistribution, SkillMetrics,
    SkillTokenInfo, TokenStats,
};
use crate::cli::OutputFormat;
use crate::discovery::merge_extra_dirs;
use anyhow::Result;
use skrills_analyze::RelationshipGraph;
use skrills_discovery::{discover_skills, extra_skill_roots};
use std::collections::HashMap;

/// Handle the `metrics` command.
pub(crate) fn handle_metrics_command(
    skill_dirs: Vec<std::path::PathBuf>,
    format: OutputFormat,
    include_validation: bool,
) -> Result<()> {
    use skrills_analyze::analyze_skill;
    use skrills_validate::{validate_skill, ValidationTarget};

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    if skills.is_empty() {
        println!("No skills found.");
        return Ok(());
    }

    // Collect metrics
    let mut by_source: HashMap<String, usize> = HashMap::new();
    let mut quality_high = 0usize;
    let mut quality_medium = 0usize;
    let mut quality_low = 0usize;
    let mut total_tokens = 0usize;
    let mut largest_skill: Option<SkillTokenInfo> = None;

    // Validation counters
    let mut passing = 0usize;
    let mut with_errors = 0usize;
    let mut with_warnings = 0usize;

    // Build dependency graph
    let mut dep_graph = RelationshipGraph::new();

    for meta in &skills {
        // Read skill content (before counting to ensure consistent totals)
        let content = match std::fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %meta.path.display(), error = %e, "Failed to read skill file");
                continue;
            }
        };

        // Count by source (after successful read for consistent totals)
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
        let skill_uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);

        // Track largest skill
        let should_replace = match largest_skill.as_ref() {
            None => true,
            Some(s) => analysis.tokens.total > s.tokens,
        };
        if should_replace {
            largest_skill = Some(SkillTokenInfo {
                uri: skill_uri.clone(),
                tokens: analysis.tokens.total,
            });
        }

        // Build dependency graph
        dep_graph.add_skill(&skill_uri);
        for dep in &analysis.dependencies.dependencies {
            if let skrills_analyze::DependencyType::Skill = dep.dep_type {
                dep_graph.add_dependency(&skill_uri, &dep.target);
            }
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

    // Compute dependency stats
    let all_skills: Vec<String> = dep_graph.skills();
    let mut total_dependencies = 0usize;
    let mut orphan_count = 0usize;
    let mut hub_counts: Vec<(String, usize)> = Vec::new();

    for skill_uri in &all_skills {
        let deps = dep_graph.dependencies(skill_uri);
        let dependents = dep_graph.dependents(skill_uri);

        total_dependencies += deps.len();

        if deps.is_empty() && dependents.is_empty() {
            orphan_count += 1;
        }

        if !dependents.is_empty() {
            hub_counts.push((skill_uri.to_string(), dependents.len()));
        }
    }

    // Sort hubs by dependent count (descending) and take top 5
    hub_counts.sort_by(|a, b| b.1.cmp(&a.1));
    let hub_skills: Vec<HubSkill> = hub_counts
        .into_iter()
        .take(5)
        .map(|(uri, count)| HubSkill {
            uri,
            dependent_count: count,
        })
        .collect();

    let skill_count = skills.len();
    let avg_deps = if skill_count > 0 {
        total_dependencies as f64 / skill_count as f64
    } else {
        0.0
    };

    let avg_tokens = if skill_count > 0 {
        total_tokens / skill_count
    } else {
        0
    };

    let validation_summary = if include_validation {
        Some(MetricsValidationSummary {
            passing,
            with_errors,
            with_warnings,
        })
    } else {
        None
    };

    let metrics = SkillMetrics {
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
    };

    // Output
    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&metrics)?);
    } else {
        print_metrics_human(&metrics);
    }

    Ok(())
}

/// Print metrics in human-readable format.
fn print_metrics_human(metrics: &SkillMetrics) {
    println!("Skill Metrics");
    println!("═════════════\n");

    println!("Total: {} skills\n", metrics.total_skills);

    // By source
    println!("By Source:");
    let total = metrics.total_skills as f64;
    let mut sources: Vec<_> = metrics.by_source.iter().collect();
    sources.sort_by(|a, b| b.1.cmp(a.1));
    for (source, count) in sources {
        let pct = if total > 0.0 {
            (*count as f64 / total) * 100.0
        } else {
            0.0
        };
        println!("  {:14} {:3} ({:.0}%)", format!("{}:", source), count, pct);
    }

    // Quality
    println!("\nQuality:");
    let q = &metrics.by_quality;
    let q_total = (q.high + q.medium + q.low) as f64;
    if q_total > 0.0 {
        println!(
            "  High (≥0.8)    {:3} ({:.0}%)",
            q.high,
            (q.high as f64 / q_total) * 100.0
        );
        println!(
            "  Medium         {:3} ({:.0}%)",
            q.medium,
            (q.medium as f64 / q_total) * 100.0
        );
        println!(
            "  Low (<0.5)     {:3} ({:.0}%)",
            q.low,
            (q.low as f64 / q_total) * 100.0
        );
    }

    // Dependencies
    println!("\nDependencies:");
    let d = &metrics.dependency_stats;
    println!("  Total edges    {}", d.total_dependencies);
    println!("  Avg/skill      {:.1}", d.avg_per_skill);
    println!("  Orphans        {}", d.orphan_count);
    if !d.hub_skills.is_empty() {
        let hub_names: Vec<&str> = d
            .hub_skills
            .iter()
            .take(3)
            .map(|h| h.uri.rsplit('/').next().unwrap_or(&h.uri))
            .collect();
        println!("  Top hubs       {}", hub_names.join(", "));
    }

    // Tokens
    println!("\nTokens:");
    let t = &metrics.token_stats;
    println!("  Total          {}", t.total_tokens);
    println!("  Average        {}", t.avg_per_skill);
    if let Some(ref largest) = t.largest_skill {
        let name = largest.uri.rsplit('/').next().unwrap_or(&largest.uri);
        println!("  Largest        {} ({})", name, largest.tokens);
    }

    // Validation summary (if present)
    if let Some(ref v) = metrics.validation_summary {
        println!("\nValidation:");
        println!("  Passing        {}", v.passing);
        println!("  With errors    {}", v.with_errors);
        println!("  With warnings  {}", v.with_warnings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Helper to create a test skill file with given content.
    fn create_skill(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        let path = skill_dir.join("SKILL.md");
        fs::write(&path, content).expect("write skill");
        path
    }

    /// Create a minimal valid skill.
    fn minimal_skill_content(name: &str, desc: &str) -> String {
        format!(
            r#"---
name: {}
description: {}
---
# {}

A test skill.
"#,
            name, desc, name
        )
    }

    #[test]
    fn test_handle_metrics_command_empty_dir() {
        // GIVEN an empty directory with no skills
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();

        // WHEN we run handle_metrics_command
        let result = handle_metrics_command(vec![skill_dir], OutputFormat::Text, false);

        // THEN it should succeed (prints "No skills found.")
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_metrics_command_single_skill_json() {
        // GIVEN a directory with one skill
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();
        create_skill(
            &skill_dir,
            "test-skill",
            &minimal_skill_content("test-skill", "Test"),
        );

        // WHEN we run handle_metrics_command with json format
        let result = handle_metrics_command(vec![skill_dir], OutputFormat::Json, false);

        // THEN it should succeed
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_metrics_command_with_validation() {
        // GIVEN a directory with one skill
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();
        create_skill(
            &skill_dir,
            "valid-skill",
            &minimal_skill_content("valid-skill", "Valid skill"),
        );

        // WHEN we run handle_metrics_command with validation enabled
        let result = handle_metrics_command(vec![skill_dir], OutputFormat::Text, true);

        // THEN it should succeed
        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_metrics_command_multiple_skills() {
        // GIVEN a directory with multiple skills of varying quality
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();

        // High quality skill (complete frontmatter, good content)
        let high_quality = r#"---
name: high-quality
description: A comprehensive skill with good documentation
---
# High Quality Skill

## Overview
This skill provides comprehensive functionality.

## Usage
```bash
skill(high-quality)
```

## Examples
- Example 1
- Example 2
"#;
        create_skill(&skill_dir, "high-quality", high_quality);

        // Medium quality skill
        create_skill(
            &skill_dir,
            "medium-skill",
            &minimal_skill_content("medium-skill", "Medium"),
        );

        // Low quality skill (minimal content)
        let low_quality = "# Minimal\nShort.";
        create_skill(&skill_dir, "low-skill", low_quality);

        // WHEN we run handle_metrics_command
        let result = handle_metrics_command(vec![skill_dir], OutputFormat::Text, false);

        // THEN it should succeed and process all 3 skills
        assert!(result.is_ok());
    }

    #[test]
    fn test_quality_distribution_new() {
        // GIVEN quality distribution values
        let dist = QualityDistribution {
            high: 5,
            medium: 3,
            low: 2,
        };

        // THEN the values should be set correctly
        assert_eq!(dist.high, 5);
        assert_eq!(dist.medium, 3);
        assert_eq!(dist.low, 2);
    }

    #[test]
    fn test_dependency_stats_empty() {
        // GIVEN empty dependency stats
        let stats = DependencyStats {
            total_dependencies: 0,
            avg_per_skill: 0.0,
            orphan_count: 0,
            hub_skills: vec![],
        };

        // THEN it should represent no dependencies
        assert_eq!(stats.total_dependencies, 0);
        assert_eq!(stats.orphan_count, 0);
        assert!(stats.hub_skills.is_empty());
    }

    #[test]
    fn test_hub_skill_creation() {
        // GIVEN a hub skill
        let hub = HubSkill {
            uri: "skill://skrills/codex/core".into(),
            dependent_count: 10,
        };

        // THEN it should have the expected values
        assert_eq!(hub.uri, "skill://skrills/codex/core");
        assert_eq!(hub.dependent_count, 10);
    }

    #[test]
    fn test_token_stats_without_largest() {
        // GIVEN token stats without a largest skill
        let stats = TokenStats {
            total_tokens: 0,
            avg_per_skill: 0,
            largest_skill: None,
        };

        // THEN largest_skill should be None
        assert!(stats.largest_skill.is_none());
    }

    #[test]
    fn test_token_stats_with_largest() {
        // GIVEN token stats with a largest skill
        let stats = TokenStats {
            total_tokens: 5000,
            avg_per_skill: 1000,
            largest_skill: Some(SkillTokenInfo {
                uri: "skill://skrills/codex/big-skill".into(),
                tokens: 2000,
            }),
        };

        // THEN largest_skill should have correct values
        let largest = stats.largest_skill.as_ref().unwrap();
        assert_eq!(largest.tokens, 2000);
        assert!(largest.uri.contains("big-skill"));
    }

    #[test]
    fn test_validation_summary() {
        // GIVEN a validation summary
        let summary = MetricsValidationSummary {
            passing: 8,
            with_errors: 1,
            with_warnings: 2,
        };

        // THEN the counts should match
        assert_eq!(summary.passing, 8);
        assert_eq!(summary.with_errors, 1);
        assert_eq!(summary.with_warnings, 2);
    }

    #[test]
    fn test_skill_metrics_serialization() {
        // GIVEN a complete SkillMetrics struct
        let metrics = SkillMetrics {
            total_skills: 10,
            by_source: {
                let mut m = HashMap::new();
                m.insert("codex".into(), 6);
                m.insert("claude".into(), 4);
                m
            },
            by_quality: QualityDistribution {
                high: 5,
                medium: 3,
                low: 2,
            },
            dependency_stats: DependencyStats {
                total_dependencies: 15,
                avg_per_skill: 1.5,
                orphan_count: 2,
                hub_skills: vec![HubSkill {
                    uri: "skill://skrills/codex/core".into(),
                    dependent_count: 5,
                }],
            },
            token_stats: TokenStats {
                total_tokens: 10000,
                avg_per_skill: 1000,
                largest_skill: Some(SkillTokenInfo {
                    uri: "skill://skrills/codex/big".into(),
                    tokens: 2500,
                }),
            },
            validation_summary: None,
        };

        // WHEN serializing to JSON
        let json = serde_json::to_string(&metrics).unwrap();

        // THEN it should contain expected fields
        assert!(json.contains("\"total_skills\":10"));
        assert!(json.contains("\"high\":5"));
        assert!(json.contains("\"total_dependencies\":15"));
        assert!(json.contains("\"total_tokens\":10000"));
        // validation_summary should be skipped when None
        assert!(!json.contains("validation_summary"));
    }

    #[test]
    fn test_skill_metrics_with_validation_serialization() {
        // GIVEN a SkillMetrics struct with validation
        let metrics = SkillMetrics {
            total_skills: 5,
            by_source: HashMap::new(),
            by_quality: QualityDistribution {
                high: 3,
                medium: 1,
                low: 1,
            },
            dependency_stats: DependencyStats {
                total_dependencies: 0,
                avg_per_skill: 0.0,
                orphan_count: 5,
                hub_skills: vec![],
            },
            token_stats: TokenStats {
                total_tokens: 2500,
                avg_per_skill: 500,
                largest_skill: None,
            },
            validation_summary: Some(MetricsValidationSummary {
                passing: 4,
                with_errors: 0,
                with_warnings: 1,
            }),
        };

        // WHEN serializing to JSON
        let json = serde_json::to_string(&metrics).unwrap();

        // THEN it should contain validation_summary
        assert!(json.contains("\"passing\":4"));
        assert!(json.contains("\"with_errors\":0"));
        assert!(json.contains("\"with_warnings\":1"));
    }
}
