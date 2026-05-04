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
    /// GitHub Copilot CLI skills directory (`~/.copilot/skills`).
    Copilot,
    /// Claude Code marketplace plugins (`~/.claude/plugins/marketplaces`).
    Marketplace,
    /// Claude Code plugin cache (`~/.claude/plugins/cache`).
    Cache,
    /// Codex mirror directory (`~/.codex/skills-mirror`).
    Mirror,
    /// Cursor IDE rules directory (`.cursor/rules`).
    Cursor,
    /// Universal agent skills (`~/.agent/skills`).
    Agent,
    /// Extra user-specified directories (indexed).
    Extra(u32),
}

impl std::fmt::Display for SkillSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

impl SkillSource {
    /// Returns a stable label for this source.
    pub fn label(&self) -> String {
        match self {
            SkillSource::Codex => "codex".into(),
            SkillSource::Claude => "claude".into(),
            SkillSource::Copilot => "copilot".into(),
            SkillSource::Cursor => "cursor".into(),
            SkillSource::Marketplace => "marketplace".into(),
            SkillSource::Cache => "cache".into(),
            SkillSource::Mirror => "mirror".into(),
            SkillSource::Agent => "agent".into(),
            SkillSource::Extra(n) => format!("extra{n}"),
        }
    }

    /// Returns a human-friendly location tag for diagnostics.
    ///
    /// - `global`: user-level shared skills (`~/.codex`, `~/.claude`, `~/.copilot`, mirror).
    /// - `universal`: cross-agent shared skills (`~/.agent`).
    /// - `project`: extra/user-specified directories.
    pub fn location(&self) -> &'static str {
        match self {
            SkillSource::Codex
            | SkillSource::Claude
            | SkillSource::Copilot
            | SkillSource::Marketplace
            | SkillSource::Cache
            | SkillSource::Mirror => "global",
            SkillSource::Cursor => "project",
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
/// assert_eq!(parse_source_key("copilot"), Some(SkillSource::Copilot));
/// assert_eq!(parse_source_key("unknown"), None);
/// ```
pub fn parse_source_key(key: &str) -> Option<SkillSource> {
    if key.eq_ignore_ascii_case("codex") {
        Some(SkillSource::Codex)
    } else if key.eq_ignore_ascii_case("claude") {
        Some(SkillSource::Claude)
    } else if key.eq_ignore_ascii_case("copilot") {
        Some(SkillSource::Copilot)
    } else if key.eq_ignore_ascii_case("marketplace") {
        Some(SkillSource::Marketplace)
    } else if key.eq_ignore_ascii_case("cache") {
        Some(SkillSource::Cache)
    } else if key.eq_ignore_ascii_case("mirror") {
        Some(SkillSource::Mirror)
    } else if key.eq_ignore_ascii_case("cursor") {
        Some(SkillSource::Cursor)
    } else if key.eq_ignore_ascii_case("agent") {
        Some(SkillSource::Agent)
    } else {
        None
    }
}

/// Category of a hookify rule.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleCategory {
    /// Pre-commit hook rules.
    PreCommit,
    /// Post-commit hook rules.
    PostCommit,
    /// Pre-push hook rules.
    PrePush,
    /// User-prompt-submit hook rules.
    PromptSubmit,
    /// Notification hook rules.
    Notification,
    /// Other/custom hook rules.
    Other(String),
}

impl RuleCategory {
    /// Returns the kebab-case identifier (`"pre-commit"`, `"post-commit"`, etc.) used in rule manifests.
    pub fn as_str(&self) -> &str {
        match self {
            Self::PreCommit => "pre-commit",
            Self::PostCommit => "post-commit",
            Self::PrePush => "pre-push",
            Self::PromptSubmit => "prompt-submit",
            Self::Notification => "notification",
            Self::Other(s) => s,
        }
    }
}

impl std::fmt::Display for RuleCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Discovered hookify rule metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleMeta {
    /// Rule name.
    pub name: String,
    /// Path to the rule configuration file.
    pub path: PathBuf,
    /// Discovery source (e.g. "claude", "project").
    pub source: String,
    /// Rule category/trigger event.
    pub category: RuleCategory,
    /// Whether the rule is currently enabled.
    pub enabled: bool,
    /// Optional description of the rule.
    pub description: Option<String>,
    /// Command or script the rule executes.
    pub command: Option<String>,
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
    /// Optional description from frontmatter (cached for search).
    ///
    /// # Invariant
    ///
    /// When present (`Some`), the description MUST be non-empty after trimming.
    /// An empty or whitespace-only description should be stored as `None`.
    /// Use [`has_valid_description`](Self::has_valid_description) to check this invariant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional frontmatter `name` field (used for cross-root deduplication).
    ///
    /// When skills from different roots share the same frontmatter name and
    /// similar descriptions, they are treated as duplicates regardless of path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter_name: Option<String>,
}

impl SkillMeta {
    /// Returns `true` if this skill has a valid (non-empty) description.
    ///
    /// A description is considered valid if it is `Some` and contains
    /// non-whitespace characters. This enforces the invariant that
    /// `Some("")` or `Some("   ")` should not occur.
    ///
    /// # Examples
    ///
    /// ```
    /// use skrills_discovery::SkillMeta;
    /// use skrills_discovery::SkillSource;
    /// use std::path::PathBuf;
    ///
    /// let mut meta = SkillMeta {
    ///     name: "test".to_string(),
    ///     path: PathBuf::from("test.md"),
    ///     source: SkillSource::Claude,
    ///     root: PathBuf::from("/skills"),
    ///     hash: "abc123".to_string(),
    ///     description: Some("A helpful skill".to_string()),
    ///     frontmatter_name: None,
    /// };
    /// assert!(meta.has_valid_description());
    ///
    /// meta.description = Some("".to_string());
    /// assert!(!meta.has_valid_description());
    ///
    /// meta.description = None;
    /// assert!(!meta.has_valid_description());
    /// ```
    pub fn has_valid_description(&self) -> bool {
        self.description
            .as_ref()
            .is_some_and(|d| !d.trim().is_empty())
    }
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

/// Model selection for agent configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentModel {
    /// Claude Sonnet model.
    Sonnet,
    /// Claude Opus model.
    Opus,
    /// Claude Haiku model.
    Haiku,
    /// Inherit model from parent context.
    Inherit,
    /// Unknown or custom model string.
    #[serde(untagged)]
    Other(String),
}

impl AgentModel {
    /// Returns the canonical model identifier (`"sonnet"`, `"opus"`, `"haiku"`, `"inherit"`) or the raw `Other` string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Sonnet => "sonnet",
            Self::Opus => "opus",
            Self::Haiku => "haiku",
            Self::Inherit => "inherit",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for AgentModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parsed configuration from an agent definition file.
///
/// This struct represents the fully parsed agent configuration, with
/// tools converted from comma-separated strings to vectors.
///
/// # Agent Definition Format
///
/// Agent files use YAML frontmatter followed by markdown content:
///
/// ```yaml
/// ---
/// name: code-reviewer
/// description: Reviews code for bugs and style issues
/// tools: Read, Grep, Glob
/// model: sonnet
/// permissionMode: default
/// skills: superpowers:code-review
/// ---
///
/// You are an expert code reviewer. Analyze the provided code for:
/// - Bugs and logic errors
/// - Style violations
/// - Security issues
/// ```
///
/// The `tools` and `skills` fields accept comma-separated strings in the
/// YAML frontmatter, which are parsed into `Vec<String>` in `AgentConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent name.
    pub name: String,
    /// Agent description.
    pub description: String,
    /// Tools the agent can use. None = inherit all, Some([]) = none.
    pub tools: Option<Vec<String>>,
    /// Model to use.
    pub model: Option<AgentModel>,
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
        serde_yaml::from_str::<RawAgentFrontmatter>(&yaml).map_err(crate::DiscoveryError::from)?
    } else {
        RawAgentFrontmatter::default()
    };

    // Convert tools from comma-separated string to Vec
    let tools = raw.tools.map(|t| parse_comma_list(&t));

    // Convert skills from comma-separated string to Vec
    let skills = raw.skills.map(|s| parse_comma_list(&s));

    // Parse model string into AgentModel enum
    let model = raw.model.map(|m| match m.to_lowercase().as_str() {
        "sonnet" => AgentModel::Sonnet,
        "opus" => AgentModel::Opus,
        "haiku" => AgentModel::Haiku,
        "inherit" => AgentModel::Inherit,
        _ => AgentModel::Other(m),
    });

    Ok(AgentConfig {
        name: raw.name.unwrap_or_else(|| fallback_name.to_string()),
        description: raw.description.unwrap_or_default(),
        tools,
        model,
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

/// A skill that was included in the output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IncludedSkill {
    /// The skill name.
    pub name: String,
    /// The discovery source label.
    pub source: String,
    /// The root directory path.
    pub root: String,
    /// The location tag (global/project/universal).
    pub location: String,
}

/// A skill that was skipped during processing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkippedSkill {
    /// The skill name.
    pub name: String,
    /// The reason the skill was skipped.
    pub reason: String,
}

/// Diagnostic information related to skill processing.
#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Diagnostics {
    /// Skills included in the output.
    pub included: Vec<IncludedSkill>,
    /// Skills skipped with a reason.
    pub skipped: Vec<SkippedSkill>,
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
        assert_eq!(config.model, Some(AgentModel::Sonnet));
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
        let cases = [
            ("sonnet", AgentModel::Sonnet),
            ("opus", AgentModel::Opus),
            ("haiku", AgentModel::Haiku),
            ("inherit", AgentModel::Inherit),
        ];
        for (model_str, expected) in cases {
            let content = format!(
                r#"---
name: test
description: test
model: {model_str}
---

Content."#
            );

            let config = parse_agent_config(&content, "fallback").unwrap();
            assert_eq!(config.model, Some(expected));
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
        assert_eq!(config.model, Some(AgentModel::Sonnet));
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

    // ============================================================
    // SkillMeta tests
    // ============================================================

    fn make_skill_meta(description: Option<String>) -> SkillMeta {
        SkillMeta {
            name: "test-skill".to_string(),
            path: PathBuf::from("/skills/test.md"),
            source: SkillSource::Claude,
            root: PathBuf::from("/skills"),
            hash: "abc123".to_string(),
            description,
            frontmatter_name: None,
        }
    }

    #[test]
    fn test_has_valid_description_with_content() {
        let meta = make_skill_meta(Some("A helpful skill".to_string()));
        assert!(meta.has_valid_description());
    }

    #[test]
    fn test_has_valid_description_empty_string() {
        let meta = make_skill_meta(Some("".to_string()));
        assert!(!meta.has_valid_description());
    }

    #[test]
    fn test_has_valid_description_whitespace_only() {
        let meta = make_skill_meta(Some("   \t\n  ".to_string()));
        assert!(!meta.has_valid_description());
    }

    #[test]
    fn test_has_valid_description_none() {
        let meta = make_skill_meta(None);
        assert!(!meta.has_valid_description());
    }

    #[test]
    fn test_has_valid_description_with_leading_trailing_whitespace() {
        // Should still be valid if there's actual content
        let meta = make_skill_meta(Some("  valid content  ".to_string()));
        assert!(meta.has_valid_description());
    }

    // ============================================================
    // RuleCategory tests
    // ============================================================

    #[test]
    fn rule_category_as_str_known_variants() {
        assert_eq!(RuleCategory::PreCommit.as_str(), "pre-commit");
        assert_eq!(RuleCategory::PostCommit.as_str(), "post-commit");
        assert_eq!(RuleCategory::PrePush.as_str(), "pre-push");
        assert_eq!(RuleCategory::PromptSubmit.as_str(), "prompt-submit");
        assert_eq!(RuleCategory::Notification.as_str(), "notification");
    }

    #[test]
    fn rule_category_as_str_other() {
        let cat = RuleCategory::Other("custom-hook".to_string());
        assert_eq!(cat.as_str(), "custom-hook");
    }

    #[test]
    fn rule_category_display() {
        assert_eq!(RuleCategory::PreCommit.to_string(), "pre-commit");
        assert_eq!(
            RuleCategory::Other("my-hook".to_string()).to_string(),
            "my-hook"
        );
    }

    #[test]
    fn rule_category_serialization_roundtrip() {
        let categories = vec![
            RuleCategory::PreCommit,
            RuleCategory::PostCommit,
            RuleCategory::PrePush,
            RuleCategory::PromptSubmit,
            RuleCategory::Notification,
            RuleCategory::Other("custom".to_string()),
        ];
        for cat in categories {
            let json = serde_json::to_string(&cat).unwrap();
            let deserialized: RuleCategory = serde_json::from_str(&json).unwrap();
            assert_eq!(cat, deserialized);
        }
    }

    // ============================================================
    // RuleMeta tests
    // ============================================================

    #[test]
    fn rule_meta_serialization_roundtrip() {
        let rule = RuleMeta {
            name: "pre-commit-lint".to_string(),
            path: PathBuf::from("/home/user/.claude/hooks/pre-commit-lint.json"),
            source: "user".to_string(),
            category: RuleCategory::PreCommit,
            enabled: true,
            description: Some("Runs linter before commit".to_string()),
            command: Some("cargo clippy".to_string()),
        };
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: RuleMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "pre-commit-lint");
        assert_eq!(deserialized.category, RuleCategory::PreCommit);
        assert!(deserialized.enabled);
        assert_eq!(
            deserialized.description,
            Some("Runs linter before commit".to_string())
        );
    }

    #[test]
    fn rule_meta_with_none_fields() {
        let rule = RuleMeta {
            name: "basic".to_string(),
            path: PathBuf::from("/tmp/basic.json"),
            source: "project".to_string(),
            category: RuleCategory::Other("misc".to_string()),
            enabled: false,
            description: None,
            command: None,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: RuleMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "basic");
        assert!(!deserialized.enabled);
        assert!(deserialized.description.is_none());
        assert!(deserialized.command.is_none());
    }
}
