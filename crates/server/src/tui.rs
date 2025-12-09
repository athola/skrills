//! Interactive terminal UI for skill management.
//!
//! It provides a user-friendly interface to:
//! - Synchronizing skills from `~/.claude`.
//! - Selecting and pinning skills.
//! - Managing skill visibility.

use anyhow::{anyhow, Result};
use inquire::{Confirm, MultiSelect};
use skrills_state::{home_dir, load_pinned, save_pinned};
use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::discovery::collect_skills;
use crate::sync::{mirror_source_root, sync_from_claude};
use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

/// Runs an interactive TUI for sync and pin management.
///
/// Users can:
/// 1. Optionally sync skills from `~/.claude` to `~/.codex/skills-mirror`.
/// 2. Select which skills to pin for autoload.
pub(crate) fn tui_flow(extra_dirs: &[PathBuf]) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        return Err(anyhow!("TUI requires a TTY"));
    }
    if Confirm::new("Run claude → codex mirror sync first? (skills, agents, commands, prefs)")
        .with_default(false)
        .prompt()?
    {
        let skip_existing_commands =
            Confirm::new("Keep existing prompts under ~/.codex/prompts (skip overwrites)?")
                .with_default(true)
                .prompt()?;

        let home = home_dir()?;
        let mirror_root = home.join(".codex/skills-mirror");
        let report = sync_from_claude(&mirror_source_root(&home), &mirror_root)?;

        // Mirror commands/prefs/MCP
        let source = ClaudeAdapter::new()?;
        let target = CodexAdapter::new()?;
        let orch = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands,
            sync_mcp_servers: true,
            sync_preferences: true,
            ..Default::default()
        };
        let sync_report = orch.sync(&params)?;

        println!(
            "Mirror sync complete:\n  Skills: copied {}, skipped {}\n  Commands: written {}, skipped {}{}\n  Prefs: {}  MCP: {}",
            report.copied,
            report.skipped,
            sync_report.commands.written,
            sync_report.commands.skipped.len(),
            if skip_existing_commands && !sync_report.commands.skipped.is_empty() {
                format!(
                    " (kept existing: {})",
                    sync_report
                        .commands
                        .skipped
                        .iter()
                        .map(|r| r.description())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            },
            sync_report.preferences.written,
            sync_report.mcp_servers.written,
        );
    }

    let skills = collect_skills(extra_dirs)?;
    if skills.is_empty() {
        println!("No skills found.");
        return Ok(());
    }

    let pinned = load_pinned().unwrap_or_default();
    let mut items = Vec::new();
    let mut default_indices = Vec::new();
    for (idx, s) in skills.iter().enumerate() {
        let display = format!(
            "[{} | {}] {}",
            s.source.label(),
            s.source.location(),
            s.name
        );
        items.push(display);
        if pinned.contains(&s.name) {
            default_indices.push(idx);
        }
    }

    let selected = MultiSelect::new(
        "Select skills to pin (space to toggle, enter to save)",
        items.clone(),
    )
    .with_default(&default_indices)
    .prompt()?;

    let mut new_pins = HashSet::new();
    for item in &selected {
        if let Some(skill) = skills.iter().find(|s| {
            let display = format!(
                "[{} | {}] {}",
                s.source.label(),
                s.source.location(),
                s.name
            );
            &display == item
        }) {
            new_pins.insert(skill.name.clone());
        }
    }
    save_pinned(&new_pins)?;
    println!("Pinned {} skills.", new_pins.len());
    Ok(())
}
