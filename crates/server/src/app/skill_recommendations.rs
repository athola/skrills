//! Skill recommendation engine.
//!
//! The one ranking used by the `recommend-skills` MCP tool
//! (`SkillService::recommend_skills`) and by `skrills recommend`
//! (through [`rank_skill_recommendations`]). Both used to carry their own
//! copy of it (SA-44).

use crate::cache::SkillCache;
use crate::metrics_types::{RecommendationRelationship, SkillRecommendation, SkillRecommendations};
use anyhow::Result;
use skrills_analyze::{analyze_skill, RelationshipGraph};
use std::collections::HashSet;
use std::fs;

use super::SkillService;

/// The skills related to `uri`, in order: its dependencies, its dependents,
/// then the siblings that share a dependency with it.
///
/// `all_uris` is every skill in the graph and `dependencies_of` returns a
/// skill's direct dependencies.
pub(crate) fn related_skills(
    uri: &str,
    all_uris: &[String],
    dependencies_of: impl Fn(&str) -> Vec<String>,
    dependents: Vec<String>,
) -> Vec<(String, RecommendationRelationship)> {
    let dependencies = dependencies_of(uri);
    let source_deps: HashSet<&String> = dependencies.iter().collect();

    let mut siblings: Vec<String> = Vec::new();
    if !source_deps.is_empty() {
        for other_uri in all_uris {
            if other_uri == uri
                || dependencies.contains(other_uri)
                || dependents.contains(other_uri)
            {
                continue;
            }
            if dependencies_of(other_uri)
                .iter()
                .any(|d| source_deps.contains(d))
            {
                siblings.push(other_uri.clone());
            }
        }
    }

    let tagged = |uris: Vec<String>, relationship: RecommendationRelationship| {
        uris.into_iter().map(move |u| (u, relationship.clone()))
    };
    tagged(dependencies, RecommendationRelationship::Dependency)
        .chain(tagged(dependents, RecommendationRelationship::Dependent))
        .chain(tagged(siblings, RecommendationRelationship::Sibling))
        .collect()
}

/// Scores `related` (3 for a dependency, 2 for a dependent, 1 for a
/// sibling, plus the quality score when `quality` returns one), sorts by
/// score and then URI, and keeps the first `limit`.
fn rank(
    source_uri: &str,
    related: Vec<(String, RecommendationRelationship)>,
    limit: usize,
    mut quality: impl FnMut(&str) -> Option<f64>,
) -> SkillRecommendations {
    let mut recommendations: Vec<SkillRecommendation> = related
        .into_iter()
        .map(|(uri, relationship)| {
            let base = match relationship {
                RecommendationRelationship::Dependency => 3.0,
                RecommendationRelationship::Dependent => 2.0,
                RecommendationRelationship::Sibling => 1.0,
            };
            let quality_score = quality(&uri);
            SkillRecommendation {
                score: base + quality_score.unwrap_or(0.0),
                uri,
                relationship,
                quality_score,
            }
        })
        .collect();

    // Ties are broken by URI: dependencies come out of a `HashSet`, so
    // without it the same request could list them in a different order.
    recommendations.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.uri.cmp(&b.uri)));

    let total_found = recommendations.len();
    recommendations.truncate(limit);

    SkillRecommendations {
        source_uri: source_uri.to_string(),
        total_found,
        recommendations,
    }
}

/// Ranks the skills related to `uri` in `graph`, the way the
/// `recommend-skills` MCP tool does.
///
/// `quality` returns a skill's quality score (0.0 to 1.0) when quality
/// should count, or `None`. A `uri` missing from `graph` yields no
/// recommendations; callers report unknown skills themselves.
pub fn rank_skill_recommendations(
    graph: &RelationshipGraph,
    uri: &str,
    limit: usize,
    quality: impl FnMut(&str) -> Option<f64>,
) -> SkillRecommendations {
    let related = related_skills(
        uri,
        &graph.skills(),
        |u| graph.dependencies(u).into_iter().collect(),
        graph.dependents(uri),
    );
    rank(uri, related, limit, quality)
}

/// The quality score of the skill at `uri`, read from disk.
///
/// `None` on I/O or cache failure (warnings logged).
fn quality_score(uri: &str, cache: &mut SkillCache) -> Option<f64> {
    match cache.skill_by_uri(uri) {
        Ok(meta) => match fs::read_to_string(&meta.path) {
            Ok(content) => Some(analyze_skill(&meta.path, &content).quality_score),
            Err(e) => {
                tracing::warn!(uri = %uri, error = %e, "Failed to read skill for quality scoring");
                None
            }
        },
        Err(e) => {
            tracing::warn!(uri = %uri, error = %e, "Failed to find skill metadata for quality scoring");
            None
        }
    }
}

impl SkillService {
    /// Gets skill recommendations based on dependencies.
    ///
    /// The algorithm:
    /// 1. Get direct dependencies of the skill (skills it needs)
    /// 2. Get direct dependents (skills that need it)
    /// 3. Find sibling skills (share common dependencies)
    /// 4. Rank by relationship type and optionally quality score
    pub(crate) fn recommend_skills(
        &self,
        uri: &str,
        limit: usize,
        include_quality: bool,
    ) -> Result<SkillRecommendations> {
        let mut cache = self.cache.lock();
        cache.ensure_fresh()?;

        let all_uris = cache.skill_uris()?;

        if !all_uris.contains(&uri.to_string()) {
            anyhow::bail!("Skill not found: {}", uri);
        }

        let related = related_skills(
            uri,
            &all_uris,
            |u| cache.dependencies_raw(u),
            cache.dependents_raw(uri),
        );

        Ok(rank(uri, related, limit, |u| {
            include_quality
                .then(|| quality_score(u, &mut cache))
                .flatten()
        }))
    }
}

#[cfg(test)]
mod tests {
    //! SA-44: the CLI's `recommend` command pins the same fixture and the same
    //! expectations in `crates/cli/src/commands/recommend.rs`, so the two
    //! front ends cannot drift apart again.

    use super::*;
    use skrills_discovery::{SkillRoot, SkillSource};
    use std::path::Path;
    use std::time::Duration;

    /// a -> {b, c, f, g}; d -> b (sibling of a); e -> a (dependent of a).
    fn write_fixture(dir: &Path) {
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
            let skill_dir = dir.join(name);
            fs::create_dir_all(&skill_dir).unwrap();
            let mut body =
                format!("---\nname: {name}\ndescription: Fixture skill {name}\n---\n# {name}\n");
            for dep in *deps {
                body.push_str(&format!("\nSee [{dep}](../{dep}/SKILL.md).\n"));
            }
            fs::write(skill_dir.join("SKILL.md"), body).unwrap();
        }
    }

    fn uri(name: &str) -> String {
        format!("skill://skrills/extra0/{name}/SKILL.md")
    }

    fn service_for(dir: &Path) -> SkillService {
        let roots = vec![SkillRoot {
            root: dir.to_path_buf(),
            source: SkillSource::Extra(0),
        }];
        let service =
            SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
        service.invalidate_cache().unwrap();
        service
    }

    fn rows(result: &SkillRecommendations) -> Vec<(String, String, f64)> {
        result
            .recommendations
            .iter()
            .map(|r| (r.uri.clone(), format!("{:?}", r.relationship), r.score))
            .collect()
    }

    /// Score descending, then URI: before SA-44 the order among equal scores
    /// was random, so the characterization compares in this order.
    fn normalized(mut rows: Vec<(String, String, f64)>) -> Vec<(String, String, f64)> {
        rows.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        rows
    }

    fn expected_without_quality() -> Vec<(String, String, f64)> {
        vec![
            (uri("b"), "Dependency".into(), 3.0),
            (uri("c"), "Dependency".into(), 3.0),
            (uri("f"), "Dependency".into(), 3.0),
            (uri("g"), "Dependency".into(), 3.0),
            (uri("e"), "Dependent".into(), 2.0),
            (uri("d"), "Sibling".into(), 1.0),
        ]
    }

    #[test]
    fn server_ranking_characterization_without_quality() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path());
        let service = service_for(tmp.path());

        let result = service.recommend_skills(&uri("a"), 10, false).unwrap();

        assert_eq!(result.source_uri, uri("a"));
        assert_eq!(result.total_found, 6);
        assert!(result
            .recommendations
            .iter()
            .all(|r| r.quality_score.is_none()));
        assert_eq!(normalized(rows(&result)), expected_without_quality());

        let limited = service.recommend_skills(&uri("a"), 2, false).unwrap();
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
    fn server_ranking_characterization_with_quality() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path());
        let service = service_for(tmp.path());

        let result = service.recommend_skills(&uri("a"), 10, true).unwrap();

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

    /// SA-44: equal scores came out in `HashSet` order, so the same request
    /// could list its dependencies differently from one run to the next.
    #[test]
    fn equal_scores_are_ordered_by_uri() {
        let tmp = tempfile::tempdir().unwrap();
        write_fixture(tmp.path());
        let service = service_for(tmp.path());

        let result = service.recommend_skills(&uri("a"), 10, false).unwrap();

        assert_eq!(rows(&result), expected_without_quality());
    }
}
