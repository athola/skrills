//! AGENTS.md management and the Claude to Codex agent copy.
//!
//! This module handles:
//! - Copying Claude agents into the Codex agents root. Skills go through
//!   [`crate::sync_claude_skills_to_codex`] instead.
//! - Generating and updating `AGENTS.md` with available skills.

use anyhow::Result;
use skrills_discovery::{AgentMeta, SkillMeta};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use std::io::{BufReader, Read};

use skrills_discovery::{collect_agents, collect_skills, relative_path};

/// Default AGENTS.md written when the file does not exist yet; the MCP
/// server also serves it as the `doc://agents` resource.
pub const AGENTS_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/AGENTS.md"));
/// Start marker for the available skills section in AGENTS.md.
pub const AGENTS_SECTION_START: &str = "<!-- available_skills:start -->";
/// End marker for the available skills section in AGENTS.md.
pub const AGENTS_SECTION_END: &str = "<!-- available_skills:end -->";
/// Start marker for available agents.
pub const AGENTS_AGENT_SECTION_START: &str = "<!-- available_agents:start -->";
/// End marker for available agents.
pub const AGENTS_AGENT_SECTION_END: &str = "<!-- available_agents:end -->";

fn is_hidden_rel_path(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        std::path::Component::Normal(s) => s.to_string_lossy().starts_with('.'),
        _ => false,
    })
}

fn mirror_path_if_changed(src: &Path, dest_root: &Path, rel: &Path) -> Result<bool> {
    let dest = dest_root.join(rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let should_copy = should_copy_by_content(src, &dest)?;
    if should_copy {
        fs::copy(src, &dest)?;
    }
    Ok(should_copy)
}

fn should_copy_by_content(src: &Path, dest: &Path) -> Result<bool> {
    if !dest.exists() {
        return Ok(true);
    }
    let src_meta = fs::metadata(src)?;
    let dest_meta = fs::metadata(dest)?;
    if src_meta.len() != dest_meta.len() {
        return Ok(true);
    }
    let mut a = BufReader::new(fs::File::open(src)?);
    let mut b = BufReader::new(fs::File::open(dest)?);
    let mut buf_a = [0u8; 8192];
    let mut buf_b = [0u8; 8192];
    loop {
        let n1 = a.read(&mut buf_a)?;
        let n2 = b.read(&mut buf_b)?;
        if n1 != n2 {
            return Ok(true);
        }
        if n1 == 0 {
            return Ok(false);
        }
        if buf_a[..n1] != buf_b[..n1] {
            return Ok(true);
        }
    }
}

/// Reports the outcome of mirroring agents into a Codex root.
///
/// Named apart from [`crate::SyncReport`], which the same call sites
/// also handle and which covers commands, MCP servers and preferences.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MirrorReport {
    pub copied: usize,
    pub skipped: usize,
    /// Relative paths of skills that were copied (new or updated).
    pub copied_names: Vec<String>,
}

/// Resolves the mirror source root, honoring `SKRILLS_MIRROR_SOURCE` when set.
pub fn mirror_source_root(home: &Path) -> PathBuf {
    std::env::var("SKRILLS_MIRROR_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".claude"))
}

/// Renders a lightweight skills reference for AGENTS.md.
///
/// Instead of embedding a massive XML list (which can exceed 60K tokens),
/// this generates a compact reference pointing users to CLI commands for
/// dynamic skill discovery. Skills are discovered at runtime and recorded in
/// `~/.codex/skills-cache.json` (or enumerated directly from `~/.codex/skills/`).
pub(crate) fn render_skills_reference(skills: &[SkillMeta]) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        r"<!-- Skills discovered dynamically. Last sync: {} UTC. Total: {} skills. -->
<!-- Use CLI commands for current skill inventory:
     jq -r '.skills[].path' ~/.codex/skills-cache.json
     find ~/.codex/skills -name SKILL.md -type f
     skrills analyze           - Analyze skills (tokens/deps) to spot issues
     skrills doctor            - View discovery diagnostics
-->",
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
        r"<!-- Agents discovered dynamically. Total: {} agents. -->
<!-- Use CLI commands for current agent inventory:
     skrills sync-agents       - Sync agents from external sources
     skrills doctor            - View agent discovery diagnostics
-->",
        agents.len()
    )
}

/// Writes or updates the AGENTS.md file with current skills.
///
/// Discovers skills from the specified directories and updates the AGENTS.md file
/// with an XML manifest of available skills.
pub fn sync_agents(path: &Path, extra_dirs: &[PathBuf]) -> Result<()> {
    let skills = collect_skills(extra_dirs)?;
    let agents = collect_agents(extra_dirs)?;
    sync_agents_with_assets(path, &skills, &agents)
}

/// Updates AGENTS.md with lightweight skill/agent references.
///
/// Instead of embedding massive XML lists (which can exceed 60K tokens),
/// this writes compact CLI references. Skills and agents are discovered
/// dynamically at runtime (see `~/.codex/skills-cache.json`) and via `skrills doctor`.
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
        let existing = fs::read_to_string(path)?;
        match replace_marked_section(
            &existing,
            AGENTS_SECTION_START,
            AGENTS_SECTION_END,
            &section,
            path,
        )? {
            Some(updated) => updated,
            None => format!("{existing}\n\n{section}"),
        }
    } else {
        format!("{AGENTS_TEXT}\n\n{section}")
    };

    let final_content = match replace_marked_section(
        &content,
        AGENTS_AGENT_SECTION_START,
        AGENTS_AGENT_SECTION_END,
        &agents_section,
        path,
    )? {
        Some(updated) => updated,
        None if agents_section.is_empty() => content,
        None => format!("{content}\n{agents_section}"),
    };

    fs::write(path, final_content)?;
    Ok(())
}

/// Replaces the text from `start` through the first `end` after it.
///
/// Returns `Ok(None)` when `start` is absent, and an error naming `path` when
/// `start` has no `end` after it, so a hand-edited file is never rewritten
/// around misordered markers.
fn replace_marked_section(
    text: &str,
    start: &str,
    end: &str,
    replacement: &str,
    path: &Path,
) -> Result<Option<String>> {
    let Some(start_idx) = text.find(start) else {
        if text.contains(end) {
            return Err(anyhow::anyhow!(
                "{} has `{end}` without `{start}`; fix the markers and rerun",
                path.display()
            ));
        }
        return Ok(None);
    };
    let Some(end_offset) = text[start_idx..].find(end) else {
        return Err(anyhow::anyhow!(
            "{} has `{start}` with no `{end}` after it; fix the markers and rerun",
            path.display()
        ));
    };
    let end_idx = start_idx + end_offset + end.len();
    // Swallow the newline after the end marker; `replacement` brings its own.
    let end_idx = if text[end_idx..].starts_with('\n') && replacement.ends_with('\n') {
        end_idx + 1
    } else {
        end_idx
    };
    let mut updated = text.to_string();
    updated.replace_range(start_idx..end_idx, replacement);
    Ok(Some(updated))
}

/// Synchronizes agent markdown files from Claude into the Codex agents root (e.g. `~/.codex/agents`).
///
/// This intentionally does **not** write to `~/.codex/skills-mirror`: skills go to
/// `~/.agents/skills` via [`crate::sync_claude_skills_to_codex`].
pub fn sync_agents_only_from_claude(
    claude_root: &Path,
    codex_agents_root: &Path,
    include_marketplace: bool,
) -> Result<MirrorReport> {
    let mut report = MirrorReport::default();
    if !claude_root.exists() {
        return Ok(report);
    }

    for entry in WalkDir::new(claude_root)
        .min_depth(1)
        .max_depth(20)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_symlink() {
                return false;
            }
            let rel = e.path().strip_prefix(claude_root).unwrap_or(e.path());
            if is_hidden_rel_path(rel) {
                return false;
            }
            true
        })
        .filter_map(|e| e.ok())
    {
        if !include_marketplace {
            let path = entry.path();
            if let Ok(rel) = path.strip_prefix(claude_root) {
                if rel.starts_with("plugins/marketplaces") {
                    continue;
                }
            }
        }

        if entry.file_type().is_symlink() || !entry.file_type().is_file() {
            continue;
        }

        let is_agent = entry.path().extension().is_some_and(|ext| ext == "md")
            && entry
                .path()
                .ancestors()
                .any(|p| p.file_name().is_some_and(|n| n == "agents"));
        if !is_agent {
            continue;
        }

        let src = entry.into_path();
        let rel = relative_path(claude_root, &src).unwrap_or_else(|| src.clone());
        let copied = mirror_path_if_changed(&src, codex_agents_root, &rel)?;
        if copied {
            report.copied += 1;
            if let Some(rel_path) = relative_path(claude_root, &src) {
                report
                    .copied_names
                    .push(rel_path.to_string_lossy().into_owned());
            }
        } else {
            report.skipped += 1;
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::{hash_file, SkillSource};
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
            description: None,
            frontmatter_name: None,
        }];
        let reference = render_skills_reference(&skills);
        assert!(reference.contains("Total: 1 skills"));
        assert!(reference.contains("skills-cache.json"));
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
            description: None,
            frontmatter_name: None,
        }];
        sync_agents_with_assets(&agents, &skills, &[])?;
        let text = fs::read_to_string(&agents)?;
        assert!(text.contains(AGENTS_SECTION_START));
        assert!(text.contains("Total: 1 skills"));
        assert!(text.contains("skills-cache.json"));
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
            description: None,
            frontmatter_name: None,
        }];
        let reference = render_skills_reference(&skills);
        assert!(reference.contains("skills-cache.json"));
        assert!(reference.contains("find ~/.codex/skills"));
        assert!(reference.contains("skrills analyze"));
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
    fn sync_agents_copies_agents_into_codex_agents_dir() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");

        let agent_dir = claude_root.join("plugins/cache/tool/agents");
        fs::create_dir_all(&agent_dir)?;
        let agent_src = agent_dir.join("helper.md");
        fs::write(&agent_src, "agent content")?;

        let agents_root = tmp.path().join("agents");
        let report = sync_agents_only_from_claude(&claude_root, &agents_root, false)?;
        assert_eq!(report.copied, 1);

        let agent_dest = agents_root.join("plugins/cache/tool/agents/helper.md");
        assert!(agent_dest.exists());
        assert_eq!(fs::read_to_string(agent_dest)?, "agent content");
        Ok(())
    }

    #[test]
    fn sync_agents_rejects_an_end_marker_above_the_start_marker() -> Result<()> {
        let tmp = tempdir()?;
        let path = tmp.path().join("AGENTS.md");
        let original = format!("# T\n{AGENTS_SECTION_END}\nnotes\n{AGENTS_SECTION_START}\nold\n");
        fs::write(&path, &original)?;

        let err = sync_agents_with_assets(&path, &[], &[]).expect_err("misordered markers");
        assert!(err.to_string().contains("AGENTS.md"), "{err}");
        assert_eq!(fs::read_to_string(&path)?, original);
        Ok(())
    }

    #[test]
    fn sync_agents_rejects_a_misordered_agents_section() -> Result<()> {
        let tmp = tempdir()?;
        let path = tmp.path().join("AGENTS.md");
        let original =
            format!("# T\n{AGENTS_AGENT_SECTION_END}\n{AGENTS_AGENT_SECTION_START}\nold\n");
        fs::write(&path, &original)?;
        let agents = vec![AgentMeta {
            name: "a.md".into(),
            path: tmp.path().join("a.md"),
            source: SkillSource::Cache,
            root: tmp.path().to_path_buf(),
            hash: "1".into(),
        }];

        assert!(sync_agents_with_assets(&path, &[], &agents).is_err());
        assert_eq!(fs::read_to_string(&path)?, original);
        Ok(())
    }

    #[test]
    fn sync_agents_rejects_an_end_marker_without_a_start_marker() -> Result<()> {
        let tmp = tempdir()?;
        let path = tmp.path().join("AGENTS.md");
        let original = format!("# T\nnotes\n{AGENTS_SECTION_END}\n");
        fs::write(&path, &original)?;

        let err = sync_agents_with_assets(&path, &[], &[]).expect_err("orphan end marker");
        assert!(err.to_string().contains("fix the markers"), "{err}");
        assert_eq!(fs::read_to_string(&path)?, original);
        Ok(())
    }

    #[test]
    fn resyncing_an_unchanged_agent_counts_it_as_skipped() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let agent_dir = claude_root.join("agents");
        fs::create_dir_all(&agent_dir)?;
        fs::write(agent_dir.join("helper.md"), "agent content")?;
        let agents_root = tmp.path().join("agents");

        sync_agents_only_from_claude(&claude_root, &agents_root, false)?;
        let again = sync_agents_only_from_claude(&claude_root, &agents_root, false)?;

        assert_eq!(again.copied, 0);
        assert_eq!(again.skipped, 1);
        assert!(again.copied_names.is_empty());
        Ok(())
    }

    /// Two files larger than one 8192-byte read buffer that differ only in
    /// their last byte, so the first buffer compares equal.
    fn same_length_pair(dir: &Path, last_src: u8, last_dest: u8) -> Result<(PathBuf, PathBuf)> {
        let mut src_bytes = vec![b'a'; 8192 + 100];
        let mut dest_bytes = src_bytes.clone();
        *src_bytes.last_mut().unwrap() = last_src;
        *dest_bytes.last_mut().unwrap() = last_dest;
        let (src, dest) = (dir.join("src.md"), dir.join("dest.md"));
        fs::write(&src, src_bytes)?;
        fs::write(&dest, dest_bytes)?;
        Ok((src, dest))
    }

    #[test]
    fn same_length_files_differing_after_the_first_buffer_need_a_copy() -> Result<()> {
        let tmp = tempdir()?;
        let (src, dest) = same_length_pair(tmp.path(), b'x', b'y')?;
        assert!(should_copy_by_content(&src, &dest)?);
        Ok(())
    }

    #[test]
    fn identical_files_larger_than_one_buffer_need_no_copy() -> Result<()> {
        let tmp = tempdir()?;
        let (src, dest) = same_length_pair(tmp.path(), b'x', b'x')?;
        assert!(!should_copy_by_content(&src, &dest)?);
        Ok(())
    }

    #[test]
    fn files_of_different_length_need_a_copy() -> Result<()> {
        let tmp = tempdir()?;
        let (src, dest) = (tmp.path().join("src.md"), tmp.path().join("dest.md"));
        fs::write(&src, "agent content")?;
        fs::write(&dest, "agent")?;
        assert!(should_copy_by_content(&src, &dest)?);
        Ok(())
    }

    #[test]
    fn sync_agents_replaces_an_existing_section_in_place() -> Result<()> {
        let tmp = tempdir()?;
        let path = tmp.path().join("AGENTS.md");
        fs::write(
            &path,
            format!("# T\n{AGENTS_SECTION_START}\nold\n{AGENTS_SECTION_END}\n# Tail\n"),
        )?;

        sync_agents_with_assets(&path, &[], &[])?;

        let text = fs::read_to_string(&path)?;
        assert!(!text.contains("\nold\n"), "{text}");
        assert_eq!(text.matches(AGENTS_SECTION_START).count(), 1);
        assert!(text.ends_with("# Tail\n"), "{text}");
        Ok(())
    }
}
