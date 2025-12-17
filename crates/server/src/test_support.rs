use std::sync::{LazyLock, Mutex, MutexGuard};

/// Serialize tests that mutate process-global state (env vars, cwd, etc).
pub(crate) fn env_guard() -> MutexGuard<'static, ()> {
    static TEST_SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}
