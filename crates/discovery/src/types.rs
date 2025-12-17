use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Represents the origin of a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Hash, Deserialize)]
#[non_exhaustive]
pub enum SkillSource {
    /// Codex CLI skills directory (`~/.codex/skills`).
    Codex,
    /// Claude Code skills directory (`~/.claude/skills`).
    Claude,
    /// Claude Code marketplace plugins (`~/.claude/plugins/marketplaces`).
    Marketplace,
    /// Claude Code plugin cache (`~/.claude/plugins/cache`).
    Cache,
    /// Codex mirror directory (`~/.codex/skills-mirror`).
    Mirror,
    /// Universal agent skills (`~/.agent/skills`).
    Agent,
    /// Extra user-specified directories (indexed).
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
///
/// ```
/// use skrills_discovery::{parse_source_key, SkillSource};
///
/// assert_eq!(parse_source_key("mirror"), Some(SkillSource::Mirror));
/// assert_eq!(parse_source_key("unknown"), None);
/// ```
pub fn parse_source_key(key: &str) -> Option<SkillSource> {
    if key.eq_ignore_ascii_case("codex") {
        Some(SkillSource::Codex)
    } else if key.eq_ignore_ascii_case("claude") {
        Some(SkillSource::Claude)
    } else if key.eq_ignore_ascii_case("marketplace") {
        Some(SkillSource::Marketplace)
    } else if key.eq_ignore_ascii_case("cache") {
        Some(SkillSource::Cache)
    } else if key.eq_ignore_ascii_case("mirror") {
        Some(SkillSource::Mirror)
    } else if key.eq_ignore_ascii_case("agent") {
        Some(SkillSource::Agent)
    } else {
        None
    }
}

/// Represents a root directory where skills are discovered, along with its associated source type.
#[derive(Debug, Clone)]
pub struct SkillRoot {
    /// The root directory path.
    pub root: PathBuf,
    /// The source type for skills in this root.
    pub source: SkillSource,
}

/// Metadata for a discovered skill.
///
/// Includes its name, file path, source of discovery, root directory, and content hash.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SkillMeta {
    /// The skill name (from frontmatter or filename).
    pub name: String,
    /// Path to the SKILL.md file.
    pub path: PathBuf,
    /// The source where this skill was discovered.
    pub source: SkillSource,
    /// The root directory containing this skill.
    pub root: PathBuf,
    /// Content hash for change detection.
    pub hash: String,
}

/// Metadata for a discovered agent definition.
#[derive(Debug, Serialize, Clone)]
pub struct AgentMeta {
    /// The agent name.
    pub name: String,
    /// Path to the agent definition file.
    pub path: PathBuf,
    /// The source where this agent was discovered.
    pub source: SkillSource,
    /// The root directory containing this agent.
    pub root: PathBuf,
    /// Content hash for change detection.
    pub hash: String,
}

/// Information about a duplicate skill that was skipped due to priority.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DuplicateInfo {
    /// The skill name that was duplicated.
    pub name: String,
    /// The source label of the skipped skill.
    pub skipped_source: String,
    /// The root path of the skipped skill.
    pub skipped_root: String,
    /// The source label of the kept skill.
    pub kept_source: String,
    /// The root path of the kept skill.
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
