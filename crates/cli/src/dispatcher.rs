//! CLI dispatch, the `skrills` binary entry point.
//!
//! Dispatch routing is kept separate from business logic.
//! This file owns nothing except the routing of parsed [`Cli`]
//! commands to their respective handlers in `crate::commands`. The
//! handlers themselves live in their own submodules; this is pure
//! dispatch.
//!
//! Rule of thumb: **no business logic here**. If you find yourself
//! adding more than a `Commands::Foo { x, y } => handle_foo(x, y)`
//! line, the logic belongs in `crate::commands::foo` (or a new
//! handler under `crate::commands`).

use crate::cli::{CertAction, Cli, Commands};
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
    handle_suggest_new_skills_command, handle_sync_agents_command, handle_sync_all_command,
    handle_sync_command, handle_sync_pull_command, handle_sync_status_command,
    handle_validate_command, run_sync_with_adapters, skipped_commands_note, ServeOptions,
    SetupOptions, SmartRecommendOptions, SyncAllArgs,
};
use crate::doctor::doctor_report;
use crate::tui::tui_flow;
use anyhow::Result;
use clap::Parser;
use skrills_discovery::merge_extra_dirs;

/// Whether stdin and stdout are both a terminal.
fn is_interactive_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Main application entry point.
pub fn run() -> Result<()> {
    // Logs go to stderr. stdout belongs to the command's output: the JSON
    // document under `--format json`, and the JSON-RPC stream under stdio
    // `serve`, where a log line is a protocol error.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Load config file and apply settings to env vars before CLI parsing.
    // This ensures precedence: CLI > ENV > config file.
    let config_error = crate::config::try_apply_config_to_env().err();

    let cli = Cli::parse();

    // Check for first-run (only for user-facing commands, not for `serve` which is called by MCP)
    // Also skip for batch/non-interactive commands like sync-all
    let command_ref = cli.command.as_ref();
    let is_serve = matches!(command_ref, Some(Commands::Serve { .. }) | None);

    // The file may set `auth_token`; serving without it would drop auth.
    // Other commands only lose defaults, so they warn and carry on.
    if let Some(e) = config_error {
        if is_serve {
            return Err(e.context(
                "refusing to serve: ~/.skrills/config.toml could not be read and may set auth_token; fix or remove it",
            ));
        }
        tracing::warn!(
            target: "skrills::config",
            error = %format!("{e:#}"),
            "Failed to parse config file (~/.skrills/config.toml); continuing without its settings"
        );
    }
    let is_setup = matches!(command_ref, Some(Commands::Setup { .. }));
    let is_batch = matches!(command_ref, Some(Commands::SyncAll { .. }));

    // The first-run prompt asks a question, so it needs a person at both
    // ends. With stdout piped (a script, `--format json | jq`, CI) its banner
    // would land in the captured output ahead of the document.
    if !is_serve && !is_setup && !is_batch && is_interactive_terminal() {
        if let Ok(true) = skrills_setup::is_first_run() {
            if let Ok(true) = skrills_setup::prompt_first_run_setup() {
                // Run interactive setup
                let config = skrills_setup::interactive_setup(
                    None, None, false, false, false, false, false, None, false,
                )?;
                skrills_setup::run_setup(config)?;
                tracing::info!(
                    "you can now use skrills; run your command again or explore 'skrills --help'"
                );
                return Ok(());
            } else {
                tracing::info!("setup skipped; run 'skrills setup' when ready");
            }
        }
    }

    match match cli.command {
        Some(command) => command,
        None => default_command()?,
    } {
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
            allowed_hosts,
            tls_auto,
            open,
        } => handle_serve_command(ServeOptions {
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
            allowed_hosts,
            tls_auto,
            open_browser: open,
        }),
        Commands::Mirror {
            dry_run,
            skip_existing_commands,
            include_marketplace,
        } => handle_mirror_command(dry_run, skip_existing_commands, include_marketplace),
        Commands::Agent {
            agent,
            skill_dirs,
            dry_run,
            dangerously_bypass_sandbox,
        } => handle_agent_command(agent, skill_dirs, dry_run, dangerously_bypass_sandbox),
        Commands::MultiCliAgent {
            agent,
            backend,
            skill_dirs,
            dry_run,
            dangerously_bypass_sandbox,
        } => handle_multi_cli_agent_command(
            agent,
            backend,
            skill_dirs,
            dry_run,
            dangerously_bypass_sandbox,
        ),
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
                tracing::warn!(
                    "syncing commands will overwrite existing files; use --skip-existing-commands to keep existing copies"
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

            tracing::info!(
                "{}{}",
                report.summary,
                skipped_commands_note(skip_existing_commands, &report)
            );
            if dry_run {
                tracing::info!("(dry run - no changes made)");
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

            tracing::info!("{}", report.summary);
            if dry_run {
                tracing::info!("(dry run - no changes made)");
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

            tracing::info!("{}", report.summary);
            if dry_run {
                tracing::info!("(dry run - no changes made)");
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
            validate,
            autofix,
        } => handle_sync_all_command(SyncAllArgs {
            from,
            to,
            dry_run,
            skip_existing_commands,
            include_marketplace,
            exclude_plugins,
            validate,
            autofix,
        }),
        Commands::SyncStatus { from, to } => handle_sync_status_command(from, to),
        Commands::Doctor => doctor_report(),
        Commands::Tui { skill_dirs } => tui_flow(&merge_extra_dirs(&skill_dirs)),
        #[cfg(feature = "dashboard")]
        Commands::Dashboard { skill_dirs } => {
            use std::io::IsTerminal;
            if !std::io::stdout().is_terminal() {
                return Err(anyhow::anyhow!("Dashboard requires a TTY"));
            }
            let dashboard = skrills_dashboard::Dashboard::new(dashboard_skill_dirs(&skill_dirs))?;
            tokio::runtime::Runtime::new()?.block_on(dashboard.run())
        }
        #[cfg(not(feature = "dashboard"))]
        Commands::Dashboard { .. } => {
            anyhow::bail!("dashboard feature not enabled; rebuild with --features dashboard")
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
            include_marketplace,
        } => handle_setup_command(SetupOptions {
            client,
            bin_dir,
            reinstall,
            uninstall,
            add,
            yes,
            universal,
            mirror_source,
            include_marketplace,
        }),
        Commands::Validate {
            skill_dirs,
            target,
            autofix,
            backup,
            format,
            errors_only,
            #[cfg(feature = "watch")]
            watch,
            #[cfg(feature = "watch")]
                debounce_ms: _debounce_ms,
        } => {
            // The watch loop is tracked in #208. Until it exists, a one-shot
            // run that exits 0 would look like a watcher that saw no changes.
            #[cfg(feature = "watch")]
            if watch {
                anyhow::bail!("validate --watch is not implemented (tracked in #208)");
            }
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
        } => handle_recommend_skills_smart_command(SmartRecommendOptions {
            uri,
            prompt,
            project_dir,
            limit,
            include_usage,
            include_context,
            auto_persist,
            format,
            skill_dirs,
        }),
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
            // it can drive the producer and browser server concurrently
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

/// The skill directories the dashboard watches: `--skill-dir` plus
/// `SKRILLS_SKILL_DIR`, as `tui` and every other command merge them.
#[cfg_attr(not(feature = "dashboard"), allow(dead_code))]
fn dashboard_skill_dirs(skill_dirs: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    merge_extra_dirs(skill_dirs)
}

/// The command bare `skrills` runs: `serve`, parsed as if typed, so its
/// environment defaults (`SKRILLS_HTTP`, `SKRILLS_TRACE_WIRE`, the TLS and
/// auth variables) apply exactly as they do to `skrills serve`.
fn default_command() -> anyhow::Result<Commands> {
    Cli::try_parse_from(["skrills", "serve"])?
        .command
        .ok_or_else(|| anyhow::anyhow!("`skrills serve` parsed to no command"))
}

#[cfg(test)]
mod default_command_tests {
    use super::*;

    #[test]
    fn dashboard_skill_dirs_include_the_environment() {
        let _g = skrills_test_utils::env_guard();
        let _dirs = skrills_test_utils::set_env_var("SKRILLS_SKILL_DIR", Some("/from/env"));

        let dirs = dashboard_skill_dirs(&[std::path::PathBuf::from("/from/flag")]);

        assert_eq!(
            dirs,
            vec![
                std::path::PathBuf::from("/from/env"),
                std::path::PathBuf::from("/from/flag")
            ]
        );
    }

    #[test]
    fn bare_skrills_honours_the_serve_environment() {
        let _g = skrills_test_utils::env_guard();
        let _http = skrills_test_utils::set_env_var("SKRILLS_HTTP", Some("127.0.0.1:9"));
        let _trace = skrills_test_utils::set_env_var("SKRILLS_TRACE_WIRE", Some("true"));
        let _token = skrills_test_utils::set_env_var("SKRILLS_AUTH_TOKEN", None);

        match default_command().unwrap() {
            Commands::Serve {
                http, trace_wire, ..
            } => {
                assert_eq!(http.as_deref(), Some("127.0.0.1:9"));
                assert!(trace_wire);
            }
            other => panic!("expected serve, got {other:?}"),
        }
    }
}

#[cfg(test)]
#[path = "dispatcher_sync_tests.rs"]
mod sync_tests;
