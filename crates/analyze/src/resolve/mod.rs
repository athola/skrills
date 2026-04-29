//! Dependency resolution for skills.
//!
//! Provides two resolution strategies:
//! - `DependencyResolver`: Trait-based, on-demand resolution (flexible)
//! - `DependencyGraph`: Pre-computed, index-based resolution (fast)
//!
//! # Performance
//!
//! For best performance, use `DependencyGraph` which:
//! - Pre-computes the entire graph at build time: O(V + E)
//! - Caches resolution results: O(1) for repeated lookups
//! - Uses index-based adjacency lists: no string hashing during traversal
//!
//! # Example
//!
//! ```rust
//! use skrills_analyze::resolve::{DependencyGraph, GraphBuilder, SkillInfo};
//! use skrills_discovery::SkillSource;
//! use skrills_validate::frontmatter::{DeclaredDependency, SkillFrontmatter};
//!
//! let base = SkillInfo {
//!     name: "base".into(),
//!     source: SkillSource::Extra(0),
//!     uri: "skill://base".into(),
//!     version: None,
//!     frontmatter: Some(SkillFrontmatter {
//!         name: Some("base".into()),
//!         ..Default::default()
//!     }),
//! };
//!
//! let child = SkillInfo {
//!     name: "child".into(),
//!     source: SkillSource::Extra(0),
//!     uri: "skill://child".into(),
//!     version: None,
//!     frontmatter: Some(SkillFrontmatter {
//!         name: Some("child".into()),
//!         depends: vec![DeclaredDependency::Simple("base".into())],
//!         ..Default::default()
//!     }),
//! };
//!
//! let graph = GraphBuilder::new()
//!     .add_skill(base)
//!     .add_skill(child)
//!     .build()
//!     .expect("graph should build");
//!
//! let resolved = graph.resolve("child").expect("resolution succeeds");
//! assert_eq!(resolved.resolved.len(), 2);
//! ```

mod graph;
mod resolver;
mod types;

pub use graph::{DependencyGraph, GraphBuilder};
pub use resolver::{DependencyResolver, InMemoryRegistry, SkillRegistry};
pub use types::{
    ResolutionResult, ResolveError, ResolveOptions, ResolvedDependency, SkillInfo,
};

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::SkillSource;
    use skrills_validate::frontmatter::{DeclaredDependency, SkillFrontmatter};

    fn make_skill(name: &str, deps: Vec<DeclaredDependency>) -> SkillInfo {
        SkillInfo {
            name: name.to_string(),
            source: SkillSource::Extra(0),
            uri: format!("skill://test/{}", name),
            version: Some(semver::Version::new(1, 0, 0)),
            frontmatter: Some(SkillFrontmatter {
                name: Some(name.to_string()),
                depends: deps,
                ..Default::default()
            }),
        }
    }

    fn make_versioned_skill(
        name: &str,
        version: (u64, u64, u64),
        deps: Vec<DeclaredDependency>,
    ) -> SkillInfo {
        SkillInfo {
            name: name.to_string(),
            source: SkillSource::Extra(0),
            uri: format!("skill://test/{}", name),
            version: Some(semver::Version::new(version.0, version.1, version.2)),
            frontmatter: Some(SkillFrontmatter {
                name: Some(name.to_string()),
                depends: deps,
                ..Default::default()
            }),
        }
    }

    // ========== DependencyResolver (trait-based) tests ==========

    #[test]
    fn test_resolver_no_dependencies() {
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill("standalone", vec![]));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("standalone").unwrap();

        assert!(result.success);
        assert_eq!(result.resolved.len(), 1);
        assert_eq!(result.resolved[0].name, "standalone");
    }

    #[test]
    fn test_resolver_simple_chain() {
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill("base", vec![]));
        registry.add(make_skill(
            "middle",
            vec![DeclaredDependency::Simple("base".to_string())],
        ));
        registry.add(make_skill(
            "top",
            vec![DeclaredDependency::Simple("middle".to_string())],
        ));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("top").unwrap();

        assert_eq!(result.resolved.len(), 3);
        assert_eq!(result.resolved[0].name, "base");
        assert_eq!(result.resolved[1].name, "middle");
        assert_eq!(result.resolved[2].name, "top");
    }

    #[test]
    fn test_resolver_circular_dependency() {
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill(
            "a",
            vec![DeclaredDependency::Simple("b".to_string())],
        ));
        registry.add(make_skill(
            "b",
            vec![DeclaredDependency::Simple("a".to_string())],
        ));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("a");

        assert!(matches!(
            result,
            Err(ResolveError::CircularDependency { .. })
        ));
    }

    #[test]
    fn test_resolver_version_mismatch() {
        let mut registry = InMemoryRegistry::new();
        registry.add(make_versioned_skill("dep", (1, 0, 0), vec![]));
        registry.add(make_skill(
            "parent",
            vec![DeclaredDependency::Simple("dep@^2.0".to_string())],
        ));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("parent");

        assert!(matches!(result, Err(ResolveError::VersionMismatch { .. })));
    }

    // ========== DependencyGraph (pre-computed) tests ==========

    #[test]
    fn test_graph_no_dependencies() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("standalone", vec![]))
            .build()
            .unwrap();

        let result = graph.resolve("standalone").unwrap();

        assert!(result.success);
        assert_eq!(result.resolved.len(), 1);
        assert_eq!(result.resolved[0].name, "standalone");
    }

    #[test]
    fn test_graph_simple_chain() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("base", vec![]))
            .add_skill(make_skill(
                "middle",
                vec![DeclaredDependency::Simple("base".to_string())],
            ))
            .add_skill(make_skill(
                "top",
                vec![DeclaredDependency::Simple("middle".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("top").unwrap();

        assert_eq!(result.resolved.len(), 3);
        assert_eq!(result.resolved[0].name, "base");
        assert_eq!(result.resolved[1].name, "middle");
        assert_eq!(result.resolved[2].name, "top");
    }

    #[test]
    fn test_graph_diamond_dependency() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("d", vec![]))
            .add_skill(make_skill(
                "b",
                vec![DeclaredDependency::Simple("d".to_string())],
            ))
            .add_skill(make_skill(
                "c",
                vec![DeclaredDependency::Simple("d".to_string())],
            ))
            .add_skill(make_skill(
                "a",
                vec![
                    DeclaredDependency::Simple("b".to_string()),
                    DeclaredDependency::Simple("c".to_string()),
                ],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("a").unwrap();

        // D should only appear once
        let names: Vec<_> = result.resolved.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names.iter().filter(|&&n| n == "d").count(), 1);

        // D should come before B and C
        let d_pos = names.iter().position(|&n| n == "d").unwrap();
        let b_pos = names.iter().position(|&n| n == "b").unwrap();
        let c_pos = names.iter().position(|&n| n == "c").unwrap();
        assert!(d_pos < b_pos);
        assert!(d_pos < c_pos);
    }

    #[test]
    fn test_graph_circular_dependency() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill(
                "a",
                vec![DeclaredDependency::Simple("b".to_string())],
            ))
            .add_skill(make_skill(
                "b",
                vec![DeclaredDependency::Simple("a".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("a");
        assert!(matches!(
            result,
            Err(ResolveError::CircularDependency { .. })
        ));
    }

    #[test]
    fn test_graph_longer_cycle() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill(
                "a",
                vec![DeclaredDependency::Simple("b".to_string())],
            ))
            .add_skill(make_skill(
                "b",
                vec![DeclaredDependency::Simple("c".to_string())],
            ))
            .add_skill(make_skill(
                "c",
                vec![DeclaredDependency::Simple("a".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("a");
        assert!(matches!(
            result,
            Err(ResolveError::CircularDependency { .. })
        ));
    }

    #[test]
    fn test_graph_version_mismatch() {
        let graph = DependencyGraph::builder()
            .add_skill(make_versioned_skill("dep", (1, 0, 0), vec![]))
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dep@^2.0".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent");
        assert!(matches!(result, Err(ResolveError::VersionMismatch { .. })));
    }

    #[test]
    fn test_graph_version_satisfied() {
        let graph = DependencyGraph::builder()
            .add_skill(make_versioned_skill("dep", (2, 5, 0), vec![]))
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dep@^2.0".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        assert_eq!(result.resolved.len(), 2);
    }

    #[test]
    fn test_graph_caching() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("base", vec![]))
            .add_skill(make_skill(
                "top",
                vec![DeclaredDependency::Simple("base".to_string())],
            ))
            .build()
            .unwrap();

        // First call computes
        let (cached_before, _) = graph.cache_stats();
        assert_eq!(cached_before, 0);

        let _ = graph.resolve("top").unwrap();

        // Second call uses cache
        let (cached_after, _) = graph.cache_stats();
        assert_eq!(cached_after, 1);

        // Result should be identical
        let result1 = graph.resolve("top").unwrap();
        let result2 = graph.resolve("top").unwrap();
        assert_eq!(result1.resolved.len(), result2.resolved.len());
    }

    #[test]
    fn test_graph_source_pinning() {
        let mut codex_skill = make_skill("shared", vec![]);
        codex_skill.source = SkillSource::Codex;
        codex_skill.uri = "skill://codex/shared".to_string();

        let mut claude_skill = make_versioned_skill("shared", (2, 0, 0), vec![]);
        claude_skill.source = SkillSource::Claude;
        claude_skill.uri = "skill://claude/shared".to_string();

        let graph = DependencyGraph::builder()
            .add_skill(codex_skill)
            .add_skill(claude_skill)
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("codex:shared".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        let shared = result.resolved.iter().find(|r| r.name == "shared").unwrap();
        assert_eq!(shared.source, SkillSource::Codex);
    }

    #[test]
    fn test_graph_max_depth() {
        let mut builder = DependencyGraph::builder().with_options(ResolveOptions {
            max_depth: 5,
            ..Default::default()
        });

        for i in 0..10 {
            let deps = if i == 0 {
                vec![]
            } else {
                vec![DeclaredDependency::Simple(format!("skill-{}", i - 1))]
            };
            builder = builder.add_skill(make_skill(&format!("skill-{}", i), deps));
        }

        let graph = builder.build().unwrap();
        let result = graph.resolve("skill-9");

        assert!(matches!(result, Err(ResolveError::MaxDepthExceeded(5))));
    }

    #[test]
    fn test_graph_duplicate_skills_warn() {
        let mut first = make_skill("dup", vec![]);
        first.uri = "skill://first/dup".to_string();
        let mut second = make_skill("dup", vec![]);
        second.uri = "skill://second/dup".to_string();

        let graph = DependencyGraph::builder()
            .add_skill(first)
            .add_skill(second)
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dup".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Duplicate skill name 'dup'")));

        let dup = result.resolved.iter().find(|r| r.name == "dup").unwrap();
        assert_eq!(dup.uri, "skill://first/dup");
    }

    #[test]
    fn test_graph_missing_at_build() {
        let result = DependencyGraph::builder()
            .add_skill(make_skill(
                "orphan",
                vec![DeclaredDependency::Simple("nonexistent".to_string())],
            ))
            .build();

        assert!(matches!(result, Err(ResolveError::NotFound { .. })));
    }

    #[test]
    fn test_graph_optional_missing_ok() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Structured {
                    name: "missing".to_string(),
                    version: None,
                    source: None,
                    optional: true,
                }],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        assert_eq!(result.resolved.len(), 1);

        // Verify warning is present for skipped optional dependency
        assert!(
            !result.warnings.is_empty(),
            "Expected warning for skipped optional dependency"
        );
        assert!(
            result.warnings[0].contains("optional"),
            "Warning should mention 'optional'"
        );
        // Check for "not found" message template, not the dependency name "missing"
        // (which would pass by coincidence since "missing" is both the dep name
        // and a word that could appear in error messages)
        assert!(
            result.warnings[0].contains("not found"),
            "Warning should indicate the dependency was not found"
        );
        // Also verify the actual dependency name appears
        assert!(
            result.warnings[0].contains("'missing'"),
            "Warning should mention the dependency name in quotes"
        );
    }

    #[test]
    fn test_graph_optional_flag_preserved() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("base", vec![]))
            .add_skill(make_skill(
                "optional-child",
                vec![DeclaredDependency::Simple("base".to_string())],
            ))
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Structured {
                    name: "optional-child".to_string(),
                    version: None,
                    source: None,
                    optional: true,
                }],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        let base = result.resolved.iter().find(|r| r.name == "base").unwrap();
        let child = result
            .resolved
            .iter()
            .find(|r| r.name == "optional-child")
            .unwrap();
        let parent = result.resolved.iter().find(|r| r.name == "parent").unwrap();

        assert!(base.optional);
        assert!(child.optional);
        assert!(!parent.optional);
    }

    // ========== Edge case tests ==========

    #[test]
    fn test_graph_len_and_is_empty() {
        // Given an empty graph
        let empty_graph = DependencyGraph::builder().build().unwrap();

        // Then len should be 0 and is_empty should be true
        assert_eq!(empty_graph.len(), 0);
        assert!(empty_graph.is_empty());

        // Given a graph with skills
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("a", vec![]))
            .add_skill(make_skill("b", vec![]))
            .build()
            .unwrap();

        // Then len should be 2 and is_empty should be false
        assert_eq!(graph.len(), 2);
        assert!(!graph.is_empty());
    }

    #[test]
    fn test_graph_get() {
        // Given a graph with a skill
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("my-skill", vec![]))
            .build()
            .unwrap();

        // When looking up by name
        let found = graph.get("my-skill");

        // Then it should find the skill
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "my-skill");

        // When looking up by source:name (Extra(0).label() = "extra0")
        let found_qualified = graph.get("extra0:my-skill");
        assert!(found_qualified.is_some());

        // When looking up non-existent
        let not_found = graph.get("nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_graph_clear_cache() {
        // Given a graph with cached resolution
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("skill", vec![]))
            .build()
            .unwrap();

        let _ = graph.resolve("skill").unwrap();
        let (cached_before, _) = graph.cache_stats();
        assert_eq!(cached_before, 1);

        // When clearing cache
        graph.clear_cache();

        // Then cache should be empty
        let (cached_after, _) = graph.cache_stats();
        assert_eq!(cached_after, 0);
    }

    #[test]
    fn test_graph_add_skills_batch() {
        // Given multiple skills to add
        let skills = vec![
            make_skill("skill-1", vec![]),
            make_skill("skill-2", vec![]),
            make_skill("skill-3", vec![]),
        ];

        // When using add_skills batch method
        let graph = DependencyGraph::builder()
            .add_skills(skills)
            .build()
            .unwrap();

        // Then all skills should be present
        assert_eq!(graph.len(), 3);
        assert!(graph.get("skill-1").is_some());
        assert!(graph.get("skill-2").is_some());
        assert!(graph.get("skill-3").is_some());
    }

    #[test]
    fn test_registry_list_skills() {
        // Given a registry with skills
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill("alpha", vec![]));
        registry.add(make_skill("beta", vec![]));

        // When listing skills
        let skills = registry.list_skills();

        // Then all unique skill names should be returned
        assert!(skills.contains(&"alpha".to_string()));
        assert!(skills.contains(&"beta".to_string()));
    }

    #[test]
    fn test_graph_self_cycle() {
        // Given a skill that depends on itself
        let graph = DependencyGraph::builder()
            .add_skill(make_skill(
                "narcissist",
                vec![DeclaredDependency::Simple("narcissist".to_string())],
            ))
            .build()
            .unwrap();

        // When resolving
        let result = graph.resolve("narcissist");

        // Then it should detect the self-cycle
        assert!(matches!(
            result,
            Err(ResolveError::CircularDependency { .. })
        ));
        if let Err(ResolveError::CircularDependency { chain }) = result {
            assert!(chain.contains("narcissist"));
        }
    }

    #[test]
    fn test_resolver_optional_skipped() {
        // Given a registry with parent but missing optional dependency
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill(
            "parent",
            vec![DeclaredDependency::Structured {
                name: "optional-missing".to_string(),
                version: None,
                source: None,
                optional: true,
            }],
        ));

        // When resolving with default options (strict_optional = false)
        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("parent").unwrap();

        // Then resolution succeeds with warning
        assert!(result.success);
        assert_eq!(result.resolved.len(), 1);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("optional"));
    }

    #[test]
    fn test_graph_prerelease_version() {
        // Given skills with prerelease versions
        let mut dep = make_versioned_skill("dep", (1, 0, 0), vec![]);
        dep.version = Some(semver::Version::parse("1.0.0-beta.1").unwrap());

        let graph = DependencyGraph::builder()
            .add_skill(dep)
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dep@^1.0.0-beta".to_string())],
            ))
            .build()
            .unwrap();

        // When resolving
        let result = graph.resolve("parent");

        // Then prerelease constraint should match prerelease version
        let _ = result.expect("prerelease constraint should match prerelease version");
    }

    #[test]
    fn test_graph_ignore_versions_option() {
        // Given a version mismatch scenario
        let graph = DependencyGraph::builder()
            .with_options(ResolveOptions {
                ignore_versions: true,
                ..Default::default()
            })
            .add_skill(make_versioned_skill("dep", (1, 0, 0), vec![]))
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dep@^2.0".to_string())],
            ))
            .build()
            .unwrap();

        // When resolving with ignore_versions enabled
        let result = graph.resolve("parent");

        // Then version mismatch should be ignored
        let resolution = result.expect("version mismatch should be ignored with ignore_versions");
        assert_eq!(resolution.resolved.len(), 2);
    }

    #[test]
    fn test_graph_not_found_error() {
        // Given an empty graph
        let graph = DependencyGraph::builder().build().unwrap();

        // When resolving non-existent skill
        let result = graph.resolve("ghost");

        // Then it should return SkillNotInGraph error
        assert!(matches!(result, Err(ResolveError::SkillNotInGraph(_))));
    }

    #[test]
    fn test_resolver_depth_tracking() {
        // Given a chain of dependencies
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill("level-0", vec![]));
        registry.add(make_skill(
            "level-1",
            vec![DeclaredDependency::Simple("level-0".to_string())],
        ));
        registry.add(make_skill(
            "level-2",
            vec![DeclaredDependency::Simple("level-1".to_string())],
        ));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("level-2").unwrap();

        // Then depths should be correctly tracked
        assert_eq!(result.resolved.len(), 3);
        assert_eq!(result.resolved[0].depth, 2); // level-0 at depth 2
        assert_eq!(result.resolved[1].depth, 1); // level-1 at depth 1
        assert_eq!(result.resolved[2].depth, 0); // level-2 at depth 0 (root)
    }

    #[test]
    fn test_resolver_strict_optional() {
        // Given a parent with missing optional dependency
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill(
            "parent",
            vec![DeclaredDependency::Structured {
                name: "missing-optional".to_string(),
                version: None,
                source: None,
                optional: true,
            }],
        ));

        // When resolving with strict_optional enabled
        let resolver = DependencyResolver::new(
            &registry,
            ResolveOptions {
                strict_optional: true,
                ..Default::default()
            },
        );
        let result = resolver.resolve("parent");

        // Then it should fail even though dependency is optional
        assert!(matches!(result, Err(ResolveError::NotFound { .. })));
    }
}
