//! Interactive terminal UI for skill sync management.
//!
//! Provides a user-friendly interface to sync skills, commands,
//! and configurations between Claude and Codex.

use anyhow::{anyhow, Result};
use inquire::Confirm;
use skrills_state::home_dir;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::sync::{mirror_source_root, sync_from_claude};
use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

/// Runs an interactive TUI for sync management.
///
/// Users can sync skills, commands, MCP servers, and preferences
/// from Claude to Codex.
pub(crate) fn tui_flow(_extra_dirs: &[PathBuf]) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        return Err(anyhow!("TUI requires a TTY"));
    }

    if !Confirm::new("Run claude → codex mirror sync? (skills, agents, commands, prefs)")
        .with_default(true)
        .prompt()?
    {
        println!("Sync cancelled.");
        return Ok(());
    }

    let skip_existing_commands =
        Confirm::new("Keep existing prompts under ~/.codex/prompts (skip overwrites)?")
            .with_default(true)
            .prompt()?;

    let include_marketplace = Confirm::new("Include marketplace content (uninstalled plugins)?")
        .with_default(false)
        .prompt()?;

    let home = home_dir()?;
    let mirror_root = home.join(".codex/skills-mirror");
    let report = sync_from_claude(
        &mirror_source_root(&home),
        &mirror_root,
        include_marketplace,
    )?;

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
        include_marketplace,
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

    Ok(())
}
