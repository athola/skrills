//! Instructions (CLAUDE.md) reading/writing for the Claude adapter.
//!
//! Claude only supports a single CLAUDE.md file at the config root.
//! When multiple instruction documents are written, they are merged
//! with header markers so a single read still reflects all sources.

use crate::adapters::utils::hash_content;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;

use std::fs;
use std::time::SystemTime;

use super::ClaudeAdapter;

pub(super) fn read_instructions_impl(adapter: &ClaudeAdapter) -> Result<Vec<Command>> {
    let path = adapter.instructions_path();
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read(&path)?;
    let metadata = fs::metadata(&path)?;
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let hash = hash_content(&content);

    // Use "CLAUDE" as the instruction name (derived from CLAUDE.md)
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("CLAUDE")
        .to_string();

    Ok(vec![Command {
        name,
        content,
        source_path: path.clone(),
        modified,
        hash,
        modules: Vec::new(),

        content_format: ContentFormat::default(),
        plugin_origin: None,
    }])
}

pub(super) fn write_instructions_impl(
    adapter: &ClaudeAdapter,
    instructions: &[Command],
) -> Result<WriteReport> {
    let mut report = WriteReport::default();

    // Claude only supports a single CLAUDE.md file
    // If multiple instructions are provided, merge them or take the first
    if instructions.is_empty() {
        return Ok(report);
    }

    let path = adapter.instructions_path();

    // Merge all instructions content if multiple are provided
    let merged_content: Vec<u8> = if instructions.len() == 1 {
        instructions[0].content.clone()
    } else {
        // Merge multiple instructions with headers
        let mut merged = Vec::new();
        for (i, instruction) in instructions.iter().enumerate() {
            if i > 0 {
                merged.extend_from_slice(b"\n\n---\n\n");
            }
            merged
                .extend_from_slice(format!("<!-- Source: {} -->\n\n", instruction.name).as_bytes());
            merged.extend_from_slice(&instruction.content);
        }
        merged
    };

    if path.exists() {
        let existing = fs::read(&path)?;
        if hash_content(&existing) == hash_content(&merged_content) {
            report.skipped.push(SkipReason::Unchanged {
                item: "CLAUDE.md".to_string(),
            });
            return Ok(report);
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &merged_content)?;
    report.written += 1;

    Ok(report)
}
