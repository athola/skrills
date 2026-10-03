//! Codex adapter for reading/writing ~/.codex configuration.
//!
//! ## Agent Support
//!
//! Codex does not have native agent/subagent support (see openai/codex#2604).
//! When syncing agents FROM Claude TO Codex, agents are converted to skills
//! with an "agent-" prefix (e.g., "my-agent" becomes skill "agent-my-agent").
//! This allows agent functionality to be preserved until Codex adds official support.

use super::json_config::{self, Dialect};
use super::traits::{AgentAdapter, FieldSupport};
use super::utils::{
    collect_module_files, hash_content, inside_skill_dir, is_hidden_path, sanitize_name,
    sanitize_name_segments,
};
use crate::common::{Command, ContentFormat, McpServer, McpTransport, Preferences};
use crate::report::WriteReport;
use crate::Result;
use anyhow::Context;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

/// Adapter for Codex CLI configuration.
pub struct CodexAdapter {
    root: PathBuf,
    kill_switch: Option<skrills_snapshot::KillSwitch>,
}

impl CodexAdapter {
    /// Creates a new CodexAdapter with the default root (~/.codex).
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        Ok(Self {
            root: home.join(".codex"),
            kill_switch: None,
        })
    }

    /// Creates a CodexAdapter with a custom root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            kill_switch: None,
        }
    }

    /// Attach a [`KillSwitch`](skrills_snapshot::KillSwitch) so that mutating
    /// operations refuse with [`SyncError::TokenBudgetExceeded`](crate::SyncError)
    /// when the cold-window engine has engaged it (FR12).
    #[must_use]
    pub fn with_kill_switch(mut self, switch: skrills_snapshot::KillSwitch) -> Self {
        self.kill_switch = Some(switch);
        self
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
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };

        match enable_skills_flag(&content) {
            Some(updated) => {
                super::utils::write_config(&path, updated.as_bytes(), false)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Warning attached when MCP servers or the model are written to
/// `config.json`. Current Codex CLI releases read both from `config.toml`
/// (`[mcp_servers.<name>]`, top-level `model`), which this crate cannot yet
/// edit without a TOML dependency, so the write is reported honestly.
fn config_toml_notice(what: &str) -> String {
    format!(
        "Wrote {what} to ~/.codex/config.json, which current Codex releases do not read; \
         copy them into ~/.codex/config.toml ([mcp_servers.<name>] tables, top-level `model`) \
         for Codex to pick them up"
    )
}

/// Returns `content` with `skills = true` under `[features]`, or `None` when
/// it is already set.
///
/// A line scanner rather than a TOML parser, because no format-preserving TOML
/// editor is a dependency of this crate. It matches the key exactly (a
/// `skills_beta` key used to be overwritten), ignores `#` inside quoted strings
/// when finding table headers, and honours a top-level dotted
/// `features.skills` key instead of appending a second `[features]` table,
/// which TOML rejects as a redefinition.
fn enable_skills_flag(content: &str) -> Option<String> {
    /// Text before the first `#` that is not inside a quoted string.
    fn strip_comment(line: &str) -> &str {
        let mut quote: Option<char> = None;
        let mut escaped = false;
        for (i, c) in line.char_indices() {
            match quote {
                Some('"') if escaped => escaped = false,
                Some('"') if c == '\\' => escaped = true,
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == '#' => return &line[..i],
                None => {}
            }
        }
        line
    }

    /// The table a header line opens, or `None` for any other line.
    fn header_name(line: &str) -> Option<String> {
        let trimmed = strip_comment(line).trim();
        if trimmed.starts_with("[[") || !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return None;
        }
        Some(
            trimmed[1..trimmed.len() - 1]
                .split('.')
                .map(|part| part.trim().trim_matches('"'))
                .collect::<Vec<_>>()
                .join("."),
        )
    }

    /// Splits `key = value` into a normalised dotted key and its value.
    fn key_value(line: &str) -> Option<(String, &str)> {
        let (key, value) = strip_comment(line).split_once('=')?;
        let key = key
            .split('.')
            .map(|part| part.trim().trim_matches('"').trim_matches('\''))
            .collect::<Vec<_>>()
            .join(".");
        Some((key, value.trim()))
    }

    let mut out: Vec<String> = Vec::new();
    // `None` before the first header: the root table.
    let mut table: Option<String> = None;
    let mut features_header_at: Option<usize> = None;
    let mut skills_set = false;
    let mut changed = false;

    for line in content.lines() {
        if let Some(name) = header_name(line) {
            if name == "features" {
                features_header_at = Some(out.len());
            }
            table = Some(name);
            out.push(line.to_string());
            continue;
        }

        let target_key = match table.as_deref() {
            None => Some("features.skills"),
            Some("features") => Some("skills"),
            _ => None,
        };
        if let (Some(target_key), Some((key, value))) = (target_key, key_value(line)) {
            if key == target_key {
                skills_set = true;
                if value != "true" {
                    out.push(format!("{target_key} = true"));
                    changed = true;
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }

    if !skills_set {
        match features_header_at {
            Some(at) => out.insert(at + 1, "skills = true".to_string()),
            None => {
                if out.last().is_some_and(|l| !l.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push("[features]".to_string());
                out.push("skills = true".to_string());
            }
        }
        changed = true;
    }

    changed.then(|| out.join("\n") + "\n")
}

// Note: We intentionally do not implement Default for CodexAdapter because
// construction requires home directory resolution which can fail. Use
// CodexAdapter::new() or CodexAdapter::with_root() instead.

/// Hidden file `write_agents` leaves in each skill directory it made from an
/// agent. The `agent-` prefix alone also matched a user's own skill.
const AGENT_MARKER: &str = ".skrills-agent";
const AGENT_MARKER_BODY: &str =
    "This skill was converted from a Claude agent by skrills sync. Delete this file to keep it as a plain skill.\n";

fn is_converted_agent(skill_dir: &Path) -> bool {
    skill_dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("agent-"))
        && skill_dir.join(AGENT_MARKER).is_file()
}

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &str {
        "codex"
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
            hooks: false,         // Codex doesn't support hooks
            agents: false,        // Codex has no agents directory to read
            instructions: false,  // Codex doesn't support instructions
            plugin_assets: false, // Codex doesn't support plugin assets
        }
    }

    /// Asymmetric: `write_agents` converts each agent into an `agent-`-prefixed
    /// skill, so Codex accepts agents as a target even though it has none to
    /// read.
    fn write_support(&self) -> FieldSupport {
        FieldSupport {
            agents: true,
            ..self.read_support()
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
                    modules: Vec::new(),
                    content_format: ContentFormat::default(),
                    plugin_origin: None,
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
                    transport: McpTransport::Stdio, // Codex only supports stdio
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
                    url: None,     // Codex doesn't support HTTP
                    headers: None, // Codex doesn't support HTTP
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
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping unreadable entry under the skills directory");
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

            // Codex skills are discovered via ~/.codex/skills/**/SKILL.md.
            // Keep legacy support for flat *.md in ~/.codex/skills for backwards compatibility,
            // but prefer the SKILL.md convention when present.
            let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
            let is_legacy_md = path.extension().is_some_and(|e| e == "md") && !is_skill_md;
            if !is_skill_md && !is_legacy_md {
                continue;
            }
            // Markdown inside a skill directory is a module of that skill.
            if is_legacy_md && inside_skill_dir(path, &skills_dir) {
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

            // Agents converted by `write_agents` are read by `read_agents()`.
            if is_skill_md && is_converted_agent(path.parent().unwrap_or(path)) {
                continue;
            }

            let read = fs::read(path).and_then(|c| Ok((c, fs::metadata(path)?)));
            let (content, metadata) = match read {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Skipping unreadable skill");
                    continue;
                }
            };
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            // Collect module files for SKILL.md based skills
            let modules = if is_skill_md {
                let skill_dir = path.parent().unwrap_or(path);
                collect_module_files(skill_dir)
            } else {
                Vec::new()
            };

            skills.push(Command {
                name,
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
                modules,
                content_format: ContentFormat::default(),
                plugin_origin: None,
            });
        }
        Ok(skills)
    }

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        super::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        let dir = self.prompts_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
        for cmd in commands {
            writer.write_single(
                &cmd.name,
                &sanitize_name(&cmd.name),
                ".md",
                &cmd.content,
                &mut report,
            )?;
        }

        Ok(report)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        super::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        let path = self.settings_path();
        let mut settings = json_config::load_object(&path)?;
        let mut report =
            json_config::merge_servers(&mut settings, servers, Dialect::StdioOnly, "codex")?;
        if report.written > 0 {
            json_config::write_json_config(&path, &settings)?;
            report.warnings.push(config_toml_notice("MCP servers"));
        }
        Ok(report)
    }

    fn write_preferences(&self, prefs: &Preferences) -> Result<WriteReport> {
        super::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        let mut report = json_config::write_model(&self.settings_path(), prefs.model.as_deref())?;
        if report.written > 0 {
            report.warnings.push(config_toml_notice("the model"));
        }
        Ok(report)
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        super::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        let dir = self.skills_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        let mut writer = super::utils::BatchWriter::new(&dir);
        for skill in skills {
            // Codex discovers SKILL.md files anywhere under ~/.codex/skills/, so
            // a nested name such as `nested/foo` keeps its directory instead of
            // being flattened to `nestedfoo`.
            let safe_rel_dir = sanitize_name_segments(&super::utils::skill_dir_name(skill));
            writer.write(
                &skill.name,
                &safe_rel_dir,
                "SKILL.md",
                &skill.content,
                &skill.modules,
                &mut report,
            )?;
        }

        // Zero skills means nothing for Codex to load, so the user's config is
        // left alone rather than created or edited.
        if !skills.is_empty() {
            let _ = self.ensure_skills_feature_flag_enabled()?;
        }

        Ok(report)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        // Codex does not support hooks
        Ok(Vec::new())
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        // Codex doesn't have native agent support, but we store agents as skills
        // with an "agent-" prefix. Read those back as agents for reverse sync.
        let skills_dir = self.skills_dir();
        if !skills_dir.exists() {
            return Ok(Vec::new());
        }

        let mut agents = Vec::new();
        for entry in WalkDir::new(&skills_dir)
            .min_depth(1)
            .max_depth(20)
            .follow_links(false)
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping unreadable entry under the skills directory");
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

            let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
            if !is_skill_md {
                continue;
            }

            // Get the skill name from the parent directory
            let skill_name = path
                .parent()
                .and_then(|p| p.strip_prefix(&skills_dir).ok())
                .and_then(|p| p.to_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown");

            // Only directories `write_agents` produced, stripping the prefix
            let Some(agent_name) = skill_name.strip_prefix("agent-").map(str::to_owned) else {
                continue;
            };
            if !is_converted_agent(path.parent().unwrap_or(path)) {
                continue;
            }

            let read = fs::read(path).and_then(|c| Ok((c, fs::metadata(path)?)));
            let (content, metadata) = match read {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Skipping unreadable agent");
                    continue;
                }
            };
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            let skill_dir = path.parent().unwrap_or(path);
            let modules = collect_module_files(skill_dir);

            agents.push(Command {
                name: agent_name,
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
                modules,
                content_format: ContentFormat::default(),
                plugin_origin: None,
            });
        }

        Ok(agents)
    }

    fn write_hooks(&self, _hooks: &[Command]) -> Result<WriteReport> {
        super::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        // Codex does not support hooks
        Ok(WriteReport::default())
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        super::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        // Codex does not have native agent support, but we can convert agents to skills.
        // Agents are written as skills with an "agent-" prefix to distinguish them.
        // This allows Claude agents to be used as Codex skills until official support arrives.
        if agents.is_empty() {
            return Ok(WriteReport::default());
        }

        let dir = self.skills_dir();
        fs::create_dir_all(&dir)?;

        let mut report = WriteReport::default();

        let mut writer = super::utils::BatchWriter::new(&dir);
        for agent in agents {
            // Prefix agent names with "agent-" to distinguish from regular skills
            let skill_name = format!("agent-{}", agent.name);
            let safe_name = sanitize_name(&skill_name);
            writer.write(
                &skill_name,
                &safe_name,
                "SKILL.md",
                &agent.content,
                &agent.modules,
                &mut report,
            )?;
            // Mark the directory only when the agent is actually there (not
            // refused), so `read_agents` never claims a user's own skill.
            let agent_dir = dir.join(&safe_name);
            let marker = agent_dir.join(AGENT_MARKER);
            if !safe_name.is_empty()
                && agent_dir.join("SKILL.md").is_file()
                && super::utils::symlink_below(&dir, &marker).is_none()
                && !marker.exists()
            {
                super::utils::write_file(&marker, AGENT_MARKER_BODY)?;
            }
        }

        let _ = self.ensure_skills_feature_flag_enabled()?;

        Ok(report)
    }

    fn read_instructions(&self) -> Result<Vec<Command>> {
        // Codex does not support instructions
        Ok(Vec::new())
    }

    fn write_instructions(&self, _instructions: &[Command]) -> Result<WriteReport> {
        super::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        // Codex does not support instructions
        Ok(WriteReport::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ModuleFile;
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
        crate::adapters::tests_common::assert_read_commands_empty(&adapter);
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
            modules: Vec::new(),

            content_format: ContentFormat::default(),
            plugin_origin: None,
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
            modules: Vec::new(),

            content_format: ContentFormat::default(),
            plugin_origin: None,
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

        let config_path = tmp.path().join("config.json");
        assert!(config_path.exists());

        let content = fs::read_to_string(&config_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(settings["mcpServers"]["my-server"].is_object());
    }

    /// Current Codex reads MCP servers and the model from config.toml, not
    /// config.json, so a write there must not pass silently as synced.
    #[test]
    fn writes_to_config_json_warn_that_codex_reads_config_toml() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let mut servers = HashMap::new();
        servers.insert(
            "s".to_string(),
            McpServer {
                name: "s".to_string(),
                transport: McpTransport::Stdio,
                command: "/bin/s".to_string(),
                args: vec![],
                env: HashMap::new(),
                url: None,
                headers: None,
                enabled: true,
                allowed_tools: vec![],
                disabled_tools: vec![],
            },
        );

        let mcp = adapter.write_mcp_servers(&servers).unwrap();
        assert_eq!(mcp.written, 1);
        assert!(
            mcp.warnings.iter().any(|w| w.contains("config.toml")),
            "{:?}",
            mcp.warnings
        );

        let prefs = adapter
            .write_preferences(&Preferences {
                model: Some("gpt-4o".to_string()),
                custom: HashMap::new(),
            })
            .unwrap();
        assert_eq!(prefs.written, 1);
        assert!(
            prefs.warnings.iter().any(|w| w.contains("config.toml")),
            "{:?}",
            prefs.warnings
        );

        // Nothing written, nothing to warn about.
        let again = adapter.write_mcp_servers(&servers).unwrap();
        assert_eq!(again.written, 0);
        assert!(again.warnings.is_empty());
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
            modules: Vec::new(),

            content_format: ContentFormat::default(),
            plugin_origin: None,
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
            modules: Vec::new(),

            content_format: ContentFormat::default(),
            plugin_origin: None,
        };

        adapter.write_skills(&[skill]).unwrap();

        let cfg = fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(cfg.contains("[features]"));
        assert!(cfg.contains("skills = true"));
    }

    /// A key that merely starts with `skills` used to be replaced, losing both
    /// its name and its value.
    #[test]
    fn skills_flag_leaves_a_similarly_named_key_alone() {
        let updated = enable_skills_flag("[features]\nskills_beta = false\n").unwrap();
        assert_eq!(updated, "[features]\nskills = true\nskills_beta = false\n");
    }

    /// A second `[features]` table is a TOML redefinition error.
    #[test]
    fn skills_flag_honours_a_top_level_dotted_key() {
        assert_eq!(enable_skills_flag("features.skills = true\n"), None);
        assert_eq!(
            enable_skills_flag("features.skills = false\nmodel = \"o3\"\n").unwrap(),
            "features.skills = true\nmodel = \"o3\"\n"
        );
    }

    /// `#` inside a quoted key was read as a comment, so the header was not
    /// recognised and the lines under it were treated as `[features]` keys.
    #[test]
    fn skills_flag_ignores_hash_inside_a_quoted_header() {
        let content = "[features]\nskills = true\n[servers.\"a#b\"]\nskills = false\n";
        assert_eq!(enable_skills_flag(content), None);
    }

    #[test]
    fn skills_flag_is_a_no_op_when_already_true() {
        assert_eq!(enable_skills_flag("[features]\nskills = true # on\n"), None);
    }

    #[test]
    fn skills_flag_appends_a_features_table_when_missing() {
        assert_eq!(
            enable_skills_flag("model = \"o3\"\n").unwrap(),
            "model = \"o3\"\n\n[features]\nskills = true\n"
        );
    }

    /// With no skills there is nothing for Codex to load, so the user's
    /// config is not created or edited.
    #[test]
    fn write_skills_with_no_skills_leaves_config_toml_alone() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        adapter.write_skills(&[]).unwrap();
        assert!(!tmp.path().join("config.toml").exists());
    }

    /// A markdown file inside a skill directory is a module of that skill;
    /// the reader also returned it as a second, legacy skill.
    #[test]
    fn read_skills_does_not_turn_a_skill_module_into_a_skill() {
        let tmp = tempdir().unwrap();
        let skills = tmp.path().join("skills");
        fs::create_dir_all(skills.join("foo/docs")).unwrap();
        fs::write(skills.join("foo/SKILL.md"), "foo").unwrap();
        fs::write(skills.join("foo/docs/reference.md"), "ref").unwrap();
        fs::write(skills.join("legacy.md"), "legacy").unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let mut names: Vec<_> = adapter
            .read_skills()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        names.sort();

        assert_eq!(names, vec!["foo", "legacy"]);
    }

    /// A user's own skill named `agent-smith` was hidden from `read_skills`
    /// and exported as an agent called `smith`.
    #[test]
    fn a_user_skill_with_the_agent_prefix_stays_a_skill() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("skills/agent-smith");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "mine").unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let skills: Vec<_> = adapter
            .read_skills()
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(skills, vec!["agent-smith"]);
        assert!(adapter.read_agents().unwrap().is_empty());
    }

    #[test]
    fn agents_written_as_skills_read_back_as_agents_only() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let agent = Command {
            name: "reviewer".to_string(),
            content: b"---\nname: reviewer\n---\nbody".to_vec(),
            source_path: PathBuf::from("/a/reviewer.md"),
            modified: SystemTime::now(),
            hash: String::new(),
            modules: Vec::new(),
            content_format: ContentFormat::default(),
            plugin_origin: None,
        };
        adapter.write_agents(&[agent]).unwrap();

        let agents: Vec<_> = adapter
            .read_agents()
            .unwrap()
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(agents, vec!["reviewer"]);
        assert!(adapter.read_skills().unwrap().is_empty());
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
    fn read_skills_collects_module_files() {
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills/my-skill");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: test\n---\n",
        )
        .unwrap();
        fs::write(skills_dir.join("helper.py"), "# Python helper").unwrap();
        fs::write(skills_dir.join("config.json"), r#"{"key": "value"}"#).unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        assert_eq!(skills[0].modules.len(), 2);

        let module_paths: std::collections::HashSet<_> = skills[0]
            .modules
            .iter()
            .map(|m| m.relative_path.to_string_lossy().to_string())
            .collect();
        assert!(module_paths.contains("helper.py"));
        assert!(module_paths.contains("config.json"));
    }

    #[test]
    fn write_skills_writes_module_files() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let skill = Command {
            name: "test-skill".to_string(),
            content: b"---\nname: test-skill\ndescription: test\n---\n".to_vec(),
            source_path: PathBuf::from("/tmp/test-skill/SKILL.md"),
            modified: SystemTime::now(),
            hash: "hash".to_string(),
            modules: vec![
                ModuleFile {
                    relative_path: PathBuf::from("helper.py"),
                    content: b"# Helper".to_vec(),
                    hash: "h1".to_string(),
                },
                ModuleFile {
                    relative_path: PathBuf::from("nested/data.json"),
                    content: b"{}".to_vec(),
                    hash: "h2".to_string(),
                },
            ],

            content_format: ContentFormat::default(),
            plugin_origin: None,
        };

        let report = adapter.write_skills(&[skill]).unwrap();
        assert_eq!(report.written, 1);

        // Verify SKILL.md was written
        assert!(tmp.path().join("skills/test-skill/SKILL.md").exists());

        // Verify module files were written
        let helper = tmp.path().join("skills/test-skill/helper.py");
        assert!(helper.exists());
        assert_eq!(fs::read_to_string(&helper).unwrap(), "# Helper");

        let nested = tmp.path().join("skills/test-skill/nested/data.json");
        assert!(nested.exists());
        assert_eq!(fs::read_to_string(&nested).unwrap(), "{}");
    }

    /// `write_agents` is a real implementation, so the target-side declaration
    /// has to say so even though Codex has no agents directory to read.
    #[test]
    fn write_support_declares_agents_although_read_support_does_not() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        assert!(
            !adapter.read_support().agents,
            "Codex has no agents directory to read"
        );
        assert!(
            adapter.write_support().agents,
            "write_agents converts agents into agent-prefixed skills"
        );
    }

    #[test]
    fn write_agents_converts_to_skills_with_prefix() {
        // Codex doesn't have native agent support, so agents are converted to skills
        // with an "agent-" prefix
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let agents = vec![
            Command {
                name: "code-reviewer".to_string(),
                content: b"---\nname: code-reviewer\ndescription: Reviews code\n---\n# Agent"
                    .to_vec(),
                source_path: PathBuf::from("/tmp/code-reviewer.md"),
                modified: SystemTime::now(),
                hash: "hash1".to_string(),
                modules: Vec::new(),
                content_format: ContentFormat::default(),
                plugin_origin: None,
            },
            Command {
                name: "test-writer".to_string(),
                content: b"---\nname: test-writer\ndescription: Writes tests\n---\n# Agent"
                    .to_vec(),
                source_path: PathBuf::from("/tmp/test-writer.md"),
                modified: SystemTime::now(),
                hash: "hash2".to_string(),
                modules: Vec::new(),
                content_format: ContentFormat::default(),
                plugin_origin: None,
            },
        ];

        let report = adapter.write_agents(&agents).unwrap();
        assert_eq!(report.written, 2);

        // Verify agents were written as skills with "agent-" prefix
        assert!(tmp
            .path()
            .join("skills/agent-code-reviewer/SKILL.md")
            .exists());
        assert!(tmp
            .path()
            .join("skills/agent-test-writer/SKILL.md")
            .exists());

        // Verify content
        let content =
            fs::read_to_string(tmp.path().join("skills/agent-code-reviewer/SKILL.md")).unwrap();
        assert!(content.contains("code-reviewer"));
    }

    #[test]
    fn write_agents_empty_returns_empty_report() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let report = adapter.write_agents(&[]).unwrap();
        assert_eq!(report.written, 0);
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn read_agents_returns_agent_prefixed_skills() {
        // For reverse sync: skills with "agent-" prefix should be returned as agents
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");

        // An agent converted by `write_agents`, which leaves its marker
        let agent_skill_dir = skills_dir.join("agent-code-reviewer");
        fs::create_dir_all(&agent_skill_dir).unwrap();
        fs::write(
            agent_skill_dir.join("SKILL.md"),
            "---\nname: code-reviewer\ndescription: Reviews code\n---\n# Agent",
        )
        .unwrap();
        fs::write(agent_skill_dir.join(AGENT_MARKER), AGENT_MARKER_BODY).unwrap();

        // Create a regular skill (should NOT be returned as agent)
        let regular_skill_dir = skills_dir.join("pdf-processing");
        fs::create_dir_all(&regular_skill_dir).unwrap();
        fs::write(
            regular_skill_dir.join("SKILL.md"),
            "---\nname: pdf-processing\ndescription: Process PDFs\n---\n# Skill",
        )
        .unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let agents = adapter.read_agents().unwrap();

        // Only agent-prefixed skill should be returned, with prefix stripped
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "code-reviewer");
    }

    #[test]
    fn read_skills_excludes_agent_prefixed() {
        // Skills with "agent-" prefix should NOT be returned by read_skills
        let tmp = tempdir().unwrap();
        let skills_dir = tmp.path().join("skills");

        // An agent converted by `write_agents`, which leaves its marker
        let agent_skill_dir = skills_dir.join("agent-code-reviewer");
        fs::create_dir_all(&agent_skill_dir).unwrap();
        fs::write(
            agent_skill_dir.join("SKILL.md"),
            "---\nname: code-reviewer\n---\n# Agent",
        )
        .unwrap();
        fs::write(agent_skill_dir.join(AGENT_MARKER), AGENT_MARKER_BODY).unwrap();

        // Create a regular skill
        let regular_skill_dir = skills_dir.join("pdf-processing");
        fs::create_dir_all(&regular_skill_dir).unwrap();
        fs::write(
            regular_skill_dir.join("SKILL.md"),
            "---\nname: pdf-processing\n---\n# Skill",
        )
        .unwrap();

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let skills = adapter.read_skills().unwrap();

        // Only regular skill should be returned (agent-prefixed excluded)
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "pdf-processing");
    }

    #[test]
    fn agent_roundtrip_preserves_content() {
        // Write agents, then read them back - content should be preserved
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let original_content =
            b"---\nname: my-agent\ndescription: Test\n---\n# My Agent\n\nInstructions here.";
        let agents = vec![Command {
            name: "my-agent".to_string(),
            content: original_content.to_vec(),
            source_path: PathBuf::from("/tmp/my-agent.md"),
            modified: SystemTime::now(),
            hash: "hash".to_string(),
            modules: Vec::new(),

            content_format: ContentFormat::default(),
            plugin_origin: None,
        }];

        adapter.write_agents(&agents).unwrap();

        let read_back = adapter.read_agents().unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].name, "my-agent");
        assert_eq!(read_back[0].content, original_content.to_vec());
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

    #[test]
    fn read_mcp_servers_with_tool_configs() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join("config.json");
        fs::write(
            &config_path,
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

        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let servers = adapter.read_mcp_servers().unwrap();

        let server = servers.get("restricted-server").unwrap();
        assert_eq!(server.allowed_tools, vec!["read_file", "search_*"]);
        assert_eq!(server.disabled_tools, vec!["delete_file", "write_file"]);
    }

    #[test]
    fn write_mcp_servers_preserves_tool_configs() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

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
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

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

        let content = fs::read_to_string(tmp.path().join("config.json")).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        let server_json = &settings["mcpServers"]["clean-server"];
        assert!(server_json.get("allowedTools").is_none());
        assert!(server_json.get("disabledTools").is_none());
    }
}
