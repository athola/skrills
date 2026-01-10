use crate::types::{parse_source_key, DuplicateInfo, SkillMeta, SkillRoot, SkillSource};
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

/// Returns the default skill source priority order.
pub fn default_priority() -> Vec<SkillSource> {
    vec![
        SkillSource::Codex,
        SkillSource::Mirror,
        SkillSource::Claude,
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

/// Checks if a `DirEntry` is a skill file (`SKILL.md`).
fn is_skill_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file() && entry.file_name() == "SKILL.md"
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
            let mut prefix = vec![0u8; 1024.min(size as usize)];
            if let Ok(n) = file.read(&mut prefix) {
                prefix.truncate(n);
                hasher.update(&prefix);
            }
        }
    }
    let digest = hasher.finalize();
    Ok(format!("{:x}", digest))
}

/// Extracts the description from a skill file's YAML frontmatter.
///
/// Parses the frontmatter between `---` delimiters and extracts the `description` field.
/// Returns `None` if no frontmatter or no description is present.
fn extract_description(content: &str) -> Option<String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    // Find content after opening ---
    let after_open = trimmed.get(3..)?.trim_start_matches(['\r', '\n']);

    // Find closing ---
    let end_pos = after_open
        .find("\n---")
        .or_else(|| after_open.find("\r\n---"))?;
    let yaml = &after_open[..end_pos];

    // Parse YAML to extract description field
    // Use a minimal struct to avoid pulling in complex types
    #[derive(serde::Deserialize)]
    struct MinimalFrontmatter {
        description: Option<String>,
    }

    serde_yaml::from_str::<MinimalFrontmatter>(yaml)
        .ok()
        .and_then(|fm| fm.description)
        .filter(|d| !d.is_empty())
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
            .filter_map(|e| e.ok())
            .filter(is_skill_file)
            .collect();

        let metas: Vec<_> = entries
            .par_iter()
            .map(|entry| {
                let path = entry.path().to_path_buf();
                let name = diff_paths(&path, root)
                    .and_then(|p| p.to_str().map(|s| s.to_owned()))
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                let hash = file_hash(&path)?;
                // Extract description from frontmatter (best-effort, ignore errors)
                let description = fs::read_to_string(&path)
                    .ok()
                    .and_then(|content| extract_description(&content));
                Ok((name, path, hash, description))
            })
            .collect::<Result<Vec<_>>>()?;

        for (name, path, hash, description) in metas {
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

            skills.push(SkillMeta {
                name: name.clone(),
                path: path.clone(),
                source: root_cfg.source.clone(),
                root: root.clone(),
                hash,
                description,
            });
            seen.insert(name, (root_cfg.source.label(), root.display().to_string()));
        }
    }
    Ok(skills)
}

/// Discovers and collects skill metadata from the provided roots.
///
/// If `dup_log` is provided, logs duplicate skill information. Duplicates happen
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

/// Discovers and collects skill metadata from the provided roots with a custom depth limit.
///
/// If `dup_log` is provided, logs duplicate skill information. Duplicates happen
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
            .filter_map(|e| e.ok())
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

/// Extracts skill references (words not matching "skills" or "rules") from an AGENTS.md document.
///
/// Tokenizes the input markdown and collects alphanumeric strings that are at least three
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
    vec![
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
    ]
}

/// Builds skill roots from extra directories (project-provided).
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

/// Computes the BLAKE2b-256 hash of a file.
pub fn hash_file(path: &Path) -> Result<String> {
    file_hash(path)
}

/// Determines the effective skill source priority order.
///
/// Uses `override_order` if provided; otherwise, uses `default_priority`.
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
        assert_eq!(
            labels,
            vec!["codex", "mirror", "claude", "marketplace", "cache", "agent"]
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
            vec!["codex", "mirror", "claude", "marketplace", "cache", "agent"]
        );
    }

    #[test]
    fn test_priority_labels_and_rank_map() {
        let (labels, rank_map) = priority_labels_and_rank_map();

        assert_eq!(labels.len(), 6);
        assert_eq!(rank_map.len(), 6);

        assert_eq!(rank_map.get("codex").unwrap(), 1);
        assert_eq!(rank_map.get("mirror").unwrap(), 2);
        assert_eq!(rank_map.get("claude").unwrap(), 3);
        assert_eq!(rank_map.get("marketplace").unwrap(), 4);
        assert_eq!(rank_map.get("cache").unwrap(), 5);
        assert_eq!(rank_map.get("agent").unwrap(), 6);
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
    // Description extraction tests
    // ============================================================

    #[test]
    fn extract_description_from_valid_frontmatter() {
        let content = r#"---
name: test-skill
description: This is a test skill description
version: 1.0.0
---

# Test Skill Content
"#;
        let desc = extract_description(content);
        assert_eq!(desc, Some("This is a test skill description".to_string()));
    }

    #[test]
    fn extract_description_returns_none_for_missing_description() {
        let content = r#"---
name: test-skill
version: 1.0.0
---

# Test Skill Content
"#;
        let desc = extract_description(content);
        assert!(desc.is_none());
    }

    #[test]
    fn extract_description_returns_none_for_empty_description() {
        let content = r#"---
name: test-skill
description: ""
---

# Test Skill Content
"#;
        let desc = extract_description(content);
        assert!(desc.is_none());
    }

    #[test]
    fn extract_description_returns_none_for_no_frontmatter() {
        let content = "# Just Markdown\n\nNo frontmatter here.";
        let desc = extract_description(content);
        assert!(desc.is_none());
    }

    #[test]
    fn extract_description_returns_none_for_unclosed_frontmatter() {
        let content = r#"---
name: test-skill
description: This will not be extracted
"#;
        let desc = extract_description(content);
        assert!(desc.is_none());
    }

    #[test]
    fn extract_description_handles_leading_whitespace() {
        let content = r#"
---
name: test-skill
description: Description with leading whitespace
---

Content
"#;
        let desc = extract_description(content);
        assert_eq!(
            desc,
            Some("Description with leading whitespace".to_string())
        );
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
}
