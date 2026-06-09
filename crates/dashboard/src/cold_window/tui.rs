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
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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

/// Width at or above which the two-column layout becomes legible.
/// Below it the panes stack into a single full-width column (phones,
/// split panes); the actual narrow maximum is therefore one less than
/// this (`MEDIUM_MIN_WIDTH - 1`).
const MEDIUM_MIN_WIDTH: u16 = 60;
/// Width at or above which the roomy three-column layout is used. The
/// band in between ([`MEDIUM_MIN_WIDTH`], `WIDE_MIN_WIDTH`) is the
/// medium tier: still two columns, but the research column is slimmed
/// so the alert/hint text keeps its width.
const WIDE_MIN_WIDTH: u16 = 80;

/// Which of the three responsive tiers the current terminal falls into.
///
/// The tier is chosen by width alone: height never changes the
/// *topology* (status bar stays pinned, panes keep their relative
/// order), only how much room each pane gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutMode {
    /// `width >= 80`: alerts over hints in a 60% left column, research
    /// filling the 40% right column.
    Wide,
    /// `60 <= width < 80`: same two-column topology as [`LayoutMode::Wide`], but the
    /// research column is narrowed so the left text stays readable.
    Medium,
    /// `width < 60`: every pane full-width, stacked top to bottom.
    Narrow,
}

/// Pick the responsive tier for `area`.
pub fn layout_mode(area: Rect) -> LayoutMode {
    if area.width >= WIDE_MIN_WIDTH {
        LayoutMode::Wide
    } else if area.width >= MEDIUM_MIN_WIDTH {
        LayoutMode::Medium
    } else {
        LayoutMode::Narrow
    }
}

/// Where each pane is drawn for a given frame. Produced by
/// [`plan_layout`] and consumed by [`draw`]; pure and `Rect`-only so it
/// can be unit-tested without a terminal.
///
/// The fields are `pub` for ergonomic read access, but the load-bearing
/// invariants (status pinned to the bottom row; the four rects tile
/// `area`) are upheld only by the private constructors (`plan_for`,
/// `columnar`). `#[non_exhaustive]` keeps external crates from
/// hand-building an instance that violates them and reserves room for a
/// future tier without a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColdWindowLayout {
    /// Alert list region.
    pub alerts: Rect,
    /// Hint list region.
    pub hints: Rect,
    /// Research panel region.
    pub research: Rect,
    /// One-line status bar region (always pinned to the bottom row).
    pub status: Rect,
}

/// Plan pane rectangles for `area`, choosing the tier by width.
///
/// `research_collapsed` only affects the [`LayoutMode::Narrow`] stack,
/// where a collapsed research pane is a 3-line badge and an expanded
/// one earns a share of the vertical space.
pub fn plan_layout(area: Rect, research_collapsed: bool) -> ColdWindowLayout {
    plan_for(layout_mode(area), area, research_collapsed)
}

/// Plan pane rectangles for an explicit `mode`. Split out from
/// [`plan_layout`] so tests can compare tiers at a fixed `area`.
fn plan_for(mode: LayoutMode, area: Rect, research_collapsed: bool) -> ColdWindowLayout {
    // One-line status bar pinned to the bottom in every tier.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let body = rows[0];
    let status = rows[1];

    match mode {
        // Roomy three-column layout: 60% left (alerts over hints),
        // 40% research.
        LayoutMode::Wide => columnar(body, status, 60),
        // Same topology, but a slimmer research column buys the
        // alert/hint text back some width.
        LayoutMode::Medium => columnar(body, status, 68),
        // Phones / split panes: stack everything full-width. A
        // collapsed research pane is a fixed 3-line badge; expanded it
        // takes a share of the column.
        LayoutMode::Narrow => {
            let constraints = if research_collapsed {
                [
                    Constraint::Min(3),
                    Constraint::Min(3),
                    Constraint::Length(3),
                ]
            } else {
                [
                    Constraint::Percentage(35),
                    Constraint::Percentage(30),
                    Constraint::Percentage(35),
                ]
            };
            let stack = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(body);
            ColdWindowLayout {
                alerts: stack[0],
                hints: stack[1],
                research: stack[2],
                status,
            }
        }
    }
}

/// Two-column body: a `left_pct`% column with alerts (55%) over hints
/// (45%), and the research pane filling the remaining right column.
fn columnar(body: Rect, status: Rect, left_pct: u16) -> ColdWindowLayout {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(body);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(cols[0]);
    ColdWindowLayout {
        alerts: left[0],
        hints: left[1],
        research: cols[1],
        status,
    }
}

/// Compose the four panes into a single frame, adapting to the
/// terminal size.
///
/// The arrangement is chosen by [`plan_layout`] from the frame's width:
/// a roomy three-column layout on wide terminals, a slimmer two-column
/// variant in the medium band, and a full-width vertical stack on
/// phone-sized screens. The one-line status bar is pinned to the bottom
/// in every tier.
pub fn draw(
    frame: &mut Frame<'_>,
    snap_state: &ColdWindowState,
    hint_state: &HintPaneState,
    research_state: &ResearchPaneState,
    research_quota: Option<ResearchQuota>,
    budget_ceiling: u64,
) {
    let layout = plan_layout(frame.area(), research_state.collapsed);

    AlertPane::render(snap_state, frame, layout.alerts);
    HintPane::render(snap_state, hint_state, frame, layout.hints);
    ResearchPane::render(snap_state, research_state, frame, layout.research);
    StatusBar::render(
        snap_state,
        research_quota,
        budget_ceiling,
        frame,
        layout.status,
    );
}

/// Route a keystroke to the panes.
///
/// Global keys win first: `q`/`Esc`/`Ctrl-C` quit. Everything else is
/// forwarded to all three pane handlers, their keybindings are
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
        Alert, AlertBand, Hint, HintCategory, LoadSample, ResearchChannel, ResearchFinding,
        ScoredHint, Severity, TokenEntry, TokenLedger,
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
        // R9: the composite must not panic across terminal sizes, with
        // research both collapsed and expanded. The expanded pass at
        // `(44, 4)` drives the height-starved expanded-Narrow arm
        // (`Percentage(35/30/35)`) the collapsed default never reaches.
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        let hint_state = HintPaneState::new();
        let collapsed = ResearchPaneState::default();
        let mut expanded = ResearchPaneState::new();
        expanded.collapsed = false;
        for research_state in [&collapsed, &expanded] {
            for (w, h) in [(20u16, 5u16), (40, 12), (200, 60), (8, 2), (44, 4)] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal
                    .draw(|f| draw(f, &snap_state, &hint_state, research_state, None, 100_000))
                    .unwrap_or_else(|e| {
                        panic!(
                            "draw panicked at {w}x{h} (collapsed={}): {e}",
                            research_state.collapsed
                        )
                    });
            }
        }
    }

    #[test]
    fn first_paint_reflects_snapshot_under_500ms() {
        // SC3: startup-to-first-snapshot must beat 500 ms. The launch
        // path's cost is dominated by ingest and a single composite
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

    // --- Responsive layout (R9: adapt to terminal size) -------------

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn mode_thresholds_map_width_to_tier() {
        assert_eq!(layout_mode(area(80, 24)), LayoutMode::Wide);
        assert_eq!(layout_mode(area(120, 40)), LayoutMode::Wide);
        assert_eq!(layout_mode(area(79, 24)), LayoutMode::Medium);
        assert_eq!(layout_mode(area(60, 24)), LayoutMode::Medium);
        assert_eq!(layout_mode(area(59, 24)), LayoutMode::Narrow);
        assert_eq!(layout_mode(area(40, 30)), LayoutMode::Narrow);
    }

    #[test]
    fn status_bar_is_always_one_line_at_the_bottom() {
        for (w, h) in [(120u16, 40u16), (70, 30), (44, 30)] {
            let l = plan_layout(area(w, h), true);
            assert_eq!(l.status.height, 1, "{w}x{h}: status must be one row");
            assert_eq!(l.status.width, w, "{w}x{h}: status spans full width");
            assert_eq!(
                l.status.y,
                h - 1,
                "{w}x{h}: status pinned to the bottom row"
            );
        }
    }

    #[test]
    fn wide_and_medium_are_columnar() {
        for mode in [LayoutMode::Wide, LayoutMode::Medium] {
            let l = plan_for(mode, area(100, 40), true);
            // Research sits to the right of the alert/hint column.
            assert!(
                l.research.x > l.alerts.x,
                "{mode:?}: research must be the right column"
            );
            // Alerts stack above hints in a shared left column.
            assert_eq!(l.alerts.x, l.hints.x, "{mode:?}: left column shared");
            assert!(l.alerts.y < l.hints.y, "{mode:?}: alerts above hints");
        }
    }

    #[test]
    fn medium_gives_left_panes_more_room_than_wide() {
        // Width 70 is inside the Medium band (60..=79), so the 68%
        // left-column constant is exercised at a size Medium actually
        // occupies, not at 100, which `layout_mode` routes to Wide.
        let wide = plan_for(LayoutMode::Wide, area(70, 40), true);
        let medium = plan_for(LayoutMode::Medium, area(70, 40), true);
        assert!(
            medium.alerts.width > wide.alerts.width,
            "medium left column ({}) should be wider than wide ({})",
            medium.alerts.width,
            wide.alerts.width
        );
        assert!(
            medium.research.width < wide.research.width,
            "medium research ({}) should be slimmer than wide ({})",
            medium.research.width,
            wide.research.width
        );
    }

    #[test]
    fn research_collapsed_only_affects_the_narrow_stack() {
        // `plan_layout`'s contract says `research_collapsed` reshapes
        // only the Narrow vertical stack. Pin it: the two columnar tiers
        // must produce byte-identical layouts regardless of the flag.
        for mode in [LayoutMode::Wide, LayoutMode::Medium] {
            assert_eq!(
                plan_for(mode, area(100, 40), true),
                plan_for(mode, area(100, 40), false),
                "{mode:?}: research_collapsed must not change the columnar layout"
            );
        }
        // ...and it *does* change the Narrow stack, so the assertion
        // above is guarding a real distinction, not a constant.
        assert_ne!(
            plan_for(LayoutMode::Narrow, area(44, 30), true),
            plan_for(LayoutMode::Narrow, area(44, 30), false),
            "Narrow: research_collapsed must reshape the stack"
        );
    }

    #[test]
    fn narrow_stacks_panes_in_a_single_full_width_column() {
        let w = 44;
        let l = plan_layout(area(w, 30), true);
        for (name, r) in [
            ("alerts", l.alerts),
            ("hints", l.hints),
            ("research", l.research),
        ] {
            assert_eq!(r.x, 0, "{name} must start at the left edge");
            assert_eq!(r.width, w, "{name} must span the full width");
        }
        assert!(l.alerts.y < l.hints.y, "alerts above hints");
        assert!(l.hints.y < l.research.y, "hints above research");
    }

    #[test]
    fn narrow_research_grows_when_expanded() {
        let collapsed = plan_layout(area(44, 30), true);
        let expanded = plan_layout(area(44, 30), false);
        assert_eq!(
            collapsed.research.height, 3,
            "collapsed research is a 3-line badge"
        );
        assert!(
            expanded.research.height > collapsed.research.height,
            "expanded research ({}) should be taller than collapsed ({})",
            expanded.research.height,
            collapsed.research.height
        );
    }

    /// Flatten a rendered `TestBackend` buffer into one plain string so
    /// tests can assert on visible text without caring about cell
    /// coordinates.
    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    /// `rich_snapshot` with a single research finding attached, so the
    /// expanded research pane has a row to render.
    fn snapshot_with_research() -> Arc<WindowSnapshot> {
        let mut snap = (*rich_snapshot()).clone();
        snap.research_findings = vec![ResearchFinding {
            fingerprint: "fp1".into(),
            channel: ResearchChannel::GitHub,
            title: "responsive layout".into(),
            url: "https://example.com/fp1".into(),
            score: 9.0,
            fetched_at_ms: 0,
        }];
        Arc::new(snap)
    }

    #[test]
    fn draw_renders_expanded_research_in_narrow_and_medium_tiers() {
        // The pure-function tests pin the *geometry* of the Medium tier
        // and the narrow expanded-research arm, but neither is ever
        // pushed through the real `ResearchPane::render`. A regression
        // that handed an expanded pane the collapsed badge's 3-line slot
        // would satisfy the geometry tests yet truncate the live render.
        // Drive both tiers through `draw` to guard the seam.
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snapshot_with_research());
        let hint_state = HintPaneState::new();
        // collapsed: false exercises the Percentage(35/30/35) narrow arm
        // and the expanded render path that lists findings.
        let research_state = ResearchPaneState {
            collapsed: false,
            ..ResearchPaneState::default()
        };

        // Narrow tier (width < 60): expanded research stacks full-width
        // with real vertical room. Its title carries the "findings"
        // marker that the collapsed badge ("press R to expand") never
        // shows, proving the expanded path reached the row renderer.
        let mut narrow = Terminal::new(TestBackend::new(44, 30)).unwrap();
        narrow
            .draw(|f| draw(f, &snap_state, &hint_state, &research_state, None, 100_000))
            .expect("narrow expanded draw must not panic");
        assert!(
            buffer_text(&narrow).contains("findings"),
            "expanded research must render its findings list in the narrow stack"
        );

        // Medium tier (60 <= width < 80): the slimmer columnar arm that
        // no `draw` test exercised before. Guard it against renderer
        // panics with the expanded pane in the right column.
        let mut medium = Terminal::new(TestBackend::new(70, 30)).unwrap();
        medium
            .draw(|f| draw(f, &snap_state, &hint_state, &research_state, None, 100_000))
            .expect("medium expanded draw must not panic");
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
