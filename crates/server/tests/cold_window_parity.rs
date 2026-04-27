//! Cold-window TUI ↔ browser parity test (TASK-023).
//!
//! Verifies SC4 (cold-window-spec § 6): the TUI render surface and the
//! browser render surface emit the same semantic content for a fixed
//! `WindowSnapshot`. "Semantic" means user-visible text — alert
//! titles + messages, hint URIs + messages, research titles, and
//! status-bar fields. Presentation differences (ANSI colors vs CSS
//! classes, ratatui borders vs HTML `<ul>`) are intentionally ignored.
//!
//! ## Plan deviation
//!
//! `docs/cold-window-plan.md` § 2 lists this test at
//! `crates/test-utils/tests/parity.rs`. Putting it there would force
//! `skrills_test_utils` to dev-depend on `skrills-server`,
//! `skrills-dashboard`, `ratatui`, and `reqwest`. Two of those
//! (`server` and `dashboard`) already dev-depend on `skrills_test_utils`,
//! so the original layout creates a dependency cycle. This file lives
//! in `skrills-server`'s tests instead, where every required dev-dep
//! already exists. The semantic guarantee — SC4 — is identical.
//!
//! ## Test strategy
//!
//! 1. Build a fixture snapshot with at least one alert, one hint, one
//!    research finding, and a non-trivial status (cadence, tokens,
//!    quota).
//! 2. **TUI render**: feed the snapshot into `ColdWindowState`, render
//!    every pane onto a `ratatui::backend::TestBackend` buffer, and
//!    flatten the buffer to a single string.
//! 3. **Browser render**: bind an ephemeral-port `axum::serve`, issue a
//!    `reqwest` GET to `/dashboard.sse`, send the snapshot via the
//!    broadcast bus once the handler subscribes, and read SSE bytes
//!    until all four named events arrive (or a 2 s timeout). Strip
//!    HTML tags + decode entities to recover plain text.
//! 4. Assert each user-visible string from the snapshot appears in
//!    both flattened texts.

#![cfg(feature = "http-transport")]

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Terminal;
use skrills_dashboard::cold_window::{
    AlertPane, ColdWindowState, HintPane, HintPaneState, ResearchPane, ResearchPaneState, StatusBar,
};
use skrills_server::api::{cold_window_routes, ColdWindowDashboardState};
use skrills_snapshot::{
    Alert, AlertBand, Hint, HintCategory, LoadSample, ResearchChannel, ResearchFinding, ScoredHint,
    Severity, TokenEntry, TokenLedger, WindowSnapshot,
};
use tokio::sync::broadcast;

/// Token-budget ceiling shared by both surfaces. The spec § 3.4
/// "Caution" tier example uses 50K so the snapshot's 25K total
/// renders as 50% — far enough below the 80% bar threshold that the
/// status bar should NOT show the warn class, and far enough above 0
/// that the bar isn't empty.
const BUDGET: u64 = 50_000;

/// Build a richer fixture than `cold_window_fixtures::standard_snapshot`:
/// the standard fixture has no alerts and no research findings, which
/// would give us a parity test that vacuously passes. We need at least
/// one of every user-visible record type to exercise SC4.
fn parity_snapshot() -> WindowSnapshot {
    WindowSnapshot {
        version: 7,
        timestamp_ms: 1_700_000_000_000,
        token_ledger: TokenLedger {
            per_skill: vec![],
            per_plugin: vec![],
            per_mcp: vec![TokenEntry {
                source: "mcp://github".into(),
                tokens: 25_000,
            }],
            conversation_cache_reads: 0,
            conversation_cache_writes: 0,
            total: 25_000,
        },
        alerts: vec![Alert {
            fingerprint: "alert-token-budget".into(),
            severity: Severity::Caution,
            title: "TokenBudgetApproaching".into(),
            message: "github MCP source consumes half the budget".into(),
            band: Some(AlertBand {
                low: 0.0,
                low_clear: 0.0,
                high: 0.5,
                high_clear: 0.45,
            }),
            fired_at_ms: 1_700_000_000_000,
            dwell_ticks: 3,
        }],
        hints: vec![ScoredHint {
            hint: Hint {
                uri: "skill://demo-redundant".into(),
                category: HintCategory::Redundancy,
                message: "ManifestValidationDuplicate".into(),
                frequency: 5,
                impact: 7.0,
                ease_score: 4.0,
                age_days: 2.0,
            },
            score: 12.5,
            pinned: false,
        }],
        research_findings: vec![ResearchFinding {
            fingerprint: "rf-1".into(),
            channel: ResearchChannel::HackerNews,
            title: "SkillManifestPatternsHN".into(),
            url: "https://news.ycombinator.com/item?id=42000000".into(),
            score: 142.0,
            fetched_at_ms: 1_700_000_000_000,
        }],
        plugin_health: vec![],
        load_sample: LoadSample {
            loadavg_1min: 0.42,
            last_edit_age_ms: None,
        },
        next_tick_ms: 4_000,
    }
}

/// Render every TUI pane into a single `TestBackend` buffer and
/// flatten the cells into a string. We use a generous 160×60 buffer
/// so wide URIs and long titles don't get truncated.
fn render_tui_text(snap: Arc<WindowSnapshot>) -> String {
    let mut state = ColdWindowState::new();
    state.ingest(snap);
    let hint_state = HintPaneState::default();
    let research_state = ResearchPaneState {
        collapsed: false,
        ..ResearchPaneState::default()
    };

    let backend = TestBackend::new(160, 60);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(10), // alert pane
                    Constraint::Length(10), // hint pane
                    Constraint::Length(35), // research pane (expanded)
                    Constraint::Length(5),  // status bar
                ])
                .split(area);
            AlertPane::render(&state, f, chunks[0]);
            HintPane::render(&state, &hint_state, f, chunks[1]);
            ResearchPane::render(&state, &research_state, f, chunks[2]);
            StatusBar::render(&state, Some((7, 10)), BUDGET, f, chunks[3]);
        })
        .unwrap();

    flatten_buffer(terminal.backend())
}

/// Walk the TestBackend buffer cell-by-cell and concatenate symbols.
/// This recovers the user-visible characters but discards styling.
fn flatten_buffer(backend: &TestBackend) -> String {
    let buffer = backend.buffer();
    let area = buffer.area;
    let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &buffer[(x, y)];
            out.push_str(cell.symbol());
        }
        out.push('\n');
    }
    out
}

/// Drive the real browser surface end-to-end: bind an ephemeral port,
/// issue a reqwest SSE GET, send a snapshot via the broadcast bus once
/// the handler has subscribed, and collect bytes until all four named
/// events arrive (or a 2 s timeout). The returned string is HTML
/// fragments stripped of their tags + decoded entities.
async fn render_browser_text(snap: Arc<WindowSnapshot>) -> String {
    let (tx, _keep_rx) = broadcast::channel(16);
    let dash_state = ColdWindowDashboardState {
        bus: tx.clone(),
        budget_ceiling: BUDGET,
        research_quota: Some((7, 10)),
    };
    let app = cold_window_routes(dash_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // Issue the SSE request in the background and drain bytes.
    let url = format!("http://{addr}/dashboard.sse");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let req = tokio::spawn(async move {
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(2000);
        loop {
            match tokio::time::timeout_at(deadline, stream.next()).await {
                Ok(Some(Ok(chunk))) => buf.extend_from_slice(&chunk),
                _ => break,
            }
            if buf.len() > 200_000 {
                break;
            }
            let s = std::str::from_utf8(&buf).unwrap_or("");
            // SSE event lines start with `event:` (no space) per
            // axum's encoder. Stop once all four are seen.
            if s.contains("event:alert")
                && s.contains("event:hint")
                && s.contains("event:research")
                && s.contains("event:status")
            {
                break;
            }
        }
        buf
    });

    // Wait until the SSE handler has subscribed to the bus, then send.
    // receiver_count() goes from 1 (the test's `_keep_rx`) to 2 once
    // the handler subscribes.
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        if tx.receiver_count() >= 2 {
            break;
        }
    }
    for _ in 0..5 {
        let _ = tx.send(snap.clone());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let bytes = req.await.unwrap_or_default();
    server.abort();

    let raw = String::from_utf8_lossy(&bytes).to_string();
    strip_html_to_text(&raw)
}

/// Strip HTML tags and decode the small set of entities that
/// `api::cold_window::html_escape` can produce. The fragments are
/// simple enough that a tag remover is sufficient for "did this
/// string survive the wire?" assertions; we don't need a full HTML
/// parser.
fn strip_html_to_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
}

#[tokio::test]
async fn tui_and_browser_render_semantic_parity() {
    let snap = Arc::new(parity_snapshot());

    let tui_text = render_tui_text(snap.clone());
    let browser_text = render_browser_text(snap.clone()).await;

    // Sanity: each surface produced *something*.
    assert!(
        !tui_text.trim().is_empty(),
        "TUI render produced empty text"
    );
    assert!(
        !browser_text.trim().is_empty(),
        "browser render produced empty text — SSE never delivered any events"
    );

    // Alert title + message must appear in both surfaces.
    for alert in &snap.alerts {
        assert!(
            tui_text.contains(&alert.title),
            "TUI missing alert title `{}`\nTUI text:\n{}",
            alert.title,
            tui_text
        );
        assert!(
            browser_text.contains(&alert.title),
            "browser missing alert title `{}`\nbrowser text:\n{}",
            alert.title,
            browser_text
        );
        assert!(
            tui_text.contains(&alert.message),
            "TUI missing alert message `{}`",
            alert.message
        );
        assert!(
            browser_text.contains(&alert.message),
            "browser missing alert message `{}`",
            alert.message
        );
    }

    // Hint URI + message must appear in both.
    for h in &snap.hints {
        assert!(
            tui_text.contains(&h.hint.uri),
            "TUI missing hint URI `{}`",
            h.hint.uri
        );
        assert!(
            browser_text.contains(&h.hint.uri),
            "browser missing hint URI `{}`",
            h.hint.uri
        );
        assert!(
            tui_text.contains(&h.hint.message),
            "TUI missing hint message `{}`",
            h.hint.message
        );
        assert!(
            browser_text.contains(&h.hint.message),
            "browser missing hint message `{}`",
            h.hint.message
        );
    }

    // Research finding title must appear in both.
    for f in &snap.research_findings {
        assert!(
            tui_text.contains(&f.title),
            "TUI missing research title `{}`",
            f.title
        );
        assert!(
            browser_text.contains(&f.title),
            "browser missing research title `{}`",
            f.title
        );
    }

    // Status fields: cadence label + token total + research quota.
    assert!(
        tui_text.contains("tick: 4.0s"),
        "TUI status missing cadence label"
    );
    assert!(
        browser_text.contains("tick: 4.0s"),
        "browser status missing cadence label"
    );
    assert!(
        tui_text.contains("25.0K"),
        "TUI status missing token total `25.0K`"
    );
    assert!(
        browser_text.contains("25.0K"),
        "browser status missing token total `25.0K`"
    );
    assert!(
        tui_text.contains("quota: 7/10"),
        "TUI status missing research quota"
    );
    assert!(
        browser_text.contains("quota: 7/10"),
        "browser status missing research quota"
    );
}
