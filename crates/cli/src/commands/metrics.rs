//! CLI handler for the `metrics` command.

use crate::cli::OutputFormat;
use anyhow::Result;
use skrills_discovery::discover_skills;
use skrills_server::app::{build_dependency_graph, compute_skill_metrics, SkillMetrics};

/// Handle the `metrics` command.
pub(crate) fn handle_metrics_command(
    skill_dirs: Vec<std::path::PathBuf>,
    format: OutputFormat,
    include_validation: bool,
) -> Result<()> {
    let roots = crate::commands::skill_roots_for(&skill_dirs);
    let skills = discover_skills(&roots, None)?;
    let metrics = compute_metrics(&skills, include_validation);

    if format.is_json() {
        // No skills is the same document with zeros, so a consumer reads the
        // same keys either way.
        println!("{}", serde_json::to_string_pretty(&metrics)?);
    } else if metrics.total_skills == 0 {
        println!("No skills found.");
    } else {
        print_metrics_human(&metrics);
    }
    Ok(())
}

/// Aggregates the metrics for `skills` the way the `skill-metrics` MCP tool
/// does (SA-44). A skill that cannot be read is logged and left out of every
/// count, `total_skills` included.
fn compute_metrics(
    skills: &[skrills_discovery::SkillMeta],
    include_validation: bool,
) -> SkillMetrics {
    compute_skill_metrics(skills, &build_dependency_graph(skills), include_validation)
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
    use skrills_server::app::{
        DependencyStats, HubSkill, MetricsValidationSummary, QualityDistribution, SkillTokenInfo,
        TokenStats,
    };
    use std::collections::HashMap;
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
        result.expect("metrics command on empty dir should succeed");
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
        result.expect("metrics command with single skill should succeed");
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
        result.expect("metrics command with validation should succeed");
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
description: A thorough skill with good documentation
---
# High Quality Skill

## Overview
This skill provides the full feature set.

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
        result.expect("metrics command with multiple skills should succeed");
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

    fn discovered(dir: &std::path::Path) -> Vec<skrills_discovery::SkillMeta> {
        discover_skills(
            &[skrills_discovery::SkillRoot {
                root: dir.to_path_buf(),
                source: skrills_discovery::SkillSource::Extra(0),
            }],
            None,
        )
        .unwrap()
    }

    /// SA-35: the no-skills JSON used different keys (`skills_by_source`,
    /// `quality`, `tokens`) from the serialized `SkillMetrics`.
    #[test]
    fn empty_metrics_serialize_with_the_same_keys_as_non_empty() {
        let tmp = tempdir().unwrap();
        create_skill(tmp.path(), "a", &minimal_skill_content("a", "Test skill"));

        let empty = serde_json::to_value(compute_metrics(&[], false)).unwrap();
        let full = serde_json::to_value(compute_metrics(&discovered(tmp.path()), false)).unwrap();

        let keys =
            |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&empty), keys(&full));
        assert_eq!(empty["total_skills"], 0);
        assert_eq!(full["total_skills"], 1);
    }

    /// SA-35: `total_skills` counted files that were skipped as unreadable.
    #[test]
    fn an_unreadable_skill_is_left_out_of_the_total() {
        let tmp = tempdir().unwrap();
        create_skill(tmp.path(), "a", &minimal_skill_content("a", "Test skill"));
        let mut skills = discovered(tmp.path());
        let mut gone = skills[0].clone();
        gone.name = "gone/SKILL.md".into();
        gone.path = tmp.path().join("gone/SKILL.md");
        skills.push(gone);

        let metrics = compute_metrics(&skills, false);

        assert_eq!(metrics.total_skills, 1);
        assert_eq!(metrics.by_source.values().sum::<usize>(), 1);
    }

    /// SA-44: `skrills metrics` and the `skill-metrics` MCP tool report the
    /// same numbers for the same skills: b is a hub for a and c, d is an
    /// orphan.
    #[test]
    fn cli_and_server_metrics_agree() {
        let _g = skrills_test_utils::env_guard();
        let home = tempdir().unwrap();
        let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        let _dirs = skrills_test_utils::set_env_var("SKRILLS_EXTRA_SKILL_DIRS", None);
        let tmp = tempdir().unwrap();
        let linking = |name: &str| {
            format!("---\nname: {name}\ndescription: Links to b\n---\n# {name}\n\nSee [b](../b/SKILL.md).\n")
        };
        create_skill(tmp.path(), "a", &linking("a"));
        create_skill(tmp.path(), "c", &linking("c"));
        create_skill(tmp.path(), "b", &minimal_skill_content("b", "Hub"));
        create_skill(tmp.path(), "d", &minimal_skill_content("d", "Orphan"));

        let cli = serde_json::to_value(compute_metrics(&discovered(tmp.path()), true)).unwrap();
        let service = skrills_server::app::SkillService::new_with_ttl(
            vec![tmp.path().to_path_buf()],
            std::time::Duration::from_secs(60),
        )
        .unwrap();
        let server = serde_json::to_value(service.compute_metrics(true).unwrap()).unwrap();

        assert_eq!(cli, server);
        assert_eq!(cli["dependency_stats"]["orphan_count"], 1);
        assert_eq!(cli["dependency_stats"]["total_dependencies"], 2);
    }

    /// An unreadable skill is no orphan either: it is left out of every count.
    #[test]
    fn an_unreadable_skill_is_not_counted_as_an_orphan() {
        let tmp = tempdir().unwrap();
        create_skill(tmp.path(), "a", &minimal_skill_content("a", "Test skill"));
        let mut skills = discovered(tmp.path());
        let mut gone = skills[0].clone();
        gone.name = "gone/SKILL.md".into();
        gone.path = tmp.path().join("gone/SKILL.md");
        skills.push(gone);

        let metrics = compute_metrics(&skills, false);

        assert_eq!(metrics.dependency_stats.orphan_count, 1);
    }

    /// SA-25: relative links count as edges between the linked skills.
    #[test]
    fn relative_links_are_counted_as_dependencies() {
        let tmp = tempdir().unwrap();
        create_skill(
            tmp.path(),
            "a",
            "---\nname: a\ndescription: Links to b\n---\n# A\n\nSee [b](../b/SKILL.md).\n",
        );
        create_skill(tmp.path(), "b", &minimal_skill_content("b", "Linked to"));

        let metrics = compute_metrics(&discovered(tmp.path()), false);

        assert_eq!(metrics.dependency_stats.total_dependencies, 1);
        assert_eq!(metrics.dependency_stats.hub_skills.len(), 1);
        assert!(metrics.dependency_stats.hub_skills[0]
            .uri
            .ends_with("b/SKILL.md"));
    }
}
