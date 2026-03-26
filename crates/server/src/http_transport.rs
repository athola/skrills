//! HTTP transport for remote MCP access.
//!
//! This module provides Streamable HTTP transport as an alternative to stdio,
//! allowing remote clients to connect to the MCP server over HTTP.
//!
//! ## Security Features
//!
//! - **Bearer Token Auth**: Validates `Authorization: Bearer <token>` header with constant-time comparison
//! - **TLS/HTTPS**: Supports TLS with custom certificates
//! - **CORS**: Configurable Cross-Origin Resource Sharing for browser clients

use crate::api::{
    dashboard_routes,
    metrics::{metrics_routes, MetricsState},
    rules::{rules_routes, RulesState},
    skills::{skills_routes, ApiState},
};
use crate::app::SkillService;
use anyhow::{Context, Result};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::IntoResponse;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tower_http::cors::{Any, CorsLayer};
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};

/// Header name for request ID.
const REQUEST_ID_HEADER: &str = "x-request-id";

/// Configuration for HTTP transport security.
///
/// Note: `Debug` is manually implemented to prevent auth_token from being logged.
#[derive(Clone, Default)]
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

// Custom Debug implementation that redacts auth_token to prevent credential leakage in logs.
impl std::fmt::Debug for HttpSecurityConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSecurityConfig")
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("tls_cert", &self.tls_cert)
            .field("tls_key", &self.tls_key)
            .field("cors_origins", &self.cors_origins)
            .finish()
    }
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
///
/// Uses constant-time comparison to prevent timing attacks on the auth token.
async fn auth_middleware(
    expected_token: Arc<String>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> impl IntoResponse {
    let uri = req.uri().path();
    let request_id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-");

    // Check Authorization header
    if let Some(auth_header) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                // Constant-time comparison to prevent timing attacks.
                // Both tokens are converted to bytes and compared in constant time.
                // Note: ct_eq requires equal-length slices, so we pad the shorter
                // one with zeros and always compare. The length mismatch is folded
                // into the final result without leaking via a timing side-channel.
                let provided = token.as_bytes();
                let expected = expected_token.as_bytes();

                let max_len = provided.len().max(expected.len());
                let mut p_padded = vec![0u8; max_len];
                let mut e_padded = vec![0u8; max_len];
                p_padded[..provided.len()].copy_from_slice(provided);
                e_padded[..expected.len()].copy_from_slice(expected);

                let length_ok = subtle::Choice::from((provided.len() == expected.len()) as u8);
                let content_ok = p_padded.ct_eq(&e_padded);
                if (length_ok & content_ok).into() {
                    tracing::debug!(
                        target: "skrills::http::auth",
                        uri,
                        request_id,
                        "Auth successful"
                    );
                    return next.run(req).await;
                }
                tracing::debug!(
                    target: "skrills::http::auth",
                    uri,
                    request_id,
                    "Auth failed: invalid token"
                );
            } else {
                tracing::debug!(
                    target: "skrills::http::auth",
                    uri,
                    request_id,
                    "Auth failed: malformed Authorization header (expected 'Bearer <token>')"
                );
            }
        } else {
            tracing::debug!(
                target: "skrills::http::auth",
                uri,
                request_id,
                "Auth failed: Authorization header not valid UTF-8"
            );
        }
    } else {
        tracing::debug!(
            target: "skrills::http::auth",
            uri,
            request_id,
            "Auth failed: missing Authorization header"
        );
    }

    // Auth failed - return 401 with WWW-Authenticate header per RFC 7235 §4.1
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        "Invalid or missing authorization token",
    )
        .into_response()
}

/// Builds CORS layer from allowed origins.
///
/// Invalid origins are logged as warnings and skipped. An empty result after
/// filtering invalid origins will disable CORS.
fn build_cors_layer(origins: &[String], has_auth: bool) -> CorsLayer {
    if origins.is_empty() {
        // No CORS - server-to-server only
        CorsLayer::new()
    } else if origins.iter().any(|o| o == "*") {
        // Security: Warn about wildcard CORS with auth enabled
        if has_auth {
            tracing::warn!(
                target: "skrills::http::cors",
                "Using wildcard CORS ('*') with authentication enabled. \
                 This may expose auth tokens to malicious sites. \
                 Consider specifying explicit origins instead."
            );
        }
        // Allow any origin
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
    } else {
        // Parse origins and log any failures
        let mut valid_origins = Vec::with_capacity(origins.len());
        for origin in origins {
            match origin.parse::<HeaderValue>() {
                Ok(header) => valid_origins.push(header),
                Err(e) => {
                    tracing::warn!(
                        target: "skrills::http::cors",
                        origin,
                        error = %e,
                        "Failed to parse CORS origin - it will be ignored. \
                         Browser requests from this origin will be rejected."
                    );
                }
            }
        }

        if valid_origins.is_empty() && !origins.is_empty() {
            tracing::warn!(
                target: "skrills::http::cors",
                "All CORS origins failed to parse. CORS will be disabled."
            );
        }

        CorsLayer::new()
            .allow_origin(valid_origins)
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
/// This endpoint has NO authentication by default.
/// Only bind to localhost or trusted network interfaces.
#[allow(dead_code)]
pub async fn serve_http<F>(service_factory: F, bind_addr: &str) -> Result<()>
where
    F: Fn() -> Result<SkillService, std::io::Error> + Send + Sync + 'static,
{
    serve_http_with_security(
        service_factory,
        bind_addr,
        HttpSecurityConfig::default(),
        vec![],
        false,
    )
    .await
}

/// Starts the MCP server over HTTP transport with security configuration.
///
/// # Arguments
/// * `service_factory` - Factory function to create SkillService instances
/// * `bind_addr` - Socket address to bind (e.g., "127.0.0.1:3000")
/// * `security` - Security configuration (auth, TLS, CORS)
/// * `skill_dirs` - Directories to scan for skills (used by dashboard API)
/// * `open_browser` - Whether to open the dashboard in the default browser after binding
pub async fn serve_http_with_security<F>(
    service_factory: F,
    bind_addr: &str,
    security: HttpSecurityConfig,
    skill_dirs: Vec<std::path::PathBuf>,
    open_browser: bool,
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

    // Build CORS layer (passes auth status for security warnings)
    let cors_layer = build_cors_layer(&security.cors_origins, security.has_auth());

    // Extract TLS config before potential move of auth_token.
    // Uses pattern matching instead of unwrap() to avoid relying on has_tls() invariant.
    let tls_config = match (&security.tls_cert, &security.tls_key) {
        (Some(cert), Some(key)) => Some((cert.clone(), key.clone())),
        _ => None,
    };

    // Build dashboard and API routes
    let api_state = Arc::new(ApiState::new(skill_dirs));
    let metrics_collector = Arc::new(
        skrills_metrics::MetricsCollector::new()
            .context("failed to create in-memory SQLite metrics collector")?,
    );
    let metrics_state = Arc::new(MetricsState {
        collector: metrics_collector,
    });

    // Discover rules for rules API
    let home = dirs::home_dir().unwrap_or_else(|| {
        tracing::warn!(
            target: "skrills::http",
            "Could not determine home directory; rule discovery may miss user-level rules"
        );
        PathBuf::new()
    });
    let project_dir = std::env::current_dir().ok();
    let rules = skrills_discovery::discover_rules(&home, project_dir.as_deref());
    let rules_state = Arc::new(RulesState {
        rules: Arc::new(rules),
    });

    // Serve static files (CSS) embedded at compile time
    let static_router = axum::Router::new().route(
        "/static/style.css",
        axum::routing::get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "text/css")],
                include_str!("../static/style.css"),
            )
        }),
    );

    // Create router with request ID and optional auth middleware
    // Request ID layers: SetRequestIdLayer generates UUID, PropagateRequestIdLayer copies to response
    let request_id_header = axum::http::HeaderName::from_static(REQUEST_ID_HEADER);
    let app = if let Some(token) = security.auth_token {
        let token = Arc::new(token);
        axum::Router::new()
            .merge(dashboard_routes())
            .merge(skills_routes(api_state))
            .merge(metrics_routes(metrics_state))
            .merge(rules_routes(rules_state))
            .merge(static_router)
            .fallback_service(http_service)
            .layer(cors_layer)
            .layer(axum::middleware::from_fn(|req: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(req).await;
                response.headers_mut().insert(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'".parse().unwrap(),
                );
                response
            }))
            .layer(axum::middleware::from_fn(move |req, next| {
                let token = token.clone();
                auth_middleware(token, req, next)
            }))
            .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
            .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
    } else {
        axum::Router::new()
            .merge(dashboard_routes())
            .merge(skills_routes(api_state))
            .merge(metrics_routes(metrics_state))
            .merge(rules_routes(rules_state))
            .merge(static_router)
            .fallback_service(http_service)
            .layer(cors_layer)
            .layer(axum::middleware::from_fn(|req: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(req).await;
                response.headers_mut().insert(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'".parse().unwrap(),
                );
                response
            }))
            .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
            .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid))
    };

    // Serve with or without TLS
    if let Some((cert_path, key_path)) = tls_config {
        serve_with_tls(app, addr, &cert_path, &key_path, open_browser).await
    } else {
        serve_without_tls(app, addr, open_browser).await
    }
}

/// Try to bind to the given address, falling back to up to 9 subsequent ports on conflict.
async fn bind_with_fallback(addr: SocketAddr) -> Result<(tokio::net::TcpListener, SocketAddr)> {
    const MAX_PORT_ATTEMPTS: u16 = 10;

    // Try the requested port first
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => return Ok((listener, addr)),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            tracing::warn!(
                target: "skrills::http",
                bind = %addr,
                "Port {} in use, trying alternatives",
                addr.port()
            );
        }
        Err(e) => return Err(e).with_context(|| format!("failed to bind to {addr}")),
    }

    // Try subsequent ports
    for offset in 1..MAX_PORT_ATTEMPTS {
        let try_port = match addr.port().checked_add(offset) {
            Some(p) => p,
            None => break,
        };
        let try_addr = SocketAddr::new(addr.ip(), try_port);
        match tokio::net::TcpListener::bind(try_addr).await {
            Ok(listener) => {
                tracing::info!(
                    target: "skrills::http",
                    original = %addr.port(),
                    actual = %try_port,
                    "Bound to fallback port"
                );
                return Ok((listener, try_addr));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e).with_context(|| format!("failed to bind to {try_addr}")),
        }
    }

    Err(anyhow::anyhow!(
        "could not bind to {} or any of the next {} ports",
        addr,
        MAX_PORT_ATTEMPTS - 1
    ))
}

/// Open a URL in the default browser.
fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        tracing::warn!(
            target: "skrills::http",
            url,
            "Cannot open browser: unsupported platform"
        );
        return;
    }

    match std::process::Command::new(cmd).arg(url).spawn() {
        Ok(mut child) => {
            // Check if the browser command exits with an error (e.g., no DISPLAY)
            std::thread::spawn(move || {
                if let Ok(status) = child.wait() {
                    if !status.success() {
                        eprintln!(
                            "Browser command exited with {}",
                            status
                                .code()
                                .map(|c| format!("code {c}"))
                                .unwrap_or_else(|| "signal".to_string())
                        );
                    }
                }
            });
        }
        Err(e) => {
            tracing::warn!(
                target: "skrills::http",
                error = %e,
                url,
                "Failed to open browser — open manually"
            );
        }
    }
}

/// Serve HTTP without TLS.
async fn serve_without_tls(app: axum::Router, addr: SocketAddr, open_browser: bool) -> Result<()> {
    let (listener, actual_addr) = bind_with_fallback(addr).await?;

    tracing::info!(
        target: "skrills::http",
        bind = %actual_addr,
        "MCP HTTP server listening"
    );

    if open_browser {
        let url = format!("http://{actual_addr}");
        open_in_browser(&url);
    }

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
    open_browser: bool,
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

    if open_browser {
        open_in_browser(&format!("https://{addr}"));
    }

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
        let layer = build_cors_layer(&[], false);
        // Should create a layer (no panic)
        let _ = layer;
    }

    #[test]
    fn cors_layer_wildcard_origin() {
        let layer = build_cors_layer(&["*".to_string()], false);
        let _ = layer;
    }

    #[test]
    fn cors_layer_specific_origins() {
        let layer = build_cors_layer(
            &[
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
            false,
        );
        let _ = layer;
    }

    #[test]
    fn security_config_debug_redacts_token() {
        let config = HttpSecurityConfig {
            auth_token: Some("super-secret-token".to_string()),
            ..Default::default()
        };
        let debug_output = format!("{:?}", config);
        assert!(!debug_output.contains("super-secret"));
        assert!(debug_output.contains("[REDACTED]"));
    }

    // Auth middleware integration tests using axum's test utilities
    mod auth_middleware_tests {
        use super::*;
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        fn test_app(token: &str) -> axum::Router {
            let token = Arc::new(token.to_string());
            axum::Router::new()
                .route("/test", axum::routing::get(|| async { "OK" }))
                .layer(axum::middleware::from_fn(move |req, next| {
                    let token = token.clone();
                    auth_middleware(token, req, next)
                }))
        }

        #[tokio::test]
        async fn auth_success_with_valid_token() {
            let app = test_app("secret-token");
            let req = Request::builder()
                .uri("/test")
                .header("Authorization", "Bearer secret-token")
                .body(Body::empty())
                .unwrap();

            let response = app.oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }

        #[tokio::test]
        async fn auth_fails_with_missing_header() {
            let app = test_app("secret-token");
            let req = Request::builder().uri("/test").body(Body::empty()).unwrap();

            let response = app.oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn auth_fails_with_invalid_token() {
            let app = test_app("secret-token");
            let req = Request::builder()
                .uri("/test")
                .header("Authorization", "Bearer wrong-token")
                .body(Body::empty())
                .unwrap();

            let response = app.oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn auth_fails_with_malformed_header() {
            let app = test_app("secret-token");
            // Missing "Bearer " prefix
            let req = Request::builder()
                .uri("/test")
                .header("Authorization", "secret-token")
                .body(Body::empty())
                .unwrap();

            let response = app.oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn auth_fails_with_basic_auth() {
            let app = test_app("secret-token");
            // Basic auth instead of Bearer
            let req = Request::builder()
                .uri("/test")
                .header("Authorization", "Basic dXNlcjpwYXNz")
                .body(Body::empty())
                .unwrap();

            let response = app.oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn auth_is_case_sensitive() {
            let app = test_app("Secret-Token");
            // Same token but different case
            let req = Request::builder()
                .uri("/test")
                .header("Authorization", "Bearer secret-token")
                .body(Body::empty())
                .unwrap();

            let response = app.oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    // TLS configuration tests
    mod tls_tests {
        use super::*;

        #[tokio::test]
        async fn tls_config_with_nonexistent_cert() {
            use axum_server::tls_rustls::RustlsConfig;
            use std::path::PathBuf;

            let cert_path = PathBuf::from("/nonexistent/cert.pem");
            let key_path = PathBuf::from("/nonexistent/key.pem");

            let result = RustlsConfig::from_pem_file(&cert_path, &key_path).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn tls_config_with_invalid_pem() {
            use axum_server::tls_rustls::RustlsConfig;
            use std::io::Write;

            // Create temp files with invalid PEM content
            let mut cert_file = tempfile::NamedTempFile::new().unwrap();
            let mut key_file = tempfile::NamedTempFile::new().unwrap();

            writeln!(cert_file, "not a valid certificate").unwrap();
            writeln!(key_file, "not a valid key").unwrap();

            let result = RustlsConfig::from_pem_file(cert_file.path(), key_file.path()).await;
            assert!(result.is_err());
        }

        #[test]
        fn security_config_requires_both_cert_and_key() {
            // Only cert, no key
            let config = HttpSecurityConfig {
                tls_cert: Some("/path/to/cert.pem".into()),
                tls_key: None,
                ..Default::default()
            };
            assert!(!config.has_tls());

            // Only key, no cert
            let config = HttpSecurityConfig {
                tls_cert: None,
                tls_key: Some("/path/to/key.pem".into()),
                ..Default::default()
            };
            assert!(!config.has_tls());

            // Both present
            let config = HttpSecurityConfig {
                tls_cert: Some("/path/to/cert.pem".into()),
                tls_key: Some("/path/to/key.pem".into()),
                ..Default::default()
            };
            assert!(config.has_tls());
        }

        #[tokio::test]
        async fn bind_with_fallback_binds_to_free_port() {
            // Grab an ephemeral port to discover a free one, then release it
            let tmp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = tmp.local_addr().unwrap();
            drop(tmp);

            let (listener, actual) = bind_with_fallback(addr).await.unwrap();
            assert_eq!(actual, addr, "should bind to the original port when free");
            drop(listener);
        }

        #[tokio::test]
        async fn bind_with_fallback_falls_back_when_port_occupied() {
            // Occupy a port
            let blocker = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let blocked_addr = blocker.local_addr().unwrap();

            // Ask for the occupied port — should fall back to blocked_port + 1..+9
            let (listener, actual) = bind_with_fallback(blocked_addr).await.unwrap();
            assert_ne!(
                actual.port(),
                blocked_addr.port(),
                "should NOT bind to the occupied port"
            );
            assert!(
                actual.port() > blocked_addr.port() && actual.port() <= blocked_addr.port() + 10,
                "fallback port should be within +1..+10 range"
            );

            drop(listener);
            drop(blocker);
        }

        #[tokio::test]
        async fn bind_with_fallback_errors_when_all_ports_occupied() {
            // Occupy 10 consecutive ports (the original + 9 fallbacks).
            // Use a fixed high port to avoid ephemeral-range race conditions.
            // If we can't block all 10, skip — another process holds one.
            let base_port: u16 = 19_100;
            let base_addr = SocketAddr::new("127.0.0.1".parse().unwrap(), base_port);

            let mut blockers = Vec::with_capacity(10);
            for offset in 0..10u16 {
                let addr = SocketAddr::new(base_addr.ip(), base_port + offset);
                match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => blockers.push(l),
                    Err(_) => {
                        // Can't block all ports — skip test rather than flake
                        eprintln!(
                            "skipping: port {} already in use by another process",
                            base_port + offset
                        );
                        return;
                    }
                }
            }

            let result = bind_with_fallback(base_addr).await;
            assert!(
                result.is_err(),
                "should error when original port and all fallbacks are occupied"
            );

            drop(blockers);
        }
    }
}
