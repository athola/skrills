use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents the origin of a skill, indicating where it was discovered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash, Deserialize)]
pub enum SkillSource {
    Codex,
    Claude,
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
            SkillSource::Mirror => "mirror".into(),
            SkillSource::Agent => "agent".into(),
            SkillSource::Extra(n) => format!("extra{n}"),
        }
    }

    /// Returns a human-friendly location tag for diagnostics.
    /// - global: user-level shared skills (~/.codex, ~/.claude, mirror)
    /// - universal: cross-agent shared skills (~/.agent)
    /// - project: extra/user-specified directories
    pub fn location(&self) -> &'static str {
        match self {
            SkillSource::Codex | SkillSource::Claude | SkillSource::Mirror => "global",
            SkillSource::Agent => "universal",
            SkillSource::Extra(_) => "project",
        }
    }
}

/// Parses a string key into a SkillSource variant.
pub fn parse_source_key(key: &str) -> Option<SkillSource> {
    match key.to_ascii_lowercase().as_str() {
        "codex" => Some(SkillSource::Codex),
        "claude" => Some(SkillSource::Claude),
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
/// This includes its name, file path, source of discovery, root directory, and content hash.
#[derive(Debug, Serialize, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub path: PathBuf,
    pub source: SkillSource,
    pub root: PathBuf,
    pub hash: String,
}

/// Information about a duplicate skill that was skipped.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateInfo {
    pub name: String,
    pub skipped_source: String,
    pub skipped_root: String,
    pub kept_source: String,
    pub kept_root: String,
}

/// Stores diagnostic information related to skill processing,
/// including included, skipped, and found duplicate skills.
#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Diagnostics {
    pub included: Vec<(String, String, String, String)>, // name, source, root, location
    pub skipped: Vec<(String, String)>,                  // name, reason
    pub duplicates: Vec<DuplicateInfo>,                  // found duplicates
    pub truncated: bool,
    pub truncated_content: bool,
    pub render_mode: Option<String>,
}
