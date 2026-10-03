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
use std::time::SystemTime;
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

        if !path.is_file() {
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

        let content = fs::read(path)?;
        let metadata = fs::metadata(path)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
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
        let safe_name = sanitize_name_segments(&agent.name);

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
        .max_depth(1) // Flat directory structure
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

        if !path.is_file() {
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

        // Extract name: strip .instructions.md suffix
        let Some(name) = file_name
            .strip_suffix(".instructions.md")
            .map(str::to_owned)
        else {
            continue;
        };

        let content = fs::read(path)?;
        let metadata = fs::metadata(path)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
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

/// Writes instructions to the instructions directory.
pub fn write_instructions(root: &Path, instructions: &[Command]) -> Result<WriteReport> {
    // Note: ~/.copilot/instructions/ is a staging location, not a standard Copilot path.
    // Copilot uses repository-level instructions at .github/instructions/*.instructions.md
    // or IDE-specific global paths (e.g., ~/.config/github-copilot/intellij/).
    let dir = instructions_dir(root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create instructions directory: {}", dir.display()))?;

    let mut report = WriteReport::default();

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for instruction in instructions {
        let safe_name = sanitize_name_segments(&instruction.name);

        writer.write_single(
            &instruction.name,
            &safe_name,
            ".instructions.md",
            &instruction.content,
            &mut report,
        )?;
    }

    // Add warning about the staging location
    if report.written > 0 {
        report.warnings.push(format!(
            "Instructions written to {} (staging). \
             Copy to .github/instructions/ in your repository for Copilot to use them, \
             or to ~/.config/github-copilot/intellij/global-copilot-instructions.md for JetBrains IDEs.",
            dir.display()
        ));
    }

    Ok(report)
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
