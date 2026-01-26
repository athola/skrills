//! Handler for the `serve` command.
//!
//! Includes fallback auto-persist behavior: when enabled via `SKRILLS_AUTO_PERSIST=1`,
//! analytics are automatically saved when the server exits. This provides persistence
//! until Claude Code exposes session-end hooks.

use crate::app::{start_fs_watcher, SkillService};
use crate::discovery::merge_extra_dirs;
use crate::tool_schemas::all_tools;
use crate::trace::stdio_with_optional_trace;
use anyhow::{anyhow, Result};
use rmcp::service::serve_server;
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

/// Handle the `serve` command.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_serve_command(
    skill_dirs: Vec<PathBuf>,
    cache_ttl_ms: Option<u64>,
    trace_wire: bool,
    #[cfg(feature = "watch")] watch: bool,
    http: Option<String>,
    list_tools: bool,
    // Phase 2 security options
    auth_token: Option<String>,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    cors_origins: Vec<String>,
    tls_auto: bool,
) -> Result<()> {
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

    let rt = Runtime::new()?;

    // HTTP transport mode
    if let Some(bind_addr) = http {
        #[cfg(feature = "http-transport")]
        {
            use crate::http_transport::HttpSecurityConfig;
            use crate::tls_auto::ensure_auto_tls_certs;

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

            // Build security config from CLI arguments
            let security = HttpSecurityConfig {
                auth_token,
                tls_cert: resolved_cert,
                tls_key: resolved_key,
                cors_origins,
            };

            return rt.block_on(async move {
                crate::http_transport::serve_http_with_security(
                    move || {
                        SkillService::new_with_ttl(merge_extra_dirs(&skill_dirs_clone), ttl)
                            .map_err(std::io::Error::other)
                    },
                    &bind_addr,
                    security,
                )
                .await
            });
        }

        #[cfg(not(feature = "http-transport"))]
        {
            let _ = bind_addr; // suppress unused warning
            let _ = (auth_token, tls_cert, tls_key, cors_origins, tls_auto); // suppress unused warnings
            return Err(anyhow!(
                "HTTP transport requested but not available (built without 'http-transport' feature)"
            ));
        }
    }

    // Default: stdio transport
    let service = SkillService::new_with_ttl(merge_extra_dirs(&skill_dirs), ttl)?;

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
