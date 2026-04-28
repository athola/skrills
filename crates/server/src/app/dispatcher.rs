//! CLI dispatch — the `skrills` binary entry point.
//!
//! Split out of `app/mod.rs` (T3.1 of the v0.8.0 refinement plan).
//! This file owns nothing except the routing of parsed [`Cli`]
//! commands to their respective handlers in [`crate::commands`]. The
//! handlers themselves live in their own submodules; this is pure
//! dispatch.
//!
//! Rule of thumb: **no business logic here**. If you find yourself
//! adding more than a `Commands::Foo { x, y } => handle_foo(x, y)`
//! line, the logic belongs in `crate::commands::foo` (or a new
//! handler under `crate::commands`).

use crate::cli::{CertAction, Cli, Commands, SyncSource};
use crate::commands::{
    handle_agent_command, handle_analyze_command, handle_analyze_project_context_command,
    handle_cert_install_command, handle_cert_renew_command, handle_cert_status_command,
    handle_create_skill_command, handle_export_analytics_command, handle_import_analytics_command,
    handle_metrics_command, handle_mirror_command, handle_multi_cli_agent_command,
    handle_pre_commit_validate_command, handle_recommend_command,
    handle_recommend_skills_smart_command, handle_resolve_dependencies_command,
    handle_search_skills_command, handle_search_skills_github_command, handle_serve_command,
    handle_setup_command, handle_skill_catalog_command, handle_skill_deprecate_command,
    handle_skill_diff_command, handle_skill_import_command, handle_skill_profile_command,
    handle_skill_rollback_command, handle_skill_score_command, handle_skill_usage_report_command,
    handle_suggest_new_skills_command, handle_sync_agents_command, handle_sync_command,
    handle_sync_pull_command, handle_validate_command,
};
use crate::discovery::merge_extra_dirs;
use crate::doctor::doctor_report;
use crate::signals::ignore_sigchld;
use crate::sync::mirror_source_root;
use crate::tui::tui_flow;
use anyhow::{anyhow, Result};
use clap::Parser;
use skrills_state::home_dir;

/// Sync helper used by Sync* command branches.
///
/// Visible at `crate::app::run_sync_with_adapters` via the
/// re-export in `app/mod.rs`; tests under `app/tests/sync.rs` rely
/// on that path.
pub(crate) fn run_sync_with_adapters(
    from: SyncSource,
    to: SyncSource,
    params: &skrills_sync::SyncParams,
) -> Result<skrills_sync::SyncReport> {
    // Same source and target - error
    if from == to {
        return Err(anyhow!(
            "Source and target cannot be the same: {}",
            from.as_str()
        ));
    }

    // Delegate to sync_between which handles adapter creation for all platforms
    skrills_sync::orchestrator::sync_between(from.as_str(), to.as_str(), params)
}

/// Main application entry point.
pub fn run() -> Result<()> {
    ignore_sigchld()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Load config file and apply settings to env vars before CLI parsing.
    // This ensures precedence: CLI > ENV > config file.
    crate::config::apply_config_to_env();

    let cli = Cli::parse();

    // Check for first-run (only for user-facing commands, not for `serve` which is called by MCP)
    // Also skip for batch/non-interactive commands like sync-all
    let command_ref = cli.command.as_ref();
    let is_serve = matches!(command_ref, Some(Commands::Serve { .. }) | None);
    let is_setup = matches!(command_ref, Some(Commands::Setup { .. }));
    let is_batch = matches!(command_ref, Some(Commands::SyncAll { .. }));

    if !is_serve && !is_setup && !is_batch {
        if let Ok(true) = crate::setup::is_first_run() {
            if let Ok(true) = crate::setup::prompt_first_run_setup() {
                // Run interactive setup
                let config = crate::setup::interactive_setup(
                    None, None, false, false, false, false, false, None,
                )?;
                crate::setup::run_setup(config)?;
                println!(
                    "\nYou can now use skrills. Run your command again or explore 'skrills --help'"
                );
                return Ok(());
            } else {
                println!("Setup skipped. Run 'skrills setup' when ready.");
            }
        }
    }

    match cli.command.unwrap_or(Commands::Serve {
        skill_dirs: Vec::new(),
        cache_ttl_ms: None,
        trace_wire: false,
        #[cfg(feature = "watch")]
        watch: false,
        http: None,
        list_tools: false,
        auth_token: None,
        tls_cert: None,
        tls_key: None,
        cors_origins: Vec::new(),
        tls_auto: false,
        open: false,
    }) {
        Commands::Serve {
            skill_dirs,
            cache_ttl_ms,
            trace_wire,
            #[cfg(feature = "watch")]
            watch,
            http,
            list_tools,
            auth_token,
            tls_cert,
            tls_key,
            cors_origins,
            tls_auto,
            open,
        } => handle_serve_command(
            skill_dirs,
            cache_ttl_ms,
            trace_wire,
            #[cfg(feature = "watch")]
            watch,
            http,
            list_tools,
            auth_token,
            tls_cert,
            tls_key,
            cors_origins,
            tls_auto,
            open,
        ),
        Commands::Mirror {
            dry_run,
            skip_existing_commands,
            include_marketplace,
        } => handle_mirror_command(dry_run, skip_existing_commands, include_marketplace),
        Commands::Agent {
            agent,
            skill_dirs,
            dry_run,
        } => handle_agent_command(agent, skill_dirs, dry_run),
        Commands::MultiCliAgent {
            agent,
            backend,
            skill_dirs,
            dry_run,
        } => handle_multi_cli_agent_command(agent, backend, skill_dirs, dry_run),
        Commands::SyncAgents { path, skill_dirs } => handle_sync_agents_command(path, skill_dirs),
        Commands::Sync {
            include_marketplace,
        } => handle_sync_command(include_marketplace),
        Commands::SyncCommands {
            from,
            to,
            dry_run,
            skip_existing_commands,
            include_marketplace,
        } => {
            use skrills_sync::SyncParams;

            let target = to.unwrap_or_else(|| from.default_target());

            if !skip_existing_commands {
                eprintln!(
                    "Warning: syncing commands will overwrite existing files. Use --skip-existing-commands to keep existing copies."
                );
            }

            let params = SyncParams {
                from: Some(from.as_str().to_string()),
                dry_run,
                sync_commands: true,
                skip_existing_commands,
                sync_mcp_servers: false,
                sync_preferences: false,
                sync_skills: false,
                include_marketplace,
                ..Default::default()
            };

            let report = run_sync_with_adapters(from, target, &params)?;

            println!(
                "{}{}",
                report.summary,
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
            );
            if dry_run {
                println!("(dry run - no changes made)");
            }
            Ok(())
        }
        Commands::SyncMcpServers { from, to, dry_run } => {
            use skrills_sync::SyncParams;

            let target = to.unwrap_or_else(|| from.default_target());

            let params = SyncParams {
                from: Some(from.as_str().to_string()),
                dry_run,
                sync_commands: false,
                sync_mcp_servers: true,
                sync_preferences: false,
                sync_skills: false,
                ..Default::default()
            };

            let report = run_sync_with_adapters(from, target, &params)?;

            println!("{}", report.summary);
            if dry_run {
                println!("(dry run - no changes made)");
            }
            Ok(())
        }
        Commands::SyncPreferences { from, to, dry_run } => {
            use skrills_sync::SyncParams;

            let target = to.unwrap_or_else(|| from.default_target());

            let params = SyncParams {
                from: Some(from.as_str().to_string()),
                dry_run,
                sync_commands: false,
                sync_mcp_servers: false,
                sync_preferences: true,
                sync_skills: false,
                ..Default::default()
            };

            let report = run_sync_with_adapters(from, target, &params)?;

            println!("{}", report.summary);
            if dry_run {
                println!("(dry run - no changes made)");
            }
            Ok(())
        }
        Commands::SyncAll {
            from,
            to,
            dry_run,
            skip_existing_commands,
            include_marketplace,
            exclude_plugins,
            validate: _validate,
            autofix: _autofix,
        } => {
            use skrills_sync::SyncParams;

            // Determine targets: explicit --to or all other CLIs
            let targets: Vec<SyncSource> = match to {
                Some(t) => vec![t],
                None => from.other_targets(),
            };

            let multi_target = targets.len() > 1;

            for target in targets {
                if multi_target {
                    println!("\n=== Syncing {} → {} ===", from.as_str(), target.as_str());
                }

                // First sync skills using existing mechanism (only for claude→codex)
                if from.is_claude() && target.is_codex() && !dry_run {
                    let home = home_dir()?;
                    let claude_root = mirror_source_root(&home);
                    let codex_skills_root = home.join(".codex/skills");
                    let skill_report = crate::sync::sync_skills_only_from_claude(
                        &claude_root,
                        &codex_skills_root,
                        include_marketplace,
                    )?;
                    let _ = crate::setup::ensure_codex_skills_feature_enabled(
                        &home.join(".codex/config.toml"),
                    );
                    println!(
                        "Skills: {} synced, {} unchanged",
                        skill_report.copied, skill_report.skipped
                    );
                }

                // Skip skills sync for Claude→Codex (handled above with special logic).
                // For →Cursor: skip flat skills copy — Cursor discovers skills from
                // its own plugins/cache/ which is synced via plugin_assets.
                let sync_skills = !(target.is_cursor() || from.is_claude() && target.is_codex());
                // Cursor needs a full plugin mirror (including skills + manifests)
                // since it has its own plugin cache at ~/.cursor/plugins/cache/
                let full_plugin_mirror = target.is_cursor();
                let params = SyncParams {
                    from: Some(from.as_str().to_string()),
                    dry_run,
                    sync_commands: true,
                    skip_existing_commands,
                    sync_mcp_servers: true,
                    sync_preferences: true,
                    sync_skills,
                    include_marketplace,
                    exclude_plugins: exclude_plugins.clone(),
                    full_plugin_mirror,
                    ..Default::default()
                };

                let report = run_sync_with_adapters(from, target, &params)?;

                println!(
                    "{}{}",
                    report.summary,
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
                );
            }

            if dry_run {
                println!("\n(dry run - no changes made)");
            }
            Ok(())
        }
        Commands::SyncStatus { from, to } => {
            use skrills_sync::SyncParams;

            let target = to.unwrap_or_else(|| from.default_target());
            // Only skip skills sync for Claude→Codex (it has special handling elsewhere)
            let sync_skills = !(from.is_claude() && target.is_codex());

            let params = SyncParams {
                from: Some(from.as_str().to_string()),
                dry_run: true,
                sync_commands: true,
                sync_mcp_servers: true,
                sync_preferences: true,
                sync_skills,
                ..Default::default()
            };

            println!("Sync direction: {} → {}", from.as_str(), target.as_str());

            let report = run_sync_with_adapters(from, target, &params)?;

            println!("\nPending changes:");
            println!("  Commands: {} would sync", report.commands.written);
            println!("  MCP Servers: {} would sync", report.mcp_servers.written);
            println!("  Preferences: {} would sync", report.preferences.written);

            // Count skills
            let home = home_dir()?;
            let source_root = match from {
                SyncSource::Claude => mirror_source_root(&home),
                SyncSource::Codex => home.join(".codex/skills"),
                SyncSource::Copilot => {
                    use skrills_sync::adapters::traits::AgentAdapter;
                    use skrills_sync::CopilotAdapter;
                    CopilotAdapter::new()
                        .map(|a| a.config_root().join("skills"))
                        .unwrap_or_else(|_| home.join(".copilot/skills"))
                }
                SyncSource::Cursor => {
                    use skrills_sync::adapters::traits::AgentAdapter;
                    use skrills_sync::CursorAdapter;
                    CursorAdapter::new()
                        .map(|a| a.config_root().join("skills"))
                        .unwrap_or_else(|_| home.join(".cursor/skills"))
                }
            };
            if source_root.exists() {
                let skill_count = walkdir::WalkDir::new(&source_root)
                    .min_depth(1)
                    .max_depth(6)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(crate::discovery::is_skill_file)
                    .count();
                println!("  Skills: {} found in source", skill_count);
            } else {
                println!("  Skills: 0 (source directory not found)");
            }

            Ok(())
        }
        Commands::Doctor => doctor_report(),
        Commands::Tui { skill_dirs } => tui_flow(&merge_extra_dirs(&skill_dirs)),
        #[cfg(feature = "dashboard")]
        Commands::Dashboard { skill_dirs } => {
            let dashboard = skrills_dashboard::Dashboard::new(skill_dirs)?;
            tokio::runtime::Runtime::new()?.block_on(dashboard.run())
        }
        #[cfg(not(feature = "dashboard"))]
        Commands::Dashboard { .. } => {
            eprintln!("Dashboard feature not enabled. Rebuild with --features dashboard");
            Ok(())
        }
        Commands::Setup {
            client,
            bin_dir,
            reinstall,
            uninstall,
            add,
            yes,
            universal,
            mirror_source,
        } => handle_setup_command(
            client,
            bin_dir,
            reinstall,
            uninstall,
            add,
            yes,
            universal,
            mirror_source,
        ),
        Commands::Validate {
            skill_dirs,
            target,
            autofix,
            backup,
            format,
            errors_only,
            #[cfg(feature = "watch")]
                watch: _watch,
            #[cfg(feature = "watch")]
                debounce_ms: _debounce_ms,
        } => {
            // TODO(#208): when watch feature is enabled, dispatch to watch loop
            // if _watch { return handle_validate_watch(...); }
            handle_validate_command(skill_dirs, target, autofix, backup, format, errors_only)
        }
        Commands::Analyze {
            skill_dirs,
            format,
            min_tokens,
            suggestions,
        } => handle_analyze_command(skill_dirs, format, min_tokens, suggestions),
        Commands::Metrics {
            skill_dirs,
            format,
            include_validation,
        } => handle_metrics_command(skill_dirs, format, include_validation),
        Commands::Recommend {
            uri,
            skill_dirs,
            format,
            limit,
            include_quality,
        } => handle_recommend_command(uri, skill_dirs, format, limit, include_quality),
        Commands::ResolveDependencies {
            uri,
            skill_dirs,
            direction,
            transitive,
            format,
        } => handle_resolve_dependencies_command(uri, skill_dirs, direction, transitive, format),
        Commands::RecommendSkillsSmart {
            uri,
            prompt,
            project_dir,
            limit,
            include_usage,
            include_context,
            auto_persist,
            format,
            skill_dirs,
        } => handle_recommend_skills_smart_command(
            uri,
            prompt,
            project_dir,
            limit,
            include_usage,
            include_context,
            auto_persist,
            format,
            skill_dirs,
        ),
        Commands::AnalyzeProjectContext {
            project_dir,
            include_git,
            commit_limit,
            format,
        } => handle_analyze_project_context_command(project_dir, include_git, commit_limit, format),
        Commands::SuggestNewSkills {
            project_dir,
            focus_areas,
            format,
            skill_dirs,
        } => handle_suggest_new_skills_command(project_dir, focus_areas, format, skill_dirs),
        Commands::CreateSkill {
            name,
            description,
            method,
            target_dir,
            project_dir,
            dry_run,
            format,
        } => handle_create_skill_command(
            name,
            description,
            method,
            target_dir,
            project_dir,
            dry_run,
            format,
        ),
        Commands::SearchSkillsGithub {
            query,
            limit,
            format,
        } => handle_search_skills_github_command(query, limit, format),
        Commands::SearchSkills {
            query,
            threshold,
            limit,
            include_description,
            skill_dirs,
            format,
        } => handle_search_skills_command(
            query,
            threshold,
            limit,
            include_description,
            skill_dirs,
            format,
        ),
        Commands::ExportAnalytics {
            output,
            force_rebuild,
            format,
        } => handle_export_analytics_command(output, force_rebuild, format),
        Commands::ImportAnalytics { input, overwrite } => {
            handle_import_analytics_command(input, overwrite)
        }
        Commands::SkillDiff {
            name,
            format,
            context,
        } => handle_skill_diff_command(name, format, context),
        Commands::SkillDeprecate {
            name,
            message,
            replacement,
            skill_dirs,
            format,
        } => handle_skill_deprecate_command(name, message, replacement, skill_dirs, format),
        Commands::SkillRollback {
            name,
            version,
            skill_dirs,
            format,
        } => handle_skill_rollback_command(name, version, skill_dirs, format),
        Commands::SyncPull {
            source,
            skill,
            target,
            dry_run,
            format,
        } => handle_sync_pull_command(source, skill, target, dry_run, format),
        Commands::SkillProfile {
            name,
            period,
            format,
        } => handle_skill_profile_command(name, period, format),
        Commands::SkillCatalog {
            search,
            source,
            category,
            limit,
            skill_dirs,
            format,
        } => handle_skill_catalog_command(search, source, category, limit, skill_dirs, format),
        Commands::PreCommitValidate {
            staged,
            target,
            skill_dirs,
        } => handle_pre_commit_validate_command(staged, target, skill_dirs),
        Commands::SkillImport {
            source,
            target,
            force,
            dry_run,
            format,
        } => handle_skill_import_command(source, target, force, dry_run, format),
        Commands::SkillUsageReport {
            period,
            format,
            output,
            skill_dirs,
        } => handle_skill_usage_report_command(period, format, output, skill_dirs),
        Commands::SkillScore {
            name,
            skill_dirs,
            format,
            below_threshold,
        } => handle_skill_score_command(name, skill_dirs, format, below_threshold),
        Commands::Cert(action) => match action {
            CertAction::Status { format } => handle_cert_status_command(format),
            CertAction::Renew { force } => handle_cert_renew_command(force),
            CertAction::Install { cert, key, format } => {
                handle_cert_install_command(cert, key, format)
            }
        },
        #[cfg(feature = "http-transport")]
        Commands::ColdWindow(args) => {
            // The cold-window subcommand owns its own tokio runtime so
            // it can drive the producer + browser server concurrently
            // and listen for SIGINT/SIGTERM via tokio signals. The
            // outer `run()` is sync (anyhow::Result) by design.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    anyhow::anyhow!("failed to build tokio runtime for cold-window: {e}")
                })?;
            runtime.block_on(crate::cold_window_cli::run(args))
        }
    }
}
