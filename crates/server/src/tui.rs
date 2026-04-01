//! Interactive terminal UI for skill sync management.
//!
//! Provides a user-friendly interface to sync skills, commands,
//! and configurations between Claude and Codex.

use anyhow::{anyhow, Result};
use inquire::Confirm;
use skrills_state::home_dir;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::sync::{mirror_source_root, sync_agents_only_from_claude, sync_skills_only_from_claude};
use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

/// Runs an interactive TUI for sync management.
///
/// Users can sync skills, commands, MCP servers, and preferences
/// from Claude to Codex.
pub(crate) fn tui_flow(_extra_dirs: &[PathBuf]) -> Result<()> {
    if !std::io::stdout().is_terminal() {
        return Err(anyhow!("TUI requires a TTY"));
    }

    if !Confirm::new("Run claude → codex sync? (skills, agents, commands, prefs)")
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
    let agent_report = sync_agents_only_from_claude(
        &mirror_source_root(&home),
        &home.join(".codex/agents"),
        include_marketplace,
    )?;
    let codex_report = sync_skills_only_from_claude(
        &mirror_source_root(&home),
        &home.join(".codex/skills"),
        include_marketplace,
    )?;
    let _ = crate::setup::ensure_codex_skills_feature_enabled(&home.join(".codex/config.toml"));

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
        "Sync complete:\n  Agents: copied {}, skipped {}\n  Skills (codex): copied {}, skipped {}\n  Commands: written {}, skipped {}{}\n  Prefs: {}  MCP: {}",
        agent_report.copied,
        agent_report.skipped,
        codex_report.copied,
        codex_report.skipped,
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

    // Show MCP servers with tool filtering info if any were synced
    if sync_report.mcp_servers.written > 0 {
        use skrills_sync::adapters::traits::AgentAdapter;
        if let Ok(target_adapter) = CodexAdapter::new() {
            if let Ok(servers) = target_adapter.read_mcp_servers() {
                let filtered: Vec<_> = servers
                    .values()
                    .filter(|s| !s.allowed_tools.is_empty() || !s.disabled_tools.is_empty())
                    .collect();
                if !filtered.is_empty() {
                    println!("  MCP tool filters:");
                    for s in filtered {
                        let mut info = format!("    {}", s.name);
                        if !s.allowed_tools.is_empty() {
                            info.push_str(&format!(" allow:[{}]", s.allowed_tools.join(",")));
                        }
                        if !s.disabled_tools.is_empty() {
                            info.push_str(&format!(" deny:[{}]", s.disabled_tools.join(",")));
                        }
                        println!("{info}");
                    }
                }
            }
        }
    }

    Ok(())
}
