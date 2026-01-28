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
