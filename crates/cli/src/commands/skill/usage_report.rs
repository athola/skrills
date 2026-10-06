use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::cli::OutputFormat;

use super::profiling::{load_usage_analytics, now_secs, usage_counts};
use super::{UsageReportResult, UsageStats};

/// Handle the skill-usage-report command.
///
/// `--skill-dir` (and `SKRILLS_SKILL_DIR`) narrow the report to skills whose
/// recorded path lies under one of those directories.
pub(crate) fn handle_skill_usage_report_command(
    period: u32,
    format: OutputFormat,
    output: Option<PathBuf>,
    skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    let generated_at = now_secs().to_string();
    let dirs = skrills_server::discovery::merge_extra_dirs(&skill_dirs);

    let report_text = match load_usage_analytics()? {
        None => {
            let empty = UsageReportResult {
                period_days: period,
                generated_at: generated_at.clone(),
                total_invocations: 0,
                unique_skills: 0,
                skills: vec![],
            };
            if format.is_json() {
                serde_json::to_string_pretty(&empty)?
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
            }
        }
        Some(analytics) => {
            let counts: Vec<(String, u64)> = usage_counts(&analytics, period, now_secs())
                .into_iter()
                .filter(|(key, _)| under_any(key, &dirs))
                .collect();
            let result = usage_report(&counts, period, generated_at);
            if format.is_json() {
                serde_json::to_string_pretty(&result)?
            } else {
                render_text(&result)
            }
        }
    };

    if let Some(ref out_path) = output {
        std::fs::write(out_path, &report_text)?;
        // stdout stays free for a report; the confirmation is a notice.
        eprintln!("Report written to: {}", out_path.display());
    } else {
        println!("{}", report_text);
    }

    Ok(())
}

/// Whether `key` is a path under one of `dirs`; true when `dirs` is empty.
fn under_any(key: &str, dirs: &[PathBuf]) -> bool {
    dirs.is_empty() || dirs.iter().any(|d| Path::new(key).starts_with(d))
}

fn usage_report(counts: &[(String, u64)], period: u32, generated_at: String) -> UsageReportResult {
    let total: u64 = counts.iter().map(|(_, n)| n).sum();
    let skills: Vec<UsageStats> = counts
        .iter()
        .map(|(name, invocations)| UsageStats {
            skill_name: name.clone(),
            invocations: *invocations,
            percentage: if total > 0 {
                (*invocations as f64 / total as f64) * 100.0
            } else {
                0.0
            },
        })
        .collect();
    UsageReportResult {
        period_days: period,
        generated_at,
        total_invocations: total,
        unique_skills: skills.len(),
        skills,
    }
}

fn render_text(result: &UsageReportResult) -> String {
    let mut text = String::new();
    text.push_str("Skill Usage Report\n");
    text.push_str("═══════════════════════════════════════════════════════════\n\n");
    text.push_str(&format!("Period: {} days\n", result.period_days));
    text.push_str(&format!("Generated: {}\n", result.generated_at));
    text.push_str(&format!(
        "Total Invocations: {}\n",
        result.total_invocations
    ));
    text.push_str(&format!("Unique Skills: {}\n\n", result.unique_skills));
    text.push_str("Usage by Skill:\n");
    text.push_str("───────────────────────────────────────────────────────────\n");
    for stats in &result.skills {
        text.push_str(&format!(
            "  {:40} {:>6} ({:>5.1}%)\n",
            stats.skill_name, stats.invocations, stats.percentage
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_percentages_and_order() {
        let counts = vec![
            ("commit".to_string(), 50),
            ("review".to_string(), 30),
            ("deploy".to_string(), 20),
        ];

        let result = usage_report(&counts, 30, "t".into());

        assert_eq!(result.total_invocations, 100);
        assert_eq!(result.unique_skills, 3);
        assert_eq!(result.skills[0].skill_name, "commit");
        assert!((result.skills[0].percentage - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn an_empty_report_has_no_division_by_zero() {
        let result = usage_report(&[], 7, "t".into());
        assert_eq!(result.total_invocations, 0);
        assert!(result.skills.is_empty());
    }

    /// SB-42: `--skill-dir` was accepted and ignored.
    #[test]
    fn skill_dirs_narrow_the_report() {
        let dirs = vec![PathBuf::from("/home/u/.claude/skills")];
        assert!(under_any("/home/u/.claude/skills/review/SKILL.md", &dirs));
        assert!(!under_any("/home/u/.codex/skills/review/SKILL.md", &dirs));
        assert!(under_any("anything", &[]));
    }
}
