//! Cold-window engine integration (TASK-008 GREEN phase).
//!
//! Wires the strategy traits declared in
//! [`super::traits`] into a single `tick()` loop. The engine takes
//! a [`TickInput`] (already-collected token attribution, raw hints,
//! plugin health, load sample, and any newly-arrived research
//! findings), builds a [`WindowSnapshot`], runs the alert policy and
//! hint scorer over it, broadcasts the result, and returns it.
//!
//! State carried across ticks:
//!
//! - **Last snapshot** — used as `prev` by [`SnapshotDiff`] and
//!   [`AlertPolicy`].
//! - **Alert history** — per-fingerprint dwell counters that drive
//!   the min-dwell timer in `LayeredAlertPolicy`.
//! - **Snapshot version** — monotonic, increments per tick.
//!
//! The engine uses `Box<dyn TraitName>` for strategies so callers
//! can swap in custom implementations at runtime. Default
//! constructors wire up [`LoadAwareCadence`], [`LayeredAlertPolicy`],
//! [`MultiSignalScorer`] (via the [`DefaultHintScorer`] adapter),
//! and [`FieldwiseDiff`].

use std::sync::Arc;

use parking_lot::Mutex;
use skrills_intelligence::cold_window_hints::MultiSignalScorer;
use skrills_snapshot::{
    Hint, LoadSample, PluginHealth, ResearchFinding, ScoredHint, TokenLedger, WindowSnapshot,
};

use super::alert::LayeredAlertPolicy;
use super::cadence::{CadenceStrategy, LoadAwareCadence};
use super::diff::FieldwiseDiff;
use super::traits::{AlertHistory, AlertPolicy, HintScorer, SnapshotDiff};
use super::{ActivityRing, ACTIVITY_RING_CAPACITY, SNAPSHOT_CHANNEL_CAPACITY};

use tokio::sync::broadcast;

/// Inputs to a single engine tick.
///
/// Producers (discovery walk, token attribution, MCP enumeration,
/// metrics queries, tome dispatcher) collect their results into one
/// of these structs and hand it to [`ColdWindowEngine::tick`]. The
/// engine adds version + alerts + hints + cadence and broadcasts.
#[derive(Debug, Clone)]
pub struct TickInput {
    /// Wall-clock timestamp at tick start (UNIX epoch milliseconds).
    pub timestamp_ms: u64,
    /// Per-source token attribution from `analyze::tokens` (T009).
    pub token_ledger: TokenLedger,
    /// Raw hints from the recommender (pre-scoring).
    pub raw_hints: Vec<Hint>,
    /// Plugin health reports (from `health.toml` participants).
    pub plugin_health: Vec<PluginHealth>,
    /// Load sample for adaptive cadence.
    pub load_sample: LoadSample,
    /// Research findings to attach to this tick's snapshot. The
    /// dispatcher (T011) lives outside the engine and feeds findings
    /// in here when they arrive asynchronously.
    pub research_findings: Vec<ResearchFinding>,
}

impl TickInput {
    /// Build an empty input (timestamp 0, all collections empty).
    /// Convenient starting point for tests.
    pub fn empty() -> Self {
        Self {
            timestamp_ms: 0,
            token_ledger: TokenLedger::default(),
            raw_hints: Vec::new(),
            plugin_health: Vec::new(),
            load_sample: LoadSample::default(),
            research_findings: Vec::new(),
        }
    }

    /// Builder: set the timestamp.
    pub fn with_timestamp_ms(mut self, ms: u64) -> Self {
        self.timestamp_ms = ms;
        self
    }

    /// Builder: set the token ledger.
    pub fn with_token_ledger(mut self, ledger: TokenLedger) -> Self {
        self.token_ledger = ledger;
        self
    }

    /// Builder: set raw hints.
    pub fn with_raw_hints(mut self, hints: Vec<Hint>) -> Self {
        self.raw_hints = hints;
        self
    }

    /// Builder: set plugin health reports.
    pub fn with_plugin_health(mut self, health: Vec<PluginHealth>) -> Self {
        self.plugin_health = health;
        self
    }

    /// Builder: set load sample.
    pub fn with_load_sample(mut self, sample: LoadSample) -> Self {
        self.load_sample = sample;
        self
    }

    /// Builder: set research findings.
    pub fn with_research_findings(mut self, findings: Vec<ResearchFinding>) -> Self {
        self.research_findings = findings;
        self
    }
}

/// Adapter wrapping [`MultiSignalScorer`] so it satisfies the
/// `HintScorer` trait declared in [`super::traits`].
///
/// The intelligence crate cannot depend on analyze (would create a
/// cycle), so the trait shape is duplicated there and bridged here.
pub struct DefaultHintScorer(pub MultiSignalScorer);

impl HintScorer for DefaultHintScorer {
    fn rank(&self, hints: Vec<Hint>) -> Vec<ScoredHint> {
        self.0.rank(hints)
    }
}

/// Per-tick state carried across calls to [`ColdWindowEngine::tick`].
#[derive(Default)]
struct EngineState {
    last_snapshot: Option<Arc<WindowSnapshot>>,
    alert_history: AlertHistory,
    version: u64,
}

/// Cold-window engine: per-tick producer of immutable
/// `Arc<WindowSnapshot>` artifacts.
///
/// Construct via [`ColdWindowEngine::with_defaults`] for the standard
/// strategy stack, or [`ColdWindowEngine::with_strategies`] when you
/// want to inject custom implementations.
pub struct ColdWindowEngine {
    tx: broadcast::Sender<Arc<WindowSnapshot>>,
    activity: Arc<Mutex<ActivityRing>>,
    cadence: Box<dyn CadenceStrategy>,
    alert_policy: Box<dyn AlertPolicy>,
    hint_scorer: Box<dyn HintScorer>,
    diff: Box<dyn SnapshotDiff>,
    state: Arc<Mutex<EngineState>>,
}

impl ColdWindowEngine {
    /// Construct an engine with the spec-default strategy stack and
    /// a user-supplied token-budget ceiling.
    pub fn with_defaults(budget_ceiling: u64) -> Self {
        Self::with_strategies(
            Box::new(LoadAwareCadence::new()),
            Box::new(LayeredAlertPolicy::new(budget_ceiling)),
            Box::new(DefaultHintScorer(MultiSignalScorer::new())),
            Box::new(FieldwiseDiff::new()),
        )
    }

    /// Construct an engine with caller-provided strategies.
    pub fn with_strategies(
        cadence: Box<dyn CadenceStrategy>,
        alert_policy: Box<dyn AlertPolicy>,
        hint_scorer: Box<dyn HintScorer>,
        diff: Box<dyn SnapshotDiff>,
    ) -> Self {
        let (tx, _) = broadcast::channel(SNAPSHOT_CHANNEL_CAPACITY);
        Self {
            tx,
            activity: Arc::new(Mutex::new(ActivityRing::with_capacity(
                ACTIVITY_RING_CAPACITY,
            ))),
            cadence,
            alert_policy,
            hint_scorer,
            diff,
            state: Arc::new(Mutex::new(EngineState::default())),
        }
    }

    /// Subscribe to the snapshot bus.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<WindowSnapshot>> {
        self.tx.subscribe()
    }

    /// Number of currently attached subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Append an activity entry to the bounded ring.
    pub fn record_activity(&self, entry: impl Into<String>) {
        self.activity.lock().push(entry.into());
    }

    /// Snapshot the activity ring (oldest first).
    pub fn activity_snapshot(&self) -> Vec<String> {
        self.activity.lock().snapshot()
    }

    /// Snapshot the carried alert history (for tests + status bar).
    pub fn alert_history(&self) -> AlertHistory {
        self.state.lock().alert_history.clone()
    }

    /// Most recent snapshot, if any tick has been processed.
    pub fn last_snapshot(&self) -> Option<Arc<WindowSnapshot>> {
        self.state.lock().last_snapshot.clone()
    }

    /// Run a single tick: produce a `WindowSnapshot` from the input,
    /// run alert policy + hint scorer + diff against the carried
    /// state, broadcast, and return the new snapshot.
    pub fn tick(&self, input: TickInput) -> Arc<WindowSnapshot> {
        let TickInput {
            timestamp_ms,
            token_ledger,
            raw_hints,
            plugin_health,
            load_sample,
            research_findings,
        } = input;

        // Compute next-tick cadence based on current load.
        let next_tick_ms = self.cadence.next_tick(load_sample).as_millis() as u64;

        // Score hints up front; they're independent of prev/curr diff.
        let hints = self.hint_scorer.rank(raw_hints);

        // Build the candidate snapshot. Alerts come last because
        // policy needs the (otherwise complete) snapshot as `curr`.
        let mut snapshot = WindowSnapshot {
            version: 0, // assigned below under lock
            timestamp_ms,
            token_ledger,
            alerts: Vec::new(),
            hints,
            research_findings,
            plugin_health,
            load_sample,
            next_tick_ms,
        };

        let mut state = self.state.lock();
        state.version += 1;
        snapshot.version = state.version;

        // Diff against prior snapshot (used by callers, optional here);
        // we run it for the side-effect of validating the policy stack
        // even when nothing observes the diff directly.
        let prev = state.last_snapshot.as_deref().cloned().unwrap_or_else(|| {
            // Construct an empty baseline for first-tick diff.
            WindowSnapshot {
                version: 0,
                timestamp_ms: 0,
                token_ledger: TokenLedger::default(),
                alerts: Vec::new(),
                hints: Vec::new(),
                research_findings: Vec::new(),
                plugin_health: Vec::new(),
                load_sample: LoadSample::default(),
                next_tick_ms: 0,
            }
        });
        let _diff_fields = self.diff.is_alertable(&prev, &snapshot);

        // Run alert policy. The policy mutates history to track
        // dwell counters even on ticks that haven't yet hit min-dwell;
        // the engine doesn't need to update history separately.
        let alerts = self
            .alert_policy
            .evaluate(&prev, &snapshot, &mut state.alert_history);
        snapshot.alerts = alerts;

        let snap_arc = Arc::new(snapshot);
        state.last_snapshot = Some(Arc::clone(&snap_arc));
        drop(state);

        let _ = self.tx.send(Arc::clone(&snap_arc));
        snap_arc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::{HintCategory, ResearchChannel, Severity, TokenEntry};
    use skrills_test_utils::cold_window_fixtures::{
        active_edit_sample, chaos_sequence, high_load_sample, sample_hint, standard_snapshot,
    };

    fn input_from_snapshot(s: &WindowSnapshot) -> TickInput {
        TickInput::empty()
            .with_timestamp_ms(s.timestamp_ms)
            .with_token_ledger(s.token_ledger.clone())
            .with_plugin_health(s.plugin_health.clone())
            .with_load_sample(s.load_sample)
    }

    #[test]
    fn first_tick_increments_version_to_one() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        let snap = engine.tick(input_from_snapshot(&standard_snapshot()));
        assert_eq!(snap.version, 1);
    }

    #[test]
    fn version_monotonically_increments() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        let s1 = engine.tick(input_from_snapshot(&standard_snapshot()));
        let s2 = engine.tick(input_from_snapshot(&standard_snapshot()));
        let s3 = engine.tick(input_from_snapshot(&standard_snapshot()));
        assert_eq!(s1.version, 1);
        assert_eq!(s2.version, 2);
        assert_eq!(s3.version, 3);
    }

    #[test]
    fn tick_under_typical_input_completes_under_50ms() {
        // Median tick budget per spec SC1. This is a smoke test on
        // the integration path, not the full benchmark (TASK-024).
        let engine = ColdWindowEngine::with_defaults(100_000);
        let input = input_from_snapshot(&standard_snapshot());
        let start = std::time::Instant::now();
        let _ = engine.tick(input);
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "tick took {elapsed:?}, expected <50ms"
        );
    }

    #[test]
    fn cadence_speeds_up_on_recent_edit() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        let input = TickInput::empty().with_load_sample(active_edit_sample());
        let snap = engine.tick(input);
        // base 2s, recent edit halves to 1s.
        assert_eq!(snap.next_tick_ms, 1_000);
    }

    #[test]
    fn cadence_backs_off_under_heavy_load() {
        // Pin cores=4 explicitly so the load-ratio threshold is
        // deterministic across machines (some test runners have
        // higher core counts than the fixture's load value).
        let engine = ColdWindowEngine::with_strategies(
            Box::new(LoadAwareCadence::new().with_cores(4)),
            Box::new(LayeredAlertPolicy::new(100_000)),
            Box::new(DefaultHintScorer(MultiSignalScorer::new())),
            Box::new(FieldwiseDiff::new()),
        );
        let input = TickInput::empty().with_load_sample(high_load_sample());
        let snap = engine.tick(input);
        // load_ratio = 4.0 / 4 = 1.0 > 0.9 → base * 4 = 8s, capped at max = 8s.
        assert_eq!(snap.next_tick_ms, 8_000);
    }

    #[test]
    fn hints_are_scored_and_ranked() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        let input = TickInput::empty().with_raw_hints(vec![
            sample_hint("a", HintCategory::Token, 1, 1.0, 5.0, 30.0),
            sample_hint("b", HintCategory::Token, 10, 9.0, 1.0, 0.0),
            sample_hint("c", HintCategory::Token, 5, 5.0, 5.0, 5.0),
        ]);
        let snap = engine.tick(input);
        assert_eq!(snap.hints.len(), 3);
        // Highest-scoring hint goes first.
        assert!(snap.hints[0].score >= snap.hints[1].score);
        assert!(snap.hints[1].score >= snap.hints[2].score);
        assert_eq!(snap.hints[0].hint.uri, "b");
    }

    #[test]
    fn chaos_sequence_eventually_fires_warning() {
        // Ramp tokens 0 → 50K (10 ticks * 5K). With budget 30K and
        // min_dwell 1, we expect Warning tier to fire by tick 8.
        let engine = ColdWindowEngine::with_strategies(
            Box::new(LoadAwareCadence::new()),
            Box::new(LayeredAlertPolicy::new(30_000).with_min_dwell(1)),
            Box::new(DefaultHintScorer(MultiSignalScorer::new())),
            Box::new(FieldwiseDiff::new()),
        );
        let mut warning_fired = false;
        for snap in chaos_sequence(11) {
            let result = engine.tick(input_from_snapshot(&snap));
            if result
                .alerts
                .iter()
                .any(|a| matches!(a.severity, Severity::Warning))
            {
                warning_fired = true;
            }
        }
        assert!(
            warning_fired,
            "expected a Warning-tier alert in chaos sequence"
        );
    }

    #[test]
    fn alert_history_persists_across_ticks() {
        let engine = ColdWindowEngine::with_strategies(
            Box::new(LoadAwareCadence::new()),
            Box::new(LayeredAlertPolicy::new(100_000).with_min_dwell(2)),
            Box::new(DefaultHintScorer(MultiSignalScorer::new())),
            Box::new(FieldwiseDiff::new()),
        );

        // Tick 1: condition crosses Advisory threshold (20K).
        // dwell becomes 1, < min_dwell 2, no alert fires yet.
        let mut input = TickInput::empty();
        input.token_ledger = TokenLedger {
            total: 25_000,
            ..Default::default()
        };
        let s1 = engine.tick(input.clone());
        assert!(s1.alerts.is_empty(), "first tick under min_dwell, no alert");

        // Tick 2: dwell becomes 2, >= min_dwell 2, alert fires.
        let s2 = engine.tick(input);
        assert!(
            !s2.alerts.is_empty(),
            "second tick should fire after min_dwell"
        );
        assert_eq!(s2.alerts[0].dwell_ticks, 2);
    }

    #[test]
    fn research_findings_passed_through_to_snapshot() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        use skrills_test_utils::cold_window_fixtures::sample_research_finding;
        let findings = vec![sample_research_finding(
            "fp",
            ResearchChannel::GitHub,
            "test finding",
            5.0,
        )];
        let input = TickInput::empty().with_research_findings(findings);
        let snap = engine.tick(input);
        assert_eq!(snap.research_findings.len(), 1);
        assert_eq!(snap.research_findings[0].title, "test finding");
    }

    #[tokio::test]
    async fn subscriber_receives_each_tick() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        let mut rx = engine.subscribe();
        let _ = engine.tick(input_from_snapshot(&standard_snapshot()));
        let received = rx.recv().await.expect("recv");
        assert_eq!(received.version, 1);
    }

    #[test]
    fn last_snapshot_returns_most_recent() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        assert!(engine.last_snapshot().is_none());
        let _ = engine.tick(input_from_snapshot(&standard_snapshot()));
        let last = engine.last_snapshot().expect("last set");
        assert_eq!(last.version, 1);
    }

    #[test]
    fn default_hint_scorer_adapts_intelligence_scorer() {
        // Compile-time + behavioral guard: DefaultHintScorer impls
        // analyze's HintScorer trait by wrapping intelligence's
        // MultiSignalScorer. Bridges the trait-shape duplication
        // between the two crates.
        let scorer: Box<dyn HintScorer> = Box::new(DefaultHintScorer(MultiSignalScorer::new()));
        let ranked = scorer.rank(vec![
            sample_hint("a", HintCategory::Token, 1, 1.0, 5.0, 0.0),
            sample_hint("b", HintCategory::Token, 10, 9.0, 1.0, 0.0),
        ]);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].hint.uri, "b");
    }

    #[test]
    fn empty_input_produces_empty_snapshot_with_defaults() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        let snap = engine.tick(TickInput::empty());
        assert_eq!(snap.token_ledger.total, 0);
        assert!(snap.hints.is_empty());
        assert!(snap.alerts.is_empty());
        assert!(snap.research_findings.is_empty());
        assert!(snap.plugin_health.is_empty());
    }

    #[test]
    fn token_ledger_passed_through_unchanged() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        let ledger = TokenLedger {
            per_skill: vec![TokenEntry {
                source: "x".into(),
                tokens: 42,
            }],
            total: 42,
            ..Default::default()
        };
        let input = TickInput::empty().with_token_ledger(ledger.clone());
        let snap = engine.tick(input);
        assert_eq!(snap.token_ledger.total, 42);
        assert_eq!(snap.token_ledger.per_skill.len(), 1);
    }
}
