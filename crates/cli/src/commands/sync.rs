use super::validate::{run_validation, Autofix};
use crate::cli::SyncSource;
use anyhow::{anyhow, bail, Result};
use skrills_discovery::{SkillRoot, SkillSource};
use skrills_server::discovery::merge_extra_dirs;
use skrills_server::sync::{
    mirror_source_root, sync_agents, sync_agents_only_from_claude, sync_skills_only_from_claude,
};
use skrills_state::home_dir;
use skrills_validate::ValidationTarget as VT;
use std::path::{Path, PathBuf};

/// Reports what stops Codex from loading the skills that were just mirrored.
///
/// `ensure_codex_skills_feature_enabled` fails when `~/.codex/config.toml` is
/// unreadable or unwritable, and Codex only loads skills when
/// `[features] skills = true`. Without this the commands print "copied: N" and
/// exit zero while Codex loads nothing.
pub(crate) fn codex_skills_feature_warning(config_path: &Path) -> Option<String> {
    let err = skrills_server::setup::ensure_codex_skills_feature_enabled(config_path).err()?;
    tracing::warn!(error = %err, "could not ensure codex skills feature flag");
    Some(format!(
        "Warning: could not enable the codex skills feature in {}: {}",
        config_path.display(),
        err
    ))
}

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
    if let Some(warning) = codex_skills_feature_warning(&home.join(".codex/config.toml")) {
        eprintln!("{warning}");
    }
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
    if let Some(warning) = codex_skills_feature_warning(&home.join(".codex/config.toml")) {
        eprintln!("{warning}");
    }
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
    // Refresh AGENTS.md with skills and agents (mirror roots now populated)
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

/// Runs one adapter sync from `from` to `to`.
pub(crate) fn run_sync_with_adapters(
    from: SyncSource,
    to: SyncSource,
    params: &skrills_sync::SyncParams,
) -> Result<skrills_sync::SyncReport> {
    if from == to {
        return Err(anyhow!(
            "Source and target cannot be the same: {}",
            from.as_str()
        ));
    }
    // sync_between handles adapter creation for every platform.
    skrills_sync::orchestrator::sync_between(from.as_str(), to.as_str(), params)
}

/// The directory a CLI keeps its skills in, as `sync-status` counts them.
pub(crate) fn source_skill_root(from: SyncSource, home: &Path) -> PathBuf {
    use skrills_sync::adapters::traits::AgentAdapter;
    match from {
        SyncSource::Claude => mirror_source_root(home),
        SyncSource::Codex => home.join(".codex/skills"),
        SyncSource::Copilot => skrills_sync::CopilotAdapter::new()
            .map(|a| a.config_root().join("skills"))
            .unwrap_or_else(|_| home.join(".copilot/skills")),
        SyncSource::Cursor => skrills_sync::CursorAdapter::new()
            .map(|a| a.config_root().join("skills"))
            .unwrap_or_else(|_| home.join(".cursor/skills")),
    }
}

/// The skill roots `sync-all` reads from `from`, for validation.
///
/// For Claude this is the user skills and the plugin cache under the mirror
/// source, plus the marketplaces when `include_marketplace` is set: the same
/// trees the skill sync walks. The rest of `~/.claude` (sessions, settings)
/// holds no skills.
fn source_validation_roots(
    from: SyncSource,
    home: &Path,
    include_marketplace: bool,
) -> Vec<SkillRoot> {
    let root = source_skill_root(from, home);
    let mut dirs = match from {
        SyncSource::Claude => vec![root.join("skills"), root.join("plugins/cache")],
        _ => vec![root.clone()],
    };
    if from.is_claude() && include_marketplace {
        dirs.push(root.join("plugins/marketplaces"));
    }
    dirs.into_iter()
        .enumerate()
        .map(|(i, root)| SkillRoot {
            root,
            source: SkillSource::Extra(i as u32),
        })
        .collect()
}

/// The strictest validation target any of `targets` needs.
fn validation_target_for(targets: &[SyncSource]) -> VT {
    let codex = targets.contains(&SyncSource::Codex);
    let copilot = targets.contains(&SyncSource::Copilot);
    match (codex, copilot) {
        (true, true) => VT::All,
        (true, false) => VT::Codex,
        (false, true) => VT::Copilot,
        // Claude and Cursor accept what Claude accepts.
        (false, false) => VT::Claude,
    }
}

/// Options for `sync-all`, one field per flag.
#[derive(Debug, Clone)]
pub(crate) struct SyncAllArgs {
    pub from: SyncSource,
    pub to: Option<SyncSource>,
    pub dry_run: bool,
    pub skip_existing_commands: bool,
    pub include_marketplace: bool,
    pub exclude_plugins: Vec<String>,
    pub validate: bool,
    pub autofix: bool,
}

/// Validates (and with `--autofix`, repairs) the source skills before any
/// target is touched. Returns an error, and so writes nothing, when an error
/// remains.
fn validate_sync_source(args: &SyncAllArgs, targets: &[SyncSource], home: &Path) -> Result<()> {
    let roots = source_validation_roots(args.from, home, args.include_marketplace);
    let target = validation_target_for(targets);
    let mode = match (args.autofix, args.dry_run) {
        (false, _) => Autofix::Off,
        // A dry run writes nothing, the source included: validate the fix
        // that would be made instead of making it.
        (true, true) => Autofix::Preview,
        (true, false) => Autofix::Write { backup: false },
    };
    let run = run_validation(&roots, target, mode)?;

    if !run.fixed.is_empty() {
        let verb = if args.dry_run { "would fix" } else { "fixed" };
        eprintln!(
            "sync-all: autofix {verb} {} source skill(s):",
            run.fixed.len()
        );
        for path in &run.fixed {
            eprintln!("  {}", path.display());
        }
    }
    if run.failed() {
        eprintln!("sync-all: source skills failed {target:?} validation:");
        for line in run.failure_lines() {
            eprintln!("{line}");
        }
        bail!(
            "sync-all aborted before syncing any target: the {} source skills do not validate{}",
            args.from.as_str(),
            if args.autofix {
                " even after autofix"
            } else {
                "; fix them or rerun with --autofix"
            }
        );
    }
    tracing::info!(
        skills = run.results.len(),
        target = ?target,
        "source skills validated"
    );
    Ok(())
}

/// Handle the `sync-all` command.
///
/// With `--validate` or `--autofix`, the source skill tree is validated for
/// the strictest of the targets before the first target is synced, and any
/// remaining error aborts the whole run with nothing written to any target.
/// `--autofix` repairs the source first (or, under `--dry-run`, checks what
/// the repair would leave) and then validates as `--validate` does.
pub(crate) fn handle_sync_all_command(args: SyncAllArgs) -> Result<()> {
    let targets: Vec<SyncSource> = match args.to {
        Some(t) => vec![t],
        None => args.from.other_targets(),
    };

    if args.validate || args.autofix {
        validate_sync_source(&args, &targets, &home_dir()?)?;
    }

    let multi_target = targets.len() > 1;
    let from = args.from;

    for target in targets {
        if multi_target {
            tracing::info!(
                from = %from.as_str(),
                to = %target.as_str(),
                "syncing target"
            );
        }

        // Skills go through the dedicated mirror for claude -> codex.
        if from.is_claude() && target.is_codex() && !args.dry_run {
            let home = home_dir()?;
            let skill_report = sync_skills_only_from_claude(
                &mirror_source_root(&home),
                &home.join(".codex/skills"),
                args.include_marketplace,
            )?;
            if let Some(warning) = codex_skills_feature_warning(&home.join(".codex/config.toml")) {
                eprintln!("{warning}");
            }
            tracing::info!(
                synced = skill_report.copied,
                unchanged = skill_report.skipped,
                "skills synced"
            );
        }

        let delivery = skrills_sync::skill_delivery(from.as_str(), target.as_str());
        let params = skrills_sync::SyncParams {
            from: Some(from.as_str().to_string()),
            dry_run: args.dry_run,
            sync_commands: true,
            skip_existing_commands: args.skip_existing_commands,
            sync_mcp_servers: true,
            sync_preferences: true,
            sync_skills: delivery.sync_skills,
            include_marketplace: args.include_marketplace,
            exclude_plugins: args.exclude_plugins.clone(),
            full_plugin_mirror: delivery.full_plugin_mirror,
            ..Default::default()
        };

        let report = run_sync_with_adapters(from, target, &params)?;

        tracing::info!(
            "{}{}",
            report.summary,
            skipped_commands_note(args.skip_existing_commands, &report)
        );
    }

    if args.dry_run {
        tracing::info!("(dry run - no changes made)");
    }
    Ok(())
}

/// The "kept target copy" line for `--skip-existing-commands`, or nothing.
pub(crate) fn skipped_commands_note(
    skip_existing_commands: bool,
    report: &skrills_sync::SyncReport,
) -> String {
    if skip_existing_commands && !report.commands.skipped.is_empty() {
        format!(
            "\nSkipped existing commands (kept target copy): {}",
            report
                .commands
                .skipped
                .iter()
                .map(|r| r.description())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        String::new()
    }
}

/// Handle the `sync-status` command.
pub(crate) fn handle_sync_status_command(from: SyncSource, to: Option<SyncSource>) -> Result<()> {
    let target = to.unwrap_or_else(|| from.default_target());
    // Same rule as sync-all, so the preview matches the run.
    let delivery = skrills_sync::skill_delivery(from.as_str(), target.as_str());

    let params = skrills_sync::SyncParams {
        from: Some(from.as_str().to_string()),
        dry_run: true,
        sync_commands: true,
        sync_mcp_servers: true,
        sync_preferences: true,
        sync_skills: delivery.sync_skills,
        full_plugin_mirror: delivery.full_plugin_mirror,
        ..Default::default()
    };

    tracing::info!(
        from = %from.as_str(),
        to = %target.as_str(),
        "sync direction"
    );

    let report = run_sync_with_adapters(from, target, &params)?;

    tracing::info!(
        commands = report.commands.written,
        mcp_servers = report.mcp_servers.written,
        preferences = report.preferences.written,
        "pending changes"
    );

    let source_root = source_skill_root(from, &home_dir()?);
    if source_root.exists() {
        // A walk error is counted and reported rather than dropped, so an
        // unreadable subdirectory does not make its skills look absent.
        let mut walk_errors: u64 = 0;
        let mut skill_count: usize = 0;
        for entry in walkdir::WalkDir::new(&source_root)
            .min_depth(1)
            .max_depth(6)
        {
            match entry {
                Ok(e) => {
                    if skrills_server::discovery::is_skill_file(&e) {
                        skill_count += 1;
                    }
                }
                Err(err) => {
                    walk_errors += 1;
                    tracing::warn!(
                        error = %err,
                        source_root = %source_root.display(),
                        "walk error while counting skills (entry skipped)"
                    );
                }
            }
        }
        if walk_errors > 0 {
            tracing::info!(skill_count, walk_errors, "skills found in source");
        } else {
            tracing::info!(skill_count, "skills found in source");
        }
    } else {
        tracing::info!("skills: 0 (source directory not found)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn codex_skills_feature_warning_names_the_config_it_could_not_write() {
        // A directory where the config file belongs makes every read of it
        // fail, which is the shape of a read-only or corrupted home.
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::create_dir_all(&config_path).expect("create config.toml as a directory");

        let warning = codex_skills_feature_warning(&config_path)
            .expect("an unreadable config should produce a warning");

        assert!(
            warning.contains("codex skills feature"),
            "warning should name the feature, got: {warning}"
        );
        assert!(
            warning.contains(&config_path.display().to_string()),
            "warning should name the config path, got: {warning}"
        );
    }

    #[test]
    fn codex_skills_feature_warning_is_silent_when_the_flag_can_be_written() {
        let temp = tempdir().expect("tempdir");
        let config_path = temp.path().join("config.toml");

        assert!(codex_skills_feature_warning(&config_path).is_none());
        assert!(
            std::fs::read_to_string(&config_path)
                .expect("config should have been written")
                .contains("skills = true"),
            "the feature flag should have been enabled"
        );
    }

    /// Restores the working directory even if the test returns early or panics.
    ///
    /// The directory is process-global, so the change is only safe while
    /// `env_guard` is held. Leaking it would move every later test in this
    /// binary into a deleted tempdir.
    struct CwdGuard {
        original: std::path::PathBuf,
    }

    impl CwdGuard {
        fn change_to(dir: &std::path::Path) -> Result<Self> {
            let original = std::env::current_dir()?;
            std::env::set_current_dir(dir)?;
            Ok(Self { original })
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    #[test]
    fn mirror_command_does_not_create_skills_mirror_dir() -> Result<()> {
        let _guard = skrills_test_utils::env_guard();

        let tmp = tempdir()?;
        let home = tmp.path();

        let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));

        // `handle_mirror_command` refreshes a relative AGENTS.md, so the test
        // has to run from the fake home rather than the repository.
        let _cwd_guard = CwdGuard::change_to(home)?;

        // Seed one skill and one agent in the Claude source tree.
        let claude_skill = home.join(".claude/skills/example-skill/SKILL.md");
        std::fs::create_dir_all(claude_skill.parent().unwrap())?;
        std::fs::write(&claude_skill, "example skill")?;

        let claude_agent = home.join(".claude/plugins/cache/tool/agents/helper.md");
        std::fs::create_dir_all(claude_agent.parent().unwrap())?;
        std::fs::write(&claude_agent, "agent content")?;

        // HOME and the working directory are restored by their guards on drop.
        handle_mirror_command(false, true, false)?;

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
