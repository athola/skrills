//! Cold-window dashboard surfaces: alert pane, hint pane, research
//! pane, and status bar.
//!
//! These widgets subscribe to the snapshot bus produced by
//! `skrills_analyze::cold_window::ColdWindowEngine` and render
//! ratatui frames. The TUI surface is one of two
//! [render targets](docs/cold-window-brief.md#3-architecture); the
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
//! - `hint_pane` and `research_pane` land in TASK-016 and TASK-017.

pub mod alert_pane;
pub mod state;
pub mod status_bar;

pub use alert_pane::{AlertAction, AlertPane};
pub use state::ColdWindowState;
pub use status_bar::StatusBar;
