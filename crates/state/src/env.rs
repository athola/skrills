use crate::Result;
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
    /// Priority order for skill sources.
    #[serde(default)]
    pub priority: Option<Vec<String>>,
    /// Whether to expose agent definitions.
    #[serde(default)]
    pub expose_agents: Option<bool>,
    /// Cache time-to-live in milliseconds.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.previous {
                env::set_var(self.key, v);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = env::var(key).ok();
        if let Some(v) = value {
            env::set_var(key, v);
        } else {
            env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    #[test]
    fn extra_dirs_from_env_splits_colon_paths() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let first = temp.path().join("skills_a");
        let second = temp.path().join("skills_b");
        let _skill_guard = set_env_var(
            "SKRILLS_SKILL_DIR",
            Some(&format!("{}:{}", first.display(), second.display())),
        );

        let dirs = extra_dirs_from_env();

        assert_eq!(dirs, vec![first, second]);
    }

    #[test]
    fn home_dir_uses_home_env() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let _home_guard = set_env_var("HOME", Some(temp.path().to_str().unwrap()));

        let dir = home_dir().expect("home dir should resolve from HOME env");

        assert_eq!(dir, temp.path());
    }

    #[test]
    fn env_flags_respect_truthy_values() {
        let _guard = env_guard();
        let _prev_claude = set_env_var("SKRILLS_INCLUDE_CLAUDE", Some("true"));
        let _prev_market = set_env_var("SKRILLS_INCLUDE_MARKETPLACE", Some("1"));
        let _prev_diag = set_env_var("SKRILLS_DIAGNOSE", Some("TRUE"));

        assert!(env_include_claude());
        assert!(env_include_marketplace());
        assert!(env_diag());
    }

    #[test]
    fn manifest_and_runtime_overrides_prefer_env_paths() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let manifest_path = temp.path().join("manifest.json");
        let runtime_path = temp.path().join("overrides.json");
        let _prev_manifest = set_env_var("SKRILLS_MANIFEST", Some(manifest_path.to_str().unwrap()));
        let _prev_runtime = set_env_var(
            "SKRILLS_RUNTIME_OVERRIDES",
            Some(runtime_path.to_str().unwrap()),
        );

        assert_eq!(manifest_file(), Some(manifest_path));
        assert_eq!(runtime_overrides_path(), Some(runtime_path));
    }

    #[test]
    fn cache_ttl_prefers_env_over_manifest() {
        let _guard = env_guard();
        let _prev = set_env_var("SKRILLS_CACHE_TTL_MS", Some("9001"));

        let ttl = cache_ttl(&|| {
            Ok(ManifestSettings {
                cache_ttl_ms: Some(1234),
                ..Default::default()
            })
        });

        assert_eq!(ttl, Duration::from_millis(9001));
    }

    #[test]
    fn cache_ttl_falls_back_to_manifest_setting() {
        let _guard = env_guard();
        let _prev = set_env_var("SKRILLS_CACHE_TTL_MS", None);

        let ttl = cache_ttl(&|| {
            Ok(ManifestSettings {
                cache_ttl_ms: Some(2048),
                ..Default::default()
            })
        });

        assert_eq!(ttl, Duration::from_millis(2048));
    }

    #[test]
    #[should_panic(expected = "forced panic during settings load")]
    fn cache_ttl_propagates_settings_panic() {
        let _guard = env_guard();
        let _prev = set_env_var("SKRILLS_CACHE_TTL_MS", None);

        let _ = cache_ttl(&|| panic!("forced panic during settings load"));
    }
}
