//! CLI handler for the `recommend` command.

use crate::app::{RecommendationRelationship, SkillRecommendations};
use crate::discovery::merge_extra_dirs;
use anyhow::Result;
use skrills_analyze::{analyze_skill, DependencyGraph, DependencyType};
use skrills_discovery::{discover_skills, extra_skill_roots};
use std::collections::{HashMap, HashSet};

/// Handle the `recommend` command.
pub(crate) fn handle_recommend_command(
    uri: String,
    skill_dirs: Vec<std::path::PathBuf>,
    format: String,
    limit: usize,
    include_quality: bool,
) -> Result<()> {
    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    if skills.is_empty() {
        println!("No skills found.");
        return Ok(());
    }

    // Build dependency graph and collect quality scores
    let mut dep_graph = DependencyGraph::new();
    let mut quality_scores: HashMap<String, f64> = HashMap::new();
    let mut uri_to_name: HashMap<String, String> = HashMap::new();

    for meta in &skills {
        let content = match std::fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let skill_uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
        uri_to_name.insert(skill_uri.clone(), meta.name.clone());

        let analysis = analyze_skill(&meta.path, &content);
        quality_scores.insert(skill_uri.clone(), analysis.quality_score);

        dep_graph.add_skill(&skill_uri);
        for dep in &analysis.dependencies.dependencies {
            if let DependencyType::Skill = dep.dep_type {
                dep_graph.add_dependency(&skill_uri, &dep.target);
            }
        }
    }

    // Check if URI exists
    if !dep_graph.skills().contains(&uri) {
        println!("Skill not found: {}", uri);
        println!("\nAvailable skills:");
        for skill_uri in dep_graph.skills().iter().take(10) {
            let name = uri_to_name.get(skill_uri).map(|s| s.as_str()).unwrap_or("");
            println!("  {} ({})", skill_uri, name);
        }
        if dep_graph.skills().len() > 10 {
            println!("  ... and {} more", dep_graph.skills().len() - 10);
        }
        return Ok(());
    }

    // Get relationships
    let dependencies: HashSet<_> = dep_graph.dependencies(&uri);
    let dependents: Vec<_> = dep_graph.dependents(&uri);
    let source_deps = &dependencies;

    // Find siblings (share common dependencies)
    let mut siblings: Vec<String> = Vec::new();
    if !source_deps.is_empty() {
        for other_uri in dep_graph.skills() {
            if other_uri == uri {
                continue;
            }
            if dependencies.contains(&other_uri) || dependents.contains(&other_uri) {
                continue;
            }
            let other_deps = dep_graph.dependencies(&other_uri);
            if !source_deps.is_disjoint(&other_deps) {
                siblings.push(other_uri);
            }
        }
    }

    // Build recommendations
    let mut recommendations = Vec::new();

    for dep_uri in &dependencies {
        let quality = if include_quality {
            quality_scores.get(dep_uri).copied()
        } else {
            None
        };
        let score = 3.0 + quality.unwrap_or(0.0);
        recommendations.push(crate::app::SkillRecommendation {
            uri: dep_uri.clone(),
            relationship: RecommendationRelationship::Dependency,
            quality_score: quality,
            score,
        });
    }

    for dep_uri in &dependents {
        let quality = if include_quality {
            quality_scores.get(dep_uri).copied()
        } else {
            None
        };
        let score = 2.0 + quality.unwrap_or(0.0);
        recommendations.push(crate::app::SkillRecommendation {
            uri: dep_uri.clone(),
            relationship: RecommendationRelationship::Dependent,
            quality_score: quality,
            score,
        });
    }

    for sib_uri in &siblings {
        let quality = if include_quality {
            quality_scores.get(sib_uri).copied()
        } else {
            None
        };
        let score = 1.0 + quality.unwrap_or(0.0);
        recommendations.push(crate::app::SkillRecommendation {
            uri: sib_uri.clone(),
            relationship: RecommendationRelationship::Sibling,
            quality_score: quality,
            score,
        });
    }

    // Sort and limit
    recommendations.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total_found = recommendations.len();
    recommendations.truncate(limit);

    let result = SkillRecommendations {
        source_uri: uri.clone(),
        total_found,
        recommendations,
    };

    // Output
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_recommendations_human(&result, &uri_to_name);
    }

    Ok(())
}

/// Print recommendations in human-readable format.
fn print_recommendations_human(
    result: &SkillRecommendations,
    uri_to_name: &HashMap<String, String>,
) {
    println!("Skill Recommendations");
    println!("=====================\n");

    println!("Source: {}", result.source_uri);
    println!(
        "Found: {} recommendations (showing {})\n",
        result.total_found,
        result.recommendations.len()
    );

    if result.recommendations.is_empty() {
        println!("No recommendations found for this skill.");
        println!("This skill has no dependencies, dependents, or siblings.");
        return;
    }

    // Group by relationship type
    let deps: Vec<_> = result
        .recommendations
        .iter()
        .filter(|r| matches!(r.relationship, RecommendationRelationship::Dependency))
        .collect();
    let dependents: Vec<_> = result
        .recommendations
        .iter()
        .filter(|r| matches!(r.relationship, RecommendationRelationship::Dependent))
        .collect();
    let siblings: Vec<_> = result
        .recommendations
        .iter()
        .filter(|r| matches!(r.relationship, RecommendationRelationship::Sibling))
        .collect();

    if !deps.is_empty() {
        println!("Dependencies (skills this skill needs):");
        for rec in deps {
            let name = uri_to_name.get(&rec.uri).map(|s| s.as_str()).unwrap_or("");
            if let Some(q) = rec.quality_score {
                println!("  {} ({}) - quality: {:.0}%", rec.uri, name, q * 100.0);
            } else {
                println!("  {} ({})", rec.uri, name);
            }
        }
        println!();
    }

    if !dependents.is_empty() {
        println!("Dependents (skills that use this skill):");
        for rec in dependents {
            let name = uri_to_name.get(&rec.uri).map(|s| s.as_str()).unwrap_or("");
            if let Some(q) = rec.quality_score {
                println!("  {} ({}) - quality: {:.0}%", rec.uri, name, q * 100.0);
            } else {
                println!("  {} ({})", rec.uri, name);
            }
        }
        println!();
    }

    if !siblings.is_empty() {
        println!("Siblings (skills sharing common dependencies):");
        for rec in siblings {
            let name = uri_to_name.get(&rec.uri).map(|s| s.as_str()).unwrap_or("");
            if let Some(q) = rec.quality_score {
                println!("  {} ({}) - quality: {:.0}%", rec.uri, name, q * 100.0);
            } else {
                println!("  {} ({})", rec.uri, name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn create_skill(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        let path = skill_dir.join("SKILL.md");
        fs::write(&path, content).expect("write skill");
        path
    }

    fn skill_with_deps(name: &str, deps: &[&str]) -> String {
        let mut content = format!(
            r#"---
name: {}
description: Test skill with dependencies
---
# {}

A test skill.
"#,
            name, name
        );
        for dep in deps {
            content.push_str(&format!(
                "\nSee [{}](skill://skrills/codex/{}) for more.\n",
                dep, dep
            ));
        }
        content
    }

    #[test]
    fn test_handle_recommend_command_empty_dir() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();

        let result = handle_recommend_command(
            "skill://test".into(),
            vec![skill_dir],
            "text".into(),
            10,
            true,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_recommend_command_not_found() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();
        create_skill(&skill_dir, "existing", &skill_with_deps("existing", &[]));

        let result = handle_recommend_command(
            "skill://nonexistent".into(),
            vec![skill_dir],
            "text".into(),
            10,
            true,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_recommend_command_with_deps() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();

        // Create skills with dependencies: a -> b -> c
        create_skill(
            &skill_dir,
            "skill-a",
            &skill_with_deps("skill-a", &["skill-b"]),
        );
        create_skill(
            &skill_dir,
            "skill-b",
            &skill_with_deps("skill-b", &["skill-c"]),
        );
        create_skill(&skill_dir, "skill-c", &skill_with_deps("skill-c", &[]));

        let result = handle_recommend_command(
            "skill://skrills/codex/skill-a".into(),
            vec![skill_dir],
            "json".into(),
            10,
            true,
        );

        assert!(result.is_ok());
    }
}
