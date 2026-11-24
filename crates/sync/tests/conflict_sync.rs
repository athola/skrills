//! Conflict detection: an item both the source and the target changed since
//! the last sync. Every test keeps its state in a tempdir; none reads or
//! writes the real `~/.skrills`.

use skrills_sync::{
    ClaudeAdapter, CopilotAdapter, Decision, DecisionScope, ItemChange, ItemDecider, ItemKind,
    SyncError, SyncOrchestrator, SyncParams, SyncReport,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A source root, a target root and a state root, each its own tempdir.
struct Rig {
    source: tempfile::TempDir,
    target: tempfile::TempDir,
    state: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        Self {
            source: tempfile::tempdir().unwrap(),
            target: tempfile::tempdir().unwrap(),
            state: tempfile::tempdir().unwrap(),
        }
    }

    fn orchestrator(&self) -> SyncOrchestrator<ClaudeAdapter, ClaudeAdapter> {
        SyncOrchestrator::new(
            ClaudeAdapter::with_root(self.source.path().to_path_buf()),
            ClaudeAdapter::with_root(self.target.path().to_path_buf()),
        )
        .with_kill_switch_marker(None)
        .with_state_root(Some(self.state.path().to_path_buf()))
    }

    fn sync(&self, params: &SyncParams) -> SyncReport {
        self.orchestrator().sync(params).unwrap()
    }

    fn baseline_files(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = fs::read_dir(self.state.path())
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    }

    fn baseline_text(&self) -> String {
        let files = self.baseline_files();
        assert_eq!(files.len(), 1, "{files:?}");
        fs::read_to_string(&files[0]).unwrap()
    }

    fn resolutions(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.state.path().join("resolutions.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name() != ".skrills-sync.lock")
        .map(|entry| {
            let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
            (relative, fs::read(entry.path()).unwrap())
        })
        .collect()
}

fn put(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).unwrap()
}

fn commands_only() -> SyncParams {
    SyncParams {
        sync_skills: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_hooks: false,
        sync_instructions: false,
        sync_plugin_assets: false,
        ..Default::default()
    }
}

fn interactive(params: SyncParams) -> SyncParams {
    SyncParams {
        interactive: true,
        ..params
    }
}

type Answer = Box<dyn Fn(&ItemChange) -> skrills_sync::Result<Decision> + Send + Sync>;

/// A decider that records each question and answers with `answer`.
struct Scripted {
    asked: Mutex<Vec<ItemChange>>,
    answer: Answer,
}

impl Scripted {
    fn answering(decision: Decision) -> Arc<Self> {
        Self::with(move |_| Ok(decision))
    }

    fn with(
        answer: impl Fn(&ItemChange) -> skrills_sync::Result<Decision> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            answer: Box::new(answer),
        })
    }

    fn asked(&self) -> Vec<ItemChange> {
        self.asked.lock().unwrap().clone()
    }
}

impl ItemDecider for Scripted {
    fn decide(&self, change: &ItemChange) -> skrills_sync::Result<Decision> {
        self.asked.lock().unwrap().push(change.clone());
        (self.answer)(change)
    }
}

fn warnings(report: &SyncReport) -> Vec<String> {
    report.commands.warnings.clone()
}

/// `foo` synced once at "v1", then edited on `source_edit` / `target_edit`.
fn synced_foo(source_edit: Option<&str>, target_edit: Option<&str>) -> Rig {
    let rig = Rig::new();
    put(rig.source.path(), "commands/foo.md", "# v1\n");
    let first = rig.sync(&commands_only());
    assert!(first.success, "{first:?}");
    if let Some(body) = source_edit {
        put(rig.source.path(), "commands/foo.md", body);
    }
    if let Some(body) = target_edit {
        put(rig.target.path(), "commands/foo.md", body);
    }
    rig
}

#[test]
fn a_first_sync_over_an_edited_target_is_not_a_conflict() {
    let rig = Rig::new();
    put(rig.source.path(), "commands/foo.md", "# source\n");
    put(rig.target.path(), "commands/foo.md", "# target edit\n");

    let report = rig.sync(&commands_only());

    assert!(warnings(&report).is_empty(), "{:?}", warnings(&report));
    assert_eq!(read(rig.target.path(), "commands/foo.md"), "# source\n");
}

#[test]
fn a_first_interactive_sync_asks_about_a_changed_item_without_conflict_information() {
    let rig = Rig::new();
    put(rig.source.path(), "commands/foo.md", "# source\n");
    put(rig.target.path(), "commands/foo.md", "# target edit\n");
    let decider = Scripted::answering(Decision::Accept);

    let _outcome = rig
        .orchestrator()
        .with_decider(decider.clone())
        .sync(&interactive(commands_only()))
        .unwrap();

    let asked = decider.asked();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].conflict.is_none());
}

#[test]
fn a_change_on_the_source_alone_since_the_last_sync_is_not_a_conflict() {
    let rig = synced_foo(Some("# v2\n"), None);

    let report = rig.sync(&commands_only());

    assert!(warnings(&report).is_empty(), "{:?}", warnings(&report));
    assert_eq!(read(rig.target.path(), "commands/foo.md"), "# v2\n");
}

#[test]
fn an_edit_on_the_target_alone_since_the_last_sync_is_not_a_conflict() {
    let rig = synced_foo(None, Some("# target edit\n"));

    let report = rig.sync(&commands_only());

    assert!(warnings(&report).is_empty(), "{:?}", warnings(&report));
    assert_eq!(read(rig.target.path(), "commands/foo.md"), "# v1\n");
}

#[test]
fn a_change_on_both_sides_warns_that_the_source_overwrote_the_target() {
    let rig = synced_foo(Some("# source v2\n"), Some("# target edit\n"));
    let snapshots = tempfile::tempdir().unwrap();

    let report = rig
        .orchestrator()
        .with_snapshot_root(Some(snapshots.path().to_path_buf()))
        .sync(&commands_only())
        .unwrap();

    let found = warnings(&report);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("foo"), "{}", found[0]);
    assert!(found[0].contains("both"), "{}", found[0]);
    assert!(found[0].contains("overwr"), "{}", found[0]);
    assert!(found[0].contains("skrills sync-rollback"), "{}", found[0]);
    assert_eq!(read(rig.target.path(), "commands/foo.md"), "# source v2\n");
    assert!(report.summary.contains("foo"), "{}", report.summary);
    assert!(rig.resolutions().is_empty());
}

#[test]
fn the_warning_does_not_promise_a_rollback_when_no_snapshot_is_taken() {
    let rig = synced_foo(Some("# source v2\n"), Some("# target edit\n"));

    let report = rig.sync(&commands_only());

    let found = warnings(&report);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(!found[0].contains("sync-rollback"), "{}", found[0]);
}

#[test]
fn both_sides_changing_to_the_same_content_is_not_a_conflict() {
    let rig = synced_foo(Some("# agreed\n"), Some("# agreed\n"));

    let report = rig.sync(&commands_only());

    assert!(warnings(&report).is_empty(), "{:?}", warnings(&report));
}

#[test]
fn a_dry_run_reports_the_conflict_but_writes_no_state_and_no_target_file() {
    let rig = synced_foo(Some("# source v2\n"), Some("# target edit\n"));
    let state_before = tree(rig.state.path());
    let target_before = tree(rig.target.path());

    let report = rig.sync(&SyncParams {
        dry_run: true,
        ..commands_only()
    });

    let found = warnings(&report);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("would overwrite"), "{}", found[0]);
    assert!(found[0].contains("foo"), "{}", found[0]);
    assert_eq!(tree(rig.state.path()), state_before);
    assert_eq!(tree(rig.target.path()), target_before);
}

#[test]
fn a_dry_run_with_no_prior_sync_creates_no_state() {
    let rig = Rig::new();
    put(rig.source.path(), "commands/foo.md", "# v1\n");

    let _report = rig.sync(&SyncParams {
        dry_run: true,
        ..commands_only()
    });

    assert!(tree(rig.state.path()).is_empty());
}

#[test]
fn a_successful_sync_records_hashes_and_no_content() {
    let rig = Rig::new();
    put(rig.source.path(), "commands/foo.md", "# TOPSECRET body\n");

    let report = rig.sync(&commands_only());

    assert!(report.success, "{report:?}");
    let baseline = rig.baseline_text();
    assert!(baseline.contains("command:foo"), "{baseline}");
    assert!(!baseline.contains("TOPSECRET"), "{baseline}");
}

#[test]
fn the_baseline_is_named_apart_for_each_target_root() {
    let rig = Rig::new();
    put(rig.source.path(), "commands/foo.md", "# v1\n");
    let other_target = tempfile::tempdir().unwrap();
    let first = rig.sync(&commands_only());
    let second = SyncOrchestrator::new(
        ClaudeAdapter::with_root(rig.source.path().to_path_buf()),
        ClaudeAdapter::with_root(other_target.path().to_path_buf()),
    )
    .with_kill_switch_marker(None)
    .with_state_root(Some(rig.state.path().to_path_buf()))
    .sync(&commands_only())
    .unwrap();

    assert!(first.success && second.success);
    assert_eq!(rig.baseline_files().len(), 2, "{:?}", rig.baseline_files());
}

#[test]
fn a_failed_sync_leaves_the_baseline_as_it_was() {
    let rig = synced_foo(Some("# v2\n"), None);
    let before = rig.baseline_text();
    // A file where the commands directory belongs makes the write fail.
    fs::remove_dir_all(rig.target.path().join("commands")).unwrap();
    put(rig.target.path(), "commands", "not a directory");

    let report = rig.sync(&commands_only());

    assert!(!report.success, "{report:?}");
    assert_eq!(rig.baseline_text(), before);
}

#[test]
fn a_run_with_a_failed_phase_still_advances_the_baseline_of_the_phases_that_wrote() {
    let rig = Rig::new();
    put(rig.source.path(), "commands/foo.md", "# v1\n");
    put(
        rig.source.path(),
        "agents/rev.md",
        "---\nname: rev\n---\nbody\n",
    );
    put(rig.target.path(), "agents", "not a directory");
    let params = SyncParams {
        sync_agents: true,
        ..commands_only()
    };

    let report = rig.sync(&params);

    assert!(!report.success, "{report:?}");
    // The commands were written; without a baseline the next sync would
    // read every later target edit to them as unrecorded.
    let baseline = rig.baseline_text();
    assert!(baseline.contains("command:foo"), "{baseline}");
    assert!(!baseline.contains("agent:rev"), "{baseline}");
}

#[test]
fn a_dry_run_does_not_advance_the_baseline() {
    let rig = synced_foo(Some("# v2\n"), None);
    let before = rig.baseline_text();

    let _report = rig.sync(&SyncParams {
        dry_run: true,
        ..commands_only()
    });

    assert_eq!(rig.baseline_text(), before);
}

#[test]
fn a_sync_without_a_state_root_neither_detects_nor_records() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    put(source.path(), "commands/foo.md", "# v1\n");
    let orchestrator = || {
        SyncOrchestrator::new(
            ClaudeAdapter::with_root(source.path().to_path_buf()),
            ClaudeAdapter::with_root(target.path().to_path_buf()),
        )
        .with_kill_switch_marker(None)
    };
    let _first = orchestrator().sync(&commands_only()).unwrap();
    put(source.path(), "commands/foo.md", "# v2\n");
    put(target.path(), "commands/foo.md", "# edit\n");

    let report = orchestrator().sync(&commands_only()).unwrap();

    assert!(warnings(&report).is_empty());
}

#[test]
fn an_interactive_conflict_carries_both_texts_target_and_source() {
    let rig = synced_foo(Some("# source v2\n"), Some("# target edit\n"));
    let decider = Scripted::answering(Decision::Accept);

    let _outcome = rig
        .orchestrator()
        .with_decider(decider.clone())
        .sync(&interactive(commands_only()))
        .unwrap();

    let asked = decider.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    let sides = asked[0].conflict.as_ref().expect("conflict information");
    assert_eq!(sides.target, "# target edit\n");
    assert_eq!(sides.source, "# source v2\n");
}

#[test]
fn keeping_the_source_overwrites_the_target_and_logs_the_choice() {
    let rig = synced_foo(Some("# source v2\n"), Some("# target edit\n"));
    let decider = Scripted::answering(Decision::Accept);

    let report = rig
        .orchestrator()
        .with_decider(decider)
        .sync(&interactive(commands_only()))
        .unwrap();

    assert!(report.success, "{report:?}");
    assert_eq!(read(rig.target.path(), "commands/foo.md"), "# source v2\n");
    let logged = rig.resolutions();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0]["choice"], "keep_source");
    assert_eq!(logged[0]["kind"], "command");
    assert_eq!(logged[0]["name"], "foo");
    assert_eq!(logged[0]["scope"], "item");
    assert!(logged[0]["source_hash"]
        .as_str()
        .is_some_and(|h| h.len() == 64));
    assert!(logged[0]["target_hash"]
        .as_str()
        .is_some_and(|h| h.len() == 64));
    assert!(logged[0]["timestamp"]
        .as_str()
        .is_some_and(|t| t.ends_with('Z')));
}

#[test]
fn keeping_the_target_leaves_every_target_byte_alone_and_deletes_nothing() {
    let rig = synced_foo(Some("# source v2\n"), Some("# target edit\n"));
    put(rig.target.path(), "commands/only-here.md", "# mine\n");
    let before = tree(rig.target.path());
    let decider = Scripted::answering(Decision::Reject);

    let report = rig
        .orchestrator()
        .with_decider(decider)
        .sync(&interactive(commands_only()))
        .unwrap();

    assert_eq!(tree(rig.target.path()), before);
    assert!(report.success, "{report:?}");
    let logged = rig.resolutions();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0]["choice"], "keep_target");
}

#[test]
fn a_resolved_conflict_is_not_a_conflict_on_the_next_sync() {
    let rig = synced_foo(Some("# source v2\n"), Some("# target edit\n"));
    let _outcome = rig
        .orchestrator()
        .with_decider(Scripted::answering(Decision::Reject))
        .sync(&interactive(commands_only()))
        .unwrap();

    let decider = Scripted::answering(Decision::Accept);
    let _again = rig
        .orchestrator()
        .with_decider(decider.clone())
        .sync(&interactive(commands_only()))
        .unwrap();

    let asked = decider.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].conflict.is_none(), "{asked:?}");
}

#[test]
fn aborting_at_a_conflict_writes_no_target_file_no_baseline_and_no_resolution() {
    let rig = synced_foo(Some("# source v2\n"), Some("# target edit\n"));
    let baseline = rig.baseline_text();
    let before = tree(rig.target.path());
    let decider = Scripted::with(|_| Err(SyncError::InteractiveAborted.into()));

    let error = rig
        .orchestrator()
        .with_decider(decider)
        .sync(&interactive(commands_only()))
        .unwrap_err();

    assert!(matches!(
        error.downcast_ref::<SyncError>(),
        Some(SyncError::InteractiveAborted)
    ));
    assert_eq!(tree(rig.target.path()), before);
    assert_eq!(rig.baseline_text(), baseline);
    assert!(rig.resolutions().is_empty());
}

#[test]
fn rejecting_a_change_that_is_not_a_conflict_leaves_its_baseline_alone() {
    let rig = synced_foo(Some("# source v2\n"), None);
    let baseline = rig.baseline_text();

    let _outcome = rig
        .orchestrator()
        .with_decider(Scripted::answering(Decision::Reject))
        .sync(&interactive(commands_only()))
        .unwrap();

    assert_eq!(rig.baseline_text(), baseline);
    assert!(rig.resolutions().is_empty());
}

#[test]
fn a_whole_category_prompt_says_how_many_of_its_items_conflict() {
    let rig = Rig::new();
    let skill = |body: &str| format!("---\nname: alpha\ndescription: A\n---\n\n{body}\n");
    let skills_only = || SyncParams {
        sync_skills: true,
        sync_commands: false,
        ..commands_only()
    };
    put(rig.source.path(), "skills/alpha/SKILL.md", &skill("v1"));
    let first = rig.sync(&skills_only());
    assert!(first.success, "{first:?}");
    put(
        rig.source.path(),
        "skills/alpha/SKILL.md",
        &skill("source v2"),
    );
    let held: Vec<PathBuf> = tree(rig.target.path())
        .into_keys()
        .filter(|path| path.ends_with("SKILL.md"))
        .collect();
    assert_eq!(held.len(), 1, "{held:?}");
    fs::write(rig.target.path().join(&held[0]), skill("target edit")).unwrap();
    let decider = Scripted::answering(Decision::Reject);

    let _outcome = rig
        .orchestrator()
        .with_decider(decider.clone())
        .sync(&interactive(skills_only()))
        .unwrap();

    let asked = decider.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].scope, DecisionScope::WholeCategory);
    assert_eq!(asked[0].conflicting_items, 1);
    assert!(asked[0].conflict.is_none());
    let logged = rig.resolutions();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0]["scope"], "category");
    assert_eq!(logged[0]["choice"], "keep_target");
    assert_eq!(logged[0]["kind"], "skill");
}

fn servers_only() -> SyncParams {
    SyncParams {
        sync_mcp_servers: true,
        sync_commands: false,
        ..commands_only()
    }
}

fn server_config(command: &str) -> String {
    format!(
        r#"{{"mcpServers": {{"srv": {{"command": "{command}", "env": {{"API_TOKEN": "s3cr3t-value"}}}}}}}}"#
    )
}

#[test]
fn an_mcp_server_conflict_shows_both_sides_without_secret_values() {
    let rig = Rig::new();
    put(rig.source.path(), ".claude.json", &server_config("/bin/v1"));
    let first = rig.sync(&servers_only());
    assert!(first.success, "{first:?}");
    put(
        rig.source.path(),
        ".claude.json",
        &server_config("/bin/source"),
    );
    put(
        rig.target.path(),
        ".claude.json",
        &server_config("/bin/target"),
    );
    let decider = Scripted::answering(Decision::Reject);

    let _outcome = rig
        .orchestrator()
        .with_decider(decider.clone())
        .sync(&interactive(servers_only()))
        .unwrap();

    let asked = decider.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].kind, ItemKind::McpServer);
    let sides = asked[0].conflict.as_ref().expect("conflict information");
    assert!(sides.target.contains("/bin/target"), "{}", sides.target);
    assert!(sides.source.contains("/bin/source"), "{}", sides.source);
    assert!(!sides.target.contains("s3cr3t-value"));
    assert!(!sides.source.contains("s3cr3t-value"));
    assert!(read(rig.target.path(), ".claude.json").contains("/bin/target"));
    let log = fs::read_to_string(rig.state.path().join("resolutions.jsonl")).unwrap();
    assert!(!log.contains("s3cr3t-value"), "{log}");
}

#[test]
fn an_mcp_server_conflict_warns_in_a_plain_sync() {
    let rig = Rig::new();
    put(rig.source.path(), ".claude.json", &server_config("/bin/v1"));
    let _first = rig.sync(&servers_only());
    put(
        rig.source.path(),
        ".claude.json",
        &server_config("/bin/source"),
    );
    put(
        rig.target.path(),
        ".claude.json",
        &server_config("/bin/target"),
    );

    let report = rig.sync(&servers_only());

    assert_eq!(report.mcp_servers.warnings.len(), 1, "{report:?}");
    assert!(report.mcp_servers.warnings[0].contains("srv"));
}

#[test]
fn a_source_only_change_on_a_target_that_rewrites_on_write_is_not_a_conflict() {
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let agent = |body: &str| format!("---\nname: rev\ndescription: R\n---\n{body}\n");
    put(source.path(), "agents/rev.md", &agent("v1"));
    let agents_only = SyncParams {
        sync_agents: true,
        sync_commands: false,
        ..commands_only()
    };
    let orchestrator = || {
        SyncOrchestrator::new(
            ClaudeAdapter::with_root(source.path().to_path_buf()),
            CopilotAdapter::with_root(target.path().to_path_buf()),
        )
        .with_kill_switch_marker(None)
        .with_state_root(Some(state.path().to_path_buf()))
    };
    let first = orchestrator().sync(&agents_only).unwrap();
    assert!(first.success, "{first:?}");
    put(source.path(), "agents/rev.md", &agent("v2"));

    let baseline: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            fs::read_dir(state.path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| path.extension().is_some_and(|ext| ext == "json"))
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let entry = &baseline["items"]["agent:rev"];
    assert_ne!(
        entry["source"], entry["target"],
        "the target must rewrite: {baseline}"
    );

    let second = orchestrator().sync(&agents_only).unwrap();

    assert!(
        second.agents.warnings.is_empty(),
        "{:?}",
        second.agents.warnings
    );
}
