//! Built-in skill templates for `create-skill --template`.
//!
//! Templates are compiled into the binary so scaffolding is offline and
//! deterministic: no GitHub search and no LLM call, unlike `--method`. They
//! live in the CLI crate because rendering needs both `skrills_sync::Platform`
//! (the target names) and `skrills_validate` (to prove the output loads),
//! and `skrills-intelligence` depends on neither.

use crate::cli::{OutputFormat, Platform};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use skrills_validate::{generate_frontmatter, validate_skill, Severity, ValidationTarget};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// The `--template` value that prints the catalog instead of creating a skill.
pub(crate) const LIST_KEYWORD: &str = "list";

/// Longest `name` Codex and Copilot accept; checked here so the error names the flag.
const MAX_NAME_CHARS: usize = 100;

/// A built-in skill: the frontmatter description and the Markdown body.
pub(crate) struct SkillTemplate {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) body: &'static str,
}

pub(crate) const TEMPLATES: [SkillTemplate; 5] = [
    SkillTemplate {
        name: "debugging",
        description: "Use when a test fails or behavior is wrong, to reproduce the failure and find the cause before fixing it",
        body: include_str!("../../templates/skills/debugging.md"),
    },
    SkillTemplate {
        name: "code-review",
        description: "Use when reviewing a diff or pull request, to check correctness, tests and compatibility and write actionable comments",
        body: include_str!("../../templates/skills/code-review.md"),
    },
    SkillTemplate {
        name: "testing",
        description: "Use when adding or changing behavior, to write the failing test first and cover boundaries and error paths",
        body: include_str!("../../templates/skills/testing.md"),
    },
    SkillTemplate {
        name: "documentation",
        description: "Use when writing or updating docs, READMEs or comments, to lead with the answer and verify every example",
        body: include_str!("../../templates/skills/documentation.md"),
    },
    SkillTemplate {
        name: "refactoring",
        description: "Use when restructuring code, to make small behavior-preserving steps with passing tests between them",
        body: include_str!("../../templates/skills/refactoring.md"),
    },
];

/// Everything `create_from_template` needs; `description` falls back to the
/// template's own when `None`.
pub(crate) struct TemplateRequest {
    pub(crate) template: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) target: Platform,
    pub(crate) target_dir: Option<PathBuf>,
    pub(crate) dry_run: bool,
}

/// What `create_from_template` produced. `written` is false for a dry run.
#[derive(Debug)]
pub(crate) struct TemplateOutcome {
    pub(crate) content: String,
    pub(crate) path: PathBuf,
    pub(crate) written: bool,
}

/// One line per template, for `--template list`.
pub(crate) fn list_templates() -> String {
    let width = TEMPLATES.iter().map(|t| t.name.len()).max().unwrap_or(0);
    TEMPLATES
        .iter()
        .map(|t| format!("{:<width$}  {}", t.name, t.description))
        .collect::<Vec<_>>()
        .join("\n")
}

fn find_template(name: &str) -> Result<&'static SkillTemplate> {
    TEMPLATES.iter().find(|t| t.name == name).ok_or_else(|| {
        let valid: Vec<&str> = TEMPLATES.iter().map(|t| t.name).collect();
        anyhow!(
            "Unknown template '{name}'. Valid templates: {}. Use `--template {LIST_KEYWORD}` to see descriptions.",
            valid.join(", ")
        )
    })
}

/// Skill roots each CLI loads user skills from; the same paths discovery scans.
pub(crate) fn default_target_dir(home: &Path, target: Platform) -> PathBuf {
    match target {
        Platform::Claude => home.join(".claude/skills"),
        // Codex loads user skills from ~/.agents/skills.
        Platform::Codex => home.join(".agents/skills"),
        Platform::Copilot => home.join(".copilot/skills"),
        Platform::Cursor => home.join(".cursor/skills"),
    }
}

/// The validator to prove a skill against. Cursor has no validator of its own
/// and reads the same `name`/`description` fields, so it gets the strictest
/// available check.
fn validation_target(target: Platform) -> ValidationTarget {
    match target {
        Platform::Claude => ValidationTarget::Claude,
        Platform::Codex => ValidationTarget::Codex,
        Platform::Copilot => ValidationTarget::Copilot,
        Platform::Cursor => ValidationTarget::All,
    }
}

/// Reject names that would write outside `<target-dir>/<name>/`.
fn check_skill_name(name: &str) -> Result<()> {
    let mut components = Path::new(name).components();
    let simple = matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
        && !name.starts_with('.')
        && !name.contains(['\n', '\r']);
    if !simple {
        bail!("Invalid name '{name}': use a simple directory name without path separators, a leading dot or newlines");
    }
    if name.chars().count() > MAX_NAME_CHARS {
        bail!("Invalid name: longer than {MAX_NAME_CHARS} characters");
    }
    Ok(())
}

/// Render a template to `SKILL.md` text and prove it passes the target's
/// validator, so a skill that would be rejected is never written.
pub(crate) fn render_skill(
    template: &str,
    name: &str,
    description: Option<&str>,
    target: Platform,
) -> Result<String> {
    let template = find_template(template)?;
    check_skill_name(name)?;
    let content = format!(
        "{}\n{}",
        generate_frontmatter(name, description.unwrap_or(template.description)),
        template.body
    );

    let verdict = validate_skill(
        &Path::new(name).join("SKILL.md"),
        &content,
        validation_target(target),
    );
    let errors: Vec<&str> = verdict
        .issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .map(|issue| issue.message.as_str())
        .collect();
    if !errors.is_empty() {
        bail!(
            "The rendered skill is invalid for {target}: {}",
            errors.join("; ")
        );
    }
    Ok(content)
}

/// Render a template and, unless `dry_run`, write `<dir>/<name>/SKILL.md`.
///
/// Like the other `create-skill` methods this refuses to replace an existing
/// file, so a hand-edited skill is never lost to a re-run.
pub(crate) fn create_from_template(request: &TemplateRequest) -> Result<TemplateOutcome> {
    let content = render_skill(
        &request.template,
        &request.name,
        request.description.as_deref(),
        request.target,
    )?;
    let root = match &request.target_dir {
        Some(dir) => dir.clone(),
        None => default_target_dir(&skrills_state::home_dir()?, request.target),
    };
    let skill_dir = root.join(&request.name);
    let path = skill_dir.join("SKILL.md");
    if request.dry_run {
        return Ok(TemplateOutcome {
            content,
            path,
            written: false,
        });
    }

    std::fs::create_dir_all(&skill_dir)
        .with_context(|| format!("Failed to create directory {}", skill_dir.display()))?;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => bail!(
            "{} already exists; remove it first to regenerate the skill",
            path.display()
        ),
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("Failed to write skill file {}", path.display())))
        }
    };
    file.write_all(content.as_bytes())
        .with_context(|| format!("Failed to write skill file {}", path.display()))?;
    Ok(TemplateOutcome {
        content,
        path,
        written: true,
    })
}

/// Handle `create-skill --template <NAME>`: list the catalog for `list`,
/// otherwise create (or preview) a skill from the named template.
pub(crate) fn handle_create_skill_template_command(
    template: String,
    name: Option<String>,
    description: Option<String>,
    target: Platform,
    target_dir: Option<PathBuf>,
    dry_run: bool,
    format: OutputFormat,
) -> Result<()> {
    if template == LIST_KEYWORD {
        if format.is_json() {
            let catalog: Vec<_> = TEMPLATES
                .iter()
                .map(|t| json!({ "name": t.name, "description": t.description }))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "templates": catalog }))?
            );
        } else {
            println!("{}", list_templates());
        }
        return Ok(());
    }

    let name =
        name.ok_or_else(|| anyhow!("A skill NAME is required with --template {template}"))?;
    let outcome = create_from_template(&TemplateRequest {
        template: template.clone(),
        name: name.clone(),
        description,
        target,
        target_dir,
        dry_run,
    })?;

    if format.is_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "success": true,
                "template": template,
                "target": target.as_str(),
                "name": name,
                "dry_run": dry_run,
                "written_path": outcome.written.then(|| outcome.path.display().to_string()),
                "content": outcome.content,
            }))?
        );
    } else if outcome.written {
        println!("Written to {}", outcome.path.display());
    } else {
        println!(
            "(dry run) would write {}\n\n{}",
            outcome.path.display(),
            outcome.content
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_NAMES: [&str; 5] = [
        "debugging",
        "code-review",
        "testing",
        "documentation",
        "refactoring",
    ];

    fn validator_target(target: Platform) -> ValidationTarget {
        match target {
            Platform::Claude => ValidationTarget::Claude,
            Platform::Codex => ValidationTarget::Codex,
            Platform::Copilot => ValidationTarget::Copilot,
            // No Cursor validator exists; `All` is the strictest check available.
            Platform::Cursor => ValidationTarget::All,
        }
    }

    fn request(template: &str, target: Platform, dir: &Path) -> TemplateRequest {
        TemplateRequest {
            template: template.to_string(),
            name: "my-skill".to_string(),
            description: None,
            target,
            target_dir: Some(dir.to_path_buf()),
            dry_run: false,
        }
    }

    #[test]
    fn ships_the_five_required_templates() {
        let names: Vec<&str> = TEMPLATES.iter().map(|t| t.name).collect();
        for expected in EXPECTED_NAMES {
            assert!(names.contains(&expected), "missing {expected}: {names:?}");
        }
    }

    #[test]
    fn every_template_renders_valid_for_every_target() {
        for template in TEMPLATES {
            for target in Platform::ALL {
                let content = render_skill(template.name, "my-skill", None, target)
                    .unwrap_or_else(|e| panic!("{} for {target}: {e:#}", template.name));
                let verdict = validate_skill(
                    Path::new("my-skill/SKILL.md"),
                    &content,
                    validator_target(target),
                );
                let errors: Vec<_> = verdict
                    .issues
                    .iter()
                    .filter(|i| i.severity == skrills_validate::Severity::Error)
                    .collect();
                assert!(
                    errors.is_empty(),
                    "{} for {target} has errors: {errors:?}",
                    template.name
                );
                assert!(content.starts_with("---\nname: my-skill\n"), "{content}");
            }
        }
    }

    #[test]
    fn template_bodies_avoid_marketing_words() {
        for template in TEMPLATES {
            let body = template.body.to_lowercase();
            for banned in ["robust", "seamless", "comprehensive", "leverage"] {
                assert!(!body.contains(banned), "{} uses {banned}", template.name);
            }
        }
    }

    #[test]
    fn explicit_description_replaces_the_template_default() {
        let content = render_skill("testing", "my-skill", Some("Custom words"), Platform::Codex)
            .expect("render");
        assert!(content.contains("description: Custom words"), "{content}");
        assert!(!content.contains(TEMPLATES[2].description), "{content}");
    }

    #[test]
    fn description_over_the_target_limit_is_refused() {
        let too_long = "x".repeat(600);
        let err = render_skill("testing", "my-skill", Some(&too_long), Platform::Codex)
            .expect_err("over-long description must not render");
        assert!(format!("{err:#}").contains("description"), "{err:#}");
    }

    #[test]
    fn unknown_template_error_lists_every_valid_name() {
        let err = render_skill("nope", "my-skill", None, Platform::Claude).expect_err("unknown");
        let message = format!("{err:#}");
        assert!(message.contains("nope"), "{message}");
        for expected in EXPECTED_NAMES {
            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn listing_shows_each_template_name_and_description() {
        let listing = list_templates();
        for template in TEMPLATES {
            assert!(listing.contains(template.name), "{listing}");
            assert!(listing.contains(template.description), "{listing}");
        }
    }

    #[test]
    fn names_that_escape_the_target_directory_are_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        for bad in ["", "../evil", ".hidden", "a/b", "two\nlines"] {
            let mut req = request("testing", Platform::Claude, tmp.path());
            req.name = bad.to_string();
            assert!(create_from_template(&req).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn create_writes_the_rendered_skill_under_the_name_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let outcome = create_from_template(&request("debugging", Platform::Copilot, tmp.path()))
            .expect("create");
        assert_eq!(outcome.path, tmp.path().join("my-skill").join("SKILL.md"));
        assert!(outcome.written);
        let on_disk = std::fs::read_to_string(&outcome.path).expect("read back");
        assert_eq!(on_disk, outcome.content);
    }

    #[test]
    fn dry_run_writes_nothing_and_creates_no_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut req = request("debugging", Platform::Claude, tmp.path());
        req.dry_run = true;
        let outcome = create_from_template(&req).expect("dry run");
        assert!(!outcome.written);
        assert!(outcome.content.starts_with("---\n"));
        assert!(!tmp.path().join("my-skill").exists());
    }

    #[test]
    fn existing_skill_file_is_not_overwritten() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let req = request("testing", Platform::Claude, tmp.path());
        let skill_path = tmp.path().join("my-skill").join("SKILL.md");
        std::fs::create_dir_all(skill_path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&skill_path, "hand-written").expect("seed");

        let err = create_from_template(&req).expect_err("must refuse");
        assert!(format!("{err:#}").contains("already exists"), "{err:#}");
        assert_eq!(
            std::fs::read_to_string(&skill_path).expect("read"),
            "hand-written"
        );
    }

    #[test]
    fn default_directory_follows_the_target_cli() {
        let home = Path::new("/home/dev");
        assert_eq!(
            default_target_dir(home, Platform::Claude),
            home.join(".claude/skills")
        );
        assert_eq!(
            default_target_dir(home, Platform::Codex),
            home.join(".agents/skills")
        );
        assert_eq!(
            default_target_dir(home, Platform::Copilot),
            home.join(".copilot/skills")
        );
        assert_eq!(
            default_target_dir(home, Platform::Cursor),
            home.join(".cursor/skills")
        );
    }
}
