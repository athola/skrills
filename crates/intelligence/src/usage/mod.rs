//! Usage analytics from Claude Code and Codex CLI session data.

mod analytics;
pub mod behavioral;
mod claude_parser;
mod codex_parser;

pub use analytics::{build_analytics, get_cooccurring_skills, recency_score};
pub use behavioral::{
    build_behavioral_patterns, detect_session_outcome, extract_common_ngrams,
    extract_file_accesses, extract_tool_calls, BehavioralEvent, BehavioralPatterns, FileAccess,
    FileOperation, OutcomeStatus, SessionOutcome, SkillUsageEventData, ToolCall, ToolStatus,
};
pub use claude_parser::{parse_claude_command_history, parse_claude_sessions};
pub use codex_parser::{
    parse_codex_command_history, parse_codex_sessions, parse_codex_skills_history,
};

use crate::types::Confidence;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Aggregated skill usage statistics from session history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageAnalytics {
    /// Skill usage frequency (skill_path -> count).
    pub frequency: HashMap<String, u64>,
    /// Skill recency scores (skill_path -> last_used_timestamp).
    pub recency: HashMap<String, u64>,
    /// Skills commonly used together (skill_a -> (skill_b -> co-occurrence_count)).
    pub cooccurrence: HashMap<String, HashMap<String, u64>>,
    /// Prompt text -> skill affinities (for semantic matching).
    pub prompt_affinities: Vec<PromptAffinity>,
    /// User command history entries.
    pub command_history: Vec<CommandEntry>,
    /// Total sessions analyzed.
    pub sessions_analyzed: usize,
    /// Time range of analyzed data.
    pub time_range: Option<TimeRange>,
}

/// Prompt-to-skill affinity mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptAffinity {
    /// Keywords/trigrams extracted from prompts.
    pub keywords: Vec<String>,
    /// Skills invoked after this prompt type.
    pub associated_skills: Vec<String>,
    /// Confidence score (0.0 - 1.0).
    pub confidence: Confidence,
}

/// A user command entered in the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEntry {
    /// The command text entered by the user.
    pub text: String,
    /// Unix timestamp when command was entered.
    pub timestamp: u64,
    /// Session ID this command belongs to.
    pub session_id: String,
    /// Project path if available.
    pub project: Option<String>,
}

/// Time range for analytics data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: u64,
    pub end: u64,
}

/// A single skill usage event extracted from session data.
#[derive(Debug, Clone)]
pub struct SkillUsageEvent {
    /// Unix timestamp of the event.
    pub timestamp: u64,
    /// Skill path or URI.
    pub skill_path: String,
    /// Session ID this event belongs to.
    pub session_id: String,
    /// Prompt context that led to this skill being used.
    pub prompt_context: Option<String>,
}

// ============================================================================
// Persistence Functions
// ============================================================================

/// Default path for analytics cache file.
///
/// Returns `~/.skrills/analytics_cache.json` or `None` if home dir unavailable.
pub fn default_analytics_cache_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".skrills").join("analytics_cache.json"))
}

/// Save usage analytics to a JSON file.
///
/// Creates parent directories if they don't exist.
///
/// # Errors
/// Returns error if file cannot be written or serialization fails.
pub fn save_analytics(analytics: &UsageAnalytics, path: &std::path::Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(analytics)?;
    // Write a sibling temp file and rename it over the cache, so a crash
    // mid-write leaves the old cache (or none), never a truncated one.
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".tmp.{}", std::process::id()));
    let tmp_path = path.with_file_name(tmp_name);
    if let Err(e) = std::fs::write(&tmp_path, json).and_then(|()| std::fs::rename(&tmp_path, path))
    {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }
    Ok(())
}

/// A cache older than this is rebuilt even if no session file changed.
const ANALYTICS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Whether the cache at `cache_path` can be reused: younger than `ttl` and
/// newer than every `*.jsonl` session file under `session_dirs`.
///
/// Only file metadata is read, which is far cheaper than re-parsing.
fn analytics_cache_is_fresh(
    cache_path: &std::path::Path,
    session_dirs: &[std::path::PathBuf],
    ttl: std::time::Duration,
) -> bool {
    let Ok(cache_mtime) = std::fs::metadata(cache_path).and_then(|m| m.modified()) else {
        return false;
    };
    match cache_mtime.elapsed() {
        Ok(age) if age > ttl => return false,
        _ => {}
    }
    let newer_session_exists = session_dirs.iter().any(|dir| {
        walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .any(|e| {
                e.metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .is_some_and(|mtime| mtime > cache_mtime)
            })
    });
    !newer_session_exists
}

/// Load usage analytics from a JSON file.
///
/// Returns `None` if file doesn't exist.
///
/// # Errors
/// Returns error if file cannot be read or deserialization fails.
pub fn load_analytics(path: &std::path::Path) -> anyhow::Result<Option<UsageAnalytics>> {
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(path)?;
    let analytics: UsageAnalytics = serde_json::from_str(&json)?;
    Ok(Some(analytics))
}

/// Load analytics from default cache location, or build fresh if not cached.
///
/// This is the recommended entry point for consumers that want cached analytics.
///
/// # Arguments
/// * `force_rebuild` - If true, ignores cache and rebuilds from session data
/// * `auto_save` - If true, saves rebuilt analytics to cache
pub fn load_or_build_analytics(
    force_rebuild: bool,
    auto_save: bool,
) -> anyhow::Result<UsageAnalytics> {
    let cache_path = default_analytics_cache_path();
    let home = dirs::home_dir().ok_or(crate::IntelligenceError::HomeDirectoryNotFound)?;

    let claude_projects = home.join(".claude").join("projects");
    let codex_sessions = home.join(".codex").join("sessions");

    // Reuse the cache (unless force_rebuild) only while it is fresh: younger
    // than the TTL and newer than every session file it was built from.
    if !force_rebuild {
        if let Some(ref path) = cache_path {
            let sources = [claude_projects.clone(), codex_sessions.clone()];
            if analytics_cache_is_fresh(path, &sources, ANALYTICS_CACHE_TTL) {
                match load_analytics(path) {
                    Ok(Some(cached)) => return Ok(cached),
                    Ok(None) => {}
                    Err(e) => tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Analytics cache unreadable; rebuilding"
                    ),
                }
            }
        }
    }

    let claude_events = parse_claude_sessions(&claude_projects)?;
    let codex_events = parse_codex_sessions(&codex_sessions)?;

    let mut all_events = claude_events;
    all_events.extend(codex_events);

    let analytics = build_analytics(all_events);

    // Auto-save if requested
    if auto_save {
        if let Some(ref path) = cache_path {
            if let Err(e) = save_analytics(&analytics, path) {
                tracing::warn!(path = %path.display(), error = %e, "Failed to save analytics cache");
            }
        }
    }

    Ok(analytics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_analytics_cache_path() {
        // Should return Some path when HOME is available
        let path = default_analytics_cache_path();
        if let Some(p) = path {
            assert!(p.ends_with("analytics_cache.json"));
            assert!(p.to_string_lossy().contains(".skrills"));
        }
        // Note: test passes even if HOME unavailable (returns None)
    }

    #[test]
    fn test_save_and_load_analytics_roundtrip() {
        let temp = tempdir().unwrap();
        let cache_path = temp.path().join("analytics_cache.json");

        // Create sample analytics
        let mut analytics = UsageAnalytics {
            sessions_analyzed: 42,
            ..Default::default()
        };
        analytics.frequency.insert("test-skill".to_string(), 10);

        // Save
        save_analytics(&analytics, &cache_path).unwrap();
        assert!(cache_path.exists());

        // Load
        let loaded = load_analytics(&cache_path).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.sessions_analyzed, 42);
        assert_eq!(loaded.frequency.get("test-skill"), Some(&10));
    }

    #[test]
    fn test_load_analytics_missing_file() {
        let temp = tempdir().unwrap();
        let missing_path = temp.path().join("nonexistent.json");

        // Should return None for missing file, not error
        let result = load_analytics(&missing_path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_save_analytics_creates_parent_dirs() {
        let temp = tempdir().unwrap();
        let nested_path = temp
            .path()
            .join("deep")
            .join("nested")
            .join("analytics.json");

        let analytics = UsageAnalytics::default();
        save_analytics(&analytics, &nested_path).unwrap();
        assert!(nested_path.exists());
    }

    #[test]
    fn test_load_analytics_invalid_json() {
        let temp = tempdir().unwrap();
        let bad_path = temp.path().join("bad.json");
        std::fs::write(&bad_path, "not valid json {{{").unwrap();

        // Should return error for invalid JSON
        let result = load_analytics(&bad_path);
        assert!(result.is_err());
    }

    fn set_mtime(path: &std::path::Path, ago: std::time::Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(std::time::SystemTime::now() - ago)
            .unwrap();
    }

    /// IN-11: a cache is reused only while it is newer than every session
    /// file and younger than the TTL.
    #[test]
    fn analytics_cache_freshness_tracks_sessions_and_ttl() {
        let temp = tempdir().unwrap();
        let sessions = temp.path().join("projects").join("p");
        std::fs::create_dir_all(&sessions).unwrap();
        let session = sessions.join("s.jsonl");
        std::fs::write(&session, "").unwrap();
        let cache = temp.path().join("analytics_cache.json");
        std::fs::write(&cache, "{}").unwrap();
        let dirs = [temp.path().join("projects")];
        let ttl = std::time::Duration::from_secs(3600);
        let minute = std::time::Duration::from_secs(60);

        set_mtime(&session, minute * 10);
        set_mtime(&cache, minute);
        assert!(analytics_cache_is_fresh(&cache, &dirs, ttl));

        // A session written after the cache makes it stale.
        set_mtime(&session, std::time::Duration::ZERO);
        assert!(!analytics_cache_is_fresh(&cache, &dirs, ttl));

        // So does age alone.
        set_mtime(&session, minute * 600);
        set_mtime(&cache, minute * 120);
        assert!(!analytics_cache_is_fresh(&cache, &dirs, ttl));

        // No cache, nothing to reuse.
        assert!(!analytics_cache_is_fresh(
            &temp.path().join("missing.json"),
            &dirs,
            ttl
        ));
    }

    /// IN-11 end to end: with a session newer than the cache,
    /// `load_or_build_analytics(false, ..)` rebuilds instead of returning
    /// the stale snapshot.
    #[test]
    fn load_or_build_analytics_rebuilds_stale_cache() {
        let _g = crate::test_support::env_guard();
        let home = tempdir().unwrap();
        let _home = crate::test_support::set_env_var("HOME", home.path().to_str());

        let stale = UsageAnalytics {
            sessions_analyzed: 999,
            ..Default::default()
        };
        let cache = home.path().join(".skrills").join("analytics_cache.json");
        save_analytics(&stale, &cache).unwrap();
        set_mtime(&cache, std::time::Duration::from_secs(600));

        let project = home.path().join(".claude").join("projects").join("p");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("s.jsonl"),
            r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"fresh"}}]}}"#,
        )
        .unwrap();

        let analytics = load_or_build_analytics(false, false).unwrap();
        assert_ne!(analytics.sessions_analyzed, 999, "stale cache was reused");
        assert!(analytics.frequency.contains_key("fresh"));

        // With the cache newer than every session, it is reused.
        let cached = UsageAnalytics {
            sessions_analyzed: 7,
            ..Default::default()
        };
        save_analytics(&cached, &cache).unwrap();
        assert_eq!(
            load_or_build_analytics(false, false)
                .unwrap()
                .sessions_analyzed,
            7
        );
    }

    /// IN-31: the cache is replaced via a temp file and rename; nothing is
    /// left beside it.
    #[test]
    fn save_analytics_leaves_no_temp_file() {
        let temp = tempdir().unwrap();
        let cache = temp.path().join("analytics_cache.json");
        std::fs::write(&cache, "old").unwrap();
        save_analytics(&UsageAnalytics::default(), &cache).unwrap();
        let names: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            names,
            vec![std::ffi::OsString::from("analytics_cache.json")]
        );
        assert!(load_analytics(&cache).unwrap().is_some());
    }
}
