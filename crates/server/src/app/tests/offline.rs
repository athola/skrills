//! Offline mode: network-backed tools serve the newest cached answer with its
//! age, or say that no cached answer exists, and never reach the network.

use super::super::offline::{
    cached_tool_result, fallback, fetch_or_replay, query_key, remember, replay, Fresh, Outcome,
    Replay, OFFLINE_NOTICE,
};
use super::super::*;
use serde_json::{json, Value};
use skrills_state::OfflineSource;
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
    let err = replay(Some(&cache), OfflineSource::Switch, "search-papers", "rust")
        .unwrap_err()
        .to_string();
    assert!(err.contains("offline mode"), "{err}");
    assert!(err.contains("no cached data"), "{err}");
    assert!(err.contains("search-papers"), "{err}");
}

/// A cache that failed to open is not the same as one with no entry.
#[test]
fn replay_without_a_cache_database_says_the_cache_could_not_be_opened() {
    let err = replay(None, OfflineSource::Switch, "resolve-doi", "10.1/x")
        .unwrap_err()
        .to_string();
    assert!(err.contains("could not be opened"), "{err}");
    assert!(!err.contains("no cached data"), "{err}");
}

/// `--offline` only sets the variable, so the message names both; a probe
/// verdict names the probe instead of telling the user to unset a variable
/// they never set.
#[test]
fn replay_error_names_what_turned_offline_mode_on() {
    let (cache, _dir) = temp_cache();
    let switch = replay(Some(&cache), OfflineSource::Switch, "resolve-doi", "d")
        .unwrap_err()
        .to_string();
    let probe = replay(Some(&cache), OfflineSource::Probe, "resolve-doi", "d")
        .unwrap_err()
        .to_string();
    assert!(switch.contains("SKRILLS_OFFLINE or --offline"), "{switch}");
    assert!(probe.contains("connectivity probe"), "{probe}");
    assert!(!probe.contains("unset SKRILLS_OFFLINE"), "{probe}");
}

#[test]
fn a_failed_live_fetch_falls_back_to_cached_answer() {
    let (cache, _dir) = temp_cache();
    remember(
        Some(&cache),
        "resolve-doi",
        "d",
        "d",
        &json!({"title": "T"}),
    );

    let outcome = block_on(fetch_or_replay(
        Some(&cache),
        None,
        "resolve-doi",
        "d",
        "d",
        || async { Err::<Fresh, _>(anyhow::anyhow!("upstream 500")) },
    ))
    .unwrap();

    let Outcome::Cached(hit) = outcome else {
        panic!("expected the cached answer");
    };
    assert_eq!(hit.value, json!({"title": "T"}));
    assert!(hit.notice().contains("upstream 500"), "{}", hit.notice());
    let rendered = cached_tool_result(hit, |_| String::new());
    let structured = rendered.structured_content.unwrap();
    assert_eq!(structured["offline"], false);
    assert!(structured["live_error"]
        .as_str()
        .unwrap()
        .contains("upstream 500"));
    assert!(structured["cached_at_unix"].as_u64().unwrap() > 0);
}

#[test]
fn fallback_without_a_stored_answer_is_none() {
    let (cache, _dir) = temp_cache();
    let error = anyhow::anyhow!("timeout");
    assert!(fallback(Some(&cache), "resolve-doi", "d", &error).is_none());
    assert!(fallback(None, "resolve-doi", "d", &error).is_none());
}

/// A transient failure inside a fetch (Unpaywall down) must not overwrite a
/// complete earlier answer, or offline use would replay the failure.
#[test]
fn a_partial_answer_is_returned_but_does_not_replace_the_stored_one() {
    let (cache, _dir) = temp_cache();
    remember(
        Some(&cache),
        "resolve-doi",
        "d",
        "d",
        &json!({"pdf_url": "https://x/p.pdf"}),
    );

    let outcome = block_on(fetch_or_replay(
        Some(&cache),
        None,
        "resolve-doi",
        "d",
        "d",
        || async { Ok(Fresh::Partial(json!({"pdf_url": null}))) },
    ))
    .unwrap();

    assert!(matches!(outcome, Outcome::Live(ref v) if v["pdf_url"].is_null()));
    let kept = replay(Some(&cache), OfflineSource::Switch, "resolve-doi", "d").unwrap();
    assert_eq!(kept.value["pdf_url"], "https://x/p.pdf");
}

/// A search with a failed source is kept only when nothing complete is
/// stored, so a transient outage never replaces a full answer offline.
#[test]
fn a_partial_paper_search_never_replaces_a_complete_stored_one() {
    use super::super::research::may_store_search_answer;
    let complete = json!({"papers": [], "count": 0, "errors": []});
    let partial = json!({"papers": [], "count": 0, "errors": ["arxiv: timeout"]});

    assert!(may_store_search_answer(false, Some(&complete)));
    assert!(!may_store_search_answer(true, Some(&complete)));
    assert!(may_store_search_answer(true, Some(&partial)));
    assert!(may_store_search_answer(true, None));
}

#[test]
fn query_key_ignores_case_and_surrounding_space() {
    assert_eq!(query_key("  Rust Async ", 10), "rust async|10");
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
    let hit = replay(Some(&cache), OfflineSource::Switch, "search-papers", "rust").unwrap();
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
        live_error: None,
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
        Some(OfflineSource::Switch),
        "search-discussions",
        "q",
        "q",
        || {
            called.set(true);
            async { Ok(Fresh::Complete(json!({}))) }
        },
    ))
    .unwrap();
    assert!(!called.get(), "offline must not reach the network");
    match outcome {
        Outcome::Cached(hit) => {
            assert_eq!(hit.value, json!({"count": 2}));
            assert!(hit.notice().starts_with(OFFLINE_NOTICE), "{}", hit.notice());
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
        Some(OfflineSource::Switch),
        "resolve-doi",
        "d",
        "d",
        || {
            called.set(true);
            async { Ok(Fresh::Complete(json!({}))) }
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
        None,
        "resolve-doi",
        "d",
        "d",
        || async { Ok(Fresh::Complete(json!({"title": "T"}))) },
    ))
    .unwrap();
    assert!(matches!(outcome, Outcome::Live(ref v) if v["title"] == "T"));
    let offline = block_on(fetch_or_replay(
        Some(&cache),
        Some(OfflineSource::Switch),
        "resolve-doi",
        "d",
        "d",
        || async { Ok(Fresh::Complete(json!({}))) },
    ))
    .unwrap();
    assert!(matches!(offline, Outcome::Cached(ref hit) if hit.value["title"] == "T"));
}

#[test]
fn online_fetch_error_propagates_and_stores_nothing() {
    let (cache, _dir) = temp_cache();
    let err = block_on(fetch_or_replay(
        Some(&cache),
        None,
        "resolve-doi",
        "d",
        "d",
        || async { Err::<Fresh, _>(anyhow::anyhow!("upstream 500")) },
    ))
    .unwrap_err()
    .to_string();
    assert!(err.contains("upstream 500"), "{err}");
    assert!(replay(Some(&cache), OfflineSource::Switch, "resolve-doi", "d").is_err());
}

#[test]
fn online_fetch_works_without_a_cache_database() {
    let outcome = block_on(fetch_or_replay(
        None,
        None,
        "resolve-doi",
        "d",
        "d",
        || async { Ok(Fresh::Complete(json!({"ok": true}))) },
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

#[test]
fn offline_fetch_pdf_serves_a_cached_pdf() {
    let _home = offline_home();
    let cache = ResearchCache::open().unwrap();
    let pdf = cache
        .pdf_dir()
        .join(super::super::research::pdf_cache_file_name("10.1/cached"));
    std::fs::create_dir_all(pdf.parent().unwrap()).unwrap();
    std::fs::write(&pdf, b"%PDF-1.7\n").unwrap();

    let served = run_tool("fetch-pdf", json!({"doi": "10.1/cached"})).unwrap();

    let structured = served.structured_content.unwrap();
    assert_eq!(structured["cached"], true);
    assert_eq!(structured["path"], pdf.to_string_lossy().as_ref());
}

#[test]
fn offline_cached_tool_results_keep_the_cache_time() {
    let _home = offline_home();
    seed(
        "search-discussions",
        "rust|10",
        &json!({"discussions": [], "count": 0}),
    );
    let served = run_tool("search-discussions", json!({"query": "rust"})).unwrap();
    assert!(served.structured_content.unwrap()["cached_at_unix"]
        .as_u64()
        .is_some_and(|at| at > 0));
}

fn run_create_skill(args: Value) -> rmcp::model::CallToolResult {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();
    service
        .create_skill_tool_sync(args.as_object().cloned().unwrap())
        .unwrap()
}

#[test]
fn offline_create_skill_github_search_serves_cache_with_notice() {
    let _home = offline_home();
    let stored = json!([{
        "repo_url": "https://github.com/o/r", "skill_path": "skills/a/SKILL.md",
        "file_url": "https://github.com/o/r/blob/main/skills/a/SKILL.md",
        "description": null, "stars": 7, "last_updated": "2026-01-01", "raw_url": null
    }]);
    seed(
        "search-skills-github",
        &query_key("demo a demo skill", 5),
        &stored,
    );

    let served = run_create_skill(json!({
        "name": "demo", "description": "a demo skill", "method": "github", "dry_run": true
    }));

    let text = text_of(&served);
    assert!(text.contains(OFFLINE_NOTICE), "{text}");
    let structured = served.structured_content.unwrap();
    assert_eq!(structured["github_results"].as_array().unwrap().len(), 1);
    assert!(structured["github_cache_notice"]
        .as_str()
        .unwrap()
        .starts_with(OFFLINE_NOTICE));
}

/// The LLM path calls a remote API and has nothing cached, so offline it is
/// refused with a reason instead of being attempted.
#[test]
fn offline_create_skill_with_llm_is_refused_naming_offline_mode() {
    let _home = offline_home();

    let served = run_create_skill(json!({
        "name": "demo", "description": "a demo skill", "method": "llm", "dry_run": true
    }));

    assert_eq!(served.is_error, Some(true));
    let text = text_of(&served);
    assert!(text.contains("offline mode"), "{text}");
    assert!(text.contains("SKRILLS_OFFLINE or --offline"), "{text}");
}

/// Every source failing is an error, and nothing is stored that offline
/// use would replay as the answer.
#[test]
fn search_papers_with_every_source_failing_caches_nothing() {
    let _home = offline_home();
    let _online = crate::test_support::set_env_var("SKRILLS_OFFLINE", Some("0"));

    let served = run_tool(
        "search-papers",
        json!({"query": "rust", "sources": ["bogus"]}),
    )
    .unwrap();

    assert_eq!(served.is_error, Some(true));
    let cache = ResearchCache::open().unwrap();
    assert_eq!(cache.get_latest("search-papers:rust|10|").unwrap(), None);
}
