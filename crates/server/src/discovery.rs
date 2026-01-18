use anyhow::{anyhow, Result};
use pathdiff::diff_paths;
use skrills_discovery::{
    default_priority, discover_agents, discover_skills, load_priority_override,
    priority_labels as disc_priority_labels,
    priority_labels_and_rank_map as disc_priority_labels_and_rank_map, AgentMeta, SkillMeta,
    SkillRoot, SkillSource,
};
use skrills_state::{
    env_include_claude, env_include_marketplace, extra_dirs_from_env, home_dir,
    load_manifest_settings,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use skrills_discovery::hash_file;
#[cfg(test)]
use skrills_discovery::{default_roots, extra_skill_roots};
#[cfg(test)]
use skrills_discovery::{extract_refs_from_agents, priority_with_override};

/// URI for the AGENTS.md document.
pub const AGENTS_URI: &str = "doc://agents";
/// Name of the AGENTS.md document.
pub const AGENTS_NAME: &str = "AGENTS.md";
/// Description of the AGENTS.md document.
pub const AGENTS_DESCRIPTION: &str = "AI Agent Development Guidelines";
/// Content of the AGENTS.md document.
pub const AGENTS_TEXT: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/AGENTS.md"));
/// Environment variable to control exposure of AGENTS.md.
pub const ENV_EXPOSE_AGENTS: &str = "SKRILLS_EXPOSE_AGENTS";
/// Command template used to launch an agent specification.
pub const DEFAULT_AGENT_RUN_TEMPLATE: &str = r#"codex --yolo exec --timeout_ms 1800000 "Load agent spec at {} and execute its instructions""#;
/// Start marker for the available skills section in AGENTS.md.
pub const AGENTS_SECTION_START: &str = "<!-- available_skills:start -->";
/// End marker for the available skills section in AGENTS.md.
pub const AGENTS_SECTION_END: &str = "<!-- available_skills:end -->";
/// Start marker for available agents.
pub const AGENTS_AGENT_SECTION_START: &str = "<!-- available_agents:start -->";
/// End marker for available agents.
pub const AGENTS_AGENT_SECTION_END: &str = "<!-- available_agents:end -->";
/// Default bytes for embedding preview.
pub const DEFAULT_EMBED_PREVIEW_BYTES: usize = 4096;
/// Default embedding threshold.
pub const DEFAULT_EMBED_THRESHOLD: f32 = 0.18;
/// Environment variable to specify CLI type for agent file selection.
pub const ENV_CLI_TYPE: &str = "SKRILLS_CLI_TYPE";

/// Represents the type of CLI that skrills is integrated with.
///
/// Each CLI type has its own configuration directory and agent file naming convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliType {
    /// Codex CLI (default) - uses `~/.codex/AGENTS.md`
    Codex,
    /// Claude CLI - uses `~/.claude/CLAUDE.md`
    Claude,
    /// Gemini CLI - uses `~/.gemini/GEMINI.md`
    Gemini,
    /// Qwen CLI - uses `~/.qwen/QWEN.md`
    Qwen,
}

impl CliType {
    /// Returns the config directory name for this CLI type.
    pub fn config_dir(&self) -> &'static str {
        match self {
            CliType::Codex => ".codex",
            CliType::Claude => ".claude",
            CliType::Gemini => ".gemini",
            CliType::Qwen => ".qwen",
        }
    }

    /// Returns the agent file name for this CLI type.
    pub fn agent_file(&self) -> &'static str {
        match self {
            CliType::Codex => "AGENTS.md",
            CliType::Claude => "CLAUDE.md",
            CliType::Gemini => "GEMINI.md",
            CliType::Qwen => "QWEN.md",
        }
    }

    /// Returns the full path to the agent file.
    pub fn agent_path(&self, home: &Path) -> PathBuf {
        home.join(self.config_dir()).join(self.agent_file())
    }
}

/// Error returned when parsing an invalid CLI type string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseCliTypeError(String);

impl std::fmt::Display for ParseCliTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown CLI type: {}", self.0)
    }
}

impl std::error::Error for ParseCliTypeError {}

impl std::str::FromStr for CliType {
    type Err = ParseCliTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "codex" => Ok(CliType::Codex),
            "claude" => Ok(CliType::Claude),
            "gemini" => Ok(CliType::Gemini),
            "qwen" => Ok(CliType::Qwen),
            _ => Err(ParseCliTypeError(s.to_string())),
        }
    }
}

impl std::fmt::Display for CliType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliType::Codex => write!(f, "codex"),
            CliType::Claude => write!(f, "claude"),
            CliType::Gemini => write!(f, "gemini"),
            CliType::Qwen => write!(f, "qwen"),
        }
    }
}

/// Detects the CLI type from environment variables.
///
/// Checks the `SKRILLS_CLI_TYPE` environment variable first.
/// Returns `CliType::Codex` as the default if not set or unrecognized.
pub fn detect_cli_type() -> CliType {
    if let Ok(cli_type) = std::env::var(ENV_CLI_TYPE) {
        cli_type.parse().unwrap_or(CliType::Codex)
    } else {
        CliType::Codex
    }
}

/// Reads the agent file content for the detected CLI type.
///
/// Attempts to read the appropriate agent file based on CLI type.
/// Falls back to the embedded `AGENTS_TEXT` if the file doesn't exist.
pub fn read_agent_file() -> Result<String> {
    let home = home_dir()?;
    read_agent_file_for_cli(detect_cli_type(), &home)
}

/// Reads the agent file content for a specific CLI type.
///
/// Attempts to read the appropriate agent file based on CLI type.
/// Falls back to the embedded `AGENTS_TEXT` if the file doesn't exist.
pub fn read_agent_file_for_cli(cli_type: CliType, home: &Path) -> Result<String> {
    let path = cli_type.agent_path(home);
    if path.exists() {
        fs::read_to_string(&path).map_err(|e| anyhow!("failed to read agent file: {}", e))
    } else {
        // Fall back to embedded AGENTS.md
        Ok(AGENTS_TEXT.to_string())
    }
}

/// Returns the path to the agent file for the detected CLI type.
///
/// Returns `None` if the file doesn't exist.
pub fn agent_file_path() -> Result<Option<PathBuf>> {
    let home = home_dir()?;
    agent_file_path_for_cli(detect_cli_type(), &home)
}

/// Returns the path to the agent file for a specific CLI type.
///
/// Returns `None` if the file doesn't exist.
pub fn agent_file_path_for_cli(cli_type: CliType, home: &Path) -> Result<Option<PathBuf>> {
    let path = cli_type.agent_path(home);
    if path.exists() {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

/// Returns priority labels for skill sources.
pub fn priority_labels() -> Vec<String> {
    disc_priority_labels()
}

/// Returns priority labels and their corresponding rank map for skill sources.
pub fn priority_labels_and_rank_map() -> (Vec<String>, serde_json::Map<String, serde_json::Value>) {
    disc_priority_labels_and_rank_map()
}

/// Determines the skill root directories based on configuration and environment.
pub fn skill_roots(extra_dirs: &[PathBuf]) -> Result<Vec<SkillRoot>> {
    let home = home_dir()?;
    let include_claude = env_include_claude();
    let order = {
        if let Some(mut override_list) =
            load_priority_override(&|| Ok(load_manifest_settings()?.priority.clone()))?
        {
            let mut seen: HashSet<String> = override_list.iter().map(|s| s.label()).collect();
            for src in default_priority() {
                if seen.insert(src.label()) {
                    override_list.push(src);
                }
            }
            override_list
        } else {
            default_priority()
        }
    };
    let mut roots = Vec::new();
    for source in order {
        let root = match source {
            SkillSource::Codex => home.join(".codex/skills"),
            SkillSource::Claude => {
                if !include_claude {
                    continue;
                }
                home.join(".claude/skills")
            }
            SkillSource::Marketplace => {
                if !include_claude || !env_include_marketplace() {
                    continue;
                }
                home.join(".claude/plugins/marketplaces")
            }
            SkillSource::Cache => {
                if !include_claude {
                    continue;
                }
                home.join(".claude/plugins/cache")
            }
            SkillSource::Mirror => home.join(".codex/skills-mirror"),
            SkillSource::Agent => home.join(".agent/skills"),
            SkillSource::Extra(_) => continue,
            _ => continue, // Handle future variants
        };
        roots.push(SkillRoot { root, source });
    }
    for (idx, dir) in extra_dirs.iter().enumerate() {
        roots.push(SkillRoot {
            root: dir.clone(),
            source: SkillSource::Extra(idx as u32),
        });
    }
    Ok(roots)
}

/// Merges extra directories from environment variables and CLI arguments.
pub fn merge_extra_dirs(cli_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = extra_dirs_from_env();
    dirs.extend(cli_dirs.iter().cloned());
    dirs
}

/// Determines the agent root directories based on configuration and environment.
pub fn agent_roots(extra_dirs: &[PathBuf]) -> Result<Vec<SkillRoot>> {
    let home = home_dir()?;
    let mut roots = Vec::new();
    // Prefer mirrored agents first, then Claude/cache/marketplace agents.
    let order = default_priority();
    let include_claude = env_include_claude();
    for source in order {
        let root = match source {
            SkillSource::Codex => home.join(".codex/agents"),
            SkillSource::Claude => {
                if !include_claude {
                    continue;
                }
                home.join(".claude/agents")
            }
            SkillSource::Marketplace => {
                if !include_claude || !env_include_marketplace() {
                    continue;
                }
                home.join(".claude/plugins/marketplaces")
            }
            SkillSource::Cache => {
                if !include_claude {
                    continue;
                }
                home.join(".claude/plugins/cache")
            }
            SkillSource::Mirror => home.join(".codex/skills-mirror"), // skills mirror may include agents; keep for completeness
            SkillSource::Agent => home.join(".agent/agents"),
            SkillSource::Extra(_) => continue,
            _ => continue, // Handle future variants
        };
        roots.push(SkillRoot { root, source });
    }
    for (idx, dir) in extra_dirs.iter().enumerate() {
        roots.push(SkillRoot {
            root: dir.clone(),
            source: SkillSource::Extra(idx as u32),
        });
    }
    Ok(roots)
}

/// Returns the path to the AGENTS.md manifest, prioritizing local over home directory.
pub fn agents_manifest() -> Result<Option<PathBuf>> {
    let path = home_dir()?.join(".codex/AGENTS.md");
    if path.exists() {
        return Ok(Some(path));
    }
    let local = std::env::current_dir()?.join("AGENTS.md");
    if local.exists() {
        return Ok(Some(local));
    }
    Ok(None)
}

/// Collects skills from configured directories.
pub fn collect_skills(extra_dirs: &[PathBuf]) -> Result<Vec<SkillMeta>> {
    discover_skills(&skill_roots(extra_dirs)?, None)
}

/// Collects agents from configured directories.
pub fn collect_agents(extra_dirs: &[PathBuf]) -> Result<Vec<AgentMeta>> {
    discover_agents(&agent_roots(extra_dirs)?)
}

/// Reads the content of a skill file.
pub fn read_skill(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(_) => {
            // Fallback to lossily decoding non-UTF-8 skill files so we can
            // still serve and mirror multilingual/binary skills without
            // crashing.
            let bytes = fs::read(path)?;
            Ok(String::from_utf8_lossy(&bytes).to_string())
        }
    }
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.len() > haystack.len() {
        return false;
    }
    for i in 0..=(haystack.len() - needle.len()) {
        let mut ok = true;
        for j in 0..needle.len() {
            if !haystack[i + j].eq_ignore_ascii_case(&needle[j]) {
                ok = false;
                break;
            }
        }
        if ok {
            return true;
        }
    }
    false
}

/// Resolves a skill specification to its canonical name.
///
/// Handles partial matches and ambiguities.
pub fn resolve_skill<'a>(spec: &str, skills: &'a [SkillMeta]) -> Result<&'a str> {
    let mut matches: Vec<&str> = skills
        .iter()
        .map(|s| s.name.as_str())
        .filter(|name| name.eq_ignore_ascii_case(spec) || contains_ignore_ascii_case(name, spec))
        .collect();
    matches.sort_unstable();
    matches.dedup();
    match matches.len() {
        0 => Err(anyhow::anyhow!("skill not found for spec: {spec}")),
        1 => Ok(matches[0]),
        _ => Err(anyhow::anyhow!(
            "spec '{spec}' is ambiguous (matches: {})",
            matches.join(", ")
        )),
    }
}

/// Resolves an agent specification to its canonical metadata.
///
/// Handles partial matches and ambiguities.
pub fn resolve_agent<'a>(spec: &str, agents: &'a [AgentMeta]) -> Result<&'a AgentMeta> {
    let mut matches: Vec<&AgentMeta> = agents
        .iter()
        .filter(|a| a.name.eq_ignore_ascii_case(spec) || contains_ignore_ascii_case(&a.name, spec))
        .collect();
    matches.sort_by(|a, b| a.name.cmp(&b.name));
    matches.dedup_by(|a, b| a.name == b.name);
    match matches.len() {
        0 => Err(anyhow!("agent not found for spec: {spec}")),
        1 => Ok(matches[0]),
        _ => Err(anyhow!(
            "spec '{spec}' is ambiguous (matches: {})",
            matches
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Checks if a directory entry is a skill file (`SKILL.md`).
pub fn is_skill_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file() && entry.file_name() == "SKILL.md"
}

/// Tokenizes a prompt into a set of lowercase alphanumeric words.
pub fn tokenize_prompt(prompt: &str) -> HashSet<String> {
    prompt
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|s| s.len() >= 3)
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

/// Counts trigram occurrences in a given text.
pub fn trigram_counts(text: &str) -> HashMap<String, usize> {
    trigram_counts_keyed(text)
        .into_iter()
        .map(|(k, v)| (trigram_key_to_string(k), v))
        .collect()
}

type TrigramKey = u64;

fn trigram_key(a: char, b: char, c: char) -> TrigramKey {
    ((a as TrigramKey) << 42) | ((b as TrigramKey) << 21) | (c as TrigramKey)
}

fn trigram_key_to_string(key: TrigramKey) -> String {
    let a = ((key >> 42) & 0x1F_FFFF) as u32;
    let b = ((key >> 21) & 0x1F_FFFF) as u32;
    let c = (key & 0x1F_FFFF) as u32;
    let a = char::from_u32(a).unwrap_or('\u{FFFD}');
    let b = char::from_u32(b).unwrap_or('\u{FFFD}');
    let c = char::from_u32(c).unwrap_or('\u{FFFD}');

    let mut s = String::with_capacity(12);
    s.push(a);
    s.push(b);
    s.push(c);
    s
}

fn trigram_counts_keyed(text: &str) -> HashMap<TrigramKey, usize> {
    let mut it = text.chars().map(|c| c.to_ascii_lowercase());
    let mut a = match it.next() {
        Some(c) => c,
        None => return HashMap::new(),
    };
    let mut b = match it.next() {
        Some(c) => c,
        None => return HashMap::new(),
    };
    let mut c = match it.next() {
        Some(c) => c,
        None => return HashMap::new(),
    };

    let mut counts = HashMap::new();
    loop {
        *counts.entry(trigram_key(a, b, c)).or_insert(0) += 1;
        a = b;
        b = c;
        c = match it.next() {
            Some(next) => next,
            None => break,
        };
    }
    counts
}

fn cosine_similarity_keyed(a: &HashMap<TrigramKey, usize>, b: &HashMap<TrigramKey, usize>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut dot = 0f32;
    let mut norm_a = 0f32;
    let mut norm_b = 0f32;
    for (gram, &count) in a.iter() {
        norm_a += (count as f32).powi(2);
        if let Some(&b_count) = b.get(gram) {
            dot += (count as f32) * (b_count as f32);
        }
    }
    for &count in b.values() {
        norm_b += (count as f32).powi(2);
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Calculates the cosine similarity between two trigram count vectors.
pub fn cosine_similarity(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut dot = 0f32;
    let mut norm_a = 0f32;
    let mut norm_b = 0f32;
    for (gram, &count) in a.iter() {
        norm_a += (count as f32).powi(2);
        if let Some(&b_count) = b.get(gram) {
            dot += (count as f32) * (b_count as f32);
        }
    }
    for &count in b.values() {
        norm_b += (count as f32).powi(2);
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Calculates the trigram similarity between a prompt and text.
pub fn trigram_similarity(prompt: &str, text: &str) -> f32 {
    let prompt_vec = trigram_counts_keyed(prompt);
    let text_vec = trigram_counts_keyed(text);
    cosine_similarity_keyed(&prompt_vec, &text_vec)
}

/// Provides a static override for embedding similarity in tests.
#[cfg(test)]
pub static EMBED_SIM_OVERRIDE: std::sync::LazyLock<std::sync::Mutex<Option<f32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

/// A guard for setting embedding similarity override in tests.
#[cfg(test)]
pub struct EmbedOverrideGuard;

#[cfg(test)]
impl EmbedOverrideGuard {
    /// Sets the embedding similarity override value.
    pub fn set(value: f32) -> Self {
        if let Ok(mut guard) = EMBED_SIM_OVERRIDE.lock() {
            *guard = Some(value);
        }
        Self
    }
}

#[cfg(test)]
impl Drop for EmbedOverrideGuard {
    /// Clears the embedding similarity override on drop.
    fn drop(&mut self) {
        if let Ok(mut guard) = EMBED_SIM_OVERRIDE.lock() {
            *guard = None;
        }
    }
}

/// Calculates trigram similarity, respecting test overrides.
pub fn trigram_similarity_checked(prompt: &str, text: &str) -> f32 {
    #[cfg(test)]
    {
        if let Ok(guard) = EMBED_SIM_OVERRIDE.lock() {
            if let Some(v) = *guard {
                return v;
            }
        }
    }
    trigram_similarity(prompt, text)
}

/// Reads a prefix of a file's content, appending "…" if truncated.
pub fn read_prefix(path: &Path, max: usize) -> Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut buf = Vec::with_capacity(max);
    file.by_ref().take(max as u64).read_to_end(&mut buf)?;
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if buf.len() == max {
        s.push('…');
    }
    Ok(s)
}

/// Calculates the relative path from one path to another.
pub fn relative_path(from: &Path, to: &Path) -> Option<PathBuf> {
    diff_paths(to, from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::test_support::env_guard()
    }

    #[test]
    fn test_extract_refs() {
        let md = "foo SKILL.md bar rules baz";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("foo"));
        assert!(!refs.contains("rules"));
    }

    #[test]
    fn default_roots_use_priority_order() {
        let tmp = tempdir().unwrap();
        let roots = default_roots(tmp.path());
        let labels: Vec<_> = roots.iter().map(|r| r.source.label()).collect();
        assert_eq!(
            labels,
            vec![
                "codex",
                "mirror",
                "claude",
                "copilot",
                "marketplace",
                "cache",
                "agent"
            ]
        );
    }

    #[test]
    fn extra_skill_roots_preserve_input_order() {
        let one = PathBuf::from("/tmp/one");
        let two = PathBuf::from("/tmp/two");
        let roots = extra_skill_roots(&[one.clone(), two.clone()]);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].root, one);
        assert_eq!(roots[1].root, two);
        assert!(matches!(roots[0].source, SkillSource::Extra(0)));
        assert!(matches!(roots[1].source, SkillSource::Extra(1)));
    }

    #[test]
    fn discover_skills_errors_on_unreadable_file() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir().unwrap();
        let root = tmp.path().join("codex");
        fs::create_dir_all(&root).unwrap();
        let skill = root.join("SKILL.md");
        fs::write(&skill, "secret").unwrap();
        let mut perms = fs::metadata(&skill).unwrap().permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&skill, perms).unwrap();

        let roots = vec![SkillRoot {
            root: root.clone(),
            source: SkillSource::Codex,
        }];
        let skills = discover_skills(&roots, None).unwrap();
        assert_eq!(skills.len(), 1);
    }

    #[test]
    fn test_priority_with_override_uses_override() {
        let override_order = Some(vec![SkillSource::Claude, SkillSource::Codex]);
        let result = priority_with_override(override_order);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], SkillSource::Claude);
        assert_eq!(result[1], SkillSource::Codex);
    }

    #[test]
    fn test_priority_with_override_uses_default_when_none() {
        let result = priority_with_override(None);
        assert_eq!(result, default_priority());
    }

    #[test]
    fn test_load_priority_override_empty_returns_none() {
        let settings = || Ok(None);
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_priority_override_parses_valid_keys() {
        let settings = || Ok(Some(vec!["codex".to_string(), "claude".to_string()]));
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_some());
        let order = result.unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], SkillSource::Codex);
        assert_eq!(order[1], SkillSource::Claude);
    }

    #[test]
    fn test_load_priority_override_deduplicates() {
        let settings = || {
            Ok(Some(vec![
                "codex".to_string(),
                "codex".to_string(),
                "claude".to_string(),
            ]))
        };
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_some());
        let order = result.unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], SkillSource::Codex);
        assert_eq!(order[1], SkillSource::Claude);
    }

    #[test]
    fn test_load_priority_override_ignores_invalid_keys() {
        let settings = || {
            Ok(Some(vec![
                "codex".to_string(),
                "invalid-key".to_string(),
                "claude".to_string(),
            ]))
        };
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_some());
        let order = result.unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], SkillSource::Codex);
        assert_eq!(order[1], SkillSource::Claude);
    }

    #[test]
    fn test_load_priority_override_all_invalid_returns_none() {
        let settings = || Ok(Some(vec!["invalid1".to_string(), "invalid2".to_string()]));
        let result = load_priority_override(&settings).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_refs_from_agents_filters_skills_keyword() {
        let md = "Use rust_testing skill for testing";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("use"));
        assert!(refs.contains("rust_testing"));
        assert!(refs.contains("testing"));
        assert!(!refs.contains("skills"));
    }

    #[test]
    fn test_extract_refs_from_agents_filters_rules_keyword() {
        let md = "Follow the rules for coding";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("follow"));
        assert!(refs.contains("coding"));
        assert!(!refs.contains("rules"));
    }

    #[test]
    fn test_extract_refs_from_agents_handles_multiline() {
        let md = "Line one\nLine two with python\nLine three";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("line"));
        assert!(refs.contains("one"));
        assert!(refs.contains("two"));
        assert!(refs.contains("python"));
        assert!(refs.contains("three"));
    }

    #[test]
    fn test_extract_refs_from_agents_handles_special_chars() {
        let md = "test-case, foo_bar; baz:qux";
        let refs = extract_refs_from_agents(md);
        assert!(refs.contains("test-case"));
        assert!(refs.contains("foo_bar"));
        assert!(refs.contains("baz"));
        assert!(refs.contains("qux"));
    }

    #[test]
    fn resolve_agent_prefers_unique_match_and_flags_ambiguity() {
        let agents = vec![
            AgentMeta {
                name: "alpha/agent.md".into(),
                path: PathBuf::from("alpha/agent.md"),
                source: SkillSource::Codex,
                root: PathBuf::from("/root"),
                hash: "a".into(),
            },
            AgentMeta {
                name: "beta/agent.md".into(),
                path: PathBuf::from("beta/agent.md"),
                source: SkillSource::Claude,
                root: PathBuf::from("/root"),
                hash: "b".into(),
            },
        ];

        let resolved = resolve_agent("alpha", &agents).unwrap();
        assert_eq!(resolved.name, "alpha/agent.md");

        let err = resolve_agent("agent", &agents).unwrap_err();
        assert!(err.to_string().contains("ambiguous"));
    }

    #[test]
    fn test_discover_skills_with_duplicates() {
        let tmp = tempdir().unwrap();

        let codex_root = tmp.path().join("codex");
        fs::create_dir_all(codex_root.join("test-skill")).unwrap();
        fs::write(codex_root.join("test-skill/SKILL.md"), "codex version").unwrap();

        let claude_root = tmp.path().join("claude");
        fs::create_dir_all(claude_root.join("test-skill")).unwrap();
        fs::write(claude_root.join("test-skill/SKILL.md"), "claude version").unwrap();

        let roots = vec![
            SkillRoot {
                root: codex_root,
                source: SkillSource::Codex,
            },
            SkillRoot {
                root: claude_root,
                source: SkillSource::Claude,
            },
        ];

        let mut dup_log = vec![];
        let skills = discover_skills(&roots, Some(&mut dup_log)).unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Codex);

        assert_eq!(dup_log.len(), 1);
        assert_eq!(dup_log[0].name, "test-skill/SKILL.md");
        assert_eq!(dup_log[0].kept_source, "codex");
        assert_eq!(dup_log[0].skipped_source, "claude");
    }

    #[test]
    fn test_discover_skills_max_depth_limit() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");

        let deep_path = root.join("a/b/c/d/e/f/g");
        fs::create_dir_all(&deep_path).unwrap();
        fs::write(deep_path.join("SKILL.md"), "deep skill").unwrap();

        let shallow_path = root.join("shallow");
        fs::create_dir_all(&shallow_path).unwrap();
        fs::write(shallow_path.join("SKILL.md"), "shallow skill").unwrap();

        // Path deliberately deeper than MAX_SKILL_DEPTH to ensure it is ignored.
        let too_deep_path = root.join(
            [
                "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9", "d10", "d11", "d12", "d13",
                "d14", "d15", "d16", "d17", "d18", "d19", "d20", "d21",
            ]
            .iter()
            .collect::<PathBuf>(),
        );
        fs::create_dir_all(&too_deep_path).unwrap();
        fs::write(too_deep_path.join("SKILL.md"), "too deep").unwrap();

        let roots = vec![SkillRoot {
            root,
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None).unwrap();

        assert_eq!(skills.len(), 2);
        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("shallow")));
        assert!(names.iter().any(|n| n.contains("a/b/c/d/e/f/g")));
        assert!(!names.iter().any(|n| n.contains("d21")));
    }

    #[test]
    fn test_discover_skills_ignores_directories() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("skills");
        fs::create_dir_all(&root).unwrap();

        fs::create_dir_all(root.join("SKILL.md")).unwrap();

        let skill_dir = root.join("real-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "real skill").unwrap();

        let roots = vec![SkillRoot {
            root,
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None).unwrap();

        assert_eq!(skills.len(), 1);
        assert!(skills[0].name.contains("real-skill"));
    }

    #[test]
    fn test_hash_file_consistent() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("test.md");
        fs::write(&file, "test content").unwrap();

        let hash1 = hash_file(&file).unwrap();
        let hash2 = hash_file(&file).unwrap();

        assert_eq!(hash1, hash2);
        assert!(!hash1.is_empty());
    }

    #[test]
    fn test_hash_file_different_content() {
        let tmp = tempdir().unwrap();

        let file1 = tmp.path().join("test1.md");
        fs::write(&file1, "content 1").unwrap();

        let file2 = tmp.path().join("test2.md");
        fs::write(&file2, "content 2").unwrap();

        let hash1 = hash_file(&file1).unwrap();
        let hash2 = hash_file(&file2).unwrap();

        assert!(!hash1.is_empty());
        assert!(!hash2.is_empty());
    }

    #[test]
    fn test_priority_labels() {
        let labels = priority_labels();
        assert_eq!(
            labels,
            vec![
                "codex",
                "mirror",
                "claude",
                "copilot",
                "marketplace",
                "cache",
                "agent"
            ]
        );
    }

    #[test]
    fn skill_roots_excludes_claude_when_env_disabled() {
        let _guard = env_guard();
        std::env::set_var("SKRILLS_INCLUDE_CLAUDE", "0");
        let roots = skill_roots(&[]).unwrap();
        let labels: Vec<_> = roots.iter().map(|r| r.source.label()).collect();
        assert!(
            !labels.contains(&"claude".to_string())
                && !labels.contains(&"marketplace".to_string())
                && !labels.contains(&"cache".to_string()),
            "Claude-derived roots should be excluded when SKRILLS_INCLUDE_CLAUDE=0"
        );
        std::env::remove_var("SKRILLS_INCLUDE_CLAUDE");
    }

    #[test]
    fn test_priority_labels_and_rank_map() {
        let (labels, rank_map) = priority_labels_and_rank_map();

        assert_eq!(labels.len(), 7);
        assert_eq!(rank_map.len(), 7);

        assert_eq!(rank_map.get("codex").unwrap(), 1);
        assert_eq!(rank_map.get("mirror").unwrap(), 2);
        assert_eq!(rank_map.get("claude").unwrap(), 3);
        assert_eq!(rank_map.get("copilot").unwrap(), 4);
        assert_eq!(rank_map.get("marketplace").unwrap(), 5);
        assert_eq!(rank_map.get("cache").unwrap(), 6);
        assert_eq!(rank_map.get("agent").unwrap(), 7);
    }

    #[test]
    fn test_discover_skills_empty_root() {
        let tmp = tempdir().unwrap();
        let empty_root = tmp.path().join("empty");
        fs::create_dir_all(&empty_root).unwrap();

        let roots = vec![SkillRoot {
            root: empty_root,
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None).unwrap();
        assert_eq!(skills.len(), 0);
    }

    #[test]
    fn test_discover_skills_nonexistent_root() {
        let tmp = tempdir().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");

        let roots = vec![SkillRoot {
            root: nonexistent,
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None).unwrap();
        assert_eq!(skills.len(), 0);
    }

    #[test]
    fn resolve_skill_is_case_insensitive_and_supports_substring_match() {
        let skills = vec![
            SkillMeta {
                name: "FooBar".to_string(),
                path: PathBuf::from("/tmp/SKILL.md"),
                source: SkillSource::Codex,
                root: PathBuf::from("/tmp"),
                hash: "x".to_string(),
                description: None,
            },
            SkillMeta {
                name: "Baz".to_string(),
                path: PathBuf::from("/tmp/SKILL.md"),
                source: SkillSource::Codex,
                root: PathBuf::from("/tmp"),
                hash: "y".to_string(),
                description: None,
            },
        ];

        assert_eq!(resolve_skill("foobar", &skills).unwrap(), "FooBar");
        assert_eq!(resolve_skill("BAR", &skills).unwrap(), "FooBar");
    }

    #[test]
    fn resolve_agent_is_case_insensitive_and_supports_substring_match() {
        let agents = vec![
            AgentMeta {
                name: "AlphaBeta".to_string(),
                path: PathBuf::from("/tmp/agent.md"),
                source: SkillSource::Codex,
                root: PathBuf::from("/tmp"),
                hash: "x".to_string(),
            },
            AgentMeta {
                name: "Gamma".to_string(),
                path: PathBuf::from("/tmp/agent.md"),
                source: SkillSource::Codex,
                root: PathBuf::from("/tmp"),
                hash: "y".to_string(),
            },
        ];

        assert_eq!(
            resolve_agent("alphabeta", &agents).unwrap().name,
            "AlphaBeta"
        );
        assert_eq!(resolve_agent("BETA", &agents).unwrap().name, "AlphaBeta");
    }

    #[test]
    fn trigram_similarity_is_ascii_case_insensitive() {
        assert_eq!(trigram_similarity("AbC", "abc"), 1.0);
        assert_eq!(trigram_similarity("abc", "abd"), 0.0);
    }

    // CliType tests

    #[test]
    fn cli_type_config_dir_returns_correct_directories() {
        assert_eq!(CliType::Codex.config_dir(), ".codex");
        assert_eq!(CliType::Claude.config_dir(), ".claude");
        assert_eq!(CliType::Gemini.config_dir(), ".gemini");
        assert_eq!(CliType::Qwen.config_dir(), ".qwen");
    }

    #[test]
    fn cli_type_agent_file_returns_correct_filenames() {
        assert_eq!(CliType::Codex.agent_file(), "AGENTS.md");
        assert_eq!(CliType::Claude.agent_file(), "CLAUDE.md");
        assert_eq!(CliType::Gemini.agent_file(), "GEMINI.md");
        assert_eq!(CliType::Qwen.agent_file(), "QWEN.md");
    }

    #[test]
    fn cli_type_agent_path_constructs_correct_path() {
        let home = PathBuf::from("/home/user");
        assert_eq!(
            CliType::Codex.agent_path(&home),
            PathBuf::from("/home/user/.codex/AGENTS.md")
        );
        assert_eq!(
            CliType::Claude.agent_path(&home),
            PathBuf::from("/home/user/.claude/CLAUDE.md")
        );
        assert_eq!(
            CliType::Gemini.agent_path(&home),
            PathBuf::from("/home/user/.gemini/GEMINI.md")
        );
        assert_eq!(
            CliType::Qwen.agent_path(&home),
            PathBuf::from("/home/user/.qwen/QWEN.md")
        );
    }

    #[test]
    fn cli_type_from_str_parses_valid_types() {
        assert_eq!("codex".parse::<CliType>(), Ok(CliType::Codex));
        assert_eq!("claude".parse::<CliType>(), Ok(CliType::Claude));
        assert_eq!("gemini".parse::<CliType>(), Ok(CliType::Gemini));
        assert_eq!("qwen".parse::<CliType>(), Ok(CliType::Qwen));
    }

    #[test]
    fn cli_type_from_str_is_case_insensitive() {
        assert_eq!("CODEX".parse::<CliType>(), Ok(CliType::Codex));
        assert_eq!("Claude".parse::<CliType>(), Ok(CliType::Claude));
        assert_eq!("GEMINI".parse::<CliType>(), Ok(CliType::Gemini));
        assert_eq!("Qwen".parse::<CliType>(), Ok(CliType::Qwen));
    }

    #[test]
    fn cli_type_from_str_returns_error_for_invalid() {
        assert!("invalid".parse::<CliType>().is_err());
        assert!("".parse::<CliType>().is_err());
        assert!("openai".parse::<CliType>().is_err());
    }

    #[test]
    fn cli_type_display_formats_correctly() {
        assert_eq!(format!("{}", CliType::Codex), "codex");
        assert_eq!(format!("{}", CliType::Claude), "claude");
        assert_eq!(format!("{}", CliType::Gemini), "gemini");
        assert_eq!(format!("{}", CliType::Qwen), "qwen");
    }

    #[test]
    fn detect_cli_type_defaults_to_codex() {
        let _guard = env_guard();
        std::env::remove_var(ENV_CLI_TYPE);
        assert_eq!(detect_cli_type(), CliType::Codex);
    }

    #[test]
    fn detect_cli_type_respects_env_var() {
        let _guard = env_guard();

        std::env::set_var(ENV_CLI_TYPE, "claude");
        assert_eq!(detect_cli_type(), CliType::Claude);

        std::env::set_var(ENV_CLI_TYPE, "gemini");
        assert_eq!(detect_cli_type(), CliType::Gemini);

        std::env::set_var(ENV_CLI_TYPE, "qwen");
        assert_eq!(detect_cli_type(), CliType::Qwen);

        std::env::remove_var(ENV_CLI_TYPE);
    }

    #[test]
    fn detect_cli_type_defaults_on_invalid_value() {
        let _guard = env_guard();
        std::env::set_var(ENV_CLI_TYPE, "invalid-cli");
        assert_eq!(detect_cli_type(), CliType::Codex);
        std::env::remove_var(ENV_CLI_TYPE);
    }

    #[test]
    fn read_agent_file_for_cli_reads_existing_file() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Create the Claude agent file
        let claude_dir = home.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let claude_file = claude_dir.join("CLAUDE.md");
        fs::write(&claude_file, "# Claude Agent Config").unwrap();

        let content = read_agent_file_for_cli(CliType::Claude, home).unwrap();
        assert_eq!(content, "# Claude Agent Config");
    }

    #[test]
    fn read_agent_file_for_cli_falls_back_to_embedded() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Don't create any files, should fall back to embedded
        let content = read_agent_file_for_cli(CliType::Claude, home).unwrap();
        assert_eq!(content, AGENTS_TEXT);
    }

    #[test]
    fn agent_file_path_for_cli_returns_path_when_exists() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        // Create the Gemini agent file
        let gemini_dir = home.join(".gemini");
        fs::create_dir_all(&gemini_dir).unwrap();
        let gemini_file = gemini_dir.join("GEMINI.md");
        fs::write(&gemini_file, "# Gemini Config").unwrap();

        let path = agent_file_path_for_cli(CliType::Gemini, home).unwrap();
        assert_eq!(path, Some(gemini_file));
    }

    #[test]
    fn agent_file_path_for_cli_returns_none_when_missing() {
        let tmp = tempdir().unwrap();
        let home = tmp.path();

        let path = agent_file_path_for_cli(CliType::Qwen, home).unwrap();
        assert_eq!(path, None);
    }
}
