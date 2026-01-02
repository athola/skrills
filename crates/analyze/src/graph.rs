//! Relationship graph for analyzing skill connections.
//!
//! Provides a simple graph structure to track relationships between skills
//! and compute transitive closures and reverse lookups.
//!
//! For full dependency resolution with version constraints and caching,
//! see [`crate::resolve::DependencyGraph`].

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// A simple graph of skill relationships.
///
/// Tracks which skills relate to (depend on) other skills, allowing for
/// transitive traversal and reverse lookups. This is a lightweight structure
/// for basic graph operations.
///
/// For full dependency resolution with version constraints, source pinning,
/// and caching, use [`crate::resolve::DependencyGraph`] instead.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelationshipGraph {
    /// Adjacency list: skill URI → set of dependency URIs
    edges: HashMap<String, HashSet<String>>,
}

impl RelationshipGraph {
    /// Create a new empty dependency graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a skill to the graph without any dependencies.
    ///
    /// If the skill already exists, this is a no-op.
    pub fn add_skill(&mut self, uri: impl Into<String>) {
        let uri = uri.into();
        self.edges.entry(uri).or_default();
    }

    /// Add a dependency edge from one skill to another.
    ///
    /// This indicates that `from_uri` depends on `to_uri`.
    /// Both skills are automatically added to the graph if not present.
    pub fn add_dependency(&mut self, from_uri: impl Into<String>, to_uri: impl Into<String>) {
        let from = from_uri.into();
        let to = to_uri.into();

        self.edges.entry(from).or_default().insert(to.clone());
        self.edges.entry(to).or_default();
    }

    /// Add multiple dependencies for a skill at once.
    pub fn add_dependencies(&mut self, from_uri: impl Into<String>, to_uris: Vec<String>) {
        let from = from_uri.into();
        for to in to_uris {
            self.add_dependency(from.clone(), to);
        }
    }

    /// Get direct dependencies of a skill.
    ///
    /// Returns an empty set if the skill is not in the graph.
    #[must_use]
    pub fn dependencies(&self, uri: &str) -> HashSet<String> {
        self.edges.get(uri).cloned().unwrap_or_default()
    }

    /// Resolve transitive dependencies for a skill.
    ///
    /// Returns all skills that the given skill depends on, directly or indirectly,
    /// in breadth-first traversal order.
    ///
    /// Handles cycles gracefully by visiting each node only once.
    #[must_use]
    pub fn resolve(&self, uri: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut queue = VecDeque::new();

        // Start with direct dependencies
        if let Some(deps) = self.edges.get(uri) {
            for dep in deps {
                queue.push_back(dep.clone());
            }
        }

        while let Some(current) = queue.pop_front() {
            if visited.contains(&current) {
                continue;
            }

            visited.insert(current.clone());
            result.push(current.clone());

            // Add transitive dependencies
            if let Some(deps) = self.edges.get(&current) {
                for dep in deps {
                    if !visited.contains(dep) {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }

        result
    }

    /// Find all skills that depend on the given skill.
    ///
    /// Returns skills that directly depend on this skill.
    #[must_use]
    pub fn dependents(&self, uri: &str) -> Vec<String> {
        let mut result = Vec::new();

        for (skill, deps) in &self.edges {
            if deps.contains(uri) && skill != uri {
                result.push(skill.clone());
            }
        }

        result.sort();
        result
    }

    /// Find all skills that transitively depend on the given skill.
    ///
    /// Returns all skills that depend on this skill, directly or indirectly.
    #[must_use]
    pub fn transitive_dependents(&self, uri: &str) -> Vec<String> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut queue = VecDeque::new();

        // Start with direct dependents
        for dependent in self.dependents(uri) {
            queue.push_back(dependent);
        }

        while let Some(current) = queue.pop_front() {
            if visited.contains(&current) {
                continue;
            }

            visited.insert(current.clone());
            result.push(current.clone());

            // Add transitive dependents
            for dependent in self.dependents(&current) {
                if !visited.contains(&dependent) {
                    queue.push_back(dependent);
                }
            }
        }

        result.sort();
        result
    }

    /// Get all skills in the graph.
    #[must_use]
    pub fn skills(&self) -> Vec<String> {
        let mut skills: Vec<_> = self.edges.keys().cloned().collect();
        skills.sort();
        skills
    }

    /// Get the number of skills in the graph.
    #[must_use]
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Check if the graph is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Detect if there's a cycle involving the given skill.
    #[must_use]
    pub fn has_cycle(&self, uri: &str) -> bool {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        self.has_cycle_util(uri, &mut visited, &mut rec_stack)
    }

    fn has_cycle_util(
        &self,
        uri: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
    ) -> bool {
        visited.insert(uri.to_string());
        rec_stack.insert(uri.to_string());

        if let Some(deps) = self.edges.get(uri) {
            for dep in deps {
                if !visited.contains(dep) {
                    if self.has_cycle_util(dep, visited, rec_stack) {
                        return true;
                    }
                } else if rec_stack.contains(dep) {
                    return true;
                }
            }
        }

        rec_stack.remove(uri);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_skill() {
        let mut graph = RelationshipGraph::new();
        graph.add_skill("skill://skrills/user/test");

        assert_eq!(graph.len(), 1);
        assert!(graph
            .skills()
            .contains(&"skill://skrills/user/test".to_string()));
    }

    #[test]
    fn test_add_dependency() {
        let mut graph = RelationshipGraph::new();
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/b");

        assert_eq!(graph.len(), 2);
        let deps = graph.dependencies("skill://skrills/user/a");
        assert!(deps.contains("skill://skrills/user/b"));
    }

    #[test]
    fn test_add_dependencies_multiple() {
        let mut graph = RelationshipGraph::new();
        graph.add_dependencies(
            "skill://skrills/user/a",
            vec![
                "skill://skrills/user/b".to_string(),
                "skill://skrills/user/c".to_string(),
            ],
        );

        let deps = graph.dependencies("skill://skrills/user/a");
        assert_eq!(deps.len(), 2);
        assert!(deps.contains("skill://skrills/user/b"));
        assert!(deps.contains("skill://skrills/user/c"));
    }

    #[test]
    fn test_resolve_transitive_simple() {
        let mut graph = RelationshipGraph::new();
        // A → B → C
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/b");
        graph.add_dependency("skill://skrills/user/b", "skill://skrills/user/c");

        let resolved = graph.resolve("skill://skrills/user/a");
        assert_eq!(resolved.len(), 2);
        assert!(resolved.contains(&"skill://skrills/user/b".to_string()));
        assert!(resolved.contains(&"skill://skrills/user/c".to_string()));
    }

    #[test]
    fn test_resolve_transitive_diamond() {
        let mut graph = RelationshipGraph::new();
        // A → B → D
        // A → C → D
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/b");
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/c");
        graph.add_dependency("skill://skrills/user/b", "skill://skrills/user/d");
        graph.add_dependency("skill://skrills/user/c", "skill://skrills/user/d");

        let resolved = graph.resolve("skill://skrills/user/a");
        assert_eq!(resolved.len(), 3);
        assert!(resolved.contains(&"skill://skrills/user/b".to_string()));
        assert!(resolved.contains(&"skill://skrills/user/c".to_string()));
        assert!(resolved.contains(&"skill://skrills/user/d".to_string()));

        // D should only appear once despite two paths to it
        assert_eq!(
            resolved
                .iter()
                .filter(|s| *s == "skill://skrills/user/d")
                .count(),
            1
        );
    }

    #[test]
    fn test_resolve_handles_cycles() {
        let mut graph = RelationshipGraph::new();
        // A → B → C → A (cycle)
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/b");
        graph.add_dependency("skill://skrills/user/b", "skill://skrills/user/c");
        graph.add_dependency("skill://skrills/user/c", "skill://skrills/user/a");

        let resolved = graph.resolve("skill://skrills/user/a");
        // Should handle gracefully without infinite loop
        assert!(resolved.len() <= 3);
        assert!(resolved.contains(&"skill://skrills/user/b".to_string()));
        assert!(resolved.contains(&"skill://skrills/user/c".to_string()));
    }

    #[test]
    fn test_dependents_direct() {
        let mut graph = RelationshipGraph::new();
        // A → C, B → C
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/c");
        graph.add_dependency("skill://skrills/user/b", "skill://skrills/user/c");

        let dependents = graph.dependents("skill://skrills/user/c");
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&"skill://skrills/user/a".to_string()));
        assert!(dependents.contains(&"skill://skrills/user/b".to_string()));
    }

    #[test]
    fn test_transitive_dependents() {
        let mut graph = RelationshipGraph::new();
        // A → B → C
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/b");
        graph.add_dependency("skill://skrills/user/b", "skill://skrills/user/c");

        let dependents = graph.transitive_dependents("skill://skrills/user/c");
        assert_eq!(dependents.len(), 2);
        assert!(dependents.contains(&"skill://skrills/user/a".to_string()));
        assert!(dependents.contains(&"skill://skrills/user/b".to_string()));
    }

    #[test]
    fn test_has_cycle_detection() {
        let mut graph = RelationshipGraph::new();
        // A → B → C → A (cycle)
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/b");
        graph.add_dependency("skill://skrills/user/b", "skill://skrills/user/c");
        graph.add_dependency("skill://skrills/user/c", "skill://skrills/user/a");

        assert!(graph.has_cycle("skill://skrills/user/a"));
        assert!(graph.has_cycle("skill://skrills/user/b"));
        assert!(graph.has_cycle("skill://skrills/user/c"));
    }

    #[test]
    fn test_no_cycle_detection() {
        let mut graph = RelationshipGraph::new();
        // A → B → C (no cycle)
        graph.add_dependency("skill://skrills/user/a", "skill://skrills/user/b");
        graph.add_dependency("skill://skrills/user/b", "skill://skrills/user/c");

        assert!(!graph.has_cycle("skill://skrills/user/a"));
        assert!(!graph.has_cycle("skill://skrills/user/b"));
        assert!(!graph.has_cycle("skill://skrills/user/c"));
    }

    #[test]
    fn test_empty_graph() {
        let graph = RelationshipGraph::new();
        assert!(graph.is_empty());
        assert_eq!(graph.len(), 0);

        let resolved = graph.resolve("skill://skrills/user/nonexistent");
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_dependencies_nonexistent_skill() {
        let graph = RelationshipGraph::new();
        let deps = graph.dependencies("skill://skrills/user/nonexistent");
        assert!(deps.is_empty());
    }
}
