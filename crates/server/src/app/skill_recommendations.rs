//! Skill recommendation engine for `SkillService`.
//!
//! Split out of `app/mod.rs` (FU-3 of v0.8.0 refinement). This is
//! the largest single method on `SkillService` (~159 LOC of
//! dependency walking, sibling detection, and quality scoring).
//! Isolating it here keeps `SkillService::recommend_skills`
//! visible at one grep target alongside its supporting types.

use crate::metrics_types::{RecommendationRelationship, SkillRecommendation, SkillRecommendations};
use anyhow::Result;
use std::fs;

use super::SkillService;

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
        use skrills_analyze::analyze_skill;
        use std::collections::HashSet;

        let mut cache = self.cache.lock();
        cache.ensure_fresh()?;

        // Collect all skill URIs for sibling detection
        let all_uris = cache.skill_uris()?;

        // Validate that the requested URI exists
        if !all_uris.contains(&uri.to_string()) {
            anyhow::bail!("Skill not found: {}", uri);
        }

        // Get direct relationships
        let dependencies: Vec<String> = cache.dependencies_raw(uri);
        let dependents: Vec<String> = cache.dependents_raw(uri);

        // Find siblings (skills that share dependencies with this skill)
        let source_deps: HashSet<_> = dependencies.iter().cloned().collect();
        let mut siblings: Vec<String> = Vec::new();

        if !source_deps.is_empty() {
            for other_uri in &all_uris {
                if other_uri == uri {
                    continue;
                }
                // Skip if already in dependencies or dependents
                if dependencies.contains(other_uri) || dependents.contains(other_uri) {
                    continue;
                }
                let other_deps: HashSet<_> =
                    cache.dependencies_raw(other_uri).into_iter().collect();
                // Check for shared dependencies
                if !source_deps.is_disjoint(&other_deps) {
                    siblings.push(other_uri.clone());
                }
            }
        }

        // Build recommendations with scores
        let mut recommendations: Vec<SkillRecommendation> = Vec::new();

        // Dependencies get highest base score (most immediately useful)
        for dep_uri in &dependencies {
            let mut rec = SkillRecommendation {
                uri: dep_uri.clone(),
                relationship: RecommendationRelationship::Dependency,
                quality_score: None,
                score: 3.0, // Base score for dependencies
            };

            if include_quality {
                match cache.skill_by_uri(dep_uri) {
                    Ok(meta) => match fs::read_to_string(&meta.path) {
                        Ok(content) => {
                            let analysis = analyze_skill(&meta.path, &content);
                            rec.quality_score = Some(analysis.quality_score);
                            rec.score += analysis.quality_score; // Add quality bonus
                        }
                        Err(e) => {
                            tracing::warn!(uri = %dep_uri, error = %e, "Failed to read skill for quality scoring");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(uri = %dep_uri, error = %e, "Failed to find skill metadata for quality scoring");
                    }
                }
            }

            recommendations.push(rec);
        }

        // Dependents get medium base score
        for dep_uri in &dependents {
            let mut rec = SkillRecommendation {
                uri: dep_uri.clone(),
                relationship: RecommendationRelationship::Dependent,
                quality_score: None,
                score: 2.0, // Base score for dependents
            };

            if include_quality {
                match cache.skill_by_uri(dep_uri) {
                    Ok(meta) => match fs::read_to_string(&meta.path) {
                        Ok(content) => {
                            let analysis = analyze_skill(&meta.path, &content);
                            rec.quality_score = Some(analysis.quality_score);
                            rec.score += analysis.quality_score;
                        }
                        Err(e) => {
                            tracing::warn!(uri = %dep_uri, error = %e, "Failed to read skill for quality scoring");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(uri = %dep_uri, error = %e, "Failed to find skill metadata for quality scoring");
                    }
                }
            }

            recommendations.push(rec);
        }

        // Siblings get lowest base score
        for sib_uri in &siblings {
            let mut rec = SkillRecommendation {
                uri: sib_uri.clone(),
                relationship: RecommendationRelationship::Sibling,
                quality_score: None,
                score: 1.0, // Base score for siblings
            };

            if include_quality {
                match cache.skill_by_uri(sib_uri) {
                    Ok(meta) => match fs::read_to_string(&meta.path) {
                        Ok(content) => {
                            let analysis = analyze_skill(&meta.path, &content);
                            rec.quality_score = Some(analysis.quality_score);
                            rec.score += analysis.quality_score;
                        }
                        Err(e) => {
                            tracing::warn!(uri = %sib_uri, error = %e, "Failed to read skill for quality scoring");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(uri = %sib_uri, error = %e, "Failed to find skill metadata for quality scoring");
                    }
                }
            }

            recommendations.push(rec);
        }

        // Sort by score descending
        recommendations.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_found = recommendations.len();

        // Apply limit
        recommendations.truncate(limit);

        Ok(SkillRecommendations {
            source_uri: uri.to_string(),
            total_found,
            recommendations,
        })
    }
}
