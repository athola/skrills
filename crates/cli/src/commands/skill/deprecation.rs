use anyhow::{Context, Result};
use skrills_discovery::discover_skills;
use std::path::PathBuf;

use crate::cli::OutputFormat;

use super::{escape_yaml_string, find_skill, skill_dir_name, DeprecationResult};

/// Handle the skill-deprecate command.
pub(crate) fn handle_skill_deprecate_command(
    name: String,
    message: Option<String>,
    replacement: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    let roots = crate::commands::skill_roots_for(&skill_dirs);
    let skills = discover_skills(&roots, None)?;
    let skill = find_skill(&skills, &name)?;

    let skill_path = skill.path.clone();
    let content = std::fs::read_to_string(&skill_path)
        .with_context(|| format!("Failed to read skill file: {}", skill_path.display()))?;

    let deprecation_msg = message.as_deref().unwrap_or("This skill is deprecated");
    let Some(new_content) = deprecated_content(
        &content,
        &skill_dir_name(skill),
        deprecation_msg,
        replacement.as_deref(),
    )?
    else {
        if format.is_json() {
            let result = DeprecationResult {
                skill_name: skill.name.clone(),
                skill_path: skill_path.clone(),
                deprecated: false,
                message: Some("Skill is already marked as deprecated".to_string()),
                replacement: None,
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("Skill '{}' is already marked as deprecated", skill.name);
        }
        return Ok(());
    };

    std::fs::write(&skill_path, &new_content)
        .with_context(|| format!("Failed to write to skill file: {}", skill_path.display()))?;

    let result = DeprecationResult {
        skill_name: skill.name.clone(),
        skill_path: skill_path.clone(),
        deprecated: true,
        message: Some(deprecation_msg.to_string()),
        replacement: replacement.clone(),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Skill '{}' has been deprecated", skill.name);
        println!("  Path: {}", skill_path.display());
        println!("  Message: {}", deprecation_msg);
        if let Some(repl) = &replacement {
            println!("  Replacement: {}", repl);
        }
    }

    Ok(())
}

/// `content` with the deprecation fields added to its frontmatter, or `None`
/// when it is already marked deprecated.
///
/// A file without frontmatter gains one whose `name` is `dir_name`, the
/// skill's directory, not the discovery key (`foo/SKILL.md`). Every inserted
/// scalar is double-quoted and escaped.
fn deprecated_content(
    content: &str,
    dir_name: &str,
    message: &str,
    replacement: Option<&str>,
) -> Result<Option<String>> {
    use skrills_validate::frontmatter::parse_frontmatter;

    let parsed = parse_frontmatter(content).map_err(|e| anyhow::anyhow!(e))?;

    let mut fields = String::from("deprecated: true\n");
    fields.push_str(&format!(
        "deprecation_message: \"{}\"\n",
        escape_yaml_string(message)
    ));
    if let Some(repl) = replacement {
        fields.push_str(&format!("replacement: \"{}\"\n", escape_yaml_string(repl)));
    }

    let mut out = String::from("---\n");
    if let Some(raw_fm) = &parsed.raw_frontmatter {
        if raw_fm.lines().any(|l| l.starts_with("deprecated:")) {
            return Ok(None);
        }
        for line in raw_fm.lines() {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&fields);
        out.push_str("---\n");
        out.push_str(&parsed.content);
    } else {
        out.push_str(&format!("name: \"{}\"\n", escape_yaml_string(dir_name)));
        out.push_str(&fields);
        out.push_str("---\n\n");
        out.push_str(content);
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::super::DeprecationResult;
    use super::*;

    #[test]
    fn deprecation_result_serializes_all_fields() {
        let result = DeprecationResult {
            skill_name: "old-skill".to_string(),
            skill_path: PathBuf::from("/skills/old-skill.md"),
            deprecated: true,
            message: Some("Use new-skill instead".to_string()),
            replacement: Some("new-skill".to_string()),
        };
        let json = serde_json::to_string_pretty(&result).unwrap();
        assert!(json.contains("\"skill_name\": \"old-skill\""));
        assert!(json.contains("\"deprecated\": true"));
        assert!(json.contains("Use new-skill instead"));
    }

    #[test]
    fn appends_fields_to_existing_frontmatter() {
        let doc = "---\nname: test-skill\ndescription: A test\n---\n# Body\n";

        let out = deprecated_content(doc, "test-skill", "Superseded", Some("better"))
            .unwrap()
            .unwrap();

        assert!(out.starts_with("---\nname: test-skill\ndescription: A test\n"));
        assert!(out.contains("deprecated: true\n"));
        assert!(out.contains("deprecation_message: \"Superseded\"\n"));
        assert!(out.contains("replacement: \"better\"\n"));
        assert!(out.ends_with("# Body\n"));
    }

    #[test]
    fn an_already_deprecated_skill_is_left_alone() {
        let doc = "---\nname: x\ndeprecated: true\n---\nbody\n";
        assert!(deprecated_content(doc, "x", "m", None).unwrap().is_none());
    }

    /// SA-46: with no frontmatter the name came from the discovery key
    /// (`foo/SKILL.md`) unquoted, and a newline in the message broke the YAML.
    #[test]
    fn new_frontmatter_uses_the_directory_name_and_escapes_the_message() {
        let doc = "# Body only\n";

        let out = deprecated_content(doc, "foo", "line one\nkey: injected", None)
            .unwrap()
            .unwrap();

        assert!(out.starts_with("---\nname: \"foo\"\n"), "{out}");
        assert!(
            out.contains("deprecation_message: \"line one\\nkey: injected\"\n"),
            "{out}"
        );
        let parsed = skrills_validate::frontmatter::parse_frontmatter(&out).unwrap();
        let raw = parsed.raw_frontmatter.expect("frontmatter");
        assert!(
            !raw.lines().any(|l| l.starts_with("key:")),
            "the message leaked a key: {raw}"
        );
        assert!(out.ends_with("# Body only\n"));
    }
}
