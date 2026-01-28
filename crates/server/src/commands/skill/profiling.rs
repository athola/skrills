use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::cli::OutputFormat;

use super::{ProfileResult, SkillStats};

/// Handle the skill-profile command.
pub(crate) fn handle_skill_profile_command(
    name: Option<String>,
    period: u32,
    format: OutputFormat,
) -> Result<()> {
    let home = dirs::home_dir().with_context(|| "Could not determine home directory")?;
    let cache_path = home.join(".skrills/analytics_cache.json");

    if !cache_path.exists() {
        if format.is_json() {
            let result = ProfileResult {
                period_days: period,
                total_invocations: 0,
                unique_skills_used: 0,
                top_skills: vec![],
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("No analytics data found.");
            println!(
                "Run `skrills recommend-skills-smart --auto-persist` to build analytics cache."
            );
        }
        return Ok(());
    }

    let analytics_json =
        std::fs::read_to_string(&cache_path).with_context(|| "Failed to read analytics cache")?;

    let analytics: serde_json::Value =
        serde_json::from_str(&analytics_json).with_context(|| "Failed to parse analytics cache")?;

    let mut skill_counts: HashMap<String, u64> = HashMap::new();

    if let Some(usage) = analytics.get("skill_usage").and_then(|u| u.as_object()) {
        for (skill_name, count) in usage {
            if let Some(n) = count.as_u64() {
                skill_counts.insert(skill_name.clone(), n);
            }
        }
    }

    if let Some(ref target_name) = name {
        let count = skill_counts.get(target_name).copied().unwrap_or(0);
        let stats = SkillStats {
            name: target_name.clone(),
            invocations: count,
            last_used: None,
            avg_tokens: None,
            success_rate: None,
        };

        if format.is_json() {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        } else {
            println!("Profile for '{}':", target_name);
            println!("  Invocations ({}d): {}", period, count);
            if count == 0 {
                println!("  No usage data found for this skill.");
            }
        }
        return Ok(());
    }

    let total: u64 = skill_counts.values().sum();
    let mut sorted: Vec<_> = skill_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let top_skills: Vec<SkillStats> = sorted
        .into_iter()
        .take(10)
        .map(|(name, invocations)| SkillStats {
            name,
            invocations,
            last_used: None,
            avg_tokens: None,
            success_rate: None,
        })
        .collect();

    let result = ProfileResult {
        period_days: period,
        total_invocations: total,
        unique_skills_used: top_skills.len(),
        top_skills: top_skills.clone(),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Skill Usage Profile (last {} days)", period);
        println!("─────────────────────────────────────");
        println!("Total invocations: {}", total);
        println!("Unique skills used: {}", result.unique_skills_used);
        println!();
        println!("Top Skills:");
        for (i, stats) in top_skills.iter().enumerate() {
            println!(
                "  {}. {} ({} invocations)",
                i + 1,
                stats.name,
                stats.invocations
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{ProfileResult, SkillStats};
    use std::collections::HashMap;

    // GIVEN an empty analytics cache
    // WHEN building a ProfileResult
    // THEN all counts are zero
    #[test]
    fn empty_profile_result() {
        let result = ProfileResult {
            period_days: 30,
            total_invocations: 0,
            unique_skills_used: 0,
            top_skills: vec![],
        };
        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("\"total_invocations\": 0"));
        assert!(json.contains("\"unique_skills_used\": 0"));
        assert!(json.contains("\"top_skills\": []"));
    }

    // GIVEN a SkillStats with only required fields
    // WHEN optional fields are None
    // THEN serialization includes null values
    #[test]
    fn skill_stats_optional_fields_null() {
        let stats = SkillStats {
            name: "test-skill".to_string(),
            invocations: 42,
            last_used: None,
            avg_tokens: None,
            success_rate: None,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"last_used\":null"));
        assert!(json.contains("\"avg_tokens\":null"));
        assert!(json.contains("\"success_rate\":null"));
        assert!(json.contains("\"invocations\":42"));
    }

    // GIVEN skill usage counts from analytics JSON
    // WHEN parsing into a HashMap
    // THEN counts are correctly extracted
    #[test]
    fn parse_skill_usage_from_json() {
        let analytics_json = r#"{"skill_usage": {"commit": 10, "review": 5, "deploy": 3}}"#;
        let analytics: serde_json::Value = serde_json::from_str(analytics_json).unwrap();

        let mut skill_counts: HashMap<String, u64> = HashMap::new();
        if let Some(usage) = analytics.get("skill_usage").and_then(|u| u.as_object()) {
            for (name, count) in usage {
                if let Some(n) = count.as_u64() {
                    skill_counts.insert(name.clone(), n);
                }
            }
        }

        assert_eq!(skill_counts.len(), 3);
        assert_eq!(skill_counts["commit"], 10);
        assert_eq!(skill_counts["review"], 5);
        assert_eq!(skill_counts["deploy"], 3);
    }

    // GIVEN skill counts
    // WHEN sorting by invocations descending and taking top 10
    // THEN the result is in correct order and capped at 10
    #[test]
    fn top_skills_sorted_and_capped() {
        let mut counts: Vec<(String, u64)> = (0..15)
            .map(|i| (format!("skill-{}", i), i as u64))
            .collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        let top: Vec<SkillStats> = counts
            .into_iter()
            .take(10)
            .map(|(name, invocations)| SkillStats {
                name,
                invocations,
                last_used: None,
                avg_tokens: None,
                success_rate: None,
            })
            .collect();

        assert_eq!(top.len(), 10);
        assert_eq!(top[0].name, "skill-14");
        assert_eq!(top[0].invocations, 14);
        assert_eq!(top[9].name, "skill-5");
    }

    // GIVEN a target skill name lookup
    // WHEN the skill exists in counts
    // THEN return its count; otherwise 0
    #[test]
    fn lookup_specific_skill_count() {
        let mut skill_counts: HashMap<String, u64> = HashMap::new();
        skill_counts.insert("commit".to_string(), 25);

        assert_eq!(skill_counts.get("commit").copied().unwrap_or(0), 25);
        assert_eq!(skill_counts.get("nonexistent").copied().unwrap_or(0), 0);
    }

    // GIVEN analytics JSON without skill_usage key
    // WHEN parsing
    // THEN skill_counts remains empty
    #[test]
    fn missing_skill_usage_key_yields_empty() {
        let analytics_json = r#"{"other_data": 123}"#;
        let analytics: serde_json::Value = serde_json::from_str(analytics_json).unwrap();

        let mut skill_counts: HashMap<String, u64> = HashMap::new();
        if let Some(usage) = analytics.get("skill_usage").and_then(|u| u.as_object()) {
            for (name, count) in usage {
                if let Some(n) = count.as_u64() {
                    skill_counts.insert(name.clone(), n);
                }
            }
        }

        assert!(skill_counts.is_empty());
    }
}
