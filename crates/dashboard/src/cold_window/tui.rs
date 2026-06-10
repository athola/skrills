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

use super::focus::FocusTarget;
use super::overlay::{self, Overlay, OverlayStack};
use super::{
    AlertPane, ColdWindowState, HintPane, HintPaneState, ResearchPane, ResearchPaneState, StatusBar,
};

/// Interface-level state that belongs to the TUI shell rather than any
/// pane: which pane holds focus and the modal overlay stack (and, in a
/// later increment, the zoom flag). Kept separate from pane state so
/// panes stay ignorant of the focus and overlay models.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiState {
    /// The pane currently holding focus (default: alerts).
    pub focus: FocusTarget,
    /// Open modal overlays; the topmost consumes all keys.
    pub overlays: OverlayStack,
    /// Per-pane selection cursors (FR-5). Stored raw and clamped
    /// against the live list length at render/use time, since lists
    /// shrink between snapshots.
    pub selected: SelectionState,
    /// When true the focused pane takes the whole body (FR-5.2); the
    /// escape hatch for tiny terminals. `z` toggles, `Esc` clears.
    pub zoomed: bool,
}

/// One selection index per pane. Indices persist across focus changes
/// so returning to a pane lands on the row the user left.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectionState {
    /// Cursor into the visible-alerts list.
    pub alerts: usize,
    /// Cursor into the visible-hints list.
    pub hints: usize,
    /// Cursor into the research findings list.
    pub research: usize,
}

impl SelectionState {
    /// The cursor for `focus`, mutably.
    pub fn for_focus_mut(&mut self, focus: FocusTarget) -> &mut usize {
        match focus {
            FocusTarget::Alerts => &mut self.alerts,
            FocusTarget::Hints => &mut self.hints,
            FocusTarget::Research => &mut self.research,
        }
    }

    /// The cursor for `focus`.
    pub fn for_focus(&self, focus: FocusTarget) -> usize {
        match focus {
            FocusTarget::Alerts => self.alerts,
            FocusTarget::Hints => self.hints,
            FocusTarget::Research => self.research,
        }
    }
}

impl UiState {
    /// Fresh interface state: focus on alerts.
    pub fn new() -> Self {
        Self::default()
    }
}

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

/// Width at or above which the stacked Narrow layout still has room
/// for three readable panes. Below it (phone SSH sessions, slim split
/// panes) the Compact tier shows only the focused pane: hiding panes
/// beats squeezing them into unreadable slivers (TR-003).
const NARROW_MIN_WIDTH: u16 = 45;
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
    /// `45 <= width < 60`: every pane full-width, stacked top to
    /// bottom.
    Narrow,
    /// `width < 45`: only the focused pane plus the status bar; focus
    /// is visibility (`Tab` switches which pane shows). The mobile
    /// tier (FR-6).
    Compact,
}

/// Pick the responsive tier for `area`.
pub fn layout_mode(area: Rect) -> LayoutMode {
    if area.width >= WIDE_MIN_WIDTH {
        LayoutMode::Wide
    } else if area.width >= MEDIUM_MIN_WIDTH {
        LayoutMode::Medium
    } else if area.width >= NARROW_MIN_WIDTH {
        LayoutMode::Narrow
    } else {
        LayoutMode::Compact
    }
}

/// Hard floor below which panes are unrenderable; [`draw`] shows a
/// one-line guard message instead of clipped pane fragments (FR-6.3).
const GUARD_MIN_WIDTH: u16 = 20;
/// See [`GUARD_MIN_WIDTH`].
const GUARD_MIN_HEIGHT: u16 = 6;

/// True when `area` is below the renderable floor.
pub fn below_size_floor(area: Rect) -> bool {
    area.width < GUARD_MIN_WIDTH || area.height < GUARD_MIN_HEIGHT
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
    plan_layout_with(area, research_collapsed, None)
}

/// [`plan_layout`] with an optional zoom override (FR-5.2/FR-5.3).
///
/// `zoom = Some(pane)` hands that pane the entire body at every tier;
/// the other panes get zero-sized rects (ratatui draws nothing into an
/// empty `Rect`). The status bar stays pinned regardless.
///
/// The Compact tier (FR-6) reuses the same single-pane shape: below
/// [`NARROW_MIN_WIDTH`] columns the focused pane is the layout,
/// whether or not the user zoomed.
pub fn plan_layout_with(
    area: Rect,
    research_collapsed: bool,
    zoom: Option<FocusTarget>,
) -> ColdWindowLayout {
    let mode = layout_mode(area);
    let pane = match (zoom, mode) {
        (Some(p), _) => p,
        // Compact without an explicit zoom target falls back to the
        // first pane; `draw` always passes the focused pane here.
        (None, LayoutMode::Compact) => FocusTarget::Alerts,
        (None, _) => return plan_for(mode, area, research_collapsed),
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let body = rows[0];
    let status = rows[1];
    let none = Rect::default();
    ColdWindowLayout {
        alerts: if pane == FocusTarget::Alerts {
            body
        } else {
            none
        },
        hints: if pane == FocusTarget::Hints {
            body
        } else {
            none
        },
        research: if pane == FocusTarget::Research {
            body
        } else {
            none
        },
        status,
    }
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
        // Single-pane tier: handled by `plan_layout_with`, which knows
        // the focused pane; reaching here without one means a direct
        // `plan_for` call, where alerts is the only sensible default.
        LayoutMode::Compact => ColdWindowLayout {
            alerts: body,
            hints: Rect::default(),
            research: Rect::default(),
            status,
        },
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
    ui: &UiState,
    snap_state: &ColdWindowState,
    hint_state: &HintPaneState,
    research_state: &ResearchPaneState,
    research_quota: Option<ResearchQuota>,
    budget_ceiling: u64,
) {
    let area = frame.area();
    // Below the hard floor nothing renders usefully: show the guard
    // line instead of clipped pane fragments (FR-6.3).
    if below_size_floor(area) {
        frame.render_widget(
            ratatui::widgets::Paragraph::new(format!(
                "terminal too small (need >= {GUARD_MIN_WIDTH}x{GUARD_MIN_HEIGHT})"
            )),
            area,
        );
        return;
    }

    // Compact (FR-6) and zoom (FR-5.2) share the single-pane layout:
    // either way the focused pane owns the body.
    let compact = layout_mode(area) == LayoutMode::Compact;
    let layout = plan_layout_with(
        area,
        research_state.collapsed,
        (ui.zoomed || compact).then_some(ui.focus),
    );

    // The selection cursor renders only on the focused pane; unfocused
    // panes keep a plain gutter so the cursor reads as "where I am".
    let cursor = |pane: FocusTarget| (ui.focus == pane).then(|| ui.selected.for_focus(pane));

    AlertPane::render(
        snap_state,
        frame,
        layout.alerts,
        ui.focus == FocusTarget::Alerts,
        cursor(FocusTarget::Alerts),
    );
    HintPane::render(
        snap_state,
        hint_state,
        frame,
        layout.hints,
        ui.focus == FocusTarget::Hints,
        cursor(FocusTarget::Hints),
    );
    ResearchPane::render(
        snap_state,
        research_state,
        frame,
        layout.research,
        ui.focus == FocusTarget::Research,
        cursor(FocusTarget::Research),
    );
    StatusBar::render(
        snap_state,
        research_quota,
        budget_ceiling,
        ui.focus,
        ui.overlays.top(),
        frame,
        layout.status,
    );

    // Modal surfaces draw last, over the panes (FR-4.3).
    overlay::render(&ui.overlays, ui.focus, frame);
}

/// Route a keystroke to the interface state and panes.
///
/// Routing order (FR-4):
///
/// 1. `Ctrl-C` always quits.
/// 2. `q` closes the topmost overlay; at the base surface it quits.
/// 3. `Esc` closes the topmost overlay; at the base surface it does
///    nothing (BREAKING since 0.8.x: `Esc` no longer quits).
/// 4. An open overlay consumes every other key.
/// 5. Globals: `Tab`/`Shift-Tab` move focus.
/// 6. Everything else is forwarded to all three pane handlers; their
///    keybindings are disjoint (`A`/`d` alerts, `0`-`5`/`P` hints,
///    `R` research), so focus does not gate them (FR-1.4): focus
///    governs only what the hint bar describes and what `Enter`/`z`
///    target.
pub fn handle_key(
    key: KeyEvent,
    ui: &mut UiState,
    snap_state: &mut ColdWindowState,
    hint_state: &mut HintPaneState,
    research_state: &mut ResearchPaneState,
) -> KeyOutcome {
    let ctrl_c = key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'));
    if ctrl_c {
        return KeyOutcome::Quit;
    }

    // The palette is a text-input surface: it must see raw characters
    // (including `q` and `?`) before any global binding fires.
    if matches!(ui.overlays.top(), Some(Overlay::Palette { .. })) {
        return handle_palette_key(key.code, ui, snap_state, hint_state, research_state);
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            return if ui.overlays.pop().is_some() {
                KeyOutcome::Redraw
            } else {
                KeyOutcome::Quit
            };
        }
        KeyCode::Esc => {
            // Pops the topmost overlay; with none open it unzooms;
            // harmless no-op at the bare base surface.
            if ui.overlays.pop().is_none() {
                ui.zoomed = false;
            }
            return KeyOutcome::Redraw;
        }
        _ => {}
    }

    // `?` toggles help: opens it at the base, closes it when help is
    // already the topmost overlay (FR-3.1, FR-3.3).
    if key.code == KeyCode::Char('?') {
        if matches!(ui.overlays.top(), Some(Overlay::Help)) {
            let _ = ui.overlays.pop();
        } else {
            ui.overlays.push(Overlay::Help);
        }
        return KeyOutcome::Redraw;
    }

    // `:` opens the command palette (the novice-expert bridge; k9s
    // pattern). The palette branch above then owns the keyboard.
    if key.code == KeyCode::Char(':') {
        ui.overlays.push(Overlay::Palette {
            query: String::new(),
            selected: 0,
        });
        return KeyOutcome::Redraw;
    }

    // The topmost overlay holds the keyboard: pane keys must not leak
    // underneath it (FR-3.3, FR-4.1).
    if !ui.overlays.is_empty() {
        return KeyOutcome::Redraw;
    }

    match key.code {
        KeyCode::Tab => {
            ui.focus = ui.focus.next();
            return KeyOutcome::Redraw;
        }
        KeyCode::BackTab => {
            ui.focus = ui.focus.prev();
            return KeyOutcome::Redraw;
        }
        // Zoom the focused pane to the full body (FR-5.2).
        KeyCode::Char('z') => {
            ui.zoomed = !ui.zoomed;
            return KeyOutcome::Redraw;
        }
        // Drill into the focused pane's selected item (FR-5.1).
        KeyCode::Enter => {
            if let Some(detail) = detail_overlay(ui, snap_state, hint_state) {
                ui.overlays.push(detail);
            }
            return KeyOutcome::Redraw;
        }
        // Selection moves within the focused pane only (FR-5).
        KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
            let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
            let len = focused_list_len(ui.focus, snap_state, hint_state);
            let cursor = ui.selected.for_focus_mut(ui.focus);
            *cursor = super::focus::step_selection(*cursor, down, len);
            return KeyOutcome::Redraw;
        }
        _ => {}
    }

    // Disjoint keymaps: forwarding the same code to each handler is
    // safe because at most one will act on it.
    let _ = AlertPane::handle_key(snap_state, key.code);
    let _ = HintPane::handle_key(snap_state, hint_state, key.code);
    let _ = ResearchPane::handle_key(snap_state, research_state, key.code);
    KeyOutcome::Redraw
}

/// Length of the list the focused pane is showing, for selection
/// clamping. The research pane counts its findings whether or not the
/// pane is expanded (the cursor is simply invisible while collapsed).
fn focused_list_len(
    focus: FocusTarget,
    snap_state: &ColdWindowState,
    hint_state: &HintPaneState,
) -> usize {
    match focus {
        FocusTarget::Alerts => snap_state.visible_alerts().len(),
        FocusTarget::Hints => hint_state.visible_hints(snap_state).len(),
        FocusTarget::Research => snap_state
            .current
            .as_deref()
            .map(|s| s.research_findings.len())
            .unwrap_or(0),
    }
}

/// Keystroke routing while the command palette is the topmost overlay.
///
/// Characters edit the query, `Up`/`Down` move the selection within
/// the filtered list, `Enter` closes the palette and replays the
/// selected command's key through [`handle_key`] (so palette execution
/// can never drift from what the key itself does), and `Esc` closes
/// without running anything.
fn handle_palette_key(
    code: KeyCode,
    ui: &mut UiState,
    snap_state: &mut ColdWindowState,
    hint_state: &mut HintPaneState,
    research_state: &mut ResearchPaneState,
) -> KeyOutcome {
    use super::focus::{clamped_selection, step_selection};
    use super::keymap::palette_matches;

    // Take the palette off the stack, mutate, and decide whether to
    // put it back; avoids aliasing the stack while editing its top.
    let Some(Overlay::Palette {
        mut query,
        mut selected,
    }) = ui.overlays.pop()
    else {
        return KeyOutcome::Redraw;
    };

    match code {
        KeyCode::Esc => KeyOutcome::Redraw, // closed, nothing run
        KeyCode::Enter => {
            let matches = palette_matches(&query);
            let Some(index) = clamped_selection(selected, matches.len()) else {
                // No matching command: keep the palette open so the
                // user can fix the query.
                ui.overlays.push(Overlay::Palette { query, selected });
                return KeyOutcome::Redraw;
            };
            let replay = matches[index].code;
            handle_key(
                KeyEvent::new(replay, KeyModifiers::NONE),
                ui,
                snap_state,
                hint_state,
                research_state,
            )
        }
        KeyCode::Up | KeyCode::Down => {
            let len = palette_matches(&query).len();
            selected = step_selection(selected, code == KeyCode::Down, len);
            ui.overlays.push(Overlay::Palette { query, selected });
            KeyOutcome::Redraw
        }
        KeyCode::Backspace => {
            query.pop();
            selected = 0;
            ui.overlays.push(Overlay::Palette { query, selected });
            KeyOutcome::Redraw
        }
        KeyCode::Char(c) => {
            query.push(c);
            selected = 0;
            ui.overlays.push(Overlay::Palette { query, selected });
            KeyOutcome::Redraw
        }
        _ => {
            ui.overlays.push(Overlay::Palette { query, selected });
            KeyOutcome::Redraw
        }
    }
}

/// Build the drill-down detail overlay for the focused pane's selected
/// item (FR-5.1). `None` when the focused list is empty, so `Enter`
/// degrades to a no-op instead of opening a blank popup.
fn detail_overlay(
    ui: &UiState,
    snap_state: &ColdWindowState,
    hint_state: &HintPaneState,
) -> Option<Overlay> {
    let index = ui.selected.for_focus(ui.focus);
    match ui.focus {
        FocusTarget::Alerts => {
            let visible = snap_state.visible_alerts();
            let alert = visible.get(super::focus::clamped_selection(index, visible.len())?)?;
            Some(Overlay::Detail {
                title: alert.title.clone(),
                lines: vec![
                    format!("severity:    {}", alert.severity.short_label()),
                    format!("fingerprint: {}", alert.fingerprint),
                    format!("fired at:    {} ms", alert.fired_at_ms),
                    format!("dwell ticks: {}", alert.dwell_ticks),
                    String::new(),
                    alert.message.clone(),
                ],
            })
        }
        FocusTarget::Hints => {
            let visible = hint_state.visible_hints(snap_state);
            let hint = visible.get(super::focus::clamped_selection(index, visible.len())?)?;
            Some(Overlay::Detail {
                title: hint.hint.uri.clone(),
                lines: vec![
                    format!("category:  {}", hint.hint.category.label()),
                    format!("score:     {:.1}", hint.score),
                    format!("frequency: {}", hint.hint.frequency),
                    format!("impact:    {:.1}", hint.hint.impact),
                    format!("ease:      {:.1}", hint.hint.ease_score),
                    format!("age:       {:.1} days", hint.hint.age_days),
                    String::new(),
                    hint.hint.message.clone(),
                ],
            })
        }
        FocusTarget::Research => {
            let snap = snap_state.current.as_deref()?;
            let findings = &snap.research_findings;
            let finding = findings.get(super::focus::clamped_selection(index, findings.len())?)?;
            Some(Overlay::Detail {
                title: finding.title.clone(),
                lines: vec![
                    format!("channel:    {}", finding.channel.short_label()),
                    format!("score:      {:.1}", finding.score),
                    format!("fetched at: {} ms", finding.fetched_at_ms),
                    String::new(),
                    finding.url.clone(),
                ],
            })
        }
    }
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
        let ui = UiState::new();
        terminal
            .draw(|f| {
                draw(
                    f,
                    &ui,
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
        let ui = UiState::new();
        let collapsed = ResearchPaneState::default();
        let mut expanded = ResearchPaneState::new();
        expanded.collapsed = false;
        for research_state in [&collapsed, &expanded] {
            for (w, h) in [(20u16, 5u16), (40, 12), (200, 60), (8, 2), (44, 4)] {
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal
                    .draw(|f| {
                        draw(
                            f,
                            &ui,
                            &snap_state,
                            &hint_state,
                            research_state,
                            None,
                            100_000,
                        )
                    })
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
        let ui = UiState::new();

        let t0 = Instant::now();
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        terminal
            .draw(|f| {
                draw(
                    f,
                    &ui,
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
        assert_eq!(layout_mode(area(45, 24)), LayoutMode::Narrow);
        assert_eq!(layout_mode(area(44, 24)), LayoutMode::Compact);
        assert_eq!(layout_mode(area(40, 30)), LayoutMode::Compact);
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
        let w = 50;
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
        let collapsed = plan_layout(area(50, 30), true);
        let expanded = plan_layout(area(50, 30), false);
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
        let ui = UiState::new();

        // Narrow tier (width < 60): expanded research stacks full-width
        // with real vertical room. Its title carries the "findings"
        // marker that the collapsed badge ("press R to expand") never
        // shows, proving the expanded path reached the row renderer.
        let mut narrow = Terminal::new(TestBackend::new(50, 30)).unwrap();
        narrow
            .draw(|f| {
                draw(
                    f,
                    &ui,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    None,
                    100_000,
                )
            })
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
            .draw(|f| {
                draw(
                    f,
                    &ui,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    None,
                    100_000,
                )
            })
            .expect("medium expanded draw must not panic");
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn q_quits_at_base_but_esc_does_not() {
        // BREAKING (FR-4.2): Esc stopped quitting when the overlay
        // stack landed; it only closes overlays now.
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        assert_eq!(
            handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r),
            KeyOutcome::Redraw,
            "Esc at the base surface must not quit"
        );
        assert_eq!(
            handle_key(key(KeyCode::Char('q')), &mut ui, &mut s, &mut h, &mut r),
            KeyOutcome::Quit
        );
    }

    #[test]
    fn esc_and_q_pop_overlays_before_anything_else() {
        // FR-4.2: Esc pops; q pops too and only quits at the base.
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();

        ui.overlays.push(Overlay::Help);
        ui.overlays.push(Overlay::Help);
        assert_eq!(
            handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r),
            KeyOutcome::Redraw
        );
        assert_eq!(
            handle_key(key(KeyCode::Char('q')), &mut ui, &mut s, &mut h, &mut r),
            KeyOutcome::Redraw,
            "q with an overlay open closes it instead of quitting"
        );
        assert!(ui.overlays.is_empty(), "both overlays popped");
        assert_eq!(
            handle_key(key(KeyCode::Char('q')), &mut ui, &mut s, &mut h, &mut r),
            KeyOutcome::Quit,
            "q at the base quits"
        );
    }

    #[test]
    fn open_overlay_consumes_pane_and_focus_keys() {
        // FR-4.1: the topmost overlay holds the keyboard; pane state
        // and focus must not change underneath it.
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        s.ingest(rich_snapshot());
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        ui.overlays.push(Overlay::Help);

        let warnings_before = s.visible_alerts().len();
        handle_key(key(KeyCode::Char('d')), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(
            s.visible_alerts().len(),
            warnings_before,
            "'d' must not reach the alert pane through an overlay"
        );
        handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(
            ui.focus,
            FocusTarget::Alerts,
            "Tab must not move focus through an overlay"
        );
        handle_key(key(KeyCode::Char('R')), &mut ui, &mut s, &mut h, &mut r);
        assert!(
            r.collapsed,
            "'R' must not toggle the research pane through an overlay"
        );
    }

    /// Snapshot with two warnings so the alert list has two rows to
    /// move a selection across.
    fn two_warning_snapshot() -> Arc<WindowSnapshot> {
        let mut snap = (*rich_snapshot()).clone();
        snap.alerts = vec![
            Alert {
                fingerprint: "w-first".into(),
                severity: Severity::Warning,
                title: "first-alert".into(),
                message: "m1".into(),
                band: None,
                fired_at_ms: 200,
                dwell_ticks: 1,
            },
            Alert {
                fingerprint: "w-second".into(),
                severity: Severity::Warning,
                title: "second-alert".into(),
                message: "m2".into(),
                band: None,
                fired_at_ms: 100,
                dwell_ticks: 1,
            },
        ];
        Arc::new(snap)
    }

    /// One terminal row as trimmed text.
    fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buf = terminal.backend().buffer();
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
    }

    #[test]
    fn jk_and_arrows_move_selection_in_the_focused_pane_only() {
        // FR-5/T5: selection keys act on the focused pane's cursor and
        // leave the other panes' cursors untouched.
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        s.ingest(two_warning_snapshot());
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();

        handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.selected.alerts, 1, "j moves the alerts cursor down");
        assert_eq!(ui.selected.hints, 0, "hints cursor untouched");
        handle_key(key(KeyCode::Char('k')), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.selected.alerts, 0, "k moves it back up");
        handle_key(key(KeyCode::Down), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.selected.alerts, 1, "Down mirrors j");
        handle_key(key(KeyCode::Up), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.selected.alerts, 0, "Up mirrors k");

        // Clamping: two items means the cursor never reaches index 2.
        handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.selected.alerts, 1, "cursor clamps at the last row");

        // Focus hints: same keys now drive the hints cursor (one hint
        // in the snapshot, so it stays clamped at 0).
        handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.selected.hints, 0, "single hint clamps at 0");
        assert_eq!(ui.selected.alerts, 1, "alerts cursor persists");
    }

    #[test]
    fn selected_row_carries_a_cursor_marker_visible_without_color() {
        // FR-5/T5: the `> ` row marker must sit on exactly the selected
        // row of the focused pane.
        let mut s = ColdWindowState::new();
        s.ingest(two_warning_snapshot());
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();
        let mut ui = UiState::new();
        ui.selected.alerts = 1;

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|f| draw(f, &ui, &s, &hint_state, &research_state, None, 100_000))
            .unwrap();
        // Alert rows start under the top border: row 1 is the first
        // alert, row 2 the second (selected) one.
        // Rows begin with the pane's left border glyph; the marker (or
        // its two-space gutter) comes immediately after it.
        let first = row_text(&terminal, 1);
        let second = row_text(&terminal, 2);
        assert!(
            second.starts_with("│>"),
            "selected row must carry the > marker after the border, got: {second:?}"
        );
        assert!(
            !first.contains('>'),
            "unselected row must not carry the marker, got: {first:?}"
        );
        assert!(second.contains("second-alert"), "marker on the right row");
    }

    #[test]
    fn compact_tier_draws_only_the_focused_pane() {
        // FR-6.1/FR-6.2: below 45 columns, focus is visibility; the
        // other panes' titles must not appear anywhere on the frame.
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();
        let mut ui = UiState::new();
        ui.focus = FocusTarget::Hints;

        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    &ui,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    None,
                    100_000,
                )
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("> Hints"), "focused pane visible: {text}");
        assert!(
            !text.contains("Alerts  W:"),
            "alert pane must be hidden in compact, got: {text}"
        );
    }

    #[test]
    fn size_guard_replaces_panes_below_the_floor() {
        // FR-6.3: 20x6 is the floor; below it (either dimension) the
        // guard message is the whole frame.
        let snap_state = ColdWindowState::new();
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();
        let ui = UiState::new();

        let render_at = |w: u16, h: u16| -> String {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| {
                    draw(
                        f,
                        &ui,
                        &snap_state,
                        &hint_state,
                        &research_state,
                        None,
                        100_000,
                    )
                })
                .unwrap();
            buffer_text(&terminal)
        };

        assert!(
            render_at(19, 10).contains("terminal too small"),
            "width below floor must show the guard"
        );
        assert!(
            render_at(30, 5).contains("terminal too small"),
            "height below floor must show the guard"
        );
        assert!(
            !render_at(20, 6).contains("terminal too small"),
            "exactly at the floor the panes render"
        );
    }

    #[test]
    fn zoom_gives_the_focused_pane_the_full_body_at_every_tier() {
        // FR-5.2/FR-5.3: zoom is a layout-level override, independent
        // of the responsive tier; the status bar stays pinned.
        for (w, h) in [(44u16, 30u16), (70, 30), (120, 40)] {
            let a = area(w, h);
            let l = plan_layout_with(a, true, Some(FocusTarget::Hints));
            assert_eq!(
                l.hints,
                Rect::new(0, 0, w, h - 1),
                "{w}x{h}: zoomed pane must own the full body"
            );
            assert_eq!(l.alerts, Rect::default(), "{w}x{h}: alerts hidden");
            assert_eq!(l.research, Rect::default(), "{w}x{h}: research hidden");
            assert_eq!(l.status.y, h - 1, "{w}x{h}: status stays pinned");
        }
    }

    #[test]
    fn z_toggles_zoom_and_esc_unzooms_only_with_no_overlay() {
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();

        handle_key(key(KeyCode::Char('z')), &mut ui, &mut s, &mut h, &mut r);
        assert!(ui.zoomed, "z zooms");

        // An open overlay absorbs Esc first; zoom survives.
        ui.overlays.push(Overlay::Help);
        handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r);
        assert!(ui.zoomed, "Esc closes the overlay before unzooming");
        assert!(ui.overlays.is_empty());

        handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r);
        assert!(!ui.zoomed, "Esc at the base unzooms");

        handle_key(key(KeyCode::Char('z')), &mut ui, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Char('z')), &mut ui, &mut s, &mut h, &mut r);
        assert!(!ui.zoomed, "z toggles back off");
    }

    #[test]
    fn hint_bar_tracks_focus_and_overlays() {
        // FR-2: the bottom row shows the focused pane's keys, swaps to
        // overlay keys while one is open, and always offers `? help`.
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();

        let bottom_row = |ui: &UiState, w: u16, h: u16| -> String {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| {
                    draw(
                        f,
                        ui,
                        &snap_state,
                        &hint_state,
                        &research_state,
                        None,
                        100_000,
                    )
                })
                .unwrap();
            row_text(&terminal, h - 1)
        };

        let alerts = bottom_row(&UiState::new(), 120, 40);
        assert!(alerts.contains("? help"), "got: {alerts}");
        assert!(
            alerts.contains("ack all non-warnings"),
            "alerts focus shows alert keys, got: {alerts}"
        );

        let mut hints_ui = UiState::new();
        hints_ui.focus = FocusTarget::Hints;
        let hints = bottom_row(&hints_ui, 120, 40);
        assert!(
            hints.contains("P pin top hint"),
            "hints focus shows hint keys, got: {hints}"
        );
        assert!(
            !hints.contains("ack all non-warnings"),
            "alert keys must leave when focus moves, got: {hints}"
        );

        let mut overlay_ui = UiState::new();
        overlay_ui.overlays.push(Overlay::Help);
        let with_overlay = bottom_row(&overlay_ui, 120, 40);
        assert!(
            with_overlay.contains("Esc close"),
            "overlay keys replace pane keys, got: {with_overlay}"
        );
        assert!(
            !with_overlay.contains("ack all"),
            "pane keys hidden under an overlay, got: {with_overlay}"
        );

        // FR-2.3: at width 40 the hints truncate with an ellipsis but
        // `? help` survives.
        let narrow = bottom_row(&UiState::new(), 40, 12);
        assert!(narrow.contains("? help"), "got: {narrow:?}");
        assert!(narrow.contains('…'), "truncation marked, got: {narrow:?}");

        // T11: the palette gets its own hint line; `q close` would be
        // a lie there since q types into the query.
        let mut palette_ui = UiState::new();
        palette_ui.overlays.push(Overlay::Palette {
            query: String::new(),
            selected: 0,
        });
        let with_palette = bottom_row(&palette_ui, 120, 40);
        assert!(
            with_palette.contains("Enter run"),
            "palette hints shown, got: {with_palette}"
        );
        assert!(
            !with_palette.contains("q close"),
            "generic overlay hints would mislead in the palette, got: {with_palette}"
        );
    }

    #[test]
    fn enter_opens_detail_for_the_selected_item_and_esc_returns() {
        // FR-5.1/T6: Enter drills into the focused pane's selection;
        // Esc lands back on the unchanged base surface.
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        s.ingest(two_warning_snapshot());
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();

        handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
        match ui.overlays.top() {
            Some(Overlay::Detail { title, lines }) => {
                assert_eq!(title, "second-alert", "detail shows the selected item");
                assert!(
                    lines.iter().any(|l| l.contains("w-second")),
                    "detail body carries the fingerprint"
                );
            }
            other => panic!("expected a Detail overlay, got {other:?}"),
        }
        handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r);
        assert!(ui.overlays.is_empty(), "Esc returns to the base surface");

        // Hints pane: Enter opens the hint's detail.
        handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
        match ui.overlays.top() {
            Some(Overlay::Detail { title, .. }) => {
                assert_eq!(title, "skill://refactor", "hint detail titled by URI");
            }
            other => panic!("expected a hint Detail overlay, got {other:?}"),
        }
    }

    #[test]
    fn enter_on_an_empty_list_is_a_noop() {
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new(); // no snapshot at all
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
        assert!(
            ui.overlays.is_empty(),
            "Enter with nothing selected must not open a blank popup"
        );
    }

    #[test]
    fn colon_opens_the_palette_and_typed_keys_edit_the_query() {
        // T11: inside the palette, `q` and `?` are text, not globals.
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        s.ingest(rich_snapshot());
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();

        handle_key(key(KeyCode::Char(':')), &mut ui, &mut s, &mut h, &mut r);
        assert!(matches!(ui.overlays.top(), Some(Overlay::Palette { .. })));

        let outcome = handle_key(key(KeyCode::Char('q')), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(
            outcome,
            KeyOutcome::Redraw,
            "q must not quit in the palette"
        );
        handle_key(key(KeyCode::Char('?')), &mut ui, &mut s, &mut h, &mut r);
        match ui.overlays.top() {
            Some(Overlay::Palette { query, .. }) => {
                assert_eq!(query, "q?", "typed characters land in the query")
            }
            other => panic!("palette must stay open, got {other:?}"),
        }

        handle_key(key(KeyCode::Backspace), &mut ui, &mut s, &mut h, &mut r);
        match ui.overlays.top() {
            Some(Overlay::Palette { query, .. }) => assert_eq!(query, "q"),
            other => panic!("expected palette, got {other:?}"),
        }

        handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r);
        assert!(ui.overlays.is_empty(), "Esc closes the palette");
    }

    #[test]
    fn palette_enter_replays_the_selected_command() {
        // T11/TR-006: executing 'zoom pane' must behave exactly like
        // pressing `z`, because the palette replays the key.
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        s.ingest(rich_snapshot());
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();

        handle_key(key(KeyCode::Char(':')), &mut ui, &mut s, &mut h, &mut r);
        for c in "zoom".chars() {
            handle_key(key(KeyCode::Char(c)), &mut ui, &mut s, &mut h, &mut r);
        }
        handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
        assert!(ui.zoomed, "palette 'zoom pane' must zoom");
        assert!(ui.overlays.is_empty(), "palette closes after running");

        // 'quit' from the palette quits, exactly like q at the base.
        handle_key(key(KeyCode::Char(':')), &mut ui, &mut s, &mut h, &mut r);
        for c in "quit".chars() {
            handle_key(key(KeyCode::Char(c)), &mut ui, &mut s, &mut h, &mut r);
        }
        let outcome = handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(outcome, KeyOutcome::Quit);

        // A query matching nothing keeps the palette open on Enter.
        let mut ui2 = UiState::new();
        handle_key(key(KeyCode::Char(':')), &mut ui2, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Char('x')), &mut ui2, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Char('x')), &mut ui2, &mut s, &mut h, &mut r);
        handle_key(key(KeyCode::Enter), &mut ui2, &mut s, &mut h, &mut r);
        assert!(
            matches!(ui2.overlays.top(), Some(Overlay::Palette { .. })),
            "Enter with no match keeps the palette for query fixes"
        );
    }

    #[test]
    fn palette_renders_query_and_filtered_commands() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();
        let mut ui = UiState::new();
        ui.overlays.push(Overlay::Palette {
            query: "filter".into(),
            selected: 1,
        });

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    &ui,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    None,
                    100_000,
                )
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Commands"), "palette frame visible: {text}");
        assert!(text.contains("filter hints: token"), "matches listed");
        assert!(
            !text.contains("zoom pane"),
            "non-matching commands filtered out"
        );
    }

    #[test]
    fn question_mark_toggles_the_help_overlay() {
        // FR-3.1: `?` opens help; `?` again (with help topmost) closes
        // it. The rendered overlay carries the help title.
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        s.ingest(rich_snapshot());
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();

        handle_key(key(KeyCode::Char('?')), &mut ui, &mut s, &mut h, &mut r);
        assert!(
            matches!(ui.overlays.top(), Some(Overlay::Help)),
            "? must open the help overlay"
        );

        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|f| draw(f, &ui, &s, &hint_state, &research_state, None, 100_000))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Help"), "help overlay must render");
        assert!(
            text.contains("next pane"),
            "help must list the keymap table's actions"
        );

        handle_key(key(KeyCode::Char('?')), &mut ui, &mut s, &mut h, &mut r);
        assert!(ui.overlays.is_empty(), "? must close an open help overlay");
    }

    #[test]
    fn draw_renders_topmost_overlay_over_panes() {
        // FR-4.3: an open overlay is visible on the composed frame.
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();
        let mut ui = UiState::new();
        ui.overlays.push(Overlay::Detail {
            title: "OVERLAY-TITLE".into(),
            lines: vec!["overlay-body".into()],
        });
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    &ui,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    None,
                    100_000,
                )
            })
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("OVERLAY-TITLE"), "overlay must draw on top");
    }

    #[test]
    fn ctrl_c_quits_but_plain_c_does_not() {
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            handle_key(ctrl_c, &mut ui, &mut s, &mut h, &mut r),
            KeyOutcome::Quit
        );
        assert_eq!(
            handle_key(key(KeyCode::Char('c')), &mut ui, &mut s, &mut h, &mut r),
            KeyOutcome::Redraw
        );
    }

    #[test]
    fn tab_cycles_focus_and_backtab_reverses() {
        let mut ui = UiState::new();
        let mut s = ColdWindowState::new();
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        assert_eq!(ui.focus, FocusTarget::Alerts, "default focus is alerts");
        assert_eq!(
            handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r),
            KeyOutcome::Redraw
        );
        assert_eq!(ui.focus, FocusTarget::Hints);
        handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.focus, FocusTarget::Research);
        handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.focus, FocusTarget::Alerts, "Tab wraps around");
        handle_key(key(KeyCode::BackTab), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(ui.focus, FocusTarget::Research, "BackTab reverses");
    }

    #[test]
    fn focused_pane_marker_follows_focus() {
        // FR-1.3: the `>` title marker must sit on exactly the focused
        // pane, and move when focus moves: legible without color.
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(rich_snapshot());
        let hint_state = HintPaneState::new();
        let research_state = ResearchPaneState::default();

        let render_with = |focus: FocusTarget| -> String {
            let ui = UiState {
                focus,
                ..UiState::default()
            };
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            terminal
                .draw(|f| {
                    draw(
                        f,
                        &ui,
                        &snap_state,
                        &hint_state,
                        &research_state,
                        None,
                        100_000,
                    )
                })
                .unwrap();
            buffer_text(&terminal)
        };

        let alerts_focused = render_with(FocusTarget::Alerts);
        assert!(
            alerts_focused.contains("> Alerts"),
            "alerts focused: marker must prefix the alerts title"
        );
        assert!(
            !alerts_focused.contains("> Hints"),
            "alerts focused: hints must not carry the marker"
        );

        let hints_focused = render_with(FocusTarget::Hints);
        assert!(
            hints_focused.contains("> Hints"),
            "hints focused: marker must prefix the hints title"
        );
        assert!(
            !hints_focused.contains("> Alerts"),
            "hints focused: alerts must not carry the marker"
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
        let mut ui = UiState::new();
        let mut h = HintPaneState::new();
        let mut r = ResearchPaneState::default();
        let outcome = handle_key(key(KeyCode::Char('A')), &mut ui, &mut s, &mut h, &mut r);
        assert_eq!(outcome, KeyOutcome::Redraw);
        assert!(
            s.visible_alerts().is_empty(),
            "master-ack 'A' did not reach the alert pane"
        );
    }
}
