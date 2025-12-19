//! Agent registry for discovering and caching agent definitions.
//!
//! The `AgentRegistry` provides a centralized way to discover agent definition
//! files from standard locations and cache their parsed configurations.

use std::collections::HashMap;

use anyhow::Result;
use skrills_discovery::{discover_agents, AgentConfig, AgentMeta, SkillRoot, SkillSource};
use tracing::warn;

/// A discovered agent with its parsed configuration.
#[derive(Debug, Clone)]
pub struct CachedAgent {
    /// Metadata about the agent file.
    pub meta: AgentMeta,
    /// Parsed configuration from the agent file.
    pub config: AgentConfig,
}

/// Registry of discovered agents with cached configurations.
///
/// The registry discovers agents from standard locations and caches
/// their parsed configurations for efficient lookup.
#[derive(Debug)]
pub struct AgentRegistry {
    /// Cached agent configs by lowercase name.
    agents: HashMap<String, CachedAgent>,
}

impl AgentRegistry {
    /// Discover agents from standard locations and parse their configs.
    ///
    /// Discovery locations (in priority order):
    /// - `~/.codex/agents/` (Codex CLI)
    /// - `~/.claude/agents/` (Claude Code)
    /// - `~/.agent/agents/` (Universal)
    ///
    /// Duplicate names are resolved by priority (first source wins).
    /// Parse failures are logged but don't fail the overall discovery.
    pub fn discover() -> Result<Self> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow::anyhow!("could not find home directory"))?;
        Self::discover_from(&home)
    }

    /// Discover agents from standard locations relative to a home directory.
    ///
    /// This is useful for testing with a custom home directory.
    pub fn discover_from(home: &std::path::Path) -> Result<Self> {
        let roots = agent_roots(home);
        Self::discover_from_roots(&roots)
    }

    /// Discover agents from the specified roots.
    pub fn discover_from_roots(roots: &[SkillRoot]) -> Result<Self> {
        let agent_metas = discover_agents(roots)?;
        let mut agents = HashMap::new();

        for meta in agent_metas {
            // Use lowercase name as key for case-insensitive lookup
            let key = agent_name_key(&meta.name);

            // Skip if we already have an agent with this name (priority first wins)
            if agents.contains_key(&key) {
                warn!(
                    name = %meta.name,
                    path = %meta.path.display(),
                    source = %meta.source.label(),
                    "skipping duplicate agent, keeping higher priority version"
                );
                continue;
            }

            // Try to parse the config
            match meta.load_config() {
                Ok(config) => {
                    agents.insert(
                        key,
                        CachedAgent {
                            meta,  // move instead of clone - meta is owned and not used after
                            config,
                        },
                    );
                }
                Err(e) => {
                    warn!(
                        name = %meta.name,
                        path = %meta.path.display(),
                        error = %e,
                        "failed to parse agent config, skipping"
                    );
                }
            }
        }

        Ok(Self { agents })
    }

    /// Get agent config by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&CachedAgent> {
        let key = agent_name_key(name);
        self.agents.get(&key)
    }

    /// List all discovered agents.
    pub fn list(&self) -> Vec<&CachedAgent> {
        self.agents.values().collect()
    }

    /// Check if agent requires CLI execution (has tools specified).
    ///
    /// Returns `true` if the agent has a non-empty tools list.
    /// Returns `false` if the agent doesn't exist, has no tools field,
    /// or has an empty tools list.
    pub fn requires_cli(&self, name: &str) -> bool {
        self.get(name)
            .map(|ca| ca.config.tools.as_ref().is_some_and(|t| !t.is_empty()))
            .unwrap_or(false)
    }

    /// Number of agents in registry.
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Is registry empty?
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }
}

/// Returns the standard agent root directories (in priority order).
fn agent_roots(home: &std::path::Path) -> Vec<SkillRoot> {
    vec![
        SkillRoot {
            root: home.join(".codex/agents"),
            source: SkillSource::Codex,
        },
        SkillRoot {
            root: home.join(".claude/agents"),
            source: SkillSource::Claude,
        },
        SkillRoot {
            root: home.join(".agent/agents"),
            source: SkillSource::Agent,
        },
    ]
}

/// Normalize agent name for case-insensitive lookup.
///
/// Converts to lowercase and strips the `.md` extension if present.
fn agent_name_key(name: &str) -> String {
    let name_lower = name.to_ascii_lowercase();
    // Strip .md extension and path components for cleaner matching
    let stem = name_lower.strip_suffix(".md").unwrap_or(&name_lower);
    // Also handle paths like "agents/foo.md" -> "foo"
    stem.rsplit('/').next().unwrap_or(stem).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn create_agent_file(dir: &std::path::Path, name: &str, content: &str) {
        let agents_dir = dir.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(agents_dir.join(name), content).unwrap();
    }

    #[test]
    fn test_discover_single_agent() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("codex");
        fs::create_dir_all(&home).unwrap();

        create_agent_file(
            &home.join(".codex"),
            "test-agent.md",
            r#"---
name: test-agent
description: A test agent
tools: Read, Bash
model: sonnet
---

You are a test agent."#,
        );

        let roots = agent_roots(&home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());

        let agent = registry.get("test-agent").unwrap();
        assert_eq!(agent.config.name, "test-agent");
        assert_eq!(agent.config.description, "A test agent");
        assert_eq!(
            agent.config.tools,
            Some(vec!["Read".to_string(), "Bash".to_string()])
        );
    }

    #[test]
    fn test_case_insensitive_lookup() {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();

        create_agent_file(
            &home.join(".codex"),
            "MyAgent.md",
            r#"---
name: MyAgent
description: Mixed case agent
---

Content."#,
        );

        let roots = agent_roots(&home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        // Should find with various case variations
        assert!(registry.get("MyAgent").is_some());
        assert!(registry.get("myagent").is_some());
        assert!(registry.get("MYAGENT").is_some());
        assert!(registry.get("Myagent").is_some());
    }

    #[test]
    fn test_priority_order_codex_wins() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Create agent in both codex and claude locations
        create_agent_file(
            &home.join(".codex"),
            "shared-agent.md",
            r#"---
name: shared-agent
description: From Codex
---

Codex version."#,
        );

        create_agent_file(
            &home.join(".claude"),
            "shared-agent.md",
            r#"---
name: shared-agent
description: From Claude
---

Claude version."#,
        );

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        assert_eq!(registry.len(), 1);
        let agent = registry.get("shared-agent").unwrap();
        // Codex should win because it's first in priority
        assert_eq!(agent.config.description, "From Codex");
        assert!(matches!(agent.meta.source, SkillSource::Codex));
    }

    #[test]
    fn test_multiple_agents_from_different_sources() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        create_agent_file(
            &home.join(".codex"),
            "codex-agent.md",
            r#"---
name: codex-agent
description: Codex only
---

Content."#,
        );

        create_agent_file(
            &home.join(".claude"),
            "claude-agent.md",
            r#"---
name: claude-agent
description: Claude only
---

Content."#,
        );

        create_agent_file(
            &home.join(".agent"),
            "universal-agent.md",
            r#"---
name: universal-agent
description: Universal only
---

Content."#,
        );

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        assert_eq!(registry.len(), 3);
        assert!(registry.get("codex-agent").is_some());
        assert!(registry.get("claude-agent").is_some());
        assert!(registry.get("universal-agent").is_some());
    }

    #[test]
    fn test_requires_cli_with_tools() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        create_agent_file(
            &home.join(".codex"),
            "tool-agent.md",
            r#"---
name: tool-agent
description: Has tools
tools: Read, Bash, Glob
---

Content."#,
        );

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        assert!(registry.requires_cli("tool-agent"));
    }

    #[test]
    fn test_requires_cli_no_tools() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        create_agent_file(
            &home.join(".codex"),
            "no-tool-agent.md",
            r#"---
name: no-tool-agent
description: No tools field
---

Content."#,
        );

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        assert!(!registry.requires_cli("no-tool-agent"));
    }

    #[test]
    fn test_requires_cli_empty_tools() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        create_agent_file(
            &home.join(".codex"),
            "empty-tool-agent.md",
            r#"---
name: empty-tool-agent
description: Empty tools
tools: ""
---

Content."#,
        );

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        assert!(!registry.requires_cli("empty-tool-agent"));
    }

    #[test]
    fn test_requires_cli_nonexistent_agent() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        assert!(!registry.requires_cli("nonexistent"));
    }

    #[test]
    fn test_list_agents() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        create_agent_file(
            &home.join(".codex"),
            "agent-a.md",
            r#"---
name: agent-a
description: First agent
---

Content."#,
        );

        create_agent_file(
            &home.join(".codex"),
            "agent-b.md",
            r#"---
name: agent-b
description: Second agent
---

Content."#,
        );

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        let list = registry.list();
        assert_eq!(list.len(), 2);

        let names: Vec<&str> = list.iter().map(|a| a.config.name.as_str()).collect();
        assert!(names.contains(&"agent-a"));
        assert!(names.contains(&"agent-b"));
    }

    #[test]
    fn test_empty_registry() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
        assert!(registry.list().is_empty());
    }

    #[test]
    fn test_parse_failure_continues() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Invalid YAML (missing closing bracket)
        create_agent_file(
            &home.join(".codex"),
            "bad-agent.md",
            r#"---
name: [invalid yaml
description: broken
---

Content."#,
        );

        // Valid agent
        create_agent_file(
            &home.join(".codex"),
            "good-agent.md",
            r#"---
name: good-agent
description: Valid agent
---

Content."#,
        );

        let roots = agent_roots(home);
        let registry = AgentRegistry::discover_from_roots(&roots).unwrap();

        // Should have one valid agent
        assert_eq!(registry.len(), 1);
        assert!(registry.get("good-agent").is_some());
        assert!(registry.get("bad-agent").is_none());
    }

    #[test]
    fn test_agent_name_key() {
        assert_eq!(agent_name_key("foo"), "foo");
        assert_eq!(agent_name_key("Foo"), "foo");
        assert_eq!(agent_name_key("FOO"), "foo");
        assert_eq!(agent_name_key("foo.md"), "foo");
        assert_eq!(agent_name_key("Foo.md"), "foo");
        assert_eq!(agent_name_key("agents/foo.md"), "foo");
        assert_eq!(agent_name_key("agents/Foo.md"), "foo");
    }

    #[test]
    fn test_discover_from_custom_home() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        create_agent_file(
            &home.join(".codex"),
            "custom-agent.md",
            r#"---
name: custom-agent
description: Custom home test
---

Content."#,
        );

        let registry = AgentRegistry::discover_from(home).unwrap();

        assert_eq!(registry.len(), 1);
        assert!(registry.get("custom-agent").is_some());
    }
}
