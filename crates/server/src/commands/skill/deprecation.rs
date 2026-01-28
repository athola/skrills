use anyhow::{Context, Result};
use skrills_discovery::{discover_skills, extra_skill_roots};
use std::path::PathBuf;

use crate::cli::OutputFormat;
use crate::discovery::merge_extra_dirs;

use super::{escape_yaml_string, DeprecationResult};

/// Handle the skill-deprecate command.
pub(crate) fn handle_skill_deprecate_command(
    name: String,
    message: Option<String>,
    replacement: Option<String>,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    use skrills_validate::frontmatter::parse_frontmatter;

    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    let skill = skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(&name) || s.path.to_string_lossy().contains(&name))
        .with_context(|| format!("Skill '{}' not found in discovered skills", name))?;

    let skill_path = skill.path.clone();
    let content = std::fs::read_to_string(&skill_path)
        .with_context(|| format!("Failed to read skill file: {}", skill_path.display()))?;

    let parsed = parse_frontmatter(&content).map_err(|e| anyhow::anyhow!(e))?;

    let deprecation_msg = message.as_deref().unwrap_or("This skill is deprecated");
    let mut new_content = String::new();

    if let Some(raw_fm) = &parsed.raw_frontmatter {
        let fm_lines: Vec<&str> = raw_fm.lines().collect();

        if fm_lines.iter().any(|l| l.starts_with("deprecated:")) {
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
        }

        new_content.push_str("---\n");
        for line in &fm_lines {
            new_content.push_str(line);
            new_content.push('\n');
        }
        new_content.push_str("deprecated: true\n");
        let escaped_msg = escape_yaml_string(deprecation_msg);
        new_content.push_str(&format!("deprecation_message: \"{}\"\n", escaped_msg));
        if let Some(ref repl) = replacement {
            let escaped_repl = escape_yaml_string(repl);
            new_content.push_str(&format!("replacement: \"{}\"\n", escaped_repl));
        }
        new_content.push_str("---\n");
        new_content.push_str(&parsed.content);
    } else {
        new_content.push_str("---\n");
        new_content.push_str(&format!("name: {}\n", skill.name));
        new_content.push_str("deprecated: true\n");
        let escaped_msg = escape_yaml_string(deprecation_msg);
        new_content.push_str(&format!("deprecation_message: \"{}\"\n", escaped_msg));
        if let Some(ref repl) = replacement {
            let escaped_repl = escape_yaml_string(repl);
            new_content.push_str(&format!("replacement: \"{}\"\n", escaped_repl));
        }
        new_content.push_str("---\n\n");
        new_content.push_str(&content);
    }

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
