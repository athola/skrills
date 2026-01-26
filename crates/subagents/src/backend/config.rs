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
        let timeout_ms = std::env::var(&timeout_var)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(default_timeout_ms);

        Ok(Self {
            api_key,
            base_url,
            model,
            timeout: Duration::from_millis(timeout_ms),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BDD Test: Configuration loading with all default values
    ///
    /// Given: Environment variables are set with minimal required values
    /// When: AdapterConfig::from_env is called
    /// Then: Configuration uses defaults for optional fields
    #[test]
    fn given_minimal_env_when_from_env_then_uses_defaults() {
        // GIVEN: Only API key is set
        std::env::set_var("SKRILLS_CLAUDE_API_KEY", "test-key-123");
        std::env::remove_var("SKRILLS_CLAUDE_BASE_URL");
        std::env::remove_var("SKRILLS_CLAUDE_MODEL");
        std::env::remove_var("SKRILLS_CLAUDE_TIMEOUT_MS");

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: Defaults are applied correctly
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.api_key, "test-key-123");
        assert_eq!(config.base_url.as_str(), "https://api.anthropic.com/");
        assert_eq!(config.model, "claude-3.5");
        assert_eq!(config.timeout, Duration::from_millis(30000));

        // Cleanup
        std::env::remove_var("SKRILLS_CLAUDE_API_KEY");
    }

    /// BDD Test: Configuration loading with custom values
    ///
    /// Given: All environment variables are set with custom values
    /// When: AdapterConfig::from_env is called
    /// Then: Configuration uses custom values over defaults
    #[test]
    fn given_custom_env_when_from_env_then_uses_custom_values() {
        // GIVEN: Custom environment variables set
        std::env::set_var("SKRILLS_CODEX_API_KEY", "custom-key-456");
        std::env::set_var("SKRILLS_CODEX_BASE_URL", "https://custom.api.com/v1");
        std::env::set_var("SKRILLS_CODEX_MODEL", "gpt-4-custom");
        std::env::set_var("SKRILLS_CODEX_TIMEOUT_MS", "60000");

        // WHEN: Loading configuration
        let result = AdapterConfig::from_env("CODEX", "gpt-3.5", "https://api.openai.com", 30000);

        // THEN: Custom values override defaults
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.api_key, "custom-key-456");
        assert_eq!(config.base_url.as_str(), "https://custom.api.com/v1");
        assert_eq!(config.model, "gpt-4-custom");
        assert_eq!(config.timeout, Duration::from_millis(60000));

        // Cleanup
        std::env::remove_var("SKRILLS_CODEX_API_KEY");
        std::env::remove_var("SKRILLS_CODEX_BASE_URL");
        std::env::remove_var("SKRILLS_CODEX_MODEL");
        std::env::remove_var("SKRILLS_CODEX_TIMEOUT_MS");
    }

    /// BDD Test: Configuration loading fails without API key
    ///
    /// Given: API key environment variable is not set
    /// When: AdapterConfig::from_env is called
    /// Then: Returns error with helpful message
    #[test]
    fn given_missing_api_key_when_from_env_then_returns_error() {
        // GIVEN: API key is not set
        std::env::remove_var("SKRILLS_CLAUDE_API_KEY");

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
        // GIVEN: Invalid URL in environment
        std::env::set_var("SKRILLS_CLAUDE_API_KEY", "test-key");
        std::env::set_var("SKRILLS_CLAUDE_BASE_URL", "not-a-valid-url");

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: URL parsing error is returned
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("SKRILLS_CLAUDE_BASE_URL"));
        assert!(err.contains("invalid"));

        // Cleanup
        std::env::remove_var("SKRILLS_CLAUDE_API_KEY");
        std::env::remove_var("SKRILLS_CLAUDE_BASE_URL");
    }

    /// BDD Test: Invalid timeout value falls back to default
    ///
    /// Given: Timeout environment variable contains non-numeric value
    /// When: AdapterConfig::from_env is called
    /// Then: Uses default timeout instead of failing
    #[test]
    fn given_invalid_timeout_when_from_env_then_uses_default_timeout() {
        // GIVEN: Invalid timeout value
        std::env::set_var("SKRILLS_CLAUDE_API_KEY", "test-key");
        std::env::set_var("SKRILLS_CLAUDE_TIMEOUT_MS", "not-a-number");

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: Falls back to default timeout
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.timeout, Duration::from_millis(30000));

        // Cleanup
        std::env::remove_var("SKRILLS_CLAUDE_API_KEY");
        std::env::remove_var("SKRILLS_CLAUDE_TIMEOUT_MS");
    }

    /// BDD Test: Zero timeout value is accepted
    ///
    /// Given: Timeout environment variable is set to zero
    /// When: AdapterConfig::from_env is called
    /// Then: Zero duration timeout is returned
    #[test]
    fn given_zero_timeout_when_from_env_then_zero_duration() {
        // GIVEN: Zero timeout
        std::env::set_var("SKRILLS_CLAUDE_API_KEY", "test-key");
        std::env::set_var("SKRILLS_CLAUDE_TIMEOUT_MS", "0");

        // WHEN: Loading configuration
        let result =
            AdapterConfig::from_env("CLAUDE", "claude-3.5", "https://api.anthropic.com", 30000);

        // THEN: Zero timeout is accepted
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.timeout, Duration::from_millis(0));

        // Cleanup
        std::env::remove_var("SKRILLS_CLAUDE_API_KEY");
        std::env::remove_var("SKRILLS_CLAUDE_TIMEOUT_MS");
    }

    /// BDD Test: Different prefixes produce different variable names
    ///
    /// Given: Multiple adapter types with different prefixes
    /// When: AdapterConfig::from_env is called with different prefixes
    /// Then: Each prefix reads from its own environment variables
    #[test]
    fn given_multiple_prefixes_when_from_env_then_isolated_configs() {
        // GIVEN: Multiple adapter configurations
        std::env::set_var("SKRILLS_CLAUDE_API_KEY", "claude-key");
        std::env::set_var("SKRILLS_CLAUDE_MODEL", "claude-3.5");
        std::env::set_var("SKRILLS_CODEX_API_KEY", "codex-key");
        std::env::set_var("SKRILLS_CODEX_MODEL", "gpt-4");

        // WHEN: Loading different adapters
        let claude =
            AdapterConfig::from_env("CLAUDE", "claude-default", "https://api.claude.com", 30000);
        let codex = AdapterConfig::from_env("CODEX", "gpt-default", "https://api.codex.com", 30000);

        // THEN: Each adapter gets its own configuration
        assert!(claude.is_ok());
        assert!(codex.is_ok());
        assert_eq!(claude.unwrap().api_key, "claude-key");
        assert_eq!(codex.unwrap().api_key, "codex-key");

        // Cleanup
        std::env::remove_var("SKRILLS_CLAUDE_API_KEY");
        std::env::remove_var("SKRILLS_CLAUDE_MODEL");
        std::env::remove_var("SKRILLS_CODEX_API_KEY");
        std::env::remove_var("SKRILLS_CODEX_MODEL");
    }
}
