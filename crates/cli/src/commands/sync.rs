use super::sync_prompt::{is_cancelled, require_terminal, PromptDecider};
use super::validate::{run_validation, Autofix};
use crate::cli::{Platform, SyncEndpoint};
use anyhow::{anyhow, bail, Result};
use skrills_discovery::merge_extra_dirs;
use skrills_discovery::{SkillRoot, SkillSource};
use skrills_state::home_dir;
use skrills_sync::{mirror_source_root, sync_agents, sync_agents_only_from_claude};
use skrills_validate::ValidationTarget as VT;
use std::path::{Path, PathBuf};

/// Prints the skill phase's warnings, such as a `~/.codex/config.toml` the
/// skills feature flag could not be written to, which would otherwise leave
/// "copied: N" on screen while Codex loads nothing.
pub(crate) fn print_skill_warnings(report: &skrills_sync::SyncReport) {
    for warning in &report.skills.warnings {
        eprintln!("Warning: {warning}");
    }
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
    let report = skrills_sync::sync_claude_skills_to_codex(
        &mirror_source_root(&home),
        &home,
        include_marketplace,
        false,
    )?;
    print_skill_warnings(&report);
    println!(
        "copied: {}, skipped: {}",
        report.skills.written,
        report.skills.skipped.len()
    );
    Ok(())
}

pub(crate) fn handle_mirror_command(
    dry_run: bool,
    skip_existing_commands: bool,
    include_marketplace: bool,
) -> Result<()> {
    let home = home_dir()?;
    let claude_root = mirror_source_root(&home);
    // The agent copy, the skills and the orchestrator below are one mirror;
    // a shared session makes one rollback undo all of it.
    let _snapshot = if dry_run {
        None
    } else {
        skrills_sync::snapshot::begin_default()
    };
    if !skip_existing_commands {
        eprintln!(
            "Warning: mirroring commands into ~/.codex/prompts will overwrite prompts with the same name unless --skip-existing-commands is used."
        );
    }
    // Mirror agents into ~/.codex/agents. The agent copier has no preview
    // mode, so a dry run skips it rather than writing.
    let agent_report = if dry_run {
        skrills_sync::MirrorReport::default()
    } else {
        sync_agents_only_from_claude(
            &claude_root,
            &home.join(".codex/agents"),
            include_marketplace,
        )?
    };
    // Skills go to ~/.agents/skills, where Codex loads them.
    let codex_report = skrills_sync::sync_claude_skills_to_codex(
        &claude_root,
        &home,
        include_marketplace,
        dry_run,
    )?;
    print_skill_warnings(&codex_report);
    // Mirror commands/mcp/prefs
    let source = skrills_sync::ClaudeAdapter::new()?;
    let target = skrills_sync::CodexAdapter::new()?;
    let orch = skrills_sync::SyncOrchestrator::new(source, target).with_default_sync_state();
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
    // Refresh Codex's global AGENTS.md with skills and agents (mirror roots
    // now populated). Never the working directory's AGENTS.md: that is
    // whatever repository `mirror` happens to run from.
    if !dry_run {
        handle_sync_agents_command(Some(home.join(".codex/AGENTS.md")), vec![])?;
    }

    println!(
        "mirror complete: agents copied {}, skipped {}; skills (codex) copied {}, skipped {}; commands written {}, skipped {}; prefs {}, mcp {}{}",
        agent_report.copied,
        agent_report.skipped,
        codex_report.skills.written,
        codex_report.skills.skipped.len(),
        sync_report.commands.written,
        sync_report.commands.skipped.len(),
        sync_report.preferences.written,
        sync_report.mcp_servers.written,
        if dry_run {
            " (dry run: nothing written; agents and ~/.codex/AGENTS.md not checked)"
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
    from: impl Into<SyncEndpoint>,
    to: impl Into<SyncEndpoint>,
    params: &skrills_sync::SyncParams,
) -> Result<skrills_sync::SyncReport> {
    let (from, to) = (from.into(), to.into());
    if from == to {
        return Err(anyhow!(
            "Source and target cannot be the same: {}",
            from.as_str()
        ));
    }
    // Adapter creation, built-in or configured, is the library's job.
    skrills_sync::orchestrator::sync_between_endpoints(&from, &to, params)
}

/// [`run_sync_with_adapters`] that asks about each change at the terminal.
fn run_interactive_sync(
    from: &SyncEndpoint,
    to: &SyncEndpoint,
    params: &skrills_sync::SyncParams,
) -> Result<skrills_sync::SyncReport> {
    if from == to {
        return Err(anyhow!(
            "Source and target cannot be the same: {}",
            from.as_str()
        ));
    }
    skrills_sync::orchestrator::sync_between_endpoints_with_decider(
        from,
        to,
        params,
        std::sync::Arc::new(PromptDecider),
    )
}

/// The directory a CLI keeps its skills in, as `sync-status` counts them.
pub(crate) fn source_skill_root(from: Platform, home: &Path) -> PathBuf {
    use skrills_sync::adapters::traits::AgentAdapter;
    match from {
        Platform::Claude => mirror_source_root(home),
        Platform::Codex => home.join(".codex/skills"),
        Platform::Copilot => skrills_sync::CopilotAdapter::new()
            .map(|a| a.config_root().join("skills"))
            .unwrap_or_else(|_| home.join(".copilot/skills")),
        Platform::Cursor => skrills_sync::CursorAdapter::new()
            .map(|a| a.config_root().join("skills"))
            .unwrap_or_else(|_| home.join(".cursor/skills")),
    }
}

/// The directory a configured adapter keeps its skills in, or `None` when its
/// file configures no `skills_dir`. Built-ins resolve as [`source_skill_root`].
pub(crate) fn endpoint_skill_root(from: &SyncEndpoint, home: &Path) -> Result<Option<PathBuf>> {
    match from {
        SyncEndpoint::Builtin(platform) => Ok(Some(source_skill_root(*platform, home))),
        SyncEndpoint::Custom(name) => Ok(skrills_sync::load_adapter(name, home)?.skills_dir),
    }
}

/// The skill roots `sync-all` reads from `from`, for validation.
///
/// For Claude this is the user skills and the plugin cache under the mirror
/// source, plus the marketplaces when `include_marketplace` is set: the same
/// trees the skill sync walks. The rest of `~/.claude` (sessions, settings)
/// holds no skills. A configured adapter contributes its `skills_dir`.
fn source_validation_roots(
    from: &SyncEndpoint,
    home: &Path,
    include_marketplace: bool,
) -> Result<Vec<SkillRoot>> {
    let mut dirs: Vec<PathBuf> = match (from, endpoint_skill_root(from, home)?) {
        (SyncEndpoint::Builtin(Platform::Claude), Some(root)) => {
            let mut dirs = vec![root.join("skills"), root.join("plugins/cache")];
            if include_marketplace {
                dirs.push(root.join("plugins/marketplaces"));
            }
            dirs
        }
        (_, root) => root.into_iter().collect(),
    };
    Ok(dirs
        .drain(..)
        .enumerate()
        .map(|(i, root)| SkillRoot {
            root,
            source: SkillSource::Extra(i as u32),
        })
        .collect())
}

/// The strictest validation target any of `targets` needs.
fn validation_target_for(targets: &[SyncEndpoint]) -> VT {
    let codex = targets.contains(&SyncEndpoint::Builtin(Platform::Codex));
    let copilot = targets.contains(&SyncEndpoint::Builtin(Platform::Copilot));
    match (codex, copilot) {
        (true, true) => VT::All,
        (true, false) => VT::Codex,
        (false, true) => VT::Copilot,
        // Claude, Cursor and configured adapters accept what Claude accepts.
        (false, false) => VT::Claude,
    }
}

/// Options for `sync-all`, one field per flag.
#[derive(Debug, Clone)]
pub(crate) struct SyncAllArgs {
    pub from: SyncEndpoint,
    pub to: Option<SyncEndpoint>,
    pub dry_run: bool,
    pub skip_existing_commands: bool,
    pub include_marketplace: bool,
    pub exclude_plugins: Vec<String>,
    pub validate: bool,
    pub autofix: bool,
    pub keep_versions: bool,
    pub interactive: bool,
}

/// Validates (and with `--autofix`, repairs) the source skills before any
/// target is touched. Returns an error, and so writes nothing, when an error
/// remains.
fn validate_sync_source(args: &SyncAllArgs, targets: &[SyncEndpoint], home: &Path) -> Result<()> {
    let roots = source_validation_roots(&args.from, home, args.include_marketplace)?;
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
    // Before validation: `--autofix` writes to the source, and "nothing was
    // written" has to hold when the prompts cannot be answered.
    let will_prompt = args.interactive && !args.dry_run;
    {
        use std::io::IsTerminal;
        require_terminal(
            will_prompt,
            std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        )?;
    }
    let targets: Vec<SyncEndpoint> = match args.to.clone() {
        Some(t) => vec![t],
        None => args.from.other_targets(),
    };

    if args.validate || args.autofix {
        validate_sync_source(&args, &targets, &home_dir()?)?;
    }

    let multi_target = targets.len() > 1;
    let from = args.from.clone();

    // One snapshot for the invocation, not one per target: a rollback then
    // undoes the whole `sync-all` instead of only its last target. Each
    // target's orchestrator joins this session.
    let _snapshot = if args.dry_run {
        None
    } else {
        skrills_sync::snapshot::begin_default()
    };

    sync_each_target(&targets, |target| {
        if multi_target {
            tracing::info!(
                from = %from.as_str(),
                to = %target.as_str(),
                "syncing target"
            );
        }

        let delivery = skrills_sync::skill_delivery_for(&from, target);
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
            keep_versions: args.keep_versions,
            interactive: args.interactive,
            ..Default::default()
        };

        let report = if will_prompt {
            run_interactive_sync(&from, target, &params)?
        } else {
            run_sync_with_adapters(from.clone(), target.clone(), &params)?
        };

        tracing::info!(
            "{}{}",
            report.summary,
            skipped_commands_note(args.skip_existing_commands, &report)
        );
        Ok(())
    })?;

    if args.dry_run {
        tracing::info!("(dry run - no changes made)");
    }
    Ok(())
}

/// Runs `sync` for every target, carrying on past a failure, and returns an
/// error naming which targets succeeded and which failed. Stopping at the
/// first failure left later targets unsynced with no word on the earlier ones.
fn sync_each_target(
    targets: &[SyncEndpoint],
    mut sync: impl FnMut(&SyncEndpoint) -> Result<()>,
) -> Result<()> {
    let mut outcomes: Vec<(&SyncEndpoint, Result<()>)> = Vec::new();
    for target in targets {
        let outcome = sync(target);
        // A cancelled prompt ends the run, not just this target's questions.
        let cancelled = outcome.as_ref().err().is_some_and(is_cancelled);
        outcomes.push((target, outcome));
        if cancelled {
            break;
        }
    }
    if outcomes.iter().all(|(_, r)| r.is_ok()) {
        return Ok(());
    }
    let summary: Vec<String> = outcomes
        .iter()
        .map(|(target, outcome)| match outcome {
            Ok(()) => format!("{}: ok", target.as_str()),
            Err(e) => format!("{}: failed: {e:#}", target.as_str()),
        })
        .collect();
    bail!(
        "sync-all failed for some targets:\n  {}",
        summary.join("\n  ")
    )
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

/// Handle the `sync-rollback` command.
pub(crate) fn handle_sync_rollback_command(
    list: bool,
    snapshot: Option<String>,
    force: bool,
) -> Result<()> {
    let root = skrills_sync::snapshot::default_root()
        .ok_or_else(|| anyhow!("no home directory, so ~/.skrills/snapshots cannot be located"))?;
    if list {
        print!(
            "{}",
            render_snapshot_list(&root, &skrills_sync::snapshot::list(&root)?)
        );
        return Ok(());
    }
    let report = if force {
        skrills_sync::snapshot::force_rollback(&root, snapshot.as_deref())?
    } else {
        skrills_sync::snapshot::rollback(&root, snapshot.as_deref())?
    };
    println!(
        "Rolled back {}: restored {} file(s), deleted {} file(s) and {} director(ies) the sync had created",
        report.snapshot, report.restored, report.removed, report.dirs_removed
    );
    for path in &report.changed_since_sync {
        println!(
            "Changed since the sync, not restored (--force restores it): {}",
            path.display()
        );
    }
    for path in &report.not_restorable {
        println!(
            "Not restored (a special file or link this platform cannot recreate): {}",
            path.display()
        );
    }
    for dir in &report.left_in_place {
        println!(
            "Left in place (no longer empty, or replaced): {}",
            dir.display()
        );
    }
    Ok(())
}

/// The `sync-rollback --list` table, newest first.
fn render_snapshot_list(root: &Path, snapshots: &[skrills_sync::snapshot::SnapshotInfo]) -> String {
    if snapshots.is_empty() {
        return format!("No snapshots in {}\n", root.display());
    }
    snapshots
        .iter()
        .map(|info| {
            format!(
                "{}  {}  {} files\n",
                info.name,
                skrills_sync::snapshot::display_time(info.taken_at),
                info.entries
            )
        })
        .collect()
}

/// Handle the `sync-status` command.
pub(crate) fn handle_sync_status_command(
    from: SyncEndpoint,
    to: Option<SyncEndpoint>,
) -> Result<()> {
    let target = to.unwrap_or_else(|| from.default_target());
    // Same rule as sync-all, so the preview matches the run.
    let delivery = skrills_sync::skill_delivery_for(&from, &target);

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

    let report = run_sync_with_adapters(from.clone(), target, &params)?;

    tracing::info!(
        commands = report.commands.written,
        mcp_servers = report.mcp_servers.written,
        preferences = report.preferences.written,
        "pending changes"
    );

    let source_root = endpoint_skill_root(&from, &home_dir()?)?;
    if let Some(source_root) = source_root.filter(|root| root.exists()) {
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
                    if skrills_discovery::is_skill_file(&e) {
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

    fn codex() -> SyncEndpoint {
        SyncEndpoint::Builtin(Platform::Codex)
    }

    fn copilot() -> SyncEndpoint {
        SyncEndpoint::Builtin(Platform::Copilot)
    }

    fn cursor() -> SyncEndpoint {
        SyncEndpoint::Builtin(Platform::Cursor)
    }

    #[test]
    fn a_cancelled_prompt_stops_the_remaining_targets() {
        let mut attempted = Vec::new();

        let err = sync_each_target(&[codex(), copilot()], |target| {
            attempted.push(target.clone());
            Err(skrills_sync::SyncError::InteractiveAborted.into())
        })
        .unwrap_err();

        assert_eq!(attempted, vec![codex()]);
        assert!(format!("{err}").contains("codex: failed"), "{err}");
    }

    #[test]
    fn sync_all_continues_past_a_failed_target_and_names_each_outcome() {
        let mut attempted = Vec::new();

        let err = sync_each_target(&[codex(), copilot(), cursor()], |target| {
            attempted.push(target.clone());
            if *target == copilot() {
                bail!("copilot dir is not writable")
            }
            Ok(())
        })
        .unwrap_err()
        .to_string();

        assert_eq!(attempted, vec![codex(), copilot(), cursor()]);
        assert!(err.contains("codex: ok"), "{err}");
        assert!(
            err.contains("copilot: failed: copilot dir is not writable"),
            "{err}"
        );
        assert!(err.contains("cursor: ok"), "{err}");
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
            home.join(".agents/skills/example-skill/SKILL.md").exists(),
            "expected skill copied into ~/.agents/skills, where Codex loads it"
        );
        assert!(!home.join(".codex/skills").exists());
        assert!(
            home.join(".codex/agents/plugins/cache/tool/agents/helper.md")
                .exists(),
            "expected agent copied into ~/.codex/agents"
        );
        Ok(())
    }

    /// Seeds one Claude skill and one Claude agent under `home`.
    fn seed_claude_skill_and_agent(home: &std::path::Path) -> Result<()> {
        let claude_skill = home.join(".claude/skills/example-skill/SKILL.md");
        std::fs::create_dir_all(claude_skill.parent().unwrap())?;
        std::fs::write(
            &claude_skill,
            "---\nname: example-skill\ndescription: An example skill.\n---\nBody\n",
        )?;
        let claude_agent = home.join(".claude/plugins/cache/tool/agents/helper.md");
        std::fs::create_dir_all(claude_agent.parent().unwrap())?;
        std::fs::write(&claude_agent, "agent content")?;
        Ok(())
    }

    #[test]
    fn mirror_dry_run_writes_no_skills_agents_config_or_agents_md() -> Result<()> {
        let _guard = skrills_test_utils::env_guard();
        let tmp = tempdir()?;
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        std::fs::create_dir_all(&cwd)?;
        let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));
        let _cwd_guard = CwdGuard::change_to(&cwd)?;
        seed_claude_skill_and_agent(&home)?;

        handle_mirror_command(true, true, false)?;

        assert!(!home.join(".agents").exists(), "dry run copied skills");
        assert!(
            !home.join(".codex/agents").exists(),
            "dry run copied agents"
        );
        assert!(
            !home.join(".codex/config.toml").exists(),
            "dry run edited config.toml"
        );
        assert!(
            !home.join(".codex/AGENTS.md").exists(),
            "dry run wrote AGENTS.md"
        );
        assert!(!cwd.join("AGENTS.md").exists(), "dry run wrote ./AGENTS.md");
        Ok(())
    }

    #[test]
    fn mirror_refreshes_codex_home_agents_md_not_the_working_directory() -> Result<()> {
        let _guard = skrills_test_utils::env_guard();
        let tmp = tempdir()?;
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("repo");
        std::fs::create_dir_all(&cwd)?;
        let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));
        let _cwd_guard = CwdGuard::change_to(&cwd)?;
        seed_claude_skill_and_agent(&home)?;

        handle_mirror_command(false, true, false)?;

        assert!(
            !cwd.join("AGENTS.md").exists(),
            "mirror rewrote AGENTS.md in the working directory"
        );
        assert!(home.join(".codex/AGENTS.md").exists());
        Ok(())
    }

    const SKILLS_AND_COMMANDS: &str = "name = \"windsurf\"\nroot = \"~/.windsurf\"\nskills_dir = \"skills\"\ncommands_dir = \"workflows\"\n";

    fn windsurf() -> SyncEndpoint {
        SyncEndpoint::Custom("windsurf".to_string())
    }

    fn sync_all_args(from: SyncEndpoint, to: Option<SyncEndpoint>) -> SyncAllArgs {
        SyncAllArgs {
            from,
            to,
            dry_run: false,
            skip_existing_commands: false,
            include_marketplace: false,
            exclude_plugins: Vec::new(),
            validate: false,
            autofix: false,
            keep_versions: false,
            interactive: false,
        }
    }

    fn put(path: &std::path::Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// Seeds a Claude skill and command, and the windsurf adapter file, under `home`.
    fn seed_home_with_windsurf(home: &std::path::Path) {
        put(
            &home.join(".claude/skills/alpha/SKILL.md"),
            "---\nname: alpha\ndescription: An example skill.\n---\nBody\n",
        );
        put(&home.join(".claude/commands/hello.md"), "# Hello\n");
        put(
            &skrills_sync::adapters_dir(home).join("windsurf.toml"),
            SKILLS_AND_COMMANDS,
        );
    }

    #[test]
    fn sync_all_into_a_custom_adapter_writes_skills_and_commands_and_rolls_back() -> Result<()> {
        let _guard = skrills_test_utils::env_guard();
        let tmp = tempdir()?;
        let home = tmp.path();
        let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));
        seed_home_with_windsurf(home);

        handle_sync_all_command(sync_all_args(
            SyncEndpoint::Builtin(Platform::Claude),
            Some(windsurf()),
        ))?;

        let root = home.join(".windsurf");
        assert!(root.join("skills/alpha/SKILL.md").is_file());
        assert_eq!(
            std::fs::read_to_string(root.join("workflows/hello.md"))?,
            "# Hello\n"
        );

        handle_sync_rollback_command(false, None, false)?;

        assert!(!root.join("skills/alpha/SKILL.md").exists());
        assert!(!root.join("workflows/hello.md").exists());
        Ok(())
    }

    #[test]
    fn sync_all_to_an_unknown_name_lists_the_built_ins_and_the_configured_adapters() -> Result<()> {
        let _guard = skrills_test_utils::env_guard();
        let tmp = tempdir()?;
        let home = tmp.path();
        let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));
        seed_home_with_windsurf(home);

        let err = handle_sync_all_command(sync_all_args(
            SyncEndpoint::Builtin(Platform::Claude),
            Some(SyncEndpoint::Custom("nope".to_string())),
        ))
        .unwrap_err();

        let shown = format!("{err:#}");
        for expected in ["nope", "claude", "codex", "copilot", "cursor", "windsurf"] {
            assert!(shown.contains(expected), "{expected} missing: {shown}");
        }
        assert!(!home.join(".windsurf").exists());
        Ok(())
    }

    #[test]
    fn sync_all_from_a_custom_adapter_validates_its_skills_and_syncs_them() -> Result<()> {
        let _guard = skrills_test_utils::env_guard();
        let tmp = tempdir()?;
        let home = tmp.path();
        let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));
        seed_home_with_windsurf(home);
        put(
            &home.join(".windsurf/skills/beta/SKILL.md"),
            "---\nname: beta\ndescription: From windsurf.\n---\nBody\n",
        );
        let mut args = sync_all_args(windsurf(), Some(SyncEndpoint::Builtin(Platform::Claude)));
        args.validate = true;

        handle_sync_all_command(args)?;

        assert!(home.join(".claude/skills/beta/SKILL.md").is_file());
        Ok(())
    }

    #[test]
    fn sync_all_from_a_custom_adapter_aborts_when_its_skills_fail_validation() -> Result<()> {
        let _guard = skrills_test_utils::env_guard();
        let tmp = tempdir()?;
        let home = tmp.path();
        let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));
        seed_home_with_windsurf(home);
        put(
            &home.join(".windsurf/skills/broken/SKILL.md"),
            "no frontmatter at all\n",
        );
        let mut args = sync_all_args(windsurf(), Some(SyncEndpoint::Builtin(Platform::Codex)));
        args.validate = true;

        let err = handle_sync_all_command(args).unwrap_err();

        assert!(err.to_string().contains("do not validate"), "{err:#}");
        assert!(!home.join(".agents/skills/broken").exists());
        Ok(())
    }

    #[test]
    fn sync_status_reports_for_a_custom_endpoint_with_and_without_a_skills_dir() -> Result<()> {
        let _guard = skrills_test_utils::env_guard();
        let tmp = tempdir()?;
        let home = tmp.path();
        let _home_guard = skrills_test_utils::set_env_var("HOME", Some(home.to_str().unwrap()));
        seed_home_with_windsurf(home);
        put(
            &skrills_sync::adapters_dir(home).join("bare.toml"),
            "name = \"bare\"\nroot = \"~/.bare\"\n",
        );

        handle_sync_status_command(windsurf(), None)?;
        handle_sync_status_command(
            SyncEndpoint::Custom("bare".to_string()),
            Some(SyncEndpoint::Builtin(Platform::Claude)),
        )?;

        Ok(())
    }

    #[test]
    fn the_strictest_validation_target_ignores_custom_endpoints() {
        assert_eq!(validation_target_for(&[windsurf(), codex()]), VT::Codex);
        assert_eq!(validation_target_for(&[windsurf()]), VT::Claude);
    }
}
