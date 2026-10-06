//! Codex adapter for reading/writing ~/.codex configuration.
//!
//! ## Agent Support
//!
//! Codex does not have native agent/subagent support (see openai/codex#2604).
//! When syncing agents FROM Claude TO Codex, agents are converted to skills
//! with an "agent-" prefix (e.g., "my-agent" becomes skill "agent-my-agent").
//! This allows agent functionality to be preserved until Codex adds official support.

use super::codex_toml;
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

    /// Legacy `config.json`, read only as a fallback for MCP servers and the
    /// model when `config.toml` has none.
    fn legacy_json_path(&self) -> PathBuf {
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

/// Returns `content` with `skills = true` under `[features]`, or `None` when
/// it is already set.
///
/// A line scanner that predates the `toml_edit` dependency (MCP servers and
/// the model go through [`codex_toml`]). It matches the key exactly (a
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
        let servers = codex_toml::read_servers(&codex_toml::load(&self.config_toml_path())?)?;
        if !servers.is_empty() {
            return Ok(servers);
        }
        let path = self.legacy_json_path();
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
                    transport: McpTransport::Stdio, // the legacy config.json held stdio servers only
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
                    url: None,
                    headers: None,
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
        if let Some(model) = codex_toml::read_model(&codex_toml::load(&self.config_toml_path())?) {
            return Ok(Preferences {
                model: Some(model),
                custom: HashMap::new(),
            });
        }
        let path = self.legacy_json_path();
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
        // Nothing to merge, so the user's config.toml is not read at all and
        // an unreadable one cannot fail an otherwise empty phase.
        if servers.is_empty() {
            return Ok(WriteReport::default());
        }
        let path = self.config_toml_path();
        let mut doc = codex_toml::load(&path)?;
        let report = codex_toml::merge_servers(&mut doc, servers)?;
        if report.written > 0 {
            codex_toml::save(&path, &doc)?;
        }
        Ok(report)
    }

    fn write_preferences(&self, prefs: &Preferences) -> Result<WriteReport> {
        super::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        let mut report = WriteReport::default();
        let Some(model) = prefs.model.as_deref() else {
            return Ok(report);
        };
        let path = self.config_toml_path();
        let mut doc = codex_toml::load(&path)?;
        if codex_toml::set_model(&mut doc, model) {
            codex_toml::save(&path, &doc)?;
            report.written += 1;
        } else {
            report.skipped.push(crate::report::SkipReason::Unchanged {
                item: "model".to_string(),
            });
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
    fn read_mcp_servers_falls_back_to_config_json() {
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

    fn stdio_server(name: &str, command: &str) -> McpServer {
        McpServer {
            name: name.to_string(),
            transport: McpTransport::Stdio,
            command: command.to_string(),
            args: vec![],
            env: HashMap::new(),
            url: None,
            headers: None,
            enabled: true,
            allowed_tools: vec![],
            disabled_tools: vec![],
        }
    }

    fn one_server(server: McpServer) -> HashMap<String, McpServer> {
        HashMap::from([(server.name.clone(), server)])
    }

    /// Strict re-parse of the written config.toml.
    fn parsed_toml(root: &Path) -> toml_edit::DocumentMut {
        fs::read_to_string(root.join("config.toml"))
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .expect("config.toml must stay valid TOML")
    }

    /// Codex reads MCP servers from `[mcp_servers.<name>]` in config.toml
    /// (SY-26); they used to go to config.json, which Codex never reads.
    #[test]
    fn write_mcp_servers_with_nothing_to_write_leaves_the_config_unread() {
        let tmp = tempdir().unwrap();
        // A directory where config.toml belongs fails every read of it.
        std::fs::create_dir_all(tmp.path().join("config.toml")).unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let report = adapter.write_mcp_servers(&HashMap::new()).unwrap();

        assert_eq!(report.written, 0);
    }

    #[test]
    fn write_mcp_servers_creates_config_toml_tables() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let mut server = stdio_server("my-server", "/bin/server");
        server.args = vec!["arg1".to_string()];
        server.env = HashMap::from([("TOKEN".to_string(), "x".to_string())]);

        let report = adapter.write_mcp_servers(&one_server(server)).unwrap();
        assert_eq!(report.written, 1);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(!tmp.path().join("config.json").exists());

        let text = fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(text.contains("[mcp_servers.my-server]"), "{text}");
        assert!(
            !text.contains("[mcp_servers]\n"),
            "no empty parent header: {text}"
        );
        let doc = parsed_toml(tmp.path());
        let entry = &doc["mcp_servers"]["my-server"];
        assert_eq!(entry["type"].as_str(), Some("stdio"));
        assert_eq!(entry["command"].as_str(), Some("/bin/server"));
        assert_eq!(entry["args"][0].as_str(), Some("arg1"));
        assert_eq!(entry["env"]["TOKEN"].as_str(), Some("x"));

        let read_back = adapter.read_mcp_servers().unwrap();
        assert_eq!(read_back["my-server"].args, vec!["arg1"]);
        assert_eq!(read_back["my-server"].env["TOKEN"], "x");

        let again = adapter.write_mcp_servers(&one_server(read_back["my-server"].clone()));
        assert_eq!(
            again.unwrap().written,
            0,
            "an identical sync writes nothing"
        );
    }

    #[test]
    fn write_mcp_servers_keeps_comments_order_and_other_tables() {
        let tmp = tempdir().unwrap();
        let original = "\
# my codex config
model = \"gpt-5\" # pinned

[features]
skills = true # keep me

# github server
[mcp_servers.github]
command = \"npx\"
startup_timeout_sec = 180
";
        fs::write(tmp.path().join("config.toml"), original).unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        // Re-writing the existing server unchanged is a byte-for-byte no-op.
        let mut github = stdio_server("github", "npx");
        let report = adapter
            .write_mcp_servers(&one_server(github.clone()))
            .unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(
            fs::read_to_string(tmp.path().join("config.toml")).unwrap(),
            original
        );

        github.args = vec!["-y".to_string()];
        let report = adapter.write_mcp_servers(&one_server(github)).unwrap();
        assert_eq!(report.written, 1);

        let text = fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        for kept in [
            "# my codex config",
            "model = \"gpt-5\" # pinned",
            "skills = true # keep me",
            "# github server",
            "startup_timeout_sec = 180",
        ] {
            assert!(text.contains(kept), "lost {kept:?}:\n{text}");
        }
        assert!(
            text.find("[features]") < text.find("[mcp_servers.github]"),
            "{text}"
        );
        assert_eq!(
            parsed_toml(tmp.path())["mcp_servers"]["github"]["args"][0].as_str(),
            Some("-y")
        );
        assert_eq!(
            fs::read_to_string(tmp.path().join("config.toml.skrills-bak")).unwrap(),
            original
        );
    }

    #[test]
    fn write_mcp_servers_merges_with_an_unmanaged_server() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            "[mcp_servers.mine]\ncommand = \"/bin/mine\"\ncwd = \"/srv\"\n",
        )
        .unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let report = adapter
            .write_mcp_servers(&one_server(stdio_server("synced", "/bin/synced")))
            .unwrap();
        assert_eq!(report.written, 1);

        let doc = parsed_toml(tmp.path());
        assert_eq!(
            doc["mcp_servers"]["mine"]["command"].as_str(),
            Some("/bin/mine")
        );
        assert_eq!(doc["mcp_servers"]["mine"]["cwd"].as_str(), Some("/srv"));
        assert_eq!(
            doc["mcp_servers"]["synced"]["command"].as_str(),
            Some("/bin/synced")
        );

        // An empty source touches nothing.
        let before = fs::read(tmp.path().join("config.toml")).unwrap();
        adapter.write_mcp_servers(&HashMap::new()).unwrap();
        assert_eq!(fs::read(tmp.path().join("config.toml")).unwrap(), before);
    }

    /// A server spelled with dotted keys or inside an inline table must be
    /// updated where it is: appending `[mcp_servers.x]` would redefine it,
    /// and Codex would then refuse the whole file.
    #[test]
    fn write_mcp_servers_updates_dotted_and_inline_entries_in_place() {
        let cases = [
            "mcp_servers.dotted.command = \"/old\"\n",
            "[mcp_servers]\ndotted.command = \"/old\"\n",
            "mcp_servers = { dotted = { command = \"/old\" } }\n",
            "[mcp_servers]\ndotted = { command = \"/old\", cwd = \"/srv\" }\n",
        ];
        for original in cases {
            let tmp = tempdir().unwrap();
            fs::write(tmp.path().join("config.toml"), original).unwrap();
            let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
            let mut servers = one_server(stdio_server("dotted", "/new"));
            servers.insert("added".to_string(), stdio_server("added", "/added"));

            let report = adapter.write_mcp_servers(&servers).unwrap();
            assert_eq!(report.written, 2, "{original}");

            let text = fs::read_to_string(tmp.path().join("config.toml")).unwrap();
            // The parser rejects a table defined twice, as Codex does.
            let strict = text
                .parse::<toml_edit::DocumentMut>()
                .unwrap_or_else(|e| panic!("{original:?} became invalid TOML: {e}\n{text}"));
            let servers = &strict["mcp_servers"];
            assert_eq!(servers.as_table_like().unwrap().len(), 2, "{text}");
            assert_eq!(
                servers["dotted"]["command"].as_str(),
                Some("/new"),
                "{text}"
            );
            assert_eq!(servers["dotted"]["type"].as_str(), Some("stdio"), "{text}");
            assert_eq!(
                servers["added"]["command"].as_str(),
                Some("/added"),
                "{text}"
            );
            if original.contains("cwd") {
                assert_eq!(servers["dotted"]["cwd"].as_str(), Some("/srv"), "{text}");
            }
        }
    }

    #[test]
    fn a_malformed_config_toml_is_refused_not_overwritten() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let broken = "[mcp_servers.x\ncommand = \"/bin/x\"\n";
        fs::write(&path, broken).unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        assert!(adapter
            .write_mcp_servers(&one_server(stdio_server("s", "/bin/s")))
            .is_err());
        assert!(adapter
            .write_preferences(&Preferences {
                model: Some("gpt-5".to_string()),
                custom: HashMap::new(),
            })
            .is_err());
        assert!(
            adapter.read_mcp_servers().is_err(),
            "no silent JSON fallback"
        );
        assert!(
            adapter.read_preferences().is_err(),
            "no silent JSON fallback"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
        assert!(!tmp.path().join("config.toml.skrills-bak").exists());
    }

    #[test]
    fn a_non_table_mcp_servers_key_is_refused() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        fs::write(&path, "mcp_servers = 3\n").unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        assert!(adapter
            .write_mcp_servers(&one_server(stdio_server("s", "/bin/s")))
            .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "mcp_servers = 3\n");
    }

    /// `model` must land in the root table even when the file ends inside
    /// another table, where a plain append would nest it.
    #[test]
    fn write_preferences_sets_the_top_level_model_in_config_toml() {
        let tmp = tempdir().unwrap();
        let original = "# top\n[features]\nskills = true\n\n[mcp_servers.a]\ncommand = \"/a\"\n";
        fs::write(tmp.path().join("config.toml"), original).unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let prefs = Preferences {
            model: Some("gpt-5".to_string()),
            custom: HashMap::new(),
        };

        let report = adapter.write_preferences(&prefs).unwrap();
        assert_eq!(report.written, 1);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert!(!tmp.path().join("config.json").exists());

        let doc = parsed_toml(tmp.path());
        assert_eq!(doc.get("model").and_then(|m| m.as_str()), Some("gpt-5"));
        assert!(doc["features"].get("model").is_none());
        assert!(doc["mcp_servers"]["a"].get("model").is_none());
        // A root key must precede every header, so it goes above the comment
        // that belongs to `[features]`; the comment stays with its table.
        let text = fs::read_to_string(tmp.path().join("config.toml")).unwrap();
        assert!(text.starts_with("model = \"gpt-5\"\n"), "{text}");
        assert!(text.contains("# top\n[features]\n"), "{text}");

        assert_eq!(
            adapter.read_preferences().unwrap().model.as_deref(),
            Some("gpt-5")
        );
        assert_eq!(adapter.write_preferences(&prefs).unwrap().written, 0);
    }

    #[test]
    fn config_toml_wins_over_config_json_when_it_has_values() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.json"),
            r#"{"model": "old", "mcpServers": {"legacy": {"command": "/legacy"}}}"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            "model = \"new\"\n[mcp_servers.current]\ncommand = \"/current\"\n",
        )
        .unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        assert_eq!(
            adapter.read_preferences().unwrap().model.as_deref(),
            Some("new")
        );
        let servers = adapter.read_mcp_servers().unwrap();
        assert_eq!(servers.keys().collect::<Vec<_>>(), vec!["current"]);
    }

    /// Fallback: config.toml without servers or model still reads config.json.
    #[test]
    fn config_json_is_read_when_config_toml_has_none() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.json"),
            r#"{"model": "gpt-4o", "mcpServers": {"legacy": {"command": "/legacy"}}}"#,
        )
        .unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            "[features]\nskills = true\n",
        )
        .unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        assert_eq!(
            adapter.read_preferences().unwrap().model.as_deref(),
            Some("gpt-4o")
        );
        assert_eq!(
            adapter.read_mcp_servers().unwrap()["legacy"].command,
            "/legacy"
        );
    }

    #[test]
    fn disabled_and_tool_filtered_servers_use_codex_keys() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let mut server = stdio_server("s", "/bin/s");
        server.enabled = false;
        server.allowed_tools = vec!["read".to_string()];
        server.disabled_tools = vec!["write".to_string()];

        adapter
            .write_mcp_servers(&one_server(server.clone()))
            .unwrap();

        let doc = parsed_toml(tmp.path());
        let entry = &doc["mcp_servers"]["s"];
        assert_eq!(entry["enabled"].as_bool(), Some(false));
        assert_eq!(entry["enabled_tools"][0].as_str(), Some("read"));
        assert_eq!(entry["disabled_tools"][0].as_str(), Some("write"));
        assert_eq!(adapter.read_mcp_servers().unwrap()["s"], server);
    }

    /// Replacing an HTTP entry with the source's stdio server must not leave
    /// `url` beside `command`, a mix Codex cannot run.
    #[test]
    fn a_stdio_server_replaces_an_http_entry_cleanly() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            "[mcp_servers.s]\ntype = \"http\"\nurl = \"http://127.0.0.1:3001/mcp\"\nbearer_token_env_var = \"TOK\"\nstartup_timeout_sec = 30\n",
        )
        .unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let report = adapter
            .write_mcp_servers(&one_server(stdio_server("s", "/bin/s")))
            .unwrap();
        assert_eq!(report.written, 1);

        let doc = parsed_toml(tmp.path());
        let entry = &doc["mcp_servers"]["s"];
        assert_eq!(entry["type"].as_str(), Some("stdio"));
        assert_eq!(entry["command"].as_str(), Some("/bin/s"));
        assert!(entry.get("url").is_none());
        assert!(entry.get("bearer_token_env_var").is_none());
        assert_eq!(entry["startup_timeout_sec"].as_integer(), Some(30));
    }

    fn http_server(name: &str, headers: &[(&str, &str)]) -> McpServer {
        let mut server = stdio_server(name, "");
        server.transport = McpTransport::Http;
        server.url = Some("https://example.invalid/mcp".to_string());
        server.headers = (!headers.is_empty()).then(|| {
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        });
        server
    }

    /// Codex runs streamable-HTTP servers, but it expands no `${VAR}` in a
    /// header value, so env references go to the keys Codex reads them from.
    #[test]
    fn an_http_server_is_written_with_its_headers_in_codex_form() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let server = http_server(
            "web",
            &[
                ("Authorization", "Bearer ${TOK}"),
                ("X-Key", "${KEY_VAR}"),
                ("X-Region", "us"),
            ],
        );

        let report = adapter
            .write_mcp_servers(&one_server(server.clone()))
            .unwrap();
        assert_eq!(report.written, 1);

        let doc = parsed_toml(tmp.path());
        let entry = &doc["mcp_servers"]["web"];
        assert_eq!(entry["type"].as_str(), Some("http"));
        assert_eq!(entry["url"].as_str(), Some("https://example.invalid/mcp"));
        assert_eq!(entry["bearer_token_env_var"].as_str(), Some("TOK"));
        assert_eq!(entry["env_http_headers"]["X-Key"].as_str(), Some("KEY_VAR"));
        assert_eq!(entry["http_headers"]["X-Region"].as_str(), Some("us"));
        assert!(entry.get("command").is_none());
        assert!(entry
            .get("http_headers")
            .unwrap()
            .get("Authorization")
            .is_none());

        assert_eq!(adapter.read_mcp_servers().unwrap()["web"], server);
        let again = adapter.write_mcp_servers(&one_server(server)).unwrap();
        assert_eq!(again.written, 0, "an unchanged HTTP server is rewritten");
    }

    #[test]
    fn an_http_server_replaces_a_stdio_entry_cleanly() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("config.toml"),
            "[mcp_servers.web]\ncommand = \"/bin/web\"\nargs = [\"-v\"]\ncwd = \"/srv\"\nstartup_timeout_sec = 30\n\n[mcp_servers.web.env]\nA = \"1\"\n",
        )
        .unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        let report = adapter
            .write_mcp_servers(&one_server(http_server("web", &[])))
            .unwrap();
        assert_eq!(report.written, 1);

        let doc = parsed_toml(tmp.path());
        let entry = &doc["mcp_servers"]["web"];
        assert_eq!(entry["url"].as_str(), Some("https://example.invalid/mcp"));
        for key in ["command", "args", "env", "cwd"] {
            assert!(entry.get(key).is_none(), "stdio key {key} left beside url");
        }
        assert_eq!(entry["startup_timeout_sec"].as_integer(), Some(30));
    }

    /// `Token ${X}` cannot be spelled in config.toml: writing it literally
    /// would send the text `${X}` to the server.
    #[test]
    fn an_http_header_codex_cannot_express_skips_the_server() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let server = http_server("web", &[("X-Api", "Token ${API}")]);

        let report = adapter.write_mcp_servers(&one_server(server)).unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(
            format!("{:?}", report.skipped[0]).contains("X-Api"),
            "{:?}",
            report.skipped
        );
        assert!(!tmp.path().join("config.toml").exists());
    }

    /// Without `url` the entry would have neither `url` nor `command`, which
    /// Codex cannot parse, and it then refuses to start.
    #[test]
    fn an_http_server_without_a_url_is_skipped() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());
        let mut server = http_server("web", &[]);
        server.url = None;

        let report = adapter.write_mcp_servers(&one_server(server)).unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.skipped.len(), 1);
        assert!(!tmp.path().join("config.toml").exists());
    }

    #[test]
    fn read_preferences_falls_back_to_config_json() {
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
    fn mcp_servers_empty_tool_configs_omitted_from_toml() {
        let tmp = tempdir().unwrap();
        let adapter = CodexAdapter::with_root(tmp.path().to_path_buf());

        adapter
            .write_mcp_servers(&one_server(stdio_server("clean-server", "/bin/server")))
            .unwrap();

        let doc = parsed_toml(tmp.path());
        let entry = &doc["mcp_servers"]["clean-server"];
        for key in ["enabled", "enabled_tools", "disabled_tools", "args", "env"] {
            assert!(
                entry.get(key).is_none(),
                "{key} written for a default server"
            );
        }
    }
}
