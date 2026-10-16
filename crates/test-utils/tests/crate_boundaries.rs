//! Crate-boundary invariants recorded in ADRs.
//!
//! Each test guards a decision, not a behavior. When one fails, the change
//! either reverts the coupling, or revises the ADR it cites and this test
//! together; editing the assertion alone hides the decision.

use std::fs;
use std::path::{Path, PathBuf};

fn crates_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/test-utils.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/ is above crates/test-utils")
        .to_path_buf()
}

fn rust_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("entry in {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

fn declares_platform_enum(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        let line = line.strip_prefix("pub(crate) ").unwrap_or(line);
        let line = line.strip_prefix("pub ").unwrap_or(line);
        line.strip_prefix("enum Platform")
            .is_some_and(|rest| rest.starts_with([' ', '{', '<']))
    })
}

/// ADR 0010: token counting must build without skrills-intelligence, so the
/// dependency exists only behind the `intelligence-scorer` feature.
#[test]
fn analyze_depends_on_intelligence_only_as_an_optional_dependency() {
    let manifest_path = crates_dir().join("analyze").join("Cargo.toml");
    let manifest: toml::Table = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()))
        .parse()
        .unwrap_or_else(|e| panic!("parse {}: {e}", manifest_path.display()));

    let intelligence = manifest
        .get("dependencies")
        .and_then(|deps| deps.get("skrills-intelligence"))
        .expect("skrills-analyze lists skrills-intelligence for the hint scorer");
    assert_eq!(
        intelligence.get("optional").and_then(toml::Value::as_bool),
        Some(true),
        "skrills-intelligence must stay optional in skrills-analyze (ADR 0010)"
    );
}

/// Commit 9a9324e: platform identity is one enum, `skrills_sync::Platform`.
/// A second definition brings back the duplicated routing it removed.
#[test]
fn only_skrills_sync_defines_a_platform_enum() {
    let mut sources = Vec::new();
    rust_sources(&crates_dir(), &mut sources);

    let definers: Vec<PathBuf> = sources
        .into_iter()
        .filter(|path| {
            let text =
                fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            declares_platform_enum(&text)
        })
        .map(|path| {
            path.strip_prefix(crates_dir())
                .expect("source is under crates/")
                .to_path_buf()
        })
        .collect();

    assert_eq!(
        definers,
        vec![PathBuf::from("sync/src/platform.rs")],
        "only skrills_sync may define `enum Platform`"
    );
}

/// ADR 0009: the CLI uses skrills-server only to start the server, so the
/// server's public modules are the server's own. Setup, config, discovery and
/// AGENTS.md helpers moved to their own crates and must not come back.
#[test]
fn server_exports_only_server_modules() {
    const SERVER_MODULES: &[&str] = &[
        "api",
        "app",
        "http_transport",
        "mcp_gateway",
        "tls_auto",
        "tool_schemas",
        "trace",
        "ui",
    ];
    let lib_path = crates_dir().join("server").join("src").join("lib.rs");
    let lib = fs::read_to_string(&lib_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", lib_path.display()));

    let exported: Vec<&str> = lib
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("pub mod "))
        .map(|rest| rest.trim_end_matches(';').trim())
        .collect();
    let unexpected: Vec<&&str> = exported
        .iter()
        .filter(|module| !SERVER_MODULES.contains(module))
        .collect();
    assert!(
        unexpected.is_empty(),
        "skrills-server exports modules outside the ADR 0009 server set: {unexpected:?}"
    );

    let reexports: Vec<&str> = lib
        .lines()
        .filter(|line| line.trim_start().starts_with("pub use "))
        .collect();
    assert!(
        reexports.is_empty(),
        "skrills-server re-exports other crates (ADR 0009 keeps none): {reexports:?}"
    );
}

#[test]
fn declares_platform_enum_matches_definitions_but_not_lookalikes() {
    assert!(declares_platform_enum("pub enum Platform {"));
    assert!(declares_platform_enum("    pub(crate) enum Platform {"));
    assert!(declares_platform_enum("enum Platform<T> {"));
    assert!(!declares_platform_enum("pub enum PlatformKind {"));
    assert!(!declares_platform_enum("use crate::Platform;"));
    assert!(!declares_platform_enum(
        "// the old enum Platform lived here"
    ));
}
