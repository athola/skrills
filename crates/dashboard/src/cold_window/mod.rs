//! Cold-window dashboard surfaces: alert pane, hint pane, research
//! pane, and status bar.
//!
//! These widgets subscribe to the snapshot bus produced by
//! `skrills_analyze::cold_window::ColdWindowEngine` and render
//! ratatui frames. The TUI surface is one of two
//! [render targets](docs/archive/2026-04-26-cold-window-brief.md#3-architecture); the
//! browser surface in `skrills-server` consumes the same
//! `WindowSnapshot` artifact.
//!
//! Module layout:
//!
//! - [`state`] holds per-tick view state (current snapshot, ack
//!   bookkeeping, bell suppression).
//! - [`alert_pane`] renders the 4-tier alert list and handles the
//!   master-acknowledge keystroke (TASK-015).
//! - [`status_bar`] renders the bottom status line: tick cadence,
//!   token budget, alert counts per tier, research-quota state
//!   (TASK-018).
//! - [`hint_pane`] renders the ranked `ScoredHint` list with
//!   category filter and persisted pin toggles (TASK-016).
//! - [`research_pane`] renders the pull-only research findings
//!   panel, collapsed by default with a badge counter (TASK-017).

pub mod alert_pane;
pub mod hint_pane;
pub mod research_pane;
pub mod state;
pub mod status_bar;

pub use alert_pane::{AlertAction, AlertPane};
pub use hint_pane::{HintAction, HintPane, HintPaneState};
pub use research_pane::{ResearchAction, ResearchPane, ResearchPaneState};
pub use state::ColdWindowState;
pub use status_bar::StatusBar;
