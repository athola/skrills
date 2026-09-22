//! CLI-dispatch smoke tests for the cold-window subcommand.
//!
//! ## Why this exists
//!
//! `Commands::ColdWindow(ColdWindowArgs)` is wired into
//! `app/mod.rs::run()` so `skrills cold-window …` dispatches.
//! Existing browser-integration tests construct the axum `Router`
//! directly (`cold_window_routes(state)`) and never go through CLI
//! dispatch, so when the dispatch arm was initially missing,
//! `cargo test` was green but `make cold-window` (real binary) failed
//! with "unrecognized subcommand". Dogfooding caught it; this file
//! ensures no future refactor can re-introduce the defect silently.
//!
//! ## Test pyramid coverage
//!
//! - `cold_window_help_dispatches`: cheap (~30 ms). Asserts the
//!   subcommand is registered with clap.
//! - `cold_window_browser_surface_serves_dashboard`: full lifecycle
//!   (~2 s). Spawns the real binary, verifies `/dashboard` returns
//!   `HTTP/1.1 200` with the expected `EventSource` script.

#![cfg(feature = "http-transport")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Pick a free 127.0.0.1 port by binding `:0` and immediately dropping
/// the listener. Race window is real: the port could be claimed by
/// another process between drop and `--port` arg. For a smoke test
/// this is acceptable; the failure mode is "test flake", not silent
/// regression.
fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    listener.local_addr().expect("local_addr").port()
}

/// Issue a raw HTTP/1.1 GET to the given port and path with an explicit
/// `Host` header, returning the full response (status line, headers, and
/// body). `Connection: close` makes the server close after one response so
/// `read_to_end` terminates promptly.
fn http_get(port: u16, path: &str, host: &str) -> std::io::Result<String> {
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Poll `path` on `port` until it answers or the deadline expires.
fn poll_http_get(port: u16, path: &str, host: &str) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(response) = http_get(port, path, host) {
            return Some(response);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Asserts `skrills cold-window --help` exits 0 with the expected
/// flags listed. This single test would have caught the dispatch regression
/// without spawning a server, doing HTTP, or waiting any meaningful
/// time. Cheapest possible smoke for "is the subcommand registered".
#[test]
fn cold_window_help_dispatches() {
    let bin = env!("CARGO_BIN_EXE_skrills");
    let output = Command::new(bin)
        .args(["cold-window", "--help"])
        .output()
        .expect("spawn skrills cold-window --help");
    assert!(
        output.status.success(),
        "`skrills cold-window --help` exited non-zero; \
         likely a missing Commands::ColdWindow arm in app/mod.rs::run() \
         (T031b).\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--browser"),
        "help output missing `--browser` flag:\n{stdout}"
    );
    assert!(
        stdout.contains("--alert-budget"),
        "help output missing `--alert-budget` flag:\n{stdout}"
    );
    assert!(
        stdout.contains("--research-rate"),
        "help output missing `--research-rate` flag:\n{stdout}"
    );
}

/// End-to-end smoke: spawn the real `skrills` binary in browser mode,
/// poll `/dashboard` until it returns, assert HTTP/1.1 200 plus the
/// expected SSE-bootstrap script. Validates that ColdWindowEngine,
/// ColdWindowDashboardState, axum router, tokio runtime, and the
/// signal handler all wire up correctly via the CLI dispatch path.
#[test]
fn cold_window_browser_surface_serves_dashboard() {
    let bin = env!("CARGO_BIN_EXE_skrills");
    let port = pick_free_port();
    let mut child = Command::new(bin)
        .args(["cold-window", "--browser", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn skrills cold-window");

    // Poll until the dashboard responds or the deadline expires.
    let response = poll_http_get(port, "/dashboard", "127.0.0.1");

    // Always tear down the child before asserting so a failure
    // doesn't leak a process that holds the test port hostage.
    let _ = child.kill();
    let _ = child.wait();

    let body = response.expect(
        "dashboard did not respond within 5 s; \
         either the binary failed to bind or the dispatch arm regressed",
    );
    assert!(
        body.contains("HTTP/1.1 200"),
        "dashboard did not return 200:\n{body}"
    );
    assert!(
        body.contains("EventSource"),
        "dashboard body missing EventSource script:\n{body}"
    );
    assert!(
        body.contains("/dashboard.sse"),
        "dashboard body missing SSE endpoint reference:\n{body}"
    );
}

/// `serve --allowed-hosts` has to land in `HttpSecurityConfig` and from there
/// in the transport. Clap parsing and the transport are covered separately, so
/// dropping the field on the way between them broke nothing in the test suite
/// while leaving the operator with a 403 for every request.
///
/// 406 is the allow-listed answer: `GET /mcp` without
/// `Accept: text/event-stream` is refused by the MCP protocol itself, which is
/// only reached once the Host check has passed.
#[test]
fn serve_allowed_hosts_flag_reaches_the_transport() {
    let bin = env!("CARGO_BIN_EXE_skrills");
    let port = pick_free_port();
    // An empty HOME keeps the developer's ~/.skrills/config.toml out of the
    // child: an auth_token there would answer 401 and tls_auto would answer
    // TLS bytes, neither of which this test is about.
    let home = tempfile::tempdir().expect("temp HOME");
    let mut child = Command::new(bin)
        .args([
            "serve",
            "--http",
            &format!("127.0.0.1:{port}"),
            "--allowed-hosts",
            "foo.example",
        ])
        .env("HOME", home.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn skrills serve");

    let response = poll_http_get(port, "/mcp", "foo.example");

    let _ = child.kill();
    let _ = child.wait();

    let response = response.expect("server did not answer /mcp within 5 s");
    assert!(
        response.contains("HTTP/1.1 406"),
        "an allow-listed Host should reach the MCP handler, got:\n{response}"
    );
}
