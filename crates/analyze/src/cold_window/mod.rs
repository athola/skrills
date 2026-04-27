//! Cold-window real-time analysis subsystem.
//!
//! Per `docs/cold-window-brief.md`, this module hosts the per-tick
//! producer (`ColdWindowEngine`) and the strategy traits that govern
//! its behavior (`AlertPolicy`, `HintScorer`, `ResearchBudget`,
//! `SnapshotDiff`, `CadenceStrategy`). The producer broadcasts
//! `Arc<WindowSnapshot>` over a bounded `tokio::sync::broadcast`
//! channel; consumers (TUI in `skrills-dashboard`, browser SSE
//! handler in `skrills-server`) subscribe to the same bus.
//!
//! Resource bounds (per cold-window plan TASK-007 / risk R11):
//!
//! - Broadcast channel capacity: [`SNAPSHOT_CHANNEL_CAPACITY`].
//!   Lagging subscribers drop and the engine logs a `Status`-tier
//!   alert; the producer never blocks.
//! - Activity ring buffer: [`ACTIVITY_RING_CAPACITY`]. Oldest entries
//!   evict; the ring never grows unboundedly.
//!
//! Type-shaped contracts live in [`traits`]; the default cadence
//! lives in [`cadence`]. Default trait implementations
//! (`LayeredAlertPolicy`, `MultiSignalScorer`, `BucketedBudget`,
//! `FieldwiseDiff`) land in TASK-010, TASK-011, TASK-013, TASK-014
//! of the cold-window plan.

pub mod cadence;
pub mod diff;
pub mod traits;

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use skrills_snapshot::WindowSnapshot;
use tokio::sync::broadcast;

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

/// Cold-window engine skeleton.
///
/// At this stage (TASK-007 of the cold-window plan), the engine
/// exposes the `SnapshotBus` and per-tick interface but does not yet
/// integrate discovery, token attribution, alert policy, or hint
/// scoring. Those land in TASK-008 (engine GREEN integration).
///
/// `tick` accepts a fully-built `WindowSnapshot` and broadcasts it.
/// Once TASK-008 lands, `tick` will assemble the snapshot internally
/// from discovery + analyze + intelligence + tome.
pub struct ColdWindowEngine {
    tx: broadcast::Sender<Arc<WindowSnapshot>>,
    activity: Arc<Mutex<ActivityRing>>,
}

impl ColdWindowEngine {
    /// Create a new engine with default bounded resources.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(SNAPSHOT_CHANNEL_CAPACITY);
        Self {
            tx,
            activity: Arc::new(Mutex::new(ActivityRing::default())),
        }
    }

    /// Subscribe to the snapshot bus. The receiver delivers
    /// `Arc<WindowSnapshot>` per tick. If a subscriber lags by more
    /// than [`SNAPSHOT_CHANNEL_CAPACITY`] snapshots, the channel
    /// returns `RecvError::Lagged` and the consumer must catch up.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<WindowSnapshot>> {
        self.tx.subscribe()
    }

    /// How many subscribers are currently attached.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Broadcast a pre-built snapshot. Returns the same `Arc` for
    /// convenience. If no subscribers are attached, the snapshot is
    /// dropped silently — broadcasting requires at least one receiver
    /// to be observed.
    pub fn publish(&self, snapshot: WindowSnapshot) -> Arc<WindowSnapshot> {
        let snap = Arc::new(snapshot);
        let _ = self.tx.send(Arc::clone(&snap));
        snap
    }

    /// Append an activity entry to the bounded ring.
    pub fn record_activity(&self, entry: impl Into<String>) {
        self.activity.lock().push(entry.into());
    }

    /// Snapshot the activity ring (oldest first).
    pub fn activity_snapshot(&self) -> Vec<String> {
        self.activity.lock().snapshot()
    }
}

impl Default for ColdWindowEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::{LoadSample, TokenLedger};

    fn empty_snapshot(version: u64) -> WindowSnapshot {
        WindowSnapshot {
            version,
            timestamp_ms: 0,
            token_ledger: TokenLedger::default(),
            alerts: vec![],
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        }
    }

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

    #[tokio::test]
    async fn engine_publishes_to_subscriber() {
        let engine = ColdWindowEngine::new();
        let mut rx = engine.subscribe();
        let published = engine.publish(empty_snapshot(1));
        let received = rx.recv().await.expect("recv");
        assert_eq!(received.version, 1);
        assert!(Arc::ptr_eq(&published, &received));
    }

    #[tokio::test]
    async fn engine_handles_no_subscribers_gracefully() {
        let engine = ColdWindowEngine::new();
        // No subscribers attached; publish must not panic or block.
        let _ = engine.publish(empty_snapshot(1));
        assert_eq!(engine.subscriber_count(), 0);
    }

    #[tokio::test]
    async fn engine_marks_lagging_subscribers() {
        let engine = ColdWindowEngine::new();
        let mut rx = engine.subscribe();
        // Publish more than the channel capacity without consuming.
        for v in 0..(SNAPSHOT_CHANNEL_CAPACITY as u64 + 5) {
            engine.publish(empty_snapshot(v));
        }
        // First recv should report lag, not deliver an old snapshot.
        let result = rx.recv().await;
        assert!(matches!(
            result,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
        ));
    }

    #[tokio::test]
    async fn thousand_ticks_do_not_grow_resident_state() {
        // Memory smoke test (R11): broadcast bound + activity ring bound
        // must hold under sustained tick load without unbounded growth.
        let engine = ColdWindowEngine::new();
        let _rx = engine.subscribe();
        for v in 0..1000u64 {
            engine.publish(empty_snapshot(v));
            engine.record_activity(format!("tick-{v}"));
        }
        // Activity ring is capped; no more than ACTIVITY_RING_CAPACITY entries.
        assert!(engine.activity_snapshot().len() <= ACTIVITY_RING_CAPACITY);
    }

    #[test]
    fn activity_ring_constants_match_plan() {
        // Per cold-window plan TASK-007 (R11):
        assert_eq!(SNAPSHOT_CHANNEL_CAPACITY, 16);
        assert_eq!(ACTIVITY_RING_CAPACITY, 100);
    }
}
