//! Composite frame rendering for the cold-window TUI.
//!
//! [`draw`] places the four panes via [`super::layout`] and paints them,
//! then draws any modal overlay on top. Pure render: no terminal setup,
//! no event handling.

use ratatui::Frame;
use skrills_snapshot::ResearchQuota;

use super::layout::{
    below_size_floor, layout_mode, plan_layout_with, LayoutMode, GUARD_MIN_HEIGHT, GUARD_MIN_WIDTH,
};
use super::UiState;
use crate::cold_window::{
    overlay, AlertPane, ColdWindowState, FocusTarget, HintPane, HintPaneState, ResearchPane,
    ResearchPaneState, StatusBar,
};

/// Compose the four panes into a single frame, adapting to the
/// terminal size.
///
/// The arrangement is chosen by [`plan_layout`](super::plan_layout) from the frame's width:
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
