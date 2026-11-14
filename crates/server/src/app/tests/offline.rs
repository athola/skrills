//! Offline mode: network-backed tools serve the newest cached answer with its
//! age, or say that no cached answer exists, and never reach the network.

use super::super::offline::{fetch_or_replay, remember, replay, Outcome, Replay, OFFLINE_NOTICE};
use super::super::*;
use serde_json::{json, Value};
use skrills_tome::cache::ResearchCache;
use std::cell::Cell;
use std::time::Duration;

fn temp_cache() -> (ResearchCache, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let cache = ResearchCache::open_at(&dir.path().join("t.db"), dir.path().join("pdfs")).unwrap();
    (cache, dir)
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn replay_without_an_entry_names_offline_mode_and_the_missing_cache() {
    let (cache, _dir) = temp_cache();
    let err = replay(Some(&cache), "search-papers", "rust")
        .unwrap_err()
        .to_string();
    assert!(err.contains("offline mode"), "{err}");
    assert!(err.contains("no cached data"), "{err}");
    assert!(err.contains("search-papers"), "{err}");
}

#[test]
fn replay_without_a_cache_database_reports_no_cached_data() {
    let err = replay(None, "resolve-doi", "10.1/x")
        .unwrap_err()
        .to_string();
    assert!(err.contains("no cached data"), "{err}");
}

#[test]
fn replay_returns_the_stored_value_with_its_age() {
    let (cache, _dir) = temp_cache();
    remember(
        Some(&cache),
        "search-papers",
        "rust",
        "rust",
        &json!({"count": 3}),
    );
    let hit = replay(Some(&cache), "search-papers", "rust").unwrap();
    assert_eq!(hit.value, json!({"count": 3}));
    assert!(hit.notice().starts_with(OFFLINE_NOTICE), "{}", hit.notice());
    assert!(hit.notice().contains("just now"), "{}", hit.notice());
}

#[test]
fn notice_reports_the_age_of_an_old_entry() {
    let two_days_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 2 * 86_400;
    let aged = Replay {
        value: json!({}),
        cached_at_unix: two_days_ago,
    };
    assert!(aged.notice().contains("2 days ago"), "{}", aged.notice());
}

#[test]
fn offline_fetch_serves_the_cache_and_never_calls_the_fetcher() {
    let (cache, _dir) = temp_cache();
    remember(
        Some(&cache),
        "search-discussions",
        "q",
        "q",
        &json!({"count": 2}),
    );
    let called = Cell::new(false);
    let outcome = block_on(fetch_or_replay(
        Some(&cache),
        true,
        "search-discussions",
        "q",
        "q",
        || {
            called.set(true);
            async { Ok(json!({})) }
        },
    ))
    .unwrap();
    assert!(!called.get(), "offline must not reach the network");
    match outcome {
        Outcome::Cached { value, notice } => {
            assert_eq!(value, json!({"count": 2}));
            assert!(notice.starts_with(OFFLINE_NOTICE), "{notice}");
        }
        Outcome::Live(_) => panic!("expected cached outcome"),
    }
}

#[test]
fn offline_fetch_without_an_entry_fails_without_calling_the_fetcher() {
    let (cache, _dir) = temp_cache();
    let called = Cell::new(false);
    let err = block_on(fetch_or_replay(
        Some(&cache),
        true,
        "resolve-doi",
        "d",
        "d",
        || {
            called.set(true);
            async { Ok(json!({})) }
        },
    ))
    .unwrap_err()
    .to_string();
    assert!(!called.get());
    assert!(
        err.contains("offline mode") && err.contains("no cached data"),
        "{err}"
    );
}

#[test]
fn online_fetch_returns_live_data_and_stores_it_for_later_offline_use() {
    let (cache, _dir) = temp_cache();
    let outcome = block_on(fetch_or_replay(
        Some(&cache),
        false,
        "resolve-doi",
        "d",
        "d",
        || async { Ok(json!({"title": "T"})) },
    ))
    .unwrap();
    assert!(matches!(outcome, Outcome::Live(ref v) if v["title"] == "T"));
    let offline = block_on(fetch_or_replay(
        Some(&cache),
        true,
        "resolve-doi",
        "d",
        "d",
        || async { Ok(json!({})) },
    ))
    .unwrap();
    assert!(matches!(offline, Outcome::Cached { ref value, .. } if value["title"] == "T"));
}

#[test]
fn online_fetch_error_propagates_and_stores_nothing() {
    let (cache, _dir) = temp_cache();
    let err = block_on(fetch_or_replay(
        Some(&cache),
        false,
        "resolve-doi",
        "d",
        "d",
        || async { Err::<Value, _>(anyhow::anyhow!("upstream 500")) },
    ))
    .unwrap_err()
    .to_string();
    assert!(err.contains("upstream 500"), "{err}");
    assert!(replay(Some(&cache), "resolve-doi", "d").is_err());
}

#[test]
fn online_fetch_works_without_a_cache_database() {
    let outcome = block_on(fetch_or_replay(
        None,
        false,
        "resolve-doi",
        "d",
        "d",
        || async { Ok(json!({"ok": true})) },
    ))
    .unwrap();
    assert!(matches!(outcome, Outcome::Live(_)));
}

// --- tool handlers ---------------------------------------------------------

struct OfflineHome {
    _guards: Vec<skrills_test_utils::EnvVarGuard>,
    _lock: std::sync::MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
}

/// Offline, with HOME and the cache base in a tempdir so the real cache is
/// never read or written.
fn offline_home() -> OfflineHome {
    let lock = crate::test_support::env_guard();
    let dir = tempfile::tempdir().unwrap();
    let cache_base = dir.path().join("cache");
    let guards = vec![
        crate::test_support::set_env_var("HOME", dir.path().to_str()),
        crate::test_support::set_env_var("XDG_CACHE_HOME", cache_base.to_str()),
        crate::test_support::set_env_var("LOCALAPPDATA", cache_base.to_str()),
        crate::test_support::set_env_var("SKRILLS_OFFLINE", Some("1")),
    ];
    OfflineHome {
        _guards: guards,
        _lock: lock,
        _dir: dir,
    }
}

fn seed(api: &str, key: &str, value: &Value) {
    let cache = ResearchCache::open().unwrap();
    remember(Some(&cache), api, key, key, value);
}

fn run_tool(tool: &str, args: Value) -> anyhow::Result<rmcp::model::CallToolResult> {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();
    let args = args.as_object().cloned().unwrap();
    block_on(async {
        match tool {
            "search-papers" => service.search_papers_tool(args).await,
            "search-discussions" => service.search_discussions_tool(args).await,
            "resolve-doi" => service.resolve_doi_tool(args).await,
            "fetch-pdf" => service.fetch_pdf_tool(args).await,
            _ => service.search_skills_github_tool(args).await,
        }
    })
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|b| b.as_text().map(|t| t.text.clone()))
        .collect()
}

#[test]
fn offline_tools_without_cache_fail_naming_offline_mode() {
    let _home = offline_home();
    for (tool, args) in [
        ("search-papers", json!({"query": "rust"})),
        ("search-discussions", json!({"query": "rust"})),
        ("resolve-doi", json!({"doi": "10.1/x"})),
        ("fetch-pdf", json!({"doi": "10.1/x"})),
        ("search-skills-github", json!({"query": "rust"})),
    ] {
        let err = run_tool(tool, args).unwrap_err().to_string();
        assert!(err.contains("offline mode"), "{tool}: {err}");
    }
}

#[test]
fn offline_search_papers_serves_cached_papers_with_the_notice() {
    let _home = offline_home();
    let stored = json!({"papers": [{"title": "P"}], "count": 1, "errors": []});
    seed(
        "search-papers",
        "rust|10|arxiv,semantic_scholar,openalex",
        &stored,
    );
    let result = run_tool("search-papers", json!({"query": "Rust"})).unwrap();
    assert!(
        text_of(&result).starts_with(OFFLINE_NOTICE),
        "{}",
        text_of(&result)
    );
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["papers"], stored["papers"]);
    assert_eq!(structured["offline"], true);
    assert!(structured["cached_at_unix"].as_u64().unwrap() > 0);
}

#[test]
fn offline_search_discussions_serves_cached_discussions_with_the_notice() {
    let _home = offline_home();
    seed(
        "search-discussions",
        "rust|10",
        &json!({"discussions": [], "count": 0}),
    );
    let result = run_tool("search-discussions", json!({"query": "rust"})).unwrap();
    assert!(
        text_of(&result).starts_with(OFFLINE_NOTICE),
        "{}",
        text_of(&result)
    );
    assert_eq!(result.structured_content.unwrap()["offline"], true);
}

#[test]
fn offline_resolve_doi_serves_cached_metadata_with_the_notice() {
    let _home = offline_home();
    seed(
        "resolve-doi",
        "10.1/x",
        &json!({"doi": "10.1/x", "title": "T", "year": 2020}),
    );
    let result = run_tool("resolve-doi", json!({"doi": "10.1/X"})).unwrap();
    assert!(
        text_of(&result).starts_with(OFFLINE_NOTICE),
        "{}",
        text_of(&result)
    );
    assert!(
        text_of(&result).contains("T (2020)"),
        "{}",
        text_of(&result)
    );
}

#[test]
fn offline_github_search_serves_cached_results_with_the_notice() {
    let _home = offline_home();
    let stored = json!([{
        "repo_url": "https://github.com/o/r", "skill_path": "skills/a/SKILL.md",
        "file_url": "https://github.com/o/r/blob/main/skills/a/SKILL.md",
        "description": null, "stars": 7, "last_updated": "2026-01-01", "raw_url": null
    }]);
    seed("search-skills-github", "rust|10", &stored);
    let result = run_tool("search-skills-github", json!({"query": "rust"})).unwrap();
    assert!(
        text_of(&result).starts_with(OFFLINE_NOTICE),
        "{}",
        text_of(&result)
    );
    assert!(
        text_of(&result).contains("skills/a/SKILL.md"),
        "{}",
        text_of(&result)
    );
    assert_eq!(result.structured_content.unwrap()["total_found"], 1);
}

#[test]
fn offline_fetch_pdf_refuses_to_download_an_uncached_pdf() {
    let _home = offline_home();
    let err = run_tool("fetch-pdf", json!({"doi": "10.1/none"}))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("offline mode") && err.contains("not cached"),
        "{err}"
    );
}
