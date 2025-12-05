//! Claude Code adapter for reading/writing ~/.claude configuration.

use super::traits::{AgentAdapter, FieldSupport};
use crate::common::{Command, McpServer, Preferences};
use crate::report::{SkipReason, WriteReport};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
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

    fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    fn hash_content(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self::new().expect("Failed to create ClaudeAdapter")
    }
}

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
        }
    }

    fn read_commands(&self) -> Result<Vec<Command>> {
        let dir = self.commands_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut commands = Vec::new();
        for entry in WalkDir::new(&dir).min_depth(1).max_depth(2) {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|e| e == "md") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let content = fs::read_to_string(path)?;
                let metadata = fs::metadata(path)?;
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let hash = Self::hash_content(&content);

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
                let server = McpServer {
                    name: name.clone(),
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
                    enabled: config
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

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        let dir = self.commands_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        for cmd in commands {
            let path = dir.join(format!("{}.md", cmd.name));

            // Check if unchanged
            if path.exists() {
                let existing = fs::read_to_string(&path)?;
                if Self::hash_content(&existing) == cmd.hash {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_commands_empty_dir() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands().unwrap();
        assert!(commands.is_empty());
    }

    #[test]
    fn read_commands_finds_md_files() {
        let tmp = tempdir().unwrap();
        let cmd_dir = tmp.path().join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(cmd_dir.join("test.md"), "# Test Command").unwrap();

        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands().unwrap();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "test");
        assert_eq!(commands[0].content, "# Test Command");
    }

    #[test]
    fn write_commands_creates_files() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let commands = vec![Command {
            name: "hello".to_string(),
            content: "# Hello World".to_string(),
            source_path: PathBuf::from("/tmp/hello.md"),
            modified: SystemTime::now(),
            hash: "abc123".to_string(),
        }];

        let report = adapter.write_commands(&commands).unwrap();
        assert_eq!(report.written, 1);

        let written = fs::read_to_string(tmp.path().join("commands/hello.md")).unwrap();
        assert_eq!(written, "# Hello World");
    }

    #[test]
    fn write_commands_skips_unchanged() {
        let tmp = tempdir().unwrap();
        let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf());

        let content = "# Unchanged Command";
        let hash = ClaudeAdapter::hash_content(content);

        // Write initial
        let commands = vec![Command {
            name: "unchanged".to_string(),
            content: content.to_string(),
            source_path: PathBuf::from("/tmp/unchanged.md"),
            modified: SystemTime::now(),
            hash: hash.clone(),
        }];
        adapter.write_commands(&commands).unwrap();

        // Write again with same hash
        let commands2 = vec![Command {
            name: "unchanged".to_string(),
            content: content.to_string(),
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
                command: "/bin/server".to_string(),
                args: vec!["arg1".to_string()],
                env: HashMap::new(),
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
}
