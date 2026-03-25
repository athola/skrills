//! Browser integration tests for the web dashboard.
//!
//! Uses chromiumoxide to launch a headless Chrome instance and test
//! the full browser experience: HTML rendering, JavaScript execution,
//! CSS layout, and user interactions.

#![cfg(feature = "http-transport")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::Page;
use futures::StreamExt;
use serial_test::serial;

/// Returns true if a Chrome/Chromium binary is available on the system.
fn chrome_available() -> bool {
    use std::process::Command;
    // Try common Chrome binary names
    for bin in &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ] {
        if Command::new(bin)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return true;
        }
    }
    // Also check CHROME_PATH env var
    if let Ok(path) = std::env::var("CHROME_PATH") {
        return std::path::Path::new(&path).exists();
    }
    false
}

/// Skip the test at runtime if Chrome is not installed.
/// Returns true if the test should be skipped.
macro_rules! skip_without_chrome {
    () => {
        if !chrome_available() {
            eprintln!("Skipping: Chrome not found on this system");
            return;
        }
    };
}

/// Poll the page until `js_expr` (a JS expression returning a boolean) evaluates to `true`.
/// Returns `Ok(())` on success, or an error message on timeout.
async fn wait_for_element(page: &Page, selector: &str, timeout: Duration) -> anyhow::Result<()> {
    let js = format!("document.querySelector('{}') !== null", selector);
    let start = Instant::now();
    loop {
        let found: bool = page
            .evaluate(js.as_str())
            .await
            .map(|v| v.into_value().unwrap_or(false))
            .unwrap_or(false);
        if found {
            return Ok(());
        }
        if start.elapsed() > timeout {
            anyhow::bail!("Timeout waiting for element: {}", selector);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Poll the page until `js_expr` evaluates to `true`.
/// Use this for conditions more complex than simple element existence.
async fn wait_for_condition(
    page: &Page,
    js_expr: &str,
    description: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let start = Instant::now();
    loop {
        let met: bool = page
            .evaluate(js_expr)
            .await
            .map(|v| v.into_value().unwrap_or(false))
            .unwrap_or(false);
        if met {
            return Ok(());
        }
        if start.elapsed() > timeout {
            anyhow::bail!("Timeout waiting for condition: {}", description);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

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
            false,
        )
        .await;
    });

    // Poll until the server is accepting connections (up to 10s)
    let client = reqwest::Client::new();
    let start = Instant::now();
    let server_timeout = Duration::from_secs(10);
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if client.get(&base_url).send().await.is_ok() {
            break;
        }
        if start.elapsed() > server_timeout {
            panic!(
                "Timeout: test server at {} did not become ready within {:?}",
                base_url, server_timeout
            );
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
async fn browser_loads_dashboard_page() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_element(&page, "header h1", Duration::from_secs(5))
        .await
        .expect("header h1 should appear after page load");

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
async fn browser_renders_three_panels() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_element(&page, ".skills-panel", Duration::from_secs(5))
        .await
        .expect("skills-panel should appear after page load");

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
async fn browser_fetches_and_renders_skills() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    // Wait for JS to fetch skills and render them into the DOM
    wait_for_condition(
        &page,
        "document.querySelectorAll('.skill-item').length === 3",
        "3 skill items to be rendered",
        Duration::from_secs(5),
    )
    .await
    .expect("skill items should be rendered after data fetch");

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
async fn browser_updates_last_update_timestamp() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    // Wait until the last-update element is populated (changes from initial '-')
    wait_for_condition(
        &page,
        "(document.getElementById('last-update')?.textContent || '-') !== '-'",
        "last-update timestamp to be populated",
        Duration::from_secs(5),
    )
    .await
    .expect("last-update element should be updated after data fetch");

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
async fn browser_click_skill_shows_details() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_element(&page, ".skill-item", Duration::from_secs(5))
        .await
        .expect("at least one skill item should appear before clicking");

    // Click the first skill item
    page.evaluate("document.querySelector('.skill-item')?.click()")
        .await
        .unwrap();

    // Wait for the metrics panel to update after the click
    wait_for_condition(
        &page,
        "!(document.getElementById('metrics-content')?.textContent || '').includes('Select a skill')",
        "metrics panel to show skill details after click",
        Duration::from_secs(5),
    )
    .await
    .expect("metrics panel should update after clicking a skill");

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
async fn browser_css_loaded_and_applied() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    // Wait until CSS is loaded: body background should not be transparent
    wait_for_condition(
        &page,
        "getComputedStyle(document.body).backgroundColor !== 'rgba(0, 0, 0, 0)'",
        "CSS to load and apply background color",
        Duration::from_secs(5),
    )
    .await
    .expect("CSS should be loaded and applied");

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
async fn browser_panels_have_correct_layout() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_condition(
        &page,
        "document.querySelectorAll('.panel').length === 3",
        "3 panel elements to be present",
        Duration::from_secs(5),
    )
    .await
    .expect("all 3 panel elements should appear");

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
async fn browser_empty_skills_shows_no_skills_found() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    // Don't create any skills

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    // Wait for the empty state to render (skill-count shows "0")
    wait_for_condition(
        &page,
        "(document.getElementById('skill-count')?.textContent || '') === '0'",
        "skill count to show '0' for empty state",
        Duration::from_secs(5),
    )
    .await
    .expect("skill count should show 0 when no skills exist");

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
async fn browser_header_shows_stats() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_element(&page, ".stats", Duration::from_secs(5))
        .await
        .expect("stats element should appear in header");

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
async fn browser_footer_exists() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_element(&page, "footer", Duration::from_secs(5))
        .await
        .expect("footer element should appear after page load");

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
async fn browser_metrics_panel_shows_select_prompt() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_element(&page, "#metrics-content", Duration::from_secs(5))
        .await
        .expect("metrics-content element should appear");

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
async fn browser_skill_items_show_source() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_condition(
        &page,
        "document.querySelectorAll('.skill-source').length === 3",
        "3 skill-source elements to be rendered",
        Duration::from_secs(5),
    )
    .await
    .expect("all skill source elements should appear after data fetch");

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
async fn browser_sort_button_toggles_order() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    // Wait for skills to load so the sort button is functional
    wait_for_element(&page, "#sort-btn", Duration::from_secs(5))
        .await
        .expect("sort button should appear");
    wait_for_condition(
        &page,
        "document.querySelectorAll('.skill-item').length === 3",
        "3 skill items to be rendered before sorting",
        Duration::from_secs(5),
    )
    .await
    .expect("skill items should be rendered before testing sort");

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
    wait_for_condition(
        &page,
        "(document.getElementById('sort-btn')?.textContent || '') === 'Sort: A-Z'",
        "sort button text to change to 'Sort: A-Z'",
        Duration::from_secs(5),
    )
    .await
    .expect("sort button should update after click");

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
    wait_for_condition(
        &page,
        "(document.getElementById('sort-btn')?.textContent || '') === 'Sort: Discovery'",
        "sort button text to change back to 'Sort: Discovery'",
        Duration::from_secs(5),
    )
    .await
    .expect("sort button should toggle back after second click");

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
async fn browser_has_scroll_sentinel() {
    skip_without_chrome!();
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let (base_url, server_handle) = start_test_server(vec![tmp.path().to_path_buf()]).await;
    let (browser, browser_handle) = launch_browser().await;

    let page = browser.new_page(&base_url).await.unwrap();
    wait_for_element(&page, "#skill-sentinel", Duration::from_secs(5))
        .await
        .expect("scroll sentinel element should appear after skills load");

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
