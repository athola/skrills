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
//! The collector is **deliberately stateless and side-effect free**,
//! mirroring the plugin collector's "cold rewalk" contract. Errors
//! reading individual entries are surfaced via `malformed`, not
//! silently dropped, same discipline applied elsewhere.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
}

const MAX_DEPTH: usize = 6;

impl SkillCollector {
    /// Construct a collector that walks the supplied directories.
    #[must_use]
    pub fn new(skill_dirs: Vec<PathBuf>) -> Self {
        Self { skill_dirs }
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
        for dir in &self.skill_dirs {
            let mut walker = Walker {
                root: dir,
                visited: &mut visited,
                found: &mut found,
                malformed: &mut output.malformed,
            };
            walker.walk(dir, 0);
        }
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

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(err) => {
                    self.report(
                        format!("{}/<entry>", dir.display()),
                        format!("skill dir entry unreadable: {err}"),
                    );
                    continue;
                }
            };
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
            self.record(&path);
        }
    }

    fn record(&mut self, path: &Path) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(err) => {
                self.report(path.display().to_string(), format!("read failed: {err}"));
                return;
            }
        };
        // Same estimator as `skrills analyze`, so one SKILL.md shows
        // one token count everywhere.
        let tokens = count_tokens(&String::from_utf8_lossy(&bytes)).total as u64;
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

        // Prose ratio is 4 chars/token, rounded up, newline included:
        // alpha is 3000 + 1 chars -> 751 tokens; beta is 4 + 1 -> 2.
        assert_eq!(output.entries[0].tokens, 751);
        assert_eq!(output.entries[1].tokens, 2);
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
