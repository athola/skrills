use anyhow::{Context, Result};
use skrills_intelligence::{load_analytics, UsageAnalytics};
use std::path::Path;

use crate::cli::OutputFormat;

use super::{ProfileResult, SkillStats};

/// Loads `~/.skrills/analytics_cache.json`, the cache that
/// `recommend-skills-smart --auto-persist` and `export-analytics` write.
///
/// The commands used to read a `skill_usage` key that this cache never has,
/// so every report said 0 invocations. It is decoded as the
/// `UsageAnalytics` it was saved from instead.
pub(super) fn load_usage_analytics() -> Result<Option<UsageAnalytics>> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    load_analytics(&home.join(".skrills/analytics_cache.json"))
        .context("Failed to read the analytics cache")
}

/// Seconds since the Unix epoch, or 0 on a clock set before it.
pub(super) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Invocation counts per skill, most used first, for the skills last used
/// within `period_days` of `now`.
///
/// The cache keeps one all-time count and one last-used time per skill, not a
/// dated log, so the period selects skills by their last use and the counts
/// stay totals over the analysed history. A `period_days` of 0, or a clock
/// that reads 0, applies no period.
pub(super) fn usage_counts(
    analytics: &UsageAnalytics,
    period_days: u32,
    now: u64,
) -> Vec<(String, u64)> {
    let cutoff =
        (period_days > 0 && now > 0).then(|| now.saturating_sub(u64::from(period_days) * 86_400));
    let mut counts: Vec<(String, u64)> = analytics
        .frequency
        .iter()
        .filter(|(skill, _)| match cutoff {
            Some(cutoff) => analytics
                .recency
                .get(*skill)
                .is_some_and(|&last| last >= cutoff),
            None => true,
        })
        .map(|(skill, n)| (skill.clone(), *n))
        .collect();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

/// Whether the analytics key `key` (a skill path or name) names `name`:
/// equal, or naming the same skill directory (`.../review/SKILL.md`).
fn key_names_skill(key: &str, name: &str) -> bool {
    if key == name {
        return true;
    }
    let path = Path::new(key);
    let dir_name = if path.file_name().is_some_and(|f| f == "SKILL.md") {
        path.parent().and_then(|p| p.file_name())
    } else {
        path.file_stem()
    };
    dir_name.is_some_and(|d| d == name)
}

/// Handle the skill-profile command.
pub(crate) fn handle_skill_profile_command(
    name: Option<String>,
    period: u32,
    format: OutputFormat,
) -> Result<()> {
    let Some(analytics) = load_usage_analytics()? else {
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
    };

    let counts = usage_counts(&analytics, period, now_secs());

    if let Some(ref target_name) = name {
        let count: u64 = counts
            .iter()
            .filter(|(key, _)| key_names_skill(key, target_name))
            .map(|(_, n)| n)
            .sum();
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

    let result = profile(&counts, period);

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Skill Usage Profile (last {} days)", period);
        println!("─────────────────────────────────────");
        println!("Total invocations: {}", result.total_invocations);
        println!("Unique skills used: {}", result.unique_skills_used);
        println!();
        println!("Top Skills:");
        for (i, stats) in result.top_skills.iter().enumerate() {
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

/// The overall profile: totals over every skill used in the period, and the
/// ten most used.
fn profile(counts: &[(String, u64)], period: u32) -> ProfileResult {
    ProfileResult {
        period_days: period,
        total_invocations: counts.iter().map(|(_, n)| n).sum(),
        // Every skill used in the period, not the length of the top-ten list.
        unique_skills_used: counts.len(),
        top_skills: counts
            .iter()
            .take(10)
            .map(|(name, invocations)| SkillStats {
                name: name.clone(),
                invocations: *invocations,
                last_used: None,
                avg_tokens: None,
                success_rate: None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;
    const NOW: u64 = 1_000 * DAY;

    fn analytics(rows: &[(&str, u64, u64)]) -> UsageAnalytics {
        let mut a = UsageAnalytics::default();
        for (skill, count, last_used) in rows {
            a.frequency.insert(skill.to_string(), *count);
            a.recency.insert(skill.to_string(), *last_used);
        }
        a
    }

    /// SB-33: the counts come from a cache written by `save_analytics`, the
    /// way `recommend-skills-smart --auto-persist` writes it.
    #[test]
    fn counts_are_read_from_a_saved_analytics_cache() {
        let _g = skrills_test_utils::env_guard();
        let home = tempfile::tempdir().unwrap();
        let _h = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        let saved = analytics(&[("/s/review/SKILL.md", 7, now_secs())]);
        skrills_intelligence::save_analytics(
            &saved,
            &home.path().join(".skrills/analytics_cache.json"),
        )
        .unwrap();

        let loaded = load_usage_analytics().unwrap().expect("cache present");
        let counts = usage_counts(&loaded, 30, now_secs());

        assert_eq!(counts, vec![("/s/review/SKILL.md".to_string(), 7)]);
    }

    #[test]
    fn the_period_selects_skills_by_last_use() {
        let a = analytics(&[("recent", 3, NOW - DAY), ("stale", 9, NOW - 60 * DAY)]);

        assert_eq!(usage_counts(&a, 30, NOW), vec![("recent".to_string(), 3)]);
        assert_eq!(usage_counts(&a, 0, NOW).len(), 2, "0 applies no period");
        assert_eq!(
            usage_counts(&a, 30, 0).len(),
            2,
            "a zero clock applies none"
        );
    }

    /// SB-33: `unique_skills_used` was the length of the top-ten list.
    #[test]
    fn unique_skills_counts_beyond_the_top_ten() {
        let rows: Vec<(String, u64, u64)> = (0..12)
            .map(|i| (format!("skill-{i:02}"), 12 - i, NOW))
            .collect();
        let a = analytics(
            &rows
                .iter()
                .map(|(s, c, t)| (s.as_str(), *c, *t))
                .collect::<Vec<_>>(),
        );

        let result = profile(&usage_counts(&a, 30, NOW), 30);

        assert_eq!(result.unique_skills_used, 12);
        assert_eq!(result.top_skills.len(), 10);
        assert_eq!(result.top_skills[0].name, "skill-00");
        assert_eq!(result.total_invocations, (1..=12).sum::<u64>());
    }

    #[test]
    fn a_skill_is_found_by_its_directory_name() {
        assert!(key_names_skill(
            "/home/u/.claude/skills/review/SKILL.md",
            "review"
        ));
        assert!(key_names_skill("review", "review"));
        assert!(key_names_skill("/x/review.md", "review"));
        assert!(!key_names_skill("/x/reviewer/SKILL.md", "review"));
    }
}
