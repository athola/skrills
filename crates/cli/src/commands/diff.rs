//! CLI handler for the skill-diff command.

use crate::cli::OutputFormat;
use anyhow::{anyhow, Result};
use serde_json::json;
use skrills_discovery::{default_roots, discover_skills, SkillRoot, SkillSource};
use skrills_state::home_dir;
use skrills_sync::text_diff::unified_diff;
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
    let home = home_dir()?;
    let roots = diff_roots(&home, &std::env::current_dir().unwrap_or_default());
    let versions = find_versions(&roots, &name)?;

    if versions.is_empty() {
        return Err(anyhow!("Skill '{}' not found in any CLI", name));
    }

    if let [(source, path, _)] = versions.as_slice() {
        if format.is_json() {
            println!(
                "{}",
                serde_json::to_string_pretty(&single_copy_json(&name, source))?
            );
        } else {
            println!(
                "Skill '{}' only found in {} at {}",
                name,
                source.label(),
                path.display()
            );
            println!("No diff available - skill exists in only one location.");
        }
        return Ok(());
    }

    // Compare every pair of copies. Two copies can share a source (two
    // plugins both shipping `review/SKILL.md`), so each copy is its own entry.
    let sources: Vec<String> = versions.iter().map(|(s, _, _)| s.label()).collect();
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
                "source_a": source_a.label(),
                "source_b": source_b.label(),
                "path_a": path_a.to_string_lossy(),
                "path_b": path_b.to_string_lossy(),
                "identical": is_identical,
                "diff": if is_identical { None } else { Some(&diff) },
                "token_diff": estimate_token_diff(content_a, content_b)
            }));

            if !format.is_json() && !is_identical {
                println!("\n=== {} vs {} ===", source_a.label(), source_b.label());
                println!("--- {}: {}", source_a.label(), path_a.display());
                println!("+++ {}: {}", source_b.label(), path_b.display());
                println!("{}", diff);
            }
        }
    }

    if format.is_json() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": name,
                "found_in": sources,
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

/// Token count difference between two contents, by the analyzer's estimate
/// so it agrees with `analyze` and `metrics`.
fn estimate_token_diff(a: &str, b: &str) -> i64 {
    let tokens_a = skrills_analyze::estimate_tokens(a.len());
    let tokens_b = skrills_analyze::estimate_tokens(b.len());
    tokens_b as i64 - tokens_a as i64
}

/// The roots `skill-diff` searches: every CLI's default roots, the Cursor
/// skills directory, and the project's `.cursor/rules` when present.
fn diff_roots(home: &std::path::Path, cwd: &std::path::Path) -> Vec<SkillRoot> {
    let mut roots = default_roots(home);
    let cursor_skills =
        crate::commands::sync::source_skill_root(crate::cli::Platform::Cursor, home);
    if !roots.iter().any(|r| r.root == cursor_skills) {
        roots.push(SkillRoot {
            root: cursor_skills,
            source: SkillSource::Cursor,
        });
    }
    let cursor_rules = cwd.join(".cursor/rules");
    if cursor_rules.exists() {
        roots.push(SkillRoot {
            root: cursor_rules,
            source: SkillSource::Cursor,
        });
    }
    roots
}

/// The JSON for a skill found once: nothing was compared, so `identical`
/// is null rather than true.
fn single_copy_json(name: &str, source: &SkillSource) -> serde_json::Value {
    json!({
        "skill": name,
        "found_in": [source.label()],
        "identical": null,
        "message": "Skill only exists in one CLI"
    })
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

    #[test]
    fn diff_roots_include_the_cursor_skills_directory() {
        let _g = skrills_test_utils::env_guard();
        let home = tempfile::tempdir().unwrap();
        let _h = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        let cwd = tempfile::tempdir().unwrap();

        let roots = diff_roots(home.path(), cwd.path());

        let cursor =
            crate::commands::sync::source_skill_root(crate::cli::Platform::Cursor, home.path());
        assert!(
            roots
                .iter()
                .any(|r| r.root == cursor && r.source == SkillSource::Cursor),
            "{roots:?}"
        );
    }

    #[test]
    fn diff_roots_include_the_project_cursor_rules_when_present() {
        let _g = skrills_test_utils::env_guard();
        let home = tempfile::tempdir().unwrap();
        let _h = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        let cwd = tempfile::tempdir().unwrap();
        let rules = cwd.path().join(".cursor/rules");

        let without = diff_roots(home.path(), cwd.path());
        std::fs::create_dir_all(&rules).unwrap();
        let with = diff_roots(home.path(), cwd.path());

        assert!(!without.iter().any(|r| r.root == rules), "{without:?}");
        assert!(
            with.iter()
                .any(|r| r.root == rules && r.source == SkillSource::Cursor),
            "{with:?}"
        );
    }

    #[test]
    fn token_diff_uses_the_analyzer_estimate() {
        let a = "x".repeat(10);
        let b = "x".repeat(20);
        let expected = skrills_analyze::estimate_tokens(20) as i64
            - skrills_analyze::estimate_tokens(10) as i64;

        assert_eq!(estimate_token_diff(&a, &b), expected);
    }

    #[test]
    fn single_copy_report_says_nothing_was_compared() {
        let report = single_copy_json("review", &SkillSource::Extra(0));

        assert!(report["identical"].is_null(), "{report}");
        assert_eq!(report["found_in"][0], "extra0");
    }
}
