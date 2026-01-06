//! Handler for the `serve` command.

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

/// Handle the `serve` command.
pub(crate) fn handle_serve_command(
    skill_dirs: Vec<PathBuf>,
    cache_ttl_ms: Option<u64>,
    trace_wire: bool,
    #[cfg(feature = "watch")] watch: bool,
    http: Option<String>,
    list_tools: bool,
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
                let display = if short_desc.len() > 72 {
                    format!("{}...", &short_desc[..69])
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
            // Clone values needed for the factory closure
            let skill_dirs_clone = skill_dirs.clone();
            return rt.block_on(async move {
                crate::http_transport::serve_http(
                    move || {
                        SkillService::new_with_ttl(merge_extra_dirs(&skill_dirs_clone), ttl)
                            .map_err(std::io::Error::other)
                    },
                    &bind_addr,
                )
                .await
            });
        }

        #[cfg(not(feature = "http-transport"))]
        {
            let _ = bind_addr; // suppress unused warning
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

    #[cfg(feature = "watch")]
    drop(_watcher);
    Ok(())
}
