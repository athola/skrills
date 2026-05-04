//! SC2/SC3 first-paint timing assertions (NI17 from PR #218).
//!
//! Spec § 6 (`docs/archive/2026-04-26-cold-window-spec.md`):
//! - **SC2**: Browser surface renders the first paint within 1 second
//!   of the dashboard URL request on `localhost`.
//! - **SC3**: TUI startup to first snapshot under 500ms.
//!
//! The browser-surface assertion is implemented in
//! [`cold_window_dashboard_first_paint_under_one_second`]. We bind an
//! ephemeral-port server, send a snapshot through the bus, then issue
//! a single GET against `/dashboard` (the HTML shell, not the SSE
//! stream — first paint = HTML byte arrival per the spec) and assert
//! the elapsed time is under 1 s.
//!
//! SC3 (TUI startup) needs a real crossterm raw-mode loop, which is
//! not yet wired up in v0.8.0 (the TUI panes are library code, not a
//! launchable binary). Tracking is via the `#[ignore]`-marked
//! placeholder below; the assertion lands when TASK-024 / the TUI
//! integration crate ships.

#![cfg(feature = "http-transport")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use skrills_server::api::{cold_window_routes, ColdWindowDashboardState};
use skrills_snapshot::{LoadSample, TokenLedger, WindowSnapshot};
use tokio::sync::broadcast;

fn empty_snap() -> WindowSnapshot {
    WindowSnapshot {
        version: 1,
        timestamp_ms: 0,
        token_ledger: TokenLedger::default(),
        alerts: vec![],
        hints: vec![],
        research_findings: vec![],
        plugin_health: vec![],
        load_sample: LoadSample::default(),
        next_tick_ms: 2_000,
    }
}

#[tokio::test]
async fn cold_window_dashboard_first_paint_under_one_second() {
    // SC2: time from GET to HTML body return must beat 1 s on localhost.
    // Bind on an ephemeral port and serve the cold-window routes.
    let (tx, _keep_rx) = broadcast::channel(16);
    let _ = tx.send(Arc::new(empty_snap()));
    let state = ColdWindowDashboardState::new(tx, 100_000);
    let app = cold_window_routes(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let url = format!("http://{addr}/dashboard");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();

    let t0 = Instant::now();
    let resp = client.get(&url).send().await.expect("dashboard GET");
    let body = resp.text().await.expect("dashboard body");
    let elapsed = t0.elapsed();

    server.abort();

    // Sanity: the HTML shell wires up the SSE EventSource so the
    // browser knows where to subscribe for tick updates.
    assert!(
        body.contains("EventSource") && body.contains("/dashboard.sse"),
        "dashboard body did not include EventSource bootstrap"
    );

    // SC2: hard wall at 1 s. On a quiet localhost this is typically
    // <50 ms; the budget exists to catch perf regressions, not to
    // pass on a hot machine.
    assert!(
        elapsed < Duration::from_millis(1_000),
        "SC2 first paint took {elapsed:?}, exceeds 1 s budget"
    );
}

#[tokio::test]
#[ignore = "SC3: pending TUI integration test (TASK-024 follow-up)"]
async fn cold_window_tui_startup_under_five_hundred_ms() {
    // SC3: TUI startup-to-first-snapshot must beat 500 ms.
    //
    // Today the cold-window TUI panes are library types in
    // `skrills_dashboard::cold_window` rendered via `TestBackend` in
    // unit tests. There is no `skrills cold-window --tui` binary path
    // yet, so this assertion has no surface to measure. When the TUI
    // launch path lands (TASK-024 in cold-window-plan.md), replace
    // this body with: spawn the TUI, drive the bus to first snapshot,
    // assert `t_first_snapshot - t_spawn < Duration::from_millis(500)`.
    unimplemented!("SC3 measurement requires the TUI launch path from TASK-024");
}
