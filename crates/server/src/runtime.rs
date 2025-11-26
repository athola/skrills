//! Runtime override helpers exposed by `skrills-server`.
//!
//! These helpers are part of the public surface area consumed by the CLI and
//! MCP runtime tools (`runtime-status`, `set-runtime-options`). They are
//! intentionally small and configuration-only. While the crate is pre-1.0,
//! the types in this module follow a best-effort stability promise: new
//! fields may be added, but existing fields and semantics will not be
//! removed or changed without a CHANGELOG note. Consumers should feature-gate
//! usage behind the crate version they build against and avoid relying on
//! private internals.
//!
//! When compiled with the optional `watch` feature, the surrounding crate also
//! exposes filesystem watching; this module itself has no feature flags and is
//! always available.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

use once_cell::sync::Lazy;
use skrills_state::{env_manifest_first, env_render_mode_log, runtime_overrides_path};
use std::sync::Mutex;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RuntimeOverrides {
    pub manifest_first: Option<bool>,
    pub render_mode_log: Option<bool>,
    pub manifest_minimal: Option<bool>,
}

impl RuntimeOverrides {
    pub fn load() -> Result<Self> {
        if let Some(path) = runtime_overrides_path() {
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<RuntimeOverrides>(&text) {
                    return Ok(val);
                }
            }
        }
        Ok(RuntimeOverrides::default())
    }

    pub fn save(&self) -> Result<()> {
        if let Some(path) = runtime_overrides_path() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let text = serde_json::to_string_pretty(self)?;
            fs::write(path, text)?;
        }
        Ok(())
    }

    pub fn manifest_first(&self) -> bool {
        self.manifest_first.unwrap_or_else(env_manifest_first)
    }

    pub fn render_mode_log(&self) -> bool {
        self.render_mode_log.unwrap_or_else(env_render_mode_log)
    }

    pub fn manifest_minimal(&self) -> bool {
        self.manifest_minimal
            .unwrap_or_else(skrills_state::env_manifest_minimal)
    }
}

static RUNTIME_CACHE: Lazy<Mutex<Option<RuntimeOverrides>>> = Lazy::new(|| Mutex::new(None));

/// Load overrides once per process; subsequent calls reuse cached value.
pub fn runtime_overrides_cached() -> RuntimeOverrides {
    if let Ok(mut guard) = RUNTIME_CACHE.lock() {
        if let Some(val) = guard.as_ref() {
            return val.clone();
        }
        if let Ok(val) = RuntimeOverrides::load() {
            *guard = Some(val.clone());
            return val;
        }
    }
    RuntimeOverrides::default()
}

pub fn reset_runtime_cache_for_tests() {
    if let Ok(mut guard) = RUNTIME_CACHE.lock() {
        *guard = None;
    }
}
