//! Operational kill-switch for the cold-window token-budget ceiling.
//!
//! The cold-window engine engages this switch when cumulative token usage
//! crosses the configured budget ceiling (FR12). Sync adapters consult the
//! same switch before performing any mutating I/O and refuse with a typed
//! error when it is engaged.
//!
//! `KillSwitch` lives in the snapshot crate because it is the only crate
//! already shared between the producer (`skrills_analyze::cold_window`) and
//! the consumer (`skrills_sync`); placing the type here avoids a circular
//! dependency through the engine.
//!
//! The type is intentionally minimal: a single `Arc<AtomicBool>` with
//! `engage`/`release`/`is_engaged`. No async, no callbacks, no logging.
//! Higher layers are responsible for emitting the WARNING-tier alert that
//! accompanies engagement (see `analyze::cold_window::alert`).
//!
//! # Example
//!
//! ```
//! use skrills_snapshot::KillSwitch;
//!
//! let switch = KillSwitch::new();
//! assert!(!switch.is_engaged());
//!
//! switch.engage();
//! assert!(switch.is_engaged());
//!
//! // Cloning is cheap (Arc bump) and shares the same underlying state.
//! let clone = switch.clone();
//! switch.release();
//! assert!(!clone.is_engaged());
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A shared, thread-safe kill-switch flag.
///
/// Cloning is cheap (`Arc` bump) — every clone observes and mutates the
/// same underlying flag. Engagement is `SeqCst` to keep the producer/
/// consumer ordering trivially auditable; reads use `Acquire` so a
/// consumer that observes `true` is guaranteed to see all prior writes
/// from the engager.
#[derive(Clone, Debug, Default)]
pub struct KillSwitch(Arc<AtomicBool>);

impl KillSwitch {
    /// Construct a new, disengaged kill-switch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Engage the switch. Subsequent `is_engaged()` calls return `true`.
    pub fn engage(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Release the switch. Subsequent `is_engaged()` calls return `false`.
    ///
    /// Used by tests and by the engine when token usage drops back below
    /// ceiling on a later tick (with hysteresis owned by the alert policy).
    pub fn release(&self) {
        self.0.store(false, Ordering::SeqCst);
    }

    /// Read the current engagement state.
    pub fn is_engaged(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_switch_is_disengaged() {
        let s = KillSwitch::new();
        assert!(!s.is_engaged());
    }

    #[test]
    fn engage_then_release_round_trips() {
        let s = KillSwitch::new();
        s.engage();
        assert!(s.is_engaged());
        s.release();
        assert!(!s.is_engaged());
    }

    #[test]
    fn clones_share_state() {
        let a = KillSwitch::new();
        let b = a.clone();
        a.engage();
        assert!(b.is_engaged());
        b.release();
        assert!(!a.is_engaged());
    }

    #[test]
    fn default_is_disengaged() {
        let s: KillSwitch = Default::default();
        assert!(!s.is_engaged());
    }
}
