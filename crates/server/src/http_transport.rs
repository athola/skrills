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
//! - **Host validation**: Every route answers 403 to a `Host` outside the
//!   loopback names and the operator's allow-list, closing DNS rebinding

use crate::api::{
    dashboard_routes, mcp_servers_routes,
    metrics::{metrics_routes, MetricsState},
    rules::{rules_routes, RulesState},
    skills::{skills_routes, ApiState},
};
use crate::app::SkillService;
use anyhow::{anyhow, Context, Result};
use axum::http::uri::Authority;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
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

/// Content-Security-Policy served with every response. `unsafe-inline` is
/// needed because the dashboard ships its scripts and styles inline.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'";

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
    /// Extra `Host` authorities the server accepts, on top of the loopback
    /// names rmcp allows by default. Needed when clients address the server by
    /// any name other than localhost, 127.0.0.1 or ::1, since Host validation
    /// refuses the rest. Each entry is a bare authority (`host` or
    /// `host:port`), never a URL.
    pub allowed_hosts: Vec<String>,
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
            .field("allowed_hosts", &self.allowed_hosts)
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

/// An authority to compare `Host` headers against: a host name with an
/// optional port.
#[derive(Debug, PartialEq, Eq)]
struct HostAuthority {
    host: String,
    port: Option<u16>,
}

impl HostAuthority {
    /// Whether this allow-list entry accepts the authority a client addressed.
    /// An entry without a port accepts any port, which is how rmcp reads its
    /// own list.
    fn accepts(&self, requested: &HostAuthority) -> bool {
        self.host == requested.host && (self.port.is_none() || self.port == requested.port)
    }
}

/// Lower-cases a host name and strips the brackets an IPv6 literal carries in
/// an authority, so `[::1]` and `::1` compare equal.
fn normalize_host(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

/// Parses a `Host` header value or an allow-list entry.
///
/// Returns `None` for anything that is not an authority, which is what makes a
/// scheme in an allow-list entry a startup error rather than a silent 403: an
/// unparseable entry would otherwise be compared as a literal host name and
/// match nothing. A bare IPv6 literal is accepted because it is not a valid
/// authority yet appears in rmcp's own default list as `::1`.
fn parse_host_authority(raw: &str) -> Option<HostAuthority> {
    let raw = raw.trim();
    if let Ok(authority) = Authority::try_from(raw) {
        if !authority.host().is_empty() {
            return Some(HostAuthority {
                host: normalize_host(authority.host()),
                port: authority.port_u16(),
            });
        }
    }
    raw.parse::<std::net::IpAddr>().ok().map(|_| HostAuthority {
        host: normalize_host(raw),
        port: None,
    })
}

/// Parses the effective `Host` allow-list, naming the first entry that is not
/// an authority.
///
/// Blank entries are skipped rather than rejected: a trailing comma in
/// `--allowed-hosts foo.example,` was harmless before the check existed, and
/// refusing to start over one would be a worse trade than ignoring it.
fn parse_host_allow_list(entries: &[String]) -> Result<Vec<HostAuthority>> {
    entries
        .iter()
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| {
            parse_host_authority(entry).ok_or_else(|| {
                anyhow!(
                    "invalid allowed host {entry:?}: expected a bare authority such as \
                     `skrills.internal` or `skrills.internal:8080`, with no scheme or path"
                )
            })
        })
        .collect()
}

/// Builds the MCP transport config, extending rmcp's loopback `allowed_hosts`
/// with any the operator supplied and switching on `Origin` validation when
/// CORS origins were named.
///
/// The loopback default is what blocks DNS rebinding (RUSTSEC-2026-0189): a
/// page served from an attacker's domain that resolves to 127.0.0.1 still
/// sends its own name in `Host`. Operator hosts are appended rather than
/// substituted, so naming a LAN host to reach a non-loopback bind cannot
/// silently drop the loopback protection.
///
/// A wildcard in `cors_origins` leaves `allowed_origins` empty on purpose:
/// rmcp reads a non-empty list as "refuse every origin outside it", and `*`
/// matches nothing, so copying it over would refuse the browsers the operator
/// just allowed.
fn build_streamable_config(security: &HttpSecurityConfig) -> StreamableHttpServerConfig {
    let mut config = StreamableHttpServerConfig::default();
    config
        .allowed_hosts
        .extend(security.allowed_hosts.iter().cloned());
    if !security.cors_origins.iter().any(|origin| origin == "*") {
        config
            .allowed_origins
            .extend(security.cors_origins.iter().cloned());
    }
    config
}

/// The authority a client addressed, from `Host` or, over HTTP/2, from the
/// `:authority` pseudo-header hyper folds into the URI.
fn requested_authority(req: &axum::extract::Request) -> Option<HostAuthority> {
    match req.headers().get(header::HOST) {
        Some(host) => parse_host_authority(host.to_str().ok()?),
        None => req.uri().authority().map(|authority| HostAuthority {
            host: normalize_host(authority.host()),
            port: authority.port_u16(),
        }),
    }
}

/// `Host` validation for every route.
///
/// rmcp checks `Host` inside the MCP service, which left the dashboard and the
/// REST API answering a rebound name. This applies the same allow-list to the
/// whole router, so a page on an attacker's domain that resolves to the bind
/// address cannot read `/api/skills` either.
async fn host_middleware(
    allowed: Arc<Vec<HostAuthority>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(requested) = requested_authority(&req) else {
        return (
            StatusCode::BAD_REQUEST,
            "Bad Request: missing or malformed Host header",
        )
            .into_response();
    };

    if allowed.iter().any(|entry| entry.accepts(&requested)) {
        return next.run(req).await;
    }

    tracing::warn!(
        target: "skrills::http::host",
        host = %requested.host,
        uri = req.uri().path(),
        "Refused a Host outside the allow-list (possible DNS rebinding attempt)"
    );
    (
        StatusCode::FORBIDDEN,
        "Forbidden: Host header is not allowed",
    )
        .into_response()
}

/// Adds the dashboard's Content-Security-Policy to every response.
async fn csp_middleware(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut response = next.run(req).await;
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    response
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
        // Wildcard CORS is worth a warning whether or not auth is on: without
        // a token any site a browser visits can drive the API, and with one it
        // can also reach the token.
        if has_auth {
            tracing::warn!(
                target: "skrills::http::cors",
                "Using wildcard CORS ('*') with authentication enabled. \
                 This may expose auth tokens to malicious sites. \
                 Consider specifying explicit origins instead."
            );
        } else {
            tracing::warn!(
                target: "skrills::http::cors",
                "Using wildcard CORS ('*') with authentication disabled. \
                 Any site a browser visits can drive this server's API. \
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

    // Configure the HTTP server. The effective Host allow-list is parsed once
    // here so an entry rmcp could not match becomes a startup error instead of
    // a 403 on every request.
    let config = build_streamable_config(&security);
    let allowed_hosts = Arc::new(parse_host_allow_list(&config.allowed_hosts)?);

    tracing::info!(
        target: "skrills::http",
        bind = %addr,
        protocol,
        auth = auth_status,
        cors = cors_status,
        allowed_hosts = ?config.allowed_hosts,
        "Starting MCP server"
    );

    // A non-loopback bind is reached under some other name, and every request
    // carrying that name is refused until it is listed.
    if !addr.ip().is_loopback() && security.allowed_hosts.is_empty() {
        tracing::warn!(
            target: "skrills::http",
            bind = %addr,
            "Bound a non-loopback address with no --allowed-hosts. Requests naming \
             anything other than localhost, 127.0.0.1 or ::1 will be refused with 403."
        );
    }

    // Create session manager for stateful connections
    let session_manager = Arc::new(LocalSessionManager::default());

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

    // Request ID layers: SetRequestIdLayer generates UUID, PropagateRequestIdLayer copies to response
    let request_id_header = axum::http::HeaderName::from_static(REQUEST_ID_HEADER);
    let mut app = axum::Router::new()
        .merge(dashboard_routes())
        .merge(skills_routes(api_state))
        .merge(metrics_routes(metrics_state))
        .merge(rules_routes(rules_state))
        .merge(mcp_servers_routes())
        .merge(static_router)
        .fallback_service(http_service)
        .layer(axum::middleware::from_fn(csp_middleware));

    if let Some(token) = security.auth_token {
        let token = Arc::new(token);
        app = app.layer(axum::middleware::from_fn(move |req, next| {
            let token = token.clone();
            auth_middleware(token, req, next)
        }));
    }

    // Layer order, outermost first: request id, Host, CORS, auth, CSP, routes.
    // CORS sits outside auth because a browser preflight carries no
    // Authorization header and would otherwise be answered with 401. Host
    // validation sits outside CORS so a rebound name is refused even on a
    // preflight.
    let app = app
        .layer(cors_layer)
        .layer(axum::middleware::from_fn(move |req, next| {
            host_middleware(allowed_hosts.clone(), req, next)
        }))
        .layer(PropagateRequestIdLayer::new(request_id_header.clone()))
        .layer(SetRequestIdLayer::new(request_id_header, MakeRequestUuid));

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
                "Failed to open browser; open manually"
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

    /// The loopback allowlist is what stops a rebound attacker domain from
    /// driving a locally bound MCP server (RUSTSEC-2026-0189), so it must be
    /// on with no configuration at all.
    #[test]
    fn streamable_config_allows_only_loopback_hosts_by_default() {
        let config = build_streamable_config(&HttpSecurityConfig::default());

        assert_eq!(
            config.allowed_hosts,
            vec![
                "localhost".to_string(),
                "127.0.0.1".to_string(),
                "::1".to_string()
            ],
            "default Host allowlist should be loopback only"
        );
    }

    /// Binding a non-loopback address (`--http 0.0.0.0:8080`) is documented,
    /// and Host validation rejects it unless the operator names the host.
    #[test]
    fn streamable_config_accepts_operator_supplied_hosts() {
        let config = build_streamable_config(&HttpSecurityConfig {
            allowed_hosts: vec!["skrills.internal:8080".to_string()],
            ..Default::default()
        });

        assert!(
            config
                .allowed_hosts
                .contains(&"skrills.internal:8080".to_string()),
            "operator host should be accepted, got {:?}",
            config.allowed_hosts
        );
    }

    /// Naming an extra host must not be a way to switch the loopback
    /// protection off by accident.
    #[test]
    fn streamable_config_keeps_loopback_when_hosts_are_added() {
        let config = build_streamable_config(&HttpSecurityConfig {
            allowed_hosts: vec!["example.com".to_string()],
            ..Default::default()
        });

        for loopback in ["localhost", "127.0.0.1", "::1"] {
            assert!(
                config.allowed_hosts.iter().any(|h| h == loopback),
                "{loopback} should survive an operator-supplied host, got {:?}",
                config.allowed_hosts
            );
        }
    }

    /// rmcp skips `Origin` validation while the list is empty, so naming CORS
    /// origins has to reach the transport for the check to run at all.
    #[test]
    fn streamable_config_carries_cors_origins_as_allowed_origins() {
        let config = build_streamable_config(&HttpSecurityConfig {
            cors_origins: vec!["https://app.example".to_string()],
            ..Default::default()
        });

        assert_eq!(config.allowed_origins, vec!["https://app.example"]);
    }

    /// `*` matches no origin in rmcp, so copying it into the list would turn
    /// "allow every browser" into "refuse every browser".
    #[test]
    fn streamable_config_leaves_origins_unset_for_wildcard_cors() {
        let config = build_streamable_config(&HttpSecurityConfig {
            cors_origins: vec!["*".to_string()],
            ..Default::default()
        });

        assert!(
            config.allowed_origins.is_empty(),
            "wildcard CORS should leave Origin validation off, got {:?}",
            config.allowed_origins
        );
    }

    /// The loopback names rmcp defaults to have to survive the parser, or a
    /// plain `skrills serve --http` would refuse to start.
    #[test]
    fn host_allow_list_parses_the_rmcp_defaults() {
        let defaults = build_streamable_config(&HttpSecurityConfig::default()).allowed_hosts;

        let parsed = parse_host_allow_list(&defaults).expect("rmcp defaults should parse");

        assert_eq!(
            parsed,
            vec![
                HostAuthority {
                    host: "localhost".to_string(),
                    port: None
                },
                HostAuthority {
                    host: "127.0.0.1".to_string(),
                    port: None
                },
                HostAuthority {
                    host: "::1".to_string(),
                    port: None
                },
            ]
        );
    }

    #[test]
    fn host_allow_list_parses_authorities_with_and_without_ports() {
        let parsed = parse_host_allow_list(&[
            "skrills.internal".to_string(),
            "skrills.internal:8080".to_string(),
            "[::1]:3000".to_string(),
        ])
        .expect("bare authorities should parse");

        assert_eq!(
            parsed,
            vec![
                HostAuthority {
                    host: "skrills.internal".to_string(),
                    port: None
                },
                HostAuthority {
                    host: "skrills.internal".to_string(),
                    port: Some(8080)
                },
                HostAuthority {
                    host: "::1".to_string(),
                    port: Some(3000)
                },
            ]
        );
    }

    /// An entry written by analogy with `cors_origins` matched nothing and left
    /// every client with a 403, so it has to be named at startup instead.
    #[test]
    fn host_allow_list_rejects_entries_that_are_not_authorities() {
        for bad in ["https://skrills.internal:8080", "skrills.internal/mcp"] {
            let Err(error) = parse_host_allow_list(&[bad.to_string()]) else {
                panic!("{bad} should be rejected");
            };

            let error = error.to_string();
            assert!(
                error.contains(bad),
                "the error should name the entry, got: {error}"
            );
        }
    }

    /// A trailing comma in `--allowed-hosts foo.example,` cost nothing before
    /// the entries were parsed, and should not start costing a failed startup.
    #[test]
    fn host_allow_list_skips_blank_entries() {
        let parsed =
            parse_host_allow_list(&["foo.example".to_string(), String::new(), "  ".to_string()])
                .expect("a blank entry should be ignored, not rejected");

        assert_eq!(
            parsed,
            vec![HostAuthority {
                host: "foo.example".to_string(),
                port: None
            }]
        );
    }

    /// An entry without a port is a host name, not a demand that the client
    /// omit the port it connected on.
    #[test]
    fn host_entry_without_port_accepts_any_port() {
        let entry = parse_host_authority("skrills.internal").unwrap();

        assert!(entry.accepts(&parse_host_authority("skrills.internal:8080").unwrap()));
        assert!(entry.accepts(&parse_host_authority("SKRILLS.INTERNAL").unwrap()));
        assert!(!entry.accepts(&parse_host_authority("other.internal:8080").unwrap()));
    }

    #[test]
    fn host_entry_with_port_rejects_another_port() {
        let entry = parse_host_authority("skrills.internal:8080").unwrap();

        assert!(entry.accepts(&parse_host_authority("skrills.internal:8080").unwrap()));
        assert!(!entry.accepts(&parse_host_authority("skrills.internal:9090").unwrap()));
        assert!(!entry.accepts(&parse_host_authority("skrills.internal").unwrap()));
    }

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

            // Ask for the occupied port, should fall back to blocked_port + 1..+9
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
            // If we can't block all 10, skip, another process holds one.
            let base_port: u16 = 19_100;
            let base_addr = SocketAddr::new("127.0.0.1".parse().unwrap(), base_port);

            let mut blockers = Vec::with_capacity(10);
            for offset in 0..10u16 {
                let addr = SocketAddr::new(base_addr.ip(), base_port + offset);
                match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => blockers.push(l),
                    Err(_) => {
                        // Can't block all ports, skip test rather than flake
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
