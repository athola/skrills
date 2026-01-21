//! Codex adapter for reading/writing ~/.codex configuration.

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

    fn config_toml_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// Ensure Codex's experimental skills feature flag is enabled in `config.toml`.
    ///
    /// Codex loads skills only when `[features] skills = true` is set.
    fn ensure_skills_feature_flag_enabled(&self) -> Result<bool> {
        let path = self.config_toml_path();
        let content = if path.exists() {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };

        fn strip_comment(s: &str) -> &str {
            s.split_once('#').map(|(a, _)| a).unwrap_or(s)
        }

        fn is_header_line(line: &str) -> bool {
            let trimmed = strip_comment(line).trim();
            trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.starts_with("[[")
        }

        fn header_name(line: &str) -> &str {
            let trimmed = strip_comment(line).trim();
            &trimmed[1..trimmed.len().saturating_sub(1)]
        }

        let mut out: Vec<String> = Vec::new();
        let mut in_features = false;
        let mut found_features = false;
        let mut skills_set = false;
        let mut changed = false;

        for line in content.lines() {
            if is_header_line(line) {
                if in_features && !skills_set {
                    out.push("skills = true".to_string());
                    skills_set = true;
                    changed = true;
                }

                let name = header_name(line);
                in_features = name == "features";
                if in_features {
                    found_features = true;
                }

                out.push(line.to_string());
                continue;
            }

            if in_features {
                let trimmed = strip_comment(line).trim_start();
                if trimmed.starts_with("skills") && trimmed.contains('=') {
                    // Overwrite the value unconditionally to avoid false/invalid values.
                    if strip_comment(trimmed)
                        .split_once('=')
                        .map(|(_, v)| v.trim())
                        != Some("true")
                    {
                        out.push("skills = true".to_string());
                        changed = true;
                    } else {
                        out.push(line.to_string());
                    }
                    skills_set = true;
                    continue;
                }
            }

            out.push(line.to_string());
        }

        if in_features && !skills_set {
            out.push("skills = true".to_string());
            changed = true;
        }

        if !found_features {
            if !out.is_empty() && !out.last().unwrap().trim().is_empty() {
                out.push(String::new());
            }
            out.push("[features]".to_string());
            out.push("skills = true".to_string());
            changed = true;
        }

        if changed {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, out.join("\n") + "\n")?;
        }

        Ok(changed)
    }
}

// Note: We intentionally do not implement Default for CodexAdapter because
// construction requires home directory resolution which can fail. Use
// CodexAdapter::new() or CodexAdapter::with_root() instead.

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
            hooks: false,  // Codex doesn't support hooks
            agents: false, // Codex doesn't support agents
        }
    }

    fn read_commands(&self, _include_marketplace: bool) -> Result<Vec<Command>> {
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
            if !entry.file_type().is_file() {
                continue;
            }

            // Codex skills are discovered via ~/.codex/skills/**/SKILL.md.
            // Keep legacy support for flat *.md in ~/.codex/skills for backwards compatibility,
            // but prefer the SKILL.md convention when present.
            let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
            let is_legacy_md = path.extension().is_some_and(|e| e == "md") && !is_skill_md;
            if !is_skill_md && !is_legacy_md {
                continue;
            }

            let name = if is_skill_md {
                // Use the parent directory path relative to skills_dir as the skill identifier.
                // Example: ~/.codex/skills/pdf-processing/SKILL.md -> "pdf-processing"
                // Example: ~/.codex/skills/nested/foo/SKILL.md -> "nested/foo"
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

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        let dir = self.prompts_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        for cmd in commands {
            let safe_name = sanitize_name(&cmd.name);
            let path = dir.join(format!("{}.md", safe_name));

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
            // Codex discovers only SKILL.md files under ~/.codex/skills/**/.
            // Write each skill into ~/.codex/skills/<skill-name>/SKILL.md by default.
            let skill_rel_dir = if skill.name.eq_ignore_ascii_case("skill")
                || skill.name.eq_ignore_ascii_case("skill.md")
                || skill
                    .name
                    .eq_ignore_ascii_case("skill.md".trim_end_matches(".md"))
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

        let _ = self.ensure_skills_feature_flag_enabled()?;

        Ok(report)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        // Codex does not support hooks
        Ok(Vec::new())
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        // Codex does not support agents
        Ok(Vec::new())
    }

    fn write_hooks(&self, _hooks: &[Command]) -> Result<WriteReport> {
        // Codex does not support hooks
        Ok(WriteReport::default())
    }

    fn write_agents(&self, _agents: &[Command]) -> Result<WriteReport> {
        // Codex does not support agents
        Ok(WriteReport::default())
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
    fn codex_adapter_name() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        assert_eq!(adapter.name(), "codex");
    }

    #[test]
    fn read_commands_empty_dir() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands(false).unwrap();
        assert!(commands.is_empty());
    }

    #[test]
    fn read_commands_finds_md_files() {
        let tmp = tempdir().unwrap();
        let cmd_dir = tmp.path().join("prompts");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(cmd_dir.join("test.md"), "# Test Command").unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let commands = adapter.read_commands(false).unwrap();

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
        let read_back = adapter.read_commands(false).unwrap();

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

    #[test]
    fn write_skills_writes_skill_md_in_directory() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

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
    fn write_skills_enables_codex_skills_feature_flag_in_config_toml() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let skill = Command {
            name: "alpha".to_string(),
            content: b"---\nname: alpha\ndescription: test\n---\n# Alpha\n".to_vec(),
            source_path: PathBuf::from("/tmp/alpha.md"),
            modified: SystemTime::now(),
            hash: "hash".to_string(),
        };

        adapter.write_skills(&[skill]).unwrap();

        let cfg = fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(cfg.contains("[features]"));
        assert!(cfg.contains("skills = true"));
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

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();
        let names: std::collections::HashSet<_> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains("nested/foo"));
    }

    #[test]
    fn read_mcp_servers_invalid_json_returns_error() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(&config_path, "{ invalid json }").unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let result = adapter.read_mcp_servers();
        assert!(result.is_err());
    }

    #[test]
    fn read_preferences_invalid_json_returns_error() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(&config_path, "not valid json at all").unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let result = adapter.read_preferences();
        assert!(result.is_err());
    }

    #[test]
    fn write_mcp_servers_invalid_existing_json_returns_error() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(&config_path, "{ corrupted json }").unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let mut servers = HashMap::new();
        servers.insert(
            "test-server".to_string(),
            McpServer {
                name: "test-server".to_string(),
                command: "/bin/test".to_string(),
                args: vec![],
                env: HashMap::new(),
                enabled: true,
            },
        );

        let result = adapter.write_mcp_servers(&servers);
        assert!(result.is_err());
    }

    #[test]
    fn write_preferences_invalid_existing_json_returns_error() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(&config_path, "{ malformed: json, }").unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let prefs = Preferences {
            model: Some("gpt-4o".to_string()),
            custom: HashMap::new(),
        };

        let result = adapter.write_preferences(&prefs);
        assert!(result.is_err());
    }
}
