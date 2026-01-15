use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

/// Serialize tests that mutate process-global state (env vars, cwd, etc).
pub(crate) fn env_guard() -> MutexGuard<'static, ()> {
    static TEST_SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Standard test fixture with pre-created directory structure.
///
/// Holds the tempdir and provides access to common paths.
/// The tempdir is automatically cleaned up when this struct is dropped.
#[allow(dead_code)] // Some fields may not be used in all tests
pub(crate) struct TestFixture {
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
/// let _guard = set_env_var("HOME", Some("/tmp/test"));
/// // HOME is set to "/tmp/test"
/// // When _guard drops, HOME is restored to its original value
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
