//! Wire-format types for the cold-window real-time analysis subsystem.
//!
//! This crate carries the contract between the cold-window producer
//! (`skrills_analyze::cold_window`) and its consumers (the TUI in
//! `skrills_dashboard` and the browser-facing SSE handler in
//! `skrills_server`). Producer and consumers depend on this crate;
//! they do not depend on each other.
//!
//! Type definitions land in TASK-003. This file is the scaffolding
//! placeholder produced by TASK-001 of the cold-window plan.
//!
//! See `docs/cold-window-brief.md` for design rationale and
//! `docs/cold-window-spec.md` for type contracts.

#![deny(unsafe_code)]
#![warn(missing_docs)]
