use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents the origin of a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash, Deserialize)]
pub enum SkillSource {
    Codex,
    Claude,
    Marketplace,
    Cache,
    Mirror,
    Agent,
    Extra(u32),
}

impl SkillSource {
    /// Returns a stable label for this source.
    pub fn label(&self) -> String {
        match self {
            SkillSource::Codex => "codex".into(),
            SkillSource::Claude => "claude".into(),
            SkillSource::Marketplace => "marketplace".into(),
            SkillSource::Cache => "cache".into(),
            SkillSource::Mirror => "mirror".into(),
            SkillSource::Agent => "agent".into(),
            SkillSource::Extra(n) => format!("extra{n}"),
        }
    }

    /// Returns a human-friendly location tag for diagnostics.
    ///
    /// - `global`: user-level shared skills (`~/.codex`, `~/.claude`, mirror).
    /// - `universal`: cross-agent shared skills (`~/.agent`).
    /// - `project`: extra/user-specified directories.
    pub fn location(&self) -> &'static str {
        match self {
            SkillSource::Codex
            | SkillSource::Claude
            | SkillSource::Marketplace
            | SkillSource::Cache
            | SkillSource::Mirror => "global",
            SkillSource::Agent => "universal",
            SkillSource::Extra(_) => "project",
        }
    }
}

/// Parses a string key into a `SkillSource` variant.
pub fn parse_source_key(key: &str) -> Option<SkillSource> {
    match key.to_ascii_lowercase().as_str() {
        "codex" => Some(SkillSource::Codex),
        "claude" => Some(SkillSource::Claude),
        "marketplace" => Some(SkillSource::Marketplace),
        "cache" => Some(SkillSource::Cache),
        "mirror" => Some(SkillSource::Mirror),
        "agent" => Some(SkillSource::Agent),
        _ => None,
    }
}

/// Represents a root directory where skills are discovered, along with its associated source type.
#[derive(Debug, Clone)]
pub struct SkillRoot {
    pub root: PathBuf,
    pub source: SkillSource,
}

/// Metadata for a discovered skill.
///
/// Includes its name, file path, source of discovery, root directory, and content hash.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub path: PathBuf,
    pub source: SkillSource,
    pub root: PathBuf,
    pub hash: String,
}

/// Metadata for a discovered agent definition.
#[derive(Debug, Serialize, Clone)]
pub struct AgentMeta {
    pub name: String,
    pub path: PathBuf,
    pub source: SkillSource,
    pub root: PathBuf,
    pub hash: String,
}

/// Information about a duplicate skill that was skipped due to priority.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateInfo {
    pub name: String,
    pub skipped_source: String,
    pub skipped_root: String,
    pub kept_source: String,
    pub kept_root: String,
}

/// Diagnostic information related to skill processing.
#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Diagnostics {
    /// Skills included in the output (name, source, root, location).
    pub included: Vec<(String, String, String, String)>,
    /// Skills skipped with a reason.
    pub skipped: Vec<(String, String)>,
    /// Duplicate skills encountered and resolved by priority.
    pub duplicates: Vec<DuplicateInfo>,
    /// Indicates if the output was truncated.
    pub truncated: bool,
    /// Indicates if skill content was omitted to fit manifest size.
    pub truncated_content: bool,
    /// The render mode selected for the output.
    pub render_mode: Option<String>,
}
