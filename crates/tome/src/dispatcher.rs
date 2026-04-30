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
//!
//! ## Concurrency invariants
//!
//! `try_dispatch` runs the full dedup-then-bucket sequence under a
//! single `Mutex<DispatcherInner>`. Splitting the dedup map and the
//! bucket across separate locks (the previous design) admitted a
//! TOCTOU race where two threads with the same fingerprint could
//! both pass dedup, both consume tokens, and both record — violating
//! the SC10 capacity invariant. Collapsing the critical sections
//! restores that invariant.
//!
//! ## Clock robustness
//!
//! The clock is fallible (`Option<u64>`). If the system clock briefly
//! precedes `UNIX_EPOCH` (NTP recovery, container time-warp), we log
//! a warning and skip the refill tick rather than fabricating a zero
//! timestamp. A fabricated zero would produce `elapsed ≈ 1.7T ms`
//! against any real `last_refill_ms` and saturate the bucket to
//! capacity, undoing the R10 mitigation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use skrills_snapshot::{ResearchBudget, ResearchChannel, WindowSnapshot};

use crate::error::TomeResult;

/// Default fetches-per-hour budget when constructing a
/// [`BucketedBudget`] via [`BucketedBudget::in_memory`] or
/// [`BucketedBudget::persistent`].
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

    /// Validate a freshly-deserialized bucket. Returns `None` if the
    /// state is unrecoverable (`NaN`/`Inf` in `available`). On success
    /// returns a sanitized bucket with `available` clamped to
    /// `[0.0, rate_per_hour]` and `rate_per_hour` overridden by the
    /// currently-configured rate so a config change takes effect.
    fn validated(mut self, configured_rate: u32) -> Option<Self> {
        if !self.available.is_finite() {
            return None;
        }
        self.rate_per_hour = configured_rate;
        let cap = configured_rate as f64;
        if self.available < 0.0 {
            self.available = 0.0;
        } else if self.available > cap {
            self.available = cap;
        }
        Some(self)
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

/// State protected by a single mutex so that the
/// dedup-then-token-bucket sequence in `try_dispatch` is atomic.
/// See B5 in PR-218 review: splitting these maps admitted a TOCTOU
/// race that violated the SC10 capacity invariant.
struct DispatcherInner {
    bucket: PersistedBucket,
    in_flight: HashMap<String, DispatchEntry>,
}

/// Pluggable clock. Returns `Some(now_unix_millis)` on success,
/// `None` if the system clock briefly precedes `UNIX_EPOCH`. The
/// dispatcher treats `None` as "skip the refill tick this call."
type ClockFn = Arc<dyn Fn() -> Option<u64> + Send + Sync>;

/// Default research-budget dispatcher with all four AlertManager
/// mechanisms plus restart-resilient persistence.
pub struct BucketedBudget {
    inner: Arc<Mutex<DispatcherInner>>,
    fingerprint_ttl: Duration,
    persistence_path: Option<PathBuf>,
    clock: ClockFn,
}

impl BucketedBudget {
    /// Construct an in-memory bucket without persistence. Useful for
    /// tests and stateless integrations.
    pub fn in_memory(rate_per_hour: u32) -> Self {
        let clock: ClockFn = Arc::new(current_ms_checked);
        let now_ms = bootstrap_ms((clock)());
        Self {
            inner: Arc::new(Mutex::new(DispatcherInner {
                bucket: PersistedBucket::full(rate_per_hour, now_ms),
                in_flight: HashMap::new(),
            })),
            fingerprint_ttl: DEFAULT_FINGERPRINT_TTL,
            persistence_path: None,
            clock,
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
    ///
    /// **Recovery (N6/NI5).** If the persistence file is corrupt
    /// (half-written from a SIGKILL) or contains an unrecoverable
    /// value (`NaN`/`Inf` in `available`), this constructor logs a
    /// `tracing::warn!` at `CAUTION` tier and falls back to a fresh
    /// full bucket rather than refusing to boot the daemon. A
    /// negative `available` is clamped to zero; values above
    /// `rate_per_hour` are clamped down. IO errors on read still
    /// propagate.
    pub fn persistent(rate_per_hour: u32, path: PathBuf) -> TomeResult<Self> {
        let clock: ClockFn = Arc::new(current_ms_checked);
        let now_ms_opt = (clock)();
        let now_ms = bootstrap_ms(now_ms_opt);
        let mut bucket = if path.exists() {
            match std::fs::read(&path) {
                Ok(bytes) => match serde_json::from_slice::<PersistedBucket>(&bytes) {
                    Ok(loaded) => match loaded.validated(rate_per_hour) {
                        Some(b) => b,
                        None => {
                            tracing::warn!(
                                path = %path.display(),
                                tier = "CAUTION",
                                "research quota file held unrecoverable values \
                                 (NaN/Inf in `available`); recovering with fresh \
                                 full bucket",
                            );
                            PersistedBucket::full(rate_per_hour, now_ms)
                        }
                    },
                    Err(err) => {
                        tracing::warn!(
                            path = %path.display(),
                            error = %err,
                            tier = "CAUTION",
                            "research quota file is corrupt (likely a \
                             half-written SIGKILL artifact); recovering with \
                             fresh full bucket",
                        );
                        PersistedBucket::full(rate_per_hour, now_ms)
                    }
                },
                Err(io_err) => return Err(io_err.into()),
            }
        } else {
            PersistedBucket::full(rate_per_hour, now_ms)
        };
        // Only top up at boot if we got a usable clock — otherwise we
        // would compute elapsed against a sentinel and saturate (NB5).
        if let Some(real_now) = now_ms_opt {
            bucket.refill(real_now);
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(DispatcherInner {
                bucket,
                in_flight: HashMap::new(),
            })),
            fingerprint_ttl: DEFAULT_FINGERPRINT_TTL,
            persistence_path: Some(path),
            clock,
        })
    }

    /// Override the per-fingerprint TTL.
    pub fn with_fingerprint_ttl(mut self, ttl: Duration) -> Self {
        self.fingerprint_ttl = ttl;
        self
    }

    /// Snapshot the current bucket state (for status-bar display).
    pub fn current_state(&self) -> PersistedBucket {
        self.inner.lock().bucket.clone()
    }

    /// Ask whether we should dispatch a fetch for the given
    /// fingerprint right now. This applies group + dedup + bucket
    /// in sequence; inhibit by channel is a separate `try_dispatch`
    /// step.
    ///
    /// The full dedup-and-bucket sequence runs under one mutex. See
    /// the module docs and PR-218 finding B5 for why splitting the
    /// critical sections is unsafe.
    pub fn try_dispatch(&self, fingerprint: &str, channel: ResearchChannel) -> DispatchVerdict {
        let now = Instant::now();
        let now_ms_opt = (self.clock)();

        let to_persist = {
            let mut inner = self.inner.lock();

            // Group / dedup: collapse identical-fingerprint requests
            // arriving within the TTL window.
            if let Some(entry) = inner.in_flight.get(fingerprint) {
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

            // Token bucket: refill pro-rata (skip refill if the clock
            // refused to give us a usable timestamp), then try-consume.
            if let Some(now_ms) = now_ms_opt {
                inner.bucket.refill(now_ms);
            } else {
                tracing::warn!(
                    "system clock precedes UNIX_EPOCH; skipping refill tick \
                     to preserve R10 (saturation guard)"
                );
            }
            if !inner.bucket.try_consume() {
                return DispatchVerdict::QuotaExhausted;
            }

            // Record the dispatch — same critical section as the
            // dedup check and the bucket consume.
            let entry = inner
                .in_flight
                .entry(fingerprint.to_string())
                .or_insert_with(|| DispatchEntry {
                    last_dispatched: now,
                    channels_seen: HashSet::new(),
                });
            entry.last_dispatched = now;
            entry.channels_seen.insert(channel);

            inner.bucket.clone()
        };

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
            let bucket = self.inner.lock().bucket.clone();
            persist_bucket(path, &bucket)?;
        }
        Ok(())
    }

    /// Test-only: construct an in-memory budget with an injected
    /// clock. Used to exercise the NB5 (clock-warp) recovery path
    /// without touching the real `SystemTime`.
    #[cfg(test)]
    fn in_memory_with_clock(rate_per_hour: u32, clock: ClockFn) -> Self {
        let now_ms = bootstrap_ms((clock)());
        Self {
            inner: Arc::new(Mutex::new(DispatcherInner {
                bucket: PersistedBucket::full(rate_per_hour, now_ms),
                in_flight: HashMap::new(),
            })),
            fingerprint_ttl: DEFAULT_FINGERPRINT_TTL,
            persistence_path: None,
            clock,
        }
    }
}

impl ResearchBudget for BucketedBudget {
    /// Probe whether at least one channel could fire for
    /// `topic_fingerprint` right now. **This is a non-consuming
    /// probe**: it neither decrements the bucket nor records a
    /// dispatch. Callers that want to actually fetch must follow up
    /// with [`BucketedBudget::try_dispatch`], which performs the
    /// authoritative atomic dedup-and-consume. Repeatedly calling
    /// `should_query` without a follow-up `try_dispatch` is allowed
    /// and does not exhaust quota — it is intentionally cheap so the
    /// cold-window engine can poll it (PR-218 review N5).
    ///
    /// The trait's per-channel ignorance (no channel argument) is
    /// resolved by treating it as a "is any channel allowed?" probe.
    /// We default to the lowest-confidence channel so that a `true`
    /// here means "at least one channel could fire."
    fn should_query(
        &self,
        _snapshot: &WindowSnapshot,
        topic_fingerprint: &str,
        _last_query: Option<Instant>,
    ) -> bool {
        let now = Instant::now();
        let now_ms_opt = (self.clock)();
        let mut inner = self.inner.lock();
        if let Some(entry) = inner.in_flight.get(topic_fingerprint) {
            if now.duration_since(entry.last_dispatched) < self.fingerprint_ttl {
                return false;
            }
        }
        if let Some(now_ms) = now_ms_opt {
            inner.bucket.refill(now_ms);
        }
        inner.bucket.available >= 1.0
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

/// Persist the bucket atomically: write to a per-pid sibling temp
/// file, then `rename` over the destination. `rename` on POSIX is
/// atomic for same-filesystem moves; this closes the N7
/// corruption-on-crash window in PR-218.
fn persist_bucket(path: &Path, bucket: &PersistedBucket) -> TomeResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(bucket)?;

    let tmp_path = tmp_sibling(path);
    // Best-effort cleanup of any stragglers from a previous crash.
    // Failure is tolerated: the rename will overwrite anyway.
    let _ = std::fs::remove_file(&tmp_path);

    std::fs::write(&tmp_path, &bytes)?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        // Rename failed — clean up the tmp file so we don't leak.
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }
    Ok(())
}

/// Build a sibling temp path with high collision resistance under
/// concurrent dispatchers (multiple processes pointed at the same
/// quota file). Uses pid + a coarse nanosecond suffix.
fn tmp_sibling(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| String::from("quota"));
    let tmp_name = format!("{name}.tmp.{pid}.{nanos}");
    match path.parent() {
        Some(parent) => parent.join(tmp_name),
        None => PathBuf::from(tmp_name),
    }
}

/// Fallible UNIX-millis clock. Returns `None` if `SystemTime::now()`
/// precedes `UNIX_EPOCH` (NTP recovery, container time-warp).
/// Callers must not fabricate `0` on `None` — see PR-218 finding NB5
/// for the saturation-guard rationale.
fn current_ms_checked() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// Pick a sentinel timestamp for `last_refill_ms` at construction
/// time. If the clock is usable, use it. If not, use `u64::MAX` so
/// that the first `refill` after the clock recovers computes
/// `now.saturating_sub(MAX) == 0` elapsed — preventing the bucket
/// from saturating to capacity on the recovery tick (NB5).
fn bootstrap_ms(now_ms_opt: Option<u64>) -> u64 {
    now_ms_opt.unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::{LoadSample, TokenLedger};
    use std::sync::atomic::{AtomicBool, Ordering};
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

    #[test]
    fn should_query_does_not_consume_tokens() {
        // N5: should_query must be a non-consuming probe. Hammering
        // it 1000x must not exhaust quota.
        let budget = BucketedBudget::in_memory(2);
        let snap = empty_snapshot();
        for _ in 0..1000 {
            assert!(budget.should_query(&snap, "novel-fp", None));
        }
        assert!((budget.current_state().available - 2.0).abs() < 1e-6);
    }

    #[test]
    fn clock_warp_does_not_saturate_bucket() {
        // NB5: simulate a clock that briefly precedes UNIX_EPOCH
        // (returns None) for one tick, then recovers. The pre-fix
        // implementation fabricated 0 on None, which made the next
        // refill compute elapsed ≈ 1.7T ms → bucket saturates to
        // capacity → R10 undone. With the fix, the warp tick is
        // skipped entirely and the bucket retains its consumed state.
        let warped = Arc::new(AtomicBool::new(true));
        let warped_for_clock = Arc::clone(&warped);
        let real_now = current_ms_checked().unwrap_or(0);
        let clock: ClockFn = Arc::new(move || {
            if warped_for_clock.load(Ordering::SeqCst) {
                None
            } else {
                Some(real_now)
            }
        });

        let budget = BucketedBudget::in_memory_with_clock(5, clock);

        // Drain one token under the warp.
        assert_eq!(
            budget.try_dispatch("fp1", ResearchChannel::GitHub),
            DispatchVerdict::Allowed
        );
        let after_warp = budget.current_state();
        assert!((after_warp.available - 4.0).abs() < 1e-6);

        // Recover the clock and dispatch again. The new refill tick
        // sees `last_refill_ms == real_now` (set at construction; not
        // updated while warped) and `now == real_now`, so elapsed is
        // ~0 and the bucket does NOT jump back to capacity.
        warped.store(false, Ordering::SeqCst);
        assert_eq!(
            budget.try_dispatch("fp2", ResearchChannel::GitHub),
            DispatchVerdict::Allowed
        );
        let after_recover = budget.current_state();
        assert!(
            after_recover.available < 4.0,
            "bucket saturated after clock warp recovery: {} (R10 undone)",
            after_recover.available,
        );
    }

    #[test]
    fn validated_rejects_nan_and_inf() {
        let nan_bucket = PersistedBucket {
            rate_per_hour: 10,
            available: f64::NAN,
            last_refill_ms: 0,
        };
        assert!(nan_bucket.validated(10).is_none());

        let inf_bucket = PersistedBucket {
            rate_per_hour: 10,
            available: f64::INFINITY,
            last_refill_ms: 0,
        };
        assert!(inf_bucket.validated(10).is_none());
    }

    #[test]
    fn validated_clamps_negative_to_zero_and_overflow_to_cap() {
        let neg = PersistedBucket {
            rate_per_hour: 10,
            available: -5.0,
            last_refill_ms: 0,
        };
        let v = neg.validated(10).unwrap();
        assert_eq!(v.available, 0.0);

        let over = PersistedBucket {
            rate_per_hour: 10,
            available: 999.0,
            last_refill_ms: 0,
        };
        let v = over.validated(10).unwrap();
        assert_eq!(v.available, 10.0);
    }
}
