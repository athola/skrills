//! Skill scaffolding from built-in templates.
//!
//! Provides pre-configured templates for common skill patterns
//! (debugging, code review, testing, documentation, refactoring)
//! that generate valid frontmatter per target CLI.

use serde::{Deserialize, Serialize};

/// Target CLI for generated skill output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetCli {
    /// Claude Code — full YAML frontmatter.
    Claude,
    /// Cursor IDE — no YAML frontmatter (plain markdown).
    Cursor,
    /// Codex CLI — minimal frontmatter (name + description only).
    Codex,
}

impl std::fmt::Display for TargetCli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Claude => write!(f, "claude"),
            Self::Cursor => write!(f, "cursor"),
            Self::Codex => write!(f, "codex"),
        }
    }
}

impl std::str::FromStr for TargetCli {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "cursor" => Ok(Self::Cursor),
            "codex" => Ok(Self::Codex),
            other => Err(format!("Unknown target CLI: '{other}'. Expected: claude, cursor, codex")),
        }
    }
}

/// A built-in skill template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTemplate {
    /// Template identifier (e.g., "debugging").
    pub name: String,
    /// Human-readable summary of the template.
    pub description: String,
    /// Category for grouping (e.g., "development", "quality").
    pub category: String,
    /// Raw template content with `{{PLACEHOLDER}}` markers.
    template_content: String,
}

impl SkillTemplate {
    /// Returns the raw template content.
    #[must_use]
    pub fn template_content(&self) -> &str {
        &self.template_content
    }
}

// Embed template files at compile time.
const TEMPLATE_DEBUGGING: &str = include_str!("templates/debugging.md");
const TEMPLATE_CODE_REVIEW: &str = include_str!("templates/code-review.md");
const TEMPLATE_TESTING: &str = include_str!("templates/testing.md");
const TEMPLATE_DOCUMENTATION: &str = include_str!("templates/documentation.md");
const TEMPLATE_REFACTORING: &str = include_str!("templates/refactoring.md");

/// Returns all built-in skill templates.
#[must_use]
pub fn list_templates() -> Vec<SkillTemplate> {
    vec![
        SkillTemplate {
            name: "debugging".into(),
            description: "Debugging and troubleshooting skill for systematic issue diagnosis".into(),
            category: "development".into(),
            template_content: TEMPLATE_DEBUGGING.into(),
        },
        SkillTemplate {
            name: "code-review".into(),
            description: "Code review skill for constructive, actionable feedback".into(),
            category: "quality".into(),
            template_content: TEMPLATE_CODE_REVIEW.into(),
        },
        SkillTemplate {
            name: "testing".into(),
            description: "Test generation skill for thorough, maintainable test suites".into(),
            category: "quality".into(),
            template_content: TEMPLATE_TESTING.into(),
        },
        SkillTemplate {
            name: "documentation".into(),
            description: "Documentation skill for clear, accurate technical writing".into(),
            category: "development".into(),
            template_content: TEMPLATE_DOCUMENTATION.into(),
        },
        SkillTemplate {
            name: "refactoring".into(),
            description: "Refactoring skill for improving code structure safely".into(),
            category: "development".into(),
            template_content: TEMPLATE_REFACTORING.into(),
        },
    ]
}

/// Look up a template by name (case-insensitive).
#[must_use]
pub fn get_template(name: &str) -> Option<SkillTemplate> {
    let lower = name.to_lowercase();
    list_templates().into_iter().find(|t| t.name == lower)
}

/// Generate a skill from a template, customised for the given name and target CLI.
///
/// # Errors
///
/// Returns an error if the template name is not recognised.
pub fn generate_skill(
    template_name: &str,
    skill_name: &str,
    target_cli: TargetCli,
) -> Result<String, String> {
    let template = get_template(template_name).ok_or_else(|| {
        let available: Vec<String> = list_templates().iter().map(|t| t.name.clone()).collect();
        format!(
            "Unknown template: '{}'. Available templates: {}",
            template_name,
            available.join(", ")
        )
    })?;

    let title = to_title_case(skill_name);
    let description = format!("{} — generated from '{}' template", title, template.name);

    // Start from the raw template and substitute placeholders.
    let rendered = template
        .template_content
        .replace("{{SKILL_NAME}}", skill_name)
        .replace("{{SKILL_DESCRIPTION}}", &description)
        .replace("{{SKILL_TITLE}}", &title);

    // Adapt output per target CLI.
    Ok(adapt_for_cli(&rendered, target_cli))
}

/// Convert `kebab-case` or `snake_case` names to `Title Case`.
fn to_title_case(s: &str) -> String {
    s.split(|c: char| c == '-' || c == '_')
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let mut out = first.to_uppercase().to_string();
                    out.extend(chars);
                    out
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Adapt rendered template for a specific CLI target.
///
/// - **Claude**: full YAML frontmatter (returned as-is).
/// - **Cursor**: strips YAML frontmatter block entirely.
/// - **Codex**: keeps only `name` and `description` in frontmatter.
fn adapt_for_cli(content: &str, target: TargetCli) -> String {
    match target {
        TargetCli::Claude => content.to_string(),
        TargetCli::Cursor => strip_frontmatter(content),
        TargetCli::Codex => minimise_frontmatter(content),
    }
}

/// Remove the YAML frontmatter block (`---` ... `---`) entirely.
fn strip_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }

    let after_open = &trimmed[3..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);

    if let Some(end_pos) = after_open.find("\n---") {
        let rest = &after_open[end_pos + 4..];
        rest.trim_start_matches(['\r', '\n']).to_string()
    } else {
        content.to_string()
    }
}

/// Keep only `name` and `description` in frontmatter (Codex minimal).
fn minimise_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }

    let after_open = &trimmed[3..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);

    if let Some(end_pos) = after_open.find("\n---") {
        let yaml_block = &after_open[..end_pos];
        let rest = &after_open[end_pos + 4..];
        let rest = rest.trim_start_matches(['\r', '\n']);

        // Extract name and description lines from YAML.
        let mut name_line: Option<&str> = None;
        let mut desc_line: Option<&str> = None;
        for line in yaml_block.lines() {
            if line.starts_with("name:") {
                name_line = Some(line);
            } else if line.starts_with("description:") {
                desc_line = Some(line);
            }
        }

        let mut out = String::from("---\n");
        if let Some(n) = name_line {
            out.push_str(n);
            out.push('\n');
        }
        if let Some(d) = desc_line {
            out.push_str(d);
            out.push('\n');
        }
        out.push_str("---\n");
        out.push_str(rest);
        out
    } else {
        content.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- list_templates tests ----

    #[test]
    fn list_templates_returns_five_templates() {
        let templates = list_templates();
        assert_eq!(templates.len(), 5);
    }

    #[test]
    fn list_templates_names_are_unique() {
        let templates = list_templates();
        let mut names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn list_templates_all_have_description_and_category() {
        for t in list_templates() {
            assert!(!t.description.is_empty(), "template '{}' has empty description", t.name);
            assert!(!t.category.is_empty(), "template '{}' has empty category", t.name);
        }
    }

    #[test]
    fn list_templates_contains_expected_names() {
        let names: Vec<String> = list_templates().iter().map(|t| t.name.clone()).collect();
        assert!(names.contains(&"debugging".to_string()));
        assert!(names.contains(&"code-review".to_string()));
        assert!(names.contains(&"testing".to_string()));
        assert!(names.contains(&"documentation".to_string()));
        assert!(names.contains(&"refactoring".to_string()));
    }

    // ---- get_template tests ----

    #[test]
    fn get_template_by_exact_name() {
        let t = get_template("debugging");
        assert!(t.is_some());
        assert_eq!(t.unwrap().name, "debugging");
    }

    #[test]
    fn get_template_case_insensitive() {
        assert!(get_template("Debugging").is_some());
        assert!(get_template("CODE-REVIEW").is_some());
    }

    #[test]
    fn get_template_returns_none_for_unknown() {
        assert!(get_template("nonexistent").is_none());
    }

    // ---- generate_skill tests (Claude target) ----

    #[test]
    fn generate_skill_claude_has_frontmatter() {
        let output = generate_skill("debugging", "my-debugger", TargetCli::Claude).unwrap();
        assert!(output.starts_with("---\n"));
        assert!(output.contains("name: my-debugger"));
        assert!(output.contains("description:"));
    }

    #[test]
    fn generate_skill_claude_substitutes_title() {
        let output = generate_skill("debugging", "my-debugger", TargetCli::Claude).unwrap();
        assert!(output.contains("# My Debugger"));
    }

    // ---- generate_skill tests (Cursor target) ----

    #[test]
    fn generate_skill_cursor_strips_frontmatter() {
        let output = generate_skill("code-review", "pr-reviewer", TargetCli::Cursor).unwrap();
        assert!(!output.contains("---"));
        assert!(!output.contains("name:"));
        // But the body should still be present.
        assert!(output.contains("# Pr Reviewer"));
    }

    // ---- generate_skill tests (Codex target) ----

    #[test]
    fn generate_skill_codex_has_minimal_frontmatter() {
        let output = generate_skill("testing", "unit-tester", TargetCli::Codex).unwrap();
        assert!(output.starts_with("---\n"));
        assert!(output.contains("name: unit-tester"));
        assert!(output.contains("description:"));
        // Body should follow.
        assert!(output.contains("# Unit Tester"));
    }

    // ---- generate_skill error case ----

    #[test]
    fn generate_skill_unknown_template_returns_error() {
        let result = generate_skill("bogus", "name", TargetCli::Claude);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Unknown template"));
        assert!(err.contains("bogus"));
    }

    // ---- all templates generate valid output for each CLI ----

    #[test]
    fn all_templates_generate_valid_claude_output() {
        for t in list_templates() {
            let output = generate_skill(&t.name, "test-skill", TargetCli::Claude).unwrap();
            // Must have frontmatter with name and description.
            assert!(output.contains("name: test-skill"), "template '{}' missing name", t.name);
            assert!(output.contains("description:"), "template '{}' missing description", t.name);
            assert!(output.starts_with("---\n"), "template '{}' missing frontmatter start", t.name);
        }
    }

    #[test]
    fn all_templates_generate_valid_cursor_output() {
        for t in list_templates() {
            let output = generate_skill(&t.name, "test-skill", TargetCli::Cursor).unwrap();
            // Must NOT have frontmatter.
            assert!(!output.starts_with("---"), "template '{}' should not have frontmatter for cursor", t.name);
        }
    }

    #[test]
    fn all_templates_generate_valid_codex_output() {
        for t in list_templates() {
            let output = generate_skill(&t.name, "test-skill", TargetCli::Codex).unwrap();
            assert!(output.contains("name: test-skill"), "template '{}' missing name for codex", t.name);
            assert!(output.contains("description:"), "template '{}' missing description for codex", t.name);
        }
    }

    // ---- validate generated skills pass validation ----

    #[test]
    fn all_claude_templates_pass_validation() {
        use skrills_validate::{validate_skill, ValidationTarget};
        use std::path::Path;

        for t in list_templates() {
            let output = generate_skill(&t.name, "test-skill", TargetCli::Claude).unwrap();
            let result = validate_skill(Path::new("SKILL.md"), &output, ValidationTarget::Claude);
            assert!(
                result.claude_valid,
                "template '{}' failed Claude validation: {:?}",
                t.name, result.issues
            );
        }
    }

    #[test]
    fn all_codex_templates_pass_validation() {
        use skrills_validate::{validate_skill, ValidationTarget};
        use std::path::Path;

        for t in list_templates() {
            let output = generate_skill(&t.name, "test-skill", TargetCli::Codex).unwrap();
            let result = validate_skill(Path::new("SKILL.md"), &output, ValidationTarget::Codex);
            assert!(
                result.codex_valid,
                "template '{}' failed Codex validation: {:?}",
                t.name, result.issues
            );
        }
    }

    // ---- to_title_case tests ----

    #[test]
    fn title_case_from_kebab() {
        assert_eq!(to_title_case("my-skill"), "My Skill");
    }

    #[test]
    fn title_case_from_snake() {
        assert_eq!(to_title_case("my_skill"), "My Skill");
    }

    #[test]
    fn title_case_single_word() {
        assert_eq!(to_title_case("debugging"), "Debugging");
    }

    // ---- TargetCli parsing tests ----

    #[test]
    fn target_cli_from_str() {
        assert_eq!("claude".parse::<TargetCli>().unwrap(), TargetCli::Claude);
        assert_eq!("cursor".parse::<TargetCli>().unwrap(), TargetCli::Cursor);
        assert_eq!("codex".parse::<TargetCli>().unwrap(), TargetCli::Codex);
        assert_eq!("CLAUDE".parse::<TargetCli>().unwrap(), TargetCli::Claude);
    }

    #[test]
    fn target_cli_from_str_invalid() {
        let err = "invalid".parse::<TargetCli>().unwrap_err();
        assert!(err.contains("Unknown target CLI"));
    }

    #[test]
    fn target_cli_display() {
        assert_eq!(TargetCli::Claude.to_string(), "claude");
        assert_eq!(TargetCli::Cursor.to_string(), "cursor");
        assert_eq!(TargetCli::Codex.to_string(), "codex");
    }

    // ---- strip_frontmatter / minimise_frontmatter unit tests ----

    #[test]
    fn strip_frontmatter_no_frontmatter() {
        let input = "# Hello\nBody.";
        assert_eq!(strip_frontmatter(input), input);
    }

    #[test]
    fn strip_frontmatter_with_frontmatter() {
        let input = "---\nname: x\n---\n# Hello\nBody.";
        let result = strip_frontmatter(input);
        assert_eq!(result, "# Hello\nBody.");
    }

    #[test]
    fn minimise_frontmatter_keeps_name_and_description() {
        let input = "---\nname: x\ndescription: y\nversion: 1.0\n---\n# Hello";
        let result = minimise_frontmatter(input);
        assert!(result.contains("name: x"));
        assert!(result.contains("description: y"));
        assert!(!result.contains("version:"));
    }
}
