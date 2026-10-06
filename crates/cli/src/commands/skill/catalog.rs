use anyhow::{bail, Result};
use skrills_discovery::{discover_skills, SkillMeta};
use std::path::PathBuf;

use crate::cli::{OutputFormat, SyncSource};

use super::{CatalogEntry, CatalogResult};

/// Handle the skill-catalog command.
pub(crate) fn handle_skill_catalog_command(
    search: Option<String>,
    source: Option<SyncSource>,
    category: Option<String>,
    limit: usize,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    // Skills carry no category field to filter on. Refuse rather than return
    // an unfiltered catalog that looks filtered.
    if category.is_some() {
        bail!("skill-catalog --category is not implemented: skills have no category field");
    }

    let roots = crate::commands::skill_roots_for(&skill_dirs);
    let skills = discover_skills(&roots, None)?;
    let result = build_catalog(&skills, search.as_deref(), source, limit);

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!(
            "Skill Catalog ({} of {} skills)",
            result.skills.len(),
            result.total_skills
        );
        println!("═══════════════════════════════════════════════════════════════════════");
        println!();

        for entry in &result.skills {
            let desc = entry
                .description
                .as_deref()
                .map(|d| truncate_chars(d, 60))
                .unwrap_or_else(|| "(no description)".to_string());
            let deprecated = if entry.deprecated {
                " (deprecated)"
            } else {
                ""
            };
            println!("  {} [{}]{}", entry.name, entry.source, deprecated);
            println!("    {}", desc);
            println!();
        }

        if let Some(ref query) = search {
            println!("Filtered by: \"{}\"", query);
        }
    }

    Ok(())
}

/// The catalog for `skills`: filtered by `search` and `source`, sorted by
/// name, then cut to `limit`. `total_skills` counts every match, before the
/// cut, so `--limit 10` shows the first ten names out of the real total.
fn build_catalog(
    skills: &[SkillMeta],
    search: Option<&str>,
    source: Option<SyncSource>,
    limit: usize,
) -> CatalogResult {
    let query = search.map(str::to_lowercase);
    let mut entries: Vec<CatalogEntry> = skills
        .iter()
        .filter(|s| match &query {
            Some(q) => {
                s.name.to_lowercase().contains(q)
                    || s.description
                        .as_ref()
                        .is_some_and(|d| d.to_lowercase().contains(q))
            }
            None => true,
        })
        .filter(|s| source.is_none_or(|src| s.source.label() == src.as_str()))
        .map(|s| CatalogEntry {
            name: s.name.clone(),
            source: s.source.label(),
            description: s.description.clone(),
            path: s.path.clone(),
            deprecated: is_deprecated(s),
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let total_skills = entries.len();
    entries.truncate(limit);

    CatalogResult {
        total_skills,
        skills: entries,
    }
}

/// Whether the skill's frontmatter carries `deprecated: true`, as
/// `skill-deprecate` writes it. An unreadable file counts as not deprecated.
fn is_deprecated(meta: &SkillMeta) -> bool {
    let Ok(content) = std::fs::read_to_string(&meta.path) else {
        return false;
    };
    skrills_validate::frontmatter::parse_frontmatter(&content)
        .ok()
        .and_then(|p| p.raw_frontmatter)
        .is_some_and(|fm| fm.lines().any(|l| l.trim_end() == "deprecated: true"))
}

/// `s` cut to at most `max` characters, with `...` marking a cut. Counts
/// characters, not bytes: slicing at byte 57 panicked when a multi-byte
/// character straddled it.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let kept: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{kept}...")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::{SkillRoot, SkillSource};

    fn skill(dir: &std::path::Path, name: &str, frontmatter_extra: &str) {
        let path = dir.join(name).join("SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!(
                "---\nname: {name}\ndescription: About {name}\n{frontmatter_extra}---\n# {name}\n"
            ),
        )
        .unwrap();
    }

    fn discovered(dir: &std::path::Path) -> Vec<SkillMeta> {
        discover_skills(
            &[SkillRoot {
                root: dir.to_path_buf(),
                source: SkillSource::Claude,
            }],
            None,
        )
        .unwrap()
    }

    /// SA-6: `&d[..57]` panicked on a multi-byte character at byte 57.
    #[test]
    fn truncate_chars_never_splits_a_character() {
        let d = format!("{}é{}", "a".repeat(56), "b".repeat(10));
        assert_eq!(d.len(), 68);

        let cut = truncate_chars(&d, 60);

        assert_eq!(cut, format!("{}é...", "a".repeat(56)));
        assert_eq!(truncate_chars("short", 60), "short");
    }

    /// SA-36: `.take(limit)` ran before the sort, and `total_skills` was the
    /// post-limit count.
    #[test]
    fn limit_applies_after_sorting_and_total_counts_every_match() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["delta", "alpha", "charlie", "bravo"] {
            skill(tmp.path(), name, "");
        }

        let catalog = build_catalog(&discovered(tmp.path()), None, None, 2);

        let names: Vec<_> = catalog.skills.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha/SKILL.md", "bravo/SKILL.md"]);
        assert_eq!(catalog.total_skills, 4);
    }

    #[test]
    fn search_and_source_filter() {
        let tmp = tempfile::tempdir().unwrap();
        skill(tmp.path(), "matching-skill", "");
        skill(tmp.path(), "other", "");
        let skills = discovered(tmp.path());

        let found = build_catalog(&skills, Some("MATCHING"), None, 100);
        assert_eq!(found.total_skills, 1);

        let none = build_catalog(&skills, None, Some(SyncSource::Codex), 100);
        assert_eq!(none.total_skills, 0);
        let all = build_catalog(&skills, None, Some(SyncSource::Claude), 100);
        assert_eq!(all.total_skills, 2);
    }

    /// SA-36: `deprecated` was hard-coded false.
    #[test]
    fn a_deprecated_skill_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        skill(tmp.path(), "old", "deprecated: true\n");
        skill(tmp.path(), "new", "");

        let catalog = build_catalog(&discovered(tmp.path()), None, None, 100);

        let flags: Vec<_> = catalog
            .skills
            .iter()
            .map(|e| (e.name.as_str(), e.deprecated))
            .collect();
        assert_eq!(flags, [("new/SKILL.md", false), ("old/SKILL.md", true)]);
    }

    #[test]
    fn category_is_refused() {
        let err = handle_skill_catalog_command(
            None,
            None,
            Some("testing".into()),
            10,
            vec![],
            OutputFormat::Json,
        )
        .unwrap_err();
        assert!(err.to_string().contains("--category"), "{err}");
    }
}
