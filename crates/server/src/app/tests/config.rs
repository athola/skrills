//! Config tests - Skill root selection and project directory resolution

use super::super::*;
use tempfile::tempdir;

#[test]
fn default_skill_root_prefers_claude_when_both_installed() {
    let home = PathBuf::from("/home/test");
    let path = select_default_skill_root(&home, true, true);

    assert_eq!(path, home.join(".claude/skills"));
}

#[test]
fn default_skill_root_uses_codex_when_only_codex_installed() {
    let home = PathBuf::from("/home/test");
    let path = select_default_skill_root(&home, false, true);

    assert_eq!(path, home.join(".codex/skills"));
}

#[test]
fn resolve_project_dir_prefers_explicit_path() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().expect("create temp directory");
    let path = temp.path().join("project");
    let resolved = resolve_project_dir(path.to_str(), "test");

    assert_eq!(resolved, Some(path));
}

#[test]
fn resolve_project_dir_uses_current_dir() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().expect("create temp directory");
    let original = std::env::current_dir().expect("get current directory");
    std::env::set_current_dir(temp.path()).expect("change to temp directory");

    let resolved = resolve_project_dir(None, "test");

    std::env::set_current_dir(original).expect("restore original directory");
    assert_eq!(resolved, Some(temp.path().to_path_buf()));
}

#[cfg(unix)]
#[test]
fn resolve_project_dir_returns_none_when_cwd_missing() {
    let _guard = crate::test_support::env_guard();
    let original = std::env::current_dir().expect("get current directory");
    let temp = tempdir().expect("create temp directory");
    let gone = temp.path().join("gone");
    std::fs::create_dir_all(&gone).expect("create gone directory");
    std::env::set_current_dir(&gone).expect("change to gone directory");
    std::fs::remove_dir_all(&gone).expect("remove gone directory");

    let resolved = resolve_project_dir(None, "test");

    std::env::set_current_dir(original).expect("restore original directory");
    assert!(resolved.is_none());
}
