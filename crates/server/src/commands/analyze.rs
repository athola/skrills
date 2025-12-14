use crate::discovery::merge_extra_dirs;
use anyhow::Result;
use skrills_discovery::{discover_skills, extra_skill_roots};

/// Handle the `analyze` command.
pub(crate) fn handle_analyze_command(
    skill_dirs: Vec<std::path::PathBuf>,
    format: String,
    min_tokens: Option<usize>,
    suggestions: bool,
) -> Result<()> {
    use skrills_analyze::{analyze_skill, AnalysisSummary, Priority};

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    if skills.is_empty() {
        println!("No skills found to analyze.");
        return Ok(());
    }

    let mut analyses = Vec::new();

    for meta in skills.iter() {
        let content = match std::fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let analysis = analyze_skill(&meta.path, &content);

        if let Some(min) = min_tokens {
            if analysis.tokens.total < min {
                continue;
            }
        }

        analyses.push(analysis);
    }

    if format == "json" {
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
