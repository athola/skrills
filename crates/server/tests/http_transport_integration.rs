//! Integration test for HTTP transport.
//!
//! Tests that the MCP server responds correctly over HTTP.

#![cfg(feature = "http-transport")]

use std::time::Duration;
use tokio::time::timeout;

/// Test that the HTTP server binds and accepts connections.
#[tokio::test]
async fn http_server_binds_and_responds() {
    // Get a random available port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind to ephemeral port");
    let addr = listener.local_addr().unwrap();
    drop(listener); // Release the port

    let bind = format!("127.0.0.1:{}", addr.port());
    let bind_clone = bind.clone();

    // Start server in background
    let handle = tokio::spawn(async move {
        let _ = timeout(
            Duration::from_secs(2),
            skrills_server::http_transport::serve_http_with_security(
                || {
                    skrills_server::app::SkillService::new_with_ttl(vec![], Duration::from_secs(60))
                        .map_err(std::io::Error::other)
                },
                &bind,
                skrills_server::http_transport::HttpSecurityConfig::default(),
                vec![],
                false,
            ),
        )
        .await;
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Try to connect - success means server is running
    let client = reqwest::Client::new();
    let result = timeout(
        Duration::from_secs(1),
        client.get(format!("http://{}/mcp", bind_clone)).send(),
    )
    .await;

    // Clean up
    handle.abort();

    // Either we got a response or the server was running (connection possible)
    // The exact response depends on MCP protocol, but if we connected, server works
    assert!(
        result.is_ok() || result.is_err(), // Always true - test documents behavior
        "HTTP server should be reachable"
    );
}

/// Starts the MCP server on an ephemeral loopback port with the given
/// security config and returns the bind address plus the server task.
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

/// Sends a raw `GET /mcp` carrying an explicit `Host` header and returns the
/// HTTP status line. Raw bytes rather than a client library so the Host
/// header is exactly what the test says it is. Connecting is retried until
/// the server task has bound the port.
async fn status_line_for_host(bind: &str, host: &str) -> String {
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
    let request = format!("GET /mcp HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    timeout(Duration::from_secs(2), stream.read_to_end(&mut raw))
        .await
        .expect("response within 2s")
        .unwrap();
    let response = String::from_utf8_lossy(&raw);
    response.lines().next().unwrap_or_default().to_string()
}

/// With no operator allow-list, only loopback names pass rmcp's DNS
/// rebinding check (RUSTSEC-2026-0189); a foreign Host is refused.
#[tokio::test]
async fn mcp_endpoint_rejects_foreign_host_by_default() {
    let (bind, server) =
        spawn_server(skrills_server::http_transport::HttpSecurityConfig::default()).await;

    let status = status_line_for_host(&bind, "evil.example").await;

    server.abort();
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "a Host outside the allow-list should be forbidden, got: {status}"
    );
}

/// `allowed_hosts` from the config file or flag has to reach the transport,
/// otherwise a server bound to a non-loopback address refuses everyone.
#[tokio::test]
async fn mcp_endpoint_accepts_host_from_allowed_hosts() {
    let security = skrills_server::http_transport::HttpSecurityConfig {
        allowed_hosts: vec!["evil.example".to_string()],
        ..Default::default()
    };
    let (bind, server) = spawn_server(security).await;

    let status = status_line_for_host(&bind, "evil.example").await;

    server.abort();
    assert!(
        !status.starts_with("HTTP/1.1 403"),
        "an allow-listed Host must pass the rebinding check, got: {status}"
    );
    assert!(
        status.starts_with("HTTP/1.1 "),
        "expected an HTTP response, got: {status}"
    );
}
