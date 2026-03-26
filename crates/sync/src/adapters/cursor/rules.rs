//! Rules reading and writing for Cursor adapter.
//!
//! Cursor rules are `.mdc` (or `.md`) files in `.cursor/rules/` with optional
//! YAML frontmatter controlling application mode:
//!
//! | Mode           | Frontmatter                                           |
//! |----------------|-------------------------------------------------------|
//! | Always         | `alwaysApply: true`                                   |
//! | Auto-attach    | `globs: "pattern"`, `alwaysApply: false`              |
//! | Agent-requested| `description: "..."` only (no globs, not alwaysApply) |
//! | Manual         | No frontmatter fields                                 |
//!
//! ## Claude → Cursor Mapping
//!
//! - `CLAUDE.md` → `.cursor/rules/claude-md.mdc` with `alwaysApply: true`
//! - `.claude/rules/*.md` with globs → `.cursor/rules/*.mdc` preserving globs
//! - Other instructions → agent-requested rules with description

use super::paths::rules_dir;
use super::utils::{parse_frontmatter, render_frontmatter, sanitize_name};
use crate::adapters::utils::hash_content;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::debug;
use walkdir::WalkDir;

/// Reads all rules from `.cursor/rules/` (both `.mdc` and `.md` files).
pub fn read_rules(root: &Path) -> Result<Vec<Command>> {
    let dir = rules_dir(root);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut rules = Vec::new();

    for entry in WalkDir::new(&dir)
        .min_depth(1)
        .max_depth(5)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Skipping rules directory entry due to traversal error"
                );
                continue;
            }
        };

        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "mdc" && ext != "md" {
            continue;
        }

        let name = path
            .strip_prefix(&dir)
            .ok()
            .and_then(|rel| {
                rel.with_extension("")
                    .to_str()
                    .map(|s| s.replace(std::path::MAIN_SEPARATOR, "-"))
            })
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });

        let content = fs::read(path)?;
        let hash = hash_content(&content);
        let modified = fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        rules.push(Command {
            name,
            content,
            source_path: path.to_path_buf(),
            modified,
            hash,
            modules: vec![],
            content_format: ContentFormat::default(),
        });
    }

    rules.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rules)
}

/// Writes rules as `.cursor/rules/{name}.mdc` files.
///
/// Generates appropriate `.mdc` frontmatter based on the source content:
/// - Content from CLAUDE.md → `alwaysApply: true`
/// - Content with existing glob metadata → preserves globs
/// - Other content → agent-requested mode (description only)
pub fn write_rules(root: &Path, instructions: &[Command]) -> Result<WriteReport> {
    let dir = rules_dir(root);
    let mut report = WriteReport::default();

    if instructions.is_empty() {
        return Ok(report);
    }

    fs::create_dir_all(&dir)?;

    for instruction in instructions {
        let content_str = String::from_utf8_lossy(&instruction.content);
        let name = sanitize_name(&instruction.name);

        // Determine rule mode based on source
        let (frontmatter, body) = derive_rule_mode(&instruction.name, &content_str);
        let mdc_content = render_frontmatter(&frontmatter, &body);

        let path = dir.join(format!("{}.mdc", name));

        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_content(&existing) == hash_content(mdc_content.as_bytes()) {
                report.skipped.push(SkipReason::Unchanged {
                    item: instruction.name.clone(),
                });
                continue;
            }
        }

        debug!(name = %name, path = ?path, "Writing Cursor rule");
        fs::write(&path, mdc_content.as_bytes())?;
        report.written += 1;
    }

    Ok(report)
}

/// Derives the appropriate Cursor rule mode from the source instruction.
///
/// Returns (frontmatter_fields, body_content).
fn derive_rule_mode(
    name: &str,
    content: &str,
) -> (std::collections::HashMap<String, String>, String) {
    let mut fields = std::collections::HashMap::new();

    // Check if the content already has frontmatter (e.g., from .claude/rules/)
    let (existing_fields, body) = parse_frontmatter(content);

    // If source already has globs, preserve them
    if let Some(globs) = existing_fields.get("globs") {
        fields.insert("globs".to_string(), globs.clone());
        fields.insert("alwaysApply".to_string(), "false".to_string());
        if let Some(desc) = existing_fields.get("description") {
            fields.insert("description".to_string(), desc.clone());
        }
        return (fields, body.to_string());
    }

    // If source has alwaysApply, honor it
    if existing_fields
        .get("alwaysApply")
        .is_some_and(|v| v == "true")
    {
        fields.insert("alwaysApply".to_string(), "true".to_string());
        if let Some(desc) = existing_fields.get("description") {
            fields.insert("description".to_string(), desc.clone());
        }
        return (fields, body.to_string());
    }

    // CLAUDE.md, CLAUDE, claude-md (read-back form), or claude-instructions → always apply
    let name_lower = name.to_lowercase();
    if name_lower == "claude"
        || name_lower == "claude.md"
        || name_lower == "claude-md"
        || name_lower.contains("claude-instruction")
    {
        fields.insert("alwaysApply".to_string(), "true".to_string());
        fields.insert(
            "description".to_string(),
            "Project instructions (migrated from CLAUDE.md)".to_string(),
        );
        return (fields, body.to_string());
    }

    // Default: agent-requested mode (description + no globs + alwaysApply false)
    fields.insert("alwaysApply".to_string(), "false".to_string());
    fields.insert(
        "description".to_string(),
        format!("Rule: {}", name.replace('-', " ")),
    );

    (fields, body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_claude_md_as_always_apply() {
        let (fields, _body) =
            derive_rule_mode("CLAUDE.md", "# Project Instructions\n\nDo things.\n");
        assert_eq!(fields.get("alwaysApply").unwrap(), "true");
        assert!(fields.get("description").unwrap().contains("CLAUDE.md"));
    }

    #[test]
    fn derive_glob_scoped_rule() {
        let content =
            "---\nglobs: \"**/*.test.ts\"\ndescription: Testing rules\n---\n\n# Test Guidelines\n";
        let (fields, body) = derive_rule_mode("testing", content);
        assert_eq!(fields.get("globs").unwrap(), "\"**/*.test.ts\"");
        assert_eq!(fields.get("alwaysApply").unwrap(), "false");
        assert!(body.starts_with("# Test Guidelines"));
    }

    #[test]
    fn derive_claude_md_readback_form_as_always_apply() {
        // After roundtrip: CLAUDE.md → claude-md.mdc → read back as "claude-md"
        let (fields, _body) =
            derive_rule_mode("claude-md", "# Project Instructions\n\nDo things.\n");
        assert_eq!(
            fields.get("alwaysApply").unwrap(),
            "true",
            "claude-md (read-back form) should be alwaysApply"
        );
    }

    #[test]
    fn derive_default_agent_requested() {
        let (fields, _body) = derive_rule_mode("code-style", "# Code Style\n\nUse tabs.\n");
        assert_eq!(fields.get("alwaysApply").unwrap(), "false");
        assert!(fields.contains_key("description"));
        assert!(!fields.contains_key("globs"));
    }
}
