//! Plugin participation in the cold-window tick (TASK-022).
//!
//! Implements FR11 from `docs/cold-window-spec.md`: third-party
//! skrills plugins may opt into the cold-window by shipping a
//! `health.toml` file alongside their `.claude-plugin/plugin.json`.
//! On each tick the [`PluginHealthCollector`] walks the plugins
//! directory, parses each `health.toml`, and produces a
//! [`CollectorOutput`] split into:
//!
//! - `healths` — successfully parsed [`PluginHealth`] reports that
//!   participate in the snapshot (FR11, SC15).
//! - `malformed` — plugins whose `health.toml` failed to parse.
//!   The engine surfaces these as `Caution`-tier [`Alert`]s (EC5)
//!   and excludes them from the snapshot until fixed.
//!
//! The collector is **deliberately stateless and side-effect free**:
//! it walks the filesystem fresh each tick (the "cold rewalk"
//! contract). Caching belongs to a follow-up if SC1 budget pressure
//! demands it; for v0.8.0 the simplicity is the feature.
//!
//! `health.toml` schema (per FR11):
//!
//! ```toml
//! plugin_name = "my-plugin"           # optional: defaults to dir name
//! overall = "ok"                      # ok | warn | error | unknown
//!
//! [[checks]]
//! name = "smoke"
//! status = "ok"
//! message = "all systems nominal"     # optional
//!
//! [[checks]]
//! name = "deps"
//! status = "warn"
//! ```

use std::path::{Path, PathBuf};

use skrills_snapshot::{Alert, AlertBand, HealthCheck, HealthStatus, PluginHealth, Severity};

/// Internal diagnostic for a plugin whose `health.toml` failed to parse.
///
/// Not part of the snapshot wire format; the engine translates these
/// into `Caution`-tier [`Alert`]s before broadcast (per EC5).
#[derive(Clone, Debug, PartialEq)]
pub struct MalformedPlugin {
    /// Directory name of the plugin (matches discovery).
    pub plugin_name: String,
    /// Parser error message.
    pub error_message: String,
}

/// Result of a single collector pass over the plugins directory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectorOutput {
    /// Successfully parsed plugin health reports.
    pub healths: Vec<PluginHealth>,
    /// Plugins whose `health.toml` failed to parse.
    pub malformed: Vec<MalformedPlugin>,
}

impl CollectorOutput {
    /// Translate malformed entries into `Caution`-tier alerts (EC5).
    ///
    /// Alerts are deterministic (no hysteresis, no min-dwell): user
    /// configuration errors need immediate visibility. The fingerprint
    /// is stable across ticks so the dispatcher dedupes.
    pub fn malformed_alerts(&self, fired_at_ms: u64) -> Vec<Alert> {
        self.malformed
            .iter()
            .map(|m| Alert {
                fingerprint: format!("plugin-health-malformed::{}", m.plugin_name),
                severity: Severity::Caution,
                title: format!("Plugin '{}' health.toml malformed", m.plugin_name),
                message: m.error_message.clone(),
                band: None::<AlertBand>,
                fired_at_ms,
                dwell_ticks: 1,
            })
            .collect()
    }
}

/// Collects per-plugin health reports from `<plugins_dir>/*/health.toml`.
///
/// Construct with [`PluginHealthCollector::new`] and call
/// [`PluginHealthCollector::collect`] each tick. The collector itself
/// holds only the configured root directory; all state lives on the
/// returned [`CollectorOutput`].
#[derive(Clone, Debug)]
pub struct PluginHealthCollector {
    plugins_dir: PathBuf,
}

impl PluginHealthCollector {
    /// Construct a collector rooted at `plugins_dir`.
    pub fn new(plugins_dir: impl Into<PathBuf>) -> Self {
        Self {
            plugins_dir: plugins_dir.into(),
        }
    }

    /// Walk the plugins directory once and return parsed reports +
    /// malformed-file diagnostics.
    ///
    /// Plugins without a `health.toml` are silently excluded — a
    /// missing file is the opt-out signal, not an error (FR11).
    /// A non-existent or unreadable plugins directory yields an
    /// empty result; the cold-window must never crash on user
    /// filesystem state.
    pub fn collect(&self) -> CollectorOutput {
        let mut output = CollectorOutput::default();
        let entries = match std::fs::read_dir(&self.plugins_dir) {
            Ok(e) => e,
            Err(_) => return output,
        };

        let mut plugin_dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();
        plugin_dirs.sort();

        for plugin_dir in plugin_dirs {
            let plugin_name = plugin_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("<unnamed>")
                .to_string();

            let health_path = plugin_dir.join("health.toml");
            if !health_path.exists() {
                continue;
            }

            match read_and_parse(&health_path, &plugin_name) {
                Ok(health) => output.healths.push(health),
                Err(err) => output.malformed.push(MalformedPlugin {
                    plugin_name,
                    error_message: err,
                }),
            }
        }

        output
    }
}

/// Parse a single `health.toml` into a [`PluginHealth`].
///
/// `default_name` is used when the file omits `plugin_name`.
fn read_and_parse(path: &Path, default_name: &str) -> Result<PluginHealth, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let parsed: HealthDoc = toml::from_str(&raw).map_err(|e| format!("parse: {e}"))?;
    Ok(PluginHealth {
        plugin_name: parsed
            .plugin_name
            .unwrap_or_else(|| default_name.to_string()),
        overall: parsed.overall.unwrap_or_default(),
        checks: parsed
            .checks
            .into_iter()
            .map(|c| HealthCheck {
                name: c.name,
                status: c.status,
                message: c.message,
            })
            .collect(),
    })
}

/// On-disk schema for a plugin's `health.toml`.
#[derive(serde::Deserialize)]
struct HealthDoc {
    plugin_name: Option<String>,
    overall: Option<HealthStatus>,
    #[serde(default)]
    checks: Vec<HealthCheckDoc>,
}

#[derive(serde::Deserialize)]
struct HealthCheckDoc {
    name: String,
    status: HealthStatus,
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn missing_plugins_dir_returns_empty_output() {
        let collector = PluginHealthCollector::new("/nonexistent/path/skrills-test");
        let out = collector.collect();
        assert!(out.healths.is_empty());
        assert!(out.malformed.is_empty());
    }

    #[test]
    fn plugin_without_health_toml_is_silently_excluded() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("silent-plugin")).unwrap();
        let out = PluginHealthCollector::new(dir.path()).collect();
        assert!(out.healths.is_empty(), "no health.toml = opt-out");
        assert!(out.malformed.is_empty(), "missing != malformed");
    }

    #[test]
    fn valid_health_toml_produces_plugin_health() {
        let dir = tempdir().unwrap();
        let plugin = dir.path().join("good-plugin");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("health.toml"),
            r#"
plugin_name = "good-plugin"
overall = "ok"

[[checks]]
name = "smoke"
status = "ok"
message = "all systems nominal"

[[checks]]
name = "deps"
status = "warn"
"#,
        )
        .unwrap();

        let out = PluginHealthCollector::new(dir.path()).collect();
        assert!(out.malformed.is_empty());
        assert_eq!(out.healths.len(), 1);
        let h = &out.healths[0];
        assert_eq!(h.plugin_name, "good-plugin");
        assert_eq!(h.overall, HealthStatus::Ok);
        assert_eq!(h.checks.len(), 2);
        assert_eq!(h.checks[0].name, "smoke");
        assert_eq!(h.checks[0].status, HealthStatus::Ok);
        assert_eq!(h.checks[0].message.as_deref(), Some("all systems nominal"));
        assert_eq!(h.checks[1].status, HealthStatus::Warn);
        assert!(h.checks[1].message.is_none());
    }

    #[test]
    fn missing_plugin_name_falls_back_to_directory_name() {
        let dir = tempdir().unwrap();
        let plugin = dir.path().join("dirname-plugin");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("health.toml"),
            r#"overall = "ok"
"#,
        )
        .unwrap();

        let out = PluginHealthCollector::new(dir.path()).collect();
        assert_eq!(out.healths.len(), 1);
        assert_eq!(out.healths[0].plugin_name, "dirname-plugin");
    }

    #[test]
    fn malformed_health_toml_yields_caution_diagnostic() {
        let dir = tempdir().unwrap();
        let plugin = dir.path().join("bad-plugin");
        fs::create_dir_all(&plugin).unwrap();
        // intentional: unterminated string and missing equals
        fs::write(plugin.join("health.toml"), "this is = NOT [valid TOML").unwrap();

        let out = PluginHealthCollector::new(dir.path()).collect();
        assert!(out.healths.is_empty(), "malformed plugin excluded");
        assert_eq!(out.malformed.len(), 1);
        assert_eq!(out.malformed[0].plugin_name, "bad-plugin");
        assert!(
            out.malformed[0].error_message.contains("parse"),
            "error message should reference parse: {}",
            out.malformed[0].error_message
        );
    }

    #[test]
    fn mixed_valid_and_malformed_plugins() {
        let dir = tempdir().unwrap();

        let good = dir.path().join("good");
        fs::create_dir_all(&good).unwrap();
        fs::write(
            good.join("health.toml"),
            r#"plugin_name = "good"
overall = "ok"
"#,
        )
        .unwrap();

        let bad = dir.path().join("bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("health.toml"), "*&^%").unwrap();

        let out = PluginHealthCollector::new(dir.path()).collect();
        assert_eq!(out.healths.len(), 1);
        assert_eq!(out.malformed.len(), 1);
        assert_eq!(out.healths[0].plugin_name, "good");
        assert_eq!(out.malformed[0].plugin_name, "bad");
    }

    #[test]
    fn malformed_alerts_yields_caution_alert_per_plugin() {
        let out = CollectorOutput {
            healths: vec![],
            malformed: vec![
                MalformedPlugin {
                    plugin_name: "a".into(),
                    error_message: "expected `=`".into(),
                },
                MalformedPlugin {
                    plugin_name: "b".into(),
                    error_message: "unexpected EOF".into(),
                },
            ],
        };
        let alerts = out.malformed_alerts(1_700_000_000_000);
        assert_eq!(alerts.len(), 2);
        for alert in &alerts {
            assert_eq!(alert.severity, Severity::Caution);
            assert_eq!(alert.dwell_ticks, 1, "deterministic, no min-dwell");
            assert_eq!(alert.fired_at_ms, 1_700_000_000_000);
        }
        assert_eq!(alerts[0].fingerprint, "plugin-health-malformed::a");
        assert!(alerts[0].title.contains("'a'"));
        assert!(alerts[0].message.contains("expected `=`"));
        assert_eq!(alerts[1].fingerprint, "plugin-health-malformed::b");
    }

    #[test]
    fn fingerprint_is_stable_across_collects() {
        // Re-running collect on the same malformed plugin must produce
        // the same fingerprint so downstream dispatchers dedupe.
        let dir = tempdir().unwrap();
        let plugin = dir.path().join("flapping");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("health.toml"), "garbage").unwrap();

        let collector = PluginHealthCollector::new(dir.path());
        let alerts1 = collector.collect().malformed_alerts(1_000);
        let alerts2 = collector.collect().malformed_alerts(2_000);
        assert_eq!(alerts1[0].fingerprint, alerts2[0].fingerprint);
        // fired_at_ms differs (each collect timestamps fresh) but
        // fingerprint is the dedup key.
        assert_ne!(alerts1[0].fired_at_ms, alerts2[0].fired_at_ms);
    }

    #[test]
    fn unknown_status_string_is_a_parse_error() {
        let dir = tempdir().unwrap();
        let plugin = dir.path().join("typo");
        fs::create_dir_all(&plugin).unwrap();
        // Status enum accepts ok|warn|error|unknown; "healthy" must fail.
        fs::write(
            plugin.join("health.toml"),
            r#"overall = "healthy"
"#,
        )
        .unwrap();

        let out = PluginHealthCollector::new(dir.path()).collect();
        assert!(out.healths.is_empty());
        assert_eq!(out.malformed.len(), 1);
        assert_eq!(out.malformed[0].plugin_name, "typo");
    }

    #[test]
    fn collected_plugins_are_returned_in_sorted_order() {
        // Determinism: tick-to-tick ordering must be stable so the
        // diff (FieldwiseDiff) doesn't see spurious reorderings.
        let dir = tempdir().unwrap();
        for name in ["zeta", "alpha", "mu"] {
            let p = dir.path().join(name);
            fs::create_dir_all(&p).unwrap();
            fs::write(
                p.join("health.toml"),
                r#"overall = "ok"
"#,
            )
            .unwrap();
        }
        let out = PluginHealthCollector::new(dir.path()).collect();
        let names: Vec<&str> = out.healths.iter().map(|h| h.plugin_name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mu", "zeta"]);
    }
}
