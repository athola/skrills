//! Manages application state and configuration.
//!
//! This crate provides utilities for:
//! - Reading environment variables for configuration.
//! - Handling manifest settings and runtime overrides.
//!
//! Note: Legacy pin/history/autoload persistence was removed in 0.3.1
//! as skill loading is now handled by Claude/Codex directly.
//!
//! # Examples
//!
//! ```
//! use skrills_state::{cache_ttl, ManifestSettings};
//!
//! let ttl = cache_ttl(&|| Ok(ManifestSettings::default()));
//! assert!(ttl.as_millis() > 0);
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Error type for state operations.
pub type Error = anyhow::Error;
/// Result type for state operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Environment and configuration utilities.
pub mod env;

pub use env::{
    cache_ttl, env_auto_persist, env_diag, env_include_claude, env_include_marketplace,
    extra_dirs_from_env, home_dir, load_manifest_settings, manifest_file, runtime_overrides_path,
    ManifestSettings,
};
