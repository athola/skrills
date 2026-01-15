//! Build usage analytics from skill usage events.

use super::{PromptAffinity, SkillUsageEvent, TimeRange, UsageAnalytics};
use crate::types::Confidence;
use std::collections::{HashMap, HashSet};

/// Tracks keyword statistics for confidence calculation.
#[derive(Default)]
struct KeywordStats {
    /// Unique skills associated with this keyword.
    skills: HashSet<String>,
    /// Number of events where this keyword appeared.
    occurrence_count: u64,
}

/// Build usage analytics from a collection of skill usage events.
pub fn build_analytics(events: Vec<SkillUsageEvent>) -> UsageAnalytics {
    if events.is_empty() {
        return UsageAnalytics::default();
    }

    let mut frequency: HashMap<String, u64> = HashMap::new();
    let mut recency: HashMap<String, u64> = HashMap::new();
    let mut cooccurrence: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let mut keyword_stats: HashMap<String, KeywordStats> = HashMap::new();

    // Track skills by session for co-occurrence
    let mut session_skills: HashMap<String, Vec<String>> = HashMap::new();

    // Track time range
    let mut min_ts = u64::MAX;
    let mut max_ts = 0u64;

    for event in &events {
        // Update frequency
        *frequency.entry(event.skill_path.clone()).or_insert(0) += 1;

        // Update recency (keep latest timestamp)
        let entry = recency.entry(event.skill_path.clone()).or_insert(0);
        if event.timestamp > *entry {
            *entry = event.timestamp;
        }

        // Track skills per session
        session_skills
            .entry(event.session_id.clone())
            .or_default()
            .push(event.skill_path.clone());

        // Track prompt -> skill mapping with occurrence counts
        if let Some(ref prompt) = event.prompt_context {
            let keywords = extract_keywords(prompt);
            for keyword in keywords {
                let stats = keyword_stats.entry(keyword).or_default();
                stats.skills.insert(event.skill_path.clone());
                stats.occurrence_count += 1;
            }
        }

        // Update time range
        if event.timestamp > 0 {
            min_ts = min_ts.min(event.timestamp);
            max_ts = max_ts.max(event.timestamp);
        }
    }

    // Build co-occurrence from session skills
    for skills in session_skills.values() {
        let unique_skills: Vec<_> = skills.iter().collect::<HashSet<_>>().into_iter().collect();
        for i in 0..unique_skills.len() {
            for j in (i + 1)..unique_skills.len() {
                let skill_a = unique_skills[i];
                let skill_b = unique_skills[j];

                // Update both directions
                *cooccurrence
                    .entry(skill_a.clone())
                    .or_default()
                    .entry(skill_b.clone())
                    .or_insert(0) += 1;
                *cooccurrence
                    .entry(skill_b.clone())
                    .or_default()
                    .entry(skill_a.clone())
                    .or_insert(0) += 1;
            }
        }
    }

    // Build prompt affinities with corrected confidence calculation
    // Confidence = occurrence_count / total_events (how often this keyword appears)
    let prompt_affinities: Vec<PromptAffinity> = keyword_stats
        .into_iter()
        .filter(|(_, stats)| !stats.skills.is_empty())
        .map(|(keyword, stats)| {
            PromptAffinity {
                keywords: vec![keyword],
                associated_skills: stats.skills.into_iter().collect(),
                // Confidence based on keyword occurrence frequency, not unique skill count
                confidence: Confidence::new(stats.occurrence_count as f64 / events.len() as f64),
            }
        })
        .collect();

    let time_range = if min_ts < u64::MAX && max_ts > 0 {
        Some(TimeRange {
            start: min_ts,
            end: max_ts,
        })
    } else {
        None
    };

    UsageAnalytics {
        frequency,
        recency,
        cooccurrence,
        prompt_affinities,
        command_history: Vec::new(), // Populated separately
        sessions_analyzed: session_skills.len(),
        time_range,
    }
}

/// Extract keywords from prompt text for affinity mapping.
fn extract_keywords(prompt: &str) -> Vec<String> {
    prompt
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|s| s.len() >= 3)
        .filter(|s| !is_stop_word(s))
        .map(|s| s.to_string())
        .collect()
}

fn is_stop_word(word: &str) -> bool {
    const STOP_WORDS: &[&str] = &[
        "the", "and", "for", "that", "this", "with", "are", "was", "were", "been", "have", "has",
        "had", "not", "but", "can", "could", "would", "should", "may", "might", "will", "shall",
        "from", "into", "about", "than", "then", "when", "where", "what", "which", "who", "how",
        "all", "each", "every", "both", "few", "more", "most", "other", "some", "such", "only",
        "same", "just", "also", "very", "even", "back", "after", "before", "between",
    ];
    STOP_WORDS.contains(&word)
}

/// Get top co-occurring skills for a given skill.
pub fn get_cooccurring_skills(
    analytics: &UsageAnalytics,
    skill: &str,
    limit: usize,
) -> Vec<(String, u64)> {
    analytics
        .cooccurrence
        .get(skill)
        .map(|coocs| {
            let mut pairs: Vec<_> = coocs.iter().map(|(k, v)| (k.clone(), *v)).collect();
            pairs.sort_by(|a, b| b.1.cmp(&a.1));
            pairs.truncate(limit);
            pairs
        })
        .unwrap_or_default()
}

/// Calculate a recency score (0.0 - 1.0) based on last usage.
pub fn recency_score(analytics: &UsageAnalytics, skill: &str, now: u64) -> f64 {
    if let Some(&last_used) = analytics.recency.get(skill) {
        if last_used == 0 || now == 0 {
            return 0.0;
        }
        // Score decays over 30 days
        let age_seconds = now.saturating_sub(last_used);
        let age_days = age_seconds as f64 / 86400.0;
        let decay_factor = 30.0; // Half-life of 30 days
        (-(age_days / decay_factor)).exp()
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_empty_analytics() {
        let analytics = build_analytics(vec![]);
        assert!(analytics.frequency.is_empty());
        assert!(analytics.cooccurrence.is_empty());
        assert_eq!(analytics.sessions_analyzed, 0);
    }

    #[test]
    fn test_build_frequency() {
        let events = vec![
            SkillUsageEvent {
                timestamp: 1000,
                skill_path: "skill-a".to_string(),
                session_id: "s1".to_string(),
                prompt_context: None,
            },
            SkillUsageEvent {
                timestamp: 2000,
                skill_path: "skill-a".to_string(),
                session_id: "s1".to_string(),
                prompt_context: None,
            },
            SkillUsageEvent {
                timestamp: 3000,
                skill_path: "skill-b".to_string(),
                session_id: "s1".to_string(),
                prompt_context: None,
            },
        ];

        let analytics = build_analytics(events);
        assert_eq!(analytics.frequency.get("skill-a"), Some(&2));
        assert_eq!(analytics.frequency.get("skill-b"), Some(&1));
    }

    #[test]
    fn test_cooccurrence() {
        let events = vec![
            SkillUsageEvent {
                timestamp: 1000,
                skill_path: "skill-a".to_string(),
                session_id: "s1".to_string(),
                prompt_context: None,
            },
            SkillUsageEvent {
                timestamp: 2000,
                skill_path: "skill-b".to_string(),
                session_id: "s1".to_string(),
                prompt_context: None,
            },
        ];

        let analytics = build_analytics(events);

        // skill-a and skill-b should be co-occurring
        assert_eq!(
            analytics
                .cooccurrence
                .get("skill-a")
                .and_then(|c| c.get("skill-b")),
            Some(&1)
        );
        assert_eq!(
            analytics
                .cooccurrence
                .get("skill-b")
                .and_then(|c| c.get("skill-a")),
            Some(&1)
        );
    }

    #[test]
    fn test_recency_score() {
        let mut analytics = UsageAnalytics::default();
        analytics.recency.insert("skill-a".to_string(), 1000);

        // Just used
        assert!((recency_score(&analytics, "skill-a", 1000) - 1.0).abs() < 0.01);

        // 30 days ago (half-life)
        let thirty_days_later = 1000 + 30 * 86400;
        let score = recency_score(&analytics, "skill-a", thirty_days_later);
        assert!(score > 0.3 && score < 0.4); // Should be around e^-1 ≈ 0.37
    }

    #[test]
    fn test_extract_keywords() {
        let prompt = "Help me test the Rust code with pytest";
        let keywords = extract_keywords(prompt);
        assert!(keywords.contains(&"help".to_string()));
        assert!(keywords.contains(&"test".to_string()));
        assert!(keywords.contains(&"rust".to_string()));
        assert!(keywords.contains(&"code".to_string()));
        assert!(keywords.contains(&"pytest".to_string()));
        // Stop words should be filtered
        assert!(!keywords.contains(&"the".to_string()));
        assert!(!keywords.contains(&"with".to_string()));
    }

    #[test]
    fn test_prompt_affinity_confidence() {
        // Test that confidence is based on keyword occurrence count, not unique skill count
        let events = vec![
            SkillUsageEvent {
                timestamp: 1000,
                skill_path: "skill-a".to_string(),
                session_id: "s1".to_string(),
                prompt_context: Some("help with rust code".to_string()),
            },
            SkillUsageEvent {
                timestamp: 2000,
                skill_path: "skill-a".to_string(), // Same skill, same keyword
                session_id: "s1".to_string(),
                prompt_context: Some("rust debugging".to_string()),
            },
            SkillUsageEvent {
                timestamp: 3000,
                skill_path: "skill-b".to_string(),
                session_id: "s1".to_string(),
                prompt_context: Some("python testing".to_string()),
            },
        ];

        let analytics = build_analytics(events);

        // Find the "rust" keyword affinity
        let rust_affinity = analytics
            .prompt_affinities
            .iter()
            .find(|a| a.keywords.contains(&"rust".to_string()));

        assert!(rust_affinity.is_some(), "Should have rust keyword affinity");
        let affinity = rust_affinity.unwrap();

        // "rust" appears in 2 of 3 events, so confidence should be 2/3 ≈ 0.667
        // NOT 1/3 (which would be if we used unique skill count = 1)
        assert!(
            affinity.confidence.value() > 0.6 && affinity.confidence.value() < 0.7,
            "Confidence should be ~0.667 (2/3 events), got {}",
            affinity.confidence.value()
        );

        // Should have 1 unique skill associated (skill-a used twice with "rust")
        assert_eq!(
            affinity.associated_skills.len(),
            1,
            "Should have 1 unique skill"
        );
    }
}
