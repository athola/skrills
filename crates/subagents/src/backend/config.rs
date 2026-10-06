use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Url;

#[derive(Debug, Clone)]
pub struct AdapterConfig {
    pub api_key: String,
    pub base_url: Url,
    pub model: String,
    pub timeout: Duration,
}

impl AdapterConfig {
    pub fn from_env(
        prefix: &str,
        default_model: &str,
        default_base: &str,
        default_timeout_ms: u64,
    ) -> Result<Self> {
        let key_var = format!("SKRILLS_{}_API_KEY", prefix);
        let api_key = std::env::var(&key_var)
            .with_context(|| format!("{key_var} must be set for subagents"))?;

        let base_var = format!("SKRILLS_{}_BASE_URL", prefix);
        let base_url = std::env::var(&base_var)
            .ok()
            .unwrap_or_else(|| default_base.to_string());
        let base_url =
            Url::parse(&base_url).with_context(|| format!("invalid {base_var} url: {base_url}"))?;

        let model_var = format!("SKRILLS_{}_MODEL", prefix);
        let model = std::env::var(&model_var).unwrap_or_else(|_| default_model.to_string());

        let timeout_var = format!("SKRILLS_{}_TIMEOUT_MS", prefix);
        let timeout_ms = parse_timeout_ms(
            &timeout_var,
            std::env::var(&timeout_var).ok(),
            default_timeout_ms,
        );

        Ok(Self {
            api_key,
            base_url,
            model,
            timeout: Duration::from_millis(timeout_ms),
        })
    }
}

/// Reads a millisecond timeout, warning when a set value does not parse so a
/// setting such as `30s` is not ignored without a trace.
pub(crate) fn parse_timeout_ms(var: &str, raw: Option<String>, default_ms: u64) -> u64 {
    match raw {
        None => default_ms,
        Some(raw) => raw.trim().parse::<u64>().unwrap_or_else(|_| {
            tracing::warn!(
                variable = var,
                value = %raw,
                default_ms,
                "invalid timeout (expected whole milliseconds); using the default"
            );
            default_ms
        }),
    }
}

/// Joins `path` onto `base`, treating `base` as a directory.
///
/// `Url::join` replaces the last path segment of a base without a trailing
/// slash, so `https://api.openai.com/v1` joined with `chat/completions` would
/// lose `v1`. The base is given a trailing slash first, and a failed join is
/// an error rather than a silent fall back to the bare base.
pub(crate) fn endpoint(base: &Url, path: &str) -> Result<Url> {
    let mut base = base.clone();
    if !base.path().ends_with('/') {
        let with_slash = format!("{}/", base.path());
        base.set_path(&with_slash);
    }
    base.join(path)
        .with_context(|| format!("cannot join {path:?} onto API base {base}"))
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    #[test]
    fn endpoint_keeps_a_version_segment_without_trailing_slash() {
        let base = Url::parse("https://api.openai.com/v1").unwrap();
        assert_eq!(
            endpoint(&base, "chat/completions").unwrap().as_str(),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn endpoint_accepts_a_base_with_trailing_slash() {
        let base = Url::parse("https://api.anthropic.com/v1/").unwrap();
        assert_eq!(
            endpoint(&base, "messages").unwrap().as_str(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn endpoint_on_a_bare_host() {
        let base = Url::parse("https://proxy.local").unwrap();
        assert_eq!(
            endpoint(&base, "messages").unwrap().as_str(),
            "https://proxy.local/messages"
        );
    }

    #[test]
    fn endpoint_fails_on_a_base_that_cannot_hold_a_path() {
        let base = Url::parse("mailto:someone@example.com").unwrap();
        assert!(endpoint(&base, "messages").is_err());
    }

    #[test]
    fn unparseable_timeout_falls_back_to_the_default() {
        assert_eq!(parse_timeout_ms("X", Some("30s".into()), 500), 500);
        assert_eq!(parse_timeout_ms("X", Some("250".into()), 500), 250);
        assert_eq!(parse_timeout_ms("X", None, 500), 500);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_test_utils::{env_guard, set_env_var};

    /// BDD Test: Configuration loading with all default values
    ///
    /// Given: Environment variables are set with minimal required values
    /// When: AdapterConfig::from_env is called
    /// Then: Configuration uses defaults for optional fields
    #[test]
    fn given_minimal_env_when_from_env_then_uses_defaults() {
        let _g = env_guard();
        // GIVEN: Only API key is set
        let _key = set_env_var("SKRILLS_CLAUDE_API_KEY", Some("test-key-123"));
        let _url = set_env_var("SKRILLS_CLAUDE_BASE_URL", None);
        let _model = set_env_var("SKRILLS_CLAUDE_MODEL", None);
        let _timeout = set_env_var("SKRILLS_CLAUDE_TIMEOUT_MS", None);

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: Defaults are applied correctly
        let config = result.expect("loading config with minimal env should succeed");
        assert_eq!(config.api_key, "test-key-123");
        assert_eq!(config.base_url.as_str(), "https://api.anthropic.com/");
        assert_eq!(config.model, "claude-3.5");
        assert_eq!(config.timeout, Duration::from_millis(30000));
    }

    /// BDD Test: Configuration loading with custom values
    ///
    /// Given: All environment variables are set with custom values
    /// When: AdapterConfig::from_env is called
    /// Then: Configuration uses custom values over defaults
    #[test]
    fn given_custom_env_when_from_env_then_uses_custom_values() {
        let _g = env_guard();
        // GIVEN: Custom environment variables set
        let _key = set_env_var("SKRILLS_CODEX_API_KEY", Some("custom-key-456"));
        let _url = set_env_var("SKRILLS_CODEX_BASE_URL", Some("https://custom.api.com/v1"));
        let _model = set_env_var("SKRILLS_CODEX_MODEL", Some("gpt-4-custom"));
        let _timeout = set_env_var("SKRILLS_CODEX_TIMEOUT_MS", Some("60000"));

        // WHEN: Loading configuration
        let result = AdapterConfig::from_env("CODEX", "gpt-3.5", "https://api.openai.com", 30000);

        // THEN: Custom values override defaults
        let config = result.expect("loading config with custom env should succeed");
        assert_eq!(config.api_key, "custom-key-456");
        assert_eq!(config.base_url.as_str(), "https://custom.api.com/v1");
        assert_eq!(config.model, "gpt-4-custom");
        assert_eq!(config.timeout, Duration::from_millis(60000));
    }

    /// BDD Test: Configuration loading fails without API key
    ///
    /// Given: API key environment variable is not set
    /// When: AdapterConfig::from_env is called
    /// Then: Returns error with helpful message
    #[test]
    fn given_missing_api_key_when_from_env_then_returns_error() {
        let _g = env_guard();
        // GIVEN: API key is not set
        let _key = set_env_var("SKRILLS_CLAUDE_API_KEY", None);

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: Error is returned with helpful message
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("SKRILLS_CLAUDE_API_KEY"));
        assert!(err.contains("must be set"));
    }

    /// BDD Test: Configuration loading fails with invalid URL
    ///
    /// Given: Base URL environment variable is set to an invalid URL
    /// When: AdapterConfig::from_env is called
    /// Then: Returns error with URL validation message
    #[test]
    fn given_invalid_base_url_when_from_env_then_returns_url_error() {
        let _g = env_guard();
        // GIVEN: Invalid URL in environment
        let _key = set_env_var("SKRILLS_CLAUDE_API_KEY", Some("test-key"));
        let _url = set_env_var("SKRILLS_CLAUDE_BASE_URL", Some("not-a-valid-url"));

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: URL parsing error is returned
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("SKRILLS_CLAUDE_BASE_URL"));
        assert!(err.contains("invalid"));
    }

    /// BDD Test: Invalid timeout value falls back to default
    ///
    /// Given: Timeout environment variable contains non-numeric value
    /// When: AdapterConfig::from_env is called
    /// Then: Uses default timeout instead of failing
    #[test]
    fn given_invalid_timeout_when_from_env_then_uses_default_timeout() {
        let _g = env_guard();
        // GIVEN: Invalid timeout value
        let _key = set_env_var("SKRILLS_CLAUDE_API_KEY", Some("test-key"));
        let _timeout = set_env_var("SKRILLS_CLAUDE_TIMEOUT_MS", Some("not-a-number"));

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: Falls back to default timeout
        let config = result.expect("invalid timeout should fall back to default");
        assert_eq!(config.timeout, Duration::from_millis(30000));
    }

    /// BDD Test: Zero timeout value is accepted
    ///
    /// Given: Timeout environment variable is set to zero
    /// When: AdapterConfig::from_env is called
    /// Then: Zero duration timeout is returned
    #[test]
    fn given_zero_timeout_when_from_env_then_zero_duration() {
        let _g = env_guard();
        // GIVEN: Zero timeout
        let _key = set_env_var("SKRILLS_CLAUDE_API_KEY", Some("test-key"));
        let _timeout = set_env_var("SKRILLS_CLAUDE_TIMEOUT_MS", Some("0"));

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: Zero timeout is accepted
        let config = result.expect("zero timeout should be accepted");
        assert_eq!(config.timeout, Duration::from_millis(0));
    }

    /// BDD Test: Different prefixes produce different variable names
    ///
    /// Given: Multiple adapter types with different prefixes
    /// When: AdapterConfig::from_env is called with different prefixes
    /// Then: Each prefix reads from its own environment variables
    #[test]
    fn given_multiple_prefixes_when_from_env_then_isolated_configs() {
        let _g = env_guard();
        // GIVEN: Multiple adapter configurations
        let _claude_key = set_env_var("SKRILLS_CLAUDE_API_KEY", Some("claude-key"));
        let _claude_model = set_env_var("SKRILLS_CLAUDE_MODEL", Some("claude-3.5"));
        let _codex_key = set_env_var("SKRILLS_CODEX_API_KEY", Some("codex-key"));
        let _codex_model = set_env_var("SKRILLS_CODEX_MODEL", Some("gpt-4"));

        // WHEN: Loading different adapters
        let claude =
            AdapterConfig::from_env("CLAUDE", "claude-default", "https://api.claude.com", 30000);
        let codex = AdapterConfig::from_env("CODEX", "gpt-default", "https://api.codex.com", 30000);

        // THEN: Each adapter gets its own configuration
        assert!(claude.is_ok());
        assert!(codex.is_ok());
        assert_eq!(claude.unwrap().api_key, "claude-key");
        assert_eq!(codex.unwrap().api_key, "codex-key");
    }
}
