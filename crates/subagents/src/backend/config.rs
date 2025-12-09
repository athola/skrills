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
