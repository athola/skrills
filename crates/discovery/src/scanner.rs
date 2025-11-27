use crate::types::{parse_source_key, DuplicateInfo, SkillMeta, SkillRoot, SkillSource};
use anyhow::Result;
use pathdiff::diff_paths;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use walkdir::WalkDir;

/// Configuration for the skill discovery process.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub roots: Vec<SkillRoot>,
    pub cache_ttl_ms: Duration,
    pub priority_override: Option<Vec<SkillSource>>, // ordered
}

impl DiscoveryConfig {
    pub fn new(
        roots: Vec<SkillRoot>,
        cache_ttl_ms: Duration,
        priority_override: Option<Vec<SkillSource>>,
    ) -> Self {
        Self {
            roots,
            cache_ttl_ms,
            priority_override,
        }
    }
}

pub fn default_priority() -> Vec<SkillSource> {
    vec![
        SkillSource::Codex,
        SkillSource::Mirror,
        SkillSource::Claude,
        SkillSource::Agent,
    ]
}

pub fn priority_labels() -> Vec<String> {
    priority_labels_and_rank_map().0
}

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

fn is_skill_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file() && entry.file_name() == "SKILL.md"
}

/// Computes the SHA256 hash of a skill file for cache-busting.
fn file_hash(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let data = fs::read(path)?;
    hasher.update(data);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Collects skill metadata from the provided roots.
fn collect_skills_from(
    roots: &[SkillRoot],
    mut dup_log: Option<&mut Vec<DuplicateInfo>>,
) -> Result<Vec<SkillMeta>> {
    let mut skills = Vec::new();
    let mut seen: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new(); // name -> (source, root)
    for root_cfg in roots {
        let root = &root_cfg.root;
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root)
            .min_depth(1)
            .max_depth(6)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !is_skill_file(&entry) {
                continue;
            }
            let path = entry.into_path();
            let name = diff_paths(&path, root)
                .and_then(|p| p.to_str().map(|s| s.to_owned()))
                .unwrap_or_else(|| path.to_string_lossy().into_owned());

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

            let hash = file_hash(&path)?;
            skills.push(SkillMeta {
                name: name.clone(),
                path: path.clone(),
                source: root_cfg.source.clone(),
                root: root.clone(),
                hash,
            });
            seen.insert(name, (root_cfg.source.label(), root.display().to_string()));
        }
    }
    Ok(skills)
}

/// Discovers and collects skill metadata from the provided roots.
///
/// If `dup_log` is provided, information about duplicate skills (where a skill
/// with the same name exists in multiple roots, and only the highest priority
/// one is kept) will be logged here.
pub fn discover_skills(
    roots: &[SkillRoot],
    dup_log: Option<&mut Vec<DuplicateInfo>>,
) -> Result<Vec<SkillMeta>> {
    collect_skills_from(roots, dup_log)
}

/// Extracts skill references (words not matching "skills" or "rules") from an AGENTS.md document.
///
/// This function tokenizes the input markdown and collects alphanumeric strings that
/// are at least three characters long and are not common keywords like "skills" or "rules".
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

/// Information about available skill roots and their precedence.
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
            root: home.join(".agent/skills"),
            source: SkillSource::Agent,
        },
    ]
}

/// Build roots from extra directories (project-provided).
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

/// Hash helper for testing.
pub fn hash_file(path: &Path) -> Result<String> {
    file_hash(path)
}

/// Determines the effective skill source priority order.
///
/// If an `override_order` is provided, it will be used. Otherwise, the `default_priority`
/// will be returned.
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
        assert_eq!(labels, vec!["codex", "mirror", "claude", "agent"]);
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
        let err = discover_skills(&roots, None).unwrap_err();
        assert!(
            err.to_string().contains("Permission denied")
                || err.to_string().contains("permission denied")
        );
    }
}
