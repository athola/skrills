//! Runtime configuration path helpers for `skrills-server`.
//!
//! Re-exports the runtime overrides path from the state crate.
//! Note: Runtime override features were removed in 0.3.1 as skill loading
//! is now handled by Claude/Codex directly.

pub use skrills_state::runtime_overrides_path;
