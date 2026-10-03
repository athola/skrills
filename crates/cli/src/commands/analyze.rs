use crate::cli::OutputFormat;
use anyhow::Result;
use skrills_discovery::discover_skills;

/// Handle the `analyze` command.
pub(crate) fn handle_analyze_command(
    skill_dirs: Vec<std::path::PathBuf>,
    format: OutputFormat,
    min_tokens: Option<usize>,
    suggestions: bool,
) -> Result<()> {
    use skrills_analyze::{analyze_skill, AnalysisSummary, Priority};

    let roots = crate::commands::skill_roots_for(&skill_dirs);
    let skills = discover_skills(&roots, None)?;

    if skills.is_empty() {
        if format.is_json() {
            println!("[]");
        } else {
            println!("No skills found to analyze.");
        }
        return Ok(());
    }

    let mut analyses = Vec::new();

    for meta in skills.iter() {
        let content = match std::fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(e) => {
                // Named on stderr rather than dropped, so a skill missing
                // from the report is not mistaken for one that was fine.
                tracing::warn!(path = %meta.path.display(), error = %e, "skipping unreadable skill");
                continue;
            }
        };

        let analysis = analyze_skill(&meta.path, &content);

        if let Some(min) = min_tokens {
            if analysis.tokens.total < min {
                continue;
            }
        }

        analyses.push(analysis);
    }

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&analyses)?);
    } else {
        let summary = AnalysisSummary::from_analyses(&analyses);
        println!(
            "Analyzed {} skills: {} total tokens",
            summary.total_skills, summary.total_tokens
        );
        println!(
            "Size distribution: {} small, {} medium, {} large, {} very-large",
            summary.by_category.small,
            summary.by_category.medium,
            summary.by_category.large,
            summary.by_category.very_large
        );
        println!("Average quality score: {:.0}%", summary.avg_quality * 100.0);

        if suggestions && summary.high_priority_count > 0 {
            println!(
                "\nHigh-priority suggestions ({}):",
                summary.high_priority_count
            );
            for analysis in &analyses {
                for suggestion in &analysis.suggestions {
                    if suggestion.priority == Priority::High {
                        println!("  {} - {}", analysis.name, suggestion.message);
                    }
                }
            }
        }
    }

    Ok(())
}
