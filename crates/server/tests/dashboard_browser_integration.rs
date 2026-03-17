//! Browser integration tests for the web dashboard.
//!
//! Uses chromiumoxide to launch a headless Chrome instance and test
//! the full browser experience: HTML rendering, JavaScript execution,
//! CSS layout, and user interactions.

#![cfg(feature = "http-transport")]

use std::path::PathBuf;
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use serial_test::serial;

/// Create a test skill directory with sample SKILL.md files.
fn create_test_skills(dir: &std::path::Path) {
    for name in &["alpha-skill", "beta-skill", "gamma-skill"] {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {}\ndescription: A test skill called {}\n---\n\n# {}\n\nTest content for {}.",
                name, name, name, name
            ),
        )
        .unwrap();
    }
}

/// Start the HTTP server in the background and return the base URL.
async fn start_test_server(skill_dirs: Vec<PathBuf>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("should bind to ephemeral port");
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    drop(listener);

    let bind = format!("127.0.0.1:{}", port);
    let base_url = format!("http://127.0.0.1:{}", port);

    let skill_dirs_for_api = skill_dirs.clone();
    let handle = tokio::spawn(async move {
        let dirs = skill_dirs.clone();
        let _ = skrills_server::http_transport::serve_http_with_security(
            move || {
                skrills_server::app::SkillService::new_with_ttl(
                    dirs.clone(),
                    Duration::from_secs(60),
                )
                .map_err(std::io::Error::other)
            },
            &bind,
            skrills_server::http_transport::HttpSecurityConfig::default(),
            skill_dirs_for_api,
        )
        .await;
    });

    // Wait for server to be ready
    let client = reqwest::Client::new();
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if client.get(&base_url).send().await.is_ok() {
            break;
        }
    }

    (base_url, handle)
}

/// Launch a headless Chrome browser and return the browser + handler task.
async fn launch_browser() -> (Browser, tokio::task::JoinHandle<()>) {
    let config = BrowserConfig::builder()
        .no_sandbox()
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--disable-dev-shm-usage")
        .build()
        .expect("Failed to build browser config");

    let (browser, mut handler) = Browser::launch(config)
        .await
        .expect("Failed to launch Chrome — is Google Chrome installed?");

    let handle = tokio::spawn(async move { while handler.next().await.is_some() {} });

    (browser, handle)
}

// ── Dashboard Loading Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_loads_dashboard_page() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Check page title
    let title = page.get_title().await.unwrap().unwrap_or_default();
    assert_eq!(
        title, "Skrills Dashboard",
        "Page title should be 'Skrills Dashboard'"
    );

    // Check header exists
    let header_text: String = page
        .evaluate("document.querySelector('header h1')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();
    assert_eq!(header_text, "Skrills Dashboard");

    server_handle.abort();
    browser_handle.abort();
}

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_renders_three_panels() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    let skills_panel: bool = page
        .evaluate("document.querySelector('.skills-panel') !== null")
        .await
        .unwrap()
        .into_value()
        .unwrap();
    assert!(skills_panel, "Skills panel should exist");

    let activity_panel: bool = page
        .evaluate("document.querySelector('.activity-panel') !== null")
        .await
        .unwrap()
        .into_value()
        .unwrap();
    assert!(activity_panel, "Activity panel should exist");

    let metrics_panel: bool = page
        .evaluate("document.querySelector('.metrics-panel') !== null")
        .await
        .unwrap()
        .into_value()
        .unwrap();
    assert!(metrics_panel, "Metrics panel should exist");

    server_handle.abort();
    browser_handle.abort();
}

// ── JavaScript Data Fetching Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_fetches_and_renders_skills() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    // Give JS time to fetch and render
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Check skill count in header
    let skill_count: String = page
        .evaluate("document.getElementById('skill-count')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();
    assert_eq!(skill_count, "3", "Skill count should be 3");

    // Check skill items rendered in the list
    let skill_items_count: i64 = page
        .evaluate("document.querySelectorAll('.skill-item').length")
        .await
        .unwrap()
        .into_value()
        .unwrap_or(0);
    assert_eq!(skill_items_count, 3, "Should render 3 skill items");

    // Verify skill names are present
    let skill_names: Vec<String> = page
        .evaluate("Array.from(document.querySelectorAll('.skill-name')).map(el => el.textContent)")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();

    // Skill names from discovery include path suffix (e.g. "alpha-skill/SKILL.md")
    assert!(
        skill_names.iter().any(|n| n.contains("alpha-skill")),
        "Should contain alpha-skill, got: {:?}",
        skill_names
    );
    assert!(
        skill_names.iter().any(|n| n.contains("beta-skill")),
        "Should contain beta-skill, got: {:?}",
        skill_names
    );
    assert!(
        skill_names.iter().any(|n| n.contains("gamma-skill")),
        "Should contain gamma-skill, got: {:?}",
        skill_names
    );

    server_handle.abort();
    browser_handle.abort();
}

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_updates_last_update_timestamp() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let last_update: String = page
        .evaluate("document.getElementById('last-update')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();

    assert_ne!(
        last_update, "-",
        "Last update should be updated from initial '-'"
    );
    assert!(!last_update.is_empty(), "Last update should not be empty");

    server_handle.abort();
    browser_handle.abort();
}

// ── Skill Selection / Click Interaction Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_click_skill_shows_details() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Click the first skill item
    page.evaluate("document.querySelector('.skill-item')?.click()")
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check that metrics panel now shows skill details
    let metrics_content: String = page
        .evaluate("document.getElementById('metrics-content')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();

    assert!(
        !metrics_content.contains("Select a skill to view details"),
        "Metrics panel should no longer show placeholder after clicking a skill"
    );

    // Should show some skill detail (name, path, source)
    assert!(
        metrics_content.contains("Path") || metrics_content.contains("Source"),
        "Metrics panel should show skill details, got: {}",
        metrics_content
    );

    server_handle.abort();
    browser_handle.abort();
}

// ── CSS Layout Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_css_loaded_and_applied() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify CSS is loaded by checking computed styles
    let bg_color: String = page
        .evaluate("getComputedStyle(document.body).backgroundColor")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();

    // CSS sets --bg-primary: #1a1a2e which renders as rgb(26, 26, 46)
    assert!(
        !bg_color.is_empty() && bg_color != "rgba(0, 0, 0, 0)",
        "Body should have a non-transparent background color (CSS loaded). Got: {}",
        bg_color
    );

    // Check grid layout on main element
    let main_display: String = page
        .evaluate("getComputedStyle(document.querySelector('main')).display")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();

    assert_eq!(main_display, "grid", "Main element should use grid layout");

    server_handle.abort();
    browser_handle.abort();
}

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_panels_have_correct_layout() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Check that all 3 panels are visible
    let panel_count: i64 = page
        .evaluate("document.querySelectorAll('.panel').length")
        .await
        .unwrap()
        .into_value()
        .unwrap_or(0);
    assert_eq!(panel_count, 3, "Should have 3 panel elements");

    // Check panels have non-zero dimensions
    let panels_visible: bool = page
        .evaluate(
            "Array.from(document.querySelectorAll('.panel')).every(p => p.offsetWidth > 0 && p.offsetHeight > 0)",
        )
        .await
        .unwrap()
        .into_value()
        .unwrap_or(false);
    assert!(panels_visible, "All panels should have non-zero dimensions");

    server_handle.abort();
    browser_handle.abort();
}

// ── Empty State Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_empty_skills_shows_no_skills_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Don't create any skills

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let skill_count: String = page
        .evaluate("document.getElementById('skill-count')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();
    assert_eq!(skill_count, "0", "Skill count should be 0");

    // Check for empty state message
    let empty_msg: String = page
        .evaluate("document.querySelector('.skill-list .empty')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();
    assert_eq!(
        empty_msg, "No skills found",
        "Should show 'No skills found' when no skills exist"
    );

    server_handle.abort();
    browser_handle.abort();
}

// ── Header Stats Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_header_shows_stats() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let header_text: String = page
        .evaluate("document.querySelector('.stats')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();

    assert!(
        header_text.contains("Skills:"),
        "Header should show skills label"
    );
    assert!(
        header_text.contains("Events:"),
        "Header should show events label"
    );

    server_handle.abort();
    browser_handle.abort();
}

// ── Footer Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_footer_exists() {
    let tmp = tempfile::TempDir::new().unwrap();

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;

    let footer_exists: bool = page
        .evaluate("document.querySelector('footer') !== null")
        .await
        .unwrap()
        .into_value()
        .unwrap();
    assert!(footer_exists, "Footer should exist");

    server_handle.abort();
    browser_handle.abort();
}

// ── Metrics Panel Default State ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_metrics_panel_shows_select_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let metrics_text: String = page
        .evaluate("document.getElementById('metrics-content')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();

    assert!(
        metrics_text.contains("Select a skill to view details"),
        "Metrics panel should prompt user to select a skill"
    );

    server_handle.abort();
    browser_handle.abort();
}

// ── Skill Source Display Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_skill_items_show_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Each skill item should have a source span
    let source_count: i64 = page
        .evaluate("document.querySelectorAll('.skill-source').length")
        .await
        .unwrap()
        .into_value()
        .unwrap_or(0);

    assert_eq!(
        source_count, 3,
        "Each skill item should have a source element"
    );

    server_handle.abort();
    browser_handle.abort();
}

// ── Sort Toggle Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_sort_button_toggles_order() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Default sort label
    let sort_text: String = page
        .evaluate("document.getElementById('sort-btn')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();
    assert_eq!(
        sort_text, "Sort: Discovery",
        "Sort button should default to 'Sort: Discovery'"
    );

    // Click to toggle to alphabetical
    page.evaluate("document.getElementById('sort-btn')?.click()")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let sort_text: String = page
        .evaluate("document.getElementById('sort-btn')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();
    assert_eq!(
        sort_text, "Sort: A-Z",
        "Sort button should change to 'Sort: A-Z' after click"
    );

    // Verify skills are in alphabetical order
    let skill_names: Vec<String> = page
        .evaluate("Array.from(document.querySelectorAll('.skill-name')).map(el => el.textContent)")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();
    assert!(
        skill_names.len() >= 2,
        "Should have at least 2 skills to verify order"
    );
    let mut sorted = skill_names.clone();
    sorted.sort();
    assert_eq!(
        skill_names, sorted,
        "Skills should be in alphabetical order after sort toggle"
    );

    // Click again to toggle back to discovery
    page.evaluate("document.getElementById('sort-btn')?.click()")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let sort_text: String = page
        .evaluate("document.getElementById('sort-btn')?.textContent || ''")
        .await
        .unwrap()
        .into_value()
        .unwrap_or_default();
    assert_eq!(
        sort_text, "Sort: Discovery",
        "Sort button should toggle back to 'Sort: Discovery'"
    );

    server_handle.abort();
    browser_handle.abort();
}

// ── Infinite Scroll Sentinel Tests ──

#[tokio::test]
#[serial]
#[ignore = "requires Chrome installed"]
async fn browser_has_scroll_sentinel() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;

    let sentinel_exists: bool = page
        .evaluate("document.getElementById('skill-sentinel') !== null")
        .await
        .unwrap()
        .into_value()
        .unwrap_or(false);
    assert!(
        sentinel_exists,
        "Skill list should contain an IntersectionObserver sentinel element"
    );

    server_handle.abort();
    browser_handle.abort();
}
