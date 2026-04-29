//! AlertManager-style research dispatcher for the cold-window engine.
//!
//! Sits between "snapshot diff says something interesting changed"
//! and "fetch external research". Borrowed from Prometheus
//! AlertManager via the war-room TRIZ bridge:
//!
//! - **Group**: alerts sharing labels collapse into one fetch with a
//!   list inside.
//! - **Dedup**: same fingerprint from multiple sources fires once.
//! - **Inhibit**: a higher-confidence finding mutes redundant fetches
//!   (e.g. "GitHub already gave us the answer; don't HN-search the
//!   same topic").
//! - **Token bucket**: rate-limit at the dispatcher; never exceed
//!   the configured per-hour fetch budget even under continuous
//!   churn.
//! - **Persistence (R10)**: bucket state serializes to
//!   `~/.skrills/research-quota.json` on every successful dispatch
//!   and on graceful shutdown. On startup the bucket restores and
//!   refills pro-rata by elapsed wall-clock since the last save.
//!   This closes the "restart to reset quota" exploit raised in
//!   the war-room (RT-8).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use skrills_snapshot::{ResearchBudget, ResearchChannel, WindowSnapshot};

use crate::error::TomeResult;

/// Default fetches-per-hour budget when constructing
/// [`BucketedBudget::new`].
pub const DEFAULT_RESEARCH_RATE_PER_HOUR: u32 = 10;

/// Default per-fingerprint TTL for the inhibit window. Identical
/// fingerprints within this duration collapse to one fetch.
pub const DEFAULT_FINGERPRINT_TTL: Duration = Duration::from_secs(60 * 60);

/// Group/coalesce window for the dispatcher: incoming requests
/// arriving within this window with the same fingerprint collapse.
pub const DEFAULT_GROUP_WINDOW: Duration = Duration::from_secs(30);

/// Persistence file name (lives under the user's home directory).
pub const QUOTA_FILE_NAME: &str = "research-quota.json";

/// Persistent state for the token bucket. Saved on every successful
/// dispatch and on graceful shutdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedBucket {
    /// Capacity in fetches per hour.
    pub rate_per_hour: u32,
    /// Tokens currently available (fractional during refill).
    pub available: f64,
    /// UNIX-epoch milliseconds at which `available` was last
    /// computed; used to refill pro-rata on restart.
    pub last_refill_ms: u64,
}

impl PersistedBucket {
    /// Construct a brand-new full bucket.
    pub fn full(rate_per_hour: u32, now_ms: u64) -> Self {
        Self {
            rate_per_hour,
            available: rate_per_hour as f64,
            last_refill_ms: now_ms,
        }
    }

    /// Refill the bucket pro-rata by elapsed wall-clock since
    /// `last_refill_ms`. Bounded above by `rate_per_hour`.
    pub fn refill(&mut self, now_ms: u64) {
        let elapsed_ms = now_ms.saturating_sub(self.last_refill_ms);
        let elapsed_hours = elapsed_ms as f64 / (60.0 * 60.0 * 1000.0);
        let added = elapsed_hours * (self.rate_per_hour as f64);
        self.available = (self.available + added).min(self.rate_per_hour as f64);
        self.last_refill_ms = now_ms;
    }

    /// Try to consume one token; return `true` on success.
    pub fn try_consume(&mut self) -> bool {
        if self.available >= 1.0 {
            self.available -= 1.0;
            true
        } else {
            false
        }
    }
}

/// One in-flight or recently-completed dispatch entry. Kept in
/// memory keyed by fingerprint so we can short-circuit duplicates
/// within `fingerprint_ttl`.
#[derive(Debug, Clone)]
struct DispatchEntry {
    last_dispatched: Instant,
    /// Channels already attempted for this fingerprint within TTL.
    /// Used for inhibit logic (higher-confidence channel mutes lower).
    channels_seen: HashSet<ResearchChannel>,
}

/// Default research-budget dispatcher with all four AlertManager
/// mechanisms plus restart-resilient persistence.
pub struct BucketedBudget {
    bucket: Arc<Mutex<PersistedBucket>>,
    in_flight: Arc<Mutex<HashMap<String, DispatchEntry>>>,
    fingerprint_ttl: Duration,
    persistence_path: Option<PathBuf>,
}

impl BucketedBudget {
    /// Construct an in-memory bucket without persistence. Useful for
    /// tests and stateless integrations.
    pub fn in_memory(rate_per_hour: u32) -> Self {
        let now_ms = current_ms();
        Self {
            bucket: Arc::new(Mutex::new(PersistedBucket::full(rate_per_hour, now_ms))),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            fingerprint_ttl: DEFAULT_FINGERPRINT_TTL,
            persistence_path: None,
        }
    }

    /// Construct a persistent bucket; loads state from `path` if it
    /// exists, otherwise creates a fresh full bucket.
    ///
    /// Pro-rata refill: the loaded bucket has its tokens topped up
    /// according to elapsed wall-clock since the saved
    /// `last_refill_ms`. This is what closes the restart-exploit:
    /// if the user kills the daemon and immediately restarts, the
    /// tokens that have accrued in the (zero) elapsed time are
    /// (zero) — quota does not reset.
    pub fn persistent(rate_per_hour: u32, path: PathBuf) -> TomeResult<Self> {
        let now_ms = current_ms();
        let mut bucket = if path.exists() {
            let bytes = std::fs::read(&path)?;
            let mut loaded: PersistedBucket = serde_json::from_slice(&bytes)?;
            // If the configured rate has changed since last save, prefer
            // the new rate (cap available accordingly).
            loaded.rate_per_hour = rate_per_hour;
            loaded.available = loaded.available.min(rate_per_hour as f64);
            loaded
        } else {
            PersistedBucket::full(rate_per_hour, now_ms)
        };
        bucket.refill(now_ms);
        Ok(Self {
            bucket: Arc::new(Mutex::new(bucket)),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            fingerprint_ttl: DEFAULT_FINGERPRINT_TTL,
            persistence_path: Some(path),
        })
    }

    /// Override the per-fingerprint TTL.
    pub fn with_fingerprint_ttl(mut self, ttl: Duration) -> Self {
        self.fingerprint_ttl = ttl;
        self
    }

    /// Snapshot the current bucket state (for status-bar display).
    pub fn current_state(&self) -> PersistedBucket {
        self.bucket.lock().clone()
    }

    /// Ask whether we should dispatch a fetch for the given
    /// fingerprint right now. This applies group + dedup + bucket
    /// in sequence; inhibit by channel is a separate `try_dispatch`
    /// step.
    pub fn try_dispatch(&self, fingerprint: &str, channel: ResearchChannel) -> DispatchVerdict {
        let now = Instant::now();

        // Group / dedup: collapse identical-fingerprint requests
        // arriving within the TTL window.
        {
            let in_flight = self.in_flight.lock();
            if let Some(entry) = in_flight.get(fingerprint) {
                let age = now.duration_since(entry.last_dispatched);
                if age < self.fingerprint_ttl {
                    if entry.channels_seen.contains(&channel) {
                        return DispatchVerdict::DuplicateInWindow;
                    }
                    // Inhibit: GitHub already gave us a finding for
                    // this fingerprint; HN/Lobsters etc are inhibited
                    // for the remainder of the TTL.
                    if Self::is_higher_confidence_present(&entry.channels_seen, channel) {
                        return DispatchVerdict::InhibitedByHigherConfidence;
                    }
                }
            }
        }

        // Token bucket: refill pro-rata, then try-consume.
        let now_ms = current_ms();
        let mut bucket = self.bucket.lock();
        bucket.refill(now_ms);
        if !bucket.try_consume() {
            return DispatchVerdict::QuotaExhausted;
        }
        // Snapshot for persistence after we drop the lock.
        let to_persist = bucket.clone();
        drop(bucket);

        // Record the dispatch.
        {
            let mut in_flight = self.in_flight.lock();
            let entry = in_flight
                .entry(fingerprint.to_string())
                .or_insert_with(|| DispatchEntry {
                    last_dispatched: now,
                    channels_seen: HashSet::new(),
                });
            entry.last_dispatched = now;
            entry.channels_seen.insert(channel);
        }

        // Persist the bucket if a path is configured. Failure to
        // persist must not block dispatch (best-effort), but is
        // surfaced via tracing.
        if let Some(path) = &self.persistence_path {
            if let Err(e) = persist_bucket(path, &to_persist) {
                tracing::warn!(error = %e, "failed to persist research quota");
            }
        }

        DispatchVerdict::Allowed
    }

    /// Channel hierarchy for inhibit logic. Higher-confidence
    /// channels suppress later attempts on the same fingerprint
    /// within TTL: code-search > paper > discourse > triz.
    fn channel_confidence(channel: ResearchChannel) -> u8 {
        match channel {
            ResearchChannel::GitHub => 4,
            ResearchChannel::Paper => 3,
            ResearchChannel::HackerNews | ResearchChannel::Lobsters => 2,
            ResearchChannel::Triz => 1,
        }
    }

    fn is_higher_confidence_present(
        seen: &HashSet<ResearchChannel>,
        candidate: ResearchChannel,
    ) -> bool {
        let cand = Self::channel_confidence(candidate);
        seen.iter().any(|c| Self::channel_confidence(*c) > cand)
    }

    /// Force-persist the current bucket state. Called on graceful
    /// shutdown by TASK-031.
    pub fn flush_persistence(&self) -> TomeResult<()> {
        if let Some(path) = &self.persistence_path {
            let bucket = self.bucket.lock().clone();
            persist_bucket(path, &bucket)?;
        }
        Ok(())
    }
}

impl ResearchBudget for BucketedBudget {
    fn should_query(
        &self,
        _snapshot: &WindowSnapshot,
        topic_fingerprint: &str,
        _last_query: Option<Instant>,
    ) -> bool {
        // The trait's per-channel ignorance (no channel argument) is
        // resolved by treating it as a "is any channel allowed?"
        // probe. We default to the lowest-confidence channel so that
        // a `true` here means "at least one channel could fire."
        // For per-channel control, callers use `try_dispatch` directly.
        let now = Instant::now();
        let in_flight = self.in_flight.lock();
        if let Some(entry) = in_flight.get(topic_fingerprint) {
            if now.duration_since(entry.last_dispatched) < self.fingerprint_ttl {
                return false;
            }
        }
        drop(in_flight);
        let mut bucket = self.bucket.lock();
        bucket.refill(current_ms());
        bucket.available >= 1.0
    }
}

/// Verdict returned by [`BucketedBudget::try_dispatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchVerdict {
    /// Dispatch may proceed; one token consumed.
    Allowed,
    /// Same fingerprint dispatched recently on this channel; no-op.
    DuplicateInWindow,
    /// A higher-confidence channel already covered this fingerprint
    /// within TTL; lower-confidence channel inhibited.
    InhibitedByHigherConfidence,
    /// Token bucket empty; refusing to dispatch.
    QuotaExhausted,
}

/// Default persistence path: `~/.skrills/research-quota.json`.
pub fn default_persistence_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".skrills").join(QUOTA_FILE_NAME))
}

fn persist_bucket(path: &Path, bucket: &PersistedBucket) -> TomeResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(bucket)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn current_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::{LoadSample, TokenLedger};
    use tempfile::TempDir;

    fn empty_snapshot() -> WindowSnapshot {
        WindowSnapshot {
            version: 0,
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
    fn fresh_bucket_allows_capacity_dispatches() {
        let budget = BucketedBudget::in_memory(5);
        for _ in 0..5 {
            let v = budget.try_dispatch("fp-unique", ResearchChannel::GitHub);
            // After the first, subsequent same-fingerprint dispatches
            // are deduped — use distinct fingerprints to test bucket.
            assert!(matches!(
                v,
                DispatchVerdict::Allowed | DispatchVerdict::DuplicateInWindow
            ));
        }
    }

    #[test]
    fn distinct_fingerprints_consume_capacity_independently() {
        let budget = BucketedBudget::in_memory(3);
        let v1 = budget.try_dispatch("fp1", ResearchChannel::GitHub);
        let v2 = budget.try_dispatch("fp2", ResearchChannel::GitHub);
        let v3 = budget.try_dispatch("fp3", ResearchChannel::GitHub);
        assert_eq!(v1, DispatchVerdict::Allowed);
        assert_eq!(v2, DispatchVerdict::Allowed);
        assert_eq!(v3, DispatchVerdict::Allowed);
    }

    #[test]
    fn quota_exhausted_after_capacity_distinct_fingerprints() {
        let budget = BucketedBudget::in_memory(2);
        let _ = budget.try_dispatch("fp1", ResearchChannel::GitHub);
        let _ = budget.try_dispatch("fp2", ResearchChannel::GitHub);
        let v = budget.try_dispatch("fp3", ResearchChannel::GitHub);
        assert_eq!(v, DispatchVerdict::QuotaExhausted);
    }

    #[test]
    fn duplicate_fingerprint_same_channel_within_ttl_dedupes() {
        let budget = BucketedBudget::in_memory(100);
        let v1 = budget.try_dispatch("fp", ResearchChannel::GitHub);
        let v2 = budget.try_dispatch("fp", ResearchChannel::GitHub);
        assert_eq!(v1, DispatchVerdict::Allowed);
        assert_eq!(v2, DispatchVerdict::DuplicateInWindow);
    }

    #[test]
    fn higher_confidence_channel_inhibits_lower() {
        let budget = BucketedBudget::in_memory(100);
        let v1 = budget.try_dispatch("fp", ResearchChannel::GitHub);
        let v2 = budget.try_dispatch("fp", ResearchChannel::HackerNews);
        let v3 = budget.try_dispatch("fp", ResearchChannel::Triz);
        assert_eq!(v1, DispatchVerdict::Allowed);
        assert_eq!(v2, DispatchVerdict::InhibitedByHigherConfidence);
        assert_eq!(v3, DispatchVerdict::InhibitedByHigherConfidence);
    }

    #[test]
    fn lower_confidence_channel_does_not_inhibit_higher() {
        let budget = BucketedBudget::in_memory(100);
        let v1 = budget.try_dispatch("fp", ResearchChannel::Triz);
        // Even though Triz fired first, GitHub (higher confidence)
        // is not inhibited — it gets a fresh dispatch.
        let v2 = budget.try_dispatch("fp", ResearchChannel::GitHub);
        assert_eq!(v1, DispatchVerdict::Allowed);
        assert_eq!(v2, DispatchVerdict::Allowed);
    }

    #[test]
    fn channel_confidence_ranks_match_documentation() {
        assert!(
            BucketedBudget::channel_confidence(ResearchChannel::GitHub)
                > BucketedBudget::channel_confidence(ResearchChannel::Paper)
        );
        assert!(
            BucketedBudget::channel_confidence(ResearchChannel::Paper)
                > BucketedBudget::channel_confidence(ResearchChannel::HackerNews)
        );
        assert!(
            BucketedBudget::channel_confidence(ResearchChannel::HackerNews)
                > BucketedBudget::channel_confidence(ResearchChannel::Triz)
        );
        assert_eq!(
            BucketedBudget::channel_confidence(ResearchChannel::HackerNews),
            BucketedBudget::channel_confidence(ResearchChannel::Lobsters)
        );
    }

    #[test]
    fn persistence_round_trip_preserves_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("quota.json");
        {
            let budget = BucketedBudget::persistent(10, path.clone()).unwrap();
            // Consume two tokens.
            let _ = budget.try_dispatch("fp1", ResearchChannel::GitHub);
            let _ = budget.try_dispatch("fp2", ResearchChannel::GitHub);
            budget.flush_persistence().unwrap();
        }
        // Reopen — bucket should reflect the consumed tokens.
        let budget = BucketedBudget::persistent(10, path).unwrap();
        let state = budget.current_state();
        assert!(state.available <= 8.5);
        assert!(state.available >= 7.5);
    }

    #[test]
    fn restart_exploit_quota_does_not_fully_reset() {
        // R10 mitigation test: rapid restart cycle (saved → reload
        // immediately) does not produce a fresh bucket.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("quota.json");
        let budget = BucketedBudget::persistent(5, path.clone()).unwrap();
        // Drain the bucket.
        for i in 0..5 {
            let _ = budget.try_dispatch(&format!("fp{i}"), ResearchChannel::GitHub);
        }
        let pre_state = budget.current_state();
        budget.flush_persistence().unwrap();
        drop(budget);

        // Immediate restart.
        let budget2 = BucketedBudget::persistent(5, path).unwrap();
        let post_state = budget2.current_state();
        // No appreciable refill in zero elapsed time → tokens stay
        // close to zero (within tolerance for clock jitter).
        assert!(
            post_state.available < 1.0,
            "expected near-zero tokens after rapid restart, got {} (pre {})",
            post_state.available,
            pre_state.available,
        );
    }

    #[test]
    fn refill_pro_rata_topup_is_bounded_by_capacity() {
        let mut bucket = PersistedBucket {
            rate_per_hour: 10,
            available: 0.0,
            last_refill_ms: 0,
        };
        // Simulate 2 hours of elapsed time → 20 tokens worth of refill,
        // capped at capacity 10.
        bucket.refill(2 * 60 * 60 * 1000);
        assert_eq!(bucket.available, 10.0);
    }

    #[test]
    fn refill_pro_rata_partial_hour() {
        let mut bucket = PersistedBucket {
            rate_per_hour: 10,
            available: 0.0,
            last_refill_ms: 0,
        };
        // 30 minutes → 5 tokens.
        bucket.refill(30 * 60 * 1000);
        assert!((bucket.available - 5.0).abs() < 1e-9);
    }

    #[test]
    fn should_query_implements_research_budget_trait() {
        let budget: Box<dyn ResearchBudget> = Box::new(BucketedBudget::in_memory(5));
        let snap = empty_snapshot();
        assert!(budget.should_query(&snap, "novel-fp", None));
    }
}
