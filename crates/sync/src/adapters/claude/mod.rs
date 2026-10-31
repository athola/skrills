//! Claude Code adapter for reading/writing ~/.claude configuration.

use super::traits::{AgentAdapter, FieldSupport};
#[cfg(test)]
use super::utils::hash_content;
use crate::common::{Command, McpServer, PluginAsset, Preferences};
#[cfg(test)]
use crate::common::{ContentFormat, McpTransport};
use crate::report::WriteReport;
use crate::Result;
use anyhow::Context;
use std::collections::HashMap;
#[cfg(test)]
use std::fs;
use std::path::PathBuf;
#[cfg(test)]
use std::time::SystemTime;

mod agents;
mod commands;
mod hooks;
mod instructions;
mod plugin_assets;
mod plugin_cache;
mod settings;
mod skills;

/// Adapter for Claude Code configuration.
pub struct ClaudeAdapter {
    root: PathBuf,
    kill_switch: Option<skrills_snapshot::KillSwitch>,
    marketplace_skills: bool,
}

impl ClaudeAdapter {
    /// Creates a new ClaudeAdapter with the default root (~/.claude).
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(Self {
            root: home.join(".claude"),
            kill_switch: None,
            marketplace_skills: false,
        })
    }

    /// Creates a ClaudeAdapter with a custom root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            kill_switch: None,
            marketplace_skills: false,
        }
    }

    /// Attach a [`KillSwitch`](skrills_snapshot::KillSwitch) so that mutating
    /// operations refuse with [`SyncError::KillSwitchEngaged`](crate::SyncError)
    /// when the cold-window engine has engaged it (FR12).
    #[must_use]
    pub fn with_kill_switch(mut self, switch: skrills_snapshot::KillSwitch) -> Self {
        self.kill_switch = Some(switch);
        self
    }

    /// Also read the skills of marketplace plugins that are not installed
    /// (`plugins/marketplaces`). A skill of the same name in `skills/` or the
    /// plugin cache wins.
    #[must_use]
    pub fn with_marketplace_skills(mut self, include: bool) -> Self {
        self.marketplace_skills = include;
        self
    }

    /// Returns a borrow of the adapter's config root.
    ///
    /// Used by sibling submodules (e.g. `plugin_assets`) that need to
    /// derive paths off the root without taking ownership.
    pub(in crate::adapters::claude) fn config_root_ref(&self) -> &std::path::Path {
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

    /// Where Claude Code keeps user-scoped MCP servers: `~/.claude.json`,
    /// beside `~/.claude` (code.claude.com/docs/en/mcp). A root that is not a
    /// `.claude` directory (a test or mirror root) keeps the file inside it.
    pub(super) fn mcp_config_path(&self) -> PathBuf {
        match (self.root.file_name(), self.root.parent()) {
            (Some(name), Some(parent)) if name == ".claude" => parent.join(".claude.json"),
            _ => self.root.join(".claude.json"),
        }
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

    fn read_support(&self) -> FieldSupport {
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

    /// Claude is the plugin cache's owner: it reads plugin assets and never
    /// writes them, so `write_plugin_assets` is not implemented and the trait
    /// default would report a successful write of nothing.
    fn write_support(&self) -> FieldSupport {
        FieldSupport {
            plugin_assets: false,
            ..self.read_support()
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
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        commands::write_commands_impl(self, commands)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        settings::write_mcp_servers_impl(self, servers)
    }

    fn write_preferences(&self, prefs: &Preferences) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        settings::write_preferences_impl(self, prefs)
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        skills::write_skills_impl(self, skills)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        hooks::read_hooks_impl(self)
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        agents::read_agents_impl(self)
    }

    fn write_hooks(&self, hooks: &[Command]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        hooks::write_hooks_impl(self, hooks)
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        agents::write_agents_impl(self, agents)
    }

    fn read_instructions(&self) -> Result<Vec<Command>> {
        instructions::read_instructions_impl(self)
    }

    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
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

        // Cache command, in the real <publisher>/<plugin>/<version> layout
        let cache_dir = root.join("plugins/cache/mp/pkg/1.0.0/commands");
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

    /// Claude Code keeps user-scoped MCP servers in `~/.claude.json`
    /// (code.claude.com/docs/en/mcp); it never reads `mcpServers` from
    /// `~/.claude/settings.json`.
    #[test]
    fn read_mcp_servers_from_claude_json() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("settings.json"),
            r#"{"mcpServers": {"ignored": {"command": "/bin/ignored"}}}"#,
        )
        .unwrap();
        let settings_path = tmp.path().join(".claude.json");
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
    fn write_mcp_servers_writes_claude_json_not_settings() {
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

        assert!(!tmp.path().join("settings.json").exists());
        let config_path = tmp.path().join(".claude.json");
        let content = fs::read_to_string(&config_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(settings["mcpServers"]["my-server"].is_object());
    }

    /// `~/.claude.json` sits beside `~/.claude`, not inside it.
    #[test]
    fn mcp_config_of_a_dot_claude_root_is_its_sibling_claude_json() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(".claude");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            tmp.path().join(".claude.json"),
            r#"{"mcpServers": {"user": {"command": "/bin/user"}}}"#,
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(root);
        let servers = adapter.read_mcp_servers().unwrap();

        assert_eq!(servers.keys().collect::<Vec<_>>(), vec!["user"]);
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
        let settings_path = tmp.path().join(".claude.json");
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
        let settings_path = tmp.path().join(".claude.json");
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
        let settings_path = tmp.path().join(".claude.json");
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

    /// An `sse` server read as stdio was written to every other tool with an
    /// empty command and no URL. It is left out instead; the merging writers
    /// then keep whatever the target already has under that name.
    #[test]
    fn read_mcp_servers_skips_a_transport_sync_cannot_carry() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join(".claude.json");
        fs::write(
            &settings_path,
            r#"{
            "mcpServers": {
                "streaming": {"type": "sse", "url": "https://x/sse"},
                "plain": {"command": "/usr/bin/plain"}
            }
        }"#,
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        assert!(!servers.contains_key("streaming"));
        assert_eq!(servers["plain"].transport, McpTransport::Stdio);
    }

    #[test]
    fn read_mcp_servers_with_tool_configs() {
        let tmp = tempdir().unwrap();
        let settings_path = tmp.path().join(".claude.json");
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

        let content = fs::read_to_string(tmp.path().join(".claude.json")).unwrap();
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

    /// Claude Code loads hooks only from the `hooks` key of its settings
    /// files, plugin `hooks/hooks.json` and skill/agent frontmatter
    /// (code.claude.com/docs/en/hooks), so a Markdown hook written to
    /// `~/.claude/hooks/*.md` did nothing while reported as synced.
    #[test]
    fn write_hooks_writes_no_markdown_hook_files_claude_never_loads() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let hook = Command {
            name: "PreToolUse".to_string(),
            content: b"# Check before tools\n".to_vec(),
            source_path: PathBuf::from("/c/hooks/PreToolUse.md"),
            modified: SystemTime::now(),
            hash: String::new(),
            modules: Vec::new(),
            content_format: ContentFormat::Markdown,
            plugin_origin: None,
        };

        let report = adapter.write_hooks(&[hook]).unwrap();

        assert_eq!(report.written, 0);
        assert!(!tmp.path().join("hooks").exists());
        assert!(matches!(
            report.skipped.as_slice(),
            [crate::report::SkipReason::AgentSpecificFeature { .. }]
        ));
    }

    /// Cursor hooks are JSON entry lists; Claude reads hooks from
    /// `settings.json`, not `~/.claude/hooks/*.md`, so writing them there
    /// produced files Claude never loads while reporting them as synced.
    #[test]
    fn write_hooks_skips_json_hooks_instead_of_writing_dead_files() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let hook = Command {
            name: "PreToolUse".to_string(),
            content: br#"[{"command": "./check.sh"}]"#.to_vec(),
            source_path: PathBuf::from("/c/hooks.json"),
            modified: SystemTime::now(),
            hash: String::new(),
            modules: Vec::new(),
            content_format: ContentFormat::Json,
            plugin_origin: None,
        };

        let report = adapter.write_hooks(&[hook]).unwrap();

        assert_eq!(report.written, 0);
        assert!(!tmp.path().join("hooks/PreToolUse.md").exists());
        assert!(matches!(
            report.skipped.as_slice(),
            [crate::report::SkipReason::AgentSpecificFeature { .. }]
        ));
    }

    fn write_at(path: &std::path::Path, body: &str, mtime_secs: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(mtime_secs))
            .unwrap();
    }

    /// Cached plugin commands, agents and hooks carried no plugin origin, so
    /// `exclude_plugins` could not filter them.
    #[test]
    fn plugin_cache_items_carry_their_plugin_origin() {
        let tmp = tempdir().unwrap();
        let version = tmp.path().join("plugins/cache/market/plug/1.0.0");
        write_at(&version.join("commands/c.md"), "c", 1_000);
        write_at(&version.join("agents/a.md"), "a", 1_000);
        write_at(&version.join("hooks/h.md"), "h", 1_000);

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let items = adapter
            .read_commands(false)
            .unwrap()
            .into_iter()
            .chain(adapter.read_agents().unwrap())
            .chain(adapter.read_hooks().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(items.len(), 3);
        for item in items {
            let origin = item.plugin_origin.expect("plugin origin");
            assert_eq!(origin.plugin_name, "plug", "{}", item.name);
            assert_eq!(origin.version, "1.0.0");
        }
    }

    /// Skills were picked by mtime across every cached version and commands by
    /// walk order, so one sync could mix two versions of a plugin.
    #[test]
    fn every_plugin_artifact_comes_from_the_latest_version() {
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugins/cache/market/plug");
        // The old version's files are newer on disk.
        write_at(&plugin.join("1.9.0/skills/s/SKILL.md"), "old", 2_000);
        write_at(&plugin.join("1.9.0/commands/c.md"), "old", 2_000);
        write_at(&plugin.join("1.10.0/skills/s/SKILL.md"), "new", 1_000);
        write_at(&plugin.join("1.10.0/commands/c.md"), "new", 1_000);

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let skill = adapter.read_skills().unwrap().pop().unwrap();
        let command = adapter.read_commands(false).unwrap().pop().unwrap();

        assert_eq!(skill.content, b"new");
        assert_eq!(command.content, b"new");
    }

    /// The readers returned `HashMap` order, so which of two colliding items a
    /// writer kept, and the report order, changed between runs.
    #[test]
    fn skills_agents_and_hooks_come_back_sorted_by_name() {
        let tmp = tempdir().unwrap();
        let names: Vec<String> = (0..16).map(|i| format!("item-{:02}", 15 - i)).collect();
        for name in &names {
            write_at(
                &tmp.path().join(format!("skills/{name}/SKILL.md")),
                "s",
                1_000,
            );
            write_at(&tmp.path().join(format!("agents/{name}.md")), "a", 1_000);
            write_at(&tmp.path().join(format!("hooks/{name}.md")), "h", 1_000);
        }
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let mut sorted = names.clone();
        sorted.sort();

        for (kind, items) in [
            ("skills", adapter.read_skills().unwrap()),
            ("agents", adapter.read_agents().unwrap()),
            ("hooks", adapter.read_hooks().unwrap()),
        ] {
            let got: Vec<String> = items.into_iter().map(|c| c.name).collect();
            assert_eq!(got, sorted, "{kind}");
        }
    }

    /// `commands/git/commit.md` and `commands/commit.md` are both `commit`;
    /// the one kept depended on readdir order. The top-level file wins.
    #[test]
    fn a_top_level_command_wins_over_a_nested_one_of_the_same_name() {
        let tmp = tempdir().unwrap();
        write_at(&tmp.path().join("commands/a/commit.md"), "nested a", 1_000);
        write_at(&tmp.path().join("commands/commit.md"), "top", 1_000);
        write_at(&tmp.path().join("commands/z/commit.md"), "nested z", 1_000);

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands(false).unwrap();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].content, b"top");
    }

    /// `commands`/`agents` used to match anywhere in a plugin, so a README in
    /// `docs/commands/` became a command and a skill's `agents/` module an
    /// agent, both written into every target.
    #[test]
    fn plugin_cache_kinds_come_only_from_the_top_level_directory() {
        let tmp = tempdir().unwrap();
        let version = tmp.path().join("plugins/cache/market/plug/1.0.0");
        write_at(&version.join("commands/real.md"), "c", 1_000);
        write_at(&version.join("docs/commands/README.md"), "doc", 1_000);
        write_at(&version.join("agents/real-agent.md"), "a", 1_000);
        write_at(&version.join("skills/foo/SKILL.md"), "s", 1_000);
        write_at(
            &version.join("skills/foo/agents/helper.md"),
            "module",
            1_000,
        );

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let commands: Vec<String> = adapter
            .read_commands(false)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        let agents: Vec<String> = adapter
            .read_agents()
            .unwrap()
            .into_iter()
            .map(|a| a.name)
            .collect();

        assert_eq!(commands, vec!["real".to_string()]);
        assert_eq!(agents, vec!["real-agent".to_string()]);
    }

    /// Plugin skills are keyed by their folder name, so a plugin shipping a
    /// skill named like the user's own one replaced it whenever the plugin's
    /// file was newer.
    #[test]
    fn read_skills_keeps_the_local_skill_over_a_newer_plugin_skill_of_the_same_name() {
        let tmp = tempdir().unwrap();
        write_at(&tmp.path().join("skills/review/SKILL.md"), "mine", 1_000);
        write_at(
            &tmp.path()
                .join("plugins/cache/market/plug/1.0.0/skills/review/SKILL.md"),
            "plugin",
            2_000,
        );

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].content, b"mine");
        assert!(skills[0].plugin_origin.is_none());
    }

    /// Two plugins shipping a skill of the same name: the newer file wins
    /// whichever plugin the cache walk meets first. Swapping the mtimes covers
    /// both walk orders, since `read_dir` order is not fixed.
    #[test]
    fn read_skills_keeps_the_newer_of_two_plugin_skills_with_the_same_name() {
        for (alpha_mtime, beta_mtime, newer) in [(1_000, 2_000, "beta"), (2_000, 1_000, "alpha")] {
            let tmp = tempdir().unwrap();
            write_at(
                &tmp.path()
                    .join("plugins/cache/market/alpha/1.0.0/skills/review/SKILL.md"),
                "alpha",
                alpha_mtime,
            );
            write_at(
                &tmp.path()
                    .join("plugins/cache/market/beta/1.0.0/skills/review/SKILL.md"),
                "beta",
                beta_mtime,
            );

            let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
            let skills = adapter.read_skills().unwrap();

            assert_eq!(skills.len(), 1);
            assert_eq!(skills[0].content, newer.as_bytes());
            assert_eq!(skills[0].plugin_origin.as_ref().unwrap().plugin_name, newer);
        }
    }

    /// A markdown file inside a skill directory is one of its modules, not a
    /// second skill named after the file.
    #[test]
    fn read_skills_does_not_turn_a_skill_module_into_a_skill() {
        let tmp = tempdir().unwrap();
        let skills = tmp.path().join("skills");
        write_at(&skills.join("foo/SKILL.md"), "foo", 1_000);
        write_at(&skills.join("foo/docs/reference.md"), "ref", 1_000);
        write_at(&skills.join("legacy.md"), "legacy", 1_000);
        let cached = tmp.path().join("plugins/cache/market/plug/1.0.0/skills");
        write_at(&cached.join("bar/SKILL.md"), "bar", 1_000);
        write_at(&cached.join("bar/notes.md"), "notes", 1_000);

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let mut names: Vec<_> = adapter
            .read_skills()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        names.sort();

        assert_eq!(names, vec!["bar", "foo", "legacy"]);
    }

    /// The core walks skip hidden paths and symlinks; the cache walks did not.
    #[cfg(unix)]
    #[test]
    fn plugin_cache_walk_skips_hidden_paths_and_symlinks() {
        let tmp = tempdir().unwrap();
        let version = tmp.path().join("plugins/cache/market/plug/1.0.0");
        write_at(&version.join(".git/commands/hidden.md"), "x", 1_000);
        write_at(&tmp.path().join("elsewhere.md"), "x", 1_000);
        fs::create_dir_all(version.join("commands")).unwrap();
        std::os::unix::fs::symlink(
            tmp.path().join("elsewhere.md"),
            version.join("commands/linked.md"),
        )
        .unwrap();
        write_at(&version.join("commands/real.md"), "x", 1_000);

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let names: Vec<_> = adapter
            .read_commands(false)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();

        assert_eq!(names, vec!["real"]);
    }

    #[cfg(unix)]
    #[test]
    fn read_plugin_assets_skips_symlinks_out_of_the_plugin_but_keeps_in_tree_ones() {
        let tmp = tempdir().unwrap();
        let version = tmp.path().join("plugins/cache/market/myplugin/1.0.0");
        fs::create_dir_all(version.join("scripts")).unwrap();
        fs::write(version.join("scripts/tool.py"), b"# tool\n").unwrap();
        fs::write(tmp.path().join("secret"), b"key").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("secret"), version.join("scripts/leak"))
            .unwrap();
        std::os::unix::fs::symlink(
            version.join("scripts/tool.py"),
            version.join("scripts/alias"),
        )
        .unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let mut paths: Vec<_> = adapter
            .read_plugin_assets(false)
            .unwrap()
            .into_iter()
            .map(|a| a.relative_path)
            .collect();
        paths.sort();

        assert_eq!(
            paths,
            vec![
                std::path::PathBuf::from("scripts/alias"),
                std::path::PathBuf::from("scripts/tool.py")
            ]
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

    /// Every asset of every plugin was held in memory with only a per-file
    /// cap. Past the total budget the read stops, and the batch carries no
    /// manifest so the mirror does not prune what it did not get to.
    #[test]
    fn read_plugin_assets_stops_at_the_total_budget_without_an_authoritative_batch() {
        let tmp = tempdir().unwrap();
        for plugin in ["a", "b", "c"] {
            let root = tmp
                .path()
                .join(format!("plugins/cache/market/{plugin}/1.0.0"));
            fs::create_dir_all(root.join(".claude-plugin")).unwrap();
            fs::write(root.join(".claude-plugin/plugin.json"), b"{}").unwrap();
            fs::create_dir_all(root.join("bin")).unwrap();
            fs::write(root.join("bin/tool"), vec![b'x'; 100]).unwrap();
        }
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let assets = plugin_assets::read_plugin_assets_capped(&adapter, true, 150).unwrap();

        let total: usize = assets.iter().map(|a| a.content.len()).sum();
        assert!(total <= 150, "read {total} bytes");
        assert!(
            manifests_in(&assets).is_empty(),
            "{:?}",
            manifests_in(&assets)
        );
    }

    fn manifests_in(assets: &[PluginAsset]) -> Vec<String> {
        let manifest = std::path::Path::new(".claude-plugin").join("plugin.json");
        let mut names: Vec<String> = assets
            .iter()
            .filter(|a| a.relative_path == manifest)
            .map(|a| a.plugin_name.clone())
            .collect();
        names.sort();
        names
    }

    /// The manifest is what lets the Cursor mirror sweep files the plugin no
    /// longer ships. A file that could not be read this run is not "no longer
    /// shipped", so that plugin's batch must not carry the manifest.
    #[cfg(unix)]
    #[test]
    fn read_plugin_assets_full_mirror_drops_the_manifest_of_a_plugin_with_an_unreadable_file() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        for plugin in ["broken", "fine"] {
            let root = tmp
                .path()
                .join(format!("plugins/cache/market/{plugin}/1.0.0"));
            fs::create_dir_all(root.join(".claude-plugin")).unwrap();
            fs::write(root.join(".claude-plugin/plugin.json"), b"{}").unwrap();
            fs::create_dir_all(root.join("hooks")).unwrap();
            fs::write(root.join("hooks/run.sh"), b"echo\n").unwrap();
        }
        let unreadable = tmp
            .path()
            .join("plugins/cache/market/broken/1.0.0/hooks/run.sh");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&unreadable).is_ok() {
            return; // running as root: permissions do not apply
        }

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets(true).unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(manifests_in(&assets), vec!["fine".to_string()]);
    }

    /// A cache directory that could not be listed hides whole plugins, which
    /// the mirror would then prune as uninstalled. No plugin in such a batch
    /// may be authoritative.
    #[cfg(unix)]
    #[test]
    fn read_plugin_assets_full_mirror_carries_no_manifest_when_the_cache_could_not_be_listed() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        for publisher in ["locked", "open"] {
            let root = tmp
                .path()
                .join(format!("plugins/cache/{publisher}/plug-{publisher}/1.0.0"));
            fs::create_dir_all(root.join(".claude-plugin")).unwrap();
            fs::write(root.join(".claude-plugin/plugin.json"), b"{}").unwrap();
        }
        let locked = tmp.path().join("plugins/cache/locked");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&locked).is_ok() {
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
            return; // running as root: permissions do not apply
        }

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets(true).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            manifests_in(&assets).is_empty(),
            "an incomplete listing must not produce an authoritative batch: {:?}",
            manifests_in(&assets)
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

    #[test]
    fn read_plugin_assets_synthesizes_missing_manifest_in_full_mirror() {
        // GIVEN a plugin with no .claude-plugin/plugin.json (e.g. typescript-lsp /
        // pyright-lsp from claude-plugins-official), only README and LICENSE
        let tmp = tempdir().unwrap();
        let plugin = tmp
            .path()
            .join("plugins/cache/claude-plugins-official/typescript-lsp/1.0.0");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("README.md"),
            b"# typescript-lsp\n\nTypeScript/JavaScript language server for Claude Code.\n",
        )
        .unwrap();
        fs::write(plugin.join("LICENSE"), b"MIT").unwrap();

        // WHEN reading plugin assets in full_mirror mode (Cursor-style)
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets(true).unwrap();

        // THEN a synthetic .claude-plugin/plugin.json is included so the plugin
        // shows up in the target's plugin list rather than being silently dropped.
        let manifest = assets
            .iter()
            .find(|a| a.relative_path == std::path::Path::new(".claude-plugin/plugin.json"))
            .expect("synthetic manifest should be emitted");

        assert_eq!(manifest.plugin_name, "typescript-lsp");
        assert_eq!(manifest.publisher, "claude-plugins-official");
        assert_eq!(manifest.version, "1.0.0");
        assert!(!manifest.executable);

        let json: serde_json::Value = serde_json::from_slice(&manifest.content)
            .expect("synthetic manifest should be valid JSON");
        assert_eq!(json["name"], "typescript-lsp");
        assert_eq!(json["version"], "1.0.0");

        // AND the description is derived from the README's first prose line
        // (skipping the heading), pinning the README-extraction contract.
        assert_eq!(
            json["description"], "TypeScript/JavaScript language server for Claude Code.",
            "description should be the first non-heading prose line from README.md"
        );

        // AND the audit marker is present so future tooling can distinguish
        // synthesized manifests from real ones.
        assert_eq!(
            json["_synthesized"], true,
            "_synthesized: true marker is required for downstream audits"
        );

        // AND the component arrays are present and empty, matching the
        // on-disk reality of MCP-server-only plugins.
        for field in ["commands", "skills", "agents", "hooks"] {
            let arr = json[field]
                .as_array()
                .unwrap_or_else(|| panic!("synthetic manifest must have `{field}` as array"));
            assert!(
                arr.is_empty(),
                "synthetic manifest's `{field}` must be an empty array, got {arr:?}"
            );
        }
    }

    #[test]
    fn read_plugin_assets_synthesized_description_falls_back_when_no_readme() {
        // GIVEN a manifest-less plugin that also ships no README.md
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugins/cache/market/lonely/2.3.4");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("LICENSE"), b"MIT").unwrap();

        // WHEN reading plugin assets in full_mirror mode
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets(true).unwrap();

        // THEN the synthesized manifest uses the documented fallback string,
        // ensuring `description` is always populated even without a README.
        let manifest = assets
            .iter()
            .find(|a| a.relative_path == std::path::Path::new(".claude-plugin/plugin.json"))
            .expect("synthetic manifest should still be emitted without a README");
        let json: serde_json::Value =
            serde_json::from_slice(&manifest.content).expect("manifest should parse");
        assert_eq!(
            json["description"], "Plugin 'lonely' (manifest synthesized by skrills sync)",
            "fallback description must reference the plugin name and synthesis source"
        );
    }

    /// The synthesized `plugin.json` of the only plugin under `root`.
    fn synthesized_manifest(root: &std::path::Path) -> serde_json::Value {
        let assets = ClaudeAdapter::with_root(root.to_path_buf())
            .read_plugin_assets(true)
            .unwrap();
        let manifest = assets
            .iter()
            .find(|a| a.relative_path == std::path::Path::new(".claude-plugin/plugin.json"))
            .expect("a manifest-less plugin gets a synthesized manifest");
        serde_json::from_slice(&manifest.content).expect("synthesized manifest is JSON")
    }

    /// A version directory of manifest-less plugin `name` with `readme` as README.md.
    fn manifest_less_plugin(root: &std::path::Path, name: &str, version: &str, readme: &[u8]) {
        let dir = root.join("plugins/cache/market").join(name).join(version);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("README.md"), readme).unwrap();
    }

    #[test]
    fn synthesized_description_falls_back_when_readme_has_only_headings() {
        let tmp = tempdir().unwrap();
        manifest_less_plugin(tmp.path(), "titled", "1.0.0", b"# Title\n## Subtitle\n");

        let json = synthesized_manifest(tmp.path());
        assert_eq!(
            json["description"],
            "Plugin 'titled' (manifest synthesized by skrills sync)"
        );
    }

    #[test]
    fn synthesized_description_skips_leading_blank_lines() {
        let tmp = tempdir().unwrap();
        manifest_less_plugin(tmp.path(), "spaced", "1.0.0", b"\n\n  \nFirst real line.\n");

        assert_eq!(
            synthesized_manifest(tmp.path())["description"],
            "First real line."
        );
    }

    #[test]
    fn synthesized_description_truncates_multibyte_text_at_280_chars() {
        let tmp = tempdir().unwrap();
        let line = "🦀".repeat(300);
        manifest_less_plugin(
            tmp.path(),
            "crabby",
            "1.0.0",
            format!("{line}\n").as_bytes(),
        );

        let json = synthesized_manifest(tmp.path());
        let description = json["description"].as_str().unwrap();
        assert_eq!(description.chars().count(), 280);
        assert!(description.chars().all(|c| c == '🦀'), "{description}");
    }

    /// Today a README that is not UTF-8 falls back without a warning (#228 T4).
    #[test]
    fn synthesized_description_falls_back_when_readme_is_not_utf8() {
        let tmp = tempdir().unwrap();
        manifest_less_plugin(tmp.path(), "latin", "1.0.0", b"Caf\xe9 au lait\n");

        assert_eq!(
            synthesized_manifest(tmp.path())["description"],
            "Plugin 'latin' (manifest synthesized by skrills sync)"
        );
    }

    #[test]
    fn synthesized_manifest_comes_from_the_highest_version() {
        let tmp = tempdir().unwrap();
        for version in ["1.0.0", "2.0.0", "1.5.0", "1.10.0"] {
            let readme = format!("Version {version}\n");
            manifest_less_plugin(tmp.path(), "multi", version, readme.as_bytes());
        }

        let json = synthesized_manifest(tmp.path());
        assert_eq!(json["version"], "2.0.0");
        assert_eq!(json["description"], "Version 2.0.0");
    }

    #[cfg(unix)]
    #[test]
    fn synthesized_manifest_escapes_a_plugin_name_with_a_quote() {
        let tmp = tempdir().unwrap();
        manifest_less_plugin(tmp.path(), "foo\"bar", "1.0.0", b"Quoted.\n");

        let json = synthesized_manifest(tmp.path());
        assert_eq!(json["name"], "foo\"bar");
        assert_eq!(
            json["description"], "Quoted.",
            "the name must not break the JSON around it"
        );
    }

    #[test]
    fn synthesized_manifest_is_emitted_for_an_empty_version_directory() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("plugins/cache/market/hollow/0.1.0");
        fs::create_dir_all(&dir).unwrap();

        let json = synthesized_manifest(tmp.path());
        assert_eq!(json["name"], "hollow");
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(
            json["description"],
            "Plugin 'hollow' (manifest synthesized by skrills sync)"
        );
    }

    #[test]
    fn read_plugin_assets_does_not_synthesize_in_skill_mirror_mode() {
        // GIVEN a manifest-less plugin (same shape as typescript-lsp)
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugins/cache/market/skill-only/1.0.0");
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("README.md"), b"# skill-only\n\nA plugin.\n").unwrap();

        // WHEN reading plugin assets in BOTH modes against the same fixture
        // (running both calls on one fixture is what differentiates "gating
        // works" from "synthesis is missing entirely", without the
        // full-mirror call, this test would also pass against a reverted
        // implementation that never synthesizes anything).
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let full_mirror_assets = adapter.read_plugin_assets(true).unwrap();
        let skill_mirror_assets = adapter.read_plugin_assets(false).unwrap();

        let manifest_rel_path = std::path::Path::new(".claude-plugin").join("plugin.json");

        // THEN full-mirror mode synthesizes a manifest for the manifest-less
        // plugin. Asserting this is what makes the test fail on revert:
        // pre-fix code never synthesized in any mode, so a skill-mirror-only
        // assertion would silently pass against a reverted implementation.
        assert!(
            full_mirror_assets
                .iter()
                .any(|a| a.relative_path == manifest_rel_path),
            "full-mirror mode should synthesize a manifest for the manifest-less plugin"
        );

        // AND skill-mirror mode does NOT, synthesis must be gated to
        // full_mirror=true so Codex-style skill-only sync targets stay
        // untouched.
        assert!(
            !skill_mirror_assets
                .iter()
                .any(|a| a.relative_path == manifest_rel_path),
            "synthesis must be gated to full_mirror; skill-mirror should leave \
             manifest-less plugins as-is"
        );
    }

    #[test]
    fn read_plugin_assets_does_not_synthesize_when_real_manifest_present() {
        // GIVEN a plugin that already has a manifest
        let tmp = tempdir().unwrap();
        let plugin = tmp.path().join("plugins/cache/market/plug/1.0.0");
        fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        fs::write(
            plugin.join(".claude-plugin/plugin.json"),
            br#"{"name":"plug","version":"1.0.0","description":"real"}"#,
        )
        .unwrap();
        fs::write(plugin.join("README.md"), b"# plug\n").unwrap();

        // WHEN reading plugin assets in full_mirror mode
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let assets = adapter.read_plugin_assets(true).unwrap();

        // THEN the real manifest is preserved, not overwritten
        let manifest = assets
            .iter()
            .find(|a| a.relative_path == std::path::Path::new(".claude-plugin/plugin.json"))
            .expect("real manifest should be present");
        let json: serde_json::Value =
            serde_json::from_slice(&manifest.content).expect("manifest should parse");
        assert_eq!(
            json["description"], "real",
            "real manifest must not be replaced by a synthetic one"
        );
    }
}
