//! Claude Code adapter for reading/writing ~/.claude configuration.

use super::traits::{AgentAdapter, FieldSupport};
use super::utils::{collect_module_files, hash_content, is_hidden_path, sanitize_name};
use crate::common::{
    Command, ContentFormat, McpServer, McpTransport, ModuleFile, PluginAsset, Preferences,
};
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

    fn instructions_path(&self) -> PathBuf {
        self.root.join("CLAUDE.md")
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
                modules: Vec::new(),
                content_format: ContentFormat::default(),
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
            instructions: true,
            plugin_assets: true,
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
                    modules: Vec::new(),
                    content_format: ContentFormat::default(),
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
                    allowed_tools: config
                        .get("allowedTools")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                    disabled_tools: config
                        .get("disabledTools")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
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
        let process_skill = |skills_map: &mut HashMap<String, Command>,
                             name: String,
                             path: &std::path::Path,
                             modules: Vec<ModuleFile>| {
            let content = match fs::read(path) {
                Ok(c) => c,
                Err(_) => return,
            };
            let metadata = match fs::metadata(path) {
                Ok(m) => m,
                Err(_) => return,
            };
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            let skill = Command {
                name: name.clone(),
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
                modules,

                content_format: ContentFormat::default(),
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
                let (name, modules) = if is_skill_md {
                    let name = path
                        .parent()
                        .and_then(|p| p.strip_prefix(&skills_dir).ok())
                        .and_then(|p| p.to_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("unknown")
                        .to_string();
                    // Collect companion files from the skill directory
                    let skill_dir = path.parent().unwrap_or(path);
                    let modules = collect_module_files(skill_dir);
                    (name, modules)
                } else {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    (name, Vec::new())
                };

                process_skill(&mut skills_map, name, path, modules);
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
                let (name, modules) = if is_skill_md {
                    // Use parent directory name as skill name
                    let name = path
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .filter(|s| !s.is_empty() && *s != "skills")
                        .unwrap_or("unknown")
                        .to_string();
                    // Collect companion files from the skill directory
                    let skill_dir = path.parent().unwrap_or(path);
                    let modules = collect_module_files(skill_dir);
                    (name, modules)
                } else {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    (name, Vec::new())
                };

                if name == "unknown" || name == "skills" {
                    continue;
                }

                process_skill(&mut skills_map, name, path, modules);
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
            if !server.allowed_tools.is_empty() {
                server_config.insert(
                    "allowedTools".into(),
                    serde_json::json!(server.allowed_tools),
                );
            }
            if !server.disabled_tools.is_empty() {
                server_config.insert(
                    "disabledTools".into(),
                    serde_json::json!(server.disabled_tools),
                );
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

            // Write module files (companion files) alongside SKILL.md
            let skill_dir = dir.join(&safe_rel_dir);
            for module in &skill.modules {
                let module_path = skill_dir.join(&module.relative_path);
                if !super::utils::is_path_contained(&module_path, &skill_dir) {
                    tracing::debug!(
                        path = %module.relative_path.display(),
                        "Skipping module with path outside skill directory"
                    );
                    continue;
                }
                if let Some(parent) = module_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&module_path, &module.content)?;
            }

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
                modules: Vec::new(),

                content_format: ContentFormat::default(),
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
                modules: Vec::new(),

                content_format: ContentFormat::default(),
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

    fn read_instructions(&self) -> Result<Vec<Command>> {
        let path = self.instructions_path();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read(&path)?;
        let metadata = fs::metadata(&path)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let hash = hash_content(&content);

        // Use "CLAUDE" as the instruction name (derived from CLAUDE.md)
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("CLAUDE")
            .to_string();

        Ok(vec![Command {
            name,
            content,
            source_path: path.clone(),
            modified,
            hash,
            modules: Vec::new(),

            content_format: ContentFormat::default(),
        }])
    }

    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport> {
        let mut report = WriteReport::default();

        // Claude only supports a single CLAUDE.md file
        // If multiple instructions are provided, merge them or take the first
        if instructions.is_empty() {
            return Ok(report);
        }

        let path = self.instructions_path();

        // Merge all instructions content if multiple are provided
        let merged_content: Vec<u8> = if instructions.len() == 1 {
            instructions[0].content.clone()
        } else {
            // Merge multiple instructions with headers
            let mut merged = Vec::new();
            for (i, instruction) in instructions.iter().enumerate() {
                if i > 0 {
                    merged.extend_from_slice(b"\n\n---\n\n");
                }
                merged.extend_from_slice(
                    format!("<!-- Source: {} -->\n\n", instruction.name).as_bytes(),
                );
                merged.extend_from_slice(&instruction.content);
            }
            merged
        };

        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_content(&existing) == hash_content(&merged_content) {
                report.skipped.push(SkipReason::Unchanged {
                    item: "CLAUDE.md".to_string(),
                });
                return Ok(report);
            }
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &merged_content)?;
        report.written += 1;

        Ok(report)
    }

    fn read_plugin_assets(&self) -> Result<Vec<PluginAsset>> {
        let cache_dir = self.root.join("plugins/cache");
        if !cache_dir.exists() {
            return Ok(vec![]);
        }

        // Directories already handled by other sync paths
        const SYNCED_DIRS: &[&str] = &["skills", "commands", "agents"];
        // Directories to skip (not needed at runtime)
        const SKIP_DIRS: &[&str] = &[
            "tests",
            ".venv",
            "__pycache__",
            "node_modules",
            ".git",
            ".claude-plugin",
            ".cursor-plugin",
        ];

        let mut assets = Vec::new();

        // Walk: cache/<marketplace>/<plugin>/<version>/
        for marketplace_entry in fs::read_dir(&cache_dir)? {
            let marketplace_entry = marketplace_entry?;
            let marketplace_path = marketplace_entry.path();
            if !marketplace_path.is_dir() {
                continue;
            }
            let publisher = marketplace_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            for plugin_entry in fs::read_dir(&marketplace_path)? {
                let plugin_entry = plugin_entry?;
                let plugin_path = plugin_entry.path();
                if !plugin_path.is_dir() {
                    continue;
                }
                let plugin_name = plugin_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                // Find the latest version directory
                let mut versions: Vec<_> = fs::read_dir(&plugin_path)?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .collect();
                versions.sort_by_key(|e| {
                    e.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH)
                });
                let version_entry = match versions.last() {
                    Some(e) => e,
                    None => continue,
                };
                let version_path = version_entry.path();
                let version = version_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("0.0.0")
                    .to_string();

                // Walk the version directory collecting asset files
                for entry in WalkDir::new(&version_path)
                    .min_depth(1)
                    .max_depth(10)
                    .follow_links(false)
                {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let path = entry.path();

                    if !path.is_file() {
                        continue;
                    }

                    let rel_path = match path.strip_prefix(&version_path) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };

                    // Skip hidden files
                    if is_hidden_path(rel_path) {
                        continue;
                    }

                    // Check if this file is under a synced or skipped directory
                    let top_component = rel_path
                        .components()
                        .next()
                        .and_then(|c| c.as_os_str().to_str())
                        .unwrap_or("");

                    if SYNCED_DIRS.contains(&top_component) {
                        continue; // Already synced by skills/commands/agents
                    }
                    if SKIP_DIRS.contains(&top_component) {
                        continue;
                    }
                    // Also check any ancestor for skip dirs (e.g., nested __pycache__)
                    if rel_path.components().any(|c| {
                        c.as_os_str()
                            .to_str()
                            .is_some_and(|s| SKIP_DIRS.contains(&s))
                    }) {
                        continue;
                    }

                    let content = match fs::read(path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    let executable = {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            fs::metadata(path)
                                .map(|m| m.permissions().mode() & 0o111 != 0)
                                .unwrap_or(false)
                        }
                        #[cfg(not(unix))]
                        {
                            false
                        }
                    };

                    let hash = hash_content(&content);

                    assets.push(PluginAsset {
                        plugin_name: plugin_name.clone(),
                        publisher: publisher.clone(),
                        version: version.clone(),
                        relative_path: rel_path.to_path_buf(),
                        content,
                        hash,
                        executable,
                    });
                }
            }
        }

        Ok(assets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::tempdir;

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
            modules: Vec::new(),

            content_format: ContentFormat::default(),
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
            modules: Vec::new(),

            content_format: ContentFormat::default(),
        }];
        adapter.write_commands(&commands).unwrap();

        // Write again with same hash
        let commands2 = vec![Command {
            name: "unchanged".to_string(),
            content: content.clone(),
            source_path: PathBuf::from("/tmp/unchanged.md"),
            modified: SystemTime::now(),
            hash,
            modules: Vec::new(),

            content_format: ContentFormat::default(),
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
                allowed_tools: vec![],
                disabled_tools: vec![],
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
            modules: Vec::new(),

            content_format: ContentFormat::default(),
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
                allowed_tools: vec![],
                disabled_tools: vec![],
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
                allowed_tools: vec![],
                disabled_tools: vec![],
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

    #[test]
    fn read_mcp_servers_with_tool_configs() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join("settings.json");
        fs::write(
            &settings_path,
            r#"{
            "mcpServers": {
                "restricted-server": {
                    "command": "/usr/bin/mcp-server",
                    "args": ["--port", "3000"],
                    "allowedTools": ["read_file", "search_*"],
                    "disabledTools": ["delete_file", "write_file"]
                }
            }
        }"#,
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        let server = servers.get("restricted-server").unwrap();
        assert_eq!(server.allowed_tools, vec!["read_file", "search_*"]);
        assert_eq!(server.disabled_tools, vec!["delete_file", "write_file"]);
    }

    #[test]
    fn write_mcp_servers_preserves_tool_configs() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let mut servers = HashMap::new();
        servers.insert(
            "my-server".to_string(),
            McpServer {
                name: "my-server".to_string(),
                transport: McpTransport::Stdio,
                command: "/bin/server".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                enabled: true,
                allowed_tools: vec!["tool_a".to_string(), "tool_b".to_string()],
                disabled_tools: vec!["tool_c".to_string()],
            },
        );

        adapter.write_mcp_servers(&servers).unwrap();

        // Read back and verify tool configs survived
        let read_back = adapter.read_mcp_servers().unwrap();
        let server = read_back.get("my-server").unwrap();
        assert_eq!(
            server.allowed_tools,
            vec!["tool_a".to_string(), "tool_b".to_string()]
        );
        assert_eq!(server.disabled_tools, vec!["tool_c".to_string()]);
    }

    #[test]
    fn mcp_servers_empty_tool_configs_omitted_from_json() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let mut servers = HashMap::new();
        servers.insert(
            "clean-server".to_string(),
            McpServer {
                name: "clean-server".to_string(),
                transport: McpTransport::Stdio,
                command: "/bin/server".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                enabled: true,
                allowed_tools: vec![],
                disabled_tools: vec![],
            },
        );

        adapter.write_mcp_servers(&servers).unwrap();

        let content = fs::read_to_string(tmp.path().join("settings.json")).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        let server_json = &settings["mcpServers"]["clean-server"];
        // Empty tool configs should not appear in JSON output
        assert!(server_json.get("allowedTools").is_none());
        assert!(server_json.get("disabledTools").is_none());
    }

    #[test]
    fn read_plugin_assets_finds_scripts() {
        let tmp = tempdir().unwrap();
        let plugin_dir = tmp
            .path()
            .join("plugins/cache/market/myplugin/1.0.0/scripts");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(plugin_dir.join("tool.py"), b"# tool\n").unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets().unwrap();

        assert_eq!(assets.len(), 1, "Should find one asset: {:?}", assets);
        assert_eq!(assets[0].plugin_name, "myplugin");
        assert_eq!(assets[0].publisher, "market");
        assert_eq!(assets[0].version, "1.0.0");
        assert_eq!(
            assets[0].relative_path,
            std::path::PathBuf::from("scripts/tool.py")
        );
    }

    #[test]
    fn read_plugin_assets_excludes_skills_and_tests() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("plugins/cache/market/plug/1.0.0");

        // Script (should be included)
        let scripts = base.join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(scripts.join("run.py"), b"# run\n").unwrap();

        // Skill (should be excluded)
        let skills = base.join("skills/my-skill");
        fs::create_dir_all(&skills).unwrap();
        fs::write(skills.join("SKILL.md"), b"# Skill\n").unwrap();

        // Tests (should be excluded)
        let tests = base.join("tests");
        fs::create_dir_all(&tests).unwrap();
        fs::write(tests.join("test_run.py"), b"# test\n").unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets().unwrap();

        assert_eq!(assets.len(), 1, "Should only find the script");
        assert_eq!(
            assets[0].relative_path,
            std::path::PathBuf::from("scripts/run.py")
        );
    }

    #[test]
    fn read_plugin_assets_picks_latest_version() {
        // GIVEN a plugin with two version directories
        let tmp = tempdir().unwrap();
        let old_ver = tmp.path().join("plugins/cache/market/plug/1.0.0/scripts");
        let new_ver = tmp.path().join("plugins/cache/market/plug/2.0.0/scripts");
        fs::create_dir_all(&old_ver).unwrap();
        fs::create_dir_all(&new_ver).unwrap();
        fs::write(old_ver.join("old.py"), b"# old\n").unwrap();

        // Ensure 2.0.0 has a newer mtime by writing after 1.0.0
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(new_ver.join("new.py"), b"# new\n").unwrap();

        // WHEN reading plugin assets
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets().unwrap();

        // THEN only the latest version's files are returned
        assert_eq!(assets.len(), 1, "Should only scan latest version");
        assert_eq!(assets[0].version, "2.0.0");
        assert_eq!(
            assets[0].relative_path,
            std::path::PathBuf::from("scripts/new.py")
        );
    }

    #[test]
    fn read_plugin_assets_skips_hidden_files() {
        // GIVEN a plugin with a hidden file alongside a normal file
        let tmp = tempdir().unwrap();
        let scripts = tmp.path().join("plugins/cache/market/plug/1.0.0/scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(scripts.join("visible.py"), b"# ok\n").unwrap();
        fs::write(scripts.join(".hidden.py"), b"# hidden\n").unwrap();

        // WHEN reading plugin assets
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets().unwrap();

        // THEN hidden files are excluded
        assert_eq!(assets.len(), 1);
        assert_eq!(
            assets[0].relative_path,
            std::path::PathBuf::from("scripts/visible.py")
        );
    }

    #[test]
    fn read_plugin_assets_includes_config_files() {
        // GIVEN a plugin with non-script config files
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("plugins/cache/market/plug/1.0.0");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("pyproject.toml"), b"[project]\n").unwrap();
        fs::write(base.join("Makefile"), b"all:\n\techo ok\n").unwrap();

        // WHEN reading plugin assets
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets().unwrap();

        // THEN config files at the plugin root are included
        assert_eq!(assets.len(), 2);
        let paths: HashSet<String> = assets
            .iter()
            .map(|a| a.relative_path.display().to_string())
            .collect();
        assert!(paths.contains("pyproject.toml"));
        assert!(paths.contains("Makefile"));
    }

    #[test]
    fn read_plugin_assets_empty_when_only_excluded_dirs() {
        // GIVEN a plugin that only has excluded directories
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("plugins/cache/market/plug/1.0.0");
        let skills = base.join("skills/my-skill");
        let tests = base.join("tests");
        let commands = base.join("commands");
        fs::create_dir_all(&skills).unwrap();
        fs::create_dir_all(&tests).unwrap();
        fs::create_dir_all(&commands).unwrap();
        fs::write(skills.join("SKILL.md"), b"# Skill\n").unwrap();
        fs::write(tests.join("test.py"), b"# test\n").unwrap();
        fs::write(commands.join("cmd.md"), b"# cmd\n").unwrap();

        // WHEN reading plugin assets
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets().unwrap();

        // THEN no assets are returned
        assert!(
            assets.is_empty(),
            "All dirs are excluded, expected 0 assets"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_plugin_assets_detects_executable_permission() {
        use std::os::unix::fs::PermissionsExt;

        // GIVEN a plugin with an executable script and a non-executable file
        let tmp = tempdir().unwrap();
        let scripts = tmp.path().join("plugins/cache/market/plug/1.0.0/scripts");
        fs::create_dir_all(&scripts).unwrap();

        let exec_path = scripts.join("run.sh");
        fs::write(&exec_path, b"#!/bin/sh\necho ok\n").unwrap();
        fs::set_permissions(&exec_path, fs::Permissions::from_mode(0o755)).unwrap();

        let data_path = scripts.join("config.json");
        fs::write(&data_path, b"{}").unwrap();
        fs::set_permissions(&data_path, fs::Permissions::from_mode(0o644)).unwrap();

        // WHEN reading plugin assets
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets().unwrap();

        // THEN the executable flag is set correctly
        assert_eq!(assets.len(), 2);
        let exec_asset = assets.iter().find(|a| a.relative_path.ends_with("run.sh"));
        let data_asset = assets
            .iter()
            .find(|a| a.relative_path.ends_with("config.json"));
        assert!(
            exec_asset.unwrap().executable,
            "run.sh should be marked executable"
        );
        assert!(
            !data_asset.unwrap().executable,
            "config.json should not be marked executable"
        );
    }
}
