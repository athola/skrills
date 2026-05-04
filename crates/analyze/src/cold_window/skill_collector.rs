//! Skill participation in the cold-window tick (NI16, PR-218 wave-4).
//!
//! Parallel to [`super::plugin_health::PluginHealthCollector`]: walks
//! the configured skill directories each tick and produces a list of
//! `(source, token_estimate)` entries that the producer feeds into the
//! tick's `TokenLedger::per_skill` attribution.
//!
//! Token estimation uses a deliberate **byte-based proxy** — file size
//! divided by 4 — so the cold rewalk stays cheap. This is wrong-in-the-
//! detail (a Markdown file is not 4 chars per token) but right-in-the-
//! shape: skills that grow show as larger consumers, skills that shrink
//! pull back. A real BPE tokenizer is a v0.9.0 follow-up; until then
//! this surfaces *real* attribution rather than the synthetic
//! `skill://demo` placeholder shipped pre-NI16.
//!
//! The collector is **deliberately stateless and side-effect free**,
//! mirroring the plugin collector's "cold rewalk" contract. Errors
//! reading individual entries are surfaced via `malformed`, not
//! silently dropped — same NI15/B2 discipline applied elsewhere.

use std::path::{Path, PathBuf};

use skrills_snapshot::TokenEntry;

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
    /// `tokens` is a byte-length / 4 estimate.
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
        for dir in &self.skill_dirs {
            walk(dir, 0, &mut output);
        }
        // Stable order so ledger comparisons are reproducible.
        output.entries.sort_by(|a, b| a.source.cmp(&b.source));
        output
    }
}

fn walk(dir: &Path, depth: usize, output: &mut SkillCollectorOutput) {
    if depth > MAX_DEPTH {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            output.malformed.push(MalformedSkillEntry {
                source: dir.display().to_string(),
                error_message: format!("skill dir unreadable: {err}"),
            });
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                output.malformed.push(MalformedSkillEntry {
                    source: format!("{}/<entry>", dir.display()),
                    error_message: format!("skill dir entry unreadable: {err}"),
                });
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(err) => {
                output.malformed.push(MalformedSkillEntry {
                    source: path.display().to_string(),
                    error_message: format!("file_type read failed: {err}"),
                });
                continue;
            }
        };
        if file_type.is_dir() {
            walk(&path, depth + 1, output);
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
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(err) => {
                output.malformed.push(MalformedSkillEntry {
                    source: path.display().to_string(),
                    error_message: format!("metadata read failed: {err}"),
                });
                continue;
            }
        };
        let bytes = metadata.len();
        // Byte-length / 4 token-estimate; saturate to u64 max well below
        // any realistic skill size but keep the saturation guard explicit.
        let tokens = bytes.saturating_div(4);
        let skill_name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("<unnamed>");
        output.entries.push(TokenEntry {
            source: format!("skill://{skill_name}"),
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
    fn collect_finds_skills_and_attributes_byte_proxy_tokens() {
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

        // alpha was 3000 bytes → 750 tokens; beta was 4 bytes → 1 token.
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
        // NI9 contract); no malformed entry, no panic.
        assert!(output.entries.is_empty());
        assert!(output.malformed.is_empty());
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
