//! Destination checks shared by every skill writer.

use skrills_sync::adapters::{
    AgentAdapter, ClaudeAdapter, CodexAdapter, CopilotAdapter, CursorAdapter,
};
use skrills_sync::common::{Command, ModuleFile};
use skrills_sync::report::SkipReason;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn skill(name: &str, body: &str) -> Command {
    Command::new(
        name.to_string(),
        format!("---\nname: {name}\ndescription: d\n---\n{body}\n").into_bytes(),
        PathBuf::from(format!("/src/{name}/SKILL.md")),
    )
}

fn module(rel: &str, content: &str) -> ModuleFile {
    ModuleFile {
        relative_path: PathBuf::from(rel),
        content: content.as_bytes().to_vec(),
        hash: String::new(),
    }
}

/// Each adapter with the directory its flat skills live in.
fn adapters(root: &Path) -> Vec<(Box<dyn AgentAdapter>, PathBuf)> {
    vec![
        (
            Box::new(ClaudeAdapter::with_root(root.join("claude"))),
            root.join("claude/skills"),
        ),
        (
            Box::new(CodexAdapter::with_root(root.join("codex"))),
            root.join("codex/skills"),
        ),
        (
            Box::new(CopilotAdapter::with_root(root.join("copilot"))),
            root.join("copilot/skills"),
        ),
        (
            Box::new(CursorAdapter::with_root(root.join("cursor"))),
            root.join("cursor/skills"),
        ),
    ]
}

/// A user edits `scripts/run.py` without touching SKILL.md. Three writers
/// skipped the whole skill as unchanged, so the edit never arrived.
#[test]
fn a_changed_module_is_written_even_when_skill_md_is_unchanged() {
    let tmp = TempDir::new().unwrap();
    for (adapter, skills_dir) in adapters(tmp.path()) {
        let mut first = skill("tool", "Body");
        first.modules = vec![module("scripts/run.py", "v1")];
        adapter.write_skills(&[first]).unwrap();

        let mut second = skill("tool", "Body");
        second.modules = vec![module("scripts/run.py", "v2")];
        let report = adapter.write_skills(&[second]).unwrap();

        assert_eq!(
            std::fs::read_to_string(skills_dir.join("tool/scripts/run.py")).unwrap(),
            "v2",
            "{}",
            adapter.name()
        );
        assert_eq!(report.written, 1, "{}", adapter.name());

        let mut third = skill("tool", "Body");
        third.modules = vec![module("scripts/run.py", "v2")];
        let report = adapter.write_skills(&[third]).unwrap();
        assert_eq!(report.written, 0, "{}: nothing changed", adapter.name());
    }
}

/// `~/.claude/skills/foo` is commonly a symlink into a git checkout; writing
/// through it overwrote the user's working tree.
#[cfg(unix)]
#[test]
fn a_symlinked_skill_directory_is_not_written_through() {
    let tmp = TempDir::new().unwrap();
    for (adapter, skills_dir) in adapters(tmp.path()) {
        let checkout = tmp.path().join(format!("checkout-{}", adapter.name()));
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(checkout.join("SKILL.md"), "mine").unwrap();
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::os::unix::fs::symlink(&checkout, skills_dir.join("foo")).unwrap();

        let report = adapter.write_skills(&[skill("foo", "theirs")]).unwrap();

        assert_eq!(
            std::fs::read_to_string(checkout.join("SKILL.md")).unwrap(),
            "mine",
            "{}",
            adapter.name()
        );
        assert!(
            report
                .skipped
                .iter()
                .any(|s| matches!(s, SkipReason::Refused { .. })),
            "{}: {:?}",
            adapter.name(),
            report.skipped
        );
    }
}

/// `my.skill` and `myskill` sanitize to one directory; the second silently
/// overwrote the first and both were reported written.
#[test]
fn two_names_that_sanitize_alike_do_not_overwrite_each_other() {
    let tmp = TempDir::new().unwrap();
    for (adapter, skills_dir) in adapters(tmp.path()) {
        if adapter.name() == "cursor" {
            // Cursor kebab-cases: `my.skill` becomes `my-skill`, a distinct name.
            continue;
        }
        let report = adapter
            .write_skills(&[skill("my.skill", "first"), skill("myskill", "second")])
            .unwrap();

        assert_eq!(report.written, 1, "{}", adapter.name());
        let text = std::fs::read_to_string(skills_dir.join("myskill/SKILL.md")).unwrap();
        assert!(text.contains("first"), "{}: {text}", adapter.name());
    }
}

/// A name that sanitizes to nothing wrote `SKILL.md` straight into the skills
/// root.
#[test]
fn a_name_that_sanitizes_to_nothing_is_refused() {
    let tmp = TempDir::new().unwrap();
    for (adapter, skills_dir) in adapters(tmp.path()) {
        let report = adapter.write_skills(&[skill("...", "x")]).unwrap();
        assert!(
            !skills_dir.join("SKILL.md").exists(),
            "{}: wrote into the skills root",
            adapter.name()
        );
        assert_eq!(report.written, 0, "{}", adapter.name());
    }
}

/// Codex finds SKILL.md at any depth, so a nested name keeps its directory.
#[test]
fn codex_keeps_nested_skill_directories() {
    let tmp = TempDir::new().unwrap();
    let codex = CodexAdapter::with_root(tmp.path().to_path_buf());
    codex.write_skills(&[skill("nested/foo", "x")]).unwrap();
    assert!(tmp.path().join("skills/nested/foo/SKILL.md").exists());
}
