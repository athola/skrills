//! CLI handler for the `recommend` command.

use super::skill_uri::{skill_uri, DependencyResolver};
use crate::cli::OutputFormat;
use anyhow::{bail, Result};
use skrills_analyze::{analyze_skill, DependencyType, RelationshipGraph};
use skrills_discovery::discover_skills;
use skrills_server::app::{RecommendationRelationship, SkillRecommendations};
use std::collections::{HashMap, HashSet};

/// Handle the `recommend` command.
pub(crate) fn handle_recommend_command(
    uri: String,
    skill_dirs: Vec<std::path::PathBuf>,
    format: OutputFormat,
    limit: usize,
    include_quality: bool,
) -> Result<()> {
    let roots = crate::commands::skill_roots_for(&skill_dirs);
    let skills = discover_skills(&roots, None)?;
    let (result, uri_to_name) = recommendations_for(&uri, &skills, limit, include_quality)?;
    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_recommendations_human(&result, &uri_to_name);
    }
    Ok(())
}

/// Ranks the skills related to `uri`: its dependencies, then its dependents,
/// then the siblings that share a dependency with it. Also returns each
/// URI's skill name, for the human-readable listing.
fn recommendations_for(
    uri: &str,
    skills: &[skrills_discovery::SkillMeta],
    limit: usize,
    include_quality: bool,
) -> Result<(SkillRecommendations, HashMap<String, String>)> {
    let uri = uri.to_string();
    if skills.is_empty() {
        bail!("Skill not found: {uri} (no skills were discovered)");
    }
    let resolver = DependencyResolver::new(skills);

    // Build dependency graph and collect quality scores
    let mut dep_graph = RelationshipGraph::new();
    let mut quality_scores: HashMap<String, f64> = HashMap::new();
    let mut uri_to_name: HashMap<String, String> = HashMap::new();

    for meta in skills {
        let content = match std::fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %meta.path.display(),
                    error = %e,
                    "Failed to read skill file, skipping in recommendation analysis"
                );
                continue;
            }
        };

        let skill_uri = skill_uri(meta);
        uri_to_name.insert(skill_uri.clone(), meta.name.clone());

        let analysis = analyze_skill(&meta.path, &content);
        quality_scores.insert(skill_uri.clone(), analysis.quality_score);

        dep_graph.add_skill(&skill_uri);
        for dep in &analysis.dependencies.dependencies {
            if let DependencyType::Skill = dep.dep_type {
                if let Some(target) = resolver.resolve(&meta.path, &dep.target) {
                    dep_graph.add_dependency(&skill_uri, &target);
                }
            }
        }
    }

    // Check if URI exists
    if !dep_graph.skills().contains(&uri) {
        let mut known: Vec<String> = dep_graph.skills();
        known.sort();
        let mut listing: Vec<String> = known
            .iter()
            .take(10)
            .map(|skill_uri| {
                let name = uri_to_name.get(skill_uri).map(|s| s.as_str()).unwrap_or("");
                format!("  {skill_uri} ({name})")
            })
            .collect();
        if known.len() > 10 {
            listing.push(format!("  ... and {} more", known.len() - 10));
        }
        bail!(
            "Skill not found: {uri}\n\nAvailable skills:\n{}",
            listing.join("\n")
        );
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
        recommendations.push(skrills_server::app::SkillRecommendation {
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
        recommendations.push(skrills_server::app::SkillRecommendation {
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
        recommendations.push(skrills_server::app::SkillRecommendation {
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

    Ok((result, uri_to_name))
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
            OutputFormat::Text,
            10,
            true,
        );

        let err = result.expect_err("an unknown URI is an error, not an empty success");
        assert!(err.to_string().contains("Skill not found"), "{err}");
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
            OutputFormat::Text,
            10,
            true,
        );

        // SA-34: this printed "Skill not found" to stdout and exited 0, even
        // under --format json.
        let err = result.expect_err("an unknown URI is an error");
        let msg = err.to_string();
        assert!(
            msg.contains("Skill not found: skill://nonexistent"),
            "{msg}"
        );
        assert!(
            msg.contains("existing"),
            "the available skills are listed: {msg}"
        );
    }

    /// SA-25 / SA-49: relative links were added to the graph verbatim, and the
    /// old test only asserted that the handler returned `Ok`.
    #[test]
    fn relative_links_become_dependency_and_dependent_recommendations() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        let link = |name: &str, target: &str| {
            format!(
                "---\nname: {name}\ndescription: Linked test skill {name}\n---\n# {name}\n\nSee [next]({target}).\n"
            )
        };
        create_skill(
            &skill_dir,
            "skill-a",
            &link("skill-a", "../skill-b/SKILL.md"),
        );
        create_skill(
            &skill_dir,
            "skill-b",
            &link("skill-b", "../skill-c/SKILL.md"),
        );
        create_skill(&skill_dir, "skill-c", &skill_with_deps("skill-c", &[]));
        let skills = discover_skills(
            &[skrills_discovery::SkillRoot {
                root: skill_dir,
                source: skrills_discovery::SkillSource::Extra(0),
            }],
            None,
        )
        .unwrap();
        let uri_of = |name: &str| {
            skill_uri(
                skills
                    .iter()
                    .find(|s| s.name.starts_with(name))
                    .expect("discovered"),
            )
        };

        let (result, _) = recommendations_for(&uri_of("skill-b"), &skills, 10, false).unwrap();

        let by_uri: HashMap<_, _> = result
            .recommendations
            .iter()
            .map(|r| (r.uri.clone(), format!("{:?}", r.relationship)))
            .collect();
        assert_eq!(
            by_uri.get(&uri_of("skill-c")),
            Some(&"Dependency".to_string()),
            "{result:?}"
        );
        assert_eq!(
            by_uri.get(&uri_of("skill-a")),
            Some(&"Dependent".to_string()),
            "{result:?}"
        );
        assert!(
            result.recommendations.iter().all(|r| !r.uri.contains("..")),
            "no literal link target may appear: {result:?}"
        );
    }
}
