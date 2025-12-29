//! Trigram-based similarity matching for fuzzy skill search.
//!
//! Uses the `trigram` crate to compute string similarity between queries
//! and skill names/descriptions. This enables typo-tolerant matching and
//! fuzzy search across the skill library.

use trigram::similarity;

/// Default similarity threshold for fuzzy matching.
pub const DEFAULT_THRESHOLD: f64 = 0.3;

/// Skill metadata for similarity matching.
#[derive(Debug, Clone)]
pub struct SkillMatch {
    /// Skill URI.
    pub uri: String,
    /// Skill name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Similarity score (0.0 - 1.0).
    pub similarity: f64,
    /// Which field matched best (name or description).
    pub matched_field: MatchedField,
}

/// Which field contributed the highest similarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedField {
    /// Name matched best.
    Name,
    /// Description matched best.
    Description,
    /// Both matched equally (or no match).
    Both,
}

/// Compute trigram similarity between two strings.
///
/// Returns a value between 0.0 (no similarity) and 1.0 (identical after normalization).
pub fn compute_similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    f64::from(similarity(&a.to_lowercase(), &b.to_lowercase()))
}

/// Find the best word match within a haystack string.
///
/// Splits the haystack into words and returns the highest similarity score
/// for any word matching the needle.
pub fn best_word_match(needle: &str, haystack: &str) -> f64 {
    if needle.is_empty() || haystack.is_empty() {
        return 0.0;
    }

    let needle_lower = needle.to_lowercase();

    haystack
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|word| word.len() >= 3)
        .map(|word| f64::from(similarity(&needle_lower, &word.to_lowercase())))
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(0.0)
}

/// Match a query against a skill's name and description.
///
/// Returns the best match score and which field matched.
pub fn match_skill(query: &str, name: &str, description: Option<&str>) -> (f64, MatchedField) {
    let name_score = compute_similarity(query, name);

    let desc_score = description
        .map(|d| best_word_match(query, d))
        .unwrap_or(0.0);

    if name_score >= desc_score {
        if name_score == desc_score {
            (name_score, MatchedField::Both)
        } else {
            (name_score, MatchedField::Name)
        }
    } else {
        (desc_score, MatchedField::Description)
    }
}

/// Simplified skill info for matching.
#[derive(Clone)]
pub struct SkillInfo<'a> {
    /// Skill URI.
    pub uri: &'a str,
    /// Skill name.
    pub name: &'a str,
    /// Optional description.
    pub description: Option<&'a str>,
}

/// Find similar skills based on a query string.
///
/// Returns skills sorted by similarity score (highest first),
/// filtered by the given threshold.
pub fn find_similar_skills<'a>(
    query: &str,
    skills: impl IntoIterator<Item = SkillInfo<'a>>,
    threshold: f64,
) -> Vec<SkillMatch> {
    let mut matches: Vec<SkillMatch> = skills
        .into_iter()
        .filter_map(|skill| {
            let (score, field) = match_skill(query, skill.name, skill.description);
            if score >= threshold {
                Some(SkillMatch {
                    uri: skill.uri.to_string(),
                    name: skill.name.to_string(),
                    description: skill.description.map(|s| s.to_string()),
                    similarity: score,
                    matched_field: field,
                })
            } else {
                None
            }
        })
        .collect();

    // Sort by similarity descending
    matches.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap());

    matches
}

/// Check if a query has high similarity to any skill.
///
/// Returns true if the query matches any skill above the threshold.
pub fn has_similar_skill<'a>(
    query: &str,
    skills: impl IntoIterator<Item = SkillInfo<'a>>,
    threshold: f64,
) -> bool {
    skills.into_iter().any(|skill| {
        let (score, _) = match_skill(query, skill.name, skill.description);
        score >= threshold
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_similarity_identical() {
        assert!((compute_similarity("database", "database") - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_similarity_typo() {
        // "databas" should have high similarity to "database"
        let score = compute_similarity("databas", "database");
        assert!(
            score >= 0.6,
            "Expected high similarity for typo, got {score}"
        );
    }

    #[test]
    fn test_compute_similarity_variation() {
        // "color" and "colour" should be similar
        let score = compute_similarity("color", "colour");
        assert!(score > 0.3, "Expected moderate similarity, got {score}");
    }

    #[test]
    fn test_compute_similarity_unrelated() {
        // Unrelated strings should have low similarity
        let score = compute_similarity("database", "frontend");
        assert!(score < 0.3, "Expected low similarity, got {score}");
    }

    #[test]
    fn test_compute_similarity_empty() {
        assert_eq!(compute_similarity("", "test"), 0.0);
        assert_eq!(compute_similarity("test", ""), 0.0);
    }

    #[test]
    fn test_best_word_match() {
        let haystack = "This is a database management skill for PostgreSQL";
        let score = best_word_match("database", haystack);
        assert!(score > 0.9, "Expected exact word match, got {score}");

        let score2 = best_word_match("postgres", haystack);
        assert!(score2 > 0.5, "Expected partial match, got {score2}");
    }

    #[test]
    fn test_match_skill_name_priority() {
        // "database" matching "database-tools" should match name better than description
        let (score, field) = match_skill(
            "database",
            "database-tools",
            Some("Code analysis utilities"),
        );
        assert!(score > 0.5, "Expected score > 0.5, got {score}");
        assert_eq!(field, MatchedField::Name);
    }

    #[test]
    fn test_match_skill_description_match() {
        let (score, field) = match_skill(
            "testing",
            "pytest-helper",
            Some("Advanced testing framework support"),
        );
        assert!(score > 0.5);
        // Description contains "testing" as a better match
        assert_eq!(field, MatchedField::Description);
    }

    #[test]
    fn test_find_similar_skills() {
        let skills = vec![
            SkillInfo {
                uri: "skill://test/database",
                name: "database",
                description: Some("SQL database operations"),
            },
            SkillInfo {
                uri: "skill://test/frontend",
                name: "frontend",
                description: Some("React frontend development"),
            },
            SkillInfo {
                uri: "skill://test/data-analysis",
                name: "data-analysis",
                description: Some("Data processing and analysis"),
            },
        ];

        let matches = find_similar_skills("databas", skills, 0.3);

        // Should find "database" and possibly "data-analysis"
        assert!(!matches.is_empty());
        assert_eq!(matches[0].name, "database");
        assert!(
            matches[0].similarity >= 0.6,
            "Expected similarity >= 0.6, got {}",
            matches[0].similarity
        );
    }

    #[test]
    fn test_find_similar_skills_threshold() {
        let skills = [SkillInfo {
            uri: "skill://test/foo",
            name: "foo",
            description: None,
        }];

        let matches = find_similar_skills("completely-different", skills, 0.5);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_has_similar_skill() {
        let skills = [SkillInfo {
            uri: "skill://test/database",
            name: "database",
            description: None,
        }];

        assert!(has_similar_skill("databas", skills.iter().cloned(), 0.5));
    }
}
