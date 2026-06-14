//! Responsive layout planning for the cold-window TUI.
//!
//! Pure, `Rect`-only tier selection and pane placement. Split out of the
//! shell so the topology invariants (status pinned to the bottom row; the
//! four rects tile the frame) can be unit-tested without a terminal.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::cold_window::FocusTarget;

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

/// Hard floor below which panes are unrenderable; [`draw`](super::draw) shows a
/// one-line guard message instead of clipped pane fragments (FR-6.3).
pub(crate) const GUARD_MIN_WIDTH: u16 = 20;
/// See [`GUARD_MIN_WIDTH`].
pub(crate) const GUARD_MIN_HEIGHT: u16 = 6;

/// True when `area` is below the renderable floor.
pub fn below_size_floor(area: Rect) -> bool {
    area.width < GUARD_MIN_WIDTH || area.height < GUARD_MIN_HEIGHT
}

/// Where each pane is drawn for a given frame. Produced by
/// [`plan_layout`] and consumed by [`draw`](super::draw); pure and `Rect`-only so it
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
/// `NARROW_MIN_WIDTH` columns the focused pane is the layout,
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
pub(crate) fn plan_for(mode: LayoutMode, area: Rect, research_collapsed: bool) -> ColdWindowLayout {
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
