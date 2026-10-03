//! Skill URIs and dependency resolution for the graph-building commands.
//!
//! `recommend` and `metrics` build their dependency graph in the CLI. They
//! used to add a dependency's link target verbatim (`../other/SKILL.md`),
//! while the server's cache resolves it to the linked skill's URI, so the CLI
//! and the MCP tools disagreed about the same skill. This resolves the same
//! way the cache does: relative to the linking skill's directory, matched by
//! canonical path, and dropped when it names no discovered skill. The
//! canonical paths are computed once per run rather than once per link.

use skrills_discovery::SkillMeta;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The URI the server and the CLI give a discovered skill.
pub(crate) fn skill_uri(meta: &SkillMeta) -> String {
    format!("skill://skrills/{}/{}", meta.source.label(), meta.name)
}

/// Resolves a skill's dependency links to the URIs of discovered skills.
pub(crate) struct DependencyResolver {
    by_canonical_path: HashMap<PathBuf, String>,
}

impl DependencyResolver {
    pub(crate) fn new(skills: &[SkillMeta]) -> Self {
        let by_canonical_path = skills
            .iter()
            .filter_map(|s| Some((s.path.canonicalize().ok()?, skill_uri(s))))
            .collect();
        Self { by_canonical_path }
    }

    /// The URI of the skill `target` points at, read relative to the
    /// directory of `skill_path`, or `None` when it names no known skill.
    pub(crate) fn resolve(&self, skill_path: &Path, target: &str) -> Option<String> {
        let resolved = skill_path.parent()?.join(target).canonicalize().ok()?;
        self.by_canonical_path.get(&resolved).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::SkillSource;

    fn meta(root: &Path, name: &str) -> SkillMeta {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "x").unwrap();
        SkillMeta {
            name: name.to_string(),
            path,
            source: SkillSource::Claude,
            root: root.to_path_buf(),
            hash: String::new(),
            description: None,
            frontmatter_name: None,
        }
    }

    /// SA-25: a relative link was reported as the literal string.
    #[test]
    fn a_relative_link_resolves_to_the_linked_skill_uri() {
        let dir = tempfile::tempdir().unwrap();
        let a = meta(dir.path(), "a/SKILL.md");
        let b = meta(dir.path(), "b/SKILL.md");
        let resolver = DependencyResolver::new(&[a.clone(), b]);

        assert_eq!(
            resolver.resolve(&a.path, "../b/SKILL.md").as_deref(),
            Some("skill://skrills/claude/b/SKILL.md")
        );
    }

    #[test]
    fn a_link_to_no_known_skill_resolves_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let a = meta(dir.path(), "a/SKILL.md");
        let resolver = DependencyResolver::new(std::slice::from_ref(&a));

        assert_eq!(resolver.resolve(&a.path, "../missing/SKILL.md"), None);
        assert_eq!(resolver.resolve(&a.path, "notes.md"), None);
    }
}
