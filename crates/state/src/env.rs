use anyhow::Result;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_CACHE_TTL_MS: u64 = 43_200_000; // 12 hours

/// Returns extra skill directories specified via `SKRILLS_SKILL_DIR` environment variable.
pub fn extra_dirs_from_env() -> Vec<PathBuf> {
    std::env::var("SKRILLS_SKILL_DIR")
        .map(|s| s.split(':').map(PathBuf::from).collect::<Vec<PathBuf>>())
        .unwrap_or_default()
}

/// Returns the user's home directory.
pub fn home_dir() -> Result<PathBuf> {
    #[cfg(unix)]
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home));
    }
    dirs::home_dir().ok_or_else(|| anyhow::anyhow!("home directory not found"))
}

/// Checks if `SKRILLS_INCLUDE_CLAUDE` environment variable is set to true.
pub fn env_include_claude() -> bool {
    std::env::var("SKRILLS_INCLUDE_CLAUDE")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Checks if `SKRILLS_INCLUDE_MARKETPLACE` environment variable is set to true.
pub fn env_include_marketplace() -> bool {
    std::env::var("SKRILLS_INCLUDE_MARKETPLACE")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Checks if `SKRILLS_MANIFEST_FIRST` environment variable is set to true.
pub fn env_manifest_first() -> bool {
    std::env::var("SKRILLS_MANIFEST_FIRST")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(true)
}

/// Checks if `SKRILLS_RENDER_MODE_LOG` environment variable is set to true.
pub fn env_render_mode_log() -> bool {
    std::env::var("SKRILLS_RENDER_MODE_LOG")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Returns the `SKRILLS_MANIFEST_MINIMAL` setting (default: true).
///
/// When true, autoload returns skill descriptions only instead of full content.
/// Set to "0" or "false" to load full skill content by default.
pub fn env_manifest_minimal() -> bool {
    std::env::var("SKRILLS_MANIFEST_MINIMAL")
        .map(|s| s != "0" && !s.eq_ignore_ascii_case("false"))
        .unwrap_or(true)
}

/// Returns the maximum bytes for autoload content from `SKRILLS_MAX_BYTES` environment variable.
pub fn env_max_bytes() -> Option<usize> {
    std::env::var("SKRILLS_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
}

/// Determines if auto-pinning is enabled, considering persisted state and environment variable.
pub fn env_auto_pin(persisted_state: bool) -> bool {
    std::env::var("SKRILLS_AUTO_PIN")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(persisted_state)
}

/// Checks if `SKRILLS_DIAGNOSE` environment variable is set to true.
pub fn env_diag() -> bool {
    std::env::var("SKRILLS_DIAGNOSE")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Returns the path to the skills manifest file.
pub fn manifest_file() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("SKRILLS_MANIFEST") {
        return Some(PathBuf::from(custom));
    }
    home_dir()
        .ok()
        .map(|h| h.join(".codex/skills-manifest.json"))
}

/// Returns the path to runtime overrides configuration.
pub fn runtime_overrides_path() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("SKRILLS_RUNTIME_OVERRIDES") {
        return Some(PathBuf::from(custom));
    }
    home_dir()
        .ok()
        .map(|h| h.join(".codex/runtime-overrides.json"))
}

/// Manifest settings parsed from `skills-manifest.json`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ManifestSettings {
    #[serde(default)]
    pub priority: Option<Vec<String>>,
    #[serde(default)]
    pub expose_agents: Option<bool>,
    #[serde(default)]
    pub cache_ttl_ms: Option<u64>,
}

/// Loads manifest settings from disk if available.
pub fn load_manifest_settings() -> Result<ManifestSettings> {
    let Some(path) = manifest_file() else {
        return Ok(ManifestSettings::default());
    };
    if !path.exists() {
        return Ok(ManifestSettings::default());
    }
    let text = fs::read_to_string(path)?;
    if let Ok(settings) = serde_json::from_str::<ManifestSettings>(&text) {
        return Ok(settings);
    }
    if let Ok(priority) = serde_json::from_str::<Vec<String>>(&text) {
        return Ok(ManifestSettings {
            priority: Some(priority),
            ..Default::default()
        });
    }
    Ok(ManifestSettings::default())
}

/// Computes the discovery cache TTL using env var or manifest settings.
pub fn cache_ttl(settings: &dyn Fn() -> Result<ManifestSettings>) -> Duration {
    let env_ttl = std::env::var("SKRILLS_CACHE_TTL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let manifest_ttl = settings().ok().and_then(|s| s.cache_ttl_ms);
    Duration::from_millis(env_ttl.or(manifest_ttl).unwrap_or(DEFAULT_CACHE_TTL_MS))
}
