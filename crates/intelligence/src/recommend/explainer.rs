//! Generate human-readable explanations for recommendations.

use super::RecommendationSignal;

/// Generate a human-readable explanation from recommendation signals.
pub fn generate_explanation(signals: &[RecommendationSignal]) -> String {
    if signals.is_empty() {
        return "Related skill".to_string();
    }

    let mut parts = Vec::new();

    for signal in signals {
        let part = match signal {
            RecommendationSignal::Dependency => "Required dependency".to_string(),
            RecommendationSignal::Dependent => "Used by your skills".to_string(),
            RecommendationSignal::Sibling => "Related via shared dependencies".to_string(),
            RecommendationSignal::CoUsed { count } => {
                format!("Frequently used together ({} times)", count)
            }
            RecommendationSignal::ProjectMatch { matched } => {
                if matched.len() == 1 {
                    format!("Matches project technology: {}", matched[0])
                } else {
                    format!("Matches: {}", matched.join(", "))
                }
            }
            RecommendationSignal::RecentlyUsed { last_used } => format_recency(*last_used),
            RecommendationSignal::PromptMatch { keywords } => {
                if keywords.len() == 1 {
                    format!("Matches your query: \"{}\"", keywords[0])
                } else {
                    format!("Matches: {}", keywords.join(", "))
                }
            }
            RecommendationSignal::HighQuality { score } => {
                format!("High quality skill ({:.0}%)", score * 100.0)
            }
        };
        parts.push(part);
    }

    parts.join("; ")
}

/// Format recency as human-readable text.
fn format_recency(timestamp: u64) -> String {
    if timestamp == 0 {
        return "Previously used".to_string();
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if now == 0 || timestamp > now {
        return "Recently used".to_string();
    }

    let age_seconds = now - timestamp;
    let age_minutes = age_seconds / 60;
    let age_hours = age_minutes / 60;
    let age_days = age_hours / 24;

    if age_days > 30 {
        format!("Used {} days ago", age_days)
    } else if age_days > 0 {
        if age_days == 1 {
            "Used yesterday".to_string()
        } else {
            format!("Used {} days ago", age_days)
        }
    } else if age_hours > 0 {
        if age_hours == 1 {
            "Used an hour ago".to_string()
        } else {
            format!("Used {} hours ago", age_hours)
        }
    } else if age_minutes > 0 {
        format!("Used {} minutes ago", age_minutes)
    } else {
        "Just used".to_string()
    }
}

/// Generate a summary of multiple recommendations.
pub fn summarize_recommendations(count: usize, has_usage: bool, has_context: bool) -> String {
    let mut parts = Vec::new();

    parts.push(format!("Found {} recommendations", count));

    if has_usage {
        parts.push("including usage patterns".to_string());
    }

    if has_context {
        parts.push("matched to project context".to_string());
    }

    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_signals() {
        assert_eq!(generate_explanation(&[]), "Related skill");
    }

    #[test]
    fn test_dependency_explanation() {
        let signals = vec![RecommendationSignal::Dependency];
        let explanation = generate_explanation(&signals);
        assert_eq!(explanation, "Required dependency");
    }

    #[test]
    fn test_multiple_signals() {
        let signals = vec![
            RecommendationSignal::Dependency,
            RecommendationSignal::CoUsed { count: 5 },
        ];
        let explanation = generate_explanation(&signals);
        assert!(explanation.contains("Required dependency"));
        assert!(explanation.contains("Frequently used together (5 times)"));
    }

    #[test]
    fn test_project_match_single() {
        let signals = vec![RecommendationSignal::ProjectMatch {
            matched: vec!["Rust".to_string()],
        }];
        let explanation = generate_explanation(&signals);
        assert_eq!(explanation, "Matches project technology: Rust");
    }

    #[test]
    fn test_project_match_multiple() {
        let signals = vec![RecommendationSignal::ProjectMatch {
            matched: vec!["Rust".to_string(), "Tokio".to_string()],
        }];
        let explanation = generate_explanation(&signals);
        assert_eq!(explanation, "Matches: Rust, Tokio");
    }

    #[test]
    fn test_format_recency_zero() {
        assert_eq!(format_recency(0), "Previously used");
    }

    #[test]
    fn test_summarize() {
        assert_eq!(
            summarize_recommendations(5, true, true),
            "Found 5 recommendations, including usage patterns, matched to project context"
        );
        assert_eq!(
            summarize_recommendations(3, false, false),
            "Found 3 recommendations"
        );
    }
}
