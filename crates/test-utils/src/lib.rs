//! Shared test utilities for skrills crates.
//!
//! This crate provides common test fixtures and utilities used across
//! multiple crates in the skrills workspace.

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

/// Serialize tests that mutate process-global state (env vars, cwd, etc).
///
/// Acquire this guard at the start of any test that modifies environment
/// variables to prevent race conditions between parallel tests.
pub fn env_guard() -> MutexGuard<'static, ()> {
    static TEST_SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// RAII guard for environment variables - restores original value on drop.
pub struct EnvVarGuard {
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
/// ```
/// let _guard = skrills_test_utils::set_env_var("MY_VAR", Some("value"));
/// // MY_VAR is set to "value"
/// // When _guard drops, MY_VAR is restored to its original value
/// ```
pub fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
    let previous = std::env::var(key).ok();
    if let Some(val) = value {
        std::env::set_var(key, val);
    } else {
        std::env::remove_var(key);
    }
    EnvVarGuard { key, previous }
}

/// Standard test fixture with pre-created directory structure.
///
/// Holds the tempdir and provides access to common paths.
/// The tempdir is automatically cleaned up when this struct is dropped.
pub struct TestFixture {
    pub tempdir: tempfile::TempDir,
    /// Path to ~/.claude/skills in the temp environment
    pub claude_skills: PathBuf,
    /// Path to ~/.codex/skills in the temp environment
    pub codex_skills: PathBuf,
}

impl TestFixture {
    /// Create a new test fixture with the standard directory structure.
    ///
    /// Creates:
    /// - `$HOME/.claude/skills/`
    /// - `$HOME/.codex/skills/`
    ///
    /// Does NOT set HOME env var - use `home_guard()` for that.
    pub fn new() -> std::io::Result<Self> {
        let tempdir = tempfile::tempdir()?;
        let claude_skills = tempdir.path().join(".claude/skills");
        let codex_skills = tempdir.path().join(".codex/skills");

        std::fs::create_dir_all(&claude_skills)?;
        std::fs::create_dir_all(&codex_skills)?;

        Ok(Self {
            tempdir,
            claude_skills,
            codex_skills,
        })
    }

    /// Get the path that should be set as HOME.
    pub fn home_path(&self) -> &std::path::Path {
        self.tempdir.path()
    }

    /// Create an RAII guard that sets HOME to this fixture's temp directory.
    pub fn home_guard(&self) -> EnvVarGuard {
        set_env_var("HOME", Some(self.home_path().to_str().unwrap()))
    }

    /// Create a minimal skill in the Claude skills directory.
    ///
    /// Returns the path to the skill directory.
    pub fn create_skill(&self, name: &str, content: &str) -> std::io::Result<PathBuf> {
        let skill_dir = self.claude_skills.join(name);
        std::fs::create_dir_all(&skill_dir)?;
        std::fs::write(skill_dir.join("SKILL.md"), content)?;
        Ok(skill_dir)
    }

    /// Create a skill with standard frontmatter.
    pub fn create_skill_with_frontmatter(
        &self,
        name: &str,
        description: &str,
        body: &str,
    ) -> std::io::Result<PathBuf> {
        let content = format!(
            "---\nname: {}\ndescription: {}\n---\n{}",
            name, description, body
        );
        self.create_skill(name, &content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_guard_serializes_tests() {
        // Simply verify we can acquire the guard
        let _g = env_guard();
        // Guard should drop cleanly
    }

    #[test]
    fn test_set_env_var_sets_and_restores() {
        let _g = env_guard();

        // Use a unique key to avoid conflicts
        const KEY: &str = "SKRILLS_TEST_UTILS_TEST_VAR";

        // Ensure clean state
        std::env::remove_var(KEY);

        {
            let _guard = set_env_var(KEY, Some("test_value"));
            assert_eq!(std::env::var(KEY).ok(), Some("test_value".to_string()));
        }
        // After guard drops, should be restored (removed since it didn't exist)
        assert!(std::env::var(KEY).is_err());
    }

    #[test]
    fn test_set_env_var_restores_previous_value() {
        let _g = env_guard();

        const KEY: &str = "SKRILLS_TEST_RESTORE_VAR";
        std::env::set_var(KEY, "original");

        {
            let _guard = set_env_var(KEY, Some("changed"));
            assert_eq!(std::env::var(KEY).ok(), Some("changed".to_string()));
        }
        // After guard drops, should restore original
        assert_eq!(std::env::var(KEY).ok(), Some("original".to_string()));

        // Cleanup
        std::env::remove_var(KEY);
    }

    #[test]
    fn test_set_env_var_removes_when_none() {
        let _g = env_guard();

        const KEY: &str = "SKRILLS_TEST_REMOVE_VAR";
        std::env::set_var(KEY, "exists");

        {
            let _guard = set_env_var(KEY, None);
            assert!(std::env::var(KEY).is_err());
        }
        // After guard drops, original value restored
        assert_eq!(std::env::var(KEY).ok(), Some("exists".to_string()));

        // Cleanup
        std::env::remove_var(KEY);
    }

    #[test]
    fn test_fixture_creates_directories() {
        let fixture = TestFixture::new().expect("fixture creation");
        assert!(fixture.claude_skills.exists());
        assert!(fixture.codex_skills.exists());
        assert!(fixture.claude_skills.is_dir());
        assert!(fixture.codex_skills.is_dir());
    }

    #[test]
    fn test_fixture_home_path() {
        let fixture = TestFixture::new().expect("fixture creation");
        let home = fixture.home_path();
        assert!(home.exists());
        assert!(home.join(".claude/skills").exists());
    }

    #[test]
    fn test_fixture_create_skill() {
        let fixture = TestFixture::new().expect("fixture creation");
        let skill_dir = fixture
            .create_skill("test-skill", "# Test\nContent here")
            .expect("create skill");

        assert!(skill_dir.exists());
        assert!(skill_dir.join("SKILL.md").exists());

        let content = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(content.contains("Content here"));
    }

    #[test]
    fn test_fixture_create_skill_with_frontmatter() {
        let fixture = TestFixture::new().expect("fixture creation");
        let skill_dir = fixture
            .create_skill_with_frontmatter("fm-skill", "A test skill", "Body content")
            .expect("create skill");

        let content = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(content.contains("name: fm-skill"));
        assert!(content.contains("description: A test skill"));
        assert!(content.contains("Body content"));
    }

    #[test]
    fn test_fixture_home_guard() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture creation");

        let original_home = std::env::var("HOME").ok();
        {
            let _home_guard = fixture.home_guard();
            let new_home = std::env::var("HOME").unwrap();
            assert_eq!(new_home, fixture.home_path().to_str().unwrap());
        }
        // Restored after guard drops
        assert_eq!(std::env::var("HOME").ok(), original_home);
    }
}
