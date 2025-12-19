use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::Result;

/// Represents the origin of a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash, Deserialize)]
#[non_exhaustive]
pub enum SkillSource {
    /// Codex CLI skills directory (`~/.codex/skills`).
    Codex,
    /// Claude Code skills directory (`~/.claude/skills`).
    Claude,
    /// Claude Code marketplace plugins (`~/.claude/plugins/marketplaces`).
    Marketplace,
    /// Claude Code plugin cache (`~/.claude/plugins/cache`).
    Cache,
    /// Codex mirror directory (`~/.codex/skills-mirror`).
    Mirror,
    /// Universal agent skills (`~/.agent/skills`).
    Agent,
    /// Extra user-specified directories (indexed).
    Extra(u32),
}

impl SkillSource {
    /// Returns a stable label for this source.
    pub fn label(&self) -> String {
        match self {
            SkillSource::Codex => "codex".into(),
            SkillSource::Claude => "claude".into(),
            SkillSource::Marketplace => "marketplace".into(),
            SkillSource::Cache => "cache".into(),
            SkillSource::Mirror => "mirror".into(),
            SkillSource::Agent => "agent".into(),
            SkillSource::Extra(n) => format!("extra{n}"),
        }
    }

    /// Returns a human-friendly location tag for diagnostics.
    ///
    /// - `global`: user-level shared skills (`~/.codex`, `~/.claude`, mirror).
    /// - `universal`: cross-agent shared skills (`~/.agent`).
    /// - `project`: extra/user-specified directories.
    pub fn location(&self) -> &'static str {
        match self {
            SkillSource::Codex
            | SkillSource::Claude
            | SkillSource::Marketplace
            | SkillSource::Cache
            | SkillSource::Mirror => "global",
            SkillSource::Agent => "universal",
            SkillSource::Extra(_) => "project",
        }
    }
}

/// Parses a string key into a `SkillSource` variant.
///
/// ```
/// use skrills_discovery::{parse_source_key, SkillSource};
///
/// assert_eq!(parse_source_key("mirror"), Some(SkillSource::Mirror));
/// assert_eq!(parse_source_key("unknown"), None);
/// ```
pub fn parse_source_key(key: &str) -> Option<SkillSource> {
    if key.eq_ignore_ascii_case("codex") {
        Some(SkillSource::Codex)
    } else if key.eq_ignore_ascii_case("claude") {
        Some(SkillSource::Claude)
    } else if key.eq_ignore_ascii_case("marketplace") {
        Some(SkillSource::Marketplace)
    } else if key.eq_ignore_ascii_case("cache") {
        Some(SkillSource::Cache)
    } else if key.eq_ignore_ascii_case("mirror") {
        Some(SkillSource::Mirror)
    } else if key.eq_ignore_ascii_case("agent") {
        Some(SkillSource::Agent)
    } else {
        None
    }
}

/// Represents a root directory where skills are discovered, along with its associated source type.
#[derive(Debug, Clone)]
pub struct SkillRoot {
    /// The root directory path.
    pub root: PathBuf,
    /// The source type for skills in this root.
    pub source: SkillSource,
}

/// Metadata for a discovered skill.
///
/// Includes its name, file path, source of discovery, root directory, and content hash.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SkillMeta {
    /// The skill name (from frontmatter or filename).
    pub name: String,
    /// Path to the SKILL.md file.
    pub path: PathBuf,
    /// The source where this skill was discovered.
    pub source: SkillSource,
    /// The root directory containing this skill.
    pub root: PathBuf,
    /// Content hash for change detection.
    pub hash: String,
}

/// Metadata for a discovered agent definition.
#[derive(Debug, Serialize, Clone)]
pub struct AgentMeta {
    /// The agent name.
    pub name: String,
    /// Path to the agent definition file.
    pub path: PathBuf,
    /// The source where this agent was discovered.
    pub source: SkillSource,
    /// The root directory containing this agent.
    pub root: PathBuf,
    /// Content hash for change detection.
    pub hash: String,
}

impl AgentMeta {
    /// Load and parse the agent configuration from the markdown file.
    ///
    /// Parses YAML frontmatter and extracts the system prompt content.
    pub fn load_config(&self) -> Result<AgentConfig> {
        let content = fs::read_to_string(&self.path)?;
        parse_agent_config(&content, &self.name)
    }

    /// Check if this agent requires local tool access.
    ///
    /// Returns `true` if the agent's tools field is non-empty (needs tool access).
    /// Returns `false` if tools is None (inherit all) or empty (no tools).
    ///
    /// Note: This reads and parses the file; consider caching the result.
    pub fn requires_tools(&self) -> Result<bool> {
        let config = self.load_config()?;
        Ok(config.tools.as_ref().is_some_and(|t| !t.is_empty()))
    }
}

/// Parsed configuration from an agent definition file.
///
/// This struct represents the fully parsed agent configuration, with
/// tools converted from comma-separated strings to vectors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent name.
    pub name: String,
    /// Agent description.
    pub description: String,
    /// Tools the agent can use. None = inherit all, Some([]) = none.
    pub tools: Option<Vec<String>>,
    /// Model to use: sonnet, opus, haiku, or inherit.
    pub model: Option<String>,
    /// Permission mode: default, acceptEdits, bypassPermissions.
    pub permission_mode: Option<String>,
    /// Skills to auto-load for this agent.
    pub skills: Option<Vec<String>>,
    /// System prompt content (markdown after frontmatter).
    pub system_prompt: String,
}

/// Raw YAML frontmatter structure for agent files.
///
/// This intermediate struct handles the raw YAML parsing before
/// conversion to `AgentConfig`.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawAgentFrontmatter {
    /// Agent name.
    name: Option<String>,
    /// Agent description.
    description: Option<String>,
    /// Tools as comma-separated string (Claude Code format).
    tools: Option<String>,
    /// Model to use.
    model: Option<String>,
    /// Permission mode (camelCase in YAML).
    #[serde(rename = "permissionMode")]
    permission_mode: Option<String>,
    /// Skills as comma-separated string.
    skills: Option<String>,
}

/// Split content into frontmatter YAML and body content.
///
/// Returns (frontmatter_yaml, body_content).
fn split_agent_frontmatter(content: &str) -> (Option<String>, String) {
    let trimmed = content.trim_start();

    if !trimmed.starts_with("---") {
        return (None, content.to_string());
    }

    // Find content after opening ---
    let after_open = &trimmed[3..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);

    // Find closing ---
    if let Some(end_pos) = after_open.find("\n---") {
        let yaml = &after_open[..end_pos];
        let rest = &after_open[end_pos + 4..];
        let rest = rest.trim_start_matches(['\r', '\n']);
        (Some(yaml.to_string()), rest.to_string())
    } else if let Some(end_pos) = after_open.find("\r\n---") {
        let yaml = &after_open[..end_pos];
        let rest = &after_open[end_pos + 5..];
        let rest = rest.trim_start_matches(['\r', '\n']);
        (Some(yaml.to_string()), rest.to_string())
    } else {
        // No closing ---, treat entire content as body
        (None, content.to_string())
    }
}

/// Parse comma-separated string into a vector of trimmed strings.
fn parse_comma_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Parse agent configuration from markdown content.
///
/// Extracts YAML frontmatter and converts it to `AgentConfig`.
pub fn parse_agent_config(content: &str, fallback_name: &str) -> Result<AgentConfig> {
    let (yaml_opt, body) = split_agent_frontmatter(content);

    let raw = if let Some(yaml) = yaml_opt {
        serde_yaml::from_str::<RawAgentFrontmatter>(&yaml)
            .map_err(|e| anyhow::anyhow!("Invalid YAML frontmatter: {e}"))?
    } else {
        RawAgentFrontmatter::default()
    };

    // Convert tools from comma-separated string to Vec
    let tools = raw.tools.map(|t| parse_comma_list(&t));

    // Convert skills from comma-separated string to Vec
    let skills = raw.skills.map(|s| parse_comma_list(&s));

    Ok(AgentConfig {
        name: raw.name.unwrap_or_else(|| fallback_name.to_string()),
        description: raw.description.unwrap_or_default(),
        tools,
        model: raw.model,
        permission_mode: raw.permission_mode,
        skills,
        system_prompt: body,
    })
}

/// Information about a duplicate skill that was skipped due to priority.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateInfo {
    /// The skill name that was duplicated.
    pub name: String,
    /// The source label of the skipped skill.
    pub skipped_source: String,
    /// The root path of the skipped skill.
    pub skipped_root: String,
    /// The source label of the kept skill.
    pub kept_source: String,
    /// The root path of the kept skill.
    pub kept_root: String,
}

/// Diagnostic information related to skill processing.
#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Diagnostics {
    /// Skills included in the output (name, source, root, location).
    pub included: Vec<(String, String, String, String)>,
    /// Skills skipped with a reason.
    pub skipped: Vec<(String, String)>,
    /// Duplicate skills encountered and resolved by priority.
    pub duplicates: Vec<DuplicateInfo>,
    /// Indicates if the output was truncated.
    pub truncated: bool,
    /// Indicates if skill content was omitted to fit manifest size.
    pub truncated_content: bool,
    /// The render mode selected for the output.
    pub render_mode: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ============================================================
    // AgentConfig parsing tests
    // ============================================================

    #[test]
    fn test_parse_agent_config_full() {
        let content = r#"---
name: code-reviewer
description: Reviews code for quality and security
tools: Read, Grep, Glob, Bash
model: sonnet
permissionMode: default
skills: review-checklist, security-scan
---

You are an expert code reviewer.

## Guidelines
- Check for security issues
- Ensure code quality
"#;

        let config = parse_agent_config(content, "fallback").unwrap();

        assert_eq!(config.name, "code-reviewer");
        assert_eq!(config.description, "Reviews code for quality and security");
        assert_eq!(
            config.tools,
            Some(vec![
                "Read".to_string(),
                "Grep".to_string(),
                "Glob".to_string(),
                "Bash".to_string()
            ])
        );
        assert_eq!(config.model, Some("sonnet".to_string()));
        assert_eq!(config.permission_mode, Some("default".to_string()));
        assert_eq!(
            config.skills,
            Some(vec![
                "review-checklist".to_string(),
                "security-scan".to_string()
            ])
        );
        assert!(config.system_prompt.contains("expert code reviewer"));
        assert!(config.system_prompt.contains("## Guidelines"));
    }

    #[test]
    fn test_parse_agent_config_minimal() {
        let content = r#"---
name: simple-agent
description: A simple agent
---

Simple system prompt."#;

        let config = parse_agent_config(content, "fallback").unwrap();

        assert_eq!(config.name, "simple-agent");
        assert_eq!(config.description, "A simple agent");
        assert!(config.tools.is_none());
        assert!(config.model.is_none());
        assert!(config.permission_mode.is_none());
        assert!(config.skills.is_none());
        assert_eq!(config.system_prompt, "Simple system prompt.");
    }

    #[test]
    fn test_parse_agent_config_no_frontmatter() {
        let content = "# Just Markdown\n\nNo frontmatter here.";

        let config = parse_agent_config(content, "fallback-name").unwrap();

        assert_eq!(config.name, "fallback-name");
        assert_eq!(config.description, "");
        assert!(config.tools.is_none());
        assert!(config.system_prompt.contains("# Just Markdown"));
    }

    #[test]
    fn test_parse_agent_config_empty_tools() {
        let content = r#"---
name: no-tools-agent
description: Agent with no tools
tools: ""
---

Prompt content."#;

        let config = parse_agent_config(content, "fallback").unwrap();

        // Empty string parses to empty vec
        assert_eq!(config.tools, Some(vec![]));
    }

    #[test]
    fn test_parse_agent_config_tools_with_whitespace() {
        let content = r#"---
name: whitespace-test
description: Tests whitespace handling
tools: "  Read  ,  Grep  ,  Glob  "
---

Content."#;

        let config = parse_agent_config(content, "fallback").unwrap();

        assert_eq!(
            config.tools,
            Some(vec![
                "Read".to_string(),
                "Grep".to_string(),
                "Glob".to_string()
            ])
        );
    }

    #[test]
    fn test_parse_agent_config_invalid_yaml() {
        let content = r#"---
name: [invalid yaml
description: missing bracket
---

Content."#;

        let result = parse_agent_config(content, "fallback");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid YAML"));
    }

    #[test]
    fn test_parse_agent_config_unclosed_frontmatter() {
        // Unclosed frontmatter should treat everything as body
        let content = r#"---
name: unclosed
description: No closing delimiter

This is the body."#;

        let config = parse_agent_config(content, "fallback-name").unwrap();

        // No closing ---, so no frontmatter parsed
        assert_eq!(config.name, "fallback-name");
        assert!(config.system_prompt.contains("name: unclosed"));
    }

    #[test]
    fn test_parse_agent_config_model_variations() {
        for model in ["sonnet", "opus", "haiku", "inherit"] {
            let content = format!(
                r#"---
name: test
description: test
model: {model}
---

Content."#
            );

            let config = parse_agent_config(&content, "fallback").unwrap();
            assert_eq!(config.model, Some(model.to_string()));
        }
    }

    #[test]
    fn test_parse_agent_config_permission_modes() {
        for mode in ["default", "acceptEdits", "bypassPermissions"] {
            let content = format!(
                r#"---
name: test
description: test
permissionMode: {mode}
---

Content."#
            );

            let config = parse_agent_config(&content, "fallback").unwrap();
            assert_eq!(config.permission_mode, Some(mode.to_string()));
        }
    }

    #[test]
    fn test_split_agent_frontmatter_basic() {
        let content = "---\nname: test\n---\nBody content";
        let (yaml, body) = split_agent_frontmatter(content);

        assert!(yaml.is_some());
        assert_eq!(yaml.unwrap(), "name: test");
        assert_eq!(body, "Body content");
    }

    #[test]
    fn test_split_agent_frontmatter_no_frontmatter() {
        let content = "# Just markdown";
        let (yaml, body) = split_agent_frontmatter(content);

        assert!(yaml.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn test_split_agent_frontmatter_leading_whitespace() {
        let content = "  \n  ---\nname: test\n---\nBody";
        let (yaml, _body) = split_agent_frontmatter(content);

        assert!(yaml.is_some());
        assert_eq!(yaml.unwrap(), "name: test");
    }

    #[test]
    fn test_parse_comma_list() {
        assert_eq!(parse_comma_list("a, b, c"), vec!["a", "b", "c"]);
        assert_eq!(parse_comma_list("  a  ,  b  "), vec!["a", "b"]);
        assert_eq!(parse_comma_list("single"), vec!["single"]);
        assert!(parse_comma_list("").is_empty());
        assert!(parse_comma_list("  ,  ,  ").is_empty());
    }

    // ============================================================
    // AgentMeta integration tests
    // ============================================================

    #[test]
    fn test_agent_meta_load_config() {
        let tmp = tempdir().unwrap();
        let agent_path = tmp.path().join("agents/test-agent.md");
        fs::create_dir_all(agent_path.parent().unwrap()).unwrap();
        fs::write(
            &agent_path,
            r#"---
name: test-agent
description: Test agent description
tools: Read, Bash
model: sonnet
---

System prompt here."#,
        )
        .unwrap();

        let meta = AgentMeta {
            name: "test-agent".to_string(),
            path: agent_path,
            source: SkillSource::Codex,
            root: tmp.path().to_path_buf(),
            hash: "abc123".to_string(),
        };

        let config = meta.load_config().unwrap();
        assert_eq!(config.name, "test-agent");
        assert_eq!(
            config.tools,
            Some(vec!["Read".to_string(), "Bash".to_string()])
        );
        assert_eq!(config.model, Some("sonnet".to_string()));
    }

    #[test]
    fn test_agent_meta_requires_tools_true() {
        let tmp = tempdir().unwrap();
        let agent_path = tmp.path().join("agent.md");
        fs::write(
            &agent_path,
            r#"---
name: tools-agent
description: Has tools
tools: Read, Bash
---

Content."#,
        )
        .unwrap();

        let meta = AgentMeta {
            name: "tools-agent".to_string(),
            path: agent_path,
            source: SkillSource::Codex,
            root: tmp.path().to_path_buf(),
            hash: "abc".to_string(),
        };

        assert!(meta.requires_tools().unwrap());
    }

    #[test]
    fn test_agent_meta_requires_tools_false_none() {
        let tmp = tempdir().unwrap();
        let agent_path = tmp.path().join("agent.md");
        fs::write(
            &agent_path,
            r#"---
name: inherit-agent
description: Inherits tools (no tools field)
---

Content."#,
        )
        .unwrap();

        let meta = AgentMeta {
            name: "inherit-agent".to_string(),
            path: agent_path,
            source: SkillSource::Codex,
            root: tmp.path().to_path_buf(),
            hash: "abc".to_string(),
        };

        // None means inherit, which is not "requires specific tools"
        assert!(!meta.requires_tools().unwrap());
    }

    #[test]
    fn test_agent_meta_requires_tools_false_empty() {
        let tmp = tempdir().unwrap();
        let agent_path = tmp.path().join("agent.md");
        fs::write(
            &agent_path,
            r#"---
name: no-tools-agent
description: Explicitly no tools
tools: ""
---

Content."#,
        )
        .unwrap();

        let meta = AgentMeta {
            name: "no-tools-agent".to_string(),
            path: agent_path,
            source: SkillSource::Codex,
            root: tmp.path().to_path_buf(),
            hash: "abc".to_string(),
        };

        assert!(!meta.requires_tools().unwrap());
    }

    #[test]
    fn test_agent_meta_load_config_file_not_found() {
        let meta = AgentMeta {
            name: "missing".to_string(),
            path: PathBuf::from("/nonexistent/path/agent.md"),
            source: SkillSource::Codex,
            root: PathBuf::from("/nonexistent"),
            hash: "abc".to_string(),
        };

        assert!(meta.load_config().is_err());
    }
}
