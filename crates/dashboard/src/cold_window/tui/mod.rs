//! Crossterm raw-mode launcher for the cold-window TUI.
//!
//! The four cold-window panes ([`AlertPane`], [`HintPane`],
//! [`ResearchPane`], [`StatusBar`]) are pure render functions tested
//! with `TestBackend`. This module tree composes them into a single
//! frame ([`draw`]), routes keystrokes ([`handle_key`]), and drives a
//! live crossterm event loop ([`run`]) that subscribes to the engine's
//! snapshot bus.
//!
//! The server's `cold-window --tui` path calls [`run`] with the
//! broadcast receiver from `ColdWindowEngine::subscribe`; everything
//! crossterm-specific stays here so the server crate needs no terminal
//! dependency of its own.
//!
//! Submodule layout:
//!
//! - [`layout`] picks the responsive tier and places the panes.
//! - [`render`] composes the panes (and overlays) into one frame.
//! - [`input`] routes keystrokes to overlays, globals, and panes.
//! - [`runner`] owns the terminal and the live event loop.
//!
//! This root holds only the interface-level shell state ([`UiState`])
//! shared across those concerns, and re-exports the public surface.
//!
//! [`AlertPane`]: crate::cold_window::AlertPane
//! [`HintPane`]: crate::cold_window::HintPane
//! [`ResearchPane`]: crate::cold_window::ResearchPane
//! [`StatusBar`]: crate::cold_window::StatusBar

mod input;
mod layout;
mod render;
mod runner;
#[cfg(test)]
mod tests;

pub use input::{handle_key, KeyOutcome};
pub use layout::{
    below_size_floor, layout_mode, plan_layout, plan_layout_with, ColdWindowLayout, LayoutMode,
};
pub use render::draw;
pub use runner::{run, QuotaFn, TuiOptions};

use crate::cold_window::focus::FocusTarget;
use crate::cold_window::overlay::OverlayStack;

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
