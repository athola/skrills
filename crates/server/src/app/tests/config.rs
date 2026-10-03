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
    let canonical = temp.path().canonicalize().expect("canonicalize temp path");
    let original = std::env::current_dir().expect("get current directory");
    std::env::set_current_dir(temp.path()).expect("change to temp directory");

    let resolved = resolve_project_dir(None, "test");

    std::env::set_current_dir(original).expect("restore original directory");
    assert_eq!(resolved, Some(canonical));
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

/// SA-13: reads (including the server's own discovery walk) must not
/// invalidate the cache; creates, writes and removals must.
#[cfg(feature = "watch")]
#[test]
fn watcher_ignores_access_events() {
    use notify::event::{
        AccessKind, AccessMode, CreateKind, DataChange, MetadataKind, ModifyKind, RemoveKind,
    };
    use notify::EventKind;

    assert!(!invalidates_cache(&EventKind::Access(AccessKind::Open(
        AccessMode::Any
    ))));
    assert!(!invalidates_cache(&EventKind::Access(AccessKind::Close(
        AccessMode::Read
    ))));
    assert!(!invalidates_cache(&EventKind::Modify(
        ModifyKind::Metadata(MetadataKind::AccessTime)
    )));
    assert!(invalidates_cache(&EventKind::Create(CreateKind::File)));
    assert!(invalidates_cache(&EventKind::Modify(ModifyKind::Data(
        DataChange::Content
    ))));
    assert!(invalidates_cache(&EventKind::Remove(RemoveKind::File)));
}

/// SA-35: total_skills counts the skills that were analysed, so it agrees
/// with by_source when a file disappears after discovery.
#[test]
fn metrics_total_matches_by_source_when_a_file_is_unreadable() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _cache = crate::test_support::set_env_var(
        "SKRILLS_CACHE_PATH",
        Some(temp.path().join("cache.json").to_str().unwrap()),
    );
    let root = temp.path().join("skills");
    for name in ["keep", "gone"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(
            root.join(name).join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name}\n---\n"),
        )
        .unwrap();
    }
    let service = SkillService::new_with_roots_for_test(
        vec![SkillRoot {
            root: root.clone(),
            source: skrills_discovery::SkillSource::Claude,
        }],
        std::time::Duration::from_secs(60),
    )
    .unwrap();
    service.current_skills_with_dups().unwrap();
    std::fs::remove_file(root.join("gone").join("SKILL.md")).unwrap();

    let metrics = service.compute_metrics(false).unwrap();
    assert_eq!(metrics.total_skills, 1);
    assert_eq!(metrics.by_source.values().sum::<usize>(), 1);
}
