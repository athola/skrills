//! CLI-dispatch smoke tests for the cold-window subcommand
//! (v0.8.1 follow-up; closes the gap left open by TASK-031b).
//!
//! ## Why this exists
//!
//! TASK-031 wired `Commands::ColdWindow(ColdWindowArgs)` into
//! `app/mod.rs::run()` so `skrills cold-window …` dispatches.
//! Existing browser-integration tests construct the axum `Router`
//! directly (`cold_window_routes(state)`) and never go through CLI
//! dispatch — so when the dispatch arm was initially missing,
//! `cargo test` was green but `make cold-window` (real binary) failed
//! with "unrecognized subcommand". Dogfooding caught it; T031b fixed
//! it; this file ensures no future refactor can re-introduce the
//! defect silently.
//!
//! ## Test pyramid coverage
//!
//! - `cold_window_help_dispatches`: cheap (~30 ms). Asserts the
//!   subcommand is registered with clap. This alone would have caught
//!   T031b.
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

/// Issue a raw HTTP/1.1 GET /dashboard to the given port and return
/// the full response (status line + headers + body). `Connection:
/// close` makes the server close after one response so
/// `read_to_end` terminates promptly.
fn http_get_dashboard(port: u16) -> std::io::Result<String> {
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(b"GET /dashboard HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Asserts `skrills cold-window --help` exits 0 with the expected
/// flags listed. This single test would have caught TASK-031b
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
        "`skrills cold-window --help` exited non-zero — \
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
    // Using a closure so we can run cleanup unconditionally below.
    let response = (|| -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(s) = http_get_dashboard(port) {
                return Some(s);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    })();

    // Always tear down the child before asserting so a failure
    // doesn't leak a process that holds the test port hostage.
    let _ = child.kill();
    let _ = child.wait();

    let body = response.expect(
        "dashboard did not respond within 5 s — \
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
