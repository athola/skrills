//! Crossterm raw-mode launcher for the cold-window TUI.
//!
//! The four cold-window panes ([`AlertPane`], [`HintPane`],
//! [`ResearchPane`], [`StatusBar`]) are pure render functions tested
//! with `TestBackend`. This module composes them into a single frame
//! ([`draw`]), routes keystrokes ([`handle_key`]), and drives a live
//! crossterm event loop ([`run`]) that subscribes to the engine's
//! snapshot bus.
//!
//! The server's `cold-window --tui` path calls [`run`] with the
//! broadcast receiver from `ColdWindowEngine::subscribe`; everything
//! crossterm-specific stays here so the server crate needs no terminal
//! dependency of its own.

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{
    Event as CEvent, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::Frame;
use ratatui::Terminal;
use skrills_snapshot::{ResearchQuota, WindowSnapshot};
use tokio::sync::{broadcast, watch};

use super::{
    AlertPane, ColdWindowState, HintPane, HintPaneState, ResearchPane, ResearchPaneState, StatusBar,
};

/// A pull-on-demand research-quota source. The runner calls it once
/// per repaint so the status bar reflects live bucket drain rather
/// than a value frozen at startup. Boxed (rather than a generic) to
/// keep [`run`]'s signature object-safe across call sites.
pub type QuotaFn = Box<dyn Fn() -> ResearchQuota + Send + Sync>;

/// Launch-time configuration for the cold-window TUI.
#[derive(Debug, Clone, Copy)]
pub struct TuiOptions {
    /// Token ceiling drawn on the status bar; mirrors `--alert-budget`.
    pub budget_ceiling: u64,
    /// Ring the terminal bell on a newly-fired WARNING; `--no-bell`
    /// clears it.
    pub bell_enabled: bool,
}

/// What the event loop should do after a keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Tear down the TUI and return.
    Quit,
    /// Re-render with the (possibly mutated) pane state.
    Redraw,
}

/// Compose the four panes into a single frame.
///
/// Layout: a one-line status bar pinned to the bottom; above it a
/// 60/40 split with alerts (top-left) over hints (bottom-left) and the
/// research pane filling the right column.
pub fn draw(
    frame: &mut Frame<'_>,
    snap_state: &ColdWindowState,
    hint_state: &HintPaneState,
    research_state: &ResearchPaneState,
    research_quota: Option<ResearchQuota>,
    budget_ceiling: u64,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(frame.area());
    let body = rows[0];
    let status = rows[1];

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(body);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(cols[0]);

    AlertPane::render(snap_state, frame, left[0]);
    HintPane::render(snap_state, hint_state, frame, left[1]);
    ResearchPane::render(snap_state, research_state, frame, cols[1]);
    StatusBar::render(snap_state, research_quota, budget_ceiling, frame, status);
}

/// Route a keystroke to the panes.
///
/// Global keys win first: `q`/`Esc`/`Ctrl-C` quit. Everything else is
/// forwarded to all three pane handlers — their keybindings are
/// disjoint (`A`/`d` for alerts, `0`-`5`/`P` for hints, `R` for
/// research), so there is no focus model to manage.
pub fn handle_key(
    key: KeyEvent,
    snap_state: &mut ColdWindowState,
    hint_state: &mut HintPaneState,
    research_state: &mut ResearchPaneState,
) -> KeyOutcome {
    let ctrl_c = key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'));
    if ctrl_c
        || matches!(
            key.code,
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc
        )
    {
        return KeyOutcome::Quit;
    }

    // Disjoint keymaps: forwarding the same code to each handler is
    // safe because at most one will act on it.
    let _ = AlertPane::handle_key(snap_state, key.code);
    let _ = HintPane::handle_key(snap_state, hint_state, key.code);
    let _ = ResearchPane::handle_key(snap_state, research_state, key.code);
    KeyOutcome::Redraw
}

/// True for key *press* events. crossterm reports press, repeat, and
/// release on some platforms (Windows); acting on release would
/// double-fire every keystroke.
fn is_actionable(key: &KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

/// Run the cold-window TUI to completion.
///
/// Owns the terminal: enters raw mode + the alternate screen, installs
/// a panic hook that restores the terminal, then loops until `q`,
/// `Ctrl-C`, the `shutdown` watch flips true, or the snapshot bus
/// closes. Restores the terminal on every exit path.
pub async fn run(
    mut snapshots: broadcast::Receiver<Arc<WindowSnapshot>>,
    mut shutdown: watch::Receiver<bool>,
    quota: Option<QuotaFn>,
    opts: TuiOptions,
) -> Result<()> {
    // Restore the terminal even if a render or pane panics, otherwise
    // the user is left in raw mode on a wrecked screen.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal")?;

    let loop_result = event_loop(
        &mut terminal,
        &mut snapshots,
        &mut shutdown,
        quota.as_deref(),
        opts,
    )
    .await;

    // Always restore, regardless of how the loop ended.
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    loop_result
}

/// The core select loop, split out from terminal setup/teardown so the
/// `?`-propagated render errors still run through [`run`]'s restore.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    snapshots: &mut broadcast::Receiver<Arc<WindowSnapshot>>,
    shutdown: &mut watch::Receiver<bool>,
    quota: Option<&(dyn Fn() -> ResearchQuota + Send + Sync)>,
    opts: TuiOptions,
) -> Result<()> {
    let mut snap_state = ColdWindowState::new();
    snap_state.bell_enabled = opts.bell_enabled;
    let mut hint_state = HintPaneState::new();
    let mut research_state = ResearchPaneState::default();
    let mut events = EventStream::new();
    // Repaint floor so the status bar's quota/clock stay fresh even
    // when no snapshot or key arrives.
    let mut repaint = tokio::time::interval(Duration::from_millis(250));

    let paint = |term: &mut Terminal<CrosstermBackend<io::Stdout>>,
                 snap_state: &ColdWindowState,
                 hint_state: &HintPaneState,
                 research_state: &ResearchPaneState|
     -> Result<()> {
        let q = quota.map(|f| f());
        term.draw(|f| {
            draw(
                f,
                snap_state,
                hint_state,
                research_state,
                q,
                opts.budget_ceiling,
            )
        })?;
        Ok(())
    };

    paint(terminal, &snap_state, &hint_state, &research_state)?;

    loop {
        if *shutdown.borrow() {
            break;
        }
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            recv = snapshots.recv() => {
                match recv {
                    Ok(snap) => {
                        if snap_state.ingest(snap) {
                            // BEL is audio-only; it does not perturb the
                            // alternate-screen buffer ratatui owns.
                            let mut out = io::stdout();
                            let _ = out.write_all(b"\x07");
                            let _ = out.flush();
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "cold-window TUI lagged the snapshot bus");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
                paint(terminal, &snap_state, &hint_state, &research_state)?;
            }
            maybe = events.next() => {
                match maybe {
                    Some(Ok(CEvent::Key(key))) if is_actionable(&key) => {
                        match handle_key(key, &mut snap_state, &mut hint_state, &mut research_state) {
                            KeyOutcome::Quit => break,
                            KeyOutcome::Redraw => {
                                paint(terminal, &snap_state, &hint_state, &research_state)?;
                            }
                        }
                    }
                    Some(Ok(CEvent::Resize(_, _))) => {
                        paint(terminal, &snap_state, &hint_state, &research_state)?;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "cold-window TUI event stream error");
                        break;
                    }
                    None => break,
                }
            }
            _ = repaint.tick() => {
                paint(terminal, &snap_state, &hint_state, &research_state)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use ratatui::backend::TestBackend;
    use skrills_snapshot::{
        Alert, AlertBand, Hint, HintCategory, LoadSample, ScoredHint, Severity, TokenEntry,
        TokenLedger,
    };

    fn rich_snapshot() -> Arc<WindowSnapshot> {
        Arc::new(WindowSnapshot {
            version: 7,
            timestamp_ms: 1_700_000_000_000,
            token_ledger: TokenLedger {
                per_skill: vec![TokenEntry {
                    source: "skill://demo".into(),
                    tokens: 42_000,
                }],
                per_plugin: vec![],
                per_mcp: vec![],
                conversation_cache_reads: 0,
                conversation_cache_writes: 0,
                total: 42_000,
            },
            alerts: vec![Alert {
                fingerprint: "w1".into(),
                severity: Severity::Warning,
                title: "budget pressure".into(),
                message: "token total climbing".into(),
                band: Some(AlertBand::new(0.0, 0.0, 1.0, 0.95).expect("band")),
                fired_at_ms: 1_700_000_000_000,
                dwell_ticks: 2,
            }],
            hints: vec![ScoredHint {
                hint: Hint {
                    uri: "skill://refactor".into(),
                    category: HintCategory::Token,
                    message: "split large skill".into(),
                    frequency: 3,
                    impact: 8.5,
                    ease_score: 6.0,
                    age_days: 1.0,
                },
                score: 0.9,
                pinned: false,
            }],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        })
    }

    #[test]
    fn draw_composes_all_panes_without_panic() {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();
        terminal
            .draw(|f| {
                draw(
                    f,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    Some(ResearchQuota::new(3, 10)),
                    100_000,
                )
            })
            .expect("composite draw must not fail");
    }

    #[test]
    fn draw_survives_tiny_and_huge_areas() {
        // R9: the composite must not panic across terminal sizes.
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();
        for (w, h) in [(20u16, 5u16), (40, 12), (200, 60), (8, 2)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| draw(f, &snap_state, &hint_state, &research_state, None, 100_000))
                .unwrap_or_else(|e| panic!("draw panicked at {w}x{h}: {e}"));
        }
    }

    #[test]
    fn first_paint_reflects_snapshot_under_500ms() {
        // SC3: startup-to-first-snapshot must beat 500 ms. The launch
        // path's cost is dominated by ingest + a single composite
        // render; we measure exactly that against a real backend.
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();

        let t0 = Instant::now();
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        terminal
            .draw(|f| {
                draw(
                    f,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    Some(ResearchQuota::new(0, 10)),
                    100_000,
                )
            })
            .unwrap();
        let elapsed = t0.elapsed();

        assert_eq!(snap_state.token_total(), 42_000, "snapshot not ingested");
        assert!(
            elapsed < Duration::from_millis(500),
            "SC3 first paint took {elapsed:?}, exceeds 500 ms budget"
        );
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn q_and_esc_quit() {
        let mut s = ColdWindowState::new();
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        assert_eq!(
            handle_key(key(KeyCode::Char('q')), &mut s, &mut h, &mut r),
            KeyOutcome::Quit
        );
        assert_eq!(
            handle_key(key(KeyCode::Esc), &mut s, &mut h, &mut r),
            KeyOutcome::Quit
        );
    }

    #[test]
    fn ctrl_c_quits_but_plain_c_does_not() {
        let mut s = ColdWindowState::new();
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(handle_key(ctrl_c, &mut s, &mut h, &mut r), KeyOutcome::Quit);
        assert_eq!(
            handle_key(key(KeyCode::Char('c')), &mut s, &mut h, &mut r),
            KeyOutcome::Redraw
        );
    }

    #[test]
    fn master_ack_key_clears_non_warning_alerts() {
        // 'A' must reach the alert pane and clear caution/advisory.
        let mut s = ColdWindowState::new();
        s.ingest(Arc::new(WindowSnapshot {
            version: 1,
            timestamp_ms: 1,
            token_ledger: TokenLedger::default(),
            alerts: vec![Alert {
                fingerprint: "c1".into(),
                severity: Severity::Caution,
                title: "t".into(),
                message: "m".into(),
                band: None,
                fired_at_ms: 1,
                dwell_ticks: 1,
            }],
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        }));
        assert_eq!(s.visible_alerts().len(), 1);
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        let outcome = handle_key(key(KeyCode::Char('A')), &mut s, &mut h, &mut r);
        assert_eq!(outcome, KeyOutcome::Redraw);
        assert!(
            s.visible_alerts().is_empty(),
            "master-ack 'A' did not reach the alert pane"
        );
    }
}
