//! Skill synchronization and AGENTS.md management.
//!
//! This module handles:
//! - Synchronizing skills from `~/.claude` to `~/.codex/skills-mirror`.
//! - Generating and updating `AGENTS.md` with available skills.

use anyhow::Result;
use skrills_discovery::{hash_file, AgentMeta, SkillMeta};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use crate::discovery::{
    collect_agents, collect_skills, is_skill_file, relative_path, AGENTS_AGENT_SECTION_END,
    AGENTS_AGENT_SECTION_START, AGENTS_SECTION_END, AGENTS_SECTION_START, AGENTS_TEXT,
};

/// Reports the outcome of a synchronization operation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SyncReport {
    pub(crate) copied: usize,
    pub(crate) skipped: usize,
    /// Relative paths of skills that were copied (new or updated).
    pub(crate) copied_names: Vec<String>,
}

/// Resolves the mirror source root, honoring `SKRILLS_MIRROR_SOURCE` when set.
pub(crate) fn mirror_source_root(home: &Path) -> PathBuf {
    std::env::var("SKRILLS_MIRROR_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".claude"))
}

/// Synchronizes skills from Claude's directory to a mirror directory.
///
/// Walks through the source directory and copies `SKILL.md` files to the destination,
/// only copying if the file is new or has changed (based on hash comparison).
pub(crate) fn sync_from_claude(
    claude_root: &Path,
    mirror_root: &Path,
    include_marketplace: bool,
) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    if !claude_root.exists() {
        return Ok(report);
    }
    // Dedicated agents mirror alongside skills mirror (e.g., ~/.codex/agents).
    let agents_root = mirror_root
        .parent()
        .map(|p| p.join("agents"))
        .unwrap_or_else(|| mirror_root.join("../agents"));
    // Track directories we've already mirrored to avoid repeated work when multiple SKILLs exist.
    let mut mirrored_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for entry in WalkDir::new(claude_root)
        .min_depth(1)
        .max_depth(8)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        // Skip marketplace if requested
        if !include_marketplace {
            let path = entry.path();
            if let Ok(rel) = path.strip_prefix(claude_root) {
                if rel.starts_with("plugins/marketplaces") {
                    continue;
                }
            }
        }

        let is_skill = is_skill_file(&entry);
        let is_agent = entry.file_type().is_file()
            && entry.path().extension().is_some_and(|ext| ext == "md")
            && entry
                .path()
                .ancestors()
                .any(|p| p.file_name().is_some_and(|n| n == "agents"));

        if !is_skill && !is_agent {
            continue;
        }
        let src = entry.into_path();
        let rel = relative_path(claude_root, &src).unwrap_or_else(|| src.clone());
        let dest = mirror_root.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        // Target in dedicated agents mirror if this is an agent.
        let agent_dest = agents_root.join(&rel);
        if is_agent {
            if let Some(parent) = agent_dest.parent() {
                fs::create_dir_all(parent)?;
            }
        }
        let should_copy = if dest.exists() {
            hash_file(&dest)? != hash_file(&src)?
        } else {
            true
        };
        if should_copy {
            fs::copy(&src, &dest)?;
            if is_agent {
                fs::copy(&src, &agent_dest)?;
            }
            if is_skill {
                report.copied += 1;
                // Store the relative path (directory name) for display
                if let Some(rel_path) = relative_path(claude_root, &src) {
                    // Extract parent directory name as the skill name (e.g., "nested" from "nested/SKILL.md")
                    let skill_name = rel_path
                        .parent()
                        .and_then(|p| p.to_str())
                        .unwrap_or_else(|| rel_path.to_str().unwrap_or("unknown"));
                    report.copied_names.push(skill_name.to_string());
                }
            }
            // Mirror additional supporting files that live alongside the SKILL.md
            if is_skill {
                if let Some(skill_dir) = src.parent() {
                    let rel_dir = relative_path(claude_root, skill_dir)
                        .unwrap_or_else(|| skill_dir.to_path_buf());
                    if mirrored_dirs.insert(rel_dir.clone()) {
                        for file in WalkDir::new(skill_dir)
                            .min_depth(1)
                            .max_depth(8)
                            .into_iter()
                            .filter_map(|e| e.ok())
                        {
                            if file.file_type().is_dir() {
                                continue;
                            }
                            let file_src = file.path();
                            // Skip SKILL.md itself; already copied above
                            if file_src.file_name().is_some_and(|n| n == "SKILL.md") {
                                continue;
                            }
                            let file_rel = relative_path(claude_root, file_src)
                                .unwrap_or_else(|| file_src.to_path_buf());
                            let file_dest = mirror_root.join(file_rel);
                            if let Some(parent) = file_dest.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            let copy_support = if file_dest.exists() {
                                hash_file(&file_dest)? != hash_file(file_src)?
                            } else {
                                true
                            };
                            if copy_support {
                                fs::copy(file_src, &file_dest)?;
                            }
                        }
                    }
                }
            }
        } else if is_skill {
            report.skipped += 1;
        }
    }
    Ok(report)
}

/// Renders a lightweight skills reference for AGENTS.md.
///
/// Instead of embedding a massive XML list (which can exceed 60K tokens),
/// this generates a compact reference pointing users to CLI commands for
/// dynamic skill discovery. Skills are discovered at runtime via `skrills list`.
pub(crate) fn render_skills_reference(skills: &[SkillMeta]) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        r#"<!-- Skills discovered dynamically. Last sync: {} UTC. Total: {} skills. -->
<!-- Use CLI commands for current skill inventory:
     skrills list              - List all discovered skills
     skrills list-pinned       - List pinned skills
     skrills doctor            - View discovery diagnostics
-->"#,
        ts,
        skills.len()
    )
}

/// Renders a lightweight agents reference for AGENTS.md.
///
/// Instead of embedding a massive list of agent paths and run commands,
/// this generates a compact reference pointing users to CLI commands.
pub(crate) fn render_agents_reference(agents: &[AgentMeta]) -> String {
    if agents.is_empty() {
        return String::new();
    }
    format!(
        r#"<!-- Agents discovered dynamically. Total: {} agents. -->
<!-- Use CLI commands for current agent inventory:
     skrills sync-agents       - Sync agents from external sources
     skrills doctor            - View agent discovery diagnostics
-->"#,
        agents.len()
    )
}

/// Writes or updates the AGENTS.md file with current skills.
///
/// Discovers skills from the specified directories and updates the AGENTS.md file
/// with an XML manifest of available skills.
pub(crate) fn sync_agents(path: &Path, extra_dirs: &[PathBuf]) -> Result<()> {
    let skills = collect_skills(extra_dirs)?;
    let agents = collect_agents(extra_dirs)?;
    sync_agents_with_assets(path, &skills, &agents)
}

/// Updates AGENTS.md with lightweight skill/agent references.
///
/// Instead of embedding massive XML lists (which can exceed 60K tokens),
/// this writes compact CLI references. Skills and agents are discovered
/// dynamically at runtime via `skrills list` and `skrills doctor`.
///
/// Creates the file with the default AGENTS.md template if it does not exist.
pub(crate) fn sync_agents_with_assets(
    path: &Path,
    skills: &[SkillMeta],
    agents: &[AgentMeta],
) -> Result<()> {
    // Generate lightweight references instead of massive inline lists
    let skills_ref = render_skills_reference(skills);
    let section = format!(
        "{start}\n{ref_text}\n{end}\n",
        start = AGENTS_SECTION_START,
        ref_text = skills_ref,
        end = AGENTS_SECTION_END
    );

    let agents_ref = render_agents_reference(agents);
    let agents_section = if agents_ref.is_empty() {
        String::new()
    } else {
        format!(
            "{start}\n{ref_text}\n{end}\n",
            start = AGENTS_AGENT_SECTION_START,
            ref_text = agents_ref,
            end = AGENTS_AGENT_SECTION_END
        )
    };

    let content = if path.exists() {
        let mut existing = fs::read_to_string(path)?;
        if let (Some(start), Some(end)) = (
            existing.find(AGENTS_SECTION_START),
            existing.find(AGENTS_SECTION_END),
        ) {
            let end_idx = end + AGENTS_SECTION_END.len();
            existing.replace_range(start..end_idx, &section);
            existing
        } else {
            format!("{existing}\n\n{section}")
        }
    } else {
        format!("{AGENTS_TEXT}\n\n{section}")
    };

    let mut final_content = content;
    if let Some(start) = final_content.find(AGENTS_AGENT_SECTION_START) {
        if let Some(end) = final_content.find(AGENTS_AGENT_SECTION_END) {
            let end_idx = end + AGENTS_AGENT_SECTION_END.len();
            final_content.replace_range(start..end_idx, &agents_section);
        } else {
            final_content.push_str(&format!("\n{}", agents_section));
        }
    } else if !agents_section.is_empty() {
        final_content.push('\n');
        final_content.push_str(&agents_section);
    }

    fs::write(path, final_content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::SkillSource;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn render_skills_reference_contains_count() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("codex/skills");
        fs::create_dir_all(&path).unwrap();
        let skill_path = path.join("alpha/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        fs::write(&skill_path, "hello").unwrap();
        let skills = vec![SkillMeta {
            name: "alpha/SKILL.md".into(),
            path: skill_path.clone(),
            source: SkillSource::Codex,
            root: path.clone(),
            hash: hash_file(&skill_path).unwrap(),
        }];
        let reference = render_skills_reference(&skills);
        assert!(reference.contains("Total: 1 skills"));
        assert!(reference.contains("skrills list"));
        assert!(reference.contains("skrills doctor"));
    }

    #[test]
    fn sync_agents_inserts_lightweight_section() -> Result<()> {
        let tmp = tempdir()?;
        let agents = tmp.path().join("AGENTS.md");
        fs::write(&agents, "# Title")?;
        let skills = vec![SkillMeta {
            name: "alpha/SKILL.md".into(),
            path: tmp.path().join("alpha/SKILL.md"),
            source: SkillSource::Codex,
            root: tmp.path().join("codex/skills"),
            hash: "abc".into(),
        }];
        sync_agents_with_assets(&agents, &skills, &[])?;
        let text = fs::read_to_string(&agents)?;
        assert!(text.contains(AGENTS_SECTION_START));
        assert!(text.contains("Total: 1 skills"));
        assert!(text.contains("skrills list"));
        assert!(text.contains(AGENTS_SECTION_END));
        assert!(text.contains("# Title"));
        // Should NOT contain the old XML format
        assert!(!text.contains("<skill name="));
        Ok(())
    }

    #[test]
    fn render_skills_reference_includes_cli_commands() -> Result<()> {
        let tmp = tempdir()?;
        let skills = vec![SkillMeta {
            name: "alpha/SKILL.md".into(),
            path: tmp.path().join("alpha/SKILL.md"),
            source: SkillSource::Codex,
            root: tmp.path().join("codex/skills"),
            hash: "abc".into(),
        }];
        let reference = render_skills_reference(&skills);
        assert!(reference.contains("skrills list"));
        assert!(reference.contains("skrills list-pinned"));
        assert!(reference.contains("skrills doctor"));
        Ok(())
    }

    #[test]
    fn sync_agents_appends_lightweight_agents_section() -> Result<()> {
        let tmp = tempdir()?;
        let agents_path = tmp.path().join("AGENTS.md");
        fs::write(&agents_path, "# Header")?;
        let skills = Vec::<SkillMeta>::new();
        let agents = vec![AgentMeta {
            name: "plugins/cache/tool/agents/helper.md".into(),
            path: tmp.path().join("plugins/cache/tool/agents/helper.md"),
            source: SkillSource::Cache,
            root: tmp.path().join("plugins/cache"),
            hash: "123".into(),
        }];
        sync_agents_with_assets(&agents_path, &skills, &agents)?;
        let text = fs::read_to_string(&agents_path)?;
        assert!(text.contains(AGENTS_AGENT_SECTION_START));
        assert!(text.contains("Total: 1 agents"));
        assert!(text.contains("skrills sync-agents"));
        // Should NOT contain the old verbose agent listing format
        assert!(!text.contains("codex --yolo exec"));
        Ok(())
    }

    #[test]
    fn sync_from_claude_copies_agents_into_codex_agents_dir() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        let agent_dir = claude_root.join("plugins/cache/tool/agents");
        fs::create_dir_all(&agent_dir)?;
        let agent_src = agent_dir.join("helper.md");
        fs::write(&agent_src, "agent content")?;

        let _report = sync_from_claude(&claude_root, &mirror_root, false)?;

        let agent_dest = mirror_root
            .parent()
            .unwrap()
            .join("agents/plugins/cache/tool/agents/helper.md");
        assert!(agent_dest.exists());
        assert_eq!(fs::read_to_string(agent_dest)?, "agent content");
        Ok(())
    }

    #[test]
    fn sync_from_claude_copies_and_updates() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");
        fs::create_dir_all(claude_root.join("nested"))?;
        let skill_src = claude_root.join("nested/SKILL.md");
        fs::write(&skill_src, "v1")?;

        let report1 = sync_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report1.copied, 1);
        let dest = mirror_root.join("nested/SKILL.md");
        assert_eq!(fs::read_to_string(&dest)?, "v1");

        std::thread::sleep(Duration::from_millis(5));
        fs::write(&skill_src, "v2")?;
        let report2 = sync_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report2.copied, 1);
        assert_eq!(fs::read_to_string(&dest)?, "v2");
        Ok(())
    }

    #[test]
    fn sync_from_claude_reaches_marketplace_depth() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        // Depth: claude/plugins/marketplaces/a/plugins/b/skills/c/SKILL.md (7 levels)
        let deep_dir = claude_root.join("plugins/marketplaces/a/plugins/b/skills/c");
        fs::create_dir_all(&deep_dir)?;
        let skill_src = deep_dir.join("SKILL.md");
        fs::write(&skill_src, "deep")?;

        let report = sync_from_claude(&claude_root, &mirror_root, true)?;
        assert_eq!(report.copied, 1);
        let dest = mirror_root.join("plugins/marketplaces/a/plugins/b/skills/c/SKILL.md");
        assert_eq!(fs::read_to_string(&dest)?, "deep");
        Ok(())
    }

    #[test]
    fn sync_from_claude_ignores_marketplace_when_disabled() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        let deep_dir = claude_root.join("plugins/marketplaces/a/plugins/b/skills/c");
        fs::create_dir_all(&deep_dir)?;
        let skill_src = deep_dir.join("SKILL.md");
        fs::write(&skill_src, "deep")?;

        let report = sync_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report.copied, 0);
        Ok(())
    }

    #[test]
    fn sync_from_claude_reaches_cache_depth() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        // Depth in cache tree
        let deep_dir = claude_root.join("plugins/cache/x/y/z/skills/foo");
        fs::create_dir_all(&deep_dir)?;
        let skill_src = deep_dir.join("SKILL.md");
        fs::write(&skill_src, "cache-skill")?;

        let report = sync_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report.copied, 1);
        let dest = mirror_root.join("plugins/cache/x/y/z/skills/foo/SKILL.md");
        assert_eq!(fs::read_to_string(&dest)?, "cache-skill");
        Ok(())
    }

    #[test]
    fn sync_from_claude_copies_supporting_files() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        let skill_dir = claude_root.join("plugins/cache/tool/skills/demo");
        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join("SKILL.md"), "skill")?;
        fs::write(skill_dir.join("helper.py"), "print('hi')")?;
        fs::write(skill_dir.join("config.json"), "{\"ok\":true}")?;

        let report = sync_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report.copied, 1);

        let helper_dest = mirror_root.join("plugins/cache/tool/skills/demo/helper.py");
        let config_dest = mirror_root.join("plugins/cache/tool/skills/demo/config.json");
        assert!(helper_dest.exists());
        assert!(config_dest.exists());
        assert_eq!(fs::read_to_string(helper_dest)?, "print('hi')");
        assert_eq!(fs::read_to_string(config_dest)?, "{\"ok\":true}");
        Ok(())
    }
}
