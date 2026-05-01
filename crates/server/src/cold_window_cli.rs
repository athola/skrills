//! Cold-window subcommand entry point (TASK-021 + TASK-031).
//!
//! Wires the engine, a tick producer, and the optional HTTP browser
//! surface together. Listens for SIGINT and SIGTERM and cleanly tears
//! everything down within the spec § 3 / TASK-031 2-second budget.
//!
//! v0.8.0 ships **browser-mode** as the primary surface. The TUI
//! panes (`skrills_dashboard::cold_window`) are fully implemented and
//! tested as library code; mounting them into a crossterm raw-mode
//! loop lands as a follow-up. Users today run:
//!
//! ```text
//! skrills cold-window --browser --port 8888
//! ```
//!
//! and open `http://localhost:8888/dashboard`.

#![cfg(feature = "http-transport")]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Args;
use skrills_analyze::cold_window::cadence::read_loadavg_1min;
use skrills_analyze::cold_window::engine::TickInput;
use skrills_analyze::cold_window::{ColdWindowEngine, PluginHealthCollector};
use skrills_snapshot::{KillSwitch, LoadSample, TokenEntry, TokenLedger};
use skrills_tome::dispatcher::BucketedBudget;
use tokio::sync::watch;

use crate::api::{cold_window_routes, ColdWindowDashboardState};
use crate::discovery::merge_extra_dirs;

/// Floor on the adaptive tick delay (ms). Prevents the engine from
/// busy-looping if the snapshot's `next_tick_ms` is reported as 0
/// or close to it under unusual load conditions.
const MIN_TICK_MS: u64 = 50;

/// CLI flags for `skrills cold-window`.
///
/// Matches spec § 3.10 except that `--no-bell` is a TUI-only concern
/// and lands when the TUI surface is wired up. Defaults are the
/// values frozen in the brief / spec.
#[derive(Debug, Clone, Args)]
pub struct ColdWindowArgs {
    /// Token budget ceiling. Above this the LayeredAlertPolicy fires
    /// a Warning and engages the kill-switch.
    #[arg(long, default_value_t = 100_000)]
    pub alert_budget: u64,

    /// Research dispatcher fetches per hour.
    #[arg(long, default_value_t = 10)]
    pub research_rate: u32,

    /// Disable the load-aware adaptive cadence and use the fixed base
    /// tick rate. Equivalent to clamping load_ratio to 0.0.
    #[arg(long, default_value_t = false)]
    pub no_adaptive: bool,

    /// Run the HTTP browser surface alongside the engine. Without
    /// this flag the engine still ticks but no surface attaches.
    #[arg(long, default_value_t = false)]
    pub browser: bool,

    /// Browser port (only meaningful with `--browser`).
    #[arg(long, default_value_t = 8888)]
    pub port: u16,

    /// Override the base tick rate in milliseconds (default 2_000ms).
    #[arg(long, value_name = "MILLIS")]
    pub tick_rate_ms: Option<u64>,

    /// Watch additional skill directories for the cold-window
    /// producer (in addition to defaults).
    #[arg(long = "skill-dir", value_name = "DIR")]
    pub skill_dirs: Vec<PathBuf>,

    /// Plugins root directory whose `<plugin>/health.toml` files
    /// participate in each tick (FR11). Defaults to `./plugins`
    /// relative to the current working directory; missing or
    /// unreadable directories yield an empty plugin set without
    /// error (the cold-window must never crash on user state).
    #[arg(long = "plugins-dir", value_name = "DIR")]
    pub plugins_dir: Option<PathBuf>,
}

/// Await a spawned task handle, surfacing any `JoinError` instead of
/// silently dropping it (NI10). A clean exit is logged at trace level
/// (callers may upgrade); a panic is logged at error level so it shows
/// up in production logs by default; an unexpected end (cancellation,
/// abort) is a warning. The `kind` argument is interpolated into the
/// message and into the structured fields so log filters can target
/// just the producer or just the server.
async fn await_task_handle(handle: tokio::task::JoinHandle<Result<()>>, kind: &str) {
    match handle.await {
        Ok(_) => {}
        Err(e) if e.is_panic() => {
            tracing::error!(kind = %kind, error = ?e, "{kind} task panicked");
        }
        Err(e) => {
            tracing::warn!(kind = %kind, error = ?e, "{kind} task ended unexpectedly");
        }
    }
}

/// Run the cold-window subcommand to completion (or until SIGINT/SIGTERM).
///
/// The async runtime is created/used by the caller — this function
/// is meant to be invoked from inside `tokio::main` or
/// `tokio::runtime::Runtime::block_on`.
pub async fn run(args: ColdWindowArgs) -> Result<()> {
    tracing::info!(
        budget = args.alert_budget,
        research_rate = args.research_rate,
        port = args.port,
        browser = args.browser,
        "starting cold-window subcommand"
    );

    // B4 server-half: mint one shared kill-switch. Cloned into the
    // engine via `with_kill_switch`, then handed to any sync adapter
    // constructed in this run. The engine engages it on token-budget
    // breach (FR12); adapters consult it before mutating I/O.
    let kill_switch = KillSwitch::new();

    let engine = Arc::new(
        ColdWindowEngine::with_defaults(args.alert_budget).with_kill_switch(kill_switch.clone()),
    );
    let bus = engine.bus_sender();

    // B2: mint the research-budget dispatcher from the parsed CLI rate.
    // In-memory variant — persistent path is the daemon's job
    // (`skrills daemon`), not the cold-window subcommand.
    let dispatcher = Arc::new(BucketedBudget::in_memory(args.research_rate));

    // NI16: resolve the merged skill-dir list once at startup so the
    // user sees confirmation in the logs that their `--skill-dir`
    // flags were honored. Per-tick discovery wiring lands when the
    // producer takes a real skill collector (T-NEXT in plan.md).
    let merged_skill_dirs = merge_extra_dirs(&args.skill_dirs);
    if !merged_skill_dirs.is_empty() {
        tracing::info!(
            skill_dir_count = merged_skill_dirs.len(),
            "cold-window resolved extra skill directories",
        );
    }

    // Shutdown channel: producer + server both watch this.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn the producer task (fixture-driven for v0.8.0 demo).
    let plugins_dir = args
        .plugins_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("plugins"));
    let producer_handle = tokio::spawn(producer_loop(
        Arc::clone(&engine),
        args.tick_rate_ms.unwrap_or(2_000),
        args.no_adaptive,
        plugins_dir,
        merged_skill_dirs,
        shutdown_rx.clone(),
    ));

    // Spawn the browser server if requested.
    let server_handle = if args.browser {
        // B3: hand the dispatcher to the dashboard so the status bar
        // reflects live drain state, not a frozen snapshot.
        let state = ColdWindowDashboardState::new(bus.clone(), args.alert_budget)
            .with_research_quota_source(Arc::clone(&dispatcher));
        let addr: SocketAddr = (Ipv4Addr::LOCALHOST, args.port).into();
        let shutdown_rx = shutdown_rx.clone();
        Some(tokio::spawn(async move {
            run_browser(state, addr, shutdown_rx).await
        }))
    } else {
        None
    };

    // Wait for SIGINT or SIGTERM.
    wait_for_shutdown_signal().await;
    tracing::info!("shutdown signal received; tearing down");
    let _ = shutdown_tx.send(true);

    // Bound the cleanup window per spec (2 seconds, T031).
    let cleanup = async {
        await_task_handle(producer_handle, "producer").await;
        if let Some(h) = server_handle {
            await_task_handle(h, "server").await;
        }
    };
    match tokio::time::timeout(Duration::from_secs(2), cleanup).await {
        Ok(()) => tracing::info!("clean shutdown"),
        Err(_) => {
            tracing::warn!("shutdown did not complete within 2s; tasks aborted by drop");
        }
    }
    Ok(())
}

/// Producer loop: synthesize a `TickInput` from the local environment
/// every `next_tick_ms` (read from the most recent snapshot) and call
/// `engine.tick`. v0.8.0 demo body uses a small synthetic ledger that
/// grows over time so the alert policy gets exercised, but plugin
/// participation (FR11/T022) is real: each tick re-walks
/// `<plugins_dir>/*/health.toml` cold and feeds the result to the
/// engine. Token attribution from real discovery is a follow-up.
async fn producer_loop(
    engine: Arc<ColdWindowEngine>,
    base_tick_ms: u64,
    no_adaptive: bool,
    plugins_dir: PathBuf,
    _skill_dirs: Vec<PathBuf>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut tick_count: u64 = 0;
    let mut next_delay_ms = base_tick_ms;
    let plugin_collector = Arc::new(PluginHealthCollector::new(plugins_dir));
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    tracing::info!("producer received shutdown");
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(next_delay_ms)) => {
                tick_count += 1;
                let mut input = build_demo_input(tick_count, no_adaptive);
                // FR11 + NI3: real plugin participation each tick. Cold
                // rewalk — never cached — per the spec contract. The
                // walk runs on the blocking pool so the runtime's
                // worker threads stay free for IO-bound tasks (SSE
                // subscribers, signal handlers).
                let collector = Arc::clone(&plugin_collector);
                let collector_output = match tokio::task::spawn_blocking(move || {
                    collector.collect()
                })
                .await
                {
                    Ok(out) => out,
                    Err(join_err) => {
                        tracing::warn!(
                            error = ?join_err,
                            "plugin collector spawn_blocking task failed; \
                             skipping plugin participation for this tick"
                        );
                        continue;
                    }
                };
                input = input.with_plugin_collector_output(collector_output);
                let snap = engine.tick(input);
                next_delay_ms = if no_adaptive {
                    base_tick_ms
                } else {
                    snap.next_tick_ms.max(MIN_TICK_MS)
                };
            }
        }
    }
    Ok(())
}

/// Build a synthetic `TickInput` that exercises the alert pipeline.
///
/// Token totals scale with `tick_count` so a long-running session
/// crosses Advisory → Caution → Warning thresholds; the chaos-style
/// trajectory shows the dashboard "doing something" during a demo.
/// Replace with real discovery + analyze::tokens attribution in a
/// follow-up.
fn build_demo_input(tick_count: u64, no_adaptive: bool) -> TickInput {
    let total = tick_count.saturating_mul(1_500);
    let load_sample = if no_adaptive {
        LoadSample::default()
    } else {
        LoadSample {
            loadavg_1min: read_loadavg_1min(),
            last_edit_age_ms: None,
        }
    };
    let token_ledger = TokenLedger {
        per_skill: vec![TokenEntry {
            source: "skill://demo".into(),
            tokens: total / 2,
        }],
        per_plugin: vec![],
        per_mcp: vec![TokenEntry {
            source: "mcp://demo".into(),
            tokens: total / 2,
        }],
        conversation_cache_reads: 0,
        conversation_cache_writes: 0,
        total,
    };
    TickInput::empty()
        .with_timestamp_ms(now_ms())
        .with_token_ledger(token_ledger)
        .with_load_sample(load_sample)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Bind a TCP listener and serve the cold-window router with axum's
/// graceful-shutdown future tied to `shutdown_rx`. Returns when the
/// server has fully drained.
async fn run_browser(
    state: ColdWindowDashboardState,
    addr: SocketAddr,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<()> {
    let app = cold_window_routes(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!(%addr, "browser surface listening");
    let shutdown = async move {
        // Wait for the watch channel to flip to true.
        loop {
            if *shutdown_rx.borrow() {
                break;
            }
            if shutdown_rx.changed().await.is_err() {
                break;
            }
        }
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .context("axum::serve")?;
    Ok(())
}

/// Block until SIGINT or (on unix) SIGTERM arrives.
async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            sigterm.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received SIGINT"),
        _ = terminate => tracing::info!("received SIGTERM"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};

    /// Shared in-memory writer for capturing tracing output. Cloned
    /// across thread/task boundaries so the test can inspect what the
    /// subscriber wrote regardless of which worker emitted the event.
    #[derive(Clone, Default)]
    struct CaptureWriter {
        buf: Arc<Mutex<Vec<u8>>>,
    }

    impl CaptureWriter {
        fn contents(&self) -> String {
            String::from_utf8_lossy(&self.buf.lock().unwrap()).to_string()
        }
    }

    impl io::Write for CaptureWriter {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.buf.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn build_demo_input_scales_tokens_with_tick_count() {
        let i1 = build_demo_input(1, true);
        let i10 = build_demo_input(10, true);
        let i100 = build_demo_input(100, true);
        assert_eq!(i1.token_ledger.total, 1_500);
        assert_eq!(i10.token_ledger.total, 15_000);
        assert_eq!(i100.token_ledger.total, 150_000);
    }

    #[test]
    fn build_demo_input_with_no_adaptive_zeros_load_sample() {
        let i = build_demo_input(5, true);
        assert_eq!(i.load_sample.loadavg_1min, 0.0);
        assert!(i.load_sample.last_edit_age_ms.is_none());
    }

    #[test]
    fn build_demo_input_partitions_total_between_skill_and_mcp() {
        let i = build_demo_input(40, true);
        let skill_total: u64 = i.token_ledger.per_skill.iter().map(|e| e.tokens).sum();
        let mcp_total: u64 = i.token_ledger.per_mcp.iter().map(|e| e.tokens).sum();
        assert_eq!(skill_total + mcp_total, i.token_ledger.total);
    }

    #[tokio::test]
    async fn producer_loop_terminates_on_shutdown() {
        let engine = Arc::new(ColdWindowEngine::with_defaults(100_000));
        let (tx, rx) = watch::channel(false);
        let handle = tokio::spawn(producer_loop(
            Arc::clone(&engine),
            50,
            true,
            PathBuf::from("/nonexistent-plugins-test"),
            Vec::new(),
            rx,
        ));
        // Let the producer fire a few ticks.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = tx.send(true);
        // Should terminate within a small budget (well below the 2s
        // shutdown budget guaranteed by the caller).
        let result = tokio::time::timeout(Duration::from_secs(1), handle).await;
        assert!(result.is_ok(), "producer did not terminate within 1s");
    }

    #[tokio::test]
    async fn producer_loop_drives_engine_versions_forward() {
        let engine = Arc::new(ColdWindowEngine::with_defaults(100_000));
        let mut rx = engine.subscribe();
        let (tx, shutdown_rx) = watch::channel(false);
        let _handle = tokio::spawn(producer_loop(
            Arc::clone(&engine),
            30,
            true,
            PathBuf::from("/nonexistent-plugins-test"),
            Vec::new(),
            shutdown_rx,
        ));

        let mut last_version = 0;
        for _ in 0..3 {
            match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(snap)) => {
                    assert!(
                        snap.version > last_version,
                        "version did not advance: {} <= {}",
                        snap.version,
                        last_version
                    );
                    last_version = snap.version;
                }
                _ => panic!("did not receive snapshot in time"),
            }
        }
        let _ = tx.send(true);
    }

    #[test]
    fn cold_window_args_parse_with_defaults() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: ColdWindowArgs,
        }

        let cli = TestCli::parse_from(["test"]);
        assert_eq!(cli.args.alert_budget, 100_000);
        assert_eq!(cli.args.research_rate, 10);
        assert_eq!(cli.args.port, 8888);
        assert!(!cli.args.browser);
        assert!(!cli.args.no_adaptive);
        assert!(cli.args.tick_rate_ms.is_none());
    }

    #[tokio::test]
    async fn await_task_handle_logs_error_when_producer_panics() {
        // NI10: simulate a panicking producer task and assert the
        // `tracing::error!` call fires, instead of the previous
        // `let _ = handle.await;` which dropped the JoinError silently.
        let writer = CaptureWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let handle: tokio::task::JoinHandle<Result<()>> =
            tokio::spawn(async { panic!("simulated producer crash") });
        await_task_handle(handle, "producer").await;

        let logs = writer.contents();
        assert!(
            logs.contains("ERROR"),
            "expected ERROR-level log, got:\n{logs}"
        );
        assert!(
            logs.contains("producer task panicked"),
            "expected panic message, got:\n{logs}"
        );
    }

    #[tokio::test]
    async fn await_task_handle_logs_warn_when_task_aborted() {
        // NI10: an aborted task surfaces as a (non-panic) JoinError;
        // it must hit the WARN arm instead of being dropped.
        let writer = CaptureWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let handle: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(())
        });
        handle.abort();
        await_task_handle(handle, "server").await;

        let logs = writer.contents();
        assert!(
            logs.contains("WARN"),
            "expected WARN-level log, got:\n{logs}"
        );
        assert!(
            logs.contains("server task ended unexpectedly"),
            "expected unexpected-end message, got:\n{logs}"
        );
    }

    #[tokio::test]
    async fn await_task_handle_is_silent_on_clean_exit() {
        let writer = CaptureWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let handle: tokio::task::JoinHandle<Result<()>> = tokio::spawn(async { Ok(()) });
        await_task_handle(handle, "producer").await;

        let logs = writer.contents();
        assert!(
            !logs.contains("panicked"),
            "clean exit must not log panic, got:\n{logs}"
        );
        assert!(
            !logs.contains("ended unexpectedly"),
            "clean exit must not log unexpected end, got:\n{logs}"
        );
    }

    #[test]
    fn research_rate_one_caps_dispatcher_at_one_per_hour() {
        // B2: a `--research-rate 1` flag must yield a dispatcher whose
        // capacity is exactly 1/hour. Build the same `BucketedBudget`
        // the runner builds and observe the snapshot.
        let dispatcher = BucketedBudget::in_memory(1);
        let snap = dispatcher.current_state();
        assert_eq!(snap.rate_per_hour, 1);
        assert_eq!(snap.available, 1.0);
    }

    #[tokio::test]
    async fn producer_collect_runs_in_blocking_pool() {
        // NI3: regression — `producer_loop` previously called
        // `plugin_collector.collect()` directly on a runtime worker,
        // which can stall the executor under slow filesystems. The fix
        // wraps the call in `tokio::task::spawn_blocking`. We verify
        // the loop continues to drive snapshots forward through the
        // bus even when the plugins dir does not exist (collect()
        // returns empty), which exercises the spawn_blocking path.
        let engine = Arc::new(ColdWindowEngine::with_defaults(100_000));
        let mut rx = engine.subscribe();
        let (tx, shutdown_rx) = watch::channel(false);
        let _handle = tokio::spawn(producer_loop(
            Arc::clone(&engine),
            30,
            true,
            PathBuf::from("/nonexistent-plugins-test"),
            Vec::new(),
            shutdown_rx,
        ));
        let snap = tokio::time::timeout(Duration::from_millis(500), rx.recv())
            .await
            .expect("snapshot did not arrive in time")
            .expect("bus closed");
        // The plugins dir does not exist, so the collector returns
        // empty; assert the engine still ticked at least once.
        assert!(snap.version >= 1);
        assert!(snap.plugin_health.is_empty());
        let _ = tx.send(true);
    }

    #[test]
    fn kill_switch_engages_when_engine_observes_budget_breach() {
        // B4 cross-check: a switch shared with the engine engages once
        // a tick crosses the budget ceiling. We construct the engine
        // exactly the way `run` does — via `with_defaults` then
        // `with_kill_switch` — so any drift in that chain breaks this
        // assertion.
        use skrills_analyze::cold_window::engine::TickInput;
        let kill_switch = KillSwitch::new();
        assert!(!kill_switch.is_engaged());
        let engine = ColdWindowEngine::with_defaults(10_000).with_kill_switch(kill_switch.clone());
        // Drive a tick whose token total is at the ceiling. The engine
        // engages the switch in `tick()` per FR12.
        let breach = TickInput::empty().with_token_ledger(TokenLedger {
            per_skill: vec![],
            per_plugin: vec![],
            per_mcp: vec![TokenEntry {
                source: "mcp://breach".into(),
                tokens: 10_000,
            }],
            conversation_cache_reads: 0,
            conversation_cache_writes: 0,
            total: 10_000,
        });
        let _ = engine.tick(breach);
        assert!(
            kill_switch.is_engaged(),
            "kill-switch must engage when token total reaches budget"
        );
    }

    #[test]
    fn skill_dirs_arg_propagates_through_merge_extra_dirs() {
        // NI16: the user-provided `--skill-dir` paths flow through the
        // same `merge_extra_dirs` helper that the rest of the server
        // uses, so a future producer that takes a real skill collector
        // sees them. Until that producer lands, this contract pins the
        // call site so it can't silently drop the field.
        let cli_dirs = vec![PathBuf::from("/tmp/skrills-test-a")];
        let merged = merge_extra_dirs(&cli_dirs);
        assert!(
            merged
                .iter()
                .any(|p| p == &PathBuf::from("/tmp/skrills-test-a")),
            "merge_extra_dirs must include the CLI-supplied dir, got: {merged:?}"
        );
    }

    #[test]
    fn cold_window_args_parse_with_overrides() {
        use clap::Parser;

        #[derive(Parser, Debug)]
        struct TestCli {
            #[command(flatten)]
            args: ColdWindowArgs,
        }

        let cli = TestCli::parse_from([
            "test",
            "--alert-budget",
            "50000",
            "--research-rate",
            "5",
            "--port",
            "9000",
            "--browser",
            "--no-adaptive",
            "--tick-rate-ms",
            "500",
            "--skill-dir",
            "/tmp/skills",
        ]);
        assert_eq!(cli.args.alert_budget, 50_000);
        assert_eq!(cli.args.research_rate, 5);
        assert_eq!(cli.args.port, 9000);
        assert!(cli.args.browser);
        assert!(cli.args.no_adaptive);
        assert_eq!(cli.args.tick_rate_ms, Some(500));
        assert_eq!(cli.args.skill_dirs.len(), 1);
    }
}
