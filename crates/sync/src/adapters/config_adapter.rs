//! The one adapter behind every `~/.skrills/adapters/<name>.toml`.
//!
//! Formats match Claude's, the tool whose layout is plain markdown: commands
//! and agents are `<name>.md` files copied byte for byte, a skill is
//! `<name>/SKILL.md` plus its companion files, instructions are a
//! skrills-managed block in one markdown file, and MCP servers are entries
//! in a JSON object under a configurable key (written in Claude's dialect).
//! Only the kinds the config names are supported.

use super::claude::instructions::{read_instructions_at, write_instructions_at};
use super::json_config::{self, Dialect};
use super::traits::{AgentAdapter, FieldSupport};
use super::utils::{
    collect_module_files, hash_content, is_hidden_component, read_item_file, sanitize_name,
    skill_folder_name, BatchWriter,
};
use crate::adapter_config::AdapterConfig;
use crate::common::{Command, ContentFormat, McpServer, ModuleFile, Preferences};
use crate::report::WriteReport;
use crate::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// An adapter described by an [`AdapterConfig`].
#[derive(Debug, Clone)]
pub struct ConfigAdapter {
    config: AdapterConfig,
}

impl ConfigAdapter {
    /// Wraps a validated config.
    pub fn new(config: AdapterConfig) -> Self {
        Self { config }
    }
}

/// Every immediate entry of `dir` that `keep` accepts, by name. A missing
/// directory has no entries. Links are not followed: a linked file could be
/// any file on disk and would be copied into every other tool.
fn entries(dir: &Path, keep: fn(&std::fs::FileType) -> bool) -> Result<Vec<(String, PathBuf)>> {
    let reader = match std::fs::read_dir(dir) {
        Ok(reader) => reader,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut found = Vec::new();
    for entry in reader {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!(error = %e, "Skipping unreadable directory entry");
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_hidden_component(&name) || !entry.file_type().is_ok_and(|kind| keep(&kind)) {
            continue;
        }
        found.push((name, entry.path()));
    }
    found.sort();
    Ok(found)
}

fn item(name: String, path: &Path, modules: Vec<ModuleFile>) -> Option<Command> {
    let (content, modified) = read_item_file(path)?;
    Some(Command {
        name,
        hash: hash_content(&content),
        content,
        source_path: path.to_path_buf(),
        modified,
        modules,
        content_format: ContentFormat::default(),
        plugin_origin: None,
    })
}

fn read_markdown_dir(dir: Option<&PathBuf>) -> Result<Vec<Command>> {
    let Some(dir) = dir else {
        return Ok(Vec::new());
    };
    Ok(entries(dir, std::fs::FileType::is_file)?
        .into_iter()
        .filter(|(_, path)| path.extension().is_some_and(|ext| ext == "md"))
        .filter_map(|(_, path)| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            item(name, &path, Vec::new())
        })
        .collect())
}

fn write_markdown_dir(dir: Option<&PathBuf>, items: &[Command]) -> Result<WriteReport> {
    let mut report = WriteReport::default();
    let Some(dir) = dir else {
        return Ok(report);
    };
    if items.is_empty() {
        return Ok(report);
    }
    std::fs::create_dir_all(dir)?;
    let mut writer = BatchWriter::new(dir);
    for entry in items {
        writer.write_single(
            &entry.name,
            &sanitize_name(&entry.name),
            ".md",
            &entry.content,
            &mut report,
        )?;
    }
    Ok(report)
}

impl AgentAdapter for ConfigAdapter {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn config_root(&self) -> PathBuf {
        self.config.root.clone()
    }

    fn read_support(&self) -> FieldSupport {
        FieldSupport {
            skills: self.config.skills_dir.is_some(),
            commands: self.config.commands_dir.is_some(),
            agents: self.config.agents_dir.is_some(),
            instructions: self.config.instructions_file.is_some(),
            mcp_servers: self.config.mcp.is_some(),
            ..FieldSupport::default()
        }
    }

    fn write_support(&self) -> FieldSupport {
        self.read_support()
    }

    fn read_commands(&self, _include_marketplace: bool) -> Result<Vec<Command>> {
        read_markdown_dir(self.config.commands_dir.as_ref())
    }

    fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>> {
        match &self.config.mcp {
            Some(mcp) => json_config::read_servers_at(&mcp.path, &mcp.servers_key),
            None => Ok(HashMap::new()),
        }
    }

    fn read_preferences(&self) -> Result<Preferences> {
        Ok(Preferences::default())
    }

    fn read_skills(&self) -> Result<Vec<Command>> {
        let Some(dir) = &self.config.skills_dir else {
            return Ok(Vec::new());
        };
        Ok(entries(dir, std::fs::FileType::is_dir)?
            .into_iter()
            .filter_map(|(name, folder)| {
                item(
                    name,
                    &folder.join("SKILL.md"),
                    collect_module_files(&folder),
                )
            })
            .collect())
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        Ok(Vec::new())
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        read_markdown_dir(self.config.agents_dir.as_ref())
    }

    fn read_instructions(&self) -> Result<Vec<Command>> {
        match &self.config.instructions_file {
            Some(path) => read_instructions_at(path),
            None => Ok(Vec::new()),
        }
    }

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        write_markdown_dir(self.config.commands_dir.as_ref(), commands)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        let Some(mcp) = &self.config.mcp else {
            return Ok(WriteReport::default());
        };
        let mut document = json_config::load_object(&mcp.path)?;
        let report = json_config::merge_servers_at(
            &mut document,
            &mcp.servers_key,
            servers,
            Dialect::Claude,
            &self.config.name,
        )?;
        if report.written > 0 {
            json_config::write_json_config(&mcp.path, &document)?;
        }
        Ok(report)
    }

    fn write_preferences(&self, _prefs: &Preferences) -> Result<WriteReport> {
        Ok(WriteReport::default())
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        let mut report = WriteReport::default();
        let Some(dir) = &self.config.skills_dir else {
            return Ok(report);
        };
        if skills.is_empty() {
            return Ok(report);
        }
        std::fs::create_dir_all(dir)?;
        let mut writer = BatchWriter::new(dir);
        for skill in skills {
            writer.write(
                &skill.name,
                &skill_folder_name(skill),
                "SKILL.md",
                &skill.content,
                &skill.modules,
                &mut report,
            )?;
        }
        Ok(report)
    }

    fn write_hooks(&self, _hooks: &[Command]) -> Result<WriteReport> {
        Ok(WriteReport::default())
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        write_markdown_dir(self.config.agents_dir.as_ref(), agents)
    }

    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport> {
        match &self.config.instructions_file {
            Some(path) => write_instructions_at(path, instructions),
            None => Ok(WriteReport::default()),
        }
    }
}
