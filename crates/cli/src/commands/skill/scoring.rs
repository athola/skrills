use anyhow::{bail, Result};
use skrills_discovery::{discover_skills, SkillMeta};
use std::path::PathBuf;

use crate::cli::OutputFormat;

use super::{ScoreBreakdown, SkillScoreResult};

/// The skills `skill-score` scores: every skill, or the one `name` names.
///
/// A name is resolved with [`super::find_skill`], so it matches a skill's
/// key, directory or frontmatter name exactly; a substring of the absolute
/// path (`test` in `/home/u/test-projects/`) no longer selects every skill
/// under that directory.
fn select_skills<'a>(skills: &'a [SkillMeta], name: Option<&str>) -> Result<Vec<&'a SkillMeta>> {
    match name {
        Some(target_name) => Ok(vec![super::find_skill(skills, target_name)?]),
        None if skills.is_empty() => bail!("No skills found to score"),
        None => Ok(skills.iter().collect()),
    }
}

/// Scores one skill out of 100, 25 per component.
fn score_skill(skill: &SkillMeta, content: &str) -> SkillScoreResult {
    use skrills_validate::frontmatter::parse_frontmatter;

    let parsed = parse_frontmatter(content);

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

    // Validated for every target, the strictest: a parse that finds no
    // frontmatter is `Ok`, so it says nothing about validity.
    let validation_score = if skrills_validate::validate_skill(
        &skill.path,
        content,
        skrills_validate::ValidationTarget::All,
    )
    .has_errors()
    {
        suggestions.push("Fix the errors `skrills validate --target all` reports".to_string());
        0u8
    } else {
        25u8
    };

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
        let estimated_tokens = skrills_analyze::estimate_tokens(content.len());
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
        suggestions.push("Consider splitting into smaller skills for token efficiency".to_string());
    }

    SkillScoreResult {
        name: skill.name.clone(),
        path: skill.path.clone(),
        total_score: frontmatter_score + validation_score + description_score + token_score,
        breakdown: ScoreBreakdown {
            frontmatter_completeness: frontmatter_score,
            validation_score,
            description_quality: description_score,
            token_efficiency: token_score,
        },
        suggestions,
    }
}

/// Handle the skill-score command.
pub(crate) fn handle_skill_score_command(
    name: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
    below_threshold: Option<u8>,
) -> Result<()> {
    let roots = crate::commands::skill_roots_for(&skill_dirs)?;
    let skills = discover_skills(&roots, None)?;
    let skills_to_score = select_skills(&skills, name.as_deref())?;

    let mut results: Vec<SkillScoreResult> = Vec::new();

    for skill in skills_to_score {
        let content = match std::fs::read_to_string(&skill.path) {
            Ok(c) => c,
            Err(e) => {
                // Named on stderr rather than dropped, so a skill missing
                // from the scores is not mistaken for one that was fine.
                tracing::warn!(path = %skill.path.display(), error = %e, "skipping unreadable skill");
                continue;
            }
        };

        let result = score_skill(skill, &content);

        if let Some(threshold) = below_threshold {
            if result.total_score >= threshold {
                continue;
            }
        }

        results.push(result);
    }

    results.sort_by_key(|a| a.total_score);

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

        result.expect("score command should succeed");
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

        result.expect("score filter by name should succeed");
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

        result.expect("score filter by threshold should succeed");
    }

    fn discovered_under(dir: &std::path::Path) -> Vec<skrills_discovery::SkillMeta> {
        discover_skills(
            &[skrills_discovery::SkillRoot {
                root: dir.to_path_buf(),
                source: skrills_discovery::SkillSource::Extra(0),
            }],
            None,
        )
        .unwrap()
    }

    fn write_skill(root: &std::path::Path, name: &str, content: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn score_name_does_not_match_a_substring_of_the_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("test-projects");
        write_skill(
            &root,
            "alpha",
            "---\nname: alpha\ndescription: A\n---\nBody\n",
        );
        write_skill(
            &root,
            "beta",
            "---\nname: beta\ndescription: B\n---\nBody\n",
        );
        let skills = discovered_under(&root);

        assert!(select_skills(&skills, Some("test")).is_err());
        let picked = select_skills(&skills, Some("alpha")).unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].name, "alpha/SKILL.md");
    }

    #[test]
    fn score_gives_no_validation_points_to_a_skill_without_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(tmp.path(), "bare", "no frontmatter here\n");
        let skills = discovered_under(tmp.path());

        let scored = score_skill(&skills[0], "no frontmatter here\n");

        assert_eq!(scored.breakdown.validation_score, 0);
        assert_eq!(scored.breakdown.frontmatter_completeness, 0);
    }

    #[test]
    fn score_gives_full_validation_points_to_a_valid_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let content = "---\nname: good\ndescription: A skill that passes validation\n---\n# Good\n";
        write_skill(tmp.path(), "good", content);
        let skills = discovered_under(tmp.path());

        let scored = score_skill(&skills[0], content);

        assert_eq!(scored.breakdown.validation_score, 25);
        assert_eq!(scored.breakdown.frontmatter_completeness, 25);
    }
}
