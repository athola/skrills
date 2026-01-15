//! Test utilities for the intelligence crate.

use std::sync::{LazyLock, Mutex, MutexGuard};

/// Serialize tests that mutate process-global state (env vars).
///
/// Acquire this guard at the start of any test that modifies environment
/// variables to prevent race conditions between parallel tests.
pub(crate) fn env_guard() -> MutexGuard<'static, ()> {
    static TEST_SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// RAII guard for environment variables - restores original value on drop.
pub(crate) struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(v) = &self.previous {
            std::env::set_var(self.key, v);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

/// Set an environment variable and return a guard that restores the original on drop.
///
/// # Example
/// ```ignore
/// let _guard = set_env_var("MY_VAR", Some("value"));
/// // MY_VAR is set to "value"
/// // When _guard drops, MY_VAR is restored to its original value
/// ```
pub(crate) fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
    let previous = std::env::var(key).ok();
    if let Some(val) = value {
        std::env::set_var(key, val);
    } else {
        std::env::remove_var(key);
    }
    EnvVarGuard { key, previous }
}
