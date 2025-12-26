//! Runtime configuration path helpers for `skrills-server`.
//!
//! Re-exports the runtime overrides path from the state crate.
//! Note: Runtime override features were removed in 0.3.1 as skill loading
//! is now handled by Claude/Codex directly.

pub use skrills_state::runtime_overrides_path;

#[cfg(test)]
mod tests {
    use super::runtime_overrides_path;
    use crate::test_support;
    use std::env;
    use tempfile::tempdir;

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
        if let Some(val) = value {
            env::set_var(key, val);
        } else {
            env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    #[test]
    fn runtime_overrides_path_defaults_to_home() {
        /*
        GIVEN no explicit runtime override path
        WHEN resolving the runtime overrides path
        THEN it should use the HOME-based default
        */
        let _guard = test_support::env_guard();
        let temp = tempdir().expect("tempdir");
        let _home = set_env_var(
            "HOME",
            Some(
                temp.path()
                    .to_str()
                    .expect("temp home should be valid utf-8"),
            ),
        );
        let _override = set_env_var("SKRILLS_RUNTIME_OVERRIDES", None);

        let expected = temp.path().join(".codex/runtime-overrides.json");
        let actual = runtime_overrides_path().expect("expected runtime overrides path");
        assert_eq!(actual, expected);
    }

    #[test]
    fn runtime_overrides_path_respects_env_override() {
        /*
        GIVEN SKRILLS_RUNTIME_OVERRIDES is set
        WHEN resolving the runtime overrides path
        THEN it should return the override path
        */
        let _guard = test_support::env_guard();
        let temp = tempdir().expect("tempdir");
        let override_path = temp.path().join("override.json");
        let _override = set_env_var(
            "SKRILLS_RUNTIME_OVERRIDES",
            Some(override_path.to_str().expect("override path")),
        );

        let actual = runtime_overrides_path().expect("expected runtime overrides path");
        assert_eq!(actual, override_path);
    }
}
