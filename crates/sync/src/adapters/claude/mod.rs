//! Claude Code adapter for reading/writing ~/.claude configuration.

use super::traits::{AgentAdapter, FieldSupport};
#[cfg(test)]
use super::utils::hash_content;
#[cfg(test)]
use crate::common::{ContentFormat, McpTransport};
use crate::common::{Command, McpServer, PluginAsset, Preferences};
use crate::report::WriteReport;
use crate::Result;
use anyhow::Context;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
#[cfg(test)]
use std::time::SystemTime;

mod agents;
mod commands;
mod hooks;
mod instructions;
mod plugin_assets;
mod settings;
mod skills;

/// Parse a directory entry name as a semver-ish tuple `(major, minor, patch)`.
///
/// Falls back to `(0, 0, 0)` for non-semver names so they sort before any real version.
pub(super) fn semver_tuple(entry: &fs::DirEntry) -> (u64, u64, u64) {
    let name = entry.file_name().to_str().map(str::to_owned).unwrap_or_else(|| {
        tracing::warn!(
            ?entry,
            "non-UTF-8 directory name; sorted before all valid semver entries"
        );
        String::new()
    });
    let parts: Vec<&str> = name.split('.').collect();
    let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0u64);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0u64);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0u64);
    (major, minor, patch)
}

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

    /// Returns a borrow of the adapter's config root.
    ///
    /// Used by sibling submodules (e.g. `plugin_assets`) that need to
    /// derive paths off the root without taking ownership.
    pub(in crate::adapters::claude) fn config_root_ref(&self) -> &PathBuf {
        &self.root
    }

    pub(super) fn commands_dir(&self) -> PathBuf {
        self.root.join("commands")
    }

    pub(super) fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    pub(super) fn hooks_dir(&self) -> PathBuf {
        self.root.join("hooks")
    }

    pub(super) fn agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    pub(super) fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub(super) fn instructions_path(&self) -> PathBuf {
        self.root.join("CLAUDE.md")
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
        commands::read_commands_impl(self, include_marketplace)
    }

    fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>> {
        settings::read_mcp_servers_impl(self)
    }

    fn read_preferences(&self) -> Result<Preferences> {
        settings::read_preferences_impl(self)
    }

    fn read_skills(&self) -> Result<Vec<Command>> {
        skills::read_skills_impl(self)
    }

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        commands::write_commands_impl(self, commands)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        settings::write_mcp_servers_impl(self, servers)
    }

    fn write_preferences(&self, prefs: &Preferences) -> Result<WriteReport> {
        settings::write_preferences_impl(self, prefs)
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        skills::write_skills_impl(self, skills)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        hooks::read_hooks_impl(self)
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        agents::read_agents_impl(self)
    }

    fn write_hooks(&self, hooks: &[Command]) -> Result<WriteReport> {
        hooks::write_hooks_impl(self, hooks)
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        agents::write_agents_impl(self, agents)
    }

    fn read_instructions(&self) -> Result<Vec<Command>> {
        instructions::read_instructions_impl(self)
    }

    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport> {
        instructions::write_instructions_impl(self, instructions)
    }

    fn read_plugin_assets(&self, full_mirror: bool) -> Result<Vec<PluginAsset>> {
        plugin_assets::read_plugin_assets_impl(self, full_mirror)
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
        crate::adapters::tests_common::assert_read_commands_empty(&adapter);
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
            plugin_origin: None,
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
            plugin_origin: None,
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
            plugin_origin: None,
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
            plugin_origin: None,
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
        let assets = adapter.read_plugin_assets(false).unwrap();

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
        let assets = adapter.read_plugin_assets(false).unwrap();

        assert_eq!(assets.len(), 1, "Should only find the script");
        assert_eq!(
            assets[0].relative_path,
            std::path::PathBuf::from("scripts/run.py")
        );
    }

    #[test]
    fn read_plugin_assets_picks_latest_version() {
        // GIVEN a plugin with two version directories (semver-sorted)
        let tmp = tempdir().unwrap();
        let old_ver = tmp.path().join("plugins/cache/market/plug/1.0.0/scripts");
        let new_ver = tmp.path().join("plugins/cache/market/plug/2.0.0/scripts");
        fs::create_dir_all(&old_ver).unwrap();
        fs::create_dir_all(&new_ver).unwrap();
        fs::write(old_ver.join("old.py"), b"# old\n").unwrap();
        fs::write(new_ver.join("new.py"), b"# new\n").unwrap();

        // WHEN reading plugin assets
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets(false).unwrap();

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
        let assets = adapter.read_plugin_assets(false).unwrap();

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
        let assets = adapter.read_plugin_assets(false).unwrap();

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
        let assets = adapter.read_plugin_assets(false).unwrap();

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
        let assets = adapter.read_plugin_assets(false).unwrap();

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
