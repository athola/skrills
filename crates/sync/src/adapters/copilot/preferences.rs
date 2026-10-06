//! Preferences reading and writing for Copilot adapter.

use super::paths::config_path;
use crate::common::Preferences;
use crate::report::WriteReport;
use crate::Result;
use anyhow::Context;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Reads preferences from the config.json file.
pub fn read_preferences(root: &Path) -> Result<Preferences> {
    let path = config_path(root);
    if !path.exists() {
        return Ok(Preferences::default());
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read preferences: {}", path.display()))?;
    let config: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse preferences as JSON: {}", path.display()))?;

    Ok(Preferences {
        model: config
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from),
        custom: HashMap::new(),
    })
}

/// Writes preferences to the config.json file.
///
/// Only `model` is set; every other key (`trusted_folders`, `allowed_urls`,
/// `denied_urls`, ...) is preserved.
pub fn write_preferences(root: &Path, prefs: &Preferences) -> Result<WriteReport> {
    crate::adapters::json_config::write_model(&config_path(root), prefs.model.as_deref())
}
