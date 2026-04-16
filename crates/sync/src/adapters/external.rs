//! Generic external adapter driven by TOML configuration.
//!
//! Allows new CLI adapters (e.g., Windsurf, Aider, Continue.dev) to be
//! registered without modifying sync crate source code. Each external
//! adapter is described by an [`ExternalAdapterConfig`] loaded from
//! `~/.skrills/adapters.toml`.
//!
//! ## Supported layout
//!
//! An external adapter maps a simple directory-based layout:
//! - **skills**: markdown files in `{config_root}/{skills_dir}/`
//! - **commands**: markdown files in `{config_root}/{commands_dir}/`
//! - **MCP servers**: JSON file at `{config_root}/{mcp_config}`
//!
//! For more complex adapters with custom formats (e.g., `.mdc` rules,
//! hook event mapping), a built-in adapter should be written instead.

use super::traits::{AgentAdapter, FieldSupport};
use super::utils::{hash_content, is_hidden_path, sanitize_name};
use crate::common::{Command, McpServer, McpTransport, Preferences};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::debug;
use walkdir::WalkDir;

/// Skill content format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillFormat {
    /// Plain markdown files (one `.md` per skill).
    #[default]
    Markdown,
    /// Markdown with YAML frontmatter (Claude-style).
    Frontmatter,
    /// MDC format with frontmatter (`description`, `globs`, `alwaysApply`).
    Mdc,
}

/// Configuration for a single external adapter.
///
/// Loaded from a `[adapter.<name>]` section in `adapters.toml`.
///
/// ```toml
/// [adapter.windsurf]
/// name = "windsurf"
/// config_root = "~/.windsurf"
/// skills_dir = "skills"
/// commands_dir = "commands"
/// mcp_config = "mcp.json"
/// skill_format = "markdown"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAdapterConfig {
    /// Adapter identifier (e.g., "windsurf").
    pub name: String,
    /// Root configuration directory (e.g., "~/.windsurf").
    /// Supports `~` expansion.
    pub config_root: String,
    /// Skills directory relative to config_root (e.g., "skills").
    #[serde(default)]
    pub skills_dir: Option<String>,
    /// Commands directory relative to config_root (e.g., "commands").
    #[serde(default)]
    pub commands_dir: Option<String>,
    /// Rules/instructions directory relative to config_root.
    #[serde(default)]
    pub rules_dir: Option<String>,
    /// MCP server config file relative to config_root (e.g., "mcp.json").
    #[serde(default)]
    pub mcp_config: Option<String>,
    /// Skill content format.
    #[serde(default)]
    pub skill_format: SkillFormat,
}

/// Validation diagnostics for an external adapter configuration.
#[derive(Debug, Clone)]
pub struct AdapterDiagnostic {
    /// Adapter name.
    pub name: String,
    /// Issues found during validation.
    pub issues: Vec<String>,
    /// Informational messages.
    pub info: Vec<String>,
}

impl ExternalAdapterConfig {
    /// Resolves `config_root` with tilde expansion.
    pub fn resolved_root(&self) -> PathBuf {
        expand_tilde(&self.config_root)
    }

    /// Validates the configuration and returns diagnostics.
    pub fn validate(&self) -> AdapterDiagnostic {
        let mut diag = AdapterDiagnostic {
            name: self.name.clone(),
            issues: Vec::new(),
            info: Vec::new(),
        };

        if self.name.is_empty() {
            diag.issues.push("adapter name is empty".to_string());
        }

        let root = self.resolved_root();
        if !root.exists() {
            diag.issues
                .push(format!("config_root does not exist: {}", root.display()));
        } else {
            diag.info
                .push(format!("config_root exists: {}", root.display()));

            if let Some(ref skills_dir) = self.skills_dir {
                let skills_path = root.join(skills_dir);
                if skills_path.exists() {
                    diag.info
                        .push(format!("skills_dir exists: {}", skills_path.display()));
                } else {
                    diag.issues.push(format!(
                        "skills_dir does not exist: {}",
                        skills_path.display()
                    ));
                }
            }

            if let Some(ref commands_dir) = self.commands_dir {
                let commands_path = root.join(commands_dir);
                if commands_path.exists() {
                    diag.info.push(format!(
                        "commands_dir exists: {}",
                        commands_path.display()
                    ));
                } else {
                    diag.issues.push(format!(
                        "commands_dir does not exist: {}",
                        commands_path.display()
                    ));
                }
            }

            if let Some(ref rules_dir) = self.rules_dir {
                let rules_path = root.join(rules_dir);
                if rules_path.exists() {
                    diag.info
                        .push(format!("rules_dir exists: {}", rules_path.display()));
                } else {
                    diag.issues.push(format!(
                        "rules_dir does not exist: {}",
                        rules_path.display()
                    ));
                }
            }

            if let Some(ref mcp_config) = self.mcp_config {
                let mcp_path = root.join(mcp_config);
                if mcp_path.exists() {
                    diag.info
                        .push(format!("mcp_config exists: {}", mcp_path.display()));
                } else {
                    diag.issues.push(format!(
                        "mcp_config does not exist: {}",
                        mcp_path.display()
                    ));
                }
            }
        }

        diag
    }
}

/// External adapter that implements `AgentAdapter` using config-driven paths.
pub struct ExternalAdapter {
    config: ExternalAdapterConfig,
    root: PathBuf,
}

impl ExternalAdapter {
    /// Creates an external adapter from its configuration.
    pub fn new(config: ExternalAdapterConfig) -> Self {
        let root = config.resolved_root();
        Self { config, root }
    }

    /// Returns a reference to the underlying configuration.
    pub fn config(&self) -> &ExternalAdapterConfig {
        &self.config
    }

    fn skills_dir(&self) -> Option<PathBuf> {
        self.config.skills_dir.as_ref().map(|d| self.root.join(d))
    }

    fn commands_dir(&self) -> Option<PathBuf> {
        self.config
            .commands_dir
            .as_ref()
            .map(|d| self.root.join(d))
    }

    fn rules_dir(&self) -> Option<PathBuf> {
        self.config.rules_dir.as_ref().map(|d| self.root.join(d))
    }

    fn mcp_config_path(&self) -> Option<PathBuf> {
        self.config
            .mcp_config
            .as_ref()
            .map(|f| self.root.join(f))
    }
}

impl AgentAdapter for ExternalAdapter {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn config_root(&self) -> PathBuf {
        self.root.clone()
    }

    fn supported_fields(&self) -> FieldSupport {
        FieldSupport {
            commands: self.config.commands_dir.is_some(),
            mcp_servers: self.config.mcp_config.is_some(),
            preferences: false,
            skills: self.config.skills_dir.is_some(),
            hooks: false,
            agents: false,
            instructions: self.config.rules_dir.is_some(),
            plugin_assets: false,
        }
    }

    fn read_commands(&self, _include_marketplace: bool) -> Result<Vec<Command>> {
        let Some(dir) = self.commands_dir() else {
            return Ok(Vec::new());
        };
        read_markdown_dir(&dir)
    }

    fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>> {
        let Some(path) = self.mcp_config_path() else {
            return Ok(HashMap::new());
        };
        read_mcp_json(&path)
    }

    fn read_preferences(&self) -> Result<Preferences> {
        Ok(Preferences::default())
    }

    fn read_skills(&self) -> Result<Vec<Command>> {
        let Some(dir) = self.skills_dir() else {
            return Ok(Vec::new());
        };
        read_markdown_dir(&dir)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        Ok(Vec::new())
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        Ok(Vec::new())
    }

    fn read_instructions(&self) -> Result<Vec<Command>> {
        let Some(dir) = self.rules_dir() else {
            return Ok(Vec::new());
        };
        read_markdown_dir(&dir)
    }

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        let Some(dir) = self.commands_dir() else {
            return Ok(unsupported_report("commands"));
        };
        write_markdown_dir(&dir, commands)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        let Some(path) = self.mcp_config_path() else {
            return Ok(unsupported_report("mcp_servers"));
        };
        write_mcp_json(&path, servers)
    }

    fn write_preferences(&self, _prefs: &Preferences) -> Result<WriteReport> {
        Ok(unsupported_report("preferences"))
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        let Some(dir) = self.skills_dir() else {
            return Ok(unsupported_report("skills"));
        };
        write_markdown_dir(&dir, skills)
    }

    fn write_hooks(&self, _hooks: &[Command]) -> Result<WriteReport> {
        Ok(unsupported_report("hooks"))
    }

    fn write_agents(&self, _agents: &[Command]) -> Result<WriteReport> {
        Ok(unsupported_report("agents"))
    }

    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport> {
        let Some(dir) = self.rules_dir() else {
            return Ok(unsupported_report("instructions"));
        };
        write_markdown_dir(&dir, instructions)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Expands `~` at the start of a path to the user's home directory.
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

/// Reads all `.md` files from a directory as [`Command`] items.
fn read_markdown_dir(dir: &Path) -> Result<Vec<Command>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut commands = Vec::new();

    for entry in WalkDir::new(dir)
        .min_depth(1)
        .max_depth(3)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                debug!(error = %e, "Skipping directory entry");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Accept .md and .mdc files
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "md" && ext != "mdc" {
            continue;
        }

        if let Ok(rel) = path.strip_prefix(dir) {
            if is_hidden_path(rel) {
                continue;
            }
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let name = sanitize_name(name);

        let content = match fs::read(path) {
            Ok(c) => c,
            Err(e) => {
                debug!(path = %path.display(), error = %e, "Skipping unreadable file");
                continue;
            }
        };

        let modified = path
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let hash = hash_content(&content);

        commands.push(Command {
            name,
            content,
            source_path: path.to_path_buf(),
            modified,
            hash,
            modules: vec![],
            content_format: crate::common::ContentFormat::Markdown,
        });
    }

    Ok(commands)
}

/// Writes [`Command`] items as markdown files into a directory.
fn write_markdown_dir(dir: &Path, commands: &[Command]) -> Result<WriteReport> {
    let mut report = WriteReport::default();

    if commands.is_empty() {
        return Ok(report);
    }

    fs::create_dir_all(dir)?;

    for cmd in commands {
        let safe_name = sanitize_name(&cmd.name);
        if safe_name.is_empty() {
            report.skipped.push(SkipReason::ParseError {
                item: cmd.name.clone(),
                error: "name sanitized to empty string".to_string(),
            });
            continue;
        }

        let target_path = dir.join(format!("{safe_name}.md"));

        // Check if unchanged
        if target_path.exists() {
            if let Ok(existing) = fs::read(&target_path) {
                if hash_content(&existing) == cmd.hash {
                    report.skipped.push(SkipReason::Unchanged {
                        item: cmd.name.clone(),
                    });
                    continue;
                }
            }
        }

        fs::write(&target_path, &cmd.content)?;
        debug!(name = %cmd.name, path = %target_path.display(), "Wrote file");
        report.written += 1;
    }

    Ok(report)
}

/// Reads MCP server configurations from a JSON file.
///
/// Supports both `{ "mcpServers": { ... } }` (Claude-style) and flat
/// `{ "server-name": { ... } }` formats.
fn read_mcp_json(path: &Path) -> Result<HashMap<String, McpServer>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let raw = fs::read_to_string(path).context("reading MCP config")?;
    let json: serde_json::Value = serde_json::from_str(&raw).context("parsing MCP config JSON")?;

    let servers_obj = if let Some(mcp) = json.get("mcpServers") {
        mcp.as_object().cloned().unwrap_or_default()
    } else {
        json.as_object().cloned().unwrap_or_default()
    };

    let mut result = HashMap::new();
    for (name, val) in servers_obj {
        let command = val
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let args: Vec<String> = val
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let env: HashMap<String, String> = val
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let url = val.get("url").and_then(|v| v.as_str()).map(String::from);
        let transport = if url.is_some() {
            McpTransport::Http
        } else {
            McpTransport::Stdio
        };

        result.insert(
            name.clone(),
            McpServer {
                name,
                transport,
                command,
                args,
                env,
                url,
                headers: None,
                enabled: true,
                allowed_tools: vec![],
                disabled_tools: vec![],
            },
        );
    }

    Ok(result)
}

/// Writes MCP server configurations to a JSON file.
fn write_mcp_json(path: &Path, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
    let mut report = WriteReport::default();

    if servers.is_empty() {
        return Ok(report);
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Read existing config to merge
    let mut existing: serde_json::Value = if path.exists() {
        let raw = fs::read_to_string(path)?;
        serde_json::from_str(&raw).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Determine if existing uses mcpServers wrapper
    let uses_wrapper = existing.get("mcpServers").is_some();

    let servers_obj = if uses_wrapper {
        existing
            .get_mut("mcpServers")
            .and_then(|v| v.as_object_mut())
    } else {
        existing.as_object_mut()
    };

    if let Some(obj) = servers_obj {
        for (name, server) in servers {
            let mut entry = serde_json::Map::new();
            entry.insert(
                "command".to_string(),
                serde_json::Value::String(server.command.clone()),
            );
            if !server.args.is_empty() {
                entry.insert(
                    "args".to_string(),
                    serde_json::Value::Array(
                        server
                            .args
                            .iter()
                            .map(|a| serde_json::Value::String(a.clone()))
                            .collect(),
                    ),
                );
            }
            if !server.env.is_empty() {
                let env_obj: serde_json::Map<String, serde_json::Value> = server
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                entry.insert("env".to_string(), serde_json::Value::Object(env_obj));
            }
            if let Some(ref url) = server.url {
                entry.insert("url".to_string(), serde_json::Value::String(url.clone()));
            }
            obj.insert(name.clone(), serde_json::Value::Object(entry));
            report.written += 1;
        }
    } else {
        // Create fresh with mcpServers wrapper
        let mut mcp_obj = serde_json::Map::new();
        for (name, server) in servers {
            let mut entry = serde_json::Map::new();
            entry.insert(
                "command".to_string(),
                serde_json::Value::String(server.command.clone()),
            );
            if !server.args.is_empty() {
                entry.insert(
                    "args".to_string(),
                    serde_json::Value::Array(
                        server
                            .args
                            .iter()
                            .map(|a| serde_json::Value::String(a.clone()))
                            .collect(),
                    ),
                );
            }
            mcp_obj.insert(name.clone(), serde_json::Value::Object(entry));
            report.written += 1;
        }
        existing = serde_json::json!({ "mcpServers": mcp_obj });
    }

    let json_str = serde_json::to_string_pretty(&existing)?;
    fs::write(path, json_str)?;

    Ok(report)
}

/// Creates a report indicating an unsupported field.
fn unsupported_report(field: &str) -> WriteReport {
    let mut report = WriteReport::default();
    report
        .skipped
        .push(SkipReason::AgentSpecificFeature {
            item: field.to_string(),
            feature: "Not configured for this external adapter".to_string(),
            suggestion: format!(
                "Add the corresponding directory setting to adapters.toml for this field"
            ),
        });
    report
}

/// Top-level structure of `adapters.toml`.
///
/// ```toml
/// [adapter.windsurf]
/// name = "windsurf"
/// config_root = "~/.windsurf"
/// skills_dir = "skills"
///
/// [adapter.aider]
/// name = "aider"
/// config_root = "~/.aider"
/// commands_dir = "prompts"
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdaptersFile {
    /// Map of adapter name to configuration.
    #[serde(default)]
    pub adapter: HashMap<String, ExternalAdapterConfig>,
}

/// Returns the default config file path for external adapters.
///
/// Checks in order:
/// 1. `$SKRILLS_ADAPTERS_CONFIG` environment variable
/// 2. `~/.skrills/adapters.toml`
/// 3. `~/.config/skrills/adapters.toml` (XDG)
pub fn adapters_config_path() -> Option<PathBuf> {
    // 1. Env var override
    if let Ok(custom) = std::env::var("SKRILLS_ADAPTERS_CONFIG") {
        return Some(PathBuf::from(custom));
    }

    let home = dirs::home_dir()?;

    // 2. Primary location
    let primary = home.join(".skrills").join("adapters.toml");
    if primary.exists() {
        return Some(primary);
    }

    // 3. XDG location
    let xdg = home.join(".config").join("skrills").join("adapters.toml");
    if xdg.exists() {
        return Some(xdg);
    }

    // Return primary as the default (even if it doesn't exist)
    Some(primary)
}

/// Loads external adapter configurations from `adapters.toml`.
///
/// Returns an empty list if the file doesn't exist.
pub fn load_external_configs() -> Result<Vec<ExternalAdapterConfig>> {
    let Some(path) = adapters_config_path() else {
        return Ok(Vec::new());
    };

    if !path.exists() {
        return Ok(Vec::new());
    }

    let raw =
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: AdaptersFile =
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

    Ok(file.adapter.into_values().collect())
}

/// Loads external adapter configurations and instantiates adapters.
pub fn load_external_adapters() -> Result<Vec<ExternalAdapter>> {
    let configs = load_external_configs()?;
    Ok(configs.into_iter().map(ExternalAdapter::new).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_config(root: &Path) -> ExternalAdapterConfig {
        ExternalAdapterConfig {
            name: "test-adapter".to_string(),
            config_root: root.to_str().unwrap().to_string(),
            skills_dir: Some("skills".to_string()),
            commands_dir: Some("commands".to_string()),
            rules_dir: None,
            mcp_config: Some("mcp.json".to_string()),
            skill_format: SkillFormat::default(),
        }
    }

    #[test]
    fn external_adapter_name_and_root() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        assert_eq!(adapter.name(), "test-adapter");
        assert_eq!(adapter.config_root(), dir.path());
    }

    #[test]
    fn external_adapter_supported_fields_reflect_config() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);
        let support = adapter.supported_fields();

        assert!(support.skills);
        assert!(support.commands);
        assert!(support.mcp_servers);
        assert!(!support.preferences);
        assert!(!support.hooks);
        assert!(!support.agents);
        assert!(!support.instructions);
    }

    #[test]
    fn external_adapter_reads_empty_when_dirs_missing() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        assert!(adapter.read_commands(false).unwrap().is_empty());
        assert!(adapter.read_skills().unwrap().is_empty());
        assert!(adapter.read_mcp_servers().unwrap().is_empty());
    }

    #[test]
    fn external_adapter_reads_skills_from_dir() {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(skills_dir.join("hello.md"), "# Hello Skill").unwrap();
        fs::write(skills_dir.join("world.md"), "# World Skill").unwrap();

        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        let skills = adapter.read_skills().unwrap();
        assert_eq!(skills.len(), 2);

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"world"));
    }

    #[test]
    fn external_adapter_writes_skills_to_dir() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        let skills = vec![Command::new(
            "my-skill".to_string(),
            b"# My Skill\nContent here".to_vec(),
            PathBuf::from("/source/my-skill.md"),
        )];

        let report = adapter.write_skills(&skills).unwrap();
        assert_eq!(report.written, 1);

        let written_path = dir.path().join("skills/my-skill.md");
        assert!(written_path.exists());
        assert_eq!(
            fs::read_to_string(&written_path).unwrap(),
            "# My Skill\nContent here"
        );
    }

    #[test]
    fn external_adapter_reads_commands_from_dir() {
        let dir = tempdir().unwrap();
        let commands_dir = dir.path().join("commands");
        fs::create_dir_all(&commands_dir).unwrap();
        fs::write(commands_dir.join("commit.md"), "# Commit message helper").unwrap();

        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        let commands = adapter.read_commands(false).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "commit");
    }

    #[test]
    fn external_adapter_writes_commands_to_dir() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        let commands = vec![Command::new(
            "deploy".to_string(),
            b"# Deploy command".to_vec(),
            PathBuf::from("/source/deploy.md"),
        )];

        let report = adapter.write_commands(&commands).unwrap();
        assert_eq!(report.written, 1);

        let written_path = dir.path().join("commands/deploy.md");
        assert!(written_path.exists());
    }

    #[test]
    fn external_adapter_reads_mcp_servers() {
        let dir = tempdir().unwrap();
        let mcp_path = dir.path().join("mcp.json");
        fs::write(
            &mcp_path,
            r#"{
                "mcpServers": {
                    "test-server": {
                        "command": "/usr/bin/test",
                        "args": ["--verbose"]
                    }
                }
            }"#,
        )
        .unwrap();

        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        let servers = adapter.read_mcp_servers().unwrap();
        assert_eq!(servers.len(), 1);
        assert!(servers.contains_key("test-server"));
        assert_eq!(servers["test-server"].command, "/usr/bin/test");
        assert_eq!(servers["test-server"].args, vec!["--verbose"]);
    }

    #[test]
    fn external_adapter_reads_flat_mcp_json() {
        let dir = tempdir().unwrap();
        let mcp_path = dir.path().join("mcp.json");
        fs::write(
            &mcp_path,
            r#"{
                "my-server": {
                    "command": "npx",
                    "args": ["-y", "my-mcp-server"]
                }
            }"#,
        )
        .unwrap();

        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        let servers = adapter.read_mcp_servers().unwrap();
        assert_eq!(servers.len(), 1);
        assert!(servers.contains_key("my-server"));
    }

    #[test]
    fn external_adapter_writes_mcp_servers() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        let mut servers = HashMap::new();
        servers.insert(
            "new-server".to_string(),
            McpServer {
                name: "new-server".to_string(),
                transport: McpTransport::Stdio,
                command: "/usr/bin/new".to_string(),
                args: vec!["--flag".to_string()],
                env: HashMap::new(),
                url: None,
                headers: None,
                enabled: true,
                allowed_tools: vec![],
                disabled_tools: vec![],
            },
        );

        let report = adapter.write_mcp_servers(&servers).unwrap();
        assert_eq!(report.written, 1);

        let mcp_path = dir.path().join("mcp.json");
        assert!(mcp_path.exists());

        let content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(content.get("mcpServers").is_some());
    }

    #[test]
    fn external_adapter_write_skips_unchanged() {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(skills_dir.join("existing.md"), "# Existing").unwrap();

        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        let skills = vec![Command::new(
            "existing".to_string(),
            b"# Existing".to_vec(),
            PathBuf::from("/source/existing.md"),
        )];

        let report = adapter.write_skills(&skills).unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn external_adapter_unsupported_operations_report_skipped() {
        let dir = tempdir().unwrap();
        let config = make_config(dir.path());
        let adapter = ExternalAdapter::new(config);

        // Hooks are not configured
        let report = adapter.write_hooks(&[]).unwrap();
        assert_eq!(report.skipped.len(), 1);

        // Agents are not configured
        let report = adapter.write_agents(&[]).unwrap();
        assert_eq!(report.skipped.len(), 1);

        // Preferences are never supported
        let report = adapter
            .write_preferences(&Preferences::default())
            .unwrap();
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn config_validation_reports_missing_root() {
        let config = ExternalAdapterConfig {
            name: "nonexistent".to_string(),
            config_root: "/tmp/definitely-does-not-exist-skrills-test".to_string(),
            skills_dir: Some("skills".to_string()),
            commands_dir: None,
            rules_dir: None,
            mcp_config: None,
            skill_format: SkillFormat::default(),
        };

        let diag = config.validate();
        assert!(!diag.issues.is_empty());
        assert!(diag.issues[0].contains("config_root does not exist"));
    }

    #[test]
    fn config_validation_reports_existing_root_and_dirs() {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        let config = ExternalAdapterConfig {
            name: "valid".to_string(),
            config_root: dir.path().to_str().unwrap().to_string(),
            skills_dir: Some("skills".to_string()),
            commands_dir: Some("commands".to_string()), // does not exist
            rules_dir: None,
            mcp_config: None,
            skill_format: SkillFormat::default(),
        };

        let diag = config.validate();
        // skills_dir exists
        assert!(diag.info.iter().any(|i| i.contains("skills_dir exists")));
        // commands_dir does not exist
        assert!(diag
            .issues
            .iter()
            .any(|i| i.contains("commands_dir does not exist")));
    }

    #[test]
    fn expand_tilde_expands_home() {
        let expanded = expand_tilde("~/test/path");
        // Should not start with ~ anymore
        assert!(!expanded.to_str().unwrap().starts_with('~'));
        assert!(expanded.to_str().unwrap().ends_with("test/path"));
    }

    #[test]
    fn expand_tilde_passes_through_absolute_paths() {
        let expanded = expand_tilde("/absolute/path");
        assert_eq!(expanded, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn adapters_file_parses_toml() {
        let toml_str = r#"
[adapter.windsurf]
name = "windsurf"
config_root = "~/.windsurf"
skills_dir = "skills"
commands_dir = "commands"
mcp_config = "mcp.json"
skill_format = "markdown"

[adapter.aider]
name = "aider"
config_root = "~/.aider"
commands_dir = "prompts"
"#;
        let file: AdaptersFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.adapter.len(), 2);
        assert!(file.adapter.contains_key("windsurf"));
        assert!(file.adapter.contains_key("aider"));

        let windsurf = &file.adapter["windsurf"];
        assert_eq!(windsurf.name, "windsurf");
        assert_eq!(windsurf.config_root, "~/.windsurf");
        assert_eq!(windsurf.skills_dir.as_deref(), Some("skills"));
        assert_eq!(windsurf.commands_dir.as_deref(), Some("commands"));
        assert_eq!(windsurf.mcp_config.as_deref(), Some("mcp.json"));
        assert_eq!(windsurf.skill_format, SkillFormat::Markdown);

        let aider = &file.adapter["aider"];
        assert_eq!(aider.name, "aider");
        assert_eq!(aider.commands_dir.as_deref(), Some("prompts"));
        assert!(aider.skills_dir.is_none());
    }

    #[test]
    fn load_external_configs_returns_empty_when_no_file() {
        // Set env to a path that doesn't exist
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        std::env::set_var("SKRILLS_ADAPTERS_CONFIG", path.to_str().unwrap());
        let configs = load_external_configs().unwrap();
        std::env::remove_var("SKRILLS_ADAPTERS_CONFIG");
        assert!(configs.is_empty());
    }

    #[test]
    fn load_external_configs_from_file() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("adapters.toml");
        fs::write(
            &config_path,
            r#"
[adapter.test-cli]
name = "test-cli"
config_root = "/tmp/test-cli"
skills_dir = "skills"
"#,
        )
        .unwrap();

        std::env::set_var("SKRILLS_ADAPTERS_CONFIG", config_path.to_str().unwrap());
        let configs = load_external_configs().unwrap();
        std::env::remove_var("SKRILLS_ADAPTERS_CONFIG");

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "test-cli");
    }

    #[test]
    fn hidden_files_skipped_in_read() {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(skills_dir.join("visible.md"), "# Visible").unwrap();
        fs::write(skills_dir.join(".hidden.md"), "# Hidden").unwrap();

        let commands = read_markdown_dir(&skills_dir).unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "visible");
    }
}
