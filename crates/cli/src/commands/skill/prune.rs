use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use skrills_discovery::{discover_skills, SkillMeta, SkillSource};
use skrills_intelligence::{load_or_build_analytics, UsageAnalytics};
use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::cli::OutputFormat;

use super::profiling::{now_secs, rfc3339};
use super::skill_dir_name;

const SECONDS_PER_DAY: u64 = 86_400;

/// A skill whose recorded use is older than the threshold, or absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Candidate {
    name: String,
    /// The skill directory, which is what `--apply` removes.
    path: PathBuf,
    /// The skill root `path` was discovered under; deletion re-checks against it.
    #[serde(skip)]
    root: PathBuf,
    /// RFC 3339 time of the last mined use; `None` means no session in the
    /// mined history used the skill, which is not the same as "long ago".
    last_used: Option<String>,
}

/// A skill that is stale by usage but must not be deleted, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Skipped {
    name: String,
    path: PathBuf,
    reason: &'static str,
}

#[derive(Debug, Default, Serialize)]
struct PrunePlan {
    candidates: Vec<Candidate>,
    skipped: Vec<Skipped>,
}

#[derive(Serialize)]
struct SuggestReport<'a> {
    older_than_days: u32,
    /// RFC 3339 time of the oldest mined event, so a reader can judge whether
    /// "never used" covers the whole threshold window.
    history_start: Option<String>,
    candidates: &'a [Candidate],
    skipped: &'a [Skipped],
}

/// `--suggest` (`apply == false`) only reports. `--apply` deletes the same
/// candidates through a snapshot session so `sync-rollback` can restore them,
/// and refuses when session history is too short to call a skill unused
/// unless `ignore_short_history`.
pub(crate) fn handle_prune_command(
    apply: bool,
    older_than_days: u32,
    yes: bool,
    ignore_short_history: bool,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    // Checked before any work: a script that forgot `--yes` should fail
    // immediately, not after mining every session log.
    if apply && !yes && !is_interactive() {
        bail!("--apply asks for confirmation and there is no terminal to ask on; pass --yes to skip the prompt");
    }

    let roots = crate::commands::skill_roots_for(&skill_dirs)?;
    let skills = discover_skills(&roots, None)?;
    let analytics = load_usage()?;
    let now = now_secs();
    let plan = plan(&skills, &analytics, now, older_than_days);
    let history_start = history_start(&analytics);

    if !apply {
        return print_suggestions(&plan, history_start, now, older_than_days, format);
    }
    let caveat = short_history_caveat(history_start, now, older_than_days);
    if let Some(caveat) = &caveat {
        if !plan.candidates.is_empty() && !ignore_short_history {
            bail!(
                "{caveat} Nothing deleted; rerun with --ignore-short-history to delete anyway, \
                 or lower --older-than-days"
            );
        }
    }
    // A snapshot is the only way back, so failing to take one stops the run
    // (sync only warns, because a sync can be repeated; a deletion cannot).
    let snapshot_root = skrills_sync::snapshot::default_root().ok_or_else(|| {
        anyhow!("no home directory, so no snapshot can be taken; nothing deleted")
    })?;
    apply_plan(&plan, yes, caveat.as_deref(), &snapshot_root)
}

/// The oldest mined event, when one has a known time.
fn history_start(analytics: &UsageAnalytics) -> Option<u64> {
    analytics
        .time_range
        .as_ref()
        .map(|range| range.start)
        .filter(|&start| start > 0)
}

/// The warning that mined history starts after the cutoff, or `None` when it
/// covers the whole window (or its start is unknown).
fn short_history_caveat(history_start: Option<u64>, now: u64, days: u32) -> Option<String> {
    let cutoff = now.saturating_sub(u64::from(days) * SECONDS_PER_DAY);
    let start = history_start.filter(|&start| start > cutoff)?;
    Some(format!(
        "Mined session history only starts {}, which is less than {days} days ago; \
         \"never used\" may just mean the skill predates it.",
        rfc3339(start).map_or_else(|| start.to_string(), |t| t[..10].to_string())
    ))
}

fn is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Usage mined from session history. Without any, every skill would look
/// unused, so this fails rather than return an empty analysis.
///
/// The cache is not written: `--suggest` must leave the machine unchanged.
fn load_usage() -> Result<UsageAnalytics> {
    let analytics = load_or_build_analytics(false, false)
        .context("Failed to read session history for skill usage")?;
    if analytics.recency.is_empty() {
        bail!(
            "no skill usage found in session history (~/.claude/projects, ~/.codex/sessions); \
             without it every skill would look stale, so nothing is suggested"
        );
    }
    Ok(analytics)
}

/// Splits the discovered skills into prune candidates and skills that are
/// stale but protected.
///
/// A skill is stale when its latest recorded use is more than
/// `older_than_days` before `now` (exactly that many days is still fresh), or
/// when none is recorded. Usage is keyed by directory name (analytics reduce
/// every path, URI and `plugin:name` to it), so a skill matches under its
/// directory name or its frontmatter name; the latest use under either counts.
/// A use whose time could not be parsed (recorded as 0) under either name
/// proves the skill is used, so it is never stale.
fn plan(
    skills: &[SkillMeta],
    analytics: &UsageAnalytics,
    now: u64,
    older_than_days: u32,
) -> PrunePlan {
    let last_use: HashMap<String, u64> = analytics
        .recency
        .iter()
        .map(|(key, &time)| (key.to_ascii_lowercase(), time))
        .collect();
    let cutoff = now.saturating_sub(u64::from(older_than_days) * SECONDS_PER_DAY);
    let all_dirs: Vec<&Path> = skills.iter().filter_map(|s| s.path.parent()).collect();

    let mut plan = PrunePlan::default();
    for skill in skills {
        let uses: Vec<u64> = [Some(skill_dir_name(skill)), skill.frontmatter_name.clone()]
            .into_iter()
            .flatten()
            .filter_map(|name| last_use.get(&name.to_ascii_lowercase()).copied())
            .collect();
        let latest = uses.iter().copied().max();
        let stale = match latest {
            None => true,
            Some(time) => !uses.contains(&0) && time < cutoff,
        };
        if !stale {
            continue;
        }
        let path = skill
            .path
            .parent()
            .map_or_else(|| skill.path.clone(), Path::to_path_buf);
        let name = skill_dir_name(skill);
        match refusal(skill, &all_dirs) {
            Some(reason) => plan.skipped.push(Skipped { name, path, reason }),
            None => plan.candidates.push(Candidate {
                name,
                path,
                root: skill.root.clone(),
                last_used: latest.and_then(rfc3339),
            }),
        }
    }
    plan.candidates.sort_by(|a, b| a.path.cmp(&b.path));
    plan.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    plan
}

const PLUGIN_REASON: &str = "managed by a plugin; the next plugin update would restore it";
const MIRROR_REASON: &str = "a sync mirror; the next sync would recreate it";

/// The refusal for a path inside a plugin cache, a plugin marketplace or a
/// skills mirror, judged by its components.
fn managed_path_refusal(path: &Path) -> Option<&'static str> {
    let names: Vec<&str> = path
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let is = |name: &str, wanted: &str| name.eq_ignore_ascii_case(wanted);
    if names
        .windows(2)
        .any(|w| is(w[0], "plugins") && (is(w[1], "cache") || is(w[1], "marketplaces")))
    {
        return Some(PLUGIN_REASON);
    }
    names
        .iter()
        .any(|name| is(name, "skills-mirror"))
        .then_some(MIRROR_REASON)
}

/// Why `skill`'s directory must not be deleted, or `None` when it may be.
///
/// Plugin and mirror skills are recognized by the root's label and by the
/// skill's own path, as given and resolved: `--skill-dir ~/.claude/plugins`
/// labels its root as a plain Claude root, and `--skill-dir .` or a symlinked
/// root hides the path, yet the plugin cache sits below them.
fn refusal(skill: &SkillMeta, all_dirs: &[&Path]) -> Option<&'static str> {
    if matches!(skill.source, SkillSource::Cache | SkillSource::Marketplace) {
        return Some(PLUGIN_REASON);
    }
    if matches!(skill.source, SkillSource::Mirror) {
        return Some(MIRROR_REASON);
    }
    if let Some(reason) = managed_path_refusal(&skill.path) {
        return Some(reason);
    }
    let dir = skill.path.parent()?;
    if let Some(reason) = dir
        .canonicalize()
        .ok()
        .and_then(|real| managed_path_refusal(&real))
    {
        return Some(reason);
    }
    if skill.path.file_name().is_none_or(|f| f != "SKILL.md") {
        return Some("not a SKILL.md in its own directory under a skill root");
    }
    if let Some(reason) = dir_refusal(dir, &skill.root) {
        return Some(reason);
    }
    if all_dirs
        .iter()
        .any(|other| *other != dir && other.starts_with(dir))
    {
        return Some("contains another skill, which deleting it would also remove");
    }
    None
}

/// Why the directory `dir` must not be removed as a skill under `root`.
///
/// Run again just before each deletion, so a directory swapped for a symlink
/// between the report and the delete is still refused.
fn dir_refusal(dir: &Path, root: &Path) -> Option<&'static str> {
    if dir == root {
        return Some("not a SKILL.md in its own directory under a skill root");
    }
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return Some("directory cannot be inspected");
    };
    if meta.file_type().is_symlink() {
        return Some("symlinked directory; the link target is not skrills' to delete");
    }
    let inside_root = match (dir.canonicalize(), root.canonicalize()) {
        (Ok(real_dir), Ok(real_root)) => real_dir.starts_with(real_root),
        _ => false,
    };
    (!inside_root).then_some("resolves outside its skill root through a symlink")
}

fn print_suggestions(
    plan: &PrunePlan,
    history_start: Option<u64>,
    now: u64,
    older_than_days: u32,
    format: OutputFormat,
) -> Result<()> {
    if format.is_json() {
        let report = SuggestReport {
            older_than_days,
            history_start: history_start.and_then(rfc3339),
            candidates: &plan.candidates,
            skipped: &plan.skipped,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    print!("{}", render_text(plan, history_start, now, older_than_days));
    Ok(())
}

fn render_text(plan: &PrunePlan, history_start: Option<u64>, now: u64, days: u32) -> String {
    let mut text = String::new();
    if plan.candidates.is_empty() {
        text.push_str(&format!("No skills unused for more than {days} days.\n"));
    } else {
        text.push_str(&format!(
            "{} skill(s) unused for more than {days} days:\n",
            plan.candidates.len()
        ));
        text.push_str(&render_candidates(&plan.candidates));
    }
    if !plan.skipped.is_empty() {
        text.push_str("\nStale but protected, so never deleted:\n");
        for skipped in &plan.skipped {
            text.push_str(&format!(
                "  {:30} {} ({})\n",
                skipped.name,
                skipped.path.display(),
                skipped.reason
            ));
        }
    }
    if let Some(caveat) = short_history_caveat(history_start, now, days) {
        text.push_str(&format!("\nNote: {caveat}\n"));
    }
    text
}

fn render_candidates(candidates: &[Candidate]) -> String {
    candidates
        .iter()
        .map(|candidate| {
            let used = match &candidate.last_used {
                Some(time) => format!("last used {}", &time[..10.min(time.len())]),
                None => "never used in mined sessions".to_string(),
            };
            format!(
                "  {:30} {:34} {}\n",
                candidate.name,
                used,
                candidate.path.display()
            )
        })
        .collect()
}

/// Deletes `plan.candidates` inside a snapshot session under
/// `snapshot_root`, after confirmation unless `yes`, and names the snapshot.
/// No candidates means no snapshot and no prompt. `caveat`, when the history
/// is short, is shown before the prompt.
fn apply_plan(
    plan: &PrunePlan,
    yes: bool,
    caveat: Option<&str>,
    snapshot_root: &Path,
) -> Result<()> {
    if plan.candidates.is_empty() {
        println!("Nothing to prune.");
        return Ok(());
    }
    print!("{}", render_candidates(&plan.candidates));
    if let Some(caveat) = caveat {
        eprintln!("Warning: {caveat}");
    }
    if !yes {
        let confirmed = inquire::Confirm::new(&format!(
            "Delete these {} skill(s)? A snapshot is taken first.",
            plan.candidates.len()
        ))
        .with_default(false)
        .prompt()?;
        if !confirmed {
            println!("Cancelled; nothing deleted.");
            return Ok(());
        }
    }

    let earlier = snapshot_names(snapshot_root);
    let _session = skrills_sync::snapshot::begin(snapshot_root)
        .context("Could not take a snapshot; nothing deleted")?;
    let snapshot = snapshot_names(snapshot_root)
        .into_iter()
        .find(|name| !earlier.contains(name))
        .map_or_else(String::new, |name| format!(" into snapshot {name}"));

    let mut removed = 0_usize;
    for candidate in &plan.candidates {
        if let Some(reason) = dir_refusal(&candidate.path, &candidate.root) {
            bail!(
                "{} is no longer safe to delete ({reason}); removed {removed} of {}. \
                 Undo with `skrills sync-rollback`",
                candidate.path.display(),
                plan.candidates.len()
            );
        }
        skrills_sync::snapshot::remove_recorded_tree(&candidate.path).with_context(|| {
            format!(
                "Failed to remove {}; removed {removed} of {}. Undo with `skrills sync-rollback`",
                candidate.path.display(),
                plan.candidates.len()
            )
        })?;
        removed += 1;
    }
    println!("Removed {removed} skill(s){snapshot}. Undo with `skrills sync-rollback`.");
    Ok(())
}

/// Names of the snapshots under `root`; an unreadable root lists none, which
/// only costs the name in the closing line.
fn snapshot_names(root: &Path) -> Vec<String> {
    skrills_sync::snapshot::list(root)
        .map(|found| found.into_iter().map(|snapshot| snapshot.name).collect())
        .unwrap_or_else(|e| {
            tracing::warn!(root = %root.display(), error = %e, "Could not list snapshots");
            Vec::new()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2027-01-15 UTC.
    const NOW: u64 = 1_800_000_000;

    fn skill_in(root: &Path, dir_name: &str, source: SkillSource, fm: Option<&str>) -> SkillMeta {
        let dir = root.join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), "# skill\n").unwrap();
        SkillMeta {
            name: format!("{dir_name}/SKILL.md"),
            path: dir.join("SKILL.md"),
            source,
            root: root.to_path_buf(),
            hash: String::new(),
            description: None,
            frontmatter_name: fm.map(str::to_string),
        }
    }

    fn used(entries: &[(&str, u64)]) -> UsageAnalytics {
        let mut analytics = UsageAnalytics::default();
        for (name, time) in entries {
            analytics.recency.insert((*name).to_string(), *time);
        }
        analytics
    }

    fn names(plan: &PrunePlan) -> Vec<&str> {
        plan.candidates.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn a_skill_used_exactly_the_threshold_ago_is_fresh_and_one_second_older_is_stale() {
        let root = tempfile::tempdir().unwrap();
        let skills = [
            skill_in(root.path(), "at-limit", SkillSource::Extra(0), None),
            skill_in(root.path(), "past-limit", SkillSource::Extra(0), None),
        ];
        let analytics = used(&[
            ("at-limit", NOW - 90 * SECONDS_PER_DAY),
            ("past-limit", NOW - 90 * SECONDS_PER_DAY - 1),
        ]);

        let plan = plan(&skills, &analytics, NOW, 90);

        assert_eq!(names(&plan), ["past-limit"]);
    }

    #[test]
    fn a_skill_with_no_recorded_use_is_a_candidate_without_a_date() {
        let root = tempfile::tempdir().unwrap();
        let skills = [skill_in(root.path(), "unused", SkillSource::Extra(0), None)];

        let plan = plan(&skills, &used(&[("other", NOW)]), NOW, 90);

        assert_eq!(names(&plan), ["unused"]);
        assert_eq!(plan.candidates[0].last_used, None);
        assert!(render_candidates(&plan.candidates).contains("never used in mined sessions"));
    }

    #[test]
    fn a_stale_candidate_reports_the_date_of_its_last_use() {
        let root = tempfile::tempdir().unwrap();
        let skills = [skill_in(root.path(), "old", SkillSource::Extra(0), None)];
        // 2023-11-14T22:13:20Z
        let analytics = used(&[("old", 1_700_000_000)]);

        let plan = plan(&skills, &analytics, NOW, 90);

        assert!(plan.candidates[0]
            .last_used
            .as_deref()
            .unwrap()
            .starts_with("2023-11-14"));
        assert!(render_candidates(&plan.candidates).contains("last used 2023-11-14"));
    }

    #[test]
    fn a_use_with_an_unknown_time_keeps_the_skill_out_of_the_candidates() {
        let root = tempfile::tempdir().unwrap();
        let skills = [skill_in(
            root.path(),
            "undated",
            SkillSource::Extra(0),
            None,
        )];

        let plan = plan(&skills, &used(&[("undated", 0)]), NOW, 90);

        assert!(plan.candidates.is_empty());
    }

    #[test]
    fn an_undated_use_under_either_name_keeps_the_skill_out_of_the_candidates() {
        let root = tempfile::tempdir().unwrap();
        let skills = [skill_in(
            root.path(),
            "dir-name",
            SkillSource::Extra(0),
            Some("front-name"),
        )];
        let analytics = used(&[("dir-name", 1_700_000_000), ("front-name", 0)]);

        let plan = plan(&skills, &analytics, NOW, 90);

        assert!(plan.candidates.is_empty(), "{plan:?}");
    }

    #[test]
    fn a_skill_under_a_plugin_cache_or_mirror_path_is_refused_whatever_its_root_label() {
        let root = tempfile::tempdir().unwrap();
        let skills = [
            skill_in(
                &root.path().join("plugins/cache/v/p/skills"),
                "cached",
                SkillSource::Claude,
                None,
            ),
            skill_in(
                &root.path().join("plugins/marketplaces/v/skills"),
                "market",
                SkillSource::Extra(0),
                None,
            ),
            skill_in(
                &root.path().join("skills-mirror"),
                "mirrored",
                SkillSource::Codex,
                None,
            ),
        ];
        let skills = skills.map(|skill| SkillMeta {
            root: root.path().to_path_buf(),
            ..skill
        });

        let plan = plan(&skills, &used(&[("other", NOW)]), NOW, 90);

        assert!(plan.candidates.is_empty(), "{plan:?}");
        assert_eq!(plan.skipped.len(), 3, "{plan:?}");
    }

    /// A root that is a symlink to a plugin cache (or `--skill-dir .` run
    /// from inside one) hides `plugins/cache` from the path as given.
    #[cfg(unix)]
    #[test]
    fn a_plugin_cache_skill_reached_through_a_symlinked_root_is_refused() {
        let real = tempfile::tempdir().unwrap();
        let linked = tempfile::tempdir().unwrap();
        let cache_skills = real.path().join("plugins/cache/acme/p/skills");
        skill_in(&cache_skills, "cached", SkillSource::Extra(0), None);
        let root = linked.path().join("root");
        std::os::unix::fs::symlink(&cache_skills, &root).unwrap();
        let skill = SkillMeta {
            name: "cached/SKILL.md".into(),
            path: root.join("cached/SKILL.md"),
            source: SkillSource::Extra(0),
            root: root.clone(),
            hash: String::new(),
            description: None,
            frontmatter_name: None,
        };

        let plan = plan(&[skill], &used(&[("other", NOW)]), NOW, 90);

        assert!(plan.candidates.is_empty(), "{plan:?}");
        assert_eq!(plan.skipped[0].reason, PLUGIN_REASON);
    }

    /// The second candidate vanished between the plan and the delete: the
    /// error says how far the run got, and the first deletion is in the
    /// snapshot.
    #[test]
    fn a_candidate_refused_at_delete_time_reports_how_many_were_removed() {
        let root = tempfile::tempdir().unwrap();
        let snapshots = tempfile::tempdir().unwrap();
        let first = skill_in(root.path(), "a-first", SkillSource::Extra(0), None);
        let candidate = |name: &str| Candidate {
            name: name.to_string(),
            path: root.path().join(name),
            root: root.path().to_path_buf(),
            last_used: None,
        };
        let plan = PrunePlan {
            candidates: vec![candidate("a-first"), candidate("b-gone")],
            skipped: Vec::new(),
        };

        let err = apply_plan(&plan, true, None, snapshots.path())
            .unwrap_err()
            .to_string();

        assert!(err.contains("removed 1 of 2"), "{err}");
        assert!(err.contains("b-gone"), "{err}");
        assert!(!first.path.exists());
        let taken = skrills_sync::snapshot::list(snapshots.path()).unwrap();
        assert_eq!(taken.len(), 1);
        assert!(taken[0].entries > 0);
    }

    #[test]
    fn use_recorded_under_the_frontmatter_name_or_another_case_counts() {
        let root = tempfile::tempdir().unwrap();
        let skills = [
            skill_in(
                root.path(),
                "dir-a",
                SkillSource::Extra(0),
                Some("Friendly-Name"),
            ),
            skill_in(root.path(), "Dir-B", SkillSource::Extra(0), None),
        ];
        let analytics = used(&[("friendly-name", NOW), ("dir-b", NOW)]);

        let plan = plan(&skills, &analytics, NOW, 90);

        assert!(plan.candidates.is_empty(), "{plan:?}");
    }

    #[test]
    fn plugin_cache_marketplace_and_mirror_skills_are_skipped_with_a_reason() {
        let root = tempfile::tempdir().unwrap();
        let skills = [
            skill_in(root.path(), "cached", SkillSource::Cache, None),
            skill_in(root.path(), "market", SkillSource::Marketplace, None),
            skill_in(root.path(), "mirrored", SkillSource::Mirror, None),
        ];

        let plan = plan(&skills, &used(&[("other", NOW)]), NOW, 90);

        assert!(plan.candidates.is_empty());
        assert_eq!(plan.skipped.len(), 3);
        assert!(plan.skipped.iter().any(|s| s.reason.contains("plugin")));
        assert!(plan.skipped.iter().any(|s| s.reason.contains("mirror")));
        assert!(render_text(&plan, None, NOW, 90).contains("Stale but protected"));
    }

    #[test]
    fn a_skill_directory_that_is_the_root_itself_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("SKILL.md"), "# skill\n").unwrap();
        let skills = [SkillMeta {
            name: "SKILL.md".into(),
            path: root.path().join("SKILL.md"),
            source: SkillSource::Extra(0),
            root: root.path().to_path_buf(),
            hash: String::new(),
            description: None,
            frontmatter_name: None,
        }];

        let plan = plan(&skills, &used(&[("other", NOW)]), NOW, 90);

        assert!(plan.candidates.is_empty());
        assert_eq!(plan.skipped.len(), 1);
    }

    #[test]
    fn a_file_that_is_not_skill_md_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        let mut skill = skill_in(root.path(), "flat", SkillSource::Extra(0), None);
        std::fs::write(root.path().join("flat/notes.md"), "x").unwrap();
        skill.path = root.path().join("flat/notes.md");

        let plan = plan(&[skill], &used(&[("other", NOW)]), NOW, 90);

        assert!(plan.candidates.is_empty());
        assert_eq!(plan.skipped.len(), 1);
    }

    #[test]
    fn a_skill_that_contains_another_skill_is_skipped_so_the_inner_one_survives() {
        let root = tempfile::tempdir().unwrap();
        let outer = skill_in(root.path(), "outer", SkillSource::Extra(0), None);
        let inner = skill_in(
            &root.path().join("outer"),
            "inner",
            SkillSource::Extra(0),
            None,
        );
        let analytics = used(&[("inner", NOW)]);

        let plan = plan(&[outer, inner], &analytics, NOW, 90);

        assert!(plan.candidates.is_empty(), "{plan:?}");
        assert!(plan.skipped[0].reason.contains("contains another skill"));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_skill_directory_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = skill_in(elsewhere.path(), "linked", SkillSource::Extra(0), None);
        std::os::unix::fs::symlink(target.path.parent().unwrap(), root.path().join("linked"))
            .unwrap();
        let linked = SkillMeta {
            path: root.path().join("linked/SKILL.md"),
            root: root.path().to_path_buf(),
            ..target
        };

        let plan = plan(&[linked], &used(&[("other", NOW)]), NOW, 90);

        assert!(plan.candidates.is_empty());
        assert!(plan.skipped[0].reason.contains("symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn a_real_directory_reached_through_a_symlinked_parent_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        skill_in(
            &elsewhere.path().join("group"),
            "inner",
            SkillSource::Extra(0),
            None,
        );
        std::os::unix::fs::symlink(elsewhere.path().join("group"), root.path().join("group"))
            .unwrap();

        let reason = dir_refusal(&root.path().join("group/inner"), root.path());

        assert!(reason.unwrap().contains("outside its skill root"));
    }

    #[test]
    fn a_missing_directory_is_refused_at_delete_time() {
        let root = tempfile::tempdir().unwrap();

        let reason = dir_refusal(&root.path().join("gone"), root.path());

        assert!(reason.unwrap().contains("cannot be inspected"));
    }

    #[test]
    fn history_shorter_than_the_threshold_adds_a_caveat() {
        let plan = PrunePlan::default();

        let short = render_text(&plan, Some(NOW - 10 * SECONDS_PER_DAY), NOW, 90);
        let long = render_text(&plan, Some(NOW - 400 * SECONDS_PER_DAY), NOW, 90);

        assert!(short.contains("only starts"), "{short}");
        assert!(!long.contains("only starts"), "{long}");
        assert!(long.contains("No skills unused for more than 90 days"));
    }
}
