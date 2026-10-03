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
//!   `HTTP/1.1 200` under the CSP, and serves the `EventSource` script it loads.

#![cfg(feature = "http-transport")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Spawns `skrills <args>` with logs on a piped stderr and waits for the log
/// line containing `marker`, returning the address in its `field=` value.
///
/// A reader thread forwards stderr lines, so a child that never logs the
/// line fails the test after the deadline instead of hanging it.
fn spawn_and_read_bound_addr(
    args: &[&str],
    home: &std::path::Path,
    marker: &str,
    field: &str,
) -> (Child, SocketAddr) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args(args)
        .env("HOME", home)
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG")
        // A developer's shell settings would otherwise decide auth, TLS and
        // the Host allow-list for the child.
        .env_remove("SKRILLS_AUTH_TOKEN")
        .env_remove("SKRILLS_TLS_CERT")
        .env_remove("SKRILLS_TLS_KEY")
        .env_remove("SKRILLS_TLS_AUTO")
        .env_remove("SKRILLS_CORS_ORIGINS")
        .env_remove("SKRILLS_ALLOWED_HOSTS")
        .env_remove("SKRILLS_HTTP")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn skrills");
    let stderr = child.stderr.take().expect("piped stderr");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(line) => {
                if line.contains(marker) {
                    if let Some(addr) = field_value(&line, field) {
                        return (child, addr);
                    }
                }
                seen.push(line);
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!(
        "no `{marker}` log line with {field}=<addr> within 10 s; stderr so far:\n{}",
        seen.join("\n")
    );
}

/// The socket address in `field=<addr>` on a log line.
fn field_value(line: &str, field: &str) -> Option<SocketAddr> {
    let needle = format!("{field}=");
    let start = line.find(&needle)? + needle.len();
    line[start..].split_whitespace().next()?.parse().ok()
}

/// Issue a raw HTTP/1.1 GET to the given port and path with an explicit
/// `Host` header, returning the full response (status line, headers, and
/// body). `Connection: close` makes the server close after one response so
/// `read_to_end` terminates promptly.
fn http_get(addr: SocketAddr, path: &str, host: &str) -> std::io::Result<String> {
    http_get_with_headers(addr, path, host, "")
}

/// [`http_get`] with extra header lines, each ending in `\r\n`.
fn http_get_with_headers(
    addr: SocketAddr,
    path: &str,
    host: &str,
    extra_headers: &str,
) -> std::io::Result<String> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra_headers}Connection: close\r\n\r\n")
            .as_bytes(),
    )?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Poll `path` on `port` until it answers or the deadline expires.
fn poll_http_get(addr: SocketAddr, path: &str, host: &str) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(response) = http_get(addr, path, host) {
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

/// `--skill-dir` help must describe what the producer walks: the given
/// directories plus `SKRILLS_EXTRA_SKILL_DIRS`, never the default skill
/// roots (`merge_extra_dirs` does not add them).
#[test]
fn cold_window_skill_dir_help_matches_what_is_walked() {
    let bin = env!("CARGO_BIN_EXE_skrills");
    let output = Command::new(bin)
        .args(["cold-window", "--help"])
        .output()
        .expect("spawn skrills cold-window --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let flat = stdout.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("SKRILLS_EXTRA_SKILL_DIRS"),
        "--skill-dir help should name SKRILLS_EXTRA_SKILL_DIRS:\n{stdout}"
    );
    assert!(
        !flat.contains("and the default skill roots"),
        "--skill-dir help claims the default skill roots are walked:\n{stdout}"
    );
    assert!(
        flat.contains("default skill roots are not walked"),
        "--skill-dir help should say the default skill roots are not walked:\n{stdout}"
    );
}

/// End-to-end smoke: spawn the real `skrills` binary in browser mode,
/// poll `/dashboard` until it returns, assert HTTP/1.1 200, the CSP, and
/// the SSE-bootstrap script the page loads. Validates that ColdWindowEngine,
/// ColdWindowDashboardState, axum router, tokio runtime, and the
/// signal handler all wire up correctly via the CLI dispatch path.
#[test]
fn cold_window_browser_surface_serves_dashboard() {
    // Port 0: the OS picks a free port and the bound address is read back
    // from the startup log, so there is no bind/drop/re-bind race.
    let home = tempfile::tempdir().expect("temp HOME");
    let (mut child, addr) = spawn_and_read_bound_addr(
        &["cold-window", "--browser", "--port", "0"],
        home.path(),
        "browser surface listening",
        "addr",
    );
    assert_ne!(addr.port(), 0, "the log must carry the bound port");

    let response = poll_http_get(addr, "/dashboard", "127.0.0.1");
    let script = http_get(addr, "/static/cold_window.js", "127.0.0.1");

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
        body.contains("content-security-policy: default-src 'self'; script-src 'self'"),
        "dashboard served without the CSP:\n{body}"
    );
    assert!(
        body.contains(r#"<script src="/static/cold_window.js"></script>"#),
        "dashboard body does not load its script:\n{body}"
    );
    let script = script.expect("/static/cold_window.js did not respond");
    assert!(
        script.contains("HTTP/1.1 200") && script.contains("EventSource"),
        "page script missing EventSource bootstrap:\n{script}"
    );
    assert!(
        script.contains("/dashboard.sse"),
        "page script missing SSE endpoint reference:\n{script}"
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
    // An empty HOME keeps the developer's ~/.skrills/config.toml out of the
    // child: an auth_token there would answer 401 and tls_auto would answer
    // TLS bytes, neither of which this test is about.
    let home = tempfile::tempdir().expect("temp HOME");
    let (mut child, addr) = spawn_and_read_bound_addr(
        &[
            "serve",
            "--http",
            "127.0.0.1:0",
            "--allowed-hosts",
            "foo.example",
        ],
        home.path(),
        "MCP HTTP server listening",
        "bind",
    );

    let response = poll_http_get(addr, "/mcp", "foo.example");

    let _ = child.kill();
    let _ = child.wait();

    let response = response.expect("server did not answer /mcp within 5 s");
    assert!(
        response.contains("HTTP/1.1 406"),
        "an allow-listed Host should reach the MCP handler, got:\n{response}"
    );
}

/// SB-8: `[serve] auth_token` in `~/.skrills/config.toml` turns auth on for
/// `serve --http`, so a request without the token is refused and one with it
/// is served. The config loader does not export the token to the
/// environment, so this proves `serve` reads it from the file.
#[test]
fn serve_config_file_auth_token_enables_auth() {
    let home = tempfile::tempdir().expect("temp HOME");
    std::fs::create_dir_all(home.path().join(".skrills")).unwrap();
    std::fs::write(
        home.path().join(".skrills/config.toml"),
        "[serve]\nauth_token = \"file-token\"\n",
    )
    .unwrap();
    let (mut child, addr) = spawn_and_read_bound_addr(
        &["serve", "--http", "127.0.0.1:0"],
        home.path(),
        "MCP HTTP server listening",
        "bind",
    );

    let response = poll_http_get(addr, "/mcp", "127.0.0.1");
    let authorized = http_get_with_headers(
        addr,
        "/api/skills",
        "127.0.0.1",
        "Authorization: Bearer file-token\r\n",
    );

    let _ = child.kill();
    let _ = child.wait();

    let response = response.expect("server did not answer /mcp within 5 s");
    assert!(
        response.contains("HTTP/1.1 401"),
        "a request without the config-file token must be refused, got:\n{response}"
    );
    let authorized = authorized.expect("server did not answer /api/skills");
    assert!(
        authorized.starts_with("HTTP/1.1 200"),
        "a request carrying the config-file token must be served, got:\n{authorized}"
    );
}
