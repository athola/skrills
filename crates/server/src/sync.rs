//! Skill synchronization and AGENTS.md management.
//!
//! This module handles:
//! - Synchronizing skills from `~/.claude` to `~/.codex/skills` (Codex discovery root).
//! - Generating and updating `AGENTS.md` with available skills.

use anyhow::Result;
use skrills_discovery::{AgentMeta, SkillMeta};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use std::collections::{HashSet, VecDeque};
use std::io::{BufReader, Read};

use crate::discovery::{
    collect_agents, collect_skills, is_skill_file, relative_path, AGENTS_AGENT_SECTION_END,
    AGENTS_AGENT_SECTION_START, AGENTS_SECTION_END, AGENTS_SECTION_START, AGENTS_TEXT,
};

fn is_hidden_rel_path(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        std::path::Component::Normal(s) => s.to_string_lossy().starts_with('.'),
        _ => false,
    })
}

fn is_external_link_target(raw: &str) -> bool {
    let t = raw.trim();
    t.starts_with('#')
        || t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("mailto:")
        || t.starts_with("tel:")
        || t.starts_with("data:")
        || t.starts_with("//")
}

fn normalize_link_target(raw: &str) -> Option<String> {
    let mut t = raw.trim();
    if t.is_empty() || is_external_link_target(t) {
        return None;
    }
    if t.starts_with('<') && t.ends_with('>') && t.len() >= 2 {
        t = &t[1..t.len() - 1];
    }
    let t = t.split_whitespace().next().unwrap_or("").trim();
    let t = t.split('#').next().unwrap_or("").trim();
    let t = t.split('?').next().unwrap_or("").trim();
    if t.is_empty() || is_external_link_target(t) {
        return None;
    }
    Some(t.to_string())
}

fn extract_markdown_link_targets(markdown: &str) -> Vec<String> {
    let bytes = markdown.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                j += 1;
                let start = j;
                let mut depth = 1i32;
                while j < bytes.len() {
                    match bytes[j] {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                let raw = &markdown[start..j];
                                if let Some(t) = normalize_link_target(raw) {
                                    out.push(t);
                                }
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out.sort();
    out.dedup();
    out
}

fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = out
                    .components()
                    .next_back()
                    .is_some_and(|last| matches!(last, Component::Normal(_)));
                if popped {
                    out.pop();
                }
            }
            Component::Normal(s) => out.push(s),
        }
    }
    out
}

fn resolve_under_root(root: &Path, base_dir: &Path, link: &str) -> Option<(PathBuf, PathBuf)> {
    let link_path = Path::new(link);
    let abs_raw = if link_path.is_absolute() {
        link_path.to_path_buf()
    } else {
        base_dir.join(link_path)
    };
    let abs = normalize_path(&abs_raw);
    let rel = abs.strip_prefix(root).ok()?.to_path_buf();
    if is_hidden_rel_path(&rel) {
        return None;
    }
    Some((abs, rel))
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

/// Most files a single SKILL.md may pull in through its links, counting both
/// the markdown it follows and every file it copies.
const MAX_LINKED_FILES: usize = 200;

/// Largest linked file that is mirrored; bigger targets are skipped.
const MAX_LINKED_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Mirrors the files a SKILL.md links to, following links in linked markdown.
///
/// A target is mirrored only when it is a regular file whose canonical path
/// stays under `source_root`, so a link through a symlinked directory cannot
/// reach outside it. `max_files` bounds every file processed and
/// `max_file_bytes` skips oversized targets.
fn mirror_linked_files_transitively(
    source_root: &Path,
    dest_root: &Path,
    skill_md_src: &Path,
    max_files: usize,
    max_file_bytes: u64,
) -> Result<()> {
    let Ok(root_canon) = source_root.canonicalize() else {
        return Ok(());
    };
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut copied: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(skill_md_src.to_path_buf());

    let budget_left = |visited: &HashSet<PathBuf>, copied: &HashSet<PathBuf>| {
        visited.len() + copied.len() < max_files
    };

    while let Some(path) = queue.pop_front() {
        if !budget_left(&visited, &copied) {
            break;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            continue;
        }
        let content = match fs::read(&path) {
            Ok(b) => String::from_utf8_lossy(&b).to_string(),
            Err(_) => continue,
        };
        let targets = extract_markdown_link_targets(&content);
        let base_dir = match path.parent() {
            Some(p) => p,
            None => continue,
        };
        for t in targets {
            if !budget_left(&visited, &copied) {
                break;
            }
            let (abs, rel) = match resolve_under_root(source_root, base_dir, &t) {
                Some(v) => v,
                None => continue,
            };
            let meta = match fs::symlink_metadata(&abs) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() || !meta.is_file() {
                continue;
            }
            // A symlinked directory earlier in the path passes the checks
            // above; the canonical path shows where the file really lives.
            match abs.canonicalize() {
                Ok(canon) if canon.starts_with(&root_canon) => {}
                _ => {
                    tracing::warn!(
                        target: "skrills::sync",
                        link = %abs.display(),
                        "Skipped a linked file that resolves outside the mirror source"
                    );
                    continue;
                }
            }
            if meta.len() > max_file_bytes {
                tracing::warn!(
                    target: "skrills::sync",
                    link = %abs.display(),
                    bytes = meta.len(),
                    "Skipped a linked file over the size limit"
                );
                continue;
            }
            if copied.insert(abs.clone()) {
                let _ = mirror_path_if_changed(&abs, dest_root, &rel)?;
            }
            if abs.extension().is_some_and(|e| e == "md") {
                queue.push_back(abs);
            }
        }
    }
    Ok(())
}

/// Reports the outcome of mirroring skills or agents into a Codex root.
///
/// Named apart from [`skrills_sync::SyncReport`], which the same call sites
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

/// Synchronizes only SKILL.md-based skills into a Codex skills root (e.g., `~/.codex/skills`).
///
/// - copies only `SKILL.md` files, their adjacent supporting files and the
///   files they link to under `claude_root`
/// - skips hidden entries and symlinks to match Codex discovery behavior
pub fn sync_skills_only_from_claude(
    claude_root: &Path,
    codex_skills_root: &Path,
    include_marketplace: bool,
) -> Result<MirrorReport> {
    let mut report = MirrorReport::default();
    if !claude_root.exists() {
        return Ok(report);
    }
    let mut mirrored_dirs: HashSet<PathBuf> = HashSet::new();
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
            if !include_marketplace && rel.starts_with("plugins/marketplaces") {
                return false;
            }
            true
        })
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_symlink() {
            continue;
        }
        let is_skill = is_skill_file(&entry);
        if !is_skill {
            continue;
        }
        let src = entry.into_path();
        let rel = relative_path(claude_root, &src).unwrap_or_else(|| src.clone());
        let copied_skill = mirror_path_if_changed(&src, codex_skills_root, &rel)?;
        if copied_skill {
            report.copied += 1;
            if let Some(rel_path) = relative_path(claude_root, &src) {
                let skill_name = rel_path
                    .parent()
                    .and_then(|p| p.to_str())
                    .unwrap_or_else(|| rel_path.to_str().unwrap_or("unknown"));
                report.copied_names.push(skill_name.to_string());
            }
        } else {
            report.skipped += 1;
        }

        // Mirror supporting files alongside the SKILL.md into the Codex skills tree.
        if let Some(skill_dir) = src.parent() {
            let rel_dir =
                relative_path(claude_root, skill_dir).unwrap_or_else(|| skill_dir.to_path_buf());
            if mirrored_dirs.insert(rel_dir.clone()) {
                for file in WalkDir::new(skill_dir)
                    .min_depth(1)
                    .max_depth(20)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if file.file_type().is_symlink() {
                        continue;
                    }
                    if file.file_type().is_dir() {
                        continue;
                    }
                    let file_src = file.path();
                    if file_src.file_name().is_some_and(|n| n == "SKILL.md") {
                        continue;
                    }
                    let file_rel = relative_path(claude_root, file_src)
                        .unwrap_or_else(|| file_src.to_path_buf());
                    if is_hidden_rel_path(&file_rel) {
                        continue;
                    }
                    let _ = mirror_path_if_changed(file_src, codex_skills_root, &file_rel)?;
                }
            }
        } else {
            // No parent directory; nothing else to mirror.
        }

        // Mirror linked files referenced from this SKILL.md (transitively).
        mirror_linked_files_transitively(
            claude_root,
            codex_skills_root,
            &src,
            MAX_LINKED_FILES,
            MAX_LINKED_FILE_BYTES,
        )?;
    }
    Ok(report)
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
/// This intentionally does **not** write to `~/.codex/skills-mirror`: skills are materialized
/// into `~/.codex/skills` via `sync_skills_only_from_claude`.
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
    fn sync_skills_copies_and_updates() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");
        fs::create_dir_all(claude_root.join("nested"))?;
        let skill_src = claude_root.join("nested/SKILL.md");
        fs::write(&skill_src, "v1")?;

        let report1 = sync_skills_only_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report1.copied, 1);
        let dest = mirror_root.join("nested/SKILL.md");
        assert_eq!(fs::read_to_string(&dest)?, "v1");

        std::thread::sleep(Duration::from_millis(5));
        fs::write(&skill_src, "v2")?;
        let report2 = sync_skills_only_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report2.copied, 1);
        assert_eq!(fs::read_to_string(&dest)?, "v2");
        Ok(())
    }

    #[test]
    fn sync_skills_reaches_marketplace_depth() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        // Depth: claude/plugins/marketplaces/a/plugins/b/skills/c/SKILL.md (7 levels)
        let deep_dir = claude_root.join("plugins/marketplaces/a/plugins/b/skills/c");
        fs::create_dir_all(&deep_dir)?;
        let skill_src = deep_dir.join("SKILL.md");
        fs::write(&skill_src, "deep")?;

        let report = sync_skills_only_from_claude(&claude_root, &mirror_root, true)?;
        assert_eq!(report.copied, 1);
        let dest = mirror_root.join("plugins/marketplaces/a/plugins/b/skills/c/SKILL.md");
        assert_eq!(fs::read_to_string(&dest)?, "deep");
        Ok(())
    }

    #[test]
    fn sync_skills_ignores_marketplace_when_disabled() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        let deep_dir = claude_root.join("plugins/marketplaces/a/plugins/b/skills/c");
        fs::create_dir_all(&deep_dir)?;
        let skill_src = deep_dir.join("SKILL.md");
        fs::write(&skill_src, "deep")?;

        let report = sync_skills_only_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report.copied, 0);
        Ok(())
    }

    #[test]
    fn sync_skills_reaches_cache_depth() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        // Depth in cache tree
        let deep_dir = claude_root.join("plugins/cache/x/y/z/skills/foo");
        fs::create_dir_all(&deep_dir)?;
        let skill_src = deep_dir.join("SKILL.md");
        fs::write(&skill_src, "cache-skill")?;

        let report = sync_skills_only_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report.copied, 1);
        let dest = mirror_root.join("plugins/cache/x/y/z/skills/foo/SKILL.md");
        assert_eq!(fs::read_to_string(&dest)?, "cache-skill");
        Ok(())
    }

    #[test]
    fn sync_skills_copies_supporting_files() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        let skill_dir = claude_root.join("plugins/cache/tool/skills/demo");
        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join("SKILL.md"), "skill")?;
        fs::write(skill_dir.join("helper.py"), "print('hi')")?;
        fs::write(skill_dir.join("config.json"), "{\"ok\":true}")?;

        let report = sync_skills_only_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report.copied, 1);

        let helper_dest = mirror_root.join("plugins/cache/tool/skills/demo/helper.py");
        let config_dest = mirror_root.join("plugins/cache/tool/skills/demo/config.json");
        assert!(helper_dest.exists());
        assert!(config_dest.exists());
        assert_eq!(fs::read_to_string(helper_dest)?, "print('hi')");
        assert_eq!(fs::read_to_string(config_dest)?, "{\"ok\":true}");
        Ok(())
    }

    #[test]
    fn sync_skills_updates_supporting_files_even_if_skill_unchanged() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let mirror_root = tmp.path().join("mirror");

        let skill_dir = claude_root.join("plugins/cache/tool/skills/demo");
        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join("SKILL.md"), "skill")?;
        fs::write(skill_dir.join("helper.py"), "print('v1')")?;

        let _ = sync_skills_only_from_claude(&claude_root, &mirror_root, false)?;
        let helper_dest = mirror_root.join("plugins/cache/tool/skills/demo/helper.py");
        assert_eq!(fs::read_to_string(&helper_dest)?, "print('v1')");

        // Update only the supporting file
        std::thread::sleep(Duration::from_millis(5));
        fs::write(skill_dir.join("helper.py"), "print('v2')")?;

        let report = sync_skills_only_from_claude(&claude_root, &mirror_root, false)?;
        assert_eq!(report.copied, 0, "SKILL.md unchanged");
        assert_eq!(fs::read_to_string(&helper_dest)?, "print('v2')");
        Ok(())
    }

    #[test]
    fn sync_skills_only_from_claude_copies_linked_files_across_directories() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let codex_root = tmp.path().join("codex-skills");

        // Shared doc outside the skill directory (but still under claude_root)
        let shared_dir = claude_root.join("skills/shared");
        fs::create_dir_all(&shared_dir)?;
        fs::write(shared_dir.join("common.md"), "Common doc")?;

        // Skill links to both a local file and a shared file
        let skill_dir = claude_root.join("skills/demo");
        fs::create_dir_all(&skill_dir)?;
        fs::write(
            skill_dir.join("subskill-info.md"),
            "See [common](../shared/common.md)",
        )?;
        fs::write(
            skill_dir.join("SKILL.md"),
            "See [sub](subskill-info.md) and [common](../shared/common.md)\n",
        )?;

        let report = sync_skills_only_from_claude(&claude_root, &codex_root, false)?;
        assert_eq!(report.copied, 1);

        // Adjacent file copied
        assert!(
            codex_root.join("skills/demo/subskill-info.md").exists(),
            "Expected adjacent file copied"
        );
        // Linked cross-directory file copied
        assert!(
            codex_root.join("skills/shared/common.md").exists(),
            "Expected linked cross-directory file copied"
        );
        Ok(())
    }

    /// Only the last path component was checked for a symlink, so a link
    /// through a symlinked directory reached files outside the source root.
    #[cfg(unix)]
    #[test]
    fn linked_files_through_a_symlinked_directory_are_not_mirrored() -> Result<()> {
        let tmp = tempdir()?;
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside)?;
        fs::write(outside.join("id_rsa"), "PRIVATE KEY")?;

        let claude_root = tmp.path().join("claude");
        let skill_dir = claude_root.join("skills/demo");
        fs::create_dir_all(&skill_dir)?;
        std::os::unix::fs::symlink(&outside, claude_root.join("skills/docs"))?;
        fs::write(skill_dir.join("SKILL.md"), "See [key](../docs/id_rsa)\n")?;

        let codex_root = tmp.path().join("codex-skills");
        sync_skills_only_from_claude(&claude_root, &codex_root, false)?;

        assert!(codex_root.join("skills/demo/SKILL.md").exists());
        assert!(
            !codex_root.join("skills/docs/id_rsa").exists(),
            "a file reached through a symlinked directory was mirrored"
        );
        Ok(())
    }

    /// The cap bounded only parsed markdown, so any number of non-markdown
    /// targets were copied.
    #[test]
    fn linked_file_cap_counts_every_copied_target() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let skill_dir = claude_root.join("skills/demo");
        fs::create_dir_all(&skill_dir)?;
        let mut links = String::new();
        for i in 0..10 {
            fs::write(skill_dir.join(format!("f{i}.txt")), "x")?;
            links.push_str(&format!("[f{i}](f{i}.txt)\n"));
        }
        let skill = skill_dir.join("SKILL.md");
        fs::write(&skill, links)?;

        let codex_root = tmp.path().join("codex-skills");
        mirror_linked_files_transitively(&claude_root, &codex_root, &skill, 4, u64::MAX)?;

        let copied = fs::read_dir(codex_root.join("skills/demo"))?.count();
        assert!(copied <= 4, "copied {copied} files past a cap of 4");
        Ok(())
    }

    #[test]
    fn linked_files_over_the_size_limit_are_skipped() -> Result<()> {
        let tmp = tempdir()?;
        let claude_root = tmp.path().join("claude");
        let skill_dir = claude_root.join("skills/demo");
        fs::create_dir_all(&skill_dir)?;
        fs::write(skill_dir.join("big.bin"), vec![0u8; 64])?;
        fs::write(skill_dir.join("small.txt"), "ok")?;
        let skill = skill_dir.join("SKILL.md");
        fs::write(&skill, "[big](big.bin) [small](small.txt)\n")?;

        let codex_root = tmp.path().join("codex-skills");
        mirror_linked_files_transitively(&claude_root, &codex_root, &skill, 200, 16)?;

        assert!(codex_root.join("skills/demo/small.txt").exists());
        assert!(!codex_root.join("skills/demo/big.bin").exists());
        Ok(())
    }

    /// Markers found independently were passed to `replace_range` in the
    /// wrong order and panicked.
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
