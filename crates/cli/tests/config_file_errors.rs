//! SB-7: a `~/.skrills/config.toml` that does not parse.
//!
//! The file may hold `[serve] auth_token`. When a parse error was only a
//! warning, a typo next to the token started `serve` without authentication.
//! `serve` (and the bare `skrills`, which serves) now refuses to start; every
//! other command keeps the warning and runs.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// A config file with an unquoted bind address, which TOML rejects.
fn home_with_broken_config() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".skrills");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.toml"),
        "[serve]\nauth_token = \"secret\"\nhttp = 127.0.0.1:3000\n",
    )
    .unwrap();
    home
}

/// Runs `skrills <args>` with stdin closed, killing it after 20 s so a server
/// that does start fails the test instead of hanging it.
fn run(home: &Path, args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_skrills"))
        .args(args)
        .env("HOME", home)
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG")
        .env_remove("SKRILLS_AUTH_TOKEN")
        .env_remove("SKRILLS_HTTP")
        .env_remove("SKRILLS_SKILL_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn skrills");
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().expect("poll child").is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().expect("collect output")
}

#[test]
fn serve_refuses_to_start_when_the_config_file_does_not_parse() {
    let home = home_with_broken_config();

    let output = run(home.path(), &["serve"]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "serve must not start on an unparseable config file; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("config.toml") && stderr.contains("refusing to serve"),
        "the error should name the file and say serve refused; stderr:\n{stderr}"
    );
}

#[test]
fn bare_skrills_refuses_to_start_when_the_config_file_does_not_parse() {
    let home = home_with_broken_config();

    let output = run(home.path(), &[]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the default command serves, so it must refuse too; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("config.toml") && stderr.contains("refusing to serve"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn other_commands_warn_and_run_when_the_config_file_does_not_parse() {
    let home = home_with_broken_config();

    let output = run(home.path(), &["analyze", "--format", "json"]);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "a command that does not serve should still run; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("config.toml"),
        "the parse error should still be reported; stderr:\n{stderr}"
    );
}
