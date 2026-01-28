use anyhow::Result;
use skrills_discovery::{discover_skills, extra_skill_roots};
use std::path::PathBuf;

use crate::cli::{OutputFormat, SyncSource};
use crate::discovery::merge_extra_dirs;

use super::{CatalogEntry, CatalogResult};

/// Handle the skill-catalog command.
pub(crate) fn handle_skill_catalog_command(
    search: Option<String>,
    source: Option<SyncSource>,
    _category: Option<String>,
    limit: usize,
    skill_dirs: Vec<PathBuf>,
    format: OutputFormat,
) -> Result<()> {
    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let roots = extra_skill_roots(&extra_dirs);
    let skills = discover_skills(&roots, None)?;

    let mut entries: Vec<CatalogEntry> = skills
        .iter()
        .filter(|s| {
            if let Some(ref query) = search {
                let q = query.to_lowercase();
                s.name.to_lowercase().contains(&q)
                    || s.description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&q))
                        .unwrap_or(false)
            } else {
                true
            }
        })
        .filter(|s| {
            if let Some(ref src) = source {
                let path_str = s.path.to_string_lossy().to_lowercase();
                match src {
                    SyncSource::Claude => path_str.contains("claude"),
                    SyncSource::Codex => path_str.contains("codex"),
                    SyncSource::Copilot => path_str.contains("copilot"),
                }
            } else {
                true
            }
        })
        .take(limit)
        .map(|s| {
            let source_name = if s.path.to_string_lossy().contains("claude") {
                "claude"
            } else if s.path.to_string_lossy().contains("codex") {
                "codex"
            } else if s.path.to_string_lossy().contains("copilot") {
                "copilot"
            } else {
                "local"
            };
            CatalogEntry {
                name: s.name.clone(),
                source: source_name.to_string(),
                description: s.description.clone(),
                path: s.path.clone(),
                deprecated: false,
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let result = CatalogResult {
        total_skills: entries.len(),
        skills: entries.clone(),
    };

    if format.is_json() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Skill Catalog ({} skills)", result.total_skills);
        println!("═══════════════════════════════════════════════════════════════════════");
        println!();

        for entry in &entries {
            let desc = entry
                .description
                .as_ref()
                .map(|d| {
                    if d.len() > 60 {
                        format!("{}...", &d[..57])
                    } else {
                        d.clone()
                    }
                })
                .unwrap_or_else(|| "(no description)".to_string());
            println!("  {} [{}]", entry.name, entry.source);
            println!("    {}", desc);
            println!();
        }

        if let Some(ref query) = search {
            println!("Filtered by: \"{}\"", query);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_test_utils::{env_guard, TestFixture};

    #[test]
    fn catalog_discovers_skills_in_fixture() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        fixture
            .create_skill_with_frontmatter("test-skill-alpha", "Alpha skill for testing", "Content")
            .expect("create alpha");
        fixture
            .create_skill_with_frontmatter("test-skill-beta", "Beta skill for testing", "Content")
            .expect("create beta");

        let result = handle_skill_catalog_command(
            None,
            None,
            None,
            100,
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "catalog command should succeed");
    }

    #[test]
    fn catalog_filters_by_search_query() {
        let _g = env_guard();
        let fixture = TestFixture::new().expect("fixture");
        let _home = fixture.home_guard();

        fixture
            .create_skill_with_frontmatter("matching-skill", "This matches the query", "Content")
            .expect("create matching");
        fixture
            .create_skill_with_frontmatter("other-skill", "This does not", "Content")
            .expect("create other");

        let result = handle_skill_catalog_command(
            Some("matching".to_string()),
            None,
            None,
            100,
            vec![fixture.claude_skills.clone()],
            OutputFormat::Json,
        );

        assert!(result.is_ok(), "filtered catalog should succeed");
    }
}
