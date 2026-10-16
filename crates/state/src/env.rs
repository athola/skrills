use crate::Result;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_CACHE_TTL_MS: u64 = 43_200_000; // 12 hours

/// Returns extra skill directories specified via `SKRILLS_SKILL_DIR` environment variable.
///
/// The value is split with the platform's path-list separator (`:`, or `;`
/// on Windows). Empty entries are dropped: an empty value or a stray
/// separator would otherwise add the working directory as a skill root.
pub fn extra_dirs_from_env() -> Vec<PathBuf> {
    std::env::var_os("SKRILLS_SKILL_DIR")
        .map(|s| {
            std::env::split_paths(&s)
                .filter(|p| !p.as_os_str().is_empty())
                .collect::<Vec<PathBuf>>()
        })
        .unwrap_or_default()
}

/// Returns the user's home directory.
///
/// An empty or relative `HOME` counts as unset: used as is, it put every
/// home path (skills, caches, mirror root) under the working directory.
pub fn home_dir() -> Result<PathBuf> {
    #[cfg(unix)]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if home.is_absolute() {
            return Ok(home);
        }
    }
    dirs::home_dir().filter(|h| h.is_absolute()).ok_or_else(|| {
        anyhow::anyhow!("home directory not found (HOME is unset, empty or relative)")
    })
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

/// Checks if `SKRILLS_AUTO_PERSIST` environment variable is set to true.
///
/// When enabled, analytics are automatically saved to cache after operations
/// that build or rebuild analytics data (e.g., `recommend-skills-smart`).
pub fn env_auto_persist() -> bool {
    std::env::var("SKRILLS_AUTO_PERSIST")
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
    let text = fs::read_to_string(&path)?;
    match parse_manifest(&text) {
        Ok(settings) => Ok(settings),
        Err(e) => {
            // The defaults still apply, as before, but no longer silently:
            // `{"cache_ttl_ms":"60000"}` used to vanish with no word.
            eprintln!(
                "Warning: ignoring {}: {e}; using default manifest settings",
                path.display()
            );
            Ok(ManifestSettings::default())
        }
    }
}

/// Parses a manifest: a settings object, or a bare priority list.
fn parse_manifest(text: &str) -> std::result::Result<ManifestSettings, String> {
    let settings_err = match serde_json::from_str::<ManifestSettings>(text) {
        Ok(settings) => return Ok(settings),
        Err(e) => e,
    };
    if let Ok(priority) = serde_json::from_str::<Vec<String>>(text) {
        return Ok(ManifestSettings {
            priority: Some(priority),
            ..Default::default()
        });
    }
    Err(settings_err.to_string())
}

/// Parses a `SKRILLS_CACHE_TTL_MS` value: whole milliseconds. `0` is
/// accepted and means every lookup rediscovers skills (no caching).
fn parse_ttl_ms(raw: &str) -> std::result::Result<u64, String> {
    raw.trim()
        .parse::<u64>()
        .map_err(|e| format!("SKRILLS_CACHE_TTL_MS={raw:?} is not whole milliseconds: {e}"))
}

/// Computes the discovery cache TTL using env var or manifest settings.
///
/// An unparsable `SKRILLS_CACHE_TTL_MS` is reported on stderr and ignored,
/// so the manifest or default applies. A TTL of 0 disables caching.
pub fn cache_ttl(settings: &dyn Fn() -> Result<ManifestSettings>) -> Duration {
    let env_ttl = std::env::var("SKRILLS_CACHE_TTL_MS")
        .ok()
        .and_then(|v| match parse_ttl_ms(&v) {
            Ok(ms) => Some(ms),
            Err(e) => {
                eprintln!("Warning: {e}; ignoring it");
                None
            }
        });
    let manifest_ttl = settings().ok().and_then(|s| s.cache_ttl_ms);
    Duration::from_millis(env_ttl.or(manifest_ttl).unwrap_or(DEFAULT_CACHE_TTL_MS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_test_utils::{env_guard, set_env_var};
    use tempfile::tempdir;

    #[test]
    fn manifest_with_a_wrongly_typed_field_is_an_error_not_defaults() {
        let err = parse_manifest(r#"{"cache_ttl_ms":"60000"}"#).unwrap_err();
        assert!(
            err.contains("cache_ttl_ms") || err.contains("invalid type"),
            "{err}"
        );
        assert_eq!(
            parse_manifest(r#"["codex","claude"]"#).unwrap().priority,
            Some(vec!["codex".to_string(), "claude".to_string()])
        );
        assert_eq!(
            parse_manifest(r#"{"cache_ttl_ms":60000}"#)
                .unwrap()
                .cache_ttl_ms,
            Some(60000)
        );
    }

    #[test]
    fn ttl_env_value_must_be_whole_milliseconds() {
        assert!(parse_ttl_ms("5m")
            .unwrap_err()
            .contains("SKRILLS_CACHE_TTL_MS"));
        assert_eq!(parse_ttl_ms("0"), Ok(0));
        assert_eq!(parse_ttl_ms("1500"), Ok(1500));
    }

    #[cfg(unix)]
    #[test]
    fn empty_or_relative_home_is_treated_as_unset() {
        let _guard = env_guard();
        let _empty = set_env_var("HOME", Some(""));
        assert!(home_dir().map_or(true, |h| h.is_absolute()));
        let _relative = set_env_var("HOME", Some("relative/home"));
        assert!(home_dir().map_or(true, |h| h.is_absolute()));
        let _absolute = set_env_var("HOME", Some("/abs/home"));
        assert_eq!(home_dir().unwrap(), PathBuf::from("/abs/home"));
    }

    #[test]
    fn extra_dirs_from_env_drops_empty_entries() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let first = temp.path().join("skills_a");
        let second = temp.path().join("skills_b");

        let _empty = set_env_var("SKRILLS_SKILL_DIR", Some(""));
        assert!(extra_dirs_from_env().is_empty(), "empty value adds cwd");

        let _colon = set_env_var("SKRILLS_SKILL_DIR", Some(":"));
        assert!(extra_dirs_from_env().is_empty(), "lone separator adds cwd");

        let _padded = set_env_var(
            "SKRILLS_SKILL_DIR",
            Some(&format!(":{}::{}:", first.display(), second.display())),
        );
        assert_eq!(extra_dirs_from_env(), vec![first, second]);
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
        let _prev_persist = set_env_var("SKRILLS_AUTO_PERSIST", Some("1"));

        assert!(env_include_claude());
        assert!(env_include_marketplace());
        assert!(env_diag());
        assert!(env_auto_persist());
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
    fn cache_ttl_ignores_an_unparsable_env_value_and_uses_the_manifest() {
        let _guard = env_guard();
        let _prev = set_env_var("SKRILLS_CACHE_TTL_MS", Some("5m"));

        let ttl = cache_ttl(&|| {
            Ok(ManifestSettings {
                cache_ttl_ms: Some(2048),
                ..Default::default()
            })
        });

        assert_eq!(ttl, Duration::from_millis(2048));
    }

    #[test]
    fn malformed_manifest_file_loads_default_settings_instead_of_failing() {
        let _guard = env_guard();
        let temp = tempdir().unwrap();
        let manifest_path = temp.path().join("manifest.json");
        fs::write(&manifest_path, r#"{"cache_ttl_ms":"60000"}"#).unwrap();
        let _prev_manifest = set_env_var("SKRILLS_MANIFEST", Some(manifest_path.to_str().unwrap()));

        let settings = load_manifest_settings().expect("malformed manifest is not an error");

        assert_eq!(settings.cache_ttl_ms, None);
        assert_eq!(settings.priority, None);
        assert_eq!(settings.expose_agents, None);
    }

    #[test]
    #[should_panic(expected = "forced panic during settings load")]
    fn cache_ttl_propagates_settings_panic() {
        let _guard = env_guard();
        let _prev = set_env_var("SKRILLS_CACHE_TTL_MS", None);

        let _ = cache_ttl(&|| panic!("forced panic during settings load"));
    }

    #[test]
    fn env_auto_persist_accepts_true_string_variants() {
        let _guard = env_guard();

        // Test "true" lowercase
        let _persist = set_env_var("SKRILLS_AUTO_PERSIST", Some("true"));
        assert!(env_auto_persist(), "'true' should enable auto-persist");

        // Test "TRUE" uppercase
        let _persist = set_env_var("SKRILLS_AUTO_PERSIST", Some("TRUE"));
        assert!(env_auto_persist(), "'TRUE' should enable auto-persist");

        // Test "True" mixed case
        let _persist = set_env_var("SKRILLS_AUTO_PERSIST", Some("True"));
        assert!(env_auto_persist(), "'True' should enable auto-persist");
    }

    #[test]
    fn env_auto_persist_defaults_to_false() {
        let _guard = env_guard();

        // Unset env var
        let _persist = set_env_var("SKRILLS_AUTO_PERSIST", None);
        assert!(!env_auto_persist(), "unset should default to false");

        // Empty string
        let _persist = set_env_var("SKRILLS_AUTO_PERSIST", Some(""));
        assert!(!env_auto_persist(), "empty string should be false");

        // "0" value
        let _persist = set_env_var("SKRILLS_AUTO_PERSIST", Some("0"));
        assert!(!env_auto_persist(), "'0' should be false");

        // "false" value
        let _persist = set_env_var("SKRILLS_AUTO_PERSIST", Some("false"));
        assert!(!env_auto_persist(), "'false' should be false");

        // Random string
        let _persist = set_env_var("SKRILLS_AUTO_PERSIST", Some("yes"));
        assert!(
            !env_auto_persist(),
            "'yes' should be false (only '1' or 'true')"
        );
    }
}
