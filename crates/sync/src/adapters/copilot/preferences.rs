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
pub fn write_preferences(root: &Path, prefs: &Preferences) -> Result<WriteReport> {
    let path = config_path(root);

    // CRITICAL: Read existing config to preserve security fields
    // (trusted_folders, allowed_urls, denied_urls)
    let mut config: serde_json::Value = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read preferences: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse preferences as JSON: {}", path.display()))?
    } else {
        serde_json::json!({})
    };

    let mut report = WriteReport::default();

    // Only update the model field - leave all other fields untouched
    if let Some(model) = &prefs.model {
        config["model"] = serde_json::json!(model);
        report.written += 1;
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }
    fs::write(&path, serde_json::to_string_pretty(&config)?)
        .with_context(|| format!("Failed to write preferences: {}", path.display()))?;

    Ok(report)
}
