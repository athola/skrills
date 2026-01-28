use anyhow::{bail, Result};
use skrills_discovery::{discover_skills, extra_skill_roots};
use std::path::PathBuf;

use crate::cli::OutputFormat;
use crate::discovery::merge_extra_dirs;

use super::{ScoreBreakdown, SkillScoreResult};

/// Handle the skill-score command.
pub(crate) fn handle_skill_score_command(
    name: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
    below_threshold: Option<u8>,
) -> Result<()> {
    use skrills_validate::frontmatter::parse_frontmatter;

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    let skills_to_score: Vec<_> = if let Some(ref target_name) = name {
        skills
            .iter()
            .filter(|s| {
                s.name.eq_ignore_ascii_case(target_name)
                    || s.path.to_string_lossy().contains(target_name)
            })
            .collect()
    } else {
        skills.iter().collect()
    };

    if skills_to_score.is_empty() {
        if let Some(ref n) = name {
            bail!("Skill '{}' not found", n);
        } else {
            bail!("No skills found to score");
        }
    }

    let mut results: Vec<SkillScoreResult> = Vec::new();

    for skill in skills_to_score {
        let content = match std::fs::read_to_string(&skill.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let parsed = parse_frontmatter(&content);

        let mut suggestions = Vec::new();

        let frontmatter_score = match &parsed {
            Ok(p) if p.frontmatter.is_some() => {
                let fm = p.frontmatter.as_ref().unwrap();
                let mut score = 10u8;

                if fm.name.is_some() {
                    score += 5;
                } else {
                    suggestions.push("Add 'name' field to frontmatter".to_string());
                }

                if fm.description.is_some() {
                    score += 10;
                } else {
                    suggestions.push("Add 'description' field to frontmatter".to_string());
                }

                score
            }
            Ok(_) => {
                suggestions.push("Add YAML frontmatter with name and description".to_string());
                0
            }
            Err(_) => {
                suggestions.push("Fix frontmatter YAML syntax errors".to_string());
                0
            }
        };

        let validation_score = if parsed.is_ok() { 25u8 } else { 0u8 };

        let description_score = skill
            .description
            .as_ref()
            .map(|d| {
                let len = d.len();
                if len >= 100 {
                    25u8
                } else if len >= 50 {
                    20u8
                } else if len >= 20 {
                    15u8
                } else if len > 0 {
                    10u8
                } else {
                    0u8
                }
            })
            .unwrap_or(0);

        if description_score < 20 {
            suggestions.push("Improve description (aim for 100+ characters)".to_string());
        }

        let token_score = {
            let estimated_tokens = content.len() / 4;
            if estimated_tokens < 500 {
                25u8
            } else if estimated_tokens < 1000 {
                20u8
            } else if estimated_tokens < 2000 {
                15u8
            } else if estimated_tokens < 5000 {
                10u8
            } else {
                5u8
            }
        };

        if token_score < 15 {
            suggestions
                .push("Consider splitting into smaller skills for token efficiency".to_string());
        }

        let total_score = frontmatter_score + validation_score + description_score + token_score;

        if let Some(threshold) = below_threshold {
            if total_score >= threshold {
                continue;
            }
        }

        results.push(SkillScoreResult {
            name: skill.name.clone(),
            path: skill.path.clone(),
            total_score,
            breakdown: ScoreBreakdown {
                frontmatter_completeness: frontmatter_score,
                validation_score,
                description_quality: description_score,
                token_efficiency: token_score,
            },
            suggestions,
        });
    }

    results.sort_by(|a, b| a.total_score.cmp(&b.total_score));

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        println!("Skill Quality Scores");
        println!("════════════════════════════════════════════════════════════════════════");
        println!();

        for result in &results {
            let grade = match result.total_score {
                90..=100 => "A",
                80..=89 => "B",
                70..=79 => "C",
                60..=69 => "D",
                _ => "F",
            };

            println!("{} - {}/100 ({})", result.name, result.total_score, grade);
            println!(
                "  Frontmatter: {}/25  Validation: {}/25  Description: {}/25  Tokens: {}/25",
                result.breakdown.frontmatter_completeness,
                result.breakdown.validation_score,
                result.breakdown.description_quality,
                result.breakdown.token_efficiency,
            );

            if !result.suggestions.is_empty() {
                println!("  Suggestions:");
                for s in &result.suggestions {
                    println!("    - {}", s);
                }
            }
            println!();
        }

        if let Some(threshold) = below_threshold {
            println!("Showing skills with score < {}", threshold);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_test_utils::{env_guard, TestFixture};

    #[test]
    fn score_calculates_for_skills() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        fixture
            .create_skill_with_frontmatter(
                "well-documented",
                "A very detailed description that exceeds 100 characters to get maximum points for description quality in scoring",
                "# Well Documented Skill\n\nThis skill has good documentation.",
            )
            .expect("create skill");

        let result = handle_skill_score_command(
            Some("well-documented".to_string()),
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
            None,
        );

        assert!(result.is_ok(), "score command should succeed");
    }

    #[test]
    fn score_filters_by_skill_name() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        fixture
            .create_skill_with_frontmatter("target-skill", "Target", "Content")
            .expect("create target");
        fixture
            .create_skill_with_frontmatter("other-skill", "Other", "Content")
            .expect("create other");

        let result = handle_skill_score_command(
            Some("target-skill".to_string()),
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
            None,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn score_errors_when_skill_not_found() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        let result = handle_skill_score_command(
            Some("nonexistent-skill".to_string()),
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
            None,
        );

        assert!(result.is_err(), "should error for nonexistent skill");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "error should mention not found");
    }

    #[test]
    fn score_filters_by_threshold() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        fixture
            .create_skill("poor-skill", "no frontmatter here")
            .expect("create poor");

        fixture
            .create_skill_with_frontmatter(
                "good-skill",
                "A detailed description with more than 100 characters to maximize the description quality score component",
                "# Good Skill\n\nContent",
            )
            .expect("create good");

        let result = handle_skill_score_command(
            None,
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
            Some(50),
        );

        assert!(result.is_ok());
    }
}
