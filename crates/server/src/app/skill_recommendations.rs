//! Skill recommendation engine for `SkillService`.
//!
//! The largest single method on `SkillService`: dependency walking,
//! sibling detection, and quality scoring, isolated here so it has
//! one grep target alongside its supporting types.

use crate::cache::SkillCache;
use crate::metrics_types::{RecommendationRelationship, SkillRecommendation, SkillRecommendations};
use anyhow::Result;
use skrills_analyze::analyze_skill;
use std::collections::HashSet;
use std::fs;

use super::SkillService;

/// Enriches `rec` with a quality score read from disk.
///
/// Leaves `rec` unchanged on I/O or cache failure (warnings logged).
fn apply_quality_score(rec: &mut SkillRecommendation, cache: &mut SkillCache) {
    match cache.skill_by_uri(&rec.uri) {
        Ok(meta) => match fs::read_to_string(&meta.path) {
            Ok(content) => {
                let analysis = analyze_skill(&meta.path, &content);
                rec.quality_score = Some(analysis.quality_score);
                rec.score += analysis.quality_score;
            }
            Err(e) => {
                tracing::warn!(uri = %rec.uri, error = %e, "Failed to read skill for quality scoring");
            }
        },
        Err(e) => {
            tracing::warn!(uri = %rec.uri, error = %e, "Failed to find skill metadata for quality scoring");
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

        let dependencies: Vec<String> = cache.dependencies_raw(uri);
        let dependents: Vec<String> = cache.dependents_raw(uri);

        let source_deps: HashSet<_> = dependencies.iter().cloned().collect();
        let mut siblings: Vec<String> = Vec::new();

        if !source_deps.is_empty() {
            for other_uri in &all_uris {
                if other_uri == uri {
                    continue;
                }
                if dependencies.contains(other_uri) || dependents.contains(other_uri) {
                    continue;
                }
                let other_deps: HashSet<_> =
                    cache.dependencies_raw(other_uri).into_iter().collect();
                if !source_deps.is_disjoint(&other_deps) {
                    siblings.push(other_uri.clone());
                }
            }
        }

        let mut recommendations: Vec<SkillRecommendation> = Vec::new();

        for dep_uri in &dependencies {
            let mut rec = SkillRecommendation {
                uri: dep_uri.clone(),
                relationship: RecommendationRelationship::Dependency,
                quality_score: None,
                score: 3.0,
            };
            if include_quality {
                apply_quality_score(&mut rec, &mut cache);
            }
            recommendations.push(rec);
        }

        for dep_uri in &dependents {
            let mut rec = SkillRecommendation {
                uri: dep_uri.clone(),
                relationship: RecommendationRelationship::Dependent,
                quality_score: None,
                score: 2.0,
            };
            if include_quality {
                apply_quality_score(&mut rec, &mut cache);
            }
            recommendations.push(rec);
        }

        for sib_uri in &siblings {
            let mut rec = SkillRecommendation {
                uri: sib_uri.clone(),
                relationship: RecommendationRelationship::Sibling,
                quality_score: None,
                score: 1.0,
            };
            if include_quality {
                apply_quality_score(&mut rec, &mut cache);
            }
            recommendations.push(rec);
        }

        recommendations.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_found = recommendations.len();
        recommendations.truncate(limit);

        Ok(SkillRecommendations {
            source_uri: uri.to_string(),
            total_found,
            recommendations,
        })
    }
}
