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

#[cfg(test)]
mod tests {
    use super::super::{UsageReportResult, UsageStats};
    use std::collections::HashMap;

    // GIVEN no analytics data
    // WHEN building empty UsageReportResult
    // THEN all counts are zero and skills list is empty
    #[test]
    fn empty_usage_report_result() {
        let result = UsageReportResult {
            period_days: 30,
            generated_at: "1706400000".to_string(),
            total_invocations: 0,
            unique_skills: 0,
            skills: vec![],
        };
        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("\"total_invocations\": 0"));
        assert!(json.contains("\"unique_skills\": 0"));
        assert!(json.contains("\"skills\": []"));
    }

    // GIVEN skill counts
    // WHEN computing percentages
    // THEN each percentage is (invocations / total) * 100
    #[test]
    fn percentage_calculation() {
        let total: u64 = 100;
        let invocations: u64 = 25;
        let percentage = (invocations as f64 / total as f64) * 100.0;
        assert!((percentage - 25.0).abs() < f64::EPSILON);
    }

    // GIVEN zero total invocations
    // WHEN computing percentage
    // THEN percentage is 0.0
    #[test]
    fn percentage_with_zero_total() {
        let total: u64 = 0;
        let invocations: u64 = 0;
        let percentage = if total > 0 {
            (invocations as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        assert!((percentage - 0.0).abs() < f64::EPSILON);
    }

    // GIVEN analytics JSON with skill_usage
    // WHEN parsing and sorting by invocations
    // THEN skills are in descending order
    #[test]
    fn skill_usage_sorted_descending() {
        let mut skill_counts: HashMap<String, u64> = HashMap::new();
        skill_counts.insert("commit".to_string(), 50);
        skill_counts.insert("review".to_string(), 30);
        skill_counts.insert("deploy".to_string(), 20);

        let total: u64 = skill_counts.values().sum();
        let mut sorted: Vec<_> = skill_counts.into_iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(&a.1));

        let skills: Vec<UsageStats> = sorted
            .into_iter()
            .map(|(name, invocations)| {
                let percentage = (invocations as f64 / total as f64) * 100.0;
                UsageStats {
                    skill_name: name,
                    invocations,
                    percentage,
                }
            })
            .collect();

        assert_eq!(skills.len(), 3);
        assert_eq!(skills[0].skill_name, "commit");
        assert_eq!(skills[0].invocations, 50);
        assert!((skills[0].percentage - 50.0).abs() < f64::EPSILON);
        assert_eq!(skills[2].skill_name, "deploy");
    }

    // GIVEN UsageStats struct
    // WHEN serialized to JSON
    // THEN all fields are present
    #[test]
    fn usage_stats_serialization() {
        let stats = UsageStats {
            skill_name: "test-skill".to_string(),
            invocations: 42,
            percentage: 21.5,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"skill_name\":\"test-skill\""));
        assert!(json.contains("\"invocations\":42"));
        assert!(json.contains("21.5"));
    }

    // GIVEN a UsageReportResult with skills
    // WHEN checking unique_skills
    // THEN it matches the skills vector length
    #[test]
    fn unique_skills_matches_vector_length() {
        let skills = vec![
            UsageStats {
                skill_name: "a".to_string(),
                invocations: 10,
                percentage: 50.0,
            },
            UsageStats {
                skill_name: "b".to_string(),
                invocations: 10,
                percentage: 50.0,
            },
        ];
        let result = UsageReportResult {
            period_days: 7,
            generated_at: "now".to_string(),
            total_invocations: 20,
            unique_skills: skills.len(),
            skills,
        };
        assert_eq!(result.unique_skills, 2);
    }

    // GIVEN analytics JSON without skill_usage key
    // WHEN parsing
    // THEN skill_counts is empty
    #[test]
    fn missing_skill_usage_yields_empty() {
        let analytics_json = r#"{"other": "data"}"#;
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
