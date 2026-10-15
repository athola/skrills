//! Build usage analytics from skill usage events.

use super::{PromptAffinity, SkillUsageEvent, TimeRange, UsageAnalytics};
use crate::types::Confidence;
use std::collections::{HashMap, HashSet};

/// Minimum length for a keyword to be considered meaningful.
const MIN_KEYWORD_LENGTH: usize = 3;
/// Most prompt affinities kept. Every distinct prompt word would otherwise
/// become an entry, and the cache that holds them grows with history.
pub(crate) const MAX_PROMPT_AFFINITIES: usize = 500;
/// Tracks keyword statistics for confidence calculation.
#[derive(Default)]
struct KeywordStats {
    /// Events per skill in which this keyword appeared.
    skills: HashMap<String, u64>,
    /// Number of events where this keyword appeared.
    occurrence_count: u64,
}

/// The key a skill is counted under, whatever form an event names it in.
///
/// Session logs name one skill as a `skill://` URI, a path to its SKILL.md, a
/// `plugin:name` reference or a bare name, so analytics and the lookups made
/// against them both reduce it to the skill's directory name. Skills that
/// share a name in different roots are counted together.
pub(crate) fn skill_key(skill: &str) -> &str {
    let trimmed = skill.trim_end_matches(['/', '\\']);
    let trimmed = trimmed
        .strip_suffix("SKILL.md")
        .map_or(trimmed, |t| t.trim_end_matches(['/', '\\']));
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let key = last.rsplit(':').next().unwrap_or(last);
    if key.is_empty() {
        skill
    } else {
        key
    }
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
        let key = skill_key(&event.skill_path).to_string();

        // Update frequency
        *frequency.entry(key.clone()).or_insert(0) += 1;

        // Update recency (keep latest timestamp)
        let entry = recency.entry(key.clone()).or_insert(0);
        if event.timestamp > *entry {
            *entry = event.timestamp;
        }

        // Track skills per session
        session_skills
            .entry(event.session_id.clone())
            .or_default()
            .push(key.clone());

        // Track prompt -> skill mapping with occurrence counts
        if let Some(ref prompt) = event.prompt_context {
            let keywords = extract_keywords(prompt);
            // Deduplicate keywords per event to avoid inflating occurrence counts
            let unique_keywords: HashSet<_> = keywords.into_iter().collect();
            for keyword in unique_keywords {
                let stats = keyword_stats.entry(keyword).or_default();
                *stats.skills.entry(key.clone()).or_insert(0) += 1;
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

    // Confidence is how specific a keyword is to its skill: the share of the
    // keyword's events that used its most common skill. A word seen once says
    // nothing (and is most often an identifier or hash), so it is dropped.
    let mut ranked: Vec<(f64, u64, PromptAffinity)> = keyword_stats
        .into_iter()
        .filter(|(_, stats)| stats.occurrence_count > 1)
        .map(|(keyword, stats)| {
            let mut skills: Vec<(String, u64)> = stats.skills.into_iter().collect();
            skills.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let top = skills.first().map_or(0, |s| s.1);
            let confidence = top as f64 / stats.occurrence_count as f64;
            let affinity = PromptAffinity {
                keywords: vec![keyword],
                associated_skills: skills.into_iter().map(|(skill, _)| skill).collect(),
                confidence: Confidence::new(confidence),
            };
            (confidence, stats.occurrence_count, affinity)
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.keywords.cmp(&b.2.keywords))
    });
    ranked.truncate(MAX_PROMPT_AFFINITIES);
    let prompt_affinities: Vec<PromptAffinity> = ranked.into_iter().map(|(_, _, a)| a).collect();

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
        .filter(|s| s.len() >= MIN_KEYWORD_LENGTH)
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
        .or_else(|| analytics.cooccurrence.get(skill_key(skill)))
        .map(|coocs| {
            let mut pairs: Vec<_> = coocs.iter().map(|(k, v)| (k.clone(), *v)).collect();
            pairs.sort_by_key(|b| std::cmp::Reverse(b.1));
            pairs.truncate(limit);
            pairs
        })
        .unwrap_or_default()
}

/// Calculate a recency score (0.0 - 1.0) based on last usage.
pub fn recency_score(analytics: &UsageAnalytics, skill: &str, now: u64) -> f64 {
    let last_used = analytics
        .recency
        .get(skill)
        .or_else(|| analytics.recency.get(skill_key(skill)));
    if let Some(&last_used) = last_used {
        if last_used == 0 || now == 0 {
            return 0.0;
        }
        // Score decays with a 30-day e-folding time (score ≈ 0.37 at 30 days).
        // True half-life is ~20.8 days (30 * ln(2)).
        let age_seconds = now.saturating_sub(last_used);
        let age_days = age_seconds as f64 / 86400.0;
        let decay_factor = 30.0;
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

    fn event(skill: &str, prompt: &str) -> SkillUsageEvent {
        SkillUsageEvent {
            timestamp: 1000,
            skill_path: skill.to_string(),
            session_id: "s1".to_string(),
            prompt_context: Some(prompt.to_string()),
        }
    }

    fn affinity<'a>(analytics: &'a UsageAnalytics, keyword: &str) -> Option<&'a PromptAffinity> {
        analytics
            .prompt_affinities
            .iter()
            .find(|a| a.keywords.iter().any(|k| k == keyword))
    }

    #[test]
    fn test_prompt_affinity_confidence() {
        // "rust" appears twice, both times with skill-a: fully specific.
        let analytics = build_analytics(vec![
            event("skill-a", "help with rust code"),
            event("skill-a", "rust debugging"),
            event("skill-b", "python testing"),
        ]);
        let affinity = affinity(&analytics, "rust").expect("rust affinity");
        assert_eq!(affinity.confidence.value(), 1.0);
        assert_eq!(affinity.associated_skills, vec!["skill-a".to_string()]);
    }

    #[test]
    fn prompt_affinity_confidence_is_the_share_of_keyword_uses_by_its_top_skill() {
        let analytics = build_analytics(vec![
            event("skill-a", "docker please"),
            event("skill-a", "docker please"),
            event("skill-b", "please"),
            event("skill-c", "please"),
        ]);
        assert_eq!(
            affinity(&analytics, "docker").unwrap().confidence.value(),
            1.0
        );
        assert_eq!(
            affinity(&analytics, "please").unwrap().confidence.value(),
            0.5
        );
    }

    #[test]
    fn prompt_affinities_drop_a_keyword_seen_once() {
        let analytics = build_analytics(vec![
            event("skill-a", "deadbeefcafe"),
            event("skill-a", "docker"),
            event("skill-a", "docker"),
        ]);
        assert!(affinity(&analytics, "deadbeefcafe").is_none());
        assert!(affinity(&analytics, "docker").is_some());
    }

    #[test]
    fn prompt_affinities_are_capped() {
        let mut events = Vec::new();
        for i in 0..MAX_PROMPT_AFFINITIES + 10 {
            let prompt = format!("keyword{i:05}");
            events.push(event("skill-a", &prompt));
            events.push(event("skill-a", &prompt));
        }
        let analytics = build_analytics(events);
        assert_eq!(analytics.prompt_affinities.len(), MAX_PROMPT_AFFINITIES);
    }

    #[test]
    fn build_analytics_keys_a_skill_by_name_whatever_form_the_event_names_it() {
        let analytics = build_analytics(vec![
            event("skill://skrills/codex/foo/SKILL.md", "x"),
            event("/home/u/.codex/skills/foo/SKILL.md", "x"),
            event("plugin:foo", "x"),
            event("foo", "x"),
        ]);
        assert_eq!(analytics.frequency.len(), 1, "{:?}", analytics.frequency);
        assert_eq!(analytics.frequency.get("foo"), Some(&4));
    }
}
