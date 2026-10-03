//! Skill URIs for the graph-building commands.
//!
//! `recommend` and `metrics` build their dependency graph with the server's
//! `build_dependency_graph`, so a link resolves the same way in the CLI and
//! in the MCP tools: relative to the linking skill's directory, matched by
//! canonical path, and dropped when it names no discovered skill (SA-25,
//! SA-44).

use skrills_discovery::SkillMeta;

/// The URI the server and the CLI give a discovered skill.
pub(crate) fn skill_uri(meta: &SkillMeta) -> String {
    format!("skill://skrills/{}/{}", meta.source.label(), meta.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::SkillSource;
    use skrills_server::app::build_dependency_graph;
    use std::path::Path;

    fn meta(root: &Path, name: &str, content: &str) -> SkillMeta {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
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
        let a = meta(dir.path(), "a/SKILL.md", "See [b](../b/SKILL.md).");
        let b = meta(dir.path(), "b/SKILL.md", "x");

        let graph = build_dependency_graph(&[a.clone(), b]);

        assert_eq!(
            graph
                .dependencies(&skill_uri(&a))
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["skill://skrills/claude/b/SKILL.md".to_string()]
        );
    }

    #[test]
    fn a_link_to_no_known_skill_resolves_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let a = meta(
            dir.path(),
            "a/SKILL.md",
            "See [missing](../missing/SKILL.md) and [notes](notes.md).",
        );

        let graph = build_dependency_graph(std::slice::from_ref(&a));

        assert!(graph.dependencies(&skill_uri(&a)).is_empty());
    }
}
