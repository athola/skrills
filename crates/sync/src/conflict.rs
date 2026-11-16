//! Conflict detection: an item both sides changed since the last sync.
//!
//! After each successful, non-dry sync the orchestrator records, per item, the
//! hash of what the source held and the hash of what the target held once the
//! write was done. Both are kept because a target that rewrites on write
//! (Cursor, Copilot agents, Codex skills) never reads back the source's bytes,
//! so one hash cannot tell "the target was edited" from "the target is the
//! source's transformed copy". A later sync reports a conflict only when the
//! source and the target both differ from their recorded hashes. With no
//! record a first sync cannot know, so it is never a conflict.
//!
//! Only hashes are stored. Configs can hold secrets, so no content is kept in
//! the baseline or in the resolution log.

use crate::adapters::utils::{atomic_write_unrecorded, hash_content};
use crate::common::{Command, McpServer};
use crate::decision::{Decision, ItemKind};
use crate::orchestrator::is_unchanged;
use crate::Result;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Log of the choices a person made on conflicts, one JSON line each.
pub const RESOLUTIONS_FILE: &str = "resolutions.jsonl";

/// Bumped when the baseline's layout changes; a file of another version is
/// ignored rather than misread.
const FORMAT_VERSION: u32 = 1;

/// `~/.skrills/sync-state`, resolved as [`crate::snapshot::default_root`] is.
pub fn default_root() -> Option<PathBuf> {
    crate::snapshot::skrills_home().map(|home| home.join("sync-state"))
}

/// Names a target's baseline file so two platforms, or one platform rooted in
/// two places, never share one.
pub(crate) fn target_key(platform: &str, config_root: &Path) -> String {
    let digest = hash_content(config_root.to_string_lossy().as_bytes());
    let platform: String = platform
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{platform}-{}", &digest[..12])
}

/// What an item hashed to at the last successful sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub source: String,
    pub target: String,
}

#[derive(Serialize, Deserialize)]
struct BaselineFile {
    version: u32,
    items: BTreeMap<String, Entry>,
}

/// Hash of an artifact: its main file and every module, so a change to a
/// companion file counts as a change to the item.
pub(crate) fn item_hash(item: &Command) -> String {
    let mut bytes = Vec::with_capacity(item.content.len() + 16);
    let mut push = |part: &[u8]| {
        bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());
        bytes.extend_from_slice(part);
    };
    push(&item.content);
    let mut modules: Vec<_> = item.modules.iter().collect();
    modules.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    for module in modules {
        push(module.relative_path.to_string_lossy().as_bytes());
        push(&module.content);
    }
    hash_content(&bytes)
}

/// Hash of an MCP server's settings, independent of map iteration order.
pub(crate) fn server_hash(server: &McpServer) -> String {
    let mut text = String::new();
    // A `McpServer` always serializes; the fallback only keeps this total.
    let value = serde_json::to_value(server).unwrap_or(serde_json::Value::Null);
    write_canonical(&value, &mut text);
    hash_content(text.as_bytes())
}

/// JSON with object keys in sorted order at every depth.
fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for key in keys {
                out.push_str(&serde_json::Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(&map[key], out);
                out.push(',');
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for item in items {
                write_canonical(item, out);
                out.push(',');
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// The conflict rule on hashes alone.
pub(crate) fn is_conflict(baseline: Option<&Entry>, source: &str, target: &str) -> bool {
    baseline.is_some_and(|base| source != target && source != base.source && target != base.target)
}

fn entry_key(kind: ItemKind, name: &str) -> String {
    format!("{kind}:{name}")
}

/// A conflict found during a sync, with both hashes for the resolution log.
#[derive(Debug, Clone)]
struct Detected {
    kind: ItemKind,
    name: String,
    source: String,
    target: String,
}

/// Whether a decision covered one item or its whole category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Item,
    Category,
}

/// The baseline loaded at the start of a sync, the conflicts found while it
/// ran, and the entries to write once it succeeds.
pub(crate) struct Ledger {
    root: Option<PathBuf>,
    key: String,
    baseline: BTreeMap<String, Entry>,
    found: RefCell<BTreeMap<String, Detected>>,
    updates: RefCell<BTreeMap<String, Entry>>,
}

impl Ledger {
    /// Loads the baseline for `key` from `root`. A missing, unreadable or
    /// unrecognised file is an empty baseline: no conflicts are reported
    /// rather than guessed at.
    pub fn load(root: Option<&Path>, key: String) -> Self {
        let baseline = root
            .map(|root| root.join(format!("{key}.json")))
            .map(|path| read_baseline(&path))
            .unwrap_or_default();
        Self {
            root: root.map(Path::to_path_buf),
            key,
            baseline,
            found: RefCell::default(),
            updates: RefCell::default(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.root.is_some()
    }

    /// Whether a baseline exists to compare against.
    pub fn has_baseline(&self) -> bool {
        !self.baseline.is_empty()
    }

    /// Whether `source` and the target's copy `held` both changed since the
    /// last sync. Remembers the item when they did.
    pub fn file_conflict(&self, kind: ItemKind, source: &Command, held: Option<&Command>) -> bool {
        let Some(held) = held else {
            return false;
        };
        if is_unchanged(held, source) {
            return false;
        }
        self.remember(kind, &source.name, item_hash(source), item_hash(held))
    }

    /// [`Self::file_conflict`] for an MCP server.
    pub fn server_conflict(
        &self,
        name: &str,
        source: &McpServer,
        held: Option<&McpServer>,
    ) -> bool {
        match held {
            Some(held) if held != source => self.remember(
                ItemKind::McpServer,
                name,
                server_hash(source),
                server_hash(held),
            ),
            _ => false,
        }
    }

    fn remember(&self, kind: ItemKind, name: &str, source: String, target: String) -> bool {
        let key = entry_key(kind, name);
        if !is_conflict(self.baseline.get(&key), &source, &target) {
            return false;
        }
        self.found.borrow_mut().insert(
            key,
            Detected {
                kind,
                name: name.to_string(),
                source,
                target,
            },
        );
        true
    }

    /// Whether `name` was found in conflict during this sync.
    pub fn is_found(&self, kind: ItemKind, name: &str) -> bool {
        self.found.borrow().contains_key(&entry_key(kind, name))
    }

    /// Names found in conflict for `kind`, sorted.
    pub fn found_names(&self, kind: ItemKind) -> Vec<String> {
        self.found
            .borrow()
            .values()
            .filter(|found| found.kind == kind)
            .map(|found| found.name.clone())
            .collect()
    }

    /// Notes what `source` and the target's copy `held_after` hashed to once
    /// the write was done, for [`Self::persist`].
    pub fn record_file(&self, kind: ItemKind, source: &Command, held_after: &Command) {
        if self.is_enabled() {
            self.note(kind, &source.name, item_hash(source), item_hash(held_after));
        }
    }

    /// [`Self::record_file`] for an MCP server.
    pub fn record_server(&self, name: &str, source: &McpServer, held_after: &McpServer) {
        if self.is_enabled() {
            self.note(
                ItemKind::McpServer,
                name,
                server_hash(source),
                server_hash(held_after),
            );
        }
    }

    fn note(&self, kind: ItemKind, name: &str, source: String, target: String) {
        self.updates
            .borrow_mut()
            .insert(entry_key(kind, name), Entry { source, target });
    }

    /// Writes the merged baseline, then appends a line to the resolution log
    /// for every conflict `answer` says the person decided.
    pub fn persist(
        &self,
        now: SystemTime,
        answer: impl Fn(ItemKind, &str) -> Option<(Decision, Scope)>,
    ) -> Result<()> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        let updates = self.updates.borrow();
        if !updates.is_empty() {
            let mut items = self.baseline.clone();
            items.extend(updates.iter().map(|(k, v)| (k.clone(), v.clone())));
            let file = BaselineFile {
                version: FORMAT_VERSION,
                items,
            };
            let path = root.join(format!("{}.json", self.key));
            let text = serde_json::to_vec_pretty(&file)?;
            atomic_write_unrecorded(&path, &text, true)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }

        let lines: Vec<String> = self
            .found
            .borrow()
            .values()
            .filter_map(|found| {
                let (decision, scope) = answer(found.kind, &found.name)?;
                Some(self.resolution_line(found, decision, scope, now))
            })
            .collect();
        if lines.is_empty() {
            return Ok(());
        }
        let path = root.join(RESOLUTIONS_FILE);
        let mut log = match std::fs::read_to_string(&path) {
            Ok(existing) => existing,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("Failed to read {}", path.display())),
        };
        for line in lines {
            log.push_str(&line);
            log.push('\n');
        }
        atomic_write_unrecorded(&path, log.as_bytes(), true)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    fn resolution_line(
        &self,
        found: &Detected,
        decision: Decision,
        scope: Scope,
        now: SystemTime,
    ) -> String {
        let (year, month, day, hour, minute, second) = crate::snapshot::civil(now);
        let record = serde_json::json!({
            "timestamp": format!(
                "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
            ),
            "target": self.key,
            "kind": found.kind.to_string(),
            "name": found.name,
            "choice": match decision {
                Decision::Accept => "keep_source",
                Decision::Reject => "keep_target",
            },
            "scope": match scope {
                Scope::Item => "item",
                Scope::Category => "category",
            },
            "source_hash": found.source,
            "target_hash": found.target,
        });
        record.to_string()
    }
}

fn read_baseline(path: &Path) -> BTreeMap<String, Entry> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Sync baseline unreadable; skipping conflict detection");
            return BTreeMap::new();
        }
    };
    match serde_json::from_str::<BaselineFile>(&text) {
        Ok(file) if file.version == FORMAT_VERSION => file.items,
        Ok(file) => {
            tracing::warn!(path = %path.display(), version = file.version, "Sync baseline has an unknown version; skipping conflict detection");
            BTreeMap::new()
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Sync baseline is corrupt; skipping conflict detection");
            BTreeMap::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn entry(source: &str, target: &str) -> Entry {
        Entry {
            source: source.to_string(),
            target: target.to_string(),
        }
    }

    fn command(name: &str, body: &str) -> Command {
        Command {
            name: name.to_string(),
            content: body.as_bytes().to_vec(),
            source_path: PathBuf::from(format!("{name}.md")),
            modified: SystemTime::UNIX_EPOCH,
            hash: String::new(),
            modules: Vec::new(),
            content_format: Default::default(),
            plugin_origin: None,
        }
    }

    fn server(env: &[(&str, &str)]) -> McpServer {
        McpServer {
            name: "srv".to_string(),
            transport: Default::default(),
            command: "/bin/srv".to_string(),
            args: Vec::new(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
            url: None,
            headers: None,
            enabled: true,
            allowed_tools: Vec::new(),
            disabled_tools: Vec::new(),
        }
    }

    #[test]
    fn no_baseline_entry_is_never_a_conflict() {
        assert!(!is_conflict(None, "s2", "t2"));
    }

    #[test]
    fn only_the_source_changing_is_not_a_conflict() {
        assert!(!is_conflict(Some(&entry("s1", "t1")), "s2", "t1"));
    }

    #[test]
    fn only_the_target_changing_is_not_a_conflict() {
        assert!(!is_conflict(Some(&entry("s1", "t1")), "s1", "t2"));
    }

    #[test]
    fn both_sides_changing_is_a_conflict() {
        assert!(is_conflict(Some(&entry("s1", "t1")), "s2", "t2"));
    }

    #[test]
    fn both_sides_changing_to_the_same_content_is_not_a_conflict() {
        assert!(!is_conflict(Some(&entry("s1", "t1")), "same", "same"));
    }

    #[test]
    fn an_item_hash_changes_when_only_a_module_changes() {
        let plain = command("skill", "body");
        let mut with_module = plain.clone();
        with_module.modules.push(crate::common::ModuleFile {
            relative_path: PathBuf::from("notes.md"),
            content: b"notes".to_vec(),
            hash: String::new(),
        });

        assert_ne!(item_hash(&plain), item_hash(&with_module));
        assert_eq!(item_hash(&plain), item_hash(&plain.clone()));
    }

    #[test]
    fn a_server_hash_ignores_environment_insertion_order() {
        let forward = server(&[("A", "1"), ("B", "2"), ("C", "3")]);
        let backward = server(&[("C", "3"), ("B", "2"), ("A", "1")]);

        assert_eq!(server_hash(&forward), server_hash(&backward));
        assert_ne!(server_hash(&forward), server_hash(&server(&[("A", "9")])));
    }

    #[test]
    fn target_keys_differ_by_platform_and_by_root() {
        let one = target_key("claude", Path::new("/home/a/.claude"));

        assert_ne!(one, target_key("codex", Path::new("/home/a/.claude")));
        assert_ne!(one, target_key("claude", Path::new("/home/b/.claude")));
        assert_eq!(one, target_key("claude", Path::new("/home/a/.claude")));
        assert!(one.starts_with("claude-"), "{one}");
        assert!(one.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn a_ledger_with_a_baseline_finds_a_two_sided_change() {
        let root = tempfile::tempdir().unwrap();
        let (src_v1, tgt_v1) = (command("c", "src1"), command("c", "tgt1"));
        let writer = Ledger::load(Some(root.path()), "claude-k".to_string());
        writer.record_file(ItemKind::Command, &src_v1, &tgt_v1);
        writer.persist(SystemTime::UNIX_EPOCH, |_, _| None).unwrap();

        let reader = Ledger::load(Some(root.path()), "claude-k".to_string());

        let (src_v2, tgt_v2) = (command("c", "src2"), command("c", "tgt2"));
        assert!(reader.file_conflict(ItemKind::Command, &src_v2, Some(&tgt_v2)));
        assert!(reader.is_found(ItemKind::Command, "c"));
        assert_eq!(reader.found_names(ItemKind::Command), vec!["c".to_string()]);
        assert!(!reader.file_conflict(ItemKind::Command, &src_v2, Some(&tgt_v1)));
        assert!(!reader.file_conflict(ItemKind::Command, &src_v2, None));
    }

    #[test]
    fn a_corrupt_baseline_file_means_no_conflicts_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("claude-k.json"), "{not json").unwrap();

        let ledger = Ledger::load(Some(root.path()), "claude-k".to_string());

        assert!(!ledger.has_baseline());
    }

    #[test]
    fn persist_merges_new_entries_into_the_existing_baseline() {
        let root = tempfile::tempdir().unwrap();
        let first = Ledger::load(Some(root.path()), "k".to_string());
        first.record_file(ItemKind::Command, &command("a", "1"), &command("a", "1"));
        first.persist(SystemTime::UNIX_EPOCH, |_, _| None).unwrap();

        let second = Ledger::load(Some(root.path()), "k".to_string());
        second.record_file(ItemKind::Command, &command("b", "2"), &command("b", "2"));
        second.persist(SystemTime::UNIX_EPOCH, |_, _| None).unwrap();

        let raw = std::fs::read_to_string(root.path().join("k.json")).unwrap();
        assert!(
            raw.contains("command:a") && raw.contains("command:b"),
            "{raw}"
        );
    }

    #[test]
    fn persist_writes_one_content_free_line_per_decided_conflict() {
        let root = tempfile::tempdir().unwrap();
        let seed = Ledger::load(Some(root.path()), "claude-k".to_string());
        seed.record_file(
            ItemKind::Command,
            &command("c", "src1 SECRET"),
            &command("c", "tgt1 SECRET"),
        );
        seed.persist(SystemTime::UNIX_EPOCH, |_, _| None).unwrap();
        let ledger = Ledger::load(Some(root.path()), "claude-k".to_string());
        assert!(ledger.file_conflict(
            ItemKind::Command,
            &command("c", "src2 SECRET"),
            Some(&command("c", "tgt2 SECRET"))
        ));

        ledger
            .persist(SystemTime::UNIX_EPOCH, |kind, name| {
                assert_eq!((kind, name), (ItemKind::Command, "c"));
                Some((Decision::Reject, Scope::Item))
            })
            .unwrap();

        let log = std::fs::read_to_string(root.path().join(RESOLUTIONS_FILE)).unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 1, "{log}");
        let record: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(record["choice"], "keep_target");
        assert_eq!(record["kind"], "command");
        assert_eq!(record["name"], "c");
        assert_eq!(record["target"], "claude-k");
        assert_eq!(record["scope"], "item");
        assert_eq!(record["timestamp"], "1970-01-01T00:00:00Z");
        assert_eq!(
            record["source_hash"],
            item_hash(&command("c", "src2 SECRET"))
        );
        assert_eq!(
            record["target_hash"],
            item_hash(&command("c", "tgt2 SECRET"))
        );
        assert!(!log.contains("SECRET"), "{log}");
    }

    #[test]
    fn a_conflict_nobody_answered_is_not_logged() {
        let root = tempfile::tempdir().unwrap();
        let seed = Ledger::load(Some(root.path()), "k".to_string());
        seed.record_file(ItemKind::Command, &command("c", "1"), &command("c", "1"));
        seed.persist(SystemTime::UNIX_EPOCH, |_, _| None).unwrap();
        let ledger = Ledger::load(Some(root.path()), "k".to_string());
        assert!(ledger.file_conflict(
            ItemKind::Command,
            &command("c", "2"),
            Some(&command("c", "3"))
        ));

        ledger.persist(SystemTime::UNIX_EPOCH, |_, _| None).unwrap();

        assert!(!root.path().join(RESOLUTIONS_FILE).exists());
    }

    #[test]
    fn a_disabled_ledger_finds_and_writes_nothing() {
        let ledger = Ledger::load(None, String::new());

        ledger.record_file(ItemKind::Command, &command("c", "1"), &command("c", "1"));

        assert!(!ledger.is_enabled());
        assert!(ledger.persist(SystemTime::UNIX_EPOCH, |_, _| None).is_ok());
    }

    #[test]
    fn the_default_root_is_under_home_skrills_sync_state() {
        let _env = skrills_test_utils::env_guard();
        let home = tempfile::tempdir().unwrap();
        let _home = skrills_test_utils::set_env_var("HOME", home.path().to_str());

        assert_eq!(
            default_root(),
            Some(home.path().join(".skrills").join("sync-state"))
        );
    }
}
