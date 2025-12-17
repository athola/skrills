use crate::discovery::merge_extra_dirs;
use crate::sync::{
    mirror_source_root, sync_agents, sync_agents_only_from_claude, sync_skills_only_from_claude,
};
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
    let report = sync_skills_only_from_claude(
        &mirror_source_root(&home),
        &home.join(".codex/skills"),
        include_marketplace,
    )?;
    let _ = crate::setup::ensure_codex_skills_feature_enabled(&home.join(".codex/config.toml"));
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
    // Mirror agents into ~/.codex/agents (skills are materialized into ~/.codex/skills only).
    let agent_report = sync_agents_only_from_claude(
        &claude_root,
        &home.join(".codex/agents"),
        include_marketplace,
    )?;
    // Also materialize skills into ~/.codex/skills so Codex's built-in skills system can discover them.
    let codex_report = sync_skills_only_from_claude(
        &claude_root,
        &home.join(".codex/skills"),
        include_marketplace,
    )?;
    let _ = crate::setup::ensure_codex_skills_feature_enabled(&home.join(".codex/config.toml"));
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
        "mirror complete: agents copied {}, skipped {}; skills (codex) copied {}, skipped {}; commands written {}, skipped {}; prefs {}, mcp {}{}",
        agent_report.copied,
        agent_report.skipped,
        codex_report.copied,
        codex_report.skipped,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn mirror_command_does_not_create_skills_mirror_dir() -> Result<()> {
        let _guard = crate::test_support::env_guard();

        let tmp = tempdir()?;
        let home = tmp.path();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home);

        let original_cwd = std::env::current_dir()?;
        std::env::set_current_dir(home)?;

        // Seed one skill and one agent in the Claude source tree.
        let claude_skill = home.join(".claude/skills/example-skill/SKILL.md");
        std::fs::create_dir_all(claude_skill.parent().unwrap())?;
        std::fs::write(&claude_skill, "example skill")?;

        let claude_agent = home.join(".claude/plugins/cache/tool/agents/helper.md");
        std::fs::create_dir_all(claude_agent.parent().unwrap())?;
        std::fs::write(&claude_agent, "agent content")?;

        let result = handle_mirror_command(false, true, false);

        // Restore cwd/HOME before assertions to avoid leaking state.
        std::env::set_current_dir(original_cwd)?;
        match original_home {
            Some(val) => std::env::set_var("HOME", val),
            None => std::env::remove_var("HOME"),
        }

        result?;

        assert!(
            !home.join(".codex/skills-mirror").exists(),
            "mirror should not create ~/.codex/skills-mirror"
        );
        assert!(
            home.join(".codex/skills/skills/example-skill/SKILL.md")
                .exists(),
            "expected skill copied into ~/.codex/skills"
        );
        assert!(
            home.join(".codex/agents/plugins/cache/tool/agents/helper.md")
                .exists(),
            "expected agent copied into ~/.codex/agents"
        );
        Ok(())
    }
}
