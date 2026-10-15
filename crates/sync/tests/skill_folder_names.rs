//! Every adapter writes a skill to the same on-disk folder (M14), and marks
//! each skill directory it writes as a skrills copy (M21).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use skrills_sync::{
    AgentAdapter, ClaudeAdapter, CodexAdapter, Command, CopilotAdapter, CursorAdapter,
};
use tempfile::TempDir;

const SYNCED_MARKER: &str = ".skrills-synced";

fn skill(name: &str) -> Command {
    let content = format!("---\nname: {name}\ndescription: test skill\n---\n# Body\n").into_bytes();
    Command {
        name: name.to_string(),
        content,
        hash: "unused".to_string(),
        source_path: PathBuf::from(format!("/src/{name}/SKILL.md")),
        modified: SystemTime::UNIX_EPOCH,
        modules: Vec::new(),
        content_format: Default::default(),
        plugin_origin: None,
    }
}

/// Each adapter under a fresh root, with the directory it writes skills to.
fn adapters(tmp: &Path) -> Vec<(Box<dyn AgentAdapter>, PathBuf)> {
    let root = |name: &str| tmp.join(name);
    vec![
        (
            Box::new(ClaudeAdapter::with_root(root("claude"))),
            root("claude").join("skills"),
        ),
        (
            Box::new(CodexAdapter::with_root(root("codex"))),
            root("codex").join(".agents/skills"),
        ),
        (
            Box::new(CopilotAdapter::with_root(root("copilot"))),
            root("copilot").join("skills"),
        ),
        (
            Box::new(CursorAdapter::with_root(root("cursor"))),
            root("cursor").join("skills"),
        ),
    ]
}

#[test]
fn every_adapter_writes_a_skill_to_the_same_folder() {
    let tmp = TempDir::new().unwrap();
    let skills = [skill("synced/abc/morning"), skill("a/b"), skill("ab")];

    for (adapter, dir) in adapters(tmp.path()) {
        let report = adapter.write_skills(&skills).unwrap();
        assert_eq!(report.written, 3, "{}: {report:?}", adapter.name());
        for folder in ["morning", "b", "ab"] {
            assert!(
                dir.join(folder).join("SKILL.md").is_file(),
                "{}: no {folder}/SKILL.md under {}",
                adapter.name(),
                dir.display()
            );
        }

        let mut names: Vec<String> = adapter
            .read_skills()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        names.sort();
        assert_eq!(names, ["ab", "b", "morning"], "{}", adapter.name());
    }
}

#[test]
fn every_skill_directory_skrills_writes_carries_the_synced_marker() {
    let tmp = TempDir::new().unwrap();

    for (adapter, dir) in adapters(tmp.path()) {
        let user_dir = dir.join("mine");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(user_dir.join("SKILL.md"), "---\nname: mine\n---\n").unwrap();

        adapter.write_skills(&[skill("demo")]).unwrap();

        assert!(
            dir.join("demo").join(SYNCED_MARKER).is_file(),
            "{}: synced dir has no marker",
            adapter.name()
        );
        assert!(
            !user_dir.join(SYNCED_MARKER).exists(),
            "{}: user dir was marked",
            adapter.name()
        );
        assert_eq!(
            std::fs::read_to_string(user_dir.join("SKILL.md")).unwrap(),
            "---\nname: mine\n---\n"
        );

        // The marker is not a module: reading the copy back carries no trace of it.
        let demo = adapter
            .read_skills()
            .unwrap()
            .into_iter()
            .find(|s| s.name == "demo")
            .unwrap();
        assert!(
            demo.modules
                .iter()
                .all(|m| !m.relative_path.ends_with(SYNCED_MARKER)),
            "{}",
            adapter.name()
        );
    }
}
