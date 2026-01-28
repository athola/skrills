use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::cli::OutputFormat;

use super::{UsageReportResult, UsageStats};

/// Handle the skill-usage-report command.
pub(crate) fn handle_skill_usage_report_command(
    period: u32,
    format: OutputFormat,
    output: Option<PathBuf>,
    _skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    let home = dirs::home_dir().with_context(|| "Could not determine home directory")?;
    let cache_path = home.join(".skrills/analytics_cache.json");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let generated_at = format!("{}", now);

    if !cache_path.exists() {
        let empty_result = UsageReportResult {
            period_days: period,
            generated_at: generated_at.clone(),
            total_invocations: 0,
            unique_skills: 0,
            skills: vec![],
        };

        let report_text = if format.is_json() {
            serde_json::to_string_pretty(&empty_result)?
        } else {
            format!(
                "Skill Usage Report\n\
                 ═══════════════════\n\
                 Period: {} days\n\
                 Generated: {}\n\n\
                 No usage data available.\n\
                 Run `skrills recommend-skills-smart --auto-persist` to build analytics.",
                period, generated_at
            )
        };

        if let Some(ref out_path) = output {
            std::fs::write(out_path, &report_text)?;
            println!("Report written to: {}", out_path.display());
        } else {
            println!("{}", report_text);
        }

        return Ok(());
    }

    let analytics_json = std::fs::read_to_string(&cache_path)?;
    let analytics: serde_json::Value = serde_json::from_str(&analytics_json)?;

    let mut skill_counts: HashMap<String, u64> = HashMap::new();
    if let Some(usage) = analytics.get("skill_usage").and_then(|u| u.as_object()) {
        for (name, count) in usage {
            if let Some(n) = count.as_u64() {
                skill_counts.insert(name.clone(), n);
            }
        }
    }

    let total: u64 = skill_counts.values().sum();
    let mut sorted: Vec<_> = skill_counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let skills: Vec<UsageStats> = sorted
        .into_iter()
        .map(|(name, invocations)| {
            let percentage = if total > 0 {
                (invocations as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            UsageStats {
                skill_name: name,
                invocations,
                percentage,
            }
        })
        .collect();

    let result = UsageReportResult {
        period_days: period,
        generated_at: generated_at.clone(),
        total_invocations: total,
        unique_skills: skills.len(),
        skills: skills.clone(),
    };

    let report_text = if format.is_json() {
        serde_json::to_string_pretty(&result)?
    } else {
        let mut text = String::new();
        text.push_str("Skill Usage Report\n");
        text.push_str("═══════════════════════════════════════════════════════════\n\n");
        text.push_str(&format!("Period: {} days\n", period));
        text.push_str(&format!("Generated: {}\n", generated_at));
        text.push_str(&format!("Total Invocations: {}\n", total));
        text.push_str(&format!("Unique Skills: {}\n\n", result.unique_skills));
        text.push_str("Usage by Skill:\n");
        text.push_str("───────────────────────────────────────────────────────────\n");

        for stats in &skills {
            text.push_str(&format!(
                "  {:40} {:>6} ({:>5.1}%)\n",
                stats.skill_name, stats.invocations, stats.percentage
            ));
        }

        text
    };

    if let Some(ref out_path) = output {
        std::fs::write(out_path, &report_text)?;
        println!("Report written to: {}", out_path.display());
    } else {
        println!("{}", report_text);
    }

    Ok(())
}
