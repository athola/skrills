//! HTTP transport for remote MCP access.
//!
//! This module provides Streamable HTTP transport as an alternative to stdio,
//! allowing remote clients to connect to the MCP server over HTTP.
//!
//! ## Security Features (Phase 2)
//!
//! - **Bearer Token Auth**: Validates `Authorization: Bearer <token>` header
//! - **TLS/HTTPS**: Supports TLS with custom certificates
//! - **CORS**: Configurable Cross-Origin Resource Sharing for browser clients

use crate::app::SkillService;
use anyhow::{Context, Result};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

/// Configuration for HTTP transport security.
#[derive(Debug, Clone, Default)]
pub struct HttpSecurityConfig {
    /// Bearer token for authentication (None = no auth).
    pub auth_token: Option<String>,
    /// TLS certificate path (requires tls_key).
    pub tls_cert: Option<std::path::PathBuf>,
    /// TLS private key path (requires tls_cert).
    pub tls_key: Option<std::path::PathBuf>,
    /// Allowed CORS origins (empty = no CORS).
    pub cors_origins: Vec<String>,
}

impl HttpSecurityConfig {
    /// Returns true if TLS is configured.
    pub fn has_tls(&self) -> bool {
        self.tls_cert.is_some() && self.tls_key.is_some()
    }

    /// Returns true if auth is required.
    pub fn has_auth(&self) -> bool {
        self.auth_token.is_some()
    }
}

/// Bearer token authentication middleware.
async fn auth_middleware(
    expected_token: Arc<String>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    // Check Authorization header
    if let Some(auth_header) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                if token == expected_token.as_str() {
                    return next.run(req).await;
                }
            }
        }
    }

    // Auth failed
    (
        StatusCode::UNAUTHORIZED,
        "Invalid or missing authorization token",
    )
        .into_response()
}

/// Builds CORS layer from allowed origins.
fn build_cors_layer(origins: &[String]) -> CorsLayer {
    if origins.is_empty() {
        // No CORS - server-to-server only
        CorsLayer::new()
    } else if origins.iter().any(|o| o == "*") {
        // Allow any origin
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
    } else {
        // Specific origins
        let origins: Vec<HeaderValue> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
    }
}

/// Starts the MCP server over HTTP transport.
///
/// # Arguments
/// * `service_factory` - Factory function to create SkillService instances
/// * `bind_addr` - Socket address to bind (e.g., "127.0.0.1:3000")
///
/// # Security
/// This endpoint has NO authentication in Phase 1.
/// Only bind to localhost or trusted network interfaces.
#[allow(dead_code)]
pub async fn serve_http<F>(service_factory: F, bind_addr: &str) -> Result<()>
where
    F: Fn() -> Result<SkillService, std::io::Error> + Send + Sync + 'static,
{
    serve_http_with_security(service_factory, bind_addr, HttpSecurityConfig::default()).await
}

/// Starts the MCP server over HTTP transport with security configuration.
///
/// # Arguments
/// * `service_factory` - Factory function to create SkillService instances
/// * `bind_addr` - Socket address to bind (e.g., "127.0.0.1:3000")
/// * `security` - Security configuration (auth, TLS, CORS)
pub async fn serve_http_with_security<F>(
    service_factory: F,
    bind_addr: &str,
    security: HttpSecurityConfig,
) -> Result<()>
where
    F: Fn() -> Result<SkillService, std::io::Error> + Send + Sync + 'static,
{
    let addr: SocketAddr = bind_addr
        .parse()
        .with_context(|| format!("invalid bind address: {bind_addr}"))?;

    let protocol = if security.has_tls() { "HTTPS" } else { "HTTP" };
    let auth_status = if security.has_auth() {
        "enabled"
    } else {
        "disabled"
    };
    let cors_status = if security.cors_origins.is_empty() {
        "disabled".to_string()
    } else if security.cors_origins.iter().any(|o| o == "*") {
        "allow-all".to_string()
    } else {
        format!("{} origins", security.cors_origins.len())
    };

    tracing::info!(
        target: "skrills::http",
        bind = %addr,
        protocol,
        auth = auth_status,
        cors = cors_status,
        "Starting MCP server"
    );

    // Create session manager for stateful connections
    let session_manager = Arc::new(LocalSessionManager::default());

    // Configure the HTTP server
    let config = StreamableHttpServerConfig::default();

    // Create the streamable HTTP service
    let http_service = StreamableHttpService::new(service_factory, session_manager, config);

    // Build CORS layer
    let cors_layer = build_cors_layer(&security.cors_origins);

    // Extract TLS config before potential move of auth_token
    let tls_config = if security.has_tls() {
        Some((
            security.tls_cert.clone().unwrap(),
            security.tls_key.clone().unwrap(),
        ))
    } else {
        None
    };

    // Create router with optional auth middleware
    let app = if let Some(token) = security.auth_token {
        let token = Arc::new(token);
        axum::Router::new()
            .fallback_service(http_service)
            .layer(cors_layer)
            .layer(axum::middleware::from_fn(move |req, next| {
                let token = token.clone();
                auth_middleware(token, req, next)
            }))
    } else {
        axum::Router::new()
            .fallback_service(http_service)
            .layer(cors_layer)
    };

    // Serve with or without TLS
    if let Some((cert_path, key_path)) = tls_config {
        serve_with_tls(app, addr, &cert_path, &key_path).await
    } else {
        serve_without_tls(app, addr).await
    }
}

/// Serve HTTP without TLS.
async fn serve_without_tls(app: axum::Router, addr: SocketAddr) -> Result<()> {
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

/// Serve HTTPS with TLS.
async fn serve_with_tls(
    app: axum::Router,
    addr: SocketAddr,
    cert_path: &Path,
    key_path: &Path,
) -> Result<()> {
    use axum_server::tls_rustls::RustlsConfig;

    let tls_config = RustlsConfig::from_pem_file(cert_path, key_path)
        .await
        .with_context(|| {
            format!(
                "failed to load TLS config from cert={} key={}",
                cert_path.display(),
                key_path.display()
            )
        })?;

    tracing::info!(
        target: "skrills::http",
        bind = %addr,
        cert = %cert_path.display(),
        "MCP HTTPS server listening (TLS enabled)"
    );

    axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service())
        .await
        .context("HTTPS server error")?;

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

    #[test]
    fn security_config_defaults() {
        let config = HttpSecurityConfig::default();
        assert!(!config.has_tls());
        assert!(!config.has_auth());
    }

    #[test]
    fn security_config_with_auth() {
        let config = HttpSecurityConfig {
            auth_token: Some("test-token".to_string()),
            ..Default::default()
        };
        assert!(config.has_auth());
        assert!(!config.has_tls());
    }

    #[test]
    fn security_config_with_tls() {
        let config = HttpSecurityConfig {
            tls_cert: Some("/path/to/cert.pem".into()),
            tls_key: Some("/path/to/key.pem".into()),
            ..Default::default()
        };
        assert!(config.has_tls());
        assert!(!config.has_auth());
    }

    #[test]
    fn cors_layer_empty_origins() {
        let layer = build_cors_layer(&[]);
        // Should create a layer (no panic)
        let _ = layer;
    }

    #[test]
    fn cors_layer_wildcard_origin() {
        let layer = build_cors_layer(&["*".to_string()]);
        let _ = layer;
    }

    #[test]
    fn cors_layer_specific_origins() {
        let layer = build_cors_layer(&[
            "http://localhost:3000".to_string(),
            "https://app.example.com".to_string(),
        ]);
        let _ = layer;
    }
}
