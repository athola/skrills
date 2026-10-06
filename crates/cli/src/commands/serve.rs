//! Handler for the `serve` command.
//!
//! Includes fallback auto-persist behavior: when enabled via `SKRILLS_AUTO_PERSIST=1`,
//! analytics are automatically saved when the server exits. This provides persistence
//! until Claude Code exposes session-end hooks.

use anyhow::{anyhow, Result};
use rmcp::service::serve_server;
#[cfg(feature = "watch")]
use skrills_server::app::start_fs_watcher;
use skrills_server::app::SkillService;
use skrills_server::discovery::merge_extra_dirs;
use skrills_server::tool_schemas::all_tools;
use skrills_server::trace::stdio_with_optional_trace;
use skrills_state::{cache_ttl, load_manifest_settings};
use std::path::PathBuf;
use std::time::Duration;
use tokio::runtime::Runtime;

/// Persist analytics to cache file on server exit.
/// Called when auto-persist is enabled and the server is shutting down.
fn persist_analytics_on_exit() {
    use skrills_intelligence::{
        default_analytics_cache_path, load_or_build_analytics, save_analytics,
    };

    tracing::info!(target: "skrills::serve", "Persisting analytics on server exit...");

    match load_or_build_analytics(false, true) {
        Ok(analytics) => {
            if let Some(cache_path) = default_analytics_cache_path() {
                match save_analytics(&analytics, &cache_path) {
                    Ok(()) => {
                        tracing::info!(
                            target: "skrills::serve",
                            path = %cache_path.display(),
                            "Analytics persisted successfully on exit"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "skrills::serve",
                            path = %cache_path.display(),
                            error = %e,
                            "Failed to persist analytics on exit"
                        );
                    }
                }
            } else {
                tracing::warn!(
                    target: "skrills::serve",
                    "Cannot persist analytics: no cache path available"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "skrills::serve",
                error = %e,
                "Failed to build analytics for exit persistence"
            );
        }
    }
}

/// The `serve` flags, one field per flag, so the call site names each one
/// instead of passing a dozen positional `bool`s and paths.
#[derive(Debug, Default)]
pub(crate) struct ServeOptions {
    pub skill_dirs: Vec<PathBuf>,
    pub cache_ttl_ms: Option<u64>,
    pub trace_wire: bool,
    #[cfg(feature = "watch")]
    pub watch: bool,
    pub http: Option<String>,
    pub list_tools: bool,
    pub auth_token: Option<String>,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    pub cors_origins: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub tls_auto: bool,
    pub open_browser: bool,
}

/// The bearer token for HTTP serve: `--auth-token` or `SKRILLS_AUTH_TOKEN`
/// when given, else `[serve] auth_token` from `~/.skrills/config.toml`.
///
/// Read from the config file here: the config loader does not export the
/// token, so child processes never inherit it. A config file that exists but cannot be read is an error: the
/// server must not start without the auth the file asks for.
#[cfg_attr(not(feature = "http-transport"), allow(dead_code))]
fn resolve_auth_token(
    from_cli_or_env: Option<String>,
    load_config: impl FnOnce() -> Result<Option<skrills_server::config::Config>>,
) -> Result<Option<String>> {
    if from_cli_or_env.is_some() {
        return Ok(from_cli_or_env);
    }
    let config = load_config().map_err(|e| {
        anyhow!("could not read ~/.skrills/config.toml, which may set auth_token: {e}")
    })?;
    Ok(config.and_then(|c| c.serve.auth_token))
}

/// `[serve] project_roots` from `~/.skrills/config.toml`, with `~` expanded.
/// `None` when the file or the key is absent.
fn resolve_project_roots(
    load_config: impl FnOnce() -> Result<Option<skrills_server::config::Config>>,
) -> Result<Option<Vec<PathBuf>>> {
    Ok(load_config()?.and_then(|c| c.serve.project_roots_expanded()))
}

/// Applies `[serve] project_roots`, when set, to a service about to be served.
fn restrict_project_dirs(service: SkillService, roots: Option<&[PathBuf]>) -> SkillService {
    match roots {
        Some(roots) => service.with_project_roots(roots.to_vec()),
        None => service,
    }
}

/// Whether `bind` (a `--http` address) listens only on a loopback interface.
/// `localhost` counts; a wildcard (`0.0.0.0`, `[::]`) or any other host does not.
#[cfg_attr(not(feature = "http-transport"), allow(dead_code))]
fn bind_is_loopback(bind: &str) -> bool {
    if let Ok(addr) = bind.parse::<std::net::SocketAddr>() {
        return addr.ip().is_loopback();
    }
    let host = bind.rsplit_once(':').map_or(bind, |(host, _)| host);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Warn once at startup when other machines can reach the server and any
/// `project_dir` a client names will be read.
#[cfg_attr(not(feature = "http-transport"), allow(dead_code))]
fn warn_if_project_dirs_unrestricted(bind: &str, project_roots: Option<&[PathBuf]>) {
    if project_roots.is_none() && !bind_is_loopback(bind) {
        tracing::warn!(
            target: "skrills::serve",
            bind,
            "serving over a non-loopback address without [serve] project_roots: \
             MCP clients can have any readable directory analyzed as project_dir; \
             set project_roots in ~/.skrills/config.toml to restrict it"
        );
    }
}

/// Handle the `serve` command.
pub(crate) fn handle_serve_command(options: ServeOptions) -> Result<()> {
    let ServeOptions {
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
        open_browser,
    } = options;

    // Handle --list-tools: print tool names and exit
    if list_tools {
        let tools = all_tools();
        println!("Available MCP tools ({} total):", tools.len());
        println!();
        for tool in &tools {
            println!("  {}", tool.name);
            if let Some(ref desc) = tool.description {
                // Truncate description to first line or 80 chars
                let short_desc = desc.lines().next().unwrap_or("");
                let display = if short_desc.chars().count() > 72 {
                    // Use char-aware truncation to avoid panic on multi-byte UTF-8
                    let truncated: String = short_desc.chars().take(69).collect();
                    format!("{}...", truncated)
                } else {
                    short_desc.to_string()
                };
                println!("    {}", display);
            }
        }
        return Ok(());
    }

    let ttl = cache_ttl_ms
        .map(Duration::from_millis)
        .unwrap_or_else(|| cache_ttl(&load_manifest_settings));

    let project_roots =
        resolve_project_roots(|| skrills_server::config::load_config().map_err(|e| anyhow!(e)))?;

    let rt = Runtime::new()?;

    // HTTP transport mode
    if let Some(bind_addr) = http {
        #[cfg(feature = "http-transport")]
        {
            use skrills_server::http_transport::HttpSecurityConfig;
            use skrills_server::tls_auto::ensure_auto_tls_certs;

            // Clone values needed for the factory closure
            let skill_dirs_clone = skill_dirs.clone();

            // Resolve TLS paths: use auto-generated certs if --tls-auto, else CLI args
            let (resolved_cert, resolved_key) = if tls_auto {
                let (cert_path, key_path) = ensure_auto_tls_certs()?;
                tracing::info!(
                    target: "skrills::tls",
                    cert = %cert_path.display(),
                    key = %key_path.display(),
                    "Using auto-generated self-signed TLS certificate"
                );
                (Some(cert_path), Some(key_path))
            } else {
                (tls_cert, tls_key)
            };

            // `serve --http --watch` has no watcher to start: the HTTP
            // transport builds a service per session. Refuse rather than
            // accept the flag and serve a cache that never reloads.
            #[cfg(feature = "watch")]
            if watch {
                return Err(anyhow!(
                    "serve --watch is not supported with --http; restart the server to pick up skill changes"
                ));
            }

            let auth_token = resolve_auth_token(auth_token, || {
                skrills_server::config::load_config().map_err(|e| anyhow!(e))
            })?;

            // Build security config from CLI arguments
            let security = HttpSecurityConfig {
                auth_token,
                tls_cert: resolved_cert,
                tls_key: resolved_key,
                cors_origins,
                allowed_hosts,
            };

            // Show the status of the certificate this server will use.
            if let Some(cert_status) = security
                .tls_cert
                .as_deref()
                .and_then(crate::commands::get_cert_status_summary)
            {
                tracing::info!(target: "skrills::tls", "{}", cert_status);
            }

            warn_if_project_dirs_unrestricted(&bind_addr, project_roots.as_deref());
            let session_roots = project_roots.clone();

            let api_skill_dirs = skill_dirs.clone();
            return rt.block_on(async move {
                skrills_server::http_transport::serve_http_with_security(
                    move || {
                        SkillService::new_with_ttl(merge_extra_dirs(&skill_dirs_clone), ttl)
                            .map(|service| restrict_project_dirs(service, session_roots.as_deref()))
                            .map_err(std::io::Error::other)
                    },
                    &bind_addr,
                    security,
                    merge_extra_dirs(&api_skill_dirs),
                    open_browser,
                )
                .await
            });
        }

        #[cfg(not(feature = "http-transport"))]
        {
            let _ = bind_addr; // suppress unused warning
            let _ = (
                auth_token,
                tls_cert,
                tls_key,
                cors_origins,
                allowed_hosts,
                tls_auto,
                open_browser,
            ); // suppress unused warnings
            return Err(anyhow!(
                "HTTP transport requested but not available (built without 'http-transport' feature)"
            ));
        }
    }

    // Default: stdio transport
    // Skill reads, validations and syncs go to ~/.skrills/metrics.db, which
    // the dashboard of an HTTP server reads.
    let service = restrict_project_dirs(
        SkillService::new_with_ttl(merge_extra_dirs(&skill_dirs), ttl)?.with_persistent_metrics(),
        project_roots.as_deref(),
    );

    #[cfg(feature = "watch")]
    let _watcher = if watch {
        Some(start_fs_watcher(&service)?)
    } else {
        None
    };

    // Check if auto-persist is enabled for exit handling
    let auto_persist_on_exit = skrills_state::env_auto_persist();
    if auto_persist_on_exit {
        tracing::debug!(
            target: "skrills::serve",
            "Auto-persist enabled, analytics will be saved on server exit"
        );
    }

    let transport = stdio_with_optional_trace(trace_wire);
    let running = rt.block_on(async {
        serve_server(service, transport)
            .await
            .map_err(|e| anyhow!("failed to start server: {e}"))
    })?;

    rt.block_on(async {
        running
            .waiting()
            .await
            .map_err(|e| anyhow!("server task ended: {e}"))
    })?;

    // Persist analytics on server exit if enabled
    // This serves as a fallback until Claude Code exposes session-end hooks
    if auto_persist_on_exit {
        persist_analytics_on_exit();
    }

    #[cfg(feature = "watch")]
    drop(_watcher);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SA-23: the warning fires only for an address other machines can reach.
    #[test]
    fn bind_is_loopback_tells_local_from_reachable_addresses() {
        for bind in ["127.0.0.1:0", "[::1]:0", "localhost:3000", "LOCALHOST:3000"] {
            assert!(bind_is_loopback(bind), "{bind} is loopback");
        }
        for bind in [
            "0.0.0.0:3000",
            "[::]:3000",
            "10.0.0.5:8080",
            "skrills.internal:8080",
        ] {
            assert!(!bind_is_loopback(bind), "{bind} is reachable");
        }
    }

    /// SA-23: `[serve] project_roots` reaches serve with `~` expanded.
    #[test]
    fn project_roots_come_from_the_config_file() {
        let config: skrills_server::config::Config =
            toml::from_str("[serve]\nproject_roots = [\"/work\"]\n").unwrap();
        assert_eq!(
            resolve_project_roots(|| Ok(Some(config))).unwrap(),
            Some(vec![PathBuf::from("/work")])
        );
        assert_eq!(resolve_project_roots(|| Ok(None)).unwrap(), None);
    }

    fn config_with_token(token: &str) -> skrills_server::config::Config {
        toml::from_str(&format!("[serve]\nauth_token = \"{token}\"\n")).unwrap()
    }

    /// SB-8: a config-file token reaches the server although the config
    /// loader no longer exports it as SKRILLS_AUTH_TOKEN.
    #[test]
    fn a_config_file_token_is_used_when_no_flag_or_env_is_given() {
        let token = resolve_auth_token(None, || Ok(Some(config_with_token("from-file")))).unwrap();
        assert_eq!(token.as_deref(), Some("from-file"));
    }

    #[test]
    fn the_flag_or_env_token_wins_and_the_file_is_not_read() {
        let token = resolve_auth_token(Some("from-flag".into()), || {
            panic!("the config file must not be read when a token was given")
        })
        .unwrap();
        assert_eq!(token.as_deref(), Some("from-flag"));
    }

    #[test]
    fn an_unreadable_config_file_stops_the_server() {
        let err = resolve_auth_token(None, || Err(anyhow!("bad toml"))).unwrap_err();
        assert!(err.to_string().contains("auth_token"), "{err}");
    }

    #[test]
    fn no_token_anywhere_means_no_auth() {
        assert_eq!(resolve_auth_token(None, || Ok(None)).unwrap(), None);
    }

    /// SB-8 through the real loader: `~/.skrills/config.toml` under a temp HOME.
    #[test]
    fn the_real_loader_reads_the_token_from_the_home_config() {
        let _g = skrills_test_utils::env_guard();
        let home = tempfile::tempdir().unwrap();
        let _h = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        std::fs::create_dir_all(home.path().join(".skrills")).unwrap();
        std::fs::write(
            home.path().join(".skrills/config.toml"),
            "[serve]\nauth_token = \"home-token\"\n",
        )
        .unwrap();

        let token = resolve_auth_token(None, || {
            skrills_server::config::load_config().map_err(|e| anyhow!(e))
        })
        .unwrap();

        assert_eq!(token.as_deref(), Some("home-token"));
    }
}
