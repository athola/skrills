//! Interactive sync: decisions are collected per item (or per category when
//! the target rewrites on write) before any writer runs, and a rejection
//! leaves the target's bytes alone.

use anyhow::anyhow;
use skrills_sync::adapters::traits::AgentAdapter;
use skrills_sync::{
    ChangeKind, ClaudeAdapter, CodexAdapter, CopilotAdapter, CursorAdapter, Decision,
    DecisionScope, ItemChange, ItemKind, SkipReason, SyncError, SyncOrchestrator, SyncParams,
    SyncReport,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TARGETS: [&str; 4] = ["claude", "codex", "copilot", "cursor"];

/// Every file under `root` with its bytes, minus the sync lock.
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

/// A Claude source at version `tag`. Version 2 adds one new item per category.
fn seed_source(root: &Path, tag: &str, with_new: bool) {
    put(root, "commands/cmd-same.md", "# Same\n");
    put(root, "commands/cmd-a.md", &format!("# A {tag}\n"));
    put(
        root,
        "skills/alpha/SKILL.md",
        &format!("---\nname: alpha\ndescription: A\n---\n\nBody {tag}\n"),
    );
    put(root, "skills/alpha/notes.md", &format!("notes {tag}\n"));
    put(
        root,
        "agents/rev.md",
        &format!("---\nname: rev\ndescription: R\n---\nBody {tag}\n"),
    );
    put(root, "CLAUDE.md", &format!("# Rules {tag}\n"));
    let mut servers = format!(r#""srv": {{"command": "/bin/srv", "args": ["{tag}"]}}"#);
    if with_new {
        put(root, "commands/fresh.md", "# Fresh\n");
        put(
            root,
            "skills/fresh-skill/SKILL.md",
            "---\nname: fresh-skill\ndescription: F\n---\n\nFresh\n",
        );
        put(
            root,
            "agents/fresh-agent.md",
            "---\nname: fresh-agent\ndescription: F\n---\nFresh\n",
        );
        servers.push_str(r#", "fresh-srv": {"command": "/bin/fresh"}"#);
    }
    put(
        root,
        ".claude.json",
        &format!("{{\"mcpServers\": {{{servers}}}}}"),
    );
    let model = if tag == "v1" { "sonnet" } else { "opus" };
    put(
        root,
        "settings.json",
        &format!("{{\"model\": \"{model}\"}}"),
    );
}

fn target_adapter(name: &str, root: &Path) -> Box<dyn AgentAdapter> {
    match name {
        "claude" => Box::new(ClaudeAdapter::with_root(root.to_path_buf())),
        "codex" => Box::new(CodexAdapter::with_root(root.to_path_buf())),
        "copilot" => Box::new(CopilotAdapter::with_root(root.to_path_buf())),
        _ => Box::new(CursorAdapter::with_root(root.to_path_buf())),
    }
}

fn params() -> SyncParams {
    SyncParams {
        sync_hooks: false,
        sync_plugin_assets: false,
        ..Default::default()
    }
}

fn orchestrator(
    target: &str,
    source_root: &Path,
    target_root: &Path,
) -> SyncOrchestrator<ClaudeAdapter, Box<dyn AgentAdapter>> {
    SyncOrchestrator::new(
        ClaudeAdapter::with_root(source_root.to_path_buf()),
        target_adapter(target, target_root),
    )
    .with_kill_switch_marker(None)
}

/// A decider that records every question and answers with `answer`.
struct Recorder {
    asked: Mutex<Vec<ItemChange>>,
    answer: Box<dyn Fn(&ItemChange) -> Decision + Send + Sync>,
}

impl Recorder {
    fn new(answer: impl Fn(&ItemChange) -> Decision + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            answer: Box::new(answer),
        })
    }

    fn asked(&self) -> Vec<ItemChange> {
        self.asked.lock().unwrap().clone()
    }
}

impl skrills_sync::ItemDecider for Recorder {
    fn decide(&self, change: &ItemChange) -> skrills_sync::Result<Decision> {
        self.asked.lock().unwrap().push(change.clone());
        Ok((self.answer)(change))
    }
}

fn interactive() -> SyncParams {
    SyncParams {
        interactive: true,
        ..params()
    }
}

fn declined(report: &SyncReport) -> Vec<String> {
    [
        &report.commands,
        &report.skills,
        &report.agents,
        &report.instructions,
        &report.mcp_servers,
        &report.preferences,
    ]
    .iter()
    .flat_map(|phase| phase.skipped.iter())
    .filter_map(|reason| match reason {
        SkipReason::Declined { item } => Some(item.clone()),
        _ => None,
    })
    .collect()
}

fn written(report: &SyncReport) -> usize {
    report.commands.written
        + report.skills.written
        + report.agents.written
        + report.instructions.written
        + report.mcp_servers.written
        + report.preferences.written
}

/// A target synced at v1, its source advanced to v2 with a new item per
/// category. Returns (source, target, target tree after the v1 sync).
fn advanced_pair(
    target: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    BTreeMap<PathBuf, Vec<u8>>,
) {
    let source = tempfile::tempdir().unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    seed_source(source.path(), "v1", false);
    let first = orchestrator(target, source.path(), target_dir.path())
        .sync(&params())
        .unwrap();
    assert!(first.success, "{target}: {first:?}");
    let synced = tree(target_dir.path());
    seed_source(source.path(), "v2", true);
    (source, target_dir, synced)
}

#[test]
fn rejecting_everything_leaves_every_target_byte_identical_and_deletes_nothing() {
    for target in TARGETS {
        let (source, target_dir, synced) = advanced_pair(target);
        let decider = Recorder::new(|_| Decision::Reject);

        let report = orchestrator(target, source.path(), target_dir.path())
            .with_decider(decider.clone())
            .sync(&interactive())
            .unwrap();

        assert_eq!(tree(target_dir.path()), synced, "{target}: bytes changed");
        assert_eq!(written(&report), 0, "{target}: {report:?}");
        assert!(!declined(&report).is_empty(), "{target}");
        assert!(!decider.asked().is_empty(), "{target}");
    }
}

#[test]
fn accepting_everything_matches_a_default_sync_byte_for_byte() {
    for target in TARGETS {
        let (source, target_dir, _) = advanced_pair(target);
        let (plain_source, plain_target, _) = advanced_pair(target);
        let decider = Recorder::new(|_| Decision::Accept);

        let report = orchestrator(target, source.path(), target_dir.path())
            .with_decider(decider)
            .sync(&interactive())
            .unwrap();
        let _outcome = orchestrator(target, plain_source.path(), plain_target.path())
            .sync(&params())
            .unwrap();

        assert!(report.success, "{target}: {report:?}");
        assert!(declined(&report).is_empty(), "{target}");
        assert_eq!(
            tree(target_dir.path()),
            tree(plain_target.path()),
            "{target}"
        );
    }
}

#[test]
fn an_unchanged_item_is_not_asked_about() {
    let (source, target_dir, _) = advanced_pair("claude");
    let decider = Recorder::new(|_| Decision::Accept);

    let _outcome = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(decider.clone())
        .sync(&interactive())
        .unwrap();

    let asked: Vec<String> = decider.asked().iter().map(|c| c.name.clone()).collect();
    assert!(!asked.contains(&"cmd-same".to_string()), "{asked:?}");
    assert!(asked.contains(&"cmd-a".to_string()), "{asked:?}");
}

#[test]
fn a_per_item_target_is_asked_about_each_item_with_the_diff() {
    let (source, target_dir, _) = advanced_pair("claude");
    let decider = Recorder::new(|_| Decision::Accept);

    let _outcome = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(decider.clone())
        .sync(&interactive())
        .unwrap();

    let asked = decider.asked();
    let changed = asked
        .iter()
        .find(|c| c.kind == ItemKind::Command && c.name == "cmd-a")
        .expect("cmd-a was asked about");
    assert_eq!(changed.change, ChangeKind::Changed);
    assert_eq!(changed.scope, DecisionScope::PerItem);
    assert!(changed.diff.contains("-# A v1"), "{}", changed.diff);
    assert!(changed.diff.contains("+# A v2"), "{}", changed.diff);
    let fresh = asked
        .iter()
        .find(|c| c.kind == ItemKind::Command && c.name == "fresh")
        .expect("fresh was asked about");
    assert_eq!(fresh.change, ChangeKind::New);
    assert!(fresh.diff.contains("+# Fresh"), "{}", fresh.diff);
}

#[test]
fn a_skill_changed_only_in_a_module_is_still_asked_about_by_name() {
    let (source, target_dir, _) = advanced_pair("claude");
    put(
        source.path(),
        "skills/alpha/SKILL.md",
        "---\nname: alpha\ndescription: A\n---\n\nBody v1\n",
    );
    let decider = Recorder::new(|_| Decision::Accept);

    let _outcome = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(decider.clone())
        .sync(&interactive())
        .unwrap();

    let asked = decider.asked();
    let skills = asked
        .iter()
        .find(|c| c.kind == ItemKind::Skill)
        .expect("skills were asked about");
    assert_eq!(skills.scope, DecisionScope::WholeCategory);
    assert!(skills.diff.contains("alpha"), "{}", skills.diff);
}

#[test]
fn accepting_a_subset_writes_only_those_items() {
    let (source, target_dir, _) = advanced_pair("claude");
    let decider = Recorder::new(|change| {
        if change.name == "fresh" || change.name == "cmd-a" {
            Decision::Accept
        } else {
            Decision::Reject
        }
    });

    let report = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(decider)
        .sync(&interactive())
        .unwrap();

    let held = |relative: &str| fs::read_to_string(target_dir.path().join(relative)).ok();
    assert_eq!(held("commands/cmd-a.md").as_deref(), Some("# A v2\n"));
    assert_eq!(held("commands/fresh.md").as_deref(), Some("# Fresh\n"));
    assert!(held("skills/alpha/notes.md").unwrap().contains("notes v1"));
    assert!(held("agents/rev.md").unwrap().contains("Body v1"));
    assert!(held("agents/fresh-agent.md").is_none());
    assert!(held("skills/fresh-skill/SKILL.md").is_none());
    assert_eq!(report.commands.written, 2);
    assert!(declined(&report).contains(&"fresh-agent".to_string()));
}

#[test]
fn a_rejected_new_item_is_not_created() {
    let (source, target_dir, _) = advanced_pair("claude");
    let decider = Recorder::new(|change| match change.change {
        ChangeKind::New => Decision::Reject,
        _ => Decision::Accept,
    });

    let _outcome = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(decider)
        .sync(&interactive())
        .unwrap();

    assert!(!target_dir.path().join("commands/fresh.md").exists());
    assert!(!target_dir.path().join("agents/fresh-agent.md").exists());
    let servers = fs::read_to_string(target_dir.path().join(".claude.json")).unwrap();
    assert!(!servers.contains("fresh-srv"), "{servers}");
    assert!(servers.contains(r#""v2""#), "{servers}");
}

#[test]
fn rejecting_the_model_preference_keeps_the_targets_model() {
    let (source, target_dir, _) = advanced_pair("claude");
    let decider = Recorder::new(|change| match change.kind {
        ItemKind::Preferences => Decision::Reject,
        _ => Decision::Accept,
    });

    let report = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(decider.clone())
        .sync(&interactive())
        .unwrap();

    let settings = fs::read_to_string(target_dir.path().join("settings.json")).unwrap();
    assert!(settings.contains("sonnet"), "{settings}");
    assert!(declined(&report).contains(&"model".to_string()));
    let model = decider
        .asked()
        .into_iter()
        .find(|c| c.kind == ItemKind::Preferences)
        .expect("the model was asked about");
    assert!(model.diff.contains("-sonnet") && model.diff.contains("+opus"));
}

#[test]
fn a_category_the_target_rewrites_is_one_question_and_a_rejection_skips_its_writer() {
    let (source, target_dir, synced) = advanced_pair("cursor");
    let decider = Recorder::new(|change| match change.kind {
        ItemKind::Skill => Decision::Reject,
        _ => Decision::Accept,
    });

    let _outcome = orchestrator("cursor", source.path(), target_dir.path())
        .with_decider(decider.clone())
        .sync(&interactive())
        .unwrap();

    let skill_questions: Vec<ItemChange> = decider
        .asked()
        .into_iter()
        .filter(|c| c.kind == ItemKind::Skill)
        .collect();
    assert_eq!(skill_questions.len(), 1, "{skill_questions:?}");
    assert_eq!(skill_questions[0].scope, DecisionScope::WholeCategory);
    assert_eq!(skill_questions[0].name, "skills");
    let after = tree(target_dir.path());
    for (path, bytes) in synced.iter().filter(|(p, _)| p.starts_with("skills")) {
        assert_eq!(after.get(path), Some(bytes), "{}", path.display());
    }
    assert!(!target_dir.path().join("skills/fresh-skill").exists());
}

#[test]
fn a_default_sync_never_consults_the_decider() {
    let (source, target_dir, _) = advanced_pair("claude");
    let decider = Recorder::new(|_| panic!("a default sync must not ask"));

    let report = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(decider.clone())
        .sync(&params())
        .unwrap();

    assert!(decider.asked().is_empty());
    assert!(report.commands.written >= 2);
    assert_eq!(
        fs::read_to_string(target_dir.path().join("commands/cmd-a.md")).unwrap(),
        "# A v2\n"
    );
}

#[test]
fn a_dry_run_previews_and_never_asks_even_when_interactive() {
    let (source, target_dir, synced) = advanced_pair("claude");
    let decider = Recorder::new(|_| panic!("a dry run must not ask"));

    let report = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(decider.clone())
        .sync(&SyncParams {
            dry_run: true,
            ..interactive()
        })
        .unwrap();

    assert!(decider.asked().is_empty());
    assert!(report.commands.written >= 2, "{report:?}");
    assert_eq!(tree(target_dir.path()), synced);
}

#[test]
fn interactive_without_a_decider_is_refused_before_anything_is_written() {
    let (source, target_dir, synced) = advanced_pair("claude");

    let result = orchestrator("claude", source.path(), target_dir.path()).sync(&interactive());

    let error = result.unwrap_err();
    assert!(format!("{error:#}").contains("decider"), "{error:#}");
    assert_eq!(tree(target_dir.path()), synced);
}

#[test]
fn a_cancelled_prompt_stops_the_run_with_nothing_written() {
    let (source, target_dir, synced) = advanced_pair("claude");
    let asked = Arc::new(Mutex::new(0_usize));
    let counter = asked.clone();
    // Accepts the first question, cancels the second: the accepted write must
    // not have happened either, since all questions come before all writes.
    let decider = move |_: &ItemChange| -> skrills_sync::Result<Decision> {
        let mut count = counter.lock().unwrap();
        *count += 1;
        if *count == 1 {
            Ok(Decision::Accept)
        } else {
            Err(SyncError::InteractiveAborted.into())
        }
    };

    let error = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(Arc::new(decider))
        .sync(&interactive())
        .unwrap_err();

    assert!(
        matches!(
            error.downcast_ref::<SyncError>(),
            Some(SyncError::InteractiveAborted)
        ),
        "{error:#}"
    );
    assert_eq!(tree(target_dir.path()), synced);
}

#[test]
fn a_decider_failure_stops_the_run_with_nothing_written() {
    let (source, target_dir, synced) = advanced_pair("claude");
    let decider =
        |_: &ItemChange| -> skrills_sync::Result<Decision> { Err(anyhow!("stdin closed")) };

    let error = orchestrator("claude", source.path(), target_dir.path())
        .with_decider(Arc::new(decider))
        .sync(&interactive())
        .unwrap_err();

    assert!(format!("{error:#}").contains("stdin closed"), "{error:#}");
    assert_eq!(tree(target_dir.path()), synced);
}
