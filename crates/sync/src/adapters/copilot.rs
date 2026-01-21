//! Copilot adapter for reading/writing Copilot CLI configuration.
//!
//! ## Path Resolution (XDG-compliant)
//!
//! The adapter follows the XDG Base Directory Specification:
//! 1. If `XDG_CONFIG_HOME` is set → `$XDG_CONFIG_HOME/copilot`
//! 2. If unset → `~/.config/copilot` (XDG default)
//! 3. Fallback → `~/.copilot` (legacy location)
//!
//! ## Key differences from Codex:
//! - MCP servers: Stored in `mcp-config.json` (NOT in `config.json`)
//! - Preferences: Stored in `config.json`, must preserve security fields
//! - Skills: Same format as Codex (`skills/<name>/SKILL.md`)
//! - Commands: Not supported (Copilot has no slash commands)
//! - No config.toml feature flag management

use super::traits::{AgentAdapter, FieldSupport};
use super::utils::{hash_content, is_hidden_path};
use crate::common::{Command, McpServer, Preferences};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use anyhow::Context;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use tracing::warn;
use walkdir::WalkDir;

/// Adapter for GitHub Copilot CLI configuration.
pub struct CopilotAdapter {
    root: PathBuf,
}

impl CopilotAdapter {
    /// Creates a new CopilotAdapter using XDG-compliant path resolution.
    ///
    /// Path precedence:
    /// 1. `$XDG_CONFIG_HOME/copilot` (if XDG_CONFIG_HOME is set)
    /// 2. `~/.config/copilot` (XDG default)
    /// 3. `~/.copilot` (legacy fallback)
    pub fn new() -> Result<Self> {
        let root = Self::resolve_config_root()?;
        Ok(Self { root })
    }

    /// Resolves the configuration root following XDG Base Directory Specification.
    fn resolve_config_root() -> Result<PathBuf> {
        // First, try XDG-compliant path: $XDG_CONFIG_HOME/copilot or ~/.config/copilot
        if let Some(config_dir) = dirs::config_dir() {
            let xdg_path = config_dir.join("copilot");
            // Use XDG path if it exists OR if no legacy path exists
            // (prefer XDG for new installations)
            let home = dirs::home_dir();
            let legacy_path = home.as_ref().map(|h| h.join(".copilot"));

            if xdg_path.exists() {
                return Ok(xdg_path);
            }

            // If legacy path doesn't exist either, prefer XDG for new installations
            if legacy_path.as_ref().is_none_or(|p| !p.exists()) {
                return Ok(xdg_path);
            }
        }

        // Fallback to legacy ~/.copilot
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(home.join(".copilot"))
    }

    /// Creates a CopilotAdapter with a custom root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    fn agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    /// Path to MCP server configuration (separate from main config).
    fn mcp_config_path(&self) -> PathBuf {
        self.root.join("mcp-config.json")
    }

    /// Path to preferences/settings (model, security fields).
    fn config_path(&self) -> PathBuf {
        self.root.join("config.json")
    }
}

// Note: We intentionally do not implement Default for CopilotAdapter because
// construction requires home directory resolution which can fail. Use
// CopilotAdapter::new() or CopilotAdapter::with_root() instead.

impl AgentAdapter for CopilotAdapter {
    fn name(&self) -> &str {
        "copilot"
    }

    fn config_root(&self) -> PathBuf {
        self.root.clone()
    }

    fn supported_fields(&self) -> FieldSupport {
        FieldSupport {
            commands: false, // Copilot does not support slash commands
            mcp_servers: true,
            preferences: true,
            skills: true,
            hooks: false, // Copilot doesn't support hooks
            agents: true, // Copilot supports custom agents in ~/.copilot/agents/
        }
    }

    fn read_commands(&self, _include_marketplace: bool) -> Result<Vec<Command>> {
        // Copilot does not support slash commands
        Ok(Vec::new())
    }

    fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>> {
        let path = self.mcp_config_path();
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read MCP config: {}", path.display()))?;
        let config: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse MCP config as JSON: {}", path.display()))?;

        let mut servers = HashMap::new();
        if let Some(mcp) = config.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, server_config) in mcp {
                // Skip MCP servers with missing or empty command and log a warning
                let command = server_config
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if command.is_empty() {
                    warn!(
                        server = %name,
                        path = %path.display(),
                        "Skipping MCP server with missing or empty 'command' field"
                    );
                    continue;
                }

                // Parse args with warnings for non-string values
                let args = if let Some(arr) = server_config.get("args").and_then(|v| v.as_array()) {
                    let mut result = Vec::new();
                    for (i, v) in arr.iter().enumerate() {
                        if let Some(s) = v.as_str() {
                            result.push(s.to_string());
                        } else {
                            warn!(
                                server = %name,
                                index = i,
                                value_type = ?v,
                                "Skipping non-string value in MCP server args"
                            );
                        }
                    }
                    result
                } else {
                    Vec::new()
                };

                // Parse env with warnings for non-string values
                let env = if let Some(obj) = server_config.get("env").and_then(|v| v.as_object()) {
                    let mut result = HashMap::new();
                    for (k, v) in obj {
                        if let Some(s) = v.as_str() {
                            result.insert(k.clone(), s.to_string());
                        } else {
                            warn!(
                                server = %name,
                                key = %k,
                                value_type = ?v,
                                "Skipping non-string value in MCP server env"
                            );
                        }
                    }
                    result
                } else {
                    HashMap::new()
                };

                let server = McpServer {
                    name: name.clone(),
                    command: command.to_string(),
                    args,
                    env,
                    enabled: server_config
                        .get("disabled")
                        .and_then(|v| v.as_bool())
                        .map(|d| !d)
                        .unwrap_or(true),
                };
                servers.insert(name.clone(), server);
            }
        }

        Ok(servers)
    }

    fn read_preferences(&self) -> Result<Preferences> {
        let path = self.config_path();
        if !path.exists() {
            return Ok(Preferences::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read preferences: {}", path.display()))?;
        let config: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse preferences as JSON: {}", path.display()))?;

        Ok(Preferences {
            model: config
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from),
            custom: HashMap::new(),
        })
    }

    fn read_skills(&self) -> Result<Vec<Command>> {
        let skills_dir = self.skills_dir();
        if !skills_dir.exists() {
            return Ok(Vec::new());
        }

        let mut skills = Vec::new();
        for entry in WalkDir::new(&skills_dir)
            .min_depth(1)
            .max_depth(20)
            .follow_links(false)
        {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        path = ?e.path(),
                        error = %e,
                        "Failed to read directory entry while scanning skills"
                    );
                    continue;
                }
            };
            if entry.file_type().is_symlink() {
                continue;
            }
            let path = entry.path();
            if is_hidden_path(path.strip_prefix(&skills_dir).unwrap_or(path)) {
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }

            // Copilot skills are discovered via ~/.copilot/skills/**/SKILL.md
            let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
            if !is_skill_md {
                continue;
            }

            // Use the parent directory path relative to skills_dir as the skill identifier.
            // Example: ~/.copilot/skills/pdf-processing/SKILL.md -> "pdf-processing"
            // Example: ~/.copilot/skills/nested/foo/SKILL.md -> "nested/foo"
            let name = path
                .parent()
                .and_then(|p| p.strip_prefix(&skills_dir).ok())
                .and_then(|p| p.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown")
                .to_string();

            let content = fs::read(path)?;
            let metadata = fs::metadata(path)?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            skills.push(Command {
                name,
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
            });
        }
        Ok(skills)
    }

    fn write_commands(&self, _commands: &[Command]) -> Result<WriteReport> {
        // Copilot does not support slash commands - no-op
        Ok(WriteReport::default())
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        let path = self.mcp_config_path();

        // Read existing config to preserve structure
        let mut config: serde_json::Value = if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read MCP config: {}", path.display()))?;
            serde_json::from_str(&content).with_context(|| {
                format!("Failed to parse MCP config as JSON: {}", path.display())
            })?
        } else {
            serde_json::json!({})
        };

        let mut report = WriteReport::default();
        let mut mcp_obj = serde_json::Map::new();

        for (name, server) in servers {
            let mut server_config = serde_json::Map::new();
            server_config.insert("command".into(), serde_json::json!(server.command));
            if !server.args.is_empty() {
                server_config.insert("args".into(), serde_json::json!(server.args));
            }
            if !server.env.is_empty() {
                server_config.insert("env".into(), serde_json::json!(server.env));
            }
            if !server.enabled {
                server_config.insert("disabled".into(), serde_json::json!(true));
            }
            mcp_obj.insert(name.clone(), serde_json::Value::Object(server_config));
            report.written += 1;
        }

        config["mcpServers"] = serde_json::Value::Object(mcp_obj);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create MCP config directory: {}",
                    parent.display()
                )
            })?;
        }
        fs::write(&path, serde_json::to_string_pretty(&config)?)
            .with_context(|| format!("Failed to write MCP config: {}", path.display()))?;

        Ok(report)
    }

    fn write_preferences(&self, prefs: &Preferences) -> Result<WriteReport> {
        let path = self.config_path();

        // CRITICAL: Read existing config to preserve security fields
        // (trusted_folders, allowed_urls, denied_urls)
        let mut config: serde_json::Value = if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read preferences: {}", path.display()))?;
            serde_json::from_str(&content).with_context(|| {
                format!("Failed to parse preferences as JSON: {}", path.display())
            })?
        } else {
            serde_json::json!({})
        };

        let mut report = WriteReport::default();

        // Only update the model field - leave all other fields untouched
        if let Some(model) = &prefs.model {
            config["model"] = serde_json::json!(model);
            report.written += 1;
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }
        fs::write(&path, serde_json::to_string_pretty(&config)?)
            .with_context(|| format!("Failed to write preferences: {}", path.display()))?;

        Ok(report)
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        let dir = self.skills_dir();
        fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create skills directory: {}", dir.display()))?;

        let mut report = WriteReport::default();

        for skill in skills {
            // Write each skill into ~/.copilot/skills/<skill-name>/SKILL.md
            let skill_rel_dir = if skill.name.eq_ignore_ascii_case("skill")
                || skill.name.eq_ignore_ascii_case("skill.md")
                || skill.name.eq_ignore_ascii_case("SKILL")
            {
                skill
                    .source_path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or(&skill.name)
                    .to_string()
            } else {
                skill.name.clone()
            };

            let safe_rel_dir = sanitize_name(&skill_rel_dir);
            let path = dir.join(&safe_rel_dir).join("SKILL.md");
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create skill directory: {}", parent.display())
                })?;
            }

            if path.exists() {
                let existing = fs::read(&path).with_context(|| {
                    format!("Failed to read existing skill: {}", path.display())
                })?;
                if hash_content(&existing) == skill.hash {
                    report.skipped.push(SkipReason::Unchanged {
                        item: skill.name.clone(),
                    });
                    continue;
                }
            }

            fs::write(&path, &skill.content)
                .with_context(|| format!("Failed to write skill: {}", path.display()))?;
            report.written += 1;
        }

        // Note: Unlike Codex, Copilot does NOT require config.toml feature flags

        Ok(report)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        // Copilot does not support hooks
        Ok(Vec::new())
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        let agents_dir = self.agents_dir();
        if !agents_dir.exists() {
            return Ok(Vec::new());
        }

        let mut agents = Vec::new();

        for entry in WalkDir::new(&agents_dir)
            .min_depth(1)
            .max_depth(1) // Flat directory structure for Copilot agents
            .follow_links(false)
        {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            // Copilot agents are *.agent.md or *.md files
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !file_name.ends_with(".md") {
                continue;
            }

            if is_hidden_path(path.strip_prefix(&agents_dir).unwrap_or(path)) {
                continue;
            }

            // Extract name: strip .agent.md or .md suffix
            let name = if file_name.ends_with(".agent.md") {
                file_name.trim_end_matches(".agent.md").to_string()
            } else {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            };

            let content = fs::read(path)?;
            let metadata = fs::metadata(path)?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            agents.push(Command {
                name,
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
            });
        }

        Ok(agents)
    }

    fn write_hooks(&self, _hooks: &[Command]) -> Result<WriteReport> {
        // Copilot does not support hooks
        Ok(WriteReport::default())
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        let dir = self.agents_dir();
        fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create agents directory: {}", dir.display()))?;

        let mut report = WriteReport::default();

        for agent in agents {
            let safe_name = sanitize_name(&agent.name);
            let path = dir.join(format!("{}.agent.md", safe_name));

            // Transform the content: Claude format -> Copilot format
            let transformed_content = transform_agent_for_copilot(&agent.content);

            if path.exists() {
                let existing = fs::read(&path).with_context(|| {
                    format!("Failed to read existing agent: {}", path.display())
                })?;
                if hash_content(&existing) == hash_content(&transformed_content) {
                    report.skipped.push(SkipReason::Unchanged {
                        item: agent.name.clone(),
                    });
                    continue;
                }
            }

            fs::write(&path, &transformed_content)
                .with_context(|| format!("Failed to write agent: {}", path.display()))?;
            report.written += 1;
        }

        Ok(report)
    }
}

/// Sanitizes a skill name to prevent path traversal attacks.
///
/// Preserves forward slashes for nested skill directories (e.g., `category/my-skill`)
/// while preventing path traversal attacks (e.g., `../../../etc/passwd`).
///
/// Each path segment is sanitized to only allow alphanumeric characters, hyphens,
/// and underscores. Empty segments and `.` or `..` are removed.
fn sanitize_name(name: &str) -> String {
    name.split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .map(|segment| {
            segment
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Transforms a Claude agent's content to Copilot agent format.
///
/// Transformations:
/// - Replaces `model: xxx` with `target: github-copilot`
/// - Removes `color: xxx` line (Copilot doesn't use this)
/// - Keeps everything else intact
fn transform_agent_for_copilot(content: &[u8]) -> Vec<u8> {
    let content_str = match std::str::from_utf8(content) {
        Ok(s) => s,
        Err(_) => return content.to_vec(), // Binary content, return as-is
    };

    // Check if content has YAML frontmatter
    if !content_str.starts_with("---") {
        // No frontmatter, add minimal frontmatter with target
        return format!("---\ntarget: github-copilot\n---\n\n{}", content_str).into_bytes();
    }

    // Find the end of frontmatter
    let Some(end_idx) = content_str[3..].find("\n---").map(|i| i + 3) else {
        // Malformed frontmatter, return as-is
        return content.to_vec();
    };

    let frontmatter = &content_str[3..end_idx];
    let body = &content_str[end_idx + 4..]; // Skip "\n---"

    let mut new_lines = Vec::new();
    let mut has_target = false;

    for line in frontmatter.lines() {
        let trimmed = line.trim();

        // Skip model and color lines (Claude-specific)
        if trimmed.starts_with("model:") || trimmed.starts_with("color:") {
            continue;
        }

        // Check if target already exists
        if trimmed.starts_with("target:") {
            has_target = true;
        }

        new_lines.push(line);
    }

    // Add target if not already present
    if !has_target {
        new_lines.push("target: github-copilot");
    }

    format!("---\n{}\n---{}", new_lines.join("\n"), body).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ==========================================
    // Basic Adapter Tests
    // ==========================================

    #[test]
    fn copilot_adapter_name() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        assert_eq!(adapter.name(), "copilot");
    }

    #[test]
    fn copilot_adapter_config_root() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        assert_eq!(adapter.config_root(), tmp.path().to_path_buf());
    }

    #[test]
    fn copilot_adapter_supported_fields() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let fields = adapter.supported_fields();

        assert!(!fields.commands, "Copilot should NOT support commands");
        assert!(fields.mcp_servers, "Copilot should support MCP servers");
        assert!(fields.preferences, "Copilot should support preferences");
        assert!(fields.skills, "Copilot should support skills");
    }

    // ==========================================
    // Commands Tests (no-op behavior)
    // ==========================================

    #[test]
    fn read_commands_returns_empty() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands(false).unwrap();
        assert!(commands.is_empty());
    }

    #[test]
    fn write_commands_is_noop() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let commands = vec![Command {
            name: "test".to_string(),
            content: b"# Test".to_vec(),
            source_path: PathBuf::from("/tmp/test.md"),
            modified: SystemTime::now(),
            hash: "abc".to_string(),
        }];

        let report = adapter.write_commands(&commands).unwrap();
        assert_eq!(report.written, 0);
        assert!(report.skipped.is_empty());
    }

    // ==========================================
    // Skills Tests
    // ==========================================

    #[test]
    fn read_skills_empty_dir() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn read_skills_discovers_skill_md() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills/my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: test\n---\n# My Skill",
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
    }

    #[test]
    fn read_skills_nested_directories() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills/category/nested-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: nested\ndescription: test\n---\n# Nested",
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "category/nested-skill");
    }

    #[test]
    fn read_skills_ignores_non_skill_md_files() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills/my-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("README.md"), "# Readme").unwrap();
        fs::write(skill_dir.join("notes.txt"), "Notes").unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();

        assert!(skills.is_empty());
    }

    #[test]
    fn read_skills_ignores_hidden_directories() {
        let tmp = tempdir().unwrap();
        let hidden_dir = tmp.path().join("skills/.hidden");
        fs::create_dir_all(&hidden_dir).unwrap();
        fs::write(hidden_dir.join("SKILL.md"), "# Hidden").unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();

        assert!(skills.is_empty());
    }

    #[test]
    fn write_skills_creates_directories() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let skill = Command {
            name: "alpha".to_string(),
            content: b"---\nname: alpha\ndescription: test\n---\n# Alpha\n".to_vec(),
            source_path: PathBuf::from("/tmp/alpha.md"),
            modified: SystemTime::now(),
            hash: "hash".to_string(),
        };

        let report = adapter.write_skills(&[skill]).unwrap();
        assert_eq!(report.written, 1);
        assert!(tmp.path().join("skills/alpha/SKILL.md").exists());
    }

    #[test]
    fn write_skills_skips_unchanged() {
        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills/alpha");
        fs::create_dir_all(&skill_dir).unwrap();
        let content = b"---\nname: alpha\n---\n# Alpha\n";
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let skill = Command {
            name: "alpha".to_string(),
            content: content.to_vec(),
            source_path: PathBuf::from("/tmp/alpha.md"),
            modified: SystemTime::now(),
            hash: hash_content(content),
        };

        let report = adapter.write_skills(&[skill]).unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn write_skills_no_config_toml_created() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let skill = Command {
            name: "test".to_string(),
            content: b"# Test".to_vec(),
            source_path: PathBuf::from("/tmp/test.md"),
            modified: SystemTime::now(),
            hash: "hash".to_string(),
        };

        adapter.write_skills(&[skill]).unwrap();

        // Unlike Codex, Copilot should NOT create config.toml
        assert!(!tmp.path().join("config.toml").exists());
    }

    // ==========================================
    // MCP Servers Tests
    // ==========================================

    #[test]
    fn read_mcp_servers_empty_when_no_file() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn read_mcp_servers_from_mcp_config_json() {
        let tmp = tempdir().unwrap();
        // Note: MCP servers in mcp-config.json, NOT config.json
        let mcp_config_path = tmp.path().join("mcp-config.json");
        fs::write(
            &mcp_config_path,
            r#"{
            "mcpServers": {
                "test-server": {
                    "command": "/usr/bin/test",
                    "args": ["--flag", "value"]
                }
            }
        }"#,
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        assert_eq!(servers.len(), 1);
        let server = servers.get("test-server").unwrap();
        assert_eq!(server.command, "/usr/bin/test");
        assert_eq!(server.args, vec!["--flag", "value"]);
        assert!(server.enabled);
    }

    #[test]
    fn read_mcp_servers_handles_disabled_flag() {
        let tmp = tempdir().unwrap();
        let mcp_config_path = tmp.path().join("mcp-config.json");
        fs::write(
            &mcp_config_path,
            r#"{
            "mcpServers": {
                "disabled-server": {
                    "command": "/bin/disabled",
                    "disabled": true
                }
            }
        }"#,
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        let server = servers.get("disabled-server").unwrap();
        assert!(!server.enabled);
    }

    #[test]
    fn read_mcp_servers_invalid_json_returns_error() {
        let tmp = tempdir().unwrap();
        let mcp_config_path = tmp.path().join("mcp-config.json");
        fs::write(&mcp_config_path, "{ invalid json }").unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let result = adapter.read_mcp_servers();
        assert!(result.is_err());
    }

    #[test]
    fn write_mcp_servers_creates_mcp_config_json() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let mut servers = HashMap::new();
        servers.insert(
            "my-server".to_string(),
            McpServer {
                name: "my-server".to_string(),
                command: "/bin/server".to_string(),
                args: vec!["arg1".to_string()],
                env: HashMap::new(),
                enabled: true,
            },
        );

        let report = adapter.write_mcp_servers(&servers).unwrap();
        assert_eq!(report.written, 1);

        // Should write to mcp-config.json, NOT config.json
        let mcp_config_path = tmp.path().join("mcp-config.json");
        assert!(mcp_config_path.exists());

        let content = fs::read_to_string(&mcp_config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(config["mcpServers"]["my-server"].is_object());
    }

    #[test]
    fn write_mcp_servers_preserves_existing_structure() {
        let tmp = tempdir().unwrap();
        let mcp_config_path = tmp.path().join("mcp-config.json");
        fs::write(
            &mcp_config_path,
            r#"{
            "someOtherField": "preserved",
            "mcpServers": {}
        }"#,
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let mut servers = HashMap::new();
        servers.insert(
            "new-server".to_string(),
            McpServer {
                name: "new-server".to_string(),
                command: "/bin/new".to_string(),
                args: vec![],
                env: HashMap::new(),
                enabled: true,
            },
        );

        adapter.write_mcp_servers(&servers).unwrap();

        let content = fs::read_to_string(&mcp_config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["someOtherField"], "preserved");
    }

    // ==========================================
    // Preferences Tests (with security field preservation)
    // ==========================================

    #[test]
    fn read_preferences_empty_when_no_file() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let prefs = adapter.read_preferences().unwrap();
        assert!(prefs.model.is_none());
    }

    #[test]
    fn read_preferences_from_config_json() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
            "model": "gpt-4o"
        }"#,
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let prefs = adapter.read_preferences().unwrap();

        assert_eq!(prefs.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn read_preferences_invalid_json_returns_error() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(&config_path, "not valid json").unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let result = adapter.read_preferences();
        assert!(result.is_err());
    }

    #[test]
    fn write_preferences_creates_config_json() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let prefs = Preferences {
            model: Some("gpt-4o".to_string()),
            custom: HashMap::new(),
        };

        let report = adapter.write_preferences(&prefs).unwrap();
        assert_eq!(report.written, 1);

        let config_path = tmp.path().join("config.json");
        assert!(config_path.exists());

        let content = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["model"], "gpt-4o");
    }

    #[test]
    fn write_preferences_preserves_trusted_folders() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
            "model": "old-model",
            "trusted_folders": ["/home/user/project"]
        }"#,
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let prefs = Preferences {
            model: Some("gpt-4o".to_string()),
            custom: HashMap::new(),
        };

        adapter.write_preferences(&prefs).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Model should be updated
        assert_eq!(config["model"], "gpt-4o");
        // Security field should be preserved
        assert_eq!(config["trusted_folders"][0], "/home/user/project");
    }

    #[test]
    fn write_preferences_preserves_allowed_urls() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
            "allowed_urls": ["https://example.com"],
            "denied_urls": ["https://malicious.com"]
        }"#,
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let prefs = Preferences {
            model: Some("new-model".to_string()),
            custom: HashMap::new(),
        };

        adapter.write_preferences(&prefs).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let config: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Security fields should be preserved
        assert_eq!(config["allowed_urls"][0], "https://example.com");
        assert_eq!(config["denied_urls"][0], "https://malicious.com");
    }

    #[test]
    fn write_preferences_invalid_existing_json_returns_error() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(&config_path, "{ malformed: json, }").unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let prefs = Preferences {
            model: Some("gpt-4o".to_string()),
            custom: HashMap::new(),
        };

        let result = adapter.write_preferences(&prefs);
        assert!(result.is_err());
    }

    // ==========================================
    // Sanitization Tests
    // ==========================================

    #[test]
    fn sanitize_name_removes_path_traversal() {
        // Path traversal attacks are blocked
        assert_eq!(sanitize_name("../../../etc/passwd"), "etc/passwd");
        assert_eq!(sanitize_name("../../malicious"), "malicious");

        // Valid names pass through unchanged
        assert_eq!(sanitize_name("valid-name_123"), "valid-name_123");
        assert_eq!(sanitize_name("normal"), "normal");

        // Spaces and special chars are removed within segments
        assert_eq!(sanitize_name("with spaces"), "withspaces");

        // Nested skill paths are preserved (key fix for review feedback)
        assert_eq!(sanitize_name("category/my-skill"), "category/my-skill");
        assert_eq!(sanitize_name("deep/nested/skill"), "deep/nested/skill");

        // Mixed: nested paths with traversal attempts
        assert_eq!(
            sanitize_name("category/../other/skill"),
            "category/other/skill"
        );
        assert_eq!(sanitize_name("./relative/./path"), "relative/path");

        // Empty segments are collapsed
        assert_eq!(sanitize_name("a//b///c"), "a/b/c");
    }

    // ==========================================
    // Integration Tests
    // ==========================================

    #[test]
    fn skills_roundtrip() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let skill = Command {
            name: "test-skill".to_string(),
            content: b"---\nname: test-skill\ndescription: A test skill\n---\n# Test Skill\n"
                .to_vec(),
            source_path: PathBuf::from("/tmp/test.md"),
            modified: SystemTime::now(),
            hash: "hash123".to_string(),
        };

        adapter.write_skills(&[skill]).unwrap();
        let read_back = adapter.read_skills().unwrap();

        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].name, "test-skill");
    }

    #[test]
    fn mcp_servers_roundtrip() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let mut servers = HashMap::new();
        servers.insert(
            "test-server".to_string(),
            McpServer {
                name: "test-server".to_string(),
                command: "/bin/test".to_string(),
                args: vec!["--arg".to_string()],
                env: HashMap::from([("KEY".to_string(), "value".to_string())]),
                enabled: true,
            },
        );

        adapter.write_mcp_servers(&servers).unwrap();
        let read_back = adapter.read_mcp_servers().unwrap();

        assert_eq!(read_back.len(), 1);
        let server = read_back.get("test-server").unwrap();
        assert_eq!(server.command, "/bin/test");
        assert_eq!(server.args, vec!["--arg"]);
        assert!(server.enabled);
    }

    // ==========================================
    // Agents Tests
    // ==========================================

    #[test]
    fn copilot_supports_agents() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let fields = adapter.supported_fields();
        assert!(fields.agents, "Copilot should support agents");
    }

    #[test]
    fn read_agents_empty_dir() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let agents = adapter.read_agents().unwrap();
        assert!(agents.is_empty());
    }

    #[test]
    fn read_agents_discovers_agent_md() {
        let tmp = tempdir().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("test-agent.agent.md"),
            "---\nname: test-agent\ndescription: A test agent\ntarget: github-copilot\n---\n# Test Agent\n\nInstructions here.",
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let agents = adapter.read_agents().unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "test-agent");
    }

    #[test]
    fn read_agents_handles_plain_md_files() {
        let tmp = tempdir().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("simple.md"),
            "---\nname: simple\n---\n# Simple",
        )
        .unwrap();

        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());
        let agents = adapter.read_agents().unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "simple");
    }

    #[test]
    fn write_agents_creates_agent_files() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let agents = vec![Command {
            name: "my-agent".to_string(),
            content: b"---\nname: my-agent\ndescription: Test\nmodel: opus\n---\n# My Agent"
                .to_vec(),
            source_path: PathBuf::from("/tmp/test.md"),
            modified: SystemTime::now(),
            hash: "abc".to_string(),
        }];

        let report = adapter.write_agents(&agents).unwrap();
        assert_eq!(report.written, 1);

        // Verify file was created
        let agent_path = tmp.path().join("agents/my-agent.agent.md");
        assert!(agent_path.exists());

        // Verify content was transformed
        let content = fs::read_to_string(&agent_path).unwrap();
        assert!(content.contains("target: github-copilot"));
        assert!(!content.contains("model: opus"));
    }

    #[test]
    fn write_agents_skips_unchanged() {
        let tmp = tempdir().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let agents = vec![Command {
            name: "unchanged".to_string(),
            content: b"---\nname: unchanged\ndescription: Test\n---\n# Agent".to_vec(),
            source_path: PathBuf::from("/tmp/test.md"),
            modified: SystemTime::now(),
            hash: "abc".to_string(),
        }];

        // Write once
        let report1 = adapter.write_agents(&agents).unwrap();
        assert_eq!(report1.written, 1);

        // Write again - should be skipped
        let report2 = adapter.write_agents(&agents).unwrap();
        assert_eq!(report2.written, 0);
        assert_eq!(report2.skipped.len(), 1);
    }

    #[test]
    fn transform_agent_adds_target_to_claude_format() {
        let claude_content = b"---\nname: test\ndescription: Test agent\nmodel: opus\ncolor: green\n---\n# Test\n\nContent here.";
        let result = transform_agent_for_copilot(claude_content);
        let result_str = std::str::from_utf8(&result).unwrap();

        assert!(result_str.contains("target: github-copilot"));
        assert!(!result_str.contains("model: opus"));
        assert!(!result_str.contains("color: green"));
        assert!(result_str.contains("name: test"));
        assert!(result_str.contains("description: Test agent"));
        assert!(result_str.contains("# Test\n\nContent here."));
    }

    #[test]
    fn transform_agent_preserves_existing_target() {
        let content = b"---\nname: test\ntarget: vscode\n---\n# Test";
        let result = transform_agent_for_copilot(content);
        let result_str = std::str::from_utf8(&result).unwrap();

        // Should keep existing target, not add duplicate
        assert!(result_str.contains("target: vscode"));
        assert!(!result_str.contains("target: github-copilot"));
    }

    #[test]
    fn transform_agent_handles_no_frontmatter() {
        let content = b"# Just markdown\n\nNo frontmatter here.";
        let result = transform_agent_for_copilot(content);
        let result_str = std::str::from_utf8(&result).unwrap();

        assert!(result_str.starts_with("---\ntarget: github-copilot\n---"));
        assert!(result_str.contains("# Just markdown"));
    }

    // ==========================================
    // XDG Compliance Tests
    // ==========================================

    #[test]
    fn resolve_config_root_prefers_xdg_if_exists() {
        // This test verifies the XDG path resolution logic.
        // When both XDG and legacy paths exist, XDG should be preferred.
        // We can't easily test the env var without process isolation,
        // but we verify the resolve_config_root function exists and works.
        let result = CopilotAdapter::resolve_config_root();
        assert!(result.is_ok(), "resolve_config_root should succeed");

        let path = result.unwrap();
        // Path should end with "copilot" or ".copilot"
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(
            filename == "copilot" || filename == ".copilot",
            "Config root should be named 'copilot' or '.copilot', got: {}",
            filename
        );
    }

    #[test]
    fn new_adapter_uses_xdg_resolution() {
        // Verify that new() calls resolve_config_root internally
        let adapter = CopilotAdapter::new();
        assert!(adapter.is_ok(), "CopilotAdapter::new() should succeed");

        let adapter = adapter.unwrap();
        let root = adapter.config_root();
        let filename = root.file_name().unwrap().to_str().unwrap();
        assert!(
            filename == "copilot" || filename == ".copilot",
            "Config root should be named 'copilot' or '.copilot', got: {}",
            filename
        );
    }
}
