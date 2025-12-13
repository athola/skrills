use crate::discovery::merge_extra_dirs;
use crate::sync::{mirror_source_root, sync_agents, sync_from_claude};
use anyhow::Result;
use skrills_state::home_dir;
use std::path::PathBuf;

pub(crate) fn handle_sync_agents_command(
    path: Option<PathBuf>,
    skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    let path = path.unwrap_or_else(|| PathBuf::from("AGENTS.md"));
    sync_agents(&path, &merge_extra_dirs(&skill_dirs))?;
    println!("Updated {}", path.display());
    Ok(())
}

/// Handle the `sync` command.
pub(crate) fn handle_sync_command(include_marketplace: bool) -> Result<()> {
    let home = home_dir()?;
    let report = sync_from_claude(
        &mirror_source_root(&home),
        &home.join(".codex/skills-mirror"),
        include_marketplace,
    )?;
    println!("copied: {}, skipped: {}", report.copied, report.skipped);
    Ok(())
}

pub(crate) fn handle_mirror_command(
    dry_run: bool,
    skip_existing_commands: bool,
    include_marketplace: bool,
) -> Result<()> {
    let home = home_dir()?;
    let claude_root = mirror_source_root(&home);
    if !skip_existing_commands {
        eprintln!(
            "Warning: mirroring commands into ~/.codex/prompts will overwrite prompts with the same name unless --skip-existing-commands is used."
        );
    }
    // Mirror skills/agents/support files
    let report = sync_from_claude(
        &claude_root,
        &home.join(".codex/skills-mirror"),
        include_marketplace,
    )?;
    // Mirror commands/mcp/prefs
    let source = skrills_sync::ClaudeAdapter::new()?;
    let target = skrills_sync::CodexAdapter::new()?;
    let orch = skrills_sync::SyncOrchestrator::new(source, target);
    let params = skrills_sync::SyncParams {
        dry_run,
        sync_skills: false,
        sync_commands: true,
        skip_existing_commands,
        sync_mcp_servers: true,
        sync_preferences: true,
        include_marketplace,
        ..Default::default()
    };
    let sync_report = orch.sync(&params)?;
    // Refresh AGENTS.md with skills + agents (mirror roots now populated)
    handle_sync_agents_command(None, vec![])?;

    println!(
        "mirror complete: skills copied {}, skipped {}; commands written {}, skipped {}; prefs {}, mcp {}{}",
        report.copied,
        report.skipped,
        sync_report.commands.written,
        sync_report.commands.skipped.len(),
        sync_report.preferences.written,
        sync_report.mcp_servers.written,
        if dry_run {
            " (dry-run for commands/prefs/mcp)"
        } else {
            ""
        }
    );

    if skip_existing_commands && !sync_report.commands.skipped.is_empty() {
        println!("Skipped existing commands (kept target copy):");
        for reason in &sync_report.commands.skipped {
            println!("  - {}", reason.description());
        }
    }
    Ok(())
}
