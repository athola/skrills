//! Claude Code adapter for reading/writing ~/.claude configuration.

use super::traits::{AgentAdapter, FieldSupport};
use super::utils::{hash_content, is_hidden_path};
use crate::common::{Command, McpServer, McpTransport, Preferences};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use anyhow::Context;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use walkdir::WalkDir;

/// Adapter for Claude Code configuration.
pub struct ClaudeAdapter {
    root: PathBuf,
}

impl ClaudeAdapter {
    /// Creates a new ClaudeAdapter with the default root (~/.claude).
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(Self {
            root: home.join(".claude"),
        })
    }

    /// Creates a ClaudeAdapter with a custom root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    fn commands_dir(&self) -> PathBuf {
        self.root.join("commands")
    }

    fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    fn hooks_dir(&self) -> PathBuf {
        self.root.join("hooks")
    }

    fn agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    fn collect_commands_from_dir(
        &self,
        dir: &PathBuf,
        seen: &mut HashSet<String>,
        commands: &mut Vec<Command>,
    ) -> Result<()> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in WalkDir::new(dir).min_depth(1).max_depth(8) {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }
            match path.extension() {
                Some(ext) if ext == "md" => {}
                _ => continue,
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            if !seen.insert(name.clone()) {
                continue;
            }

            let content = fs::read(path)?;
            let metadata = fs::metadata(path)?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            commands.push(Command {
                name,
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
            });
        }

        Ok(())
    }
}

// Note: We intentionally do not implement Default for ClaudeAdapter because
// construction requires home directory resolution which can fail. Use
// ClaudeAdapter::new() or ClaudeAdapter::with_root() instead.

impl AgentAdapter for ClaudeAdapter {
    fn name(&self) -> &str {
        "claude"
    }

    fn config_root(&self) -> PathBuf {
        self.root.clone()
    }

    fn supported_fields(&self) -> FieldSupport {
        FieldSupport {
            commands: true,
            mcp_servers: true,
            preferences: true,
            skills: true,
            hooks: true,
            agents: true,
        }
    }

    fn read_commands(&self, include_marketplace: bool) -> Result<Vec<Command>> {
        let mut commands = Vec::new();
        let mut seen = HashSet::new();

        // 1) Core ~/.claude/commands
        self.collect_commands_from_dir(&self.commands_dir(), &mut seen, &mut commands)?;

        // 2) Marketplaces & Cache
        let mut bases = vec!["plugins/cache"];
        if include_marketplace {
            bases.push("plugins/marketplaces");
        }

        for base in bases {
            let base_path = self.root.join(base);
            if !base_path.exists() {
                continue;
            }
            for entry in WalkDir::new(&base_path).min_depth(1).max_depth(8) {
                let entry = entry?;
                let path = entry.path();

                if !path.is_file() {
                    continue;
                }
                match path.extension() {
                    Some(ext) if ext == "md" => {}
                    _ => continue,
                }

                // Only include files that live under a commands directory
                if !path
                    .ancestors()
                    .any(|p| p.file_name().is_some_and(|n| n == "commands"))
                {
                    continue;
                }

                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                if !seen.insert(name.clone()) {
                    continue;
                }

                let content = fs::read(path)?;
                let metadata = fs::metadata(path)?;
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let hash = hash_content(&content);

                commands.push(Command {
                    name,
                    content,
                    source_path: path.to_path_buf(),
                    modified,
                    hash,
                });
            }
        }

        Ok(commands)
    }

    fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>> {
        let path = self.settings_path();
        if !path.exists() {
            return Ok(HashMap::new());
        }

        let content = fs::read_to_string(&path)?;
        let settings: serde_json::Value = serde_json::from_str(&content)?;

        let mut servers = HashMap::new();
        if let Some(mcp) = settings.get("mcpServers").and_then(|v| v.as_object()) {
            for (name, config) in mcp {
                // Determine transport type from "type" field (default to stdio)
                let transport = match config.get("type").and_then(|v| v.as_str()) {
                    Some("http") => McpTransport::Http,
                    Some("stdio") | None => McpTransport::Stdio,
                    Some(other) => {
                        tracing::warn!(unknown_type = other, name = %name, "Unknown MCP server type, defaulting to stdio");
                        McpTransport::Stdio
                    }
                };

                let server = McpServer {
                    name: name.clone(),
                    transport,
                    command: config
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    args: config
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    env: config
                        .get("env")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect()
                        })
                        .unwrap_or_default(),
                    url: config.get("url").and_then(|v| v.as_str()).map(String::from),
                    headers: config
                        .get("headers")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                                .collect()
                        }),
                    enabled: config
                        .get("disabled")
                        .and_then(|v| v.as_bool())
                        .map(|d| !d)
                        .unwrap_or(true),
                };

                // Warn if HTTP transport is missing URL (required for HTTP servers)
                if server.transport == McpTransport::Http && server.url.is_none() {
                    tracing::warn!(
                        name = %name,
                        "HTTP MCP server is missing required 'url' field"
                    );
                }

                servers.insert(name.clone(), server);
            }
        }

        Ok(servers)
    }

    fn read_preferences(&self) -> Result<Preferences> {
        let path = self.settings_path();
        if !path.exists() {
            return Ok(Preferences::default());
        }

        let content = fs::read_to_string(&path)?;
        let settings: serde_json::Value = serde_json::from_str(&content)?;

        Ok(Preferences {
            model: settings
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from),
            custom: HashMap::new(), // Could extract other fields here
        })
    }

    fn read_skills(&self) -> Result<Vec<Command>> {
        // Track skills by name, keeping the most recently modified version
        let mut skills_map: HashMap<String, Command> = HashMap::new();

        // Helper to process a skill and update the map if it's newer
        let mut process_skill = |name: String, path: &std::path::Path| -> Result<()> {
            let content = fs::read(path)?;
            let metadata = fs::metadata(path)?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            let skill = Command {
                name: name.clone(),
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
            };

            // Keep the most recently modified version
            match skills_map.get(&name) {
                Some(existing) if existing.modified >= modified => {
                    // Existing is newer or same, skip
                }
                _ => {
                    skills_map.insert(name, skill);
                }
            }
            Ok(())
        };

        // 1) Core ~/.claude/skills
        let skills_dir = self.skills_dir();
        if skills_dir.exists() {
            for entry in WalkDir::new(&skills_dir)
                .min_depth(1)
                .max_depth(20)
                .follow_links(false)
            {
                let entry = entry?;
                if entry.file_type().is_symlink() {
                    continue;
                }
                let path = entry.path();
                if is_hidden_path(path.strip_prefix(&skills_dir).unwrap_or(path)) {
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "md") {
                    continue;
                }

                let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
                let name = if is_skill_md {
                    path.parent()
                        .and_then(|p| p.strip_prefix(&skills_dir).ok())
                        .and_then(|p| p.to_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("unknown")
                        .to_string()
                } else {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string()
                };

                process_skill(name, path)?;
            }
        }

        // 2) Plugins cache ~/.claude/plugins/cache/**/*
        let cache_dir = self.root.join("plugins/cache");
        if cache_dir.exists() {
            for entry in WalkDir::new(&cache_dir).min_depth(1).max_depth(10) {
                let entry = entry?;
                let path = entry.path();

                if !path.is_file() {
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "md") {
                    continue;
                }

                // Only include files under a skills directory
                if !path
                    .ancestors()
                    .any(|p| p.file_name().is_some_and(|n| n == "skills"))
                {
                    continue;
                }

                // Extract skill name from path
                let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
                let name = if is_skill_md {
                    // Use parent directory name as skill name
                    path.parent()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .filter(|s| !s.is_empty() && *s != "skills")
                        .unwrap_or("unknown")
                        .to_string()
                } else {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string()
                };

                if name == "unknown" || name == "skills" {
                    continue;
                }

                process_skill(name, path)?;
            }
        }

        Ok(skills_map.into_values().collect())
    }

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        let dir = self.commands_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        for cmd in commands {
            let safe_name = sanitize_name(&cmd.name);
            let path = dir.join(format!("{}.md", safe_name));

            // Check if unchanged
            if path.exists() {
                let existing = fs::read(&path)?;
                if hash_content(&existing) == cmd.hash {
                    report.skipped.push(SkipReason::Unchanged {
                        item: cmd.name.clone(),
                    });
                    continue;
                }
            }

            fs::write(&path, &cmd.content)?;
            report.written += 1;
        }

        Ok(report)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        let path = self.settings_path();

        // Read existing settings or create new
        let mut settings: serde_json::Value = if path.exists() {
            let content = fs::read_to_string(&path)?;
            serde_json::from_str(&content)?
        } else {
            serde_json::json!({})
        };

        let mut report = WriteReport::default();
        let mut mcp_obj = serde_json::Map::new();

        for (name, server) in servers {
            let mut server_config = serde_json::Map::new();

            // Write transport type (only for non-stdio to keep config clean)
            if server.transport != McpTransport::Stdio {
                server_config.insert(
                    "type".into(),
                    serde_json::json!(match server.transport {
                        McpTransport::Stdio => "stdio",
                        McpTransport::Http => "http",
                    }),
                );
            }

            match server.transport {
                McpTransport::Http => {
                    // HTTP transport: write url and headers
                    if let Some(ref url) = server.url {
                        server_config.insert("url".into(), serde_json::json!(url));
                    }
                    if let Some(ref headers) = server.headers {
                        let headers_obj: serde_json::Map<String, serde_json::Value> = headers
                            .iter()
                            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                            .collect();
                        server_config
                            .insert("headers".into(), serde_json::Value::Object(headers_obj));
                    }
                }
                McpTransport::Stdio => {
                    // stdio transport: write command, args, env
                    server_config.insert("command".into(), serde_json::json!(server.command));
                    if !server.args.is_empty() {
                        server_config.insert("args".into(), serde_json::json!(server.args));
                    }
                    if !server.env.is_empty() {
                        server_config.insert("env".into(), serde_json::json!(server.env));
                    }
                }
            }

            if !server.enabled {
                server_config.insert("disabled".into(), serde_json::json!(true));
            }
            mcp_obj.insert(name.clone(), serde_json::Value::Object(server_config));
            report.written += 1;
        }

        settings["mcpServers"] = serde_json::Value::Object(mcp_obj);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serde_json::to_string_pretty(&settings)?)?;

        Ok(report)
    }

    fn write_preferences(&self, prefs: &Preferences) -> Result<WriteReport> {
        let path = self.settings_path();

        let mut settings: serde_json::Value = if path.exists() {
            let content = fs::read_to_string(&path)?;
            serde_json::from_str(&content)?
        } else {
            serde_json::json!({})
        };

        let mut report = WriteReport::default();

        if let Some(model) = &prefs.model {
            settings["model"] = serde_json::json!(model);
            report.written += 1;
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serde_json::to_string_pretty(&settings)?)?;

        Ok(report)
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        let dir = self.skills_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        for skill in skills {
            // Claude is permissive, but writing Codex-style SKILL.md keeps skills portable.
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
                fs::create_dir_all(parent)?;
            }

            // Check if unchanged
            if path.exists() {
                let existing = fs::read(&path)?;
                if hash_content(&existing) == skill.hash {
                    report.skipped.push(SkipReason::Unchanged {
                        item: skill.name.clone(),
                    });
                    continue;
                }
            }

            fs::write(&path, &skill.content)?;
            report.written += 1;
        }

        Ok(report)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        // Track hooks by name, keeping the most recently modified version
        let mut hooks_map: HashMap<String, Command> = HashMap::new();

        // Helper to process a hook and update the map if it's newer
        let mut process_hook = |name: String, path: &std::path::Path| -> Result<()> {
            let content = fs::read(path)?;
            let metadata = fs::metadata(path)?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            let hook = Command {
                name: name.clone(),
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
            };

            match hooks_map.get(&name) {
                Some(existing) if existing.modified >= modified => {}
                _ => {
                    hooks_map.insert(name, hook);
                }
            }
            Ok(())
        };

        // 1) Core ~/.claude/hooks
        let hooks_dir = self.hooks_dir();
        if hooks_dir.exists() {
            for entry in WalkDir::new(&hooks_dir)
                .min_depth(1)
                .max_depth(10)
                .follow_links(false)
            {
                let entry = entry?;
                if entry.file_type().is_symlink() {
                    continue;
                }
                let path = entry.path();
                if is_hidden_path(path.strip_prefix(&hooks_dir).unwrap_or(path)) {
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "md") {
                    continue;
                }

                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                process_hook(name, path)?;
            }
        }

        // 2) Plugins cache ~/.claude/plugins/cache/**/hooks/
        let cache_dir = self.root.join("plugins/cache");
        if cache_dir.exists() {
            for entry in WalkDir::new(&cache_dir).min_depth(1).max_depth(10) {
                let entry = entry?;
                let path = entry.path();

                if !path.is_file() {
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "md") {
                    continue;
                }

                // Only include files under a hooks directory
                if !path
                    .ancestors()
                    .any(|p| p.file_name().is_some_and(|n| n == "hooks"))
                {
                    continue;
                }

                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                if name == "unknown" || name == "hooks" {
                    continue;
                }

                process_hook(name, path)?;
            }
        }

        Ok(hooks_map.into_values().collect())
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        // Track agents by name, keeping the most recently modified version
        let mut agents_map: HashMap<String, Command> = HashMap::new();

        // Helper to process an agent and update the map if it's newer
        let mut process_agent = |name: String, path: &std::path::Path| -> Result<()> {
            let content = fs::read(path)?;
            let metadata = fs::metadata(path)?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            let agent = Command {
                name: name.clone(),
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
            };

            match agents_map.get(&name) {
                Some(existing) if existing.modified >= modified => {}
                _ => {
                    agents_map.insert(name, agent);
                }
            }
            Ok(())
        };

        // 1) Core ~/.claude/agents
        let agents_dir = self.agents_dir();
        if agents_dir.exists() {
            for entry in WalkDir::new(&agents_dir)
                .min_depth(1)
                .max_depth(10)
                .follow_links(false)
            {
                let entry = entry?;
                if entry.file_type().is_symlink() {
                    continue;
                }
                let path = entry.path();
                if is_hidden_path(path.strip_prefix(&agents_dir).unwrap_or(path)) {
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "md") {
                    continue;
                }

                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                process_agent(name, path)?;
            }
        }

        // 2) Plugins cache ~/.claude/plugins/cache/**/agents/
        let cache_dir = self.root.join("plugins/cache");
        if cache_dir.exists() {
            for entry in WalkDir::new(&cache_dir).min_depth(1).max_depth(10) {
                let entry = entry?;
                let path = entry.path();

                if !path.is_file() {
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "md") {
                    continue;
                }

                // Only include files under an agents directory
                if !path
                    .ancestors()
                    .any(|p| p.file_name().is_some_and(|n| n == "agents"))
                {
                    continue;
                }

                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                if name == "unknown" || name == "agents" {
                    continue;
                }

                process_agent(name, path)?;
            }
        }

        Ok(agents_map.into_values().collect())
    }

    fn write_hooks(&self, hooks: &[Command]) -> Result<WriteReport> {
        let dir = self.hooks_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        for hook in hooks {
            let safe_name = sanitize_name(&hook.name);
            let path = dir.join(format!("{}.md", safe_name));

            if path.exists() {
                let existing = fs::read(&path)?;
                if hash_content(&existing) == hook.hash {
                    report.skipped.push(SkipReason::Unchanged {
                        item: hook.name.clone(),
                    });
                    continue;
                }
            }

            fs::write(&path, &hook.content)?;
            report.written += 1;
        }

        Ok(report)
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        let dir = self.agents_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        for agent in agents {
            let safe_name = sanitize_name(&agent.name);
            let path = dir.join(format!("{}.md", safe_name));

            if path.exists() {
                let existing = fs::read(&path)?;
                if hash_content(&existing) == agent.hash {
                    report.skipped.push(SkipReason::Unchanged {
                        item: agent.name.clone(),
                    });
                    continue;
                }
            }

            fs::write(&path, &agent.content)?;
            report.written += 1;
        }

        Ok(report)
    }
}

/// Sanitizes a command/skill name to prevent path traversal attacks.
/// Only allows alphanumeric characters, hyphens, and underscores.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::tempdir;

    #[test]
    fn sanitize_name_removes_path_traversal() {
        assert_eq!(sanitize_name("../../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_name("valid-name_123"), "valid-name_123");
        assert_eq!(sanitize_name("../../malicious"), "malicious");
        assert_eq!(sanitize_name("normal"), "normal");
        assert_eq!(sanitize_name("with spaces"), "withspaces");
        assert_eq!(sanitize_name("has/slashes"), "hasslashes");
    }

    #[test]
    fn read_commands_empty_dir() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands(false).unwrap();
        assert!(commands.is_empty());
    }

    #[test]
    fn read_commands_finds_md_files() {
        let tmp = tempdir().unwrap();
        let cmd_dir = tmp.path().join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(cmd_dir.join("test.md"), "# Test Command").unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands(false).unwrap();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "test");
        assert_eq!(commands[0].content, b"# Test Command".to_vec());
    }

    #[test]
    fn read_commands_includes_marketplace_when_enabled() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        // Core command
        let core_dir = root.join("commands");
        fs::create_dir_all(&core_dir).unwrap();
        fs::write(core_dir.join("core.md"), "# Core").unwrap();

        // Marketplace command
        let mp_dir = root.join("plugins/marketplaces/mp/plugins/tool/commands");
        fs::create_dir_all(&mp_dir).unwrap();
        fs::write(mp_dir.join("market.md"), "# Market").unwrap();

        // Cache command
        let cache_dir = root.join("plugins/cache/pkg/commands");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::write(cache_dir.join("cached.md"), "# Cached").unwrap();

        let adapter = ClaudeAdapter::with_root(root.to_path_buf());

        // With flag=true
        let cmds = adapter.read_commands(true).unwrap();
        let names: HashSet<_> = cmds.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains("core"));
        assert!(names.contains("market"));
        assert!(names.contains("cached"));

        // With flag=false
        let cmds_off = adapter.read_commands(false).unwrap();
        let names_off: HashSet<_> = cmds_off.iter().map(|c| c.name.as_str()).collect();
        assert!(names_off.contains("core"));
        assert!(!names_off.contains("market"));
        assert!(names_off.contains("cached"));
    }

    #[test]
    fn marketplace_commands_do_not_override_core_duplicates() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        let core_dir = root.join("commands");
        fs::create_dir_all(&core_dir).unwrap();
        fs::write(core_dir.join("shared.md"), "core").unwrap();

        let mp_dir = root.join("plugins/marketplaces/mp/plugins/tool/commands");
        fs::create_dir_all(&mp_dir).unwrap();
        fs::write(mp_dir.join("shared.md"), "market").unwrap();

        let adapter = ClaudeAdapter::with_root(root.to_path_buf());
        // Must enable marketplace to test collision
        let cmds = adapter.read_commands(true).unwrap();
        let shared: Vec<_> = cmds.iter().filter(|c| c.name == "shared").collect();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].content, b"core".to_vec());
    }

    #[test]
    fn write_commands_creates_files() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let commands = vec![Command {
            name: "hello".to_string(),
            content: b"# Hello World".to_vec(),
            source_path: PathBuf::from("/tmp/hello.md"),
            modified: SystemTime::now(),
            hash: "abc123".to_string(),
        }];

        let report = adapter.write_commands(&commands).unwrap();
        assert_eq!(report.written, 1);

        let written = fs::read(tmp.path().join("commands/hello.md")).unwrap();
        assert_eq!(written, b"# Hello World");
    }

    #[test]
    fn write_commands_skips_unchanged() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let content = b"# Unchanged Command".to_vec();
        let hash = hash_content(&content);

        // Write initial
        let commands = vec![Command {
            name: "unchanged".to_string(),
            content: content.clone(),
            source_path: PathBuf::from("/tmp/unchanged.md"),
            modified: SystemTime::now(),
            hash: hash.clone(),
        }];
        adapter.write_commands(&commands).unwrap();

        // Write again with same hash
        let commands2 = vec![Command {
            name: "unchanged".to_string(),
            content: content.clone(),
            source_path: PathBuf::from("/tmp/unchanged.md"),
            modified: SystemTime::now(),
            hash,
        }];
        let report = adapter.write_commands(&commands2).unwrap();

        assert_eq!(report.written, 0);
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn read_mcp_servers_from_settings() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(
            &settings_path,
            r#"{
            "mcpServers": {
                "test-server": {
                    "command": "/usr/bin/test",
                    "args": ["--flag", "value"],
                    "env": {
                        "VAR": "value"
                    }
                }
            }
        }"#,
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        assert_eq!(servers.len(), 1);
        let server = servers.get("test-server").unwrap();
        assert_eq!(server.command, "/usr/bin/test");
        assert_eq!(server.args, vec!["--flag", "value"]);
        assert_eq!(server.env.get("VAR").unwrap(), "value");
        assert!(server.enabled);
    }

    #[test]
    fn write_mcp_servers_creates_settings() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let mut servers = HashMap::new();
        servers.insert(
            "my-server".to_string(),
            McpServer {
                name: "my-server".to_string(),
                transport: McpTransport::Stdio,
                command: "/bin/server".to_string(),
                args: vec!["arg1".to_string()],
                env: HashMap::new(),
                url: None,
                headers: None,
                enabled: true,
            },
        );

        let report = adapter.write_mcp_servers(&servers).unwrap();
        assert_eq!(report.written, 1);

        let settings_path = tmp.path().join("settings.json");
        assert!(settings_path.exists());

        let content = fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(settings["mcpServers"]["my-server"].is_object());
    }

    #[test]
    fn read_preferences_from_settings() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(
            &settings_path,
            r#"{
            "model": "claude-3-opus-20240229"
        }"#,
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let prefs = adapter.read_preferences().unwrap();

        assert_eq!(prefs.model.as_deref(), Some("claude-3-opus-20240229"));
    }

    #[test]
    fn write_preferences_updates_settings() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let prefs = Preferences {
            model: Some("claude-3-sonnet-20240229".to_string()),
            custom: HashMap::new(),
        };

        let report = adapter.write_preferences(&prefs).unwrap();
        assert_eq!(report.written, 1);

        let settings_path = tmp.path().join("settings.json");
        assert!(settings_path.exists());

        let content = fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(settings["model"].as_str(), Some("claude-3-sonnet-20240229"));
    }

    #[test]
    fn write_skills_writes_skill_md_in_directory() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

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
    fn read_skills_uses_parent_directory_name_for_skill_md() {
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills/nested/foo");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: foo\ndescription: test\n---\n",
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();
        let names: HashSet<_> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains("nested/foo"));
    }

    #[test]
    fn read_skills_from_plugins_cache_extracts_skill_name() {
        let tmp = tempdir().unwrap();

        // Create a skill in plugins cache structure
        // ~/.claude/plugins/cache/marketplace/plugin/1.0.0/skills/my-skill/SKILL.md
        let cache_skill = tmp
            .path()
            .join("plugins/cache/marketplace/plugin/1.0.0/skills/my-skill");
        fs::create_dir_all(&cache_skill).unwrap();
        fs::write(
            cache_skill.join("SKILL.md"),
            "---\nname: my-skill\ndescription: A cached skill\n---\n# My Skill\n",
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();

        // Should extract just "my-skill" as the name, not the full cache path
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
    }

    #[test]
    fn read_mcp_servers_invalid_json_returns_error() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(&settings_path, "{ invalid json }").unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let result = adapter.read_mcp_servers();
        assert!(result.is_err());
    }

    #[test]
    fn read_preferences_invalid_json_returns_error() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(&settings_path, "not valid json at all").unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let result = adapter.read_preferences();
        assert!(result.is_err());
    }

    #[test]
    fn write_mcp_servers_invalid_existing_json_returns_error() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(&settings_path, "{ corrupted json }").unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let mut servers = HashMap::new();
        servers.insert(
            "test-server".to_string(),
            McpServer {
                name: "test-server".to_string(),
                transport: McpTransport::Stdio,
                command: "/bin/test".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                enabled: true,
            },
        );

        let result = adapter.write_mcp_servers(&servers);
        assert!(result.is_err());
    }

    #[test]
    fn write_preferences_invalid_existing_json_returns_error() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(&settings_path, "{ malformed: json, }").unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let prefs = Preferences {
            model: Some("claude-3".to_string()),
            custom: HashMap::new(),
        };

        let result = adapter.write_preferences(&prefs);
        assert!(result.is_err());
    }

    #[test]
    fn read_mcp_servers_http_type() {
        // Test reading HTTP-type MCP servers (issue #111)
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(
            &settings_path,
            r#"{
            "mcpServers": {
                "context7": {
                    "type": "http",
                    "url": "https://mcp.context7.com/mcp",
                    "headers": {
                        "CONTEXT7_API_KEY": "test-key"
                    }
                },
                "skrills": {
                    "type": "stdio",
                    "command": "/usr/bin/skrills",
                    "args": ["serve"]
                }
            }
        }"#,
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        assert_eq!(servers.len(), 2);

        // Verify HTTP server
        let http_server = servers.get("context7").unwrap();
        assert_eq!(http_server.transport, McpTransport::Http);
        assert_eq!(
            http_server.url,
            Some("https://mcp.context7.com/mcp".to_string())
        );
        assert_eq!(
            http_server.headers,
            Some(HashMap::from([(
                "CONTEXT7_API_KEY".to_string(),
                "test-key".to_string()
            )]))
        );

        // Verify stdio server
        let stdio_server = servers.get("skrills").unwrap();
        assert_eq!(stdio_server.transport, McpTransport::Stdio);
        assert_eq!(stdio_server.command, "/usr/bin/skrills");
        assert_eq!(stdio_server.args, vec!["serve"]);
    }

    #[test]
    fn write_mcp_servers_http_type() {
        // Test writing HTTP-type MCP servers (issue #111)
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let mut servers = HashMap::new();
        servers.insert(
            "context7".to_string(),
            McpServer {
                name: "context7".to_string(),
                transport: McpTransport::Http,
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
                url: Some("https://mcp.context7.com/mcp".to_string()),
                headers: Some(HashMap::from([(
                    "X-API-Key".to_string(),
                    "test-key".to_string(),
                )])),
                enabled: true,
            },
        );

        let report = adapter.write_mcp_servers(&servers).unwrap();
        assert_eq!(report.written, 1);

        // Read back and verify
        let read_servers = adapter.read_mcp_servers().unwrap();
        let server = read_servers.get("context7").unwrap();
        assert_eq!(server.transport, McpTransport::Http);
        assert_eq!(server.url, Some("https://mcp.context7.com/mcp".to_string()));
        assert_eq!(
            server.headers,
            Some(HashMap::from([(
                "X-API-Key".to_string(),
                "test-key".to_string()
            ),]))
        );
        // HTTP servers should not have command
        assert!(server.command.is_empty());
    }

    #[test]
    fn read_mcp_servers_unknown_type_falls_back_to_stdio() {
        // Test that unknown transport types fall back to stdio with warning
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(
            &settings_path,
            r#"{
            "mcpServers": {
                "weird-server": {
                    "type": "grpc",
                    "command": "/usr/bin/weird"
                }
            }
        }"#,
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        let server = servers.get("weird-server").unwrap();
        // Unknown types should fall back to stdio
        assert_eq!(server.transport, McpTransport::Stdio);
        assert_eq!(server.command, "/usr/bin/weird");
    }
}
