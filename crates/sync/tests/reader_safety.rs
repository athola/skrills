//! What every single-file reader (commands, agents, rules, instructions)
//! must refuse to pick up from a tool's directory.

use skrills_sync::adapters::{
    AgentAdapter, ClaudeAdapter, CodexAdapter, CopilotAdapter, CursorAdapter,
};
use skrills_sync::common::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

type Reader = fn(&dyn AgentAdapter) -> Vec<Command>;

fn commands(adapter: &dyn AgentAdapter) -> Vec<Command> {
    adapter.read_commands(false).unwrap()
}
fn agents(adapter: &dyn AgentAdapter) -> Vec<Command> {
    adapter.read_agents().unwrap()
}
fn instructions(adapter: &dyn AgentAdapter) -> Vec<Command> {
    adapter.read_instructions().unwrap()
}

/// A reader under test: label, adapter, directory it reads, file suffix.
type ReaderCase = (
    &'static str,
    Box<dyn AgentAdapter>,
    PathBuf,
    &'static str,
    Reader,
);

/// Each reader with the directory it reads and the file suffix it expects.
fn readers(root: &Path) -> Vec<ReaderCase> {
    let root = root.to_path_buf();
    vec![
        (
            "claude commands",
            Box::new(ClaudeAdapter::with_root(root.clone())),
            root.join("commands"),
            ".md",
            commands as Reader,
        ),
        (
            "codex commands",
            Box::new(CodexAdapter::with_root(root.clone())),
            root.join("prompts"),
            ".md",
            commands,
        ),
        (
            "cursor commands",
            Box::new(CursorAdapter::with_root(root.clone())),
            root.join("commands"),
            ".md",
            commands,
        ),
        (
            "cursor agents",
            Box::new(CursorAdapter::with_root(root.clone())),
            root.join("agents"),
            ".md",
            agents,
        ),
        (
            "cursor rules",
            Box::new(CursorAdapter::with_root(root.clone())),
            root.join("rules"),
            ".mdc",
            instructions,
        ),
        (
            "copilot agents",
            Box::new(CopilotAdapter::with_root(root.clone())),
            root.join("agents"),
            ".agent.md",
            agents,
        ),
        (
            "copilot instructions",
            Box::new(CopilotAdapter::with_root(root.clone())),
            root.join("instructions"),
            ".instructions.md",
            instructions,
        ),
    ]
}

/// A link in a tool's directory can point at any file on disk, and whatever is
/// read gets copied into every other tool's directory.
#[cfg(unix)]
#[test]
fn readers_skip_symlinked_files_and_hidden_paths() {
    let outside = TempDir::new().unwrap();
    let secret = outside.path().join("secret.md");
    std::fs::write(&secret, "secret\n").unwrap();

    let tmp = TempDir::new().unwrap();
    for (label, adapter, dir, suffix, read) in readers(tmp.path()) {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("real{suffix}")), "real\n").unwrap();
        std::os::unix::fs::symlink(&secret, dir.join(format!("leak{suffix}"))).unwrap();
        std::fs::write(dir.join(format!(".hidden{suffix}")), "hidden\n").unwrap();

        let mut names: Vec<String> = read(adapter.as_ref())
            .into_iter()
            .map(|item| item.name)
            .collect();
        names.sort();

        assert_eq!(names, vec!["real".to_string()], "{label}");
    }
}
