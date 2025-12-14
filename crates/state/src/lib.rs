//! Manages application state and configuration.
//!
//! This crate provides utilities for:
//! - Reading environment variables for configuration.
//! - Handling manifest settings and runtime overrides.
//!
//! Note: Legacy pin/history/autoload persistence was removed in 0.3.1
//! as skill loading is now handled by Claude/Codex directly.

pub mod env;
pub mod persistence;

pub use env::{
    cache_ttl, env_diag, env_include_claude, env_include_marketplace, extra_dirs_from_env,
    home_dir, load_manifest_settings, manifest_file, runtime_overrides_path, ManifestSettings,
};
