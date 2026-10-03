//! CLI handler for the `recommend` command.

use super::skill_uri::{skill_uri, DependencyResolver};
use crate::cli::OutputFormat;
use anyhow::{bail, Result};
use skrills_analyze::{analyze_skill, DependencyType, RelationshipGraph};
use skrills_discovery::discover_skills;
use skrills_server::app::{
    rank_skill_recommendations, RecommendationRelationship, SkillRecommendations,
};
use std::collections::HashMap;

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

    // The ranking is the server's, so the CLI and the MCP tool agree (SA-44).
    let result = rank_skill_recommendations(&dep_graph, &uri, limit, |u| {
        include_quality
            .then(|| quality_scores.get(u).copied())
            .flatten()
    });

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

    /// SA-44: the same fixture and expectations as the server's
    /// `app::skill_recommendations::tests`, so `skrills recommend` and the
    /// `recommend-skills` MCP tool rank alike.
    /// a -> {b, c, f, g}; d -> b (sibling of a); e -> a (dependent of a).
    fn write_shared_fixture(dir: &std::path::Path) -> Vec<skrills_discovery::SkillMeta> {
        let links: &[(&str, &[&str])] = &[
            ("a", &["b", "c", "f", "g"]),
            ("b", &[]),
            ("c", &[]),
            ("d", &["b"]),
            ("e", &["a"]),
            ("f", &[]),
            ("g", &[]),
        ];
        for (name, deps) in links {
            let mut body =
                format!("---\nname: {name}\ndescription: Fixture skill {name}\n---\n# {name}\n");
            for dep in *deps {
                body.push_str(&format!("\nSee [{dep}](../{dep}/SKILL.md).\n"));
            }
            create_skill(dir, name, &body);
        }
        discover_skills(
            &[skrills_discovery::SkillRoot {
                root: dir.to_path_buf(),
                source: skrills_discovery::SkillSource::Extra(0),
            }],
            None,
        )
        .unwrap()
    }

    fn fixture_uri(name: &str) -> String {
        format!("skill://skrills/extra0/{name}/SKILL.md")
    }

    fn rows(result: &SkillRecommendations) -> Vec<(String, String, f64)> {
        result
            .recommendations
            .iter()
            .map(|r| (r.uri.clone(), format!("{:?}", r.relationship), r.score))
            .collect()
    }

    fn normalized(mut rows: Vec<(String, String, f64)>) -> Vec<(String, String, f64)> {
        rows.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        rows
    }

    fn expected_without_quality() -> Vec<(String, String, f64)> {
        vec![
            (fixture_uri("b"), "Dependency".into(), 3.0),
            (fixture_uri("c"), "Dependency".into(), 3.0),
            (fixture_uri("f"), "Dependency".into(), 3.0),
            (fixture_uri("g"), "Dependency".into(), 3.0),
            (fixture_uri("e"), "Dependent".into(), 2.0),
            (fixture_uri("d"), "Sibling".into(), 1.0),
        ]
    }

    #[test]
    fn cli_ranking_characterization_without_quality() {
        let tmp = tempdir().unwrap();
        let skills = write_shared_fixture(tmp.path());

        let (result, _) = recommendations_for(&fixture_uri("a"), &skills, 10, false).unwrap();

        assert_eq!(result.source_uri, fixture_uri("a"));
        assert_eq!(result.total_found, 6);
        assert!(result
            .recommendations
            .iter()
            .all(|r| r.quality_score.is_none()));
        assert_eq!(normalized(rows(&result)), expected_without_quality());

        let (limited, _) = recommendations_for(&fixture_uri("a"), &skills, 2, false).unwrap();
        assert_eq!(
            limited.total_found, 6,
            "total_found counts before the limit"
        );
        assert_eq!(limited.recommendations.len(), 2);
        assert!(limited
            .recommendations
            .iter()
            .all(|r| matches!(r.relationship, RecommendationRelationship::Dependency)));
    }

    #[test]
    fn cli_ranking_characterization_with_quality() {
        let tmp = tempdir().unwrap();
        let skills = write_shared_fixture(tmp.path());

        let (result, _) = recommendations_for(&fixture_uri("a"), &skills, 10, true).unwrap();

        assert_eq!(result.total_found, 6);
        for rec in &result.recommendations {
            let name = rec.uri.split('/').nth(4).unwrap();
            let path = tmp.path().join(name).join("SKILL.md");
            let expected = analyze_skill(&path, &fs::read_to_string(&path).unwrap()).quality_score;
            let base = match rec.relationship {
                RecommendationRelationship::Dependency => 3.0,
                RecommendationRelationship::Dependent => 2.0,
                RecommendationRelationship::Sibling => 1.0,
            };
            assert_eq!(rec.quality_score, Some(expected), "{rec:?}");
            assert_eq!(rec.score, base + expected, "{rec:?}");
        }
        assert!(result
            .recommendations
            .windows(2)
            .all(|w| w[0].score >= w[1].score));
    }

    /// SA-44: equal scores came out in `HashSet` order.
    #[test]
    fn cli_equal_scores_are_ordered_by_uri() {
        let tmp = tempdir().unwrap();
        let skills = write_shared_fixture(tmp.path());

        let (result, _) = recommendations_for(&fixture_uri("a"), &skills, 10, false).unwrap();

        assert_eq!(rows(&result), expected_without_quality());
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
