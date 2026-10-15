//! CLI handler for the skill-diff command.

use crate::cli::OutputFormat;
use anyhow::{anyhow, Result};
use serde_json::json;
use skrills_discovery::{default_roots, discover_skills, SkillRoot, SkillSource};
use skrills_state::home_dir;
use std::path::PathBuf;

/// One copy of a skill: where it was found and what it says.
type Version = (SkillSource, PathBuf, String);

/// Every readable copy of the skill `name` under `roots`, one entry per file.
///
/// Each root is searched on its own so discovery's de-duplication does not
/// hide a copy. A map keyed by source kept only the last copy per source.
fn find_versions(roots: &[SkillRoot], name: &str) -> Result<Vec<Version>> {
    let search_name = normalize_skill_name(name);
    let mut versions: Vec<Version> = Vec::new();
    for root in roots {
        for meta in discover_skills(std::slice::from_ref(root), None)? {
            if normalize_skill_name(&meta.name) != search_name && meta.name != name {
                continue;
            }
            if versions.iter().any(|(_, p, _)| *p == meta.path) {
                continue;
            }
            match std::fs::read_to_string(&meta.path) {
                Ok(content) => versions.push((meta.source.clone(), meta.path.clone(), content)),
                Err(e) => {
                    tracing::warn!(path = %meta.path.display(), error = %e, "skipping unreadable copy")
                }
            }
        }
    }
    Ok(versions)
}

/// Handle the `skill-diff` command.
pub(crate) fn handle_skill_diff_command(
    name: String,
    format: OutputFormat,
    context_lines: usize,
) -> Result<()> {
    // Discover skills from each source separately to avoid deduplication.
    // This ensures we find ALL versions of a skill across different CLIs.
    // Use default_roots which includes all CLIs (Codex, Claude, Copilot).
    let home = home_dir()?;
    let mut roots = default_roots(&home);
    // Add Cursor rules from current working directory (project-local)
    let cursor_rules = std::env::current_dir()
        .unwrap_or_default()
        .join(".cursor/rules");
    if cursor_rules.exists() {
        roots.push(SkillRoot {
            root: cursor_rules,
            source: SkillSource::Cursor,
        });
    }
    let versions = find_versions(&roots, &name)?;

    if versions.is_empty() {
        return Err(anyhow!("Skill '{}' not found in any CLI", name));
    }

    if let [(source, path, _)] = versions.as_slice() {
        if format.is_json() {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "skill": name,
                    "found_in": [format!("{:?}", source)],
                    "identical": true,
                    "message": "Skill only exists in one CLI"
                }))?
            );
        } else {
            println!(
                "Skill '{}' only found in {:?} at {}",
                name,
                source,
                path.display()
            );
            println!("No diff available - skill exists in only one location.");
        }
        return Ok(());
    }

    // Compare every pair of copies. Two copies can share a source (two
    // plugins both shipping `review/SKILL.md`), so each copy is its own entry.
    let sources: Vec<_> = versions.iter().map(|(s, _, _)| s.clone()).collect();
    let mut comparisons = Vec::new();
    let mut all_identical = true;

    for i in 0..versions.len() {
        for j in (i + 1)..versions.len() {
            let (source_a, path_a, content_a) = &versions[i];
            let (source_b, path_b, content_b) = &versions[j];

            let diff = unified_diff(content_a, content_b, context_lines);
            let is_identical = content_a == content_b;

            if !is_identical {
                all_identical = false;
            }

            comparisons.push(json!({
                "source_a": format!("{:?}", source_a),
                "source_b": format!("{:?}", source_b),
                "path_a": path_a.to_string_lossy(),
                "path_b": path_b.to_string_lossy(),
                "identical": is_identical,
                "diff": if is_identical { None } else { Some(&diff) },
                "token_diff": estimate_token_diff(content_a, content_b)
            }));

            if !format.is_json() && !is_identical {
                println!("\n=== {:?} vs {:?} ===", source_a, source_b);
                println!("--- {:?}: {}", source_a, path_a.display());
                println!("+++ {:?}: {}", source_b, path_b.display());
                println!("{}", diff);
            }
        }
    }

    if format.is_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": name,
                "found_in": sources.iter().map(|s| format!("{:?}", s)).collect::<Vec<_>>(),
                "identical": all_identical,
                "comparisons": comparisons
            }))?
        );
    } else if all_identical {
        println!(
            "Skill '{}' is identical across {} copies: {:?}",
            name,
            versions.len(),
            sources
        );
    } else {
        println!(
            "\nSummary: Skill '{}' found in {} copies with differences",
            name,
            versions.len()
        );
    }

    Ok(())
}

/// One line of an edit script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op<'a> {
    Keep(&'a str),
    Delete(&'a str),
    Insert(&'a str),
}

/// Above this many table cells (lines of `a` times lines of `b`), the diff
/// is "delete all, insert all" rather than an LCS table that size.
const MAX_LCS_CELLS: usize = 16_000_000;

/// The shortest edit script from `a` to `b`, from a longest common
/// subsequence of their lines.
fn diff_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    if a.len().saturating_mul(b.len()) > MAX_LCS_CELLS {
        let mut ops: Vec<Op> = a.iter().map(|l| Op::Delete(l)).collect();
        ops.extend(b.iter().map(|l| Op::Insert(l)));
        return ops;
    }
    // lcs[i][j]: length of the LCS of a[i..] and b[j..].
    let width = b.len() + 1;
    let mut lcs = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if a[i] == b[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::with_capacity(a.len() + b.len());
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            ops.push(Op::Keep(a[i]));
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * width + j] >= lcs[i * width + j + 1] {
            ops.push(Op::Delete(a[i]));
            i += 1;
        } else {
            ops.push(Op::Insert(b[j]));
            j += 1;
        }
    }
    ops.extend(a[i..].iter().map(|l| Op::Delete(l)));
    ops.extend(b[j..].iter().map(|l| Op::Insert(l)));
    ops
}

/// A unified diff of `a` and `b` with `context` lines around each change and
/// an `@@ -start,len +start,len @@` header per hunk. Empty when they match.
fn unified_diff(a: &str, b: &str, context: usize) -> String {
    let lines_a: Vec<&str> = a.lines().collect();
    let lines_b: Vec<&str> = b.lines().collect();
    let ops = diff_ops(&lines_a, &lines_b);
    let changes: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, Op::Keep(_)))
        .map(|(i, _)| i)
        .collect();

    let mut output = String::new();
    let last = ops.len().saturating_sub(1);
    let mut k = 0;
    while k < changes.len() {
        let start = changes[k].saturating_sub(context);
        let mut end = changes[k].saturating_add(context).min(last);
        k += 1;
        // Merge the next change into this hunk when their context touches.
        while k < changes.len() && changes[k].saturating_sub(context) <= end.saturating_add(1) {
            end = changes[k].saturating_add(context).min(last);
            k += 1;
        }

        let before = &ops[..start];
        let hunk = &ops[start..=end];
        let a_before = before
            .iter()
            .filter(|o| !matches!(o, Op::Insert(_)))
            .count();
        let b_before = before
            .iter()
            .filter(|o| !matches!(o, Op::Delete(_)))
            .count();
        let a_len = hunk.iter().filter(|o| !matches!(o, Op::Insert(_))).count();
        let b_len = hunk.iter().filter(|o| !matches!(o, Op::Delete(_))).count();
        // An empty side names the line before the hunk, as `diff -u` does.
        let a_start = if a_len == 0 { a_before } else { a_before + 1 };
        let b_start = if b_len == 0 { b_before } else { b_before + 1 };
        output.push_str(&format!("@@ -{a_start},{a_len} +{b_start},{b_len} @@\n"));
        for op in hunk {
            let (mark, line) = match op {
                Op::Keep(l) => (' ', l),
                Op::Delete(l) => ('-', l),
                Op::Insert(l) => ('+', l),
            };
            output.push(mark);
            output.push_str(line);
            output.push('\n');
        }
    }

    output
}

/// Estimate token count difference between two contents.
fn estimate_token_diff(a: &str, b: &str) -> i64 {
    let tokens_a = estimate_tokens(a);
    let tokens_b = estimate_tokens(b);
    tokens_b as i64 - tokens_a as i64
}

/// Simple token estimation (words and punctuation).
fn estimate_tokens(content: &str) -> usize {
    // Rough estimate: ~4 chars per token for English text
    content.len() / 4
}

/// Normalize a skill name by extracting the base name from the path.
///
/// Converts names like "test-skill/SKILL.md" or "plugins/cache/.../skills/my-skill/SKILL.md"
/// to just "test-skill" or "my-skill".
fn normalize_skill_name(name: &str) -> String {
    // Remove trailing /SKILL.md or SKILL.md
    let name = name
        .trim_end_matches("/SKILL.md")
        .trim_end_matches("SKILL.md");

    // Extract the last component (skill directory name)
    if let Some(last_slash) = name.rfind('/') {
        name[last_slash + 1..].to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &std::path::Path, rel: &str, body: &str) -> PathBuf {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }

    /// SA-37: versions were keyed by source, so a second copy from the same
    /// source overwrote the first and was never compared.
    #[test]
    fn two_copies_from_the_same_source_are_both_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let a = write(
            tmp.path(),
            "plugin-a/review/SKILL.md",
            "---\nname: review\n---\nA\n",
        );
        let b = write(
            tmp.path(),
            "plugin-b/review/SKILL.md",
            "---\nname: review\n---\nB\n",
        );
        let roots: Vec<SkillRoot> = ["plugin-a", "plugin-b"]
            .iter()
            .map(|d| SkillRoot {
                root: tmp.path().join(d),
                source: SkillSource::Claude,
            })
            .collect();

        let versions = find_versions(&roots, "review").unwrap();

        let paths: Vec<_> = versions.iter().map(|(_, p, _)| p.clone()).collect();
        assert_eq!(paths, vec![a, b]);
    }

    #[test]
    fn the_same_file_reached_from_two_roots_is_counted_once() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "review/SKILL.md", "x");
        let root = SkillRoot {
            root: tmp.path().to_path_buf(),
            source: SkillSource::Claude,
        };

        let versions = find_versions(&[root.clone(), root], "review").unwrap();

        assert_eq!(versions.len(), 1);
    }

    /// One inserted line is one `+` line: the old diff compared line N to
    /// line N and marked every later line as changed.
    #[test]
    fn an_inserted_line_marks_only_that_line_changed() {
        let a = "1\n2\n3\n4\n5\n6\n";
        let b = "1\nnew\n2\n3\n4\n5\n6\n";

        assert_eq!(unified_diff(a, b, 1), "@@ -1,2 +1,3 @@\n 1\n+new\n 2\n");
    }

    #[test]
    fn distant_changes_get_separate_hunks() {
        let a = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let b = "A\nb\nc\nd\ne\nf\ng\nH\n";

        assert_eq!(
            unified_diff(a, b, 1),
            "@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -7,2 +7,2 @@\n g\n-h\n+H\n"
        );
    }

    #[test]
    fn identical_text_has_an_empty_diff() {
        assert_eq!(unified_diff("a\nb\n", "a\nb\n", 3), "");
    }

    /// `skill-diff -C 18446744073709551615` overflowed `i + context`.
    #[test]
    fn a_huge_context_does_not_overflow() {
        let diff = unified_diff("a\nb\n", "a\nc\n", usize::MAX);

        assert_eq!(diff, "@@ -1,2 +1,2 @@\n a\n-b\n+c\n");
    }
}
