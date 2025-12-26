use std::ffi::OsStr;

pub(crate) const DEFAULT_CLI_BINARY: &str = "claude";

pub(crate) fn normalize_cli_binary(value: Option<String>) -> Option<String> {
    match value.as_deref() {
        Some(raw) if raw.trim().is_empty() => None,
        Some(raw) if raw.eq_ignore_ascii_case("auto") => None,
        _ => value,
    }
}

pub(crate) fn client_hint_from_env() -> Option<&'static str> {
    if let Ok(client) = std::env::var("SKRILLS_CLIENT") {
        if client.eq_ignore_ascii_case("codex") {
            return Some("codex");
        }
        if client.eq_ignore_ascii_case("claude") {
            return Some("claude");
        }
    }

    if std::env::var("CLAUDE_CODE_SESSION").is_ok()
        || std::env::var("CLAUDE_CLI").is_ok()
        || std::env::var("__CLAUDE_MCP_SERVER").is_ok()
        || std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok()
    {
        return Some("claude");
    }

    if std::env::var("CODEX_CLI").is_ok()
        || std::env::var("CODEX_SESSION_ID").is_ok()
        || std::env::var("CODEX_HOME").is_ok()
    {
        return Some("codex");
    }

    None
}

pub(crate) fn cli_binary_from_client_env() -> Option<String> {
    client_hint_from_env().map(|client| client.to_string())
}

pub(crate) fn client_hint_from_exe_path() -> Option<&'static str> {
    let exe = std::env::current_exe().ok()?;
    for component in exe.components() {
        let part = component.as_os_str();
        if part == OsStr::new(".codex") {
            return Some("codex");
        }
        if part == OsStr::new(".claude") {
            return Some("claude");
        }
    }
    None
}

pub(crate) fn cli_binary_from_exe_path() -> Option<String> {
    client_hint_from_exe_path().map(|client| client.to_string())
}

pub(crate) fn default_cli_binary() -> String {
    cli_binary_from_client_env()
        .or_else(cli_binary_from_exe_path)
        .unwrap_or_else(|| DEFAULT_CLI_BINARY.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

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

    fn set_env(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = env::var(key).ok();
        if let Some(val) = value {
            env::set_var(key, val);
        } else {
            env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    fn clear_client_env() -> Vec<EnvVarGuard> {
        vec![
            set_env("SKRILLS_CLIENT", None),
            set_env("CLAUDE_CODE_SESSION", None),
            set_env("CLAUDE_CLI", None),
            set_env("__CLAUDE_MCP_SERVER", None),
            set_env("CLAUDE_CODE_ENTRYPOINT", None),
            set_env("CODEX_CLI", None),
            set_env("CODEX_SESSION_ID", None),
            set_env("CODEX_HOME", None),
        ]
    }

    #[test]
    fn normalize_cli_binary_handles_auto_and_empty() {
        /*
        GIVEN cli binary overrides
        WHEN they are empty or "auto"
        THEN they should be normalized to None
        */
        assert_eq!(normalize_cli_binary(None), None);
        assert_eq!(normalize_cli_binary(Some("".to_string())), None);
        assert_eq!(normalize_cli_binary(Some("auto".to_string())), None);
        assert_eq!(normalize_cli_binary(Some("AUTO".to_string())), None);
        assert_eq!(
            normalize_cli_binary(Some("codex".to_string())),
            Some("codex".to_string())
        );
    }

    #[test]
    fn client_hint_from_env_prefers_skrills_client() {
        /*
        GIVEN SKRILLS_CLIENT is set
        WHEN inspecting the environment
        THEN it should override other client hints
        */
        let _guard = env_guard();
        let _clear = clear_client_env();
        let _skrills = set_env("SKRILLS_CLIENT", Some("codex"));
        let _claude = set_env("CLAUDE_CODE_SESSION", Some("1"));

        assert_eq!(client_hint_from_env(), Some("codex"));
    }

    #[test]
    fn client_hint_from_env_detects_claude_markers() {
        /*
        GIVEN Claude environment markers
        WHEN SKRILLS_CLIENT is not set
        THEN Claude should be detected
        */
        let _guard = env_guard();
        let _clear = clear_client_env();
        let _claude = set_env("CLAUDE_CODE_SESSION", Some("1"));

        assert_eq!(client_hint_from_env(), Some("claude"));
    }

    #[test]
    fn default_cli_binary_uses_env_hint() {
        /*
        GIVEN a SKRILLS_CLIENT override
        WHEN resolving the default cli binary
        THEN it should use the env hint
        */
        let _guard = env_guard();
        let _clear = clear_client_env();
        let _skrills = set_env("SKRILLS_CLIENT", Some("codex"));

        assert_eq!(default_cli_binary(), "codex".to_string());
    }
}
