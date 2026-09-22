//! Integration tests for HTTP transport.
//!
//! These cover the perimeter the transport owns: `Host` validation on every
//! route rather than only the MCP endpoint, `Origin` validation when the
//! operator named CORS origins, a browser preflight surviving bearer auth, and
//! the startup failure when an allow-list entry cannot be parsed.

#![cfg(feature = "http-transport")]

use std::time::Duration;
use tokio::time::timeout;

/// Starts the MCP server on an ephemeral loopback port with the given
/// security config and returns the bind address plus the server task.
///
/// The ephemeral port is discovered and released before the server re-binds
/// it, the same accepted race as `pick_free_port` in the CLI smoke tests: if
/// another process claims the port in the gap, `bind_with_fallback` moves the
/// server to the next port and the assertions below fail rather than pass by
/// accident.
async fn spawn_server(
    security: skrills_server::http_transport::HttpSecurityConfig,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind to ephemeral port");
    let bind = listener.local_addr().unwrap().to_string();
    drop(listener);

    let bind_for_server = bind.clone();
    let handle = tokio::spawn(async move {
        let _ = skrills_server::http_transport::serve_http_with_security(
            || {
                skrills_server::app::SkillService::new_with_ttl(vec![], Duration::from_secs(60))
                    .map_err(std::io::Error::other)
            },
            &bind_for_server,
            security,
            vec![],
            false,
        )
        .await;
    });
    (bind, handle)
}

/// Sends a raw HTTP/1.1 request and returns the whole response. Raw bytes
/// rather than a client library so `Host` and `Origin` are exactly what the
/// test says they are. Connecting is retried until the server task has bound
/// the port.
async fn raw_request(bind: &str, request_line: &str, headers: &[(&str, &str)]) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match tokio::net::TcpStream::connect(bind).await {
            Ok(stream) => break stream,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(e) => panic!("server at {bind} never accepted a connection: {e}"),
        }
    };

    let mut request = format!("{request_line}\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("Connection: close\r\n\r\n");

    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    timeout(Duration::from_secs(2), stream.read_to_end(&mut raw))
        .await
        .expect("response within 2s")
        .unwrap();
    String::from_utf8_lossy(&raw).into_owned()
}

/// First line of a raw response, for example `HTTP/1.1 403 Forbidden`.
fn status_line(response: &str) -> &str {
    response.lines().next().unwrap_or_default()
}

/// Sends `GET <path>` carrying an explicit `Host` and returns the status line.
async fn get_status(bind: &str, path: &str, host: &str) -> String {
    let response = raw_request(bind, &format!("GET {path} HTTP/1.1"), &[("Host", host)]).await;
    status_line(&response).to_string()
}

/// With no operator allow-list, only loopback names pass the DNS rebinding
/// check (RUSTSEC-2026-0189); a foreign Host is refused.
#[tokio::test]
async fn mcp_endpoint_rejects_foreign_host_by_default() {
    let (bind, server) =
        spawn_server(skrills_server::http_transport::HttpSecurityConfig::default()).await;

    let status = get_status(&bind, "/mcp", "evil.example").await;

    server.abort();
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "a Host outside the allow-list should be forbidden, got: {status}"
    );
}

/// `allowed_hosts` from the config file or flag has to reach the transport,
/// otherwise a server bound to a non-loopback address refuses everyone.
///
/// 406 is what an allow-listed request gets here: `GET /mcp` without
/// `Accept: text/event-stream` is refused by the MCP protocol itself, which is
/// proof the request passed both Host checks and reached the handler. A weaker
/// "not 403" assertion would also accept a 500 from broken wiring.
#[tokio::test]
async fn mcp_endpoint_accepts_host_from_allowed_hosts() {
    let security = skrills_server::http_transport::HttpSecurityConfig {
        allowed_hosts: vec!["evil.example".to_string()],
        ..Default::default()
    };
    let (bind, server) = spawn_server(security).await;

    let status = get_status(&bind, "/mcp", "evil.example").await;

    server.abort();
    assert_eq!(
        status, "HTTP/1.1 406 Not Acceptable",
        "an allow-listed Host must reach the MCP handler"
    );
}

/// The dashboard and the REST API are merged ahead of the MCP service, so the
/// transport's own Host check never saw them. A rebound page could read them.
#[tokio::test]
async fn rest_route_rejects_foreign_host() {
    let (bind, server) =
        spawn_server(skrills_server::http_transport::HttpSecurityConfig::default()).await;

    let status = get_status(&bind, "/api/mcp-servers", "evil.example").await;

    server.abort();
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "a REST route must refuse a Host outside the allow-list, got: {status}"
    );
}

/// The Host check must not cost the operator the dashboard on a loopback bind.
#[tokio::test]
async fn rest_route_accepts_loopback_host() {
    let (bind, server) =
        spawn_server(skrills_server::http_transport::HttpSecurityConfig::default()).await;
    let port = bind.rsplit(':').next().unwrap().to_string();

    let status = get_status(&bind, "/api/mcp-servers", &format!("localhost:{port}")).await;

    server.abort();
    assert_eq!(
        status, "HTTP/1.1 200 OK",
        "a loopback Host must still be served"
    );
}

/// A browser preflight carries no `Authorization` header, so auth running
/// outside CORS made every cross-origin request from a browser fail with 401.
#[tokio::test]
async fn cors_preflight_is_answered_when_auth_is_enabled() {
    let security = skrills_server::http_transport::HttpSecurityConfig {
        auth_token: Some("secret-token".to_string()),
        cors_origins: vec!["http://app.example".to_string()],
        ..Default::default()
    };
    let (bind, server) = spawn_server(security).await;
    let port = bind.rsplit(':').next().unwrap().to_string();

    let response = raw_request(
        &bind,
        "OPTIONS /api/skills HTTP/1.1",
        &[
            ("Host", &format!("localhost:{port}")),
            ("Origin", "http://app.example"),
            ("Access-Control-Request-Method", "GET"),
        ],
    )
    .await;

    server.abort();
    let status = status_line(&response);
    assert!(
        !status.starts_with("HTTP/1.1 401"),
        "a preflight must not be refused for missing auth, got: {status}"
    );
    assert!(
        response
            .to_ascii_lowercase()
            .contains("access-control-allow-origin: http://app.example"),
        "preflight response should carry the allowed origin:\n{response}"
    );
}

/// Moving CORS outside auth must buy the preflight through and nothing else:
/// an ordinary request still needs the bearer token.
#[tokio::test]
async fn rest_route_still_requires_auth_when_cors_is_configured() {
    let security = skrills_server::http_transport::HttpSecurityConfig {
        auth_token: Some("secret-token".to_string()),
        cors_origins: vec!["http://app.example".to_string()],
        ..Default::default()
    };
    let (bind, server) = spawn_server(security).await;
    let port = bind.rsplit(':').next().unwrap().to_string();

    let response = raw_request(
        &bind,
        "GET /api/mcp-servers HTTP/1.1",
        &[
            ("Host", &format!("localhost:{port}")),
            ("Origin", "http://app.example"),
        ],
    )
    .await;

    server.abort();
    let status = status_line(&response);
    assert!(
        status.starts_with("HTTP/1.1 401"),
        "a request with no bearer token should still be refused, got: {status}"
    );
}

/// Naming CORS origins should also switch on the transport's `Origin` check,
/// which rmcp skips while the list is empty.
#[tokio::test]
async fn mcp_endpoint_rejects_origin_outside_cors_origins() {
    let security = skrills_server::http_transport::HttpSecurityConfig {
        cors_origins: vec!["http://app.example".to_string()],
        ..Default::default()
    };
    let (bind, server) = spawn_server(security).await;
    let port = bind.rsplit(':').next().unwrap().to_string();

    let response = raw_request(
        &bind,
        "GET /mcp HTTP/1.1",
        &[
            ("Host", &format!("localhost:{port}")),
            ("Origin", "http://evil.example"),
        ],
    )
    .await;

    server.abort();
    let status = status_line(&response);
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "an Origin outside the CORS list should be forbidden, got: {status}"
    );
}

/// An allow-list entry written with a scheme parses as no authority at all,
/// which used to leave every client with a 403 and nothing in the log.
#[tokio::test]
async fn startup_fails_when_an_allowed_host_carries_a_scheme() {
    let security = skrills_server::http_transport::HttpSecurityConfig {
        allowed_hosts: vec!["https://skrills.internal:8080".to_string()],
        ..Default::default()
    };

    // Bounded: without the check the call serves forever instead of returning.
    let error = timeout(
        Duration::from_secs(5),
        skrills_server::http_transport::serve_http_with_security(
            || {
                skrills_server::app::SkillService::new_with_ttl(vec![], Duration::from_secs(60))
                    .map_err(std::io::Error::other)
            },
            "127.0.0.1:0",
            security,
            vec![],
            false,
        ),
    )
    .await
    .expect("startup should return instead of serving")
    .expect_err("a host entry with a scheme should fail startup");

    assert!(
        error.to_string().contains("https://skrills.internal:8080"),
        "the error should name the offending entry, got: {error}"
    );
}
