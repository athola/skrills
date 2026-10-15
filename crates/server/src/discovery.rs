use anyhow::{anyhow, Result};
use pathdiff::diff_paths;
use skrills_discovery::{
    default_priority, discover_agents, discover_skills, priority_labels as disc_priority_labels,
    priority_labels_and_rank_map as disc_priority_labels_and_rank_map, AgentMeta, RootSettings,
    SkillMeta, SkillRoot, SkillSource,
};
use skrills_state::{env_include_claude, env_include_marketplace, extra_dirs_from_env, home_dir};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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
/// Command template used to launch an agent specification: the command
/// `skrills agent` runs, with `{}` for the agent file path. Built by
/// `skrills_subagents::launch`; a test keeps the two equal.
pub const DEFAULT_AGENT_RUN_TEMPLATE: &str = r#"codex exec --sandbox workspace-write "Load the agent spec file at the path \"{}\" and execute its instructions""#;
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
    Ok(skrills_discovery::skill_roots(
        &home_dir()?,
        &RootSettings::from_env()?,
        extra_dirs,
    ))
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

/// Returns the path to the AGENTS.md manifest: `~/.codex/AGENTS.md` when it
/// exists, otherwise `AGENTS.md` in the current directory.
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
/// A unique case-insensitive exact match wins outright; otherwise the spec is
/// matched as a substring and must select exactly one skill.
pub fn resolve_skill<'a>(spec: &str, skills: &'a [SkillMeta]) -> Result<&'a str> {
    let mut exact: Vec<&str> = skills
        .iter()
        .map(|s| s.name.as_str())
        .filter(|name| name.eq_ignore_ascii_case(spec))
        .collect();
    exact.sort_unstable();
    exact.dedup();
    if let [only] = exact.as_slice() {
        return Ok(only);
    }
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
/// A unique case-insensitive exact match wins outright; otherwise the spec is
/// matched as a substring and must select exactly one agent.
pub fn resolve_agent<'a>(spec: &str, agents: &'a [AgentMeta]) -> Result<&'a AgentMeta> {
    let mut exact: Vec<&AgentMeta> = agents
        .iter()
        .filter(|a| a.name.eq_ignore_ascii_case(spec))
        .collect();
    exact.sort_by(|a, b| a.name.cmp(&b.name));
    exact.dedup_by(|a, b| a.name == b.name);
    if let [only] = exact.as_slice() {
        return Ok(only);
    }
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

/// Checks if a directory entry is a skill file (`SKILL.md`): the rule
/// discovery itself uses, re-exported so callers here cannot drift from it.
pub use skrills_discovery::is_skill_file;

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

fn cosine_similarity_keyed<K: std::hash::Hash + Eq>(
    a: &HashMap<K, usize>,
    b: &HashMap<K, usize>,
) -> f32 {
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
    cosine_similarity_keyed(a, b)
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
    // One byte past `max` tells a file of exactly `max` bytes from a longer one.
    let mut buf = Vec::with_capacity(max.saturating_add(1));
    file.by_ref()
        .take((max as u64).saturating_add(1))
        .read_to_end(&mut buf)?;
    let truncated = buf.len() > max;
    if truncated {
        buf.truncate(max);
        // Drop a character the cut split rather than show U+FFFD.
        if let Err(e) = std::str::from_utf8(&buf) {
            if e.error_len().is_none() {
                buf.truncate(e.valid_up_to());
            }
        }
    }
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
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

    fn skill(name: &str) -> SkillMeta {
        SkillMeta {
            name: name.to_string(),
            path: PathBuf::from("/tmp/SKILL.md"),
            source: SkillSource::Codex,
            root: PathBuf::from("/tmp"),
            hash: "x".to_string(),
            description: None,
            frontmatter_name: None,
        }
    }

    fn agent(name: &str) -> AgentMeta {
        AgentMeta {
            name: name.to_string(),
            path: PathBuf::from("/tmp/agent.md"),
            source: SkillSource::Codex,
            root: PathBuf::from("/tmp"),
            hash: "x".to_string(),
        }
    }

    /// The public String-keyed variant now delegates to the keyed one, so pin
    /// that both agree on the same text.
    #[test]
    fn cosine_similarity_matches_trigram_similarity() {
        let a = trigram_counts("skill loader");
        let b = trigram_counts("skill reader");
        let public = cosine_similarity(&a, &b);
        assert!((public - trigram_similarity("skill loader", "skill reader")).abs() < 1e-6);
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
        assert_eq!(cosine_similarity(&a, &HashMap::new()), 0.0);
    }

    /// `foo` used to be reported ambiguous whenever `foobar` existed, so it
    /// could never be selected at all.
    #[test]
    fn resolve_skill_prefers_an_exact_match_over_substring_matches() {
        let skills = vec![skill("foo"), skill("foobar")];

        assert_eq!(resolve_skill("foo", &skills).unwrap(), "foo");
        assert_eq!(resolve_skill("FOO", &skills).unwrap(), "foo");
        assert!(resolve_skill("fo", &skills)
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
    }

    #[test]
    fn resolve_agent_prefers_an_exact_match_over_substring_matches() {
        let agents = vec![agent("foo"), agent("foobar")];

        assert_eq!(resolve_agent("foo", &agents).unwrap().name, "foo");
        assert!(resolve_agent("fo", &agents)
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
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
                "cursor",
                "marketplace",
                "cache",
                "agent"
            ]
        );
    }

    #[test]
    fn skill_roots_excludes_claude_when_env_disabled() {
        let _guard = env_guard();
        let _include = crate::test_support::set_env_var("SKRILLS_INCLUDE_CLAUDE", Some("0"));
        let roots = skill_roots(&[]).unwrap();
        let labels: Vec<_> = roots.iter().map(|r| r.source.label()).collect();
        assert!(
            !labels.contains(&"claude".to_string())
                && !labels.contains(&"marketplace".to_string())
                && !labels.contains(&"cache".to_string()),
            "Claude-derived roots should be excluded when SKRILLS_INCLUDE_CLAUDE=0"
        );
    }

    /// `/api/skills` serves Copilot skills from both paths; MCP resources
    /// must see the same roots.
    #[test]
    fn skill_roots_include_both_copilot_paths() {
        let _guard = env_guard();
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::test_support::set_env_var("HOME", temp.path().to_str());
        let roots = skill_roots(&[]).unwrap();
        let copilot: Vec<_> = roots
            .iter()
            .filter(|r| r.source == SkillSource::Copilot)
            .map(|r| r.root.clone())
            .collect();
        assert!(
            copilot.contains(&temp.path().join(".copilot/skills")),
            "{copilot:?}"
        );
        if let Some(config) = dirs::config_dir() {
            assert!(
                copilot.contains(&config.join("copilot/skills")),
                "{copilot:?}"
            );
        }
    }

    /// Cursor loads skills from `~/.cursor/skills`, where sync writes them;
    /// MCP discovery must list that root too.
    #[test]
    fn skill_roots_include_the_cursor_skills_dir() {
        let _guard = env_guard();
        let temp = tempfile::tempdir().unwrap();
        let _home = crate::test_support::set_env_var("HOME", temp.path().to_str());
        let roots = skill_roots(&[]).unwrap();
        assert!(
            roots
                .iter()
                .any(|r| r.source == SkillSource::Cursor
                    && r.root == temp.path().join(".cursor/skills")),
            "{roots:?}"
        );
    }

    /// A file of exactly `max` bytes is complete, and a cut must not split a
    /// UTF-8 character into U+FFFD.
    #[test]
    fn read_prefix_marks_only_real_truncation_on_a_char_boundary() {
        let temp = tempdir().unwrap();
        let exact = temp.path().join("exact.md");
        fs::write(&exact, "abcd").unwrap();
        assert_eq!(read_prefix(&exact, 4).unwrap(), "abcd");

        let long = temp.path().join("long.md");
        fs::write(&long, "abcé more").unwrap(); // é is two bytes at 3..5
        assert_eq!(read_prefix(&long, 4).unwrap(), "abc…");
        assert_eq!(read_prefix(&long, 5).unwrap(), "abcé…");
    }

    #[test]
    fn test_priority_labels_and_rank_map() {
        let (labels, rank_map) = priority_labels_and_rank_map();

        assert_eq!(labels.len(), 8);
        assert_eq!(rank_map.len(), 8);

        assert_eq!(rank_map.get("codex").unwrap(), 1);
        assert_eq!(rank_map.get("mirror").unwrap(), 2);
        assert_eq!(rank_map.get("claude").unwrap(), 3);
        assert_eq!(rank_map.get("copilot").unwrap(), 4);
        assert_eq!(rank_map.get("cursor").unwrap(), 5);
        assert_eq!(rank_map.get("marketplace").unwrap(), 6);
        assert_eq!(rank_map.get("cache").unwrap(), 7);
        assert_eq!(rank_map.get("agent").unwrap(), 8);
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
                frontmatter_name: None,
            },
            SkillMeta {
                name: "Baz".to_string(),
                path: PathBuf::from("/tmp/SKILL.md"),
                source: SkillSource::Codex,
                root: PathBuf::from("/tmp"),
                hash: "y".to_string(),
                description: None,
                frontmatter_name: None,
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
}
