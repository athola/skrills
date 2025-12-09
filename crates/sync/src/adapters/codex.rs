//! Codex adapter for reading/writing ~/.codex configuration.

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

/// Adapter for Codex CLI configuration.
pub struct CodexAdapter {
    root: PathBuf,
}

impl CodexAdapter {
    /// Creates a new CodexAdapter with the default root (~/.codex).
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(Self {
            root: home.join(".codex"),
        })
    }

    /// Creates a CodexAdapter with a custom root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    fn prompts_dir(&self) -> PathBuf {
        self.root.join("prompts")
    }

    fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    fn settings_path(&self) -> PathBuf {
        // Codex uses config.json, not settings.json
        self.root.join("config.json")
    }

    fn hash_content(content: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content);
        format!("{:x}", hasher.finalize())
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new().expect("Failed to create CodexAdapter")
    }
}

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &str {
        "codex"
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
        let active_dir = self.prompts_dir();
        if !active_dir.exists() {
            return Ok(Vec::new());
        }

        let mut commands = Vec::new();
        for entry in WalkDir::new(&active_dir).min_depth(1).max_depth(2) {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|e| e == "md") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let content = fs::read(path)?;
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
        // Codex uses "mcpServers" same as Claude
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
            custom: HashMap::new(),
        })
    }

    fn read_skills(&self) -> Result<Vec<Command>> {
        let skills_dir = self.skills_dir();
        if !skills_dir.exists() {
            return Ok(Vec::new());
        }

        let mut skills = Vec::new();
        for entry in WalkDir::new(&skills_dir).min_depth(1).max_depth(2) {
            let entry = entry?;
            let path = entry.path();

            if path.extension().is_some_and(|e| e == "md") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let content = fs::read(path)?;
                let metadata = fs::metadata(path)?;
                let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let hash = Self::hash_content(&content);

                skills.push(Command {
                    name,
                    content,
                    source_path: path.to_path_buf(),
                    modified,
                    hash,
                });
            }
        }
        Ok(skills)
    }

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        let dir = self.prompts_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        for cmd in commands {
            let path = dir.join(format!("{}.md", cmd.name));

            if path.exists() {
                let existing = fs::read(&path)?;
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

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        let dir = self.skills_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        for skill in skills {
            let path = dir.join(format!("{}.md", skill.name));

            if path.exists() {
                let existing = fs::read(&path)?;
                if Self::hash_content(&existing) == skill.hash {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn codex_adapter_name() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        assert_eq!(adapter.name(), "codex");
    }

    #[test]
    fn read_commands_empty_dir() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands().unwrap();
        assert!(commands.is_empty());
    }

    #[test]
    fn read_commands_finds_md_files() {
        let tmp = tempdir().unwrap();
        let cmd_dir = tmp.path().join("prompts");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(cmd_dir.join("test.md"), "# Test Command").unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands().unwrap();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "test");
        assert_eq!(commands[0].content, b"# Test Command".to_vec());
    }

    #[test]
    fn write_commands_creates_files() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let commands = vec![Command {
            name: "hello".to_string(),
            content: b"# Hello World".to_vec(),
            source_path: PathBuf::from("/tmp/hello.md"),
            modified: SystemTime::now(),
            hash: "abc123".to_string(),
        }];

        let report = adapter.write_commands(&commands).unwrap();
        assert_eq!(report.written, 1);

        let written = fs::read(tmp.path().join("prompts/hello.md")).unwrap();
        assert_eq!(written, b"# Hello World");
    }

    #[test]
    fn read_write_roundtrip() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let commands = vec![Command {
            name: "test-cmd".to_string(),
            content: b"# Test".to_vec(),
            source_path: PathBuf::from("/tmp/test.md"),
            modified: SystemTime::now(),
            hash: "hash123".to_string(),
        }];

        adapter.write_commands(&commands).unwrap();
        let read_back = adapter.read_commands().unwrap();

        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].name, "test-cmd");
        assert_eq!(read_back[0].content, b"# Test".to_vec());
    }

    #[test]
    fn read_mcp_servers_from_config() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(
            &config_path,
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

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        assert_eq!(servers.len(), 1);
        let server = servers.get("test-server").unwrap();
        assert_eq!(server.command, "/usr/bin/test");
        assert_eq!(server.args, vec!["--flag", "value"]);
        assert!(server.enabled);
    }

    #[test]
    fn write_mcp_servers_creates_config() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

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

        let config_path = tmp.path().join("config.json");
        assert!(config_path.exists());

        let content = fs::read_to_string(&config_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(settings["mcpServers"]["my-server"].is_object());
    }

    #[test]
    fn read_preferences_from_config() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
            "model": "gpt-4o"
        }"#,
        )
        .unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let prefs = adapter.read_preferences().unwrap();

        assert_eq!(prefs.model.as_deref(), Some("gpt-4o"));
    }
}
