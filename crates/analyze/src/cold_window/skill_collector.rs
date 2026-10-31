//! Skill participation in the cold-window tick.
//!
//! Parallel to [`super::plugin_health::PluginHealthCollector`]: walks
//! the configured skill directories each tick and produces a list of
//! `(source, token_estimate)` entries that the producer feeds into the
//! tick's `TokenLedger::per_skill` attribution.
//!
//! Token estimation uses [`crate::tokens::count_tokens`], the same
//! content-aware heuristic `skrills analyze` reports, so one SKILL.md
//! shows one token count in both places. Symlinked skill directories
//! and SKILL.md files are followed; a directory reached twice (symlink
//! cycle, overlapping roots) is walked once.
//!
//! Sources are `skill://<dir-name>`. When two skills share a directory
//! name (the same skill shipped by two plugins) both use their path
//! relative to the configured root instead, so the diff still sees two
//! sources.
//!
//! The collector is **side-effect free** and rewalks cold every tick,
//! mirroring the plugin collector's contract; the one thing it keeps is
//! each SKILL.md's token count keyed by path, mtime and length, so an
//! unchanged file is not re-read and re-tokenized every tick. Errors
//! reading individual entries are surfaced via `malformed`, not
//! silently dropped, same discipline applied elsewhere.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;

use skrills_snapshot::TokenEntry;

use crate::tokens::count_tokens;

/// One skill-collection failure surfaced for operator visibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalformedSkillEntry {
    /// Path or directory name that failed to read (best-effort).
    pub source: String,
    /// Human-readable error message.
    pub error_message: String,
}

/// Result of a single collector pass over the skill directories.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillCollectorOutput {
    /// One entry per discovered skill: `source` is `skill://<name>`,
    /// `tokens` is the [`crate::tokens::count_tokens`] total.
    pub entries: Vec<TokenEntry>,
    /// Per-entry I/O errors (unreadable file metadata, permission
    /// denied during walk, etc.).
    pub malformed: Vec<MalformedSkillEntry>,
}

/// Walks a list of `skill_dirs` and yields a [`SkillCollectorOutput`].
///
/// Recognized skill files: `SKILL.md` (canonical) and `skill.md`
/// (legacy). One token-entry per skill file. Subdirectories are
/// recursed up to `MAX_DEPTH` to match the behavior the discovery
/// scanner uses.
#[derive(Clone, Debug)]
pub struct SkillCollector {
    skill_dirs: Vec<PathBuf>,
    /// Token counts of files already read, by path.
    cache: Arc<Mutex<HashMap<PathBuf, CachedCount>>>,
}

/// A file's token count and the metadata it was computed for.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CachedCount {
    modified: Option<SystemTime>,
    len: u64,
    tokens: u64,
}

const MAX_DEPTH: usize = 6;

/// Largest SKILL.md read; a bigger file is reported, not tokenized.
const MAX_SKILL_BYTES: u64 = 1024 * 1024;

impl SkillCollector {
    /// Construct a collector that walks the supplied directories.
    #[must_use]
    pub fn new(skill_dirs: Vec<PathBuf>) -> Self {
        Self {
            skill_dirs,
            cache: Arc::default(),
        }
    }

    /// Returns true when no directories are configured (the producer
    /// can short-circuit and skip the blocking-pool dispatch entirely).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.skill_dirs.is_empty()
    }

    /// Walk all configured directories once. The cold rewalk is
    /// expected to be called from a `spawn_blocking` so the runtime
    /// worker threads stay free for IO-bound tasks.
    pub fn collect(&self) -> SkillCollectorOutput {
        let mut output = SkillCollectorOutput::default();
        let mut found = Vec::new();
        // Canonical directories already walked, shared across roots,
        // so a symlink cycle or two overlapping roots count once.
        let mut visited = HashSet::new();
        let mut cache = self.cache.lock();
        let mut seen = HashSet::new();
        for dir in &self.skill_dirs {
            let mut walker = Walker {
                root: dir,
                cache: &mut cache,
                seen: &mut seen,
                visited: &mut visited,
                found: &mut found,
                malformed: &mut output.malformed,
            };
            walker.walk(dir, 0);
        }
        // Forget files no longer found, so the cache tracks the tree.
        cache.retain(|path, _| seen.contains(path));
        drop(cache);
        output.entries = attribute(found);
        // Stable order so ledger comparisons are reproducible.
        output.entries.sort_by(|a, b| a.source.cmp(&b.source));
        output
    }
}

/// One SKILL.md found by the walk, before source ids are assigned.
struct FoundSkill {
    /// Name of the directory holding the SKILL.md.
    name: String,
    /// That directory relative to the configured root.
    rel_dir: PathBuf,
    /// That directory as walked (for the last-resort id).
    dir: PathBuf,
    tokens: u64,
}

/// Assign `skill://` source ids. A skill whose directory name is
/// unique keeps the short `skill://<name>`; same-named skills (the
/// same skill name shipped by two plugins) use the directory path
/// relative to their root, and the full path if even that collides,
/// so neither is lost when the diff collapses sources into a set.
fn attribute(found: Vec<FoundSkill>) -> Vec<TokenEntry> {
    let mut name_counts: HashMap<&str, usize> = HashMap::new();
    let mut rel_counts: HashMap<&Path, usize> = HashMap::new();
    for skill in &found {
        *name_counts.entry(skill.name.as_str()).or_default() += 1;
        *rel_counts.entry(skill.rel_dir.as_path()).or_default() += 1;
    }
    let source_for = |skill: &FoundSkill| {
        if name_counts[skill.name.as_str()] == 1 {
            format!("skill://{}", skill.name)
        } else if rel_counts[skill.rel_dir.as_path()] == 1 {
            format!("skill://{}", slash_path(&skill.rel_dir))
        } else {
            format!("skill://{}", slash_path(&skill.dir))
        }
    };
    found
        .iter()
        .map(|skill| TokenEntry {
            source: source_for(skill),
            tokens: skill.tokens,
        })
        .collect()
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

struct Walker<'a> {
    root: &'a Path,
    cache: &'a mut HashMap<PathBuf, CachedCount>,
    /// Skill files found this pass.
    seen: &'a mut HashSet<PathBuf>,
    visited: &'a mut HashSet<PathBuf>,
    found: &'a mut Vec<FoundSkill>,
    malformed: &'a mut Vec<MalformedSkillEntry>,
}

impl Walker<'_> {
    fn report(&mut self, source: impl Into<String>, error_message: String) {
        self.malformed.push(MalformedSkillEntry {
            source: source.into(),
            error_message,
        });
    }

    fn walk(&mut self, dir: &Path, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        // Skip a directory already walked under another name (symlink
        // cycle, or a link back into a root). If it cannot be
        // canonicalized, read_dir below reports the real error.
        if let Ok(canonical) = std::fs::canonicalize(dir) {
            if !self.visited.insert(canonical) {
                return;
            }
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
            Err(err) => {
                self.report(
                    dir.display().to_string(),
                    format!("skill dir unreadable: {err}"),
                );
                return;
            }
        };

        // Sorted, so which alias of a linked directory is walked first
        // (and so which name it is attributed to) does not depend on
        // readdir order, and `SKILL.md` comes before `skill.md`.
        let mut sorted = Vec::new();
        for entry in entries {
            match entry {
                Ok(e) => sorted.push(e),
                Err(err) => self.report(
                    format!("{}/<entry>", dir.display()),
                    format!("skill dir entry unreadable: {err}"),
                ),
            }
        }
        sorted.sort_by_key(|e| e.file_name());
        let mut recorded_skill = false;

        for entry in sorted {
            let path = entry.path();
            // `DirEntry::file_type` does not follow symlinks; a skill
            // installed as a link (`skills/foo -> ~/src/foo`) must be
            // resolved to what it points at.
            let file_type = match entry.file_type() {
                Ok(t) if t.is_symlink() => match std::fs::metadata(&path) {
                    Ok(m) => m.file_type(),
                    Err(err) => {
                        self.report(
                            path.display().to_string(),
                            format!("symlink target unreadable: {err}"),
                        );
                        continue;
                    }
                },
                Ok(t) => t,
                Err(err) => {
                    self.report(
                        path.display().to_string(),
                        format!("file_type read failed: {err}"),
                    );
                    continue;
                }
            };
            if file_type.is_dir() {
                self.walk(&path, depth + 1);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !name.eq_ignore_ascii_case("SKILL.md") && !name.eq_ignore_ascii_case("skill.md") {
                continue;
            }
            // One skill per directory, even where `SKILL.md` and
            // `skill.md` both exist.
            if recorded_skill {
                continue;
            }
            recorded_skill = true;
            self.record(&path);
        }
    }

    fn record(&mut self, path: &Path) {
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(err) => {
                self.report(path.display().to_string(), format!("stat failed: {err}"));
                return;
            }
        };
        if meta.len() > MAX_SKILL_BYTES {
            self.report(
                path.display().to_string(),
                format!(
                    "skill file is {} bytes, over the {MAX_SKILL_BYTES}-byte limit; not counted",
                    meta.len()
                ),
            );
            return;
        }
        self.seen.insert(path.to_path_buf());
        let modified = meta.modified().ok();
        let cached = self
            .cache
            .get(path)
            .filter(|c| c.modified.is_some() && c.modified == modified && c.len == meta.len())
            .map(|c| c.tokens);
        let tokens = match cached {
            Some(tokens) => tokens,
            None => {
                let bytes = match std::fs::read(path) {
                    Ok(b) => b,
                    Err(err) => {
                        self.report(path.display().to_string(), format!("read failed: {err}"));
                        return;
                    }
                };
                // Same estimator as `skrills analyze`, so one SKILL.md
                // shows one token count everywhere.
                let tokens = count_tokens(&String::from_utf8_lossy(&bytes)).total as u64;
                self.cache.insert(
                    path.to_path_buf(),
                    CachedCount {
                        modified,
                        len: meta.len(),
                        tokens,
                    },
                );
                tokens
            }
        };
        let dir = path.parent().unwrap_or(self.root).to_path_buf();
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unnamed>")
            .to_string();
        let rel_dir = match dir.strip_prefix(self.root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.to_path_buf(),
            _ => PathBuf::from(&name),
        };
        self.found.push(FoundSkill {
            name,
            rel_dir,
            dir,
            tokens,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, name: &str, body: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), body).unwrap();
    }

    /// M296: when two links name one skill directory, the walk keeps
    /// the name that sorts first, whatever order readdir returns.
    #[cfg(unix)]
    #[test]
    fn aliased_skill_dir_is_attributed_to_the_first_name_in_sort_order() {
        let tmp = TempDir::new().unwrap();
        let real = TempDir::new().unwrap();
        write_skill(real.path(), "target", "body");
        let target = real.path().join("target");
        for name in ["zeta", "mid", "alpha", "omega", "beta"] {
            std::os::unix::fs::symlink(&target, tmp.path().join(name)).unwrap();
        }
        let output = SkillCollector::new(vec![tmp.path().to_path_buf()]).collect();
        let sources: Vec<&str> = output.entries.iter().map(|e| e.source.as_str()).collect();
        assert_eq!(sources, ["skill://alpha"]);
    }

    /// M297: `SKILL.md` and `skill.md` in one directory are one skill.
    #[test]
    fn skill_md_in_both_cases_is_recorded_once() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "dup", "canonical");
        let dir = tmp.path().join("dup");
        fs::write(dir.join("skill.md"), "legacy body that is longer").unwrap();
        let output = SkillCollector::new(vec![tmp.path().to_path_buf()]).collect();
        assert_eq!(output.entries.len(), 1, "{:?}", output.entries);
        // Where the filesystem keeps both, SKILL.md is the one counted.
        if fs::read_dir(&dir).unwrap().count() == 2 {
            assert_eq!(
                output.entries[0].tokens,
                count_tokens("canonical").total as u64
            );
        }
    }

    /// M286: an unchanged SKILL.md (same mtime and length) is not
    /// re-read; a file over MAX_SKILL_BYTES is reported, not read.
    #[test]
    fn unchanged_skill_files_are_not_reread_and_huge_ones_are_skipped() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "memo", "aaaa aaaa aaaa");
        let file = tmp.path().join("memo/SKILL.md");
        let collector = SkillCollector::new(vec![tmp.path().to_path_buf()]);
        let first = collector.collect();

        let mtime = fs::metadata(&file).unwrap().modified().unwrap();
        fs::write(&file, "```\nb\n```\n\n\n").unwrap();
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        let second = collector.collect();
        assert_eq!(first.entries, second.entries);

        write_skill(
            tmp.path(),
            "huge",
            &"x".repeat(MAX_SKILL_BYTES as usize + 1),
        );
        let third = collector.collect();
        assert!(third.entries.iter().all(|e| e.source != "skill://huge"));
        assert!(
            third.malformed.iter().any(|m| m.source.contains("huge")),
            "{:?}",
            third.malformed
        );
    }

    #[test]
    fn empty_collector_returns_empty_output() {
        let collector = SkillCollector::new(vec![]);
        assert!(collector.is_empty());
        let output = collector.collect();
        assert!(output.entries.is_empty());
        assert!(output.malformed.is_empty());
    }

    #[test]
    fn collect_finds_skills_and_attributes_estimated_tokens() {
        let tmp = TempDir::new().unwrap();
        let body = "# A".repeat(1000); // 3000 bytes
        write_skill(tmp.path(), "alpha", &body);
        write_skill(tmp.path(), "beta", "tiny");

        let collector = SkillCollector::new(vec![tmp.path().to_path_buf()]);
        let output = collector.collect();
        assert_eq!(output.entries.len(), 2);
        assert!(output.malformed.is_empty());

        // Sorted lexicographically by `source`.
        assert_eq!(output.entries[0].source, "skill://alpha");
        assert_eq!(output.entries[1].source, "skill://beta");

        // Prose ratio is 4 chars/token, rounded up; the files end with
        // no newline: alpha is 3000 chars -> 750 tokens; beta is 4 -> 1.
        assert_eq!(output.entries[0].tokens, 750);
        assert_eq!(output.entries[1].tokens, 1);
    }

    #[test]
    fn collect_recurses_into_nested_skill_dirs() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join("nested").join("group");
        fs::create_dir_all(&nested).unwrap();
        write_skill(&nested, "deep", "body");

        let collector = SkillCollector::new(vec![tmp.path().to_path_buf()]);
        let output = collector.collect();
        assert_eq!(output.entries.len(), 1);
        assert_eq!(output.entries[0].source, "skill://deep");
    }

    #[test]
    fn collect_ignores_files_that_are_not_skill_md() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("alpha");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("README.md"), "readme").unwrap();
        fs::write(dir.join("config.toml"), "config").unwrap();

        let collector = SkillCollector::new(vec![tmp.path().to_path_buf()]);
        let output = collector.collect();
        assert!(output.entries.is_empty());
    }

    #[test]
    fn collect_silently_skips_missing_skill_root() {
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");

        let collector = SkillCollector::new(vec![nonexistent]);
        let output = collector.collect();
        // NotFound is the legitimate empty case (mirrors plugin_health
        // silent-drop contract); no malformed entry, no panic.
        assert!(output.entries.is_empty());
        assert!(output.malformed.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn collect_follows_symlinked_skill_dirs_and_files() {
        // IN-20: `~/.claude/skills/foo -> ~/src/foo` must be counted.
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        write_skill(&src, "linked-dir", "linked dir body");
        let file_target = src.join("file-target.md");
        fs::write(&file_target, "linked file body").unwrap();

        let root = tmp.path().join("skills");
        fs::create_dir_all(root.join("linked-file")).unwrap();
        std::os::unix::fs::symlink(src.join("linked-dir"), root.join("linked-dir")).unwrap();
        std::os::unix::fs::symlink(&file_target, root.join("linked-file").join("SKILL.md"))
            .unwrap();

        let output = SkillCollector::new(vec![root]).collect();
        let sources: Vec<_> = output.entries.iter().map(|e| e.source.as_str()).collect();
        assert_eq!(sources, ["skill://linked-dir", "skill://linked-file"]);
        assert!(output.malformed.is_empty(), "{:?}", output.malformed);
    }

    #[cfg(unix)]
    #[test]
    fn collect_does_not_loop_through_a_symlink_cycle() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("skills");
        write_skill(&root, "alpha", "body");
        std::os::unix::fs::symlink(&root, root.join("alpha").join("loop")).unwrap();

        let output = SkillCollector::new(vec![root]).collect();
        assert_eq!(output.entries.len(), 1, "{:?}", output.entries);
    }

    #[test]
    fn same_skill_name_under_two_plugins_gets_distinct_sources() {
        // IN-64: `pluginA/skills/review` and `pluginB/skills/review`
        // both became `skill://review` and collapsed in the diff.
        let tmp = TempDir::new().unwrap();
        write_skill(&tmp.path().join("pluginA").join("skills"), "review", "a");
        write_skill(&tmp.path().join("pluginB").join("skills"), "review", "b");
        write_skill(tmp.path(), "solo", "c");

        let output = SkillCollector::new(vec![tmp.path().to_path_buf()]).collect();
        let sources: Vec<_> = output.entries.iter().map(|e| e.source.as_str()).collect();
        assert_eq!(
            sources,
            [
                "skill://pluginA/skills/review",
                "skill://pluginB/skills/review",
                "skill://solo",
            ]
        );
    }

    #[test]
    fn ledger_tokens_match_the_analyze_estimator() {
        // IN-65: one estimator for `skrills analyze` and the ledger.
        let tmp = TempDir::new().unwrap();
        let body = "---\nname: alpha\n---\n# Alpha\n\n```sh\necho hi\n```\nProse here.\n";
        write_skill(tmp.path(), "alpha", body);
        let output = SkillCollector::new(vec![tmp.path().to_path_buf()]).collect();
        assert_eq!(
            output.entries[0].tokens,
            crate::tokens::count_tokens(body).total as u64
        );
    }

    #[test]
    fn collect_accepts_legacy_lowercase_skill_md() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("legacy");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("skill.md"), "legacy body").unwrap();

        let collector = SkillCollector::new(vec![tmp.path().to_path_buf()]);
        let output = collector.collect();
        assert_eq!(output.entries.len(), 1);
        assert_eq!(output.entries[0].source, "skill://legacy");
    }
}
