use anyhow::{anyhow, Result};
use dirs;
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_CACHE_TTL_MS: u64 = 1_500;

#[derive(Debug, Default, Deserialize, Clone)]
pub struct ManifestSettings {
    #[serde(default)]
    pub priority: Option<Vec<String>>,
    #[serde(default)]
    pub expose_agents: Option<bool>,
    #[serde(default)]
    pub cache_ttl_ms: Option<u64>,
}

/// Returns the user's home directory.
pub fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("HOME not set"))
}

pub fn manifest_file() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("CODEX_SKILLS_MANIFEST") {
        return Ok(PathBuf::from(custom));
    }
    Ok(home_dir()?.join(".codex/skills-manifest.json"))
}

pub fn load_manifest_settings() -> Result<ManifestSettings> {
    let path = manifest_file()?;
    if !path.exists() {
        return Ok(ManifestSettings::default());
    }
    let data = std::fs::read_to_string(path)?;
    let val: serde_json::Value = serde_json::from_str(&data)?;
    if let Some(arr) = val.as_array() {
        let list: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        return Ok(ManifestSettings {
            priority: Some(list),
            expose_agents: None,
            cache_ttl_ms: None,
        });
    }
    if let Some(obj) = val.as_object() {
        let priority = obj.get("priority").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });
        let expose_agents = obj.get("expose_agents").and_then(|v| v.as_bool());
        let cache_ttl_ms = obj.get("cache_ttl_ms").and_then(|v| v.as_u64());
        return Ok(ManifestSettings {
            priority,
            expose_agents,
            cache_ttl_ms,
        });
    }
    Err(anyhow!("invalid manifest format"))
}

pub fn extra_dirs_from_env() -> Vec<PathBuf> {
    std::env::var("CODEX_SKILLS_EXTRA_DIRS")
        .ok()
        .map(|v| {
            v.split(':')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn env_include_claude() -> bool {
    std::env::var("CODEX_SKILLS_INCLUDE_CLAUDE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn env_diag() -> bool {
    std::env::var("CODEX_SKILLS_DIAG")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn env_auto_pin(default: bool) -> bool {
    if let Ok(v) = std::env::var("CODEX_SKILLS_AUTO_PIN") {
        return v == "1" || v.eq_ignore_ascii_case("true");
    }
    default
}

/// Reads an optional max-bytes override for autoload payloads from env.
pub fn env_max_bytes() -> Option<usize> {
    std::env::var("CODEX_SKILLS_AUTOLOAD_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
}

/// Returns the in-memory cache TTL for skill discovery in milliseconds.
/// Precedence: env CODEX_SKILLS_CACHE_TTL_MS > manifest cache_ttl_ms > default.
pub fn cache_ttl(load_manifest: &dyn Fn() -> Result<ManifestSettings>) -> Duration {
    if let Some(ms) = std::env::var("CODEX_SKILLS_CACHE_TTL_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        return Duration::from_millis(ms);
    }

    if let Ok(manifest) = load_manifest() {
        if let Some(ms) = manifest.cache_ttl_ms {
            return Duration::from_millis(ms);
        }
    }

    Duration::from_millis(DEFAULT_CACHE_TTL_MS)
}
