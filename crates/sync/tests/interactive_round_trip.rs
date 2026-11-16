//! An interactive sync keeps the target's copy of a rejected, changed item by
//! handing the writer the target's own read-back item. That is only safe for
//! a target and category that give the bytes back: this test holds every
//! `DecisionScope::PerItem` declaration to it, so an adapter cannot opt a
//! category in without proof.
//!
//! The targets are perturbed the way a person's edits would: a trailing
//! comment, an extra frontmatter key, a name the writer would rewrite, and no
//! `.skrills-synced` marker (a directory skrills did not write).

use skrills_sync::adapters::traits::AgentAdapter;
use skrills_sync::common::ModuleFile;
use skrills_sync::{
    ClaudeAdapter, CodexAdapter, Command, CopilotAdapter, CursorAdapter, DecisionScope, ItemKind,
    McpServer,
};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const ADAPTERS: [&str; 4] = ["claude", "codex", "copilot", "cursor"];
const FILE_KINDS: [ItemKind; 5] = [
    ItemKind::Command,
    ItemKind::Skill,
    ItemKind::Agent,
    ItemKind::Instruction,
    ItemKind::Hook,
];

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name() != ".skrills-sync.lock")
        .map(|entry| {
            let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
            (relative, std::fs::read(entry.path()).unwrap())
        })
        .collect()
}

fn item(name: &str, body: &str) -> Command {
    Command::new(
        name.into(),
        body.as_bytes().to_vec(),
        PathBuf::from(format!("/src/{name}.md")),
    )
}

fn skill(name: &str) -> Command {
    let mut skill = item(
        name,
        &format!("---\nname: {name}\ndescription: D\n---\n\nBody\n"),
    );
    skill.modules.push(ModuleFile {
        relative_path: "notes.md".into(),
        content: b"notes\n".to_vec(),
        hash: String::new(),
    });
    skill
}

fn seed(kind: ItemKind, name: &str) -> Command {
    match kind {
        ItemKind::Skill => skill(name),
        ItemKind::Agent => item(
            name,
            &format!("---\nname: {name}\ndescription: R\n---\nBody\n"),
        ),
        ItemKind::Instruction => item("CLAUDE", "# Rules\n\nbe nice\n"),
        ItemKind::Hook => {
            let mut hook = item(
                "PreToolUse",
                r#"[{"matcher":"Bash","hooks":[{"type":"command","command":"echo hi"}]}]"#,
            );
            hook.content_format = skrills_sync::ContentFormat::Json;
            hook
        }
        _ => item(name, "# Hello\n"),
    }
}

fn adapter(name: &str, root: &Path) -> Box<dyn AgentAdapter> {
    match name {
        "claude" => Box::new(ClaudeAdapter::with_root(root.to_path_buf())),
        "codex" => Box::new(CodexAdapter::with_root(root.to_path_buf())),
        "copilot" => Box::new(CopilotAdapter::with_root(root.to_path_buf())),
        _ => Box::new(CursorAdapter::with_root(root.to_path_buf())),
    }
}

type Reader<'a> = Box<dyn Fn() -> skrills_sync::Result<Vec<Command>> + 'a>;
type Writer<'a> = Box<dyn Fn(&[Command]) -> skrills_sync::Result<skrills_sync::WriteReport> + 'a>;

fn reader<'a>(a: &'a dyn AgentAdapter, kind: ItemKind) -> Reader<'a> {
    match kind {
        ItemKind::Skill => Box::new(|| a.read_skills()),
        ItemKind::Agent => Box::new(|| a.read_agents()),
        ItemKind::Instruction => Box::new(|| a.read_instructions()),
        ItemKind::Hook => Box::new(|| a.read_hooks()),
        _ => Box::new(|| a.read_commands(false)),
    }
}

fn writer<'a>(a: &'a dyn AgentAdapter, kind: ItemKind) -> Writer<'a> {
    match kind {
        ItemKind::Skill => Box::new(|items| a.write_skills(items)),
        ItemKind::Agent => Box::new(|items| a.write_agents(items)),
        ItemKind::Instruction => Box::new(|items| a.write_instructions(items)),
        ItemKind::Hook => Box::new(|items| a.write_hooks(items)),
        _ => Box::new(|items| a.write_commands(items)),
    }
}

/// Edits every text file under `root` the way `perturbation` says.
fn perturb(root: &Path, perturbation: usize) {
    for (path, bytes) in tree(root) {
        let full = root.join(&path);
        if path.file_name().is_some_and(|n| n == ".skrills-synced") {
            if perturbation == 3 {
                std::fs::remove_file(full).unwrap();
            }
            continue;
        }
        if path.extension().is_none_or(|e| e != "md" && e != "mdc") {
            continue;
        }
        let text = String::from_utf8(bytes).unwrap();
        let edited = match perturbation {
            0 => format!("{text}\n<!-- user -->\n"),
            1 => text.replacen("---\n", "---\nx-extra: 1\n", 1),
            2 => text.replacen("name: ", "name: Mixed ", 1),
            _ => continue,
        };
        std::fs::write(full, edited).unwrap();
    }
}

#[test]
fn every_per_item_category_gives_the_targets_bytes_back() {
    for name in ADAPTERS {
        for kind in FILE_KINDS {
            let root = tempfile::tempdir().unwrap();
            let target = adapter(name, &root.path().join(name));
            if target.decision_scope(kind) != DecisionScope::PerItem {
                continue;
            }
            let (read, write) = (reader(&*target, kind), writer(&*target, kind));
            write(&[seed(kind, "alpha")]).unwrap();
            for perturbation in 0..4 {
                perturb(&root.path().join(name), perturbation);
                let before = tree(&root.path().join(name));
                let back: Vec<Command> = read()
                    .unwrap()
                    .into_iter()
                    .filter(|c| c.plugin_origin.is_none())
                    .collect();
                assert!(!back.is_empty(), "{name} {kind:?}: nothing read back");

                write(&back).unwrap();

                assert_eq!(
                    tree(&root.path().join(name)),
                    before,
                    "{name} {kind:?} perturbation {perturbation}: not byte-identical, \
                     so it must be DecisionScope::WholeCategory"
                );
            }
        }
    }
}

#[test]
fn every_per_item_category_keeps_what_a_smaller_batch_leaves_out() {
    for name in ADAPTERS {
        for kind in [ItemKind::Command, ItemKind::Skill, ItemKind::Agent] {
            let root = tempfile::tempdir().unwrap();
            let target = adapter(name, &root.path().join(name));
            if target.decision_scope(kind) != DecisionScope::PerItem {
                continue;
            }
            let write = writer(&*target, kind);
            write(&[seed(kind, "alpha"), seed(kind, "beta")]).unwrap();
            let before = tree(&root.path().join(name));

            write(&[seed(kind, "alpha")]).unwrap();

            assert_eq!(
                tree(&root.path().join(name)),
                before,
                "{name} {kind:?}: a batch without `beta` deleted or changed files"
            );
        }
    }
}

fn server(name: &str, arg: &str) -> McpServer {
    McpServer {
        name: name.to_string(),
        transport: Default::default(),
        command: "/bin/srv".to_string(),
        args: vec![arg.to_string()],
        env: HashMap::new(),
        url: None,
        headers: None,
        enabled: true,
        allowed_tools: Vec::new(),
        disabled_tools: Vec::new(),
    }
}

#[test]
fn mcp_servers_give_the_targets_config_back_when_one_is_kept() {
    for name in ADAPTERS {
        let root = tempfile::tempdir().unwrap();
        let target = adapter(name, &root.path().join(name));
        if target.decision_scope(ItemKind::McpServer) != DecisionScope::PerItem {
            continue;
        }
        let both = HashMap::from([
            ("one".to_string(), server("one", "1")),
            ("two".to_string(), server("two", "2")),
        ]);
        target.write_mcp_servers(&both).unwrap();
        // A person's edits: a key skrills does not model, and their layout.
        for (path, bytes) in tree(&root.path().join(name)) {
            let full = root.path().join(name).join(&path);
            let text = String::from_utf8(bytes).unwrap();
            match path.extension().and_then(|e| e.to_str()) {
                Some("json") => {
                    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
                    add_extra_key(&mut value);
                    std::fs::write(full, serde_json::to_string(&value).unwrap()).unwrap();
                }
                Some("toml") => {
                    let edited = text.replace("command = ", "x-extra = 1\ncommand = ");
                    std::fs::write(full, format!("# mine\n\n{edited}")).unwrap();
                }
                _ => {}
            }
        }
        let before = tree(&root.path().join(name));
        let held = target.read_mcp_servers().unwrap();
        assert_eq!(held.len(), 2, "{name}: {held:?}");

        // What a rejected change to "one" hands the writer, with "two" accepted.
        let mut handed = held.clone();
        handed.insert("two".to_string(), server("two", "2"));
        target.write_mcp_servers(&handed).unwrap();

        assert_eq!(
            tree(&root.path().join(name)),
            before,
            "{name}: keeping a server rewrote the config, so MCP must be WholeCategory"
        );
    }
}

fn add_extra_key(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if map.contains_key("command") || map.contains_key("url") {
                map.insert("x-extra".to_string(), serde_json::json!(1));
            }
            map.values_mut().for_each(add_extra_key);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(add_extra_key),
        _ => {}
    }
}
