use crate::types::{
    parse_source_key, DuplicateInfo, RuleCategory, RuleMeta, SkillMeta, SkillRoot, SkillSource,
};
use crate::Result;
use blake2::digest::consts::U32;
use blake2::{Blake2b, Digest};
use pathdiff::diff_paths;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;

/// Default maximum depth for skill discovery.
pub const DEFAULT_MAX_DEPTH: usize = 20;

// Common heavy directories to skip during discovery to reduce first-run latency.
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

/// Configuration for skill discovery.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Root directories to scan for skills.
    pub roots: Vec<SkillRoot>,
    /// Cache time-to-live duration.
    pub cache_ttl_ms: Duration,
    /// Ordered list of skill sources to override default priority.
    pub priority_override: Option<Vec<SkillSource>>,
    /// Maximum directory depth for scanning.
    pub max_depth: usize,
}

impl DiscoveryConfig {
    /// Creates a new `DiscoveryConfig`.
    pub fn new(
        roots: Vec<SkillRoot>,
        cache_ttl_ms: Duration,
        priority_override: Option<Vec<SkillSource>>,
    ) -> Self {
        Self {
            roots,
            cache_ttl_ms,
            priority_override,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }

    /// Creates a new `DiscoveryConfig` with a custom max depth.
    pub fn with_max_depth(
        roots: Vec<SkillRoot>,
        cache_ttl_ms: Duration,
        priority_override: Option<Vec<SkillSource>>,
        max_depth: usize,
    ) -> Self {
        Self {
            roots,
            cache_ttl_ms,
            priority_override,
            max_depth,
        }
    }
}

/// Returns the default skill source priority.
pub fn default_priority() -> Vec<SkillSource> {
    vec![
        SkillSource::Codex,
        SkillSource::Mirror,
        SkillSource::Claude,
        SkillSource::Copilot,
        SkillSource::Cursor,
        SkillSource::Marketplace,
        SkillSource::Cache,
        SkillSource::Agent,
    ]
}

/// Returns labels for default skill sources.
pub fn priority_labels() -> Vec<String> {
    priority_labels_and_rank_map().0
}

/// Returns labels and a rank map for default skill sources.
pub fn priority_labels_and_rank_map() -> (Vec<String>, serde_json::Map<String, serde_json::Value>) {
    let labels = default_priority()
        .into_iter()
        .map(|s| s.label())
        .collect::<Vec<_>>();
    let rank_map = serde_json::Map::from_iter(
        labels
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), serde_json::json!(i + 1))),
    );
    (labels, rank_map)
}

/// Loads and parses priority override settings.
pub fn load_priority_override(
    settings: &dyn Fn() -> Result<Option<Vec<String>>>,
) -> Result<Option<Vec<SkillSource>>> {
    let Some(list) = settings()? else {
        return Ok(None);
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for key in list {
        if let Some(src) = parse_source_key(&key) {
            if seen.insert(src.label()) {
                out.push(src);
            }
        }
    }
    if out.is_empty() {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

/// Checks if a `DirEntry` is a `SKILL.md` file.
fn is_skill_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file() && entry.file_name() == "SKILL.md"
}

/// Checks if a `DirEntry` is a Cursor rule file (`.mdc` or `.md` in a rules directory).
fn is_cursor_rule_file(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_file() {
        return false;
    }
    let ext = entry
        .path()
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    ext == "mdc" || ext == "md"
}

/// Checks if a `DirEntry` is an agent file (any markdown under an `agents` directory).
fn is_agent_file(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_file() {
        return false;
    }
    if entry.path().extension().is_none_or(|ext| ext != "md") {
        return false;
    }
    entry
        .path()
        .ancestors()
        .any(|p| p.file_name().is_some_and(|n| n == "agents"))
}

type Blake2b256 = Blake2b<U32>;

/// Computes the BLAKE2b-256 hash of a file.
fn file_hash(path: &Path) -> Result<String> {
    let meta = fs::metadata(path)?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Using size + mtime gives us a cheap fingerprint without reading file contents.
    // Hash only a small prefix to stay cheap but content-sensitive.
    let mut hasher = Blake2b256::new();
    hasher.update(size.to_le_bytes());
    hasher.update(mtime.to_le_bytes());
    if size > 0 {
        use std::io::Read;
        if let Ok(mut file) = fs::File::open(path) {
            // I5 (PR-218 wave-4): saturation-guard expresses intent
            // explicitly. Pre-fix pattern was `1024.min(usize::try_from(size).unwrap_or(usize::MAX))`
            // — benign today (the outer `.min(1024)` clamps back) but a
            // refactor reordering the operands was one OOM allocation
            // away. The new shape caps at the prefix length up front.
            let prefix_len = usize::try_from(size).unwrap_or(1024).min(1024);
            let mut prefix = vec![0u8; prefix_len];
            if let Ok(n) = file.read(&mut prefix) {
                prefix.truncate(n);
                hasher.update(&prefix);
            }
        }
    }
    let digest = hasher.finalize();
    Ok(format!("{:x}", digest))
}

/// Extracts identity fields (name and description) from a skill file's YAML frontmatter.
///
/// Parses frontmatter between `---` delimiters to extract the `name` and `description` fields.
/// Returns `(None, None)` if no frontmatter is present.
fn extract_frontmatter_identity(content: &str) -> (Option<String>, Option<String>) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, None);
    }

    // Find content after opening ---
    let after_open = match trimmed.get(3..) {
        Some(s) => s.trim_start_matches(['\r', '\n']),
        None => return (None, None),
    };

    // Find closing ---
    let end_pos = match after_open
        .find("\n---")
        .or_else(|| after_open.find("\r\n---"))
    {
        Some(pos) => pos,
        None => return (None, None),
    };
    let yaml = &after_open[..end_pos];

    // Parse YAML to extract name and description fields
    // Use a minimal struct to avoid pulling in complex types
    #[derive(serde::Deserialize)]
    struct MinimalFrontmatter {
        name: Option<String>,
        description: Option<String>,
    }

    match serde_yaml::from_str::<MinimalFrontmatter>(yaml) {
        Ok(fm) => {
            let name = fm.name.filter(|n| !n.trim().is_empty());
            let description = fm.description.filter(|d| !d.trim().is_empty());
            (name, description)
        }
        Err(e) => {
            tracing::debug!(error = %e, "Failed to parse skill frontmatter");
            (None, None)
        }
    }
}

/// Computes Jaccard similarity between two strings based on whitespace-separated word sets.
///
/// Returns a value in [0.0, 1.0] where 1.0 means identical word sets.
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    use std::collections::HashSet;
    let words_a: Vec<String> = a.split_whitespace().map(|w| w.to_lowercase()).collect();
    let words_b: Vec<String> = b.split_whitespace().map(|w| w.to_lowercase()).collect();
    let set_a: HashSet<&str> = words_a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = words_b.iter().map(|s| s.as_str()).collect();
    if set_a.is_empty() && set_b.is_empty() {
        return 0.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    intersection as f64 / union as f64
}

/// Collects skill metadata from the provided roots.
fn collect_skills_from(
    roots: &[SkillRoot],
    mut dup_log: Option<&mut Vec<DuplicateInfo>>,
    max_depth: usize,
) -> Result<Vec<SkillMeta>> {
    let mut skills = Vec::new();
    let mut seen: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new(); // name -> (source, root)
                                          // Frontmatter-name-based dedup: catches duplicates across roots with different paths
                                          // but the same frontmatter identity (name + similar description).
    let mut seen_by_fm_name: std::collections::HashMap<String, (String, String, Option<String>)> =
        std::collections::HashMap::new();
    // key: lowercased frontmatter name
    // value: (source_label, root_display, description)
    for root_cfg in roots {
        let root = &root_cfg.root;
        if !root.exists() {
            continue;
        }
        let entries: Vec<_> = WalkDir::new(root)
            .min_depth(1)
            .max_depth(max_depth)
            .into_iter()
            .filter_entry(|e| {
                // SAFETY: Known TOCTOU limitation - a file could become a symlink between
                // this check and subsequent fs::read(). Mitigated by:
                // 1. WalkDir's follow_links(false) default behavior
                // 2. Skills directories are typically user-controlled, not attacker-writable
                // 3. The sanitize_name() function prevents path traversal in output paths
                // See: https://github.com/athola/skrills/issues/135
                if e.file_type().is_symlink() {
                    return false;
                }
                let name = e.file_name().to_string_lossy();
                if name.starts_with('.') {
                    return false;
                }
                if e.file_type().is_dir() {
                    return !IGNORE_DIRS.iter().any(|d| name == *d);
                }
                true
            })
            // I4 (PR-218 wave-4): walkdir errors are surfaced via
            // `tracing::warn!` so a single unreadable subdirectory
            // doesn't silently shrink the discovered skill count.
            .filter_map(|e| match e {
                Ok(entry) => Some(entry),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        root = %root.display(),
                        "skill walk error (entry skipped)"
                    );
                    None
                }
            })
            .filter(|e| {
                if root_cfg.source == SkillSource::Cursor {
                    is_cursor_rule_file(e)
                } else {
                    is_skill_file(e)
                }
            })
            .collect();

        let metas: Vec<_> = entries
            .par_iter()
            .map(|entry| {
                let path = entry.path().to_path_buf();
                let raw_name = diff_paths(&path, root)
                    .and_then(|p| p.to_str().map(|s| s.to_owned()))
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                // For Cursor .mdc files, use the file stem as the name
                let name = if root_cfg.source == SkillSource::Cursor {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&raw_name)
                        .to_string()
                } else {
                    raw_name
                };
                let hash = file_hash(&path)?;
                // Extract frontmatter identity (best-effort, log errors)
                let (frontmatter_name, description) = match fs::read_to_string(&path) {
                    Ok(content) => extract_frontmatter_identity(&content),
                    Err(e) => {
                        tracing::trace!(path = %path.display(), error = %e, "Failed to read skill file");
                        (None, None)
                    }
                };
                Ok((name, path, hash, frontmatter_name, description))
            })
            .collect::<Result<Vec<_>>>()?;

        for (name, path, hash, frontmatter_name, description) in metas {
            if let Some((seen_src, seen_root)) = seen.get(&name) {
                if let Some(dup_log) = dup_log.as_mut() {
                    dup_log.push(DuplicateInfo {
                        name: name.clone(),
                        skipped_source: root_cfg.source.label(),
                        skipped_root: root.display().to_string(),
                        kept_source: seen_src.clone(),
                        kept_root: seen_root.clone(),
                    });
                }
                continue;
            }

            // Frontmatter-name dedup: same frontmatter name + similar description = duplicate
            if let Some(ref fm_name) = frontmatter_name {
                let fm_key = fm_name.to_lowercase();
                if let Some((prev_src, prev_root, prev_desc)) = seen_by_fm_name.get(&fm_key) {
                    let is_dup = match (&description, prev_desc) {
                        (Some(a), Some(b)) => jaccard_similarity(a, b) >= 0.5,
                        _ => false, // missing description = can't compare semantically
                    };
                    if is_dup {
                        if let Some(dup_log) = dup_log.as_mut() {
                            dup_log.push(DuplicateInfo {
                                name: name.clone(),
                                skipped_source: root_cfg.source.label(),
                                skipped_root: root.display().to_string(),
                                kept_source: prev_src.clone(),
                                kept_root: prev_root.clone(),
                            });
                        }
                        continue;
                    }
                }
                seen_by_fm_name.insert(
                    fm_key,
                    (
                        root_cfg.source.label(),
                        root.display().to_string(),
                        description.clone(),
                    ),
                );
            }

            skills.push(SkillMeta {
                name: name.clone(),
                path: path.clone(),
                source: root_cfg.source.clone(),
                root: root.clone(),
                hash,
                description,
                frontmatter_name,
            });
            seen.insert(name, (root_cfg.source.label(), root.display().to_string()));
        }
    }

    // Log discovery summary for observability
    let with_description = skills.iter().filter(|s| s.description.is_some()).count();
    tracing::info!(
        total_skills = skills.len(),
        with_description,
        without_description = skills.len().saturating_sub(with_description),
        "Skill discovery complete"
    );

    Ok(skills)
}

/// Discovers skill metadata from the provided roots.
///
/// Logs duplicate skill information if `dup_log` is provided. Duplicates happen
/// when a skill with the same name exists in multiple roots; only the highest priority
/// one is kept.
///
/// Uses [`DEFAULT_MAX_DEPTH`] for directory traversal depth.
pub fn discover_skills(
    roots: &[SkillRoot],
    dup_log: Option<&mut Vec<DuplicateInfo>>,
) -> Result<Vec<SkillMeta>> {
    collect_skills_from(roots, dup_log, DEFAULT_MAX_DEPTH)
}

/// Discovers skill metadata from the provided roots with a custom depth limit.
///
/// Logs duplicate skill information if `dup_log` is provided. Duplicates happen
/// when a skill with the same name exists in multiple roots; only the highest priority
/// one is kept.
///
/// The `max_depth` parameter controls how deep the scanner will traverse into subdirectories.
pub fn discover_skills_with_depth(
    roots: &[SkillRoot],
    dup_log: Option<&mut Vec<DuplicateInfo>>,
    max_depth: usize,
) -> Result<Vec<SkillMeta>> {
    collect_skills_from(roots, dup_log, max_depth)
}

/// Collects agent metadata from the provided roots.
pub fn discover_agents(roots: &[SkillRoot]) -> Result<Vec<crate::types::AgentMeta>> {
    let mut agents = Vec::new();
    for root_cfg in roots {
        let root = &root_cfg.root;
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .min_depth(1)
            .max_depth(20)
            .into_iter()
            .filter_entry(|e| {
                // SAFETY: Known TOCTOU limitation - a file could become a symlink between
                // this check and subsequent fs::read(). Mitigated by:
                // 1. WalkDir's follow_links(false) default behavior
                // 2. Skills directories are typically user-controlled, not attacker-writable
                // 3. The sanitize_name() function prevents path traversal in output paths
                // See: https://github.com/athola/skrills/issues/135
                if e.file_type().is_symlink() {
                    return false;
                }
                let name = e.file_name().to_string_lossy();
                if name.starts_with('.') {
                    return false;
                }
                if e.file_type().is_dir() {
                    return !IGNORE_DIRS.iter().any(|d| name == *d);
                }
                true
            })
            // I4 (PR-218 wave-4): see paired note in `discover_skills`.
            .filter_map(|e| match e {
                Ok(entry) => Some(entry),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        root = %root.display(),
                        "agent walk error (entry skipped)"
                    );
                    None
                }
            })
        {
            if !is_agent_file(&entry) {
                continue;
            }
            let path = entry.into_path();
            let name = diff_paths(&path, root)
                .and_then(|p| p.to_str().map(|s| s.to_owned()))
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            let hash = file_hash(&path)?;
            agents.push(crate::types::AgentMeta {
                name,
                path: path.clone(),
                source: root_cfg.source.clone(),
                root: root.clone(),
                hash,
            });
        }
    }
    Ok(agents)
}

/// Extracts skill references from an AGENTS.md document.
///
/// Tokenizes input markdown to collect alphanumeric strings that are at least three
/// characters long and not common keywords.
pub fn extract_refs_from_agents(md: &str) -> std::collections::HashSet<String> {
    let mut refs = std::collections::HashSet::new();
    for line in md.lines() {
        for token in line.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_') {
            let t = token.trim();
            if t.is_empty() {
                continue;
            }
            if t.eq_ignore_ascii_case("skills") || t.eq_ignore_ascii_case("rules") {
                continue;
            }
            refs.insert(t.to_ascii_lowercase());
        }
    }
    refs
}

/// Returns the default skill root directories.
pub fn default_roots(home: &Path) -> Vec<SkillRoot> {
    let mut roots = vec![
        SkillRoot {
            root: home.join(".codex/skills"),
            source: SkillSource::Codex,
        },
        SkillRoot {
            root: home.join(".codex/skills-mirror"),
            source: SkillSource::Mirror,
        },
        SkillRoot {
            root: home.join(".claude/skills"),
            source: SkillSource::Claude,
        },
    ];

    // Copilot: Add both XDG and legacy paths for multi-faceted discovery.
    // XDG path: $XDG_CONFIG_HOME/copilot/skills or ~/.config/copilot/skills
    // Legacy path: ~/.copilot/skills
    if let Some(config_dir) = dirs::config_dir() {
        let xdg_copilot_skills = config_dir.join("copilot/skills");
        roots.push(SkillRoot {
            root: xdg_copilot_skills,
            source: SkillSource::Copilot,
        });
    }
    // Always include legacy path for backwards compatibility
    roots.push(SkillRoot {
        root: home.join(".copilot/skills"),
        source: SkillSource::Copilot,
    });

    roots.extend([
        SkillRoot {
            root: home.join(".claude/plugins/marketplaces"),
            source: SkillSource::Marketplace,
        },
        SkillRoot {
            root: home.join(".claude/plugins/cache"),
            source: SkillSource::Cache,
        },
        SkillRoot {
            root: home.join(".agent/skills"),
            source: SkillSource::Agent,
        },
    ]);

    roots
}

/// Builds skill roots from extra directories.
pub fn extra_skill_roots(extra: &[PathBuf]) -> Vec<SkillRoot> {
    extra
        .iter()
        .enumerate()
        .map(|(i, p)| SkillRoot {
            root: p.clone(),
            source: SkillSource::Extra(i as u32),
        })
        .collect()
}

/// Returns default skill roots using the user's home directory.
///
/// Returns an empty vector if the home directory cannot be determined.
pub fn default_roots_auto() -> Vec<SkillRoot> {
    dirs::home_dir()
        .map(|home| default_roots(&home))
        .unwrap_or_default()
}

/// Returns skill roots from custom directories, or defaults if empty.
///
/// Custom directories are assigned `SkillSource` based on path component matching:
/// - Path containing a `.claude` component → `SkillSource::Claude`
/// - Path containing a `.codex` component → `SkillSource::Codex`
/// - Path containing a `.copilot` or `copilot` component → `SkillSource::Copilot`
/// - Otherwise → `SkillSource::Extra(index)`
pub fn skill_roots_or_default(custom: &[PathBuf]) -> Vec<SkillRoot> {
    if custom.is_empty() {
        default_roots_auto()
    } else {
        custom
            .iter()
            .enumerate()
            .map(|(i, p)| {
                // Match against individual path components to avoid false positives
                // from substring matching (e.g. "/home/alice/my-codex-tools")
                let has_component = |name: &str| {
                    p.components().any(|c| {
                        c.as_os_str()
                            .to_str()
                            .is_some_and(|s| s.eq_ignore_ascii_case(name))
                    })
                };
                let source = if has_component(".claude") {
                    SkillSource::Claude
                } else if has_component(".codex") {
                    SkillSource::Codex
                } else if has_component(".copilot") || has_component("copilot") {
                    SkillSource::Copilot
                } else if has_component(".cursor") {
                    SkillSource::Cursor
                } else {
                    SkillSource::Extra(u32::try_from(i).unwrap_or(u32::MAX))
                };
                SkillRoot {
                    root: p.clone(),
                    source,
                }
            })
            .collect()
    }
}

/// Computes the BLAKE2b-256 hash of a file.
pub fn hash_file(path: &Path) -> Result<String> {
    file_hash(path)
}

/// Determines effective skill source priority.
///
/// Uses `override_order` if provided; otherwise, defaults to `default_priority`.
///
/// ```
/// use skrills_discovery::{priority_with_override, SkillSource};
///
/// let override_order = vec![SkillSource::Mirror, SkillSource::Codex];
/// let prioritized = priority_with_override(Some(override_order.clone()));
/// assert_eq!(prioritized, override_order);
///
/// let fallback = priority_with_override(None);
/// assert!(fallback.starts_with(&[SkillSource::Codex, SkillSource::Mirror]));
/// ```
pub fn priority_with_override(override_order: Option<Vec<SkillSource>>) -> Vec<SkillSource> {
    override_order.unwrap_or_else(default_priority)
}

/// Discover hookify rules from known locations.
///
/// Scans for rule configuration files in:
/// - `~/.claude/hooks/` (user-level rules)
/// - `<project>/.claude/hooks/` (project-level rules)
/// - `<project>/.hookify/` (hookify-specific rules)
pub fn discover_rules(home: &Path, project_dir: Option<&Path>) -> Vec<RuleMeta> {
    let mut rules = Vec::new();

    // Check ~/.claude/hooks/ for user-level rules
    let user_hooks = home.join(".claude").join("hooks");
    if user_hooks.is_dir() {
        scan_hooks_dir(&user_hooks, "user", &mut rules);
    }

    // Check project-level hooks
    if let Some(proj) = project_dir {
        let proj_hooks = proj.join(".claude").join("hooks");
        if proj_hooks.is_dir() {
            scan_hooks_dir(&proj_hooks, "project", &mut rules);
        }
        let hookify_dir = proj.join(".hookify");
        if hookify_dir.is_dir() {
            scan_hooks_dir(&hookify_dir, "hookify", &mut rules);
        }
    }

    rules
}

/// Scan a directory for hook/rule configuration files.
fn scan_hooks_dir(dir: &Path, source: &str, rules: &mut Vec<RuleMeta>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e == "json" || e == "yaml" || e == "yml" || e == "toml")
        {
            // Try to parse as a hook/rule definition
            if let Ok(contents) = fs::read_to_string(&path) {
                if let Some(rule) = parse_rule_file(&path, &contents, source) {
                    rules.push(rule);
                }
            }
        }
    }
}

/// Attempt to parse a rule configuration file into `RuleMeta`.
fn parse_rule_file(path: &Path, contents: &str, source: &str) -> Option<RuleMeta> {
    let name = path.file_stem()?.to_str()?.to_string();
    let category = infer_category_from_name(&name);

    // Try to extract description from file content
    let description = contents
        .lines()
        .find(|l| l.contains("description") || l.contains("# "))
        .map(|l| l.trim().trim_start_matches('#').trim().to_string());

    // Try to extract command
    let command = contents
        .lines()
        .find(|l| l.contains("command") || l.contains("run"))
        .map(|l| l.trim().to_string());

    Some(RuleMeta {
        name,
        path: path.to_path_buf(),
        source: source.to_string(),
        category,
        enabled: true, // Default to enabled
        description,
        command,
    })
}

/// Infer a `RuleCategory` from a rule file name.
fn infer_category_from_name(name: &str) -> RuleCategory {
    let lower = name.to_lowercase();
    if lower.contains("pre-commit") || lower.contains("precommit") {
        RuleCategory::PreCommit
    } else if lower.contains("post-commit") || lower.contains("postcommit") {
        RuleCategory::PostCommit
    } else if lower.contains("pre-push") || lower.contains("prepush") {
        RuleCategory::PrePush
    } else if lower.contains("prompt") || lower.contains("submit") {
        RuleCategory::PromptSubmit
    } else if lower.contains("notif") {
        RuleCategory::Notification
    } else {
        RuleCategory::Other(name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_extract_refs() {
        let md = "foo SKILL.md bar rules baz";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("foo"));
        assert!(!refs.contains("rules"));
    }

    #[test]
    fn default_roots_use_priority_order() {
        let tmp = tempdir().unwrap();
        let roots = default_roots(tmp.path());
        let labels: Vec<_> = roots.iter().map(|r| r.source.label()).collect();
        // Note: "copilot" appears twice - once for XDG path (~/.config/copilot/skills)
        // and once for legacy path (~/.copilot/skills) for multi-faceted discovery
        assert_eq!(
            labels,
            vec![
                "codex",
                "mirror",
                "claude",
                "copilot", // XDG path
                "copilot", // Legacy path
                "marketplace",
                "cache",
                "agent"
            ]
        );
    }

    #[test]
    fn extra_skill_roots_preserve_input_order() {
        let one = PathBuf::from("/tmp/one");
        let two = PathBuf::from("/tmp/two");
        let roots = extra_skill_roots(&[one.clone(), two.clone()]);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].root, one);
        assert_eq!(roots[1].root, two);
        assert!(matches!(roots[0].source, SkillSource::Extra(0)));
        assert!(matches!(roots[1].source, SkillSource::Extra(1)));
    }

    #[test]
    fn discover_skills_errors_on_unreadable_file() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir().unwrap();
        let root = tmp.path().join("codex");
        fs::create_dir_all(&root).unwrap();
        let skill = root.join("SKILL.md");
        fs::write(&skill, "secret").unwrap();
        let mut perms = fs::metadata(&skill).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&skill, perms).unwrap();

        let roots = vec![SkillRoot {
            root: root.clone(),
            source: SkillSource::Codex,
        }];
        let skills = discover_skills(&roots, None).unwrap();
        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn test_priority_with_override_uses_override() {
        let override_order = Some(vec![SkillSource::Claude, SkillSource::Codex]);
        let result = priority_with_override(override_order);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], SkillSource::Claude);
        assert_eq!(result[1], SkillSource::Codex);
    }

    #[test]
    fn test_priority_with_override_uses_default_when_none() {
        let result = priority_with_override(None);
        assert_eq!(result, default_priority());
    }

    #[test]
    fn test_load_priority_override_empty_returns_none() {
        let settings = || Ok(None);
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_priority_override_parses_valid_keys() {
        let settings = || Ok(Some(vec!["codex".to_string(), "claude".to_string()]));
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_some());
        let order = result.unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], SkillSource::Codex);
        assert_eq!(order[1], SkillSource::Claude);
    }

    #[test]
    fn test_load_priority_override_deduplicates() {
        let settings = || {
            Ok(Some(vec![
                "codex".to_string(),
                "codex".to_string(),
                "claude".to_string(),
            ]))
        };
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_some());
        let order = result.unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], SkillSource::Codex);
        assert_eq!(order[1], SkillSource::Claude);
    }

    #[test]
    fn test_load_priority_override_ignores_invalid_keys() {
        let settings = || {
            Ok(Some(vec![
                "codex".to_string(),
                "invalid-key".to_string(),
                "claude".to_string(),
            ]))
        };
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_some());
        let order = result.unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], SkillSource::Codex);
        assert_eq!(order[1], SkillSource::Claude);
    }

    #[test]
    fn test_load_priority_override_all_invalid_returns_none() {
        let settings = || Ok(Some(vec!["invalid1".to_string(), "invalid2".to_string()]));
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_refs_from_agents_filters_skills_keyword() {
        let md = "Use rust_testing skill for testing";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("use"));
        assert!(refs.contains("rust_testing"));
        assert!(refs.contains("testing"));
        assert!(!refs.contains("skills"));
    }

    #[test]
    fn test_extract_refs_from_agents_filters_rules_keyword() {
        let md = "Follow the rules for coding";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("follow"));
        assert!(refs.contains("coding"));
        assert!(!refs.contains("rules"));
    }

    #[test]
    fn test_extract_refs_from_agents_handles_multiline() {
        let md = "Line one\nLine two with python\nLine three";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("line"));
        assert!(refs.contains("one"));
        assert!(refs.contains("two"));
        assert!(refs.contains("python"));
        assert!(refs.contains("three"));
    }

    #[test]
    fn test_extract_refs_from_agents_handles_special_chars() {
        let md = "test-case, foo_bar; baz:qux";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("test-case"));
        assert!(refs.contains("foo_bar"));
        assert!(refs.contains("baz"));
        assert!(refs.contains("qux"));
    }

    #[test]
    fn test_discover_skills_with_duplicates() {
        let tmp = tempdir().unwrap();

        let codex_root = tmp.path().join("codex");
        fs::create_dir_all(codex_root.join("test-skill")).unwrap();
        fs::write(codex_root.join("test-skill/SKILL.md"), "codex version").unwrap();

        let claude_root = tmp.path().join("claude");
        fs::create_dir_all(claude_root.join("test-skill")).unwrap();
        fs::write(claude_root.join("test-skill/SKILL.md"), "claude version").unwrap();

        let roots = vec![
            SkillRoot {
                root: codex_root,
                source: SkillSource::Codex,
            },
            SkillRoot {
                root: claude_root,
                source: SkillSource::Claude,
            },
        ];

        let mut dup_log = vec![];
        let skills = discover_skills(&roots, Some(&mut dup_log)).unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Codex);

        assert_eq!(dup_log.len(), 1);
        assert_eq!(dup_log[0].name, "test-skill/SKILL.md");
        assert_eq!(dup_log[0].kept_source, "codex");
        assert_eq!(dup_log[0].skipped_source, "claude");
    }

    #[test]
    fn test_discover_skills_max_depth_limit() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");

        let deep_path = root.join("a/b/c/d/e/f/g");
        fs::create_dir_all(&deep_path).unwrap();
        fs::write(deep_path.join("SKILL.md"), "deep skill").unwrap();

        let shallow_path = root.join("shallow");
        fs::create_dir_all(&shallow_path).unwrap();
        fs::write(shallow_path.join("SKILL.md"), "shallow skill").unwrap();

        // Path deliberately deeper than MAX_SKILL_DEPTH to ensure it is ignored.
        let too_deep_path = root.join(
            [
                "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9", "d10", "d11", "d12", "d13",
                "d14", "d15", "d16", "d17", "d18", "d19", "d20", "d21",
            ]
            .iter()
            .collect::<PathBuf>(),
        );
        fs::create_dir_all(&too_deep_path).unwrap();
        fs::write(too_deep_path.join("SKILL.md"), "too deep").unwrap();

        let roots = vec![SkillRoot {
            root,
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None).unwrap();

        assert_eq!(skills.len(), 2);
        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("shallow")));
        assert!(names.iter().any(|n| n.contains("a/b/c/d/e/f/g")));
        assert!(!names.iter().any(|n| n.contains("d21")));
    }

    #[test]
    fn test_discover_skills_ignores_directories() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");
        fs::create_dir_all(&root).unwrap();

        fs::create_dir_all(root.join("SKILL.md")).unwrap();

        let skill_dir = root.join("real-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "real skill").unwrap();

        let roots = vec![SkillRoot {
            root,
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None).unwrap();

        assert_eq!(skills.len(), 1);
        assert!(skills[0].name.contains("real-skill"));
    }

    #[test]
    fn test_hash_file_consistent() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("test.md");
        fs::write(&file, "test content").unwrap();

        let hash1 = hash_file(&file).unwrap();
        let hash2 = hash_file(&file).unwrap();

        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }

    #[test]
    fn hash_file_uses_blake2b_256_length() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("len.md");
        fs::write(&file, "len").unwrap();

        let hash = hash_file(&file).unwrap();
        assert_eq!(hash.len(), 64, "BLAKE2b-256 hex is 64 chars");
    }

    #[test]
    fn test_hash_file_different_content() {
        let tmp = tempdir().unwrap();

        let file1 = tmp.path().join("test1.md");
        fs::write(&file1, "content 1").unwrap();

        let file2 = tmp.path().join("test2.md");
        fs::write(&file2, "content 2").unwrap();

        let hash1 = hash_file(&file1).unwrap();
        let hash2 = hash_file(&file2).unwrap();

        assert!(!hash1.is_empty());
        assert!(!hash2.is_empty());
    }

    #[test]
    fn hash_file_detects_same_size_content_change() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("same_size.md");
        fs::write(&file, "abcd1234").unwrap();
        let hash1 = hash_file(&file).unwrap();

        // Overwrite with same length content but different bytes.
        fs::write(&file, "wxyz5678").unwrap();
        // Ensure mtime advances for filesystems with coarse resolution.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let hash2 = hash_file(&file).unwrap();

        assert_ne!(hash1, hash2, "hash should change when content changes");
    }

    #[test]
    fn test_priority_labels() {
        let labels = priority_labels();
        assert_eq!(
            labels,
            vec![
                "codex",
                "mirror",
                "claude",
                "copilot",
                "cursor",
                "marketplace",
                "cache",
                "agent"
            ]
        );
    }

    #[test]
    fn test_priority_labels_and_rank_map() {
        let (labels, rank_map) = priority_labels_and_rank_map();

        assert_eq!(labels.len(), 8);
        assert_eq!(rank_map.len(), 8);

        assert_eq!(rank_map.get("codex").unwrap(), 1);
        assert_eq!(rank_map.get("mirror").unwrap(), 2);
        assert_eq!(rank_map.get("claude").unwrap(), 3);
        assert_eq!(rank_map.get("copilot").unwrap(), 4);
        assert_eq!(rank_map.get("cursor").unwrap(), 5);
        assert_eq!(rank_map.get("marketplace").unwrap(), 6);
        assert_eq!(rank_map.get("cache").unwrap(), 7);
        assert_eq!(rank_map.get("agent").unwrap(), 8);
    }

    #[test]
    fn test_discover_skills_empty_root() {
        let tmp = tempdir().unwrap();
        let empty_root = tmp.path().join("empty");
        fs::create_dir_all(&empty_root).unwrap();

        let roots = vec![SkillRoot {
            root: empty_root,
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None).unwrap();
        assert_eq!(skills.len(), 0);
    }

    #[test]
    fn test_discover_skills_nonexistent_root() {
        let tmp = tempdir().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");

        let roots = vec![SkillRoot {
            root: nonexistent,
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None).unwrap();
        assert_eq!(skills.len(), 0);
    }

    #[test]
    fn test_discover_skills_respects_custom_depth() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");

        // Create skill at depth 3 (a/b/c)
        let deep_path = root.join("a/b/c");
        fs::create_dir_all(&deep_path).unwrap();
        fs::write(deep_path.join("SKILL.md"), "deep skill").unwrap();

        let roots = vec![SkillRoot {
            root: root.clone(),
            source: SkillSource::Codex,
        }];

        // With depth 2, should NOT find the skill at depth 3
        let skills = discover_skills_with_depth(&roots, None, 2).unwrap();
        assert!(
            skills.is_empty(),
            "Skill at depth 3 should not be found with max_depth=2"
        );

        // With depth 4, should find it
        let skills = discover_skills_with_depth(&roots, None, 4).unwrap();
        assert_eq!(
            skills.len(),
            1,
            "Skill at depth 3 should be found with max_depth=4"
        );
    }

    #[test]
    fn test_discovery_config_default_max_depth() {
        let config = DiscoveryConfig::new(vec![], Duration::from_millis(1000), None);
        assert_eq!(config.max_depth, DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn test_discovery_config_with_custom_max_depth() {
        let config = DiscoveryConfig::with_max_depth(vec![], Duration::from_millis(1000), None, 5);
        assert_eq!(config.max_depth, 5);
    }

    // ============================================================
    // Frontmatter identity extraction tests
    // ============================================================

    #[test]
    fn extract_frontmatter_identity_from_valid_frontmatter() {
        let content = r#"---
name: test-skill
description: This is a test skill description
version: 1.0.0
---

# Test Skill Content
"#;
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("test-skill".to_string()));
        assert_eq!(desc, Some("This is a test skill description".to_string()));
    }

    #[test]
    fn extract_frontmatter_identity_returns_none_for_missing_description() {
        let content = r#"---
name: test-skill
version: 1.0.0
---

# Test Skill Content
"#;
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("test-skill".to_string()));
        assert!(desc.is_none());
    }

    #[test]
    fn extract_frontmatter_identity_returns_none_for_empty_description() {
        let content = r#"---
name: test-skill
description: ""
---

# Test Skill Content
"#;
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("test-skill".to_string()));
        assert!(desc.is_none());
    }

    #[test]
    fn extract_frontmatter_identity_returns_none_for_no_frontmatter() {
        let content = "# Just Markdown\n\nNo frontmatter here.";
        let (name, desc) = extract_frontmatter_identity(content);
        assert!(name.is_none());
        assert!(desc.is_none());
    }

    #[test]
    fn extract_frontmatter_identity_returns_none_for_unclosed_frontmatter() {
        let content = r#"---
name: test-skill
description: This will not be extracted
"#;
        let (name, desc) = extract_frontmatter_identity(content);
        assert!(name.is_none());
        assert!(desc.is_none());
    }

    #[test]
    fn extract_frontmatter_identity_handles_leading_whitespace() {
        let content = r#"
---
name: test-skill
description: Description with leading whitespace
---

Content
"#;
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("test-skill".to_string()));
        assert_eq!(
            desc,
            Some("Description with leading whitespace".to_string())
        );
    }

    #[test]
    fn extract_frontmatter_identity_yaml_special_characters() {
        // Test quotes in description
        let content = r#"---
name: test-skill
description: "Description with 'quotes' and \"double quotes\""
---
Content
"#;
        let (_, desc) = extract_frontmatter_identity(content);
        assert_eq!(
            desc,
            Some("Description with 'quotes' and \"double quotes\"".to_string())
        );

        // Test colon in description (must be quoted in YAML)
        let content = r#"---
name: test-skill
description: "Note: this has a colon"
---
Content
"#;
        let (_, desc) = extract_frontmatter_identity(content);
        assert_eq!(desc, Some("Note: this has a colon".to_string()));

        // Test newlines in multiline description
        let content = r#"---
name: test-skill
description: |
  Line one
  Line two
---
Content
"#;
        let (_, desc) = extract_frontmatter_identity(content);
        assert!(desc.is_some());
        let d = desc.unwrap();
        assert!(d.contains("Line one"));
        assert!(d.contains("Line two"));
    }

    #[test]
    fn extract_frontmatter_identity_crlf_line_endings() {
        // CRLF line endings (Windows-style)
        let content = "---\r\nname: test-skill\r\ndescription: CRLF test\r\n---\r\nContent";
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("test-skill".to_string()));
        assert_eq!(desc, Some("CRLF test".to_string()));

        // Mixed line endings
        let content = "---\nname: test-skill\r\ndescription: Mixed endings\n---\r\nContent";
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("test-skill".to_string()));
        assert_eq!(desc, Some("Mixed endings".to_string()));
    }

    #[test]
    fn extract_frontmatter_identity_very_long_description() {
        // Very long description (edge case)
        let long_desc = "A".repeat(10_000);
        let content = format!(
            "---\nname: test-skill\ndescription: {}\n---\nContent",
            long_desc
        );
        let (name, desc) = extract_frontmatter_identity(&content);
        assert_eq!(name, Some("test-skill".to_string()));
        assert_eq!(desc, Some(long_desc));
    }

    #[test]
    fn extract_frontmatter_identity_whitespace_only() {
        // Whitespace-only descriptions are filtered out at extraction
        let content = r#"---
name: test-skill
description: "   "
---
Content
"#;
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("test-skill".to_string()));
        assert!(
            desc.is_none(),
            "Whitespace-only descriptions should return None"
        );
    }

    #[test]
    fn extract_frontmatter_identity_returns_both() {
        let content = "---\nname: my-skill\ndescription: Does stuff\n---\nBody";
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("my-skill".to_string()));
        assert_eq!(desc, Some("Does stuff".to_string()));
    }

    #[test]
    fn extract_frontmatter_identity_name_only() {
        let content = "---\nname: my-skill\n---\nBody";
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("my-skill".to_string()));
        assert!(desc.is_none());
    }

    #[test]
    fn extract_frontmatter_identity_no_frontmatter() {
        let content = "# Just markdown";
        let (name, desc) = extract_frontmatter_identity(content);
        assert!(name.is_none());
        assert!(desc.is_none());
    }

    #[test]
    fn discover_skills_includes_description() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("test-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: test-skill
description: A skill with a cached description
---

# Test Content
"#,
        )
        .unwrap();

        let roots = vec![SkillRoot {
            root: tmp.path().to_path_buf(),
            source: SkillSource::Codex,
        }];
        let skills = discover_skills(&roots, None).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(
            skills[0].description,
            Some("A skill with a cached description".to_string())
        );
    }

    #[test]
    fn discover_skills_with_same_description() {
        // Multiple skills can have identical descriptions - they're independent
        // (different frontmatter names means they are distinct skills)
        let tmp = tempdir().unwrap();

        // Create two skills with the same description but different names
        let skill1_dir = tmp.path().join("skill-one");
        fs::create_dir_all(&skill1_dir).unwrap();
        fs::write(
            skill1_dir.join("SKILL.md"),
            r#"---
name: skill-one
description: Shared description for database operations
---
# Skill One
"#,
        )
        .unwrap();

        let skill2_dir = tmp.path().join("skill-two");
        fs::create_dir_all(&skill2_dir).unwrap();
        fs::write(
            skill2_dir.join("SKILL.md"),
            r#"---
name: skill-two
description: Shared description for database operations
---
# Skill Two
"#,
        )
        .unwrap();

        let roots = vec![SkillRoot {
            root: tmp.path().to_path_buf(),
            source: SkillSource::Codex,
        }];
        let skills = discover_skills(&roots, None).unwrap();

        assert_eq!(skills.len(), 2);
        // Both should have the same description
        let descriptions: Vec<_> = skills.iter().map(|s| &s.description).collect();
        assert!(descriptions
            .iter()
            .all(|d| *d == &Some("Shared description for database operations".to_string())));
    }

    #[test]
    fn extract_frontmatter_identity_multiword_matching() {
        // Test that multi-word descriptions are preserved correctly
        let content = r#"---
name: test-skill
description: Database query optimization and performance tuning
---
Content
"#;
        let (name, desc) = extract_frontmatter_identity(content);
        assert_eq!(name, Some("test-skill".to_string()));
        let d = desc.unwrap();

        // All words should be present
        assert!(d.contains("Database"));
        assert!(d.contains("query"));
        assert!(d.contains("optimization"));
        assert!(d.contains("performance"));
        assert!(d.contains("tuning"));

        // The exact phrase should be preserved
        assert_eq!(d, "Database query optimization and performance tuning");
    }

    // ============================================================
    // Jaccard similarity tests
    // ============================================================

    #[test]
    fn jaccard_similarity_identical() {
        assert!((jaccard_similarity("hello world", "hello world") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_similarity_disjoint() {
        assert!((jaccard_similarity("hello world", "foo bar")).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_similarity_partial_overlap() {
        let sim = jaccard_similarity("hello world foo", "hello world bar");
        assert!(sim > 0.3 && sim < 0.7); // 2/4 = 0.5
    }

    #[test]
    fn jaccard_similarity_case_insensitive() {
        assert!((jaccard_similarity("Hello World", "hello world") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jaccard_similarity_both_empty() {
        assert!((jaccard_similarity("", "")).abs() < f64::EPSILON);
    }

    // ============================================================
    // Frontmatter dedup integration tests
    // ============================================================

    #[test]
    fn frontmatter_dedup_same_name_similar_desc() {
        let tmp = tempdir().unwrap();

        let root_a = tmp.path().join("codex");
        fs::create_dir_all(root_a.join("plugin-a/commit")).unwrap();
        fs::write(
            root_a.join("plugin-a/commit/SKILL.md"),
            "---\nname: commit-messages\ndescription: Generate conventional commit messages\n---\nContent A",
        ).unwrap();

        let root_b = tmp.path().join("marketplace");
        fs::create_dir_all(root_b.join("sanctum/commit")).unwrap();
        fs::write(
            root_b.join("sanctum/commit/SKILL.md"),
            "---\nname: commit-messages\ndescription: Generate conventional commit messages for repos\n---\nContent B",
        ).unwrap();

        let roots = vec![
            SkillRoot {
                root: root_a,
                source: SkillSource::Codex,
            },
            SkillRoot {
                root: root_b,
                source: SkillSource::Marketplace,
            },
        ];

        let mut dup_log = vec![];
        let skills = discover_skills(&roots, Some(&mut dup_log)).unwrap();

        assert_eq!(skills.len(), 1, "Same fm name + similar desc should dedup");
        assert_eq!(dup_log.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Codex);
    }

    #[test]
    fn frontmatter_dedup_same_name_different_desc() {
        let tmp = tempdir().unwrap();

        let root_a = tmp.path().join("codex");
        fs::create_dir_all(root_a.join("skill-a")).unwrap();
        fs::write(
            root_a.join("skill-a/SKILL.md"),
            "---\nname: deploy\ndescription: Deploy to production AWS servers\n---\nContent A",
        )
        .unwrap();

        let root_b = tmp.path().join("marketplace");
        fs::create_dir_all(root_b.join("skill-b")).unwrap();
        fs::write(
            root_b.join("skill-b/SKILL.md"),
            "---\nname: deploy\ndescription: Run local Docker containers for testing\n---\nContent B",
        ).unwrap();

        let roots = vec![
            SkillRoot {
                root: root_a,
                source: SkillSource::Codex,
            },
            SkillRoot {
                root: root_b,
                source: SkillSource::Marketplace,
            },
        ];

        let mut dup_log = vec![];
        let skills = discover_skills(&roots, Some(&mut dup_log)).unwrap();

        assert_eq!(
            skills.len(),
            2,
            "Same fm name but very different desc should keep both"
        );
        assert!(dup_log.is_empty());
    }

    #[test]
    fn frontmatter_dedup_one_missing_desc() {
        let tmp = tempdir().unwrap();

        let root_a = tmp.path().join("codex");
        fs::create_dir_all(root_a.join("skill-a")).unwrap();
        fs::write(
            root_a.join("skill-a/SKILL.md"),
            "---\nname: my-skill\ndescription: Does useful things\n---\nContent A",
        )
        .unwrap();

        let root_b = tmp.path().join("marketplace");
        fs::create_dir_all(root_b.join("skill-b")).unwrap();
        fs::write(
            root_b.join("skill-b/SKILL.md"),
            "---\nname: my-skill\n---\nContent B",
        )
        .unwrap();

        let roots = vec![
            SkillRoot {
                root: root_a,
                source: SkillSource::Codex,
            },
            SkillRoot {
                root: root_b,
                source: SkillSource::Marketplace,
            },
        ];

        let mut dup_log = vec![];
        let skills = discover_skills(&roots, Some(&mut dup_log)).unwrap();

        assert_eq!(
            skills.len(),
            2,
            "Same fm name + one missing desc should NOT dedup (can't compare semantically)"
        );
        assert_eq!(dup_log.len(), 0);
    }

    #[test]
    fn frontmatter_dedup_no_fm_name() {
        let tmp = tempdir().unwrap();

        let root_a = tmp.path().join("codex");
        fs::create_dir_all(root_a.join("skill-x")).unwrap();
        fs::write(
            root_a.join("skill-x/SKILL.md"),
            "---\ndescription: No name field here\n---\nContent A",
        )
        .unwrap();

        let root_b = tmp.path().join("marketplace");
        fs::create_dir_all(root_b.join("skill-y")).unwrap();
        fs::write(
            root_b.join("skill-y/SKILL.md"),
            "---\ndescription: No name field here either\n---\nContent B",
        )
        .unwrap();

        let roots = vec![
            SkillRoot {
                root: root_a,
                source: SkillSource::Codex,
            },
            SkillRoot {
                root: root_b,
                source: SkillSource::Marketplace,
            },
        ];

        let skills = discover_skills(&roots, None).unwrap();

        assert_eq!(skills.len(), 2, "Without fm name, only path dedup applies");
    }

    // ============================================================
    // Rule discovery tests
    // ============================================================

    #[test]
    fn discover_rules_from_user_hooks_dir() {
        let tmp = tempdir().unwrap();
        let hooks_dir = tmp.path().join(".claude").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        fs::write(
            hooks_dir.join("pre-commit-lint.json"),
            r#"{"description": "Run linter", "command": "cargo clippy"}"#,
        )
        .unwrap();

        let rules = discover_rules(tmp.path(), None);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "pre-commit-lint");
        assert_eq!(rules[0].source, "user");
        assert_eq!(rules[0].category, RuleCategory::PreCommit);
        assert!(rules[0].enabled);
    }

    #[test]
    fn discover_rules_from_project_hooks_dir() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        fs::create_dir_all(&home).unwrap();

        let proj_hooks = project.join(".claude").join("hooks");
        fs::create_dir_all(&proj_hooks).unwrap();
        fs::write(
            proj_hooks.join("post-commit-notify.yaml"),
            "description: Notify on commit\ncommand: notify-send",
        )
        .unwrap();

        let rules = discover_rules(&home, Some(&project));
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "post-commit-notify");
        assert_eq!(rules[0].source, "project");
        assert_eq!(rules[0].category, RuleCategory::PostCommit);
    }

    #[test]
    fn discover_rules_from_hookify_dir() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        fs::create_dir_all(&home).unwrap();

        let hookify_dir = project.join(".hookify");
        fs::create_dir_all(&hookify_dir).unwrap();
        fs::write(
            hookify_dir.join("notification-slack.toml"),
            "description = \"Slack notification\"\ncommand = \"curl ...\"",
        )
        .unwrap();

        let rules = discover_rules(&home, Some(&project));
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "notification-slack");
        assert_eq!(rules[0].source, "hookify");
        assert_eq!(rules[0].category, RuleCategory::Notification);
    }

    #[test]
    fn discover_rules_combines_all_sources() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");

        // User hooks
        let user_hooks = home.join(".claude").join("hooks");
        fs::create_dir_all(&user_hooks).unwrap();
        fs::write(user_hooks.join("pre-push-check.json"), "{}").unwrap();

        // Project hooks
        let proj_hooks = project.join(".claude").join("hooks");
        fs::create_dir_all(&proj_hooks).unwrap();
        fs::write(proj_hooks.join("precommit-format.json"), "{}").unwrap();

        // Hookify rules
        let hookify_dir = project.join(".hookify");
        fs::create_dir_all(&hookify_dir).unwrap();
        fs::write(hookify_dir.join("prompt-validate.yml"), "{}").unwrap();

        let rules = discover_rules(&home, Some(&project));
        assert_eq!(rules.len(), 3);

        let sources: Vec<&str> = rules.iter().map(|r| r.source.as_str()).collect();
        assert!(sources.contains(&"user"));
        assert!(sources.contains(&"project"));
        assert!(sources.contains(&"hookify"));
    }

    #[test]
    fn discover_rules_skips_non_config_files() {
        let tmp = tempdir().unwrap();
        let hooks_dir = tmp.path().join(".claude").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        fs::write(hooks_dir.join("readme.txt"), "not a rule").unwrap();
        fs::write(hooks_dir.join("script.sh"), "#!/bin/bash").unwrap();
        fs::write(hooks_dir.join("valid.json"), "{}").unwrap();

        let rules = discover_rules(tmp.path(), None);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "valid");
    }

    #[test]
    fn discover_rules_empty_when_no_dirs_exist() {
        let tmp = tempdir().unwrap();
        let rules = discover_rules(tmp.path(), None);
        assert!(rules.is_empty());
    }

    #[test]
    fn infer_category_known_patterns() {
        assert_eq!(
            infer_category_from_name("pre-commit-lint"),
            RuleCategory::PreCommit
        );
        assert_eq!(
            infer_category_from_name("precommit-format"),
            RuleCategory::PreCommit
        );
        assert_eq!(
            infer_category_from_name("post-commit-notify"),
            RuleCategory::PostCommit
        );
        assert_eq!(
            infer_category_from_name("postcommit-hook"),
            RuleCategory::PostCommit
        );
        assert_eq!(
            infer_category_from_name("pre-push-check"),
            RuleCategory::PrePush
        );
        assert_eq!(
            infer_category_from_name("prepush-validate"),
            RuleCategory::PrePush
        );
        assert_eq!(
            infer_category_from_name("prompt-validate"),
            RuleCategory::PromptSubmit
        );
        assert_eq!(
            infer_category_from_name("on-submit-check"),
            RuleCategory::PromptSubmit
        );
        assert_eq!(
            infer_category_from_name("notification-slack"),
            RuleCategory::Notification
        );
    }

    #[test]
    fn infer_category_unknown_falls_back_to_other() {
        let cat = infer_category_from_name("custom-hook");
        assert_eq!(cat, RuleCategory::Other("custom-hook".to_string()));
    }
}
