//! Agents, instructions, and hooks for Copilot adapter.

use super::paths::{agents_dir, instructions_dir};
use super::utils::transform_agent_for_copilot;
use crate::adapters::utils::{hash_content, is_hidden_path, sanitize_name_segments};
use crate::common::{Command, ContentFormat};
use crate::report::WriteReport;
use crate::Result;
use anyhow::Context;
use std::fs;
use std::path::Path;
use tracing::warn;
use walkdir::WalkDir;

/// Reads agents from the agents directory.
pub fn read_agents(root: &Path) -> Result<Vec<Command>> {
    let agents_dir = agents_dir(root);
    if !agents_dir.exists() {
        return Ok(Vec::new());
    }

    let mut agents = Vec::new();

    for entry in WalkDir::new(&agents_dir)
        .min_depth(1)
        .max_depth(1) // Flat directory structure for Copilot agents
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    path = ?e.path(),
                    error = %e,
                    "Failed to read directory entry while scanning agents"
                );
                continue;
            }
        };
        let path = entry.path();

        // `file_type` does not follow links: a linked file could be any file
        // on disk, and it would be copied into every other tool.
        if !entry.file_type().is_file() {
            continue;
        }

        // Copilot agents are *.agent.md or *.md files
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !file_name.ends_with(".md") {
            continue;
        }

        if is_hidden_path(path.strip_prefix(&agents_dir).unwrap_or(path)) {
            continue;
        }

        // Extract name: strip .agent.md or .md suffix
        // `strip_suffix` removes one suffix; `trim_end_matches` stripped every
        // repeat, so `x.agent.md.agent.md` read back as `x`.
        let name = if let Some(stem) = file_name.strip_suffix(".agent.md") {
            stem.to_string()
        } else {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        };

        let Some((content, modified)) = crate::adapters::utils::read_item_file(path) else {
            continue;
        };
        let hash = hash_content(&content);

        agents.push(Command {
            name,
            content,
            source_path: path.to_path_buf(),
            modified,
            hash,
            modules: Vec::new(),
            content_format: ContentFormat::default(),
            plugin_origin: None,
        });
    }

    Ok(agents)
}

/// Writes agents to the agents directory.
pub fn write_agents(root: &Path, agents: &[Command]) -> Result<WriteReport> {
    let dir = agents_dir(root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create agents directory: {}", dir.display()))?;

    let mut report = WriteReport::default();

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for agent in agents {
        // Flat, like the reader: a nested `team/reviewer` becomes
        // `team-reviewer.agent.md` rather than a file it never reads back.
        let safe_name = sanitize_name_segments(&agent.name).replace('/', "-");

        // Transform the content: Claude format -> Copilot format
        let transformed_content = transform_agent_for_copilot(&agent.content);

        writer.write_single(
            &agent.name,
            &safe_name,
            ".agent.md",
            &transformed_content,
            &mut report,
        )?;
    }

    Ok(report)
}

/// Reads instructions from the instructions directory.
pub fn read_instructions(root: &Path) -> Result<Vec<Command>> {
    let dir = instructions_dir(root);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut instructions = Vec::new();
    for entry in WalkDir::new(&dir)
        .min_depth(1)
        // Copilot CLI loads `instructions/**/*.instructions.md`.
        .max_depth(8)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    path = ?e.path(),
                    error = %e,
                    "Failed to read directory entry while scanning instructions"
                );
                continue;
            }
        };
        let path = entry.path();

        // `file_type` does not follow links: a linked file could be any file
        // on disk, and it would be copied into every other tool.
        if !entry.file_type().is_file() {
            continue;
        }

        // Copilot instructions are *.instructions.md files
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !file_name.ends_with(".instructions.md") {
            continue;
        }

        if is_hidden_path(path.strip_prefix(&dir).unwrap_or(path)) {
            continue;
        }

        // Name: the path below the directory without .instructions.md, so a
        // nested file reads back under the name it was written with.
        let Some(name) = path
            .strip_prefix(&dir)
            .ok()
            .and_then(|rel| rel.to_str())
            .and_then(|rel| rel.strip_suffix(".instructions.md"))
            .map(|rel| rel.replace(std::path::MAIN_SEPARATOR, "/"))
        else {
            continue;
        };

        let Some((content, modified)) = crate::adapters::utils::read_item_file(path) else {
            continue;
        };
        // Copilot scopes with `applyTo`; the other tools call it `globs`.
        let content = rename_frontmatter_key(content, "applyTo", "globs");
        let hash = hash_content(&content);

        instructions.push(Command {
            name,
            content,
            source_path: path.to_path_buf(),
            modified,
            hash,
            modules: Vec::new(),
            content_format: ContentFormat::default(),
            plugin_origin: None,
        });
    }

    Ok(instructions)
}

/// Writes instructions to `~/.copilot/instructions/`, which Copilot CLI loads
/// as `**/*.instructions.md`, scoped by an `applyTo` glob.
pub fn write_instructions(root: &Path, instructions: &[Command]) -> Result<WriteReport> {
    let dir = instructions_dir(root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create instructions directory: {}", dir.display()))?;

    let mut report = WriteReport::default();

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for instruction in instructions {
        let safe_name = sanitize_name_segments(&instruction.name);

        let content = rename_frontmatter_key(instruction.content.clone(), "globs", "applyTo");
        writer.write_single(
            &instruction.name,
            &safe_name,
            ".instructions.md",
            &content,
            &mut report,
        )?;
    }

    Ok(report)
}

/// `content` with the top-level frontmatter key `from` renamed to `to`. Left
/// as it is when it is not UTF-8, has no frontmatter, or already has `to`.
fn rename_frontmatter_key(content: Vec<u8>, from: &str, to: &str) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(&content) else {
        return content;
    };
    let Some(rest) = text.strip_prefix("---\n") else {
        return content;
    };
    let Some(end) = rest.find("\n---") else {
        return content;
    };
    let frontmatter = &rest[..end];
    let (from_key, to_key) = (format!("{from}:"), format!("{to}:"));
    if frontmatter.lines().any(|l| l.starts_with(&to_key))
        || !frontmatter.lines().any(|l| l.starts_with(&from_key))
    {
        return content;
    }
    let renamed: Vec<String> = frontmatter
        .lines()
        .map(|line| match line.strip_prefix(&from_key) {
            Some(value) => format!("{to_key}{value}"),
            None => line.to_string(),
        })
        .collect();
    format!("---\n{}{}", renamed.join("\n"), &rest[end..]).into_bytes()
}

/// Reads hooks - Copilot does not support hooks.
pub fn read_hooks(_root: &Path) -> Result<Vec<Command>> {
    // Copilot does not support hooks
    Ok(Vec::new())
}

/// Writes hooks - Copilot does not support hooks.
pub fn write_hooks(_root: &Path, _hooks: &[Command]) -> Result<WriteReport> {
    // Copilot does not support hooks
    Ok(WriteReport::default())
}
