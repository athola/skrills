//! Terminal lifecycle and event loop for the cold-window TUI.
//!
//! [`run`] owns the terminal (raw mode, alternate screen, panic-safe
//! restore) and drives [`event_loop`], which multiplexes the snapshot
//! bus, key events, and a repaint floor, delegating to [`super::render`]
//! and [`super::input`].

use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as CEvent, EventStream, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use skrills_snapshot::{ResearchQuota, WindowSnapshot};
use tokio::sync::{broadcast, watch};

use super::input::{handle_key, KeyOutcome};
use super::render::draw;
use super::UiState;
use crate::cold_window::{ColdWindowState, HintPaneState, ResearchPaneState};

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

/// True for key *press* events. crossterm reports press, repeat, and
/// release on some platforms (Windows); acting on release would
/// double-fire every keystroke.
fn is_actionable(key: &KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

/// Run the cold-window TUI to completion.
///
/// Owns the terminal: enters raw mode and the alternate screen, installs
/// a panic hook that restores the terminal (raw mode off, leave the
/// alternate screen, show the cursor), then loops until `q`, `Ctrl-C`,
/// the `shutdown` watch flips true, or the snapshot bus closes.
/// Restores the terminal on every exit path, and reinstates the prior
/// panic hook on normal exit so nothing leaks into the host process.
pub async fn run(
    mut snapshots: broadcast::Receiver<Arc<WindowSnapshot>>,
    mut shutdown: watch::Receiver<bool>,
    quota: Option<QuotaFn>,
    opts: TuiOptions,
) -> Result<()> {
    // Restore the terminal even if a render or pane panics, otherwise
    // the user is left in raw mode on a wrecked screen. The hook is
    // shared via `Arc` so the normal-exit path below can reinstate the
    // prior hook instead of leaking ours into the rest of the process.
    let original_hook = Arc::new(std::panic::take_hook());
    {
        let original_hook = Arc::clone(&original_hook);
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            // Mirror the normal-exit teardown order, including
            // `Show`: a panic must not leave the cursor hidden.
            let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
            original_hook(info);
        }));
    }

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

    // Reinstate the prior panic hook on the normal-exit path so our
    // terminal-restoring hook does not persist for the rest of the
    // process. (On a panic we never reach here, the hook fires during
    // unwinding, which is exactly when we still need it.)
    std::panic::set_hook(Box::new(move |info| original_hook(info)));

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
    let mut ui = UiState::new();
    let mut snap_state = ColdWindowState::new();
    snap_state.bell_enabled = opts.bell_enabled;
    let mut hint_state = HintPaneState::new();
    let mut research_state = ResearchPaneState::default();
    let mut events = EventStream::new();
    // Repaint floor so the status bar's quota/clock stay fresh even
    // when no snapshot or key arrives.
    let mut repaint = tokio::time::interval(Duration::from_millis(250));

    let paint = |term: &mut Terminal<CrosstermBackend<io::Stdout>>,
                 ui: &UiState,
                 snap_state: &ColdWindowState,
                 hint_state: &HintPaneState,
                 research_state: &ResearchPaneState|
     -> Result<()> {
        let q = quota.map(|f| f());
        term.draw(|f| {
            draw(
                f,
                ui,
                snap_state,
                hint_state,
                research_state,
                q,
                opts.budget_ceiling,
            )
        })?;
        Ok(())
    };

    paint(terminal, &ui, &snap_state, &hint_state, &research_state)?;
    // `interval` yields its first tick immediately; without this reset
    // the first `select!` iteration would repaint again right after the
    // explicit paint above. Reset so the first floor-repaint lands one
    // full period out.
    repaint.reset();

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
                        // `ingest` returns true only when this snapshot
                        // should ring the bell (new WARNING and bell
                        // enabled); the `bell_enabled` gate is inside
                        // `ingest`, so we ring unconditionally here.
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
                paint(terminal, &ui, &snap_state, &hint_state, &research_state)?;
            }
            maybe = events.next() => {
                match maybe {
                    Some(Ok(CEvent::Key(key))) if is_actionable(&key) => {
                        match handle_key(key, &mut ui, &mut snap_state, &mut hint_state, &mut research_state) {
                            KeyOutcome::Quit => break,
                            KeyOutcome::Redraw => {
                                paint(terminal, &ui, &snap_state, &hint_state, &research_state)?;
                            }
                        }
                    }
                    Some(Ok(CEvent::Resize(_, _))) => {
                        paint(terminal, &ui, &snap_state, &hint_state, &research_state)?;
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
                paint(terminal, &ui, &snap_state, &hint_state, &research_state)?;
            }
        }
    }

    Ok(())
}
