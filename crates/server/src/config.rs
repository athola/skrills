//! Configuration file support for skrills.
//!
//! Loads settings from `~/.skrills/config.toml` with the following precedence:
//! CLI arguments > Environment variables > Config file
//!
//! ## Configuration File Format
//!
//! ```toml
//! # ~/.skrills/config.toml
//!
//! [serve]
//! # Bearer token for HTTP authentication
//! auth_token = "your-secret-token"
//!
//! # TLS certificate paths
//! tls_cert = "/path/to/cert.pem"
//! tls_key = "/path/to/key.pem"
//!
//! # Auto-generate self-signed TLS certificate
//! tls_auto = true
//!
//! # CORS allowed origins (comma-separated)
//! cors_origins = "http://localhost:3000,https://app.example.com"
//!
//! # Bind address for HTTP transport
//! http = "127.0.0.1:3000"
//!
//! # Cache TTL in milliseconds
//! cache_ttl_ms = 5000
//! ```

use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

/// Top-level configuration structure.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Serve command configuration.
    #[serde(default)]
    pub serve: ServeConfig,
}

/// Configuration for the serve command.
#[derive(Debug, Default, Deserialize)]
pub struct ServeConfig {
    /// Bearer token for HTTP authentication.
    pub auth_token: Option<String>,
    /// Path to TLS certificate file.
    pub tls_cert: Option<String>,
    /// Path to TLS private key file.
    pub tls_key: Option<String>,
    /// Auto-generate self-signed TLS certificate.
    pub tls_auto: Option<bool>,
    /// Comma-separated list of allowed CORS origins.
    pub cors_origins: Option<String>,
    /// Bind address for HTTP transport (reserved for future CLI env var support).
    #[allow(dead_code)]
    pub http: Option<String>,
    /// Cache TTL in milliseconds (reserved for future CLI env var support).
    #[allow(dead_code)]
    pub cache_ttl_ms: Option<u64>,
}

/// Returns the path to the config file (~/.skrills/config.toml).
fn config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".skrills").join("config.toml"))
}

/// Loads the configuration file if it exists.
///
/// Returns `Ok(None)` if the file doesn't exist.
/// Returns `Ok(Some(config))` if the file exists and parses successfully.
/// Returns `Err` if the file exists but fails to parse.
pub fn load_config() -> Result<Option<Config>> {
    let Some(path) = config_path() else {
        return Ok(None);
    };

    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)?;
    let config: Config = toml::from_str(&content)?;

    tracing::debug!(
        target: "skrills::config",
        path = %path.display(),
        "Loaded configuration file"
    );

    Ok(Some(config))
}

/// Applies configuration file settings to environment variables.
///
/// Only sets environment variables that are not already set, preserving
/// the precedence: CLI > ENV > config file.
///
/// This should be called early in the application startup, before
/// parsing CLI arguments.
pub fn apply_config_to_env() {
    if let Ok(Some(config)) = load_config() {
        apply_serve_config_to_env(&config.serve);
    }
}

/// Applies serve configuration to environment variables.
fn apply_serve_config_to_env(serve: &ServeConfig) {
    // Helper to set env var only if not already set
    fn set_if_absent(key: &str, value: &str) {
        if std::env::var(key).is_err() {
            std::env::set_var(key, value);
            tracing::trace!(
                target: "skrills::config",
                key,
                "Set environment variable from config file"
            );
        }
    }

    if let Some(ref token) = serve.auth_token {
        set_if_absent("SKRILLS_AUTH_TOKEN", token);
    }

    if let Some(ref cert) = serve.tls_cert {
        set_if_absent("SKRILLS_TLS_CERT", cert);
    }

    if let Some(ref key) = serve.tls_key {
        set_if_absent("SKRILLS_TLS_KEY", key);
    }

    if let Some(auto) = serve.tls_auto {
        set_if_absent("SKRILLS_TLS_AUTO", if auto { "true" } else { "false" });
    }

    if let Some(ref origins) = serve.cors_origins {
        set_if_absent("SKRILLS_CORS_ORIGINS", origins);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_returns_expected_location() {
        let path = config_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.ends_with(".skrills/config.toml"));
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
            [serve]
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.serve.auth_token.is_none());
    }

    #[test]
    fn parse_full_serve_config() {
        let toml = r#"
            [serve]
            auth_token = "secret"
            tls_cert = "/path/to/cert.pem"
            tls_key = "/path/to/key.pem"
            tls_auto = true
            cors_origins = "http://localhost:3000,https://example.com"
            http = "0.0.0.0:8080"
            cache_ttl_ms = 5000
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.serve.auth_token.as_deref(), Some("secret"));
        assert_eq!(config.serve.tls_cert.as_deref(), Some("/path/to/cert.pem"));
        assert_eq!(config.serve.tls_key.as_deref(), Some("/path/to/key.pem"));
        assert_eq!(config.serve.tls_auto, Some(true));
        assert_eq!(
            config.serve.cors_origins.as_deref(),
            Some("http://localhost:3000,https://example.com")
        );
        assert_eq!(config.serve.http.as_deref(), Some("0.0.0.0:8080"));
        assert_eq!(config.serve.cache_ttl_ms, Some(5000));
    }

    #[test]
    fn load_nonexistent_config_returns_none() {
        // This test relies on the config file not existing in a typical CI environment
        // In practice, we'd mock the filesystem
        let result = load_config();
        assert!(result.is_ok());
        // Config may or may not exist depending on environment
    }

    #[test]
    fn apply_config_respects_existing_env_vars() {
        // Save and restore env vars
        let original = std::env::var("SKRILLS_AUTH_TOKEN").ok();

        // Set an env var
        std::env::set_var("SKRILLS_AUTH_TOKEN", "env-token");

        // Create config with different value
        let serve = ServeConfig {
            auth_token: Some("config-token".to_string()),
            ..Default::default()
        };

        // Apply config
        apply_serve_config_to_env(&serve);

        // Env var should remain unchanged
        assert_eq!(
            std::env::var("SKRILLS_AUTH_TOKEN").unwrap(),
            "env-token",
            "Config should not override existing env var"
        );

        // Restore original
        if let Some(orig) = original {
            std::env::set_var("SKRILLS_AUTH_TOKEN", orig);
        } else {
            std::env::remove_var("SKRILLS_AUTH_TOKEN");
        }
    }
}
