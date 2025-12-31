//! HTTP transport for remote MCP access.
//!
//! This module provides Streamable HTTP transport as an alternative to stdio,
//! allowing remote clients to connect to the MCP server over HTTP.
//!
//! **Security Note:** Phase 1 has no authentication. Only use on trusted networks.

use crate::app::SkillService;
use anyhow::{Context, Result};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::net::SocketAddr;
use std::sync::Arc;

/// Starts the MCP server over HTTP transport.
///
/// # Arguments
/// * `service_factory` - Factory function to create SkillService instances
/// * `bind_addr` - Socket address to bind (e.g., "127.0.0.1:3000")
///
/// # Security
/// This endpoint has NO authentication in Phase 1.
/// Only bind to localhost or trusted network interfaces.
#[allow(dead_code)] // Will be used by serve command in Task 4
pub async fn serve_http<F>(service_factory: F, bind_addr: &str) -> Result<()>
where
    F: Fn() -> Result<SkillService, std::io::Error> + Send + Sync + 'static,
{
    let addr: SocketAddr = bind_addr
        .parse()
        .with_context(|| format!("invalid bind address: {bind_addr}"))?;

    tracing::info!(
        target: "skrills::http",
        bind = %addr,
        "Starting MCP server over HTTP (no auth - trusted networks only)"
    );

    // Create session manager for stateful connections
    let session_manager = Arc::new(LocalSessionManager::default());

    // Configure the HTTP server
    let config = StreamableHttpServerConfig::default();

    // Create the streamable HTTP service
    let http_service = StreamableHttpService::new(service_factory, session_manager, config);

    // Create axum app with the MCP service
    let app = axum::Router::new().fallback_service(http_service);

    // Bind and serve
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind to {addr}"))?;

    tracing::info!(
        target: "skrills::http",
        bind = %addr,
        "MCP HTTP server listening"
    );

    axum::serve(listener, app)
        .await
        .context("HTTP server error")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_bind_address() {
        let addr: Result<SocketAddr, _> = "127.0.0.1:3000".parse();
        assert!(addr.is_ok());
    }

    #[test]
    fn parse_invalid_bind_address() {
        let addr: Result<SocketAddr, _> = "not-an-address".parse();
        assert!(addr.is_err());
    }

    #[test]
    fn parse_ipv6_bind_address() {
        let addr: Result<SocketAddr, _> = "[::1]:3000".parse();
        assert!(addr.is_ok());
    }

    #[test]
    fn parse_wildcard_bind_address() {
        let addr: Result<SocketAddr, _> = "0.0.0.0:3000".parse();
        assert!(addr.is_ok());
    }
}
