//! Cold-window real-time analysis subsystem.
//!
//! Per `docs/archive/2026-04-26-cold-window-brief.md`, this module hosts the per-tick
//! producer (`ColdWindowEngine`) and the strategy traits that govern
//! its behavior (`AlertPolicy`, `HintScorer`, `ResearchBudget`,
//! `SnapshotDiff`, `CadenceStrategy`). The producer broadcasts
//! `Arc<WindowSnapshot>` over a bounded `tokio::sync::broadcast`
//! channel; consumers (TUI in `skrills-dashboard`, browser SSE
//! handler in `skrills-server`) subscribe to the same bus.
//!
//! Resource bounds (R11):
//!
//! - Broadcast channel capacity: [`SNAPSHOT_CHANNEL_CAPACITY`].
//!   Lagging subscribers drop and the engine logs a `Status`-tier
//!   alert; the producer never blocks.
//! - Activity ring buffer: [`ACTIVITY_RING_CAPACITY`]. Oldest entries
//!   evict; the ring never grows unboundedly.
//!
//! Type-shaped contracts live in [`traits`]; the default cadence
//! lives in [`cadence`]. Default trait implementations are
//! `LayeredAlertPolicy` (in [`alert`]), `MultiSignalScorer` (in
//! `skrills-intelligence`), `BucketedBudget` (in `skrills-tome`),
//! and `FieldwiseDiff` (in [`diff`]).

pub mod alert;
pub mod cadence;
pub mod diff;
pub mod engine;
pub mod plugin_health;
pub mod traits;

pub use engine::{ColdWindowEngine, DefaultHintScorer, TickInput};
pub use plugin_health::{CollectorOutput, MalformedPlugin, PluginHealthCollector};

use std::collections::VecDeque;

/// Broadcast capacity: lagging subscribers drop after this many
/// queued snapshots so the producer never blocks (R11 bound).
pub const SNAPSHOT_CHANNEL_CAPACITY: usize = 16;

/// Activity ring capacity: oldest entries evict on overflow (R11 bound).
pub const ACTIVITY_RING_CAPACITY: usize = 100;

/// Bounded ring buffer for human-readable activity entries.
///
/// Used by the TUI's activity feed pane. Capped at
/// [`ACTIVITY_RING_CAPACITY`]; oldest entries evict on overflow.
#[derive(Debug)]
pub struct ActivityRing {
    entries: VecDeque<String>,
    capacity: usize,
}

impl ActivityRing {
    /// Create a new ring with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Append an entry; evict the oldest if at capacity.
    pub fn push(&mut self, entry: String) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Number of entries currently retained.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entries are retained.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Snapshot the ring contents as a Vec (oldest first).
    pub fn snapshot(&self) -> Vec<String> {
        self.entries.iter().cloned().collect()
    }
}

impl Default for ActivityRing {
    fn default() -> Self {
        Self::with_capacity(ACTIVITY_RING_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_ring_caps_at_capacity() {
        let mut ring = ActivityRing::with_capacity(3);
        for i in 0..10 {
            ring.push(format!("entry-{i}"));
        }
        assert_eq!(ring.len(), 3);
        let snap = ring.snapshot();
        assert_eq!(snap, vec!["entry-7", "entry-8", "entry-9"]);
    }

    #[test]
    fn activity_ring_default_is_empty() {
        let ring = ActivityRing::default();
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn activity_ring_constants_match_plan() {
        // R11 resource-bounds invariants:
        assert_eq!(SNAPSHOT_CHANNEL_CAPACITY, 16);
        assert_eq!(ACTIVITY_RING_CAPACITY, 100);
    }
}
