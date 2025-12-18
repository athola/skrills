//! Skill loading trace/probe helpers.
//!
//! Claude Code and Codex CLI load "skills" from directories on disk, but neither
//! client guarantees a built-in, user-visible report of which skill files were
//! actually injected into the current prompt. This module provides an opt-in,
//! deterministic validation path:
//! 1) Instrument skill files with HTML comment markers (safe for Markdown).
//! 2) Install a small "trace" skill that prints the markers it can see at the
//!    end of each assistant response.
//! 3) Install a "probe" skill that responds to a sentinel prompt, proving that
//!    skills are being loaded for the current session.

use anyhow::{anyhow, Context, Result};
use pathdiff::diff_paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// Keep this aligned with discovery's skip list to avoid crawling large dirs.
const IGNORE_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "vendor",
    ".venv",
    "__pycache__",
    ".cache",
    ".tox",
];

const TRACE_SKILL_DIR: &str = "skrills-skill-trace";
const PROBE_SKILL_DIR: &str = "skrills-skill-probe";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ClientTarget {
    Claude,
    Codex,
    #[default]
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceInstallOptions {
    pub instrument: bool,
    pub backup: bool,
    pub dry_run: bool,
    pub include_mirror: bool,
    pub include_agent: bool,
    pub include_cache: bool,
    pub include_marketplace: bool,
}

impl Default for TraceInstallOptions {
    fn default() -> Self {
        Self {
            instrument: true,
            backup: true,
            dry_run: false,
            include_mirror: true,
            include_agent: true,
            include_cache: false,
            include_marketplace: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraceInstallReport {
    pub target: ClientTarget,
    pub installed_trace_skill: bool,
    pub installed_probe_skill: bool,
    pub instrumented_files: usize,
    pub skipped_files: usize,
    pub roots: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillLoadingStatus {
    pub target: ClientTarget,
    pub roots: Vec<String>,
    pub skill_files_found: usize,
    pub trace_skill_installed: bool,
    pub probe_skill_installed: bool,
    pub instrumented_markers_found: usize,
    /// Warnings encountered during status check (e.g., permission errors).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn trace_skill_content() -> String {
    // Codex requires frontmatter. Keep the output footer very stable so it can
    // be grepped/parsed by humans and tooling.
    [
        "---",
        "name: skrills-skill-trace",
        "description: Debug-only skill that reports which instrumented skills are visible in the current prompt.",
        "---",
        "",
        "# Skrills Skill Trace (debug)",
        "",
        "This skill is for debugging only.",
        "",
        "At the end of **every** assistant response:",
        "1) Scan your entire context (including loaded skills) for HTML comments that look like:",
        "   `<!-- skrills-skill-id: ... -->`",
        "2) Print a footer in this exact format (and do not add extra commentary in the footer):",
        "",
        "SKRILLS_SKILLS_LOADED: <JSON array of strings>",
        "SKRILLS_SKILLS_USED: <JSON array of strings>",
        "",
        "If you cannot find any markers, still print:",
        "SKRILLS_SKILLS_LOADED: []",
        "SKRILLS_SKILLS_USED: []",
        "and add a second line:",
        "SKRILLS_SKILLS_TRACE_WARNING: \"no markers found (skills may not be instrumented)\"",
        "",
        "`SKRILLS_SKILLS_USED` should be a subset of `SKRILLS_SKILLS_LOADED` based on what you actually used to produce the answer. If uncertain, return the full loaded list.",
        "",
        "Do not include markdown formatting in the footer.",
        "",
    ]
    .join("\n")
}

fn probe_skill_content() -> String {
    [
        "---",
        "name: skrills-skill-probe",
        "description: Debug-only probe. Responds to SKRILLS_PROBE:<token> to prove skills are loading.",
        "---",
        "",
        "# Skrills Skill Probe (debug)",
        "",
        "If the user message contains a single line of the form:",
        "`SKRILLS_PROBE:<token>`",
        "",
        "then respond with exactly:",
        "`SKRILLS_PROBE_OK:<token>`",
        "",
        "Do not include any other text in that response.",
        "",
    ]
    .join("\n")
}

fn client_skill_dir(home: &Path, client: ClientTarget) -> Option<PathBuf> {
    match client {
        ClientTarget::Claude => Some(home.join(".claude/skills")),
        ClientTarget::Codex => Some(home.join(".codex/skills")),
        ClientTarget::Both => None,
    }
}

fn roots_for_target(
    home: &Path,
    target: ClientTarget,
    opts: &TraceInstallOptions,
) -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();

    let push_root = |roots: &mut Vec<(String, PathBuf)>, label: &str, path: PathBuf| {
        roots.push((label.to_string(), path));
    };

    match target {
        ClientTarget::Claude => {
            push_root(&mut roots, "claude", home.join(".claude/skills"));
            if opts.include_cache {
                push_root(&mut roots, "cache", home.join(".claude/plugins/cache"));
            }
            if opts.include_marketplace {
                push_root(
                    &mut roots,
                    "marketplace",
                    home.join(".claude/plugins/marketplaces"),
                );
            }
            if opts.include_agent {
                push_root(&mut roots, "agent", home.join(".agent/skills"));
            }
        }
        ClientTarget::Codex => {
            push_root(&mut roots, "codex", home.join(".codex/skills"));
            if opts.include_mirror {
                push_root(&mut roots, "mirror", home.join(".codex/skills-mirror"));
            }
            if opts.include_agent {
                push_root(&mut roots, "agent", home.join(".agent/skills"));
            }
        }
        ClientTarget::Both => {
            let mut claude = roots_for_target(home, ClientTarget::Claude, opts);
            let mut codex = roots_for_target(home, ClientTarget::Codex, opts);
            roots.append(&mut claude);
            roots.append(&mut codex);
        }
    }

    roots
}

fn is_skill_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("SKILL.md"))
        .unwrap_or(false)
}

fn is_internal_skrills_skill(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == TRACE_SKILL_DIR || s == PROBE_SKILL_DIR
    })
}

fn ensure_marker(content: &str, marker_line: &str) -> (String, bool) {
    if content.contains(marker_line) {
        return (content.to_string(), false);
    }
    let mut out = content.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(marker_line);
    out.push('\n');
    (out, true)
}

fn write_with_optional_backup(
    path: &Path,
    content: &str,
    backup: bool,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        return Ok(());
    }
    if backup && path.exists() {
        let backup_path = path.with_extension("md.bak");
        if !backup_path.exists() {
            fs::copy(path, &backup_path).with_context(|| {
                format!(
                    "failed to create backup {} from {}",
                    backup_path.display(),
                    path.display()
                )
            })?;
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create dir {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn instrument_root(
    root_label: &str,
    root: &Path,
    opts: &TraceInstallOptions,
    report: &mut TraceInstallReport,
) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    const MAX_DEPTH: usize = 20;
    for entry_result in WalkDir::new(root)
        .min_depth(1)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                return !IGNORE_DIRS.iter().any(|d| name == *d);
            }
            true
        })
    {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                report.warnings.push(format!(
                    "failed to access entry in {}: {}",
                    root.display(),
                    e
                ));
                continue;
            }
        };
        let path = entry.path();
        if !entry.file_type().is_file() || !is_skill_file(path) {
            continue;
        }
        if is_internal_skrills_skill(path) {
            report.skipped_files += 1;
            continue;
        }

        let rel = diff_paths(path, root)
            .and_then(|p| p.to_str().map(|s| s.to_owned()))
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let marker_line = format!("<!-- skrills-skill-id: {}:{} -->", root_label, rel);

        let original = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                report
                    .warnings
                    .push(format!("failed to read {}: {e}", path.display()));
                continue;
            }
        };
        let (updated, modified) = ensure_marker(&original, &marker_line);
        if !modified {
            report.skipped_files += 1;
            continue;
        }
        write_with_optional_backup(path, &updated, opts.backup, opts.dry_run)?;
        report.instrumented_files += 1;
    }
    Ok(())
}

fn install_skill(
    home: &Path,
    client: ClientTarget,
    dir_name: &str,
    content: &str,
    opts: &TraceInstallOptions,
) -> Result<bool> {
    let Some(skills_dir) = client_skill_dir(home, client) else {
        return Err(anyhow!("invalid client for install: {client:?}"));
    };
    let path = skills_dir.join(dir_name).join("SKILL.md");
    if path.exists() {
        if let Ok(existing) = fs::read_to_string(&path) {
            if existing == content {
                return Ok(false);
            }
        }
        write_with_optional_backup(&path, content, opts.backup, opts.dry_run)?;
        return Ok(true);
    }
    write_with_optional_backup(&path, content, false, opts.dry_run)?;
    Ok(true)
}

pub fn ensure_probe(home: &Path, target: ClientTarget, dry_run: bool) -> Result<bool> {
    let opts = TraceInstallOptions {
        instrument: false,
        backup: false,
        dry_run,
        ..Default::default()
    };
    match target {
        ClientTarget::Claude => install_skill(
            home,
            ClientTarget::Claude,
            PROBE_SKILL_DIR,
            &probe_skill_content(),
            &opts,
        ),
        ClientTarget::Codex => install_skill(
            home,
            ClientTarget::Codex,
            PROBE_SKILL_DIR,
            &probe_skill_content(),
            &opts,
        ),
        ClientTarget::Both => {
            let a = install_skill(
                home,
                ClientTarget::Claude,
                PROBE_SKILL_DIR,
                &probe_skill_content(),
                &opts,
            )?;
            let b = install_skill(
                home,
                ClientTarget::Codex,
                PROBE_SKILL_DIR,
                &probe_skill_content(),
                &opts,
            )?;
            Ok(a || b)
        }
    }
}

#[allow(dead_code)]
pub fn ensure_trace_skill(home: &Path, target: ClientTarget, dry_run: bool) -> Result<bool> {
    let opts = TraceInstallOptions {
        instrument: false,
        backup: false,
        dry_run,
        ..Default::default()
    };
    match target {
        ClientTarget::Claude => install_skill(
            home,
            ClientTarget::Claude,
            TRACE_SKILL_DIR,
            &trace_skill_content(),
            &opts,
        ),
        ClientTarget::Codex => install_skill(
            home,
            ClientTarget::Codex,
            TRACE_SKILL_DIR,
            &trace_skill_content(),
            &opts,
        ),
        ClientTarget::Both => {
            let a = install_skill(
                home,
                ClientTarget::Claude,
                TRACE_SKILL_DIR,
                &trace_skill_content(),
                &opts,
            )?;
            let b = install_skill(
                home,
                ClientTarget::Codex,
                TRACE_SKILL_DIR,
                &trace_skill_content(),
                &opts,
            )?;
            Ok(a || b)
        }
    }
}

pub fn enable_trace(
    home: &Path,
    target: ClientTarget,
    opts: TraceInstallOptions,
) -> Result<TraceInstallReport> {
    let mut report = TraceInstallReport {
        target,
        ..Default::default()
    };

    match target {
        ClientTarget::Claude => {
            report.installed_trace_skill = install_skill(
                home,
                ClientTarget::Claude,
                TRACE_SKILL_DIR,
                &trace_skill_content(),
                &opts,
            )?;
            report.installed_probe_skill = install_skill(
                home,
                ClientTarget::Claude,
                PROBE_SKILL_DIR,
                &probe_skill_content(),
                &opts,
            )?;
        }
        ClientTarget::Codex => {
            report.installed_trace_skill = install_skill(
                home,
                ClientTarget::Codex,
                TRACE_SKILL_DIR,
                &trace_skill_content(),
                &opts,
            )?;
            report.installed_probe_skill = install_skill(
                home,
                ClientTarget::Codex,
                PROBE_SKILL_DIR,
                &probe_skill_content(),
                &opts,
            )?;
        }
        ClientTarget::Both => {
            let a = install_skill(
                home,
                ClientTarget::Claude,
                TRACE_SKILL_DIR,
                &trace_skill_content(),
                &opts,
            )?;
            let b = install_skill(
                home,
                ClientTarget::Codex,
                TRACE_SKILL_DIR,
                &trace_skill_content(),
                &opts,
            )?;
            report.installed_trace_skill = a || b;
            let c = install_skill(
                home,
                ClientTarget::Claude,
                PROBE_SKILL_DIR,
                &probe_skill_content(),
                &opts,
            )?;
            let d = install_skill(
                home,
                ClientTarget::Codex,
                PROBE_SKILL_DIR,
                &probe_skill_content(),
                &opts,
            )?;
            report.installed_probe_skill = c || d;
        }
    }

    let roots = roots_for_target(home, target, &opts);
    report.roots = roots
        .iter()
        .map(|(label, path)| format!("{label}:{}", path.display()))
        .collect();

    if opts.instrument {
        for (label, root) in roots {
            instrument_root(&label, &root, &opts, &mut report)?;
        }
    }

    Ok(report)
}

pub fn disable_trace(home: &Path, target: ClientTarget, dry_run: bool) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    let remove_skill_dir = |removed: &mut Vec<String>, dir: &Path, dry_run: bool| -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }
        if dry_run {
            removed.push(format!("(dry-run) {}", dir.display()));
            return Ok(());
        }
        fs::remove_dir_all(dir).with_context(|| format!("failed to remove {}", dir.display()))?;
        removed.push(dir.display().to_string());
        Ok(())
    };

    let do_client =
        |client: ClientTarget, removed: &mut Vec<String>, dry_run: bool| -> Result<()> {
            let Some(skills_dir) = client_skill_dir(home, client) else {
                return Ok(());
            };
            remove_skill_dir(removed, &skills_dir.join(TRACE_SKILL_DIR), dry_run)?;
            remove_skill_dir(removed, &skills_dir.join(PROBE_SKILL_DIR), dry_run)?;
            Ok(())
        };

    match target {
        ClientTarget::Claude => do_client(ClientTarget::Claude, &mut removed, dry_run)?,
        ClientTarget::Codex => do_client(ClientTarget::Codex, &mut removed, dry_run)?,
        ClientTarget::Both => {
            do_client(ClientTarget::Claude, &mut removed, dry_run)?;
            do_client(ClientTarget::Codex, &mut removed, dry_run)?;
        }
    }

    Ok(removed)
}

pub fn status(
    home: &Path,
    target: ClientTarget,
    opts: &TraceInstallOptions,
) -> Result<SkillLoadingStatus> {
    let roots = roots_for_target(home, target, opts);
    let roots_str = roots
        .iter()
        .map(|(label, path)| format!("{label}:{}", path.display()))
        .collect::<Vec<_>>();

    let trace_installed = match target {
        ClientTarget::Claude => home
            .join(".claude/skills")
            .join(TRACE_SKILL_DIR)
            .join("SKILL.md")
            .exists(),
        ClientTarget::Codex => home
            .join(".codex/skills")
            .join(TRACE_SKILL_DIR)
            .join("SKILL.md")
            .exists(),
        ClientTarget::Both => {
            home.join(".claude/skills")
                .join(TRACE_SKILL_DIR)
                .join("SKILL.md")
                .exists()
                || home
                    .join(".codex/skills")
                    .join(TRACE_SKILL_DIR)
                    .join("SKILL.md")
                    .exists()
        }
    };

    let probe_installed = match target {
        ClientTarget::Claude => home
            .join(".claude/skills")
            .join(PROBE_SKILL_DIR)
            .join("SKILL.md")
            .exists(),
        ClientTarget::Codex => home
            .join(".codex/skills")
            .join(PROBE_SKILL_DIR)
            .join("SKILL.md")
            .exists(),
        ClientTarget::Both => {
            home.join(".claude/skills")
                .join(PROBE_SKILL_DIR)
                .join("SKILL.md")
                .exists()
                || home
                    .join(".codex/skills")
                    .join(PROBE_SKILL_DIR)
                    .join("SKILL.md")
                    .exists()
        }
    };

    let mut skill_files_found = 0usize;
    let mut markers_found = 0usize;
    let mut warnings = Vec::new();

    const MAX_DEPTH: usize = 20;
    for (_label, root) in roots {
        if !root.exists() {
            continue;
        }
        for entry_result in WalkDir::new(&root)
            .min_depth(1)
            .max_depth(MAX_DEPTH)
            .into_iter()
            .filter_entry(|e| {
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    return !IGNORE_DIRS.iter().any(|d| name == *d);
                }
                true
            })
        {
            let entry = match entry_result {
                Ok(e) => e,
                Err(e) => {
                    warnings.push(format!(
                        "failed to access entry in {}: {}",
                        root.display(),
                        e
                    ));
                    continue;
                }
            };
            let path = entry.path();
            if !entry.file_type().is_file() || !is_skill_file(path) {
                continue;
            }
            skill_files_found += 1;
            if let Ok(content) = fs::read_to_string(path) {
                if content.contains("<!-- skrills-skill-id:") {
                    markers_found += 1;
                }
            }
        }
    }

    Ok(SkillLoadingStatus {
        target,
        roots: roots_str,
        skill_files_found,
        trace_skill_installed: trace_installed,
        probe_skill_installed: probe_installed,
        instrumented_markers_found: markers_found,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_skill(root: &Path, rel_dir: &str, content: &str) {
        let path = root.join(rel_dir).join("SKILL.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn enable_trace_instruments_and_is_idempotent() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        write_skill(
            &home.join(".codex/skills"),
            "alpha",
            "---\nname: alpha\ndescription: a\n---\nbody",
        );

        let opts = TraceInstallOptions {
            instrument: true,
            backup: true,
            dry_run: false,
            include_mirror: false,
            include_agent: false,
            include_cache: false,
            include_marketplace: false,
        };

        let report = enable_trace(home, ClientTarget::Codex, opts.clone()).unwrap();
        assert!(report.installed_trace_skill);
        assert!(report.installed_probe_skill);
        assert!(report.instrumented_files >= 1);

        let content = fs::read_to_string(home.join(".codex/skills/alpha/SKILL.md")).unwrap();
        assert!(content.contains("<!-- skrills-skill-id: codex:alpha/SKILL.md -->"));

        // Second run should not duplicate markers and should not reinstall skills.
        let report2 = enable_trace(home, ClientTarget::Codex, opts).unwrap();
        assert!(!report2.installed_trace_skill);
        assert!(!report2.installed_probe_skill);

        let content2 = fs::read_to_string(home.join(".codex/skills/alpha/SKILL.md")).unwrap();
        assert_eq!(
            content2
                .matches("<!-- skrills-skill-id: codex:alpha/SKILL.md -->")
                .count(),
            1
        );
    }

    #[test]
    fn status_counts_markers() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();
        write_skill(&home.join(".claude/skills"), "beta", "# beta");
        write_skill(
            &home.join(".claude/skills"),
            "gamma",
            "# gamma\n<!-- skrills-skill-id: claude:gamma/SKILL.md -->\n",
        );

        let st = status(home, ClientTarget::Claude, &TraceInstallOptions::default()).unwrap();
        assert_eq!(st.skill_files_found, 2);
        assert_eq!(st.instrumented_markers_found, 1);
    }
}
