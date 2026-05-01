//! Cold-window engine integration.
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
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use skrills_intelligence::cold_window_hints::MultiSignalScorer;
use skrills_snapshot::{
    Alert, AlertBand, Hint, KillSwitch, LoadSample, PluginHealth, ResearchFinding, ScoredHint,
    Severity, TokenLedger, WindowSnapshot,
};

use super::alert::LayeredAlertPolicy;
use super::cadence::{CadenceStrategy, LoadAwareCadence};
use super::diff::FieldwiseDiff;
use super::plugin_health::{CollectorOutput, MalformedPlugin};
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
    /// Plugins whose `health.toml` failed to parse this tick (FR11
    /// EC5). Each entry is translated into a `Caution`-tier alert
    /// before broadcast and excluded from `plugin_health`.
    pub malformed_plugins: Vec<MalformedPlugin>,
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
            malformed_plugins: Vec::new(),
            load_sample: LoadSample::default(),
            research_findings: Vec::new(),
        }
    }

    /// Builder: ingest a [`CollectorOutput`] from
    /// [`super::PluginHealthCollector`]. Splits into `plugin_health`
    /// (snapshot wire-format) and `malformed_plugins` (alert source).
    pub fn with_plugin_collector_output(mut self, output: CollectorOutput) -> Self {
        self.plugin_health = output.healths;
        self.malformed_plugins = output.malformed;
        self
    }

    /// Builder: set malformed plugins explicitly (for tests).
    pub fn with_malformed_plugins(mut self, m: Vec<MalformedPlugin>) -> Self {
        self.malformed_plugins = m;
        self
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
    /// Consecutive ticks whose wall-clock body exceeded the configured
    /// budget. Reset on the first under-budget tick. Drives the NB1/NB2
    /// STATUS → ADVISORY escalation.
    consecutive_overruns: u32,
    /// Ticks since last `tracing::debug!` for "broadcast send had no
    /// subscribers". Throttles the log to once per
    /// [`SUBSCRIBERLESS_LOG_INTERVAL`] ticks (NI14).
    subscriberless_ticks: u32,
}

/// How many consecutive overruns of the tick budget escalate the
/// per-tick alert from `Severity::Status` (informational) to
/// `Severity::Advisory` (awareness-only). Spec NB1.
pub const TICK_OVERRUN_ADVISORY_THRESHOLD: u32 = 3;

/// Cadence at which the engine emits a `tracing::debug!` with the
/// count of broadcast-send failures (no subscribers). Once per N
/// ticks; counter resets after each emission. NI14 keeps the log
/// from spamming when no consumer is attached.
pub const SUBSCRIBERLESS_LOG_INTERVAL: u32 = 100;

/// Cold-window engine: per-tick producer of immutable
/// `Arc<WindowSnapshot>` artifacts.
///
/// Construct via [`ColdWindowEngine::with_defaults`] for the standard
/// strategy stack, or [`ColdWindowEngine::with_strategies`] when you
/// want to inject custom implementations. Both constructors yield an
/// engine wired to a fresh [`KillSwitch`]; callers that already
/// minted one (e.g. `skrills-server` for sharing with sync adapters)
/// must replace it via [`ColdWindowEngine::with_kill_switch`].
pub struct ColdWindowEngine {
    tx: broadcast::Sender<Arc<WindowSnapshot>>,
    activity: Arc<Mutex<ActivityRing>>,
    cadence: Box<dyn CadenceStrategy>,
    alert_policy: Box<dyn AlertPolicy>,
    hint_scorer: Box<dyn HintScorer>,
    diff: Box<dyn SnapshotDiff>,
    state: Arc<Mutex<EngineState>>,
    /// Hard budget for one tick body. If wall-clock exceeds this,
    /// the engine emits a `Severity::Status` alert (or
    /// `Severity::Advisory` after [`TICK_OVERRUN_ADVISORY_THRESHOLD`]
    /// consecutive overruns). Defaults to 50 ms (SC1 median budget).
    tick_budget: Duration,
    /// Externally-shared kill-switch. The engine engages it when the
    /// active alert policy reports the budget has been breached
    /// (FR12 hard kill). Cloned out via [`Self::kill_switch`] so
    /// adapters (sync, server) can observe the same flag.
    kill_switch: KillSwitch,
    /// Token budget ceiling forwarded by the alert policy. Stored so
    /// the engine can engage the kill-switch without re-classifying
    /// the snapshot itself (the policy already did).
    budget_ceiling: u64,
}

/// Default per-tick wall-clock budget. Matches the SC1 median budget
/// used by the criterion bench in `benches/tick_budget.rs`.
pub const DEFAULT_TICK_BUDGET: Duration = Duration::from_millis(50);

impl ColdWindowEngine {
    /// Construct an engine with the spec-default strategy stack and
    /// a user-supplied token-budget ceiling. Wires a fresh
    /// [`KillSwitch`]; share state by chaining
    /// [`ColdWindowEngine::with_kill_switch`].
    pub fn with_defaults(budget_ceiling: u64) -> Self {
        Self::with_strategies(
            Box::new(LoadAwareCadence::new()),
            Box::new(LayeredAlertPolicy::new(budget_ceiling)),
            Box::new(DefaultHintScorer(MultiSignalScorer::new())),
            Box::new(FieldwiseDiff::new()),
        )
        .with_budget_ceiling(budget_ceiling)
    }

    /// Construct an engine with caller-provided strategies. The token
    /// budget ceiling defaults to `u64::MAX` (kill-switch never
    /// engages); chain [`ColdWindowEngine::with_budget_ceiling`] when
    /// the alert policy uses a real ceiling.
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
            tick_budget: DEFAULT_TICK_BUDGET,
            kill_switch: KillSwitch::new(),
            budget_ceiling: u64::MAX,
        }
    }

    /// Replace the engine's [`KillSwitch`] with a caller-provided one.
    /// Used by `skrills-server` to share a single switch across the
    /// engine and the sync adapters (FR12).
    pub fn with_kill_switch(mut self, kill_switch: KillSwitch) -> Self {
        self.kill_switch = kill_switch;
        self
    }

    /// Override the per-tick wall-clock budget. Defaults to
    /// [`DEFAULT_TICK_BUDGET`].
    pub fn with_tick_budget(mut self, budget: Duration) -> Self {
        self.tick_budget = budget;
        self
    }

    /// Set the token budget ceiling that the engine will use to engage
    /// the kill-switch. Should match the ceiling configured on the
    /// alert policy.
    pub fn with_budget_ceiling(mut self, ceiling: u64) -> Self {
        self.budget_ceiling = ceiling;
        self
    }

    /// Clone out the kill-switch so adapters (sync, server, dashboard)
    /// can observe engagement without holding a reference to the
    /// engine.
    pub fn kill_switch(&self) -> KillSwitch {
        self.kill_switch.clone()
    }

    /// Subscribe to the snapshot bus.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<WindowSnapshot>> {
        self.tx.subscribe()
    }

    /// Clone the snapshot bus sender. Useful for wiring browser routes
    /// (`skrills_server::api::cold_window::ColdWindowDashboardState`)
    /// that need to construct subscribers per-request rather than
    /// holding a single long-lived receiver.
    pub fn bus_sender(&self) -> broadcast::Sender<Arc<WindowSnapshot>> {
        self.tx.clone()
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
        let tick_start = Instant::now();
        let TickInput {
            timestamp_ms,
            token_ledger,
            raw_hints,
            plugin_health,
            malformed_plugins,
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
        let mut alerts = self
            .alert_policy
            .evaluate(&prev, &snapshot, &mut state.alert_history);

        // Append CAUTION alerts for malformed plugin health.toml files
        // (FR11 EC5). These are deterministic — no hysteresis, no
        // min-dwell — because user configuration errors need
        // immediate visibility. Stable fingerprint allows downstream
        // dispatchers to dedupe across ticks.
        if !malformed_plugins.is_empty() {
            let synthetic = CollectorOutput {
                healths: Vec::new(),
                malformed: malformed_plugins,
            };
            alerts.extend(synthetic.malformed_alerts(timestamp_ms));
        }

        // B4 engine-half: engage shared kill-switch when token total
        // breaches the configured budget ceiling (FR12). One-way:
        // never released by the engine; only daemon restart clears.
        if snapshot.token_ledger.total >= self.budget_ceiling {
            self.kill_switch.engage();
        }

        // NB1 + NB2: tick-budget overrun.
        let elapsed = tick_start.elapsed();
        if elapsed > self.tick_budget {
            state.consecutive_overruns = state.consecutive_overruns.saturating_add(1);
            let severity = if state.consecutive_overruns >= TICK_OVERRUN_ADVISORY_THRESHOLD {
                Severity::Advisory
            } else {
                Severity::Status
            };
            let elapsed_ms = elapsed.as_millis() as u64;
            let budget_ms = self.tick_budget.as_millis() as u64;
            // Hysteresis band over (budget_ms .. elapsed_ms+1) — the
            // alert is "value-driven" so a band is informative even
            // though the gate logic is just elapsed > budget.
            let high = (elapsed_ms.max(budget_ms + 1)) as f64;
            let band = AlertBand::new(0.0, 0.0, high, budget_ms as f64).ok();
            alerts.push(Alert {
                fingerprint: "tick-budget-overrun".into(),
                severity,
                title: "Tick budget overrun".into(),
                message: format!(
                    "tick exceeded budget: elapsed_ms={elapsed_ms} budget_ms={budget_ms} \
                     consecutive_overruns={}",
                    state.consecutive_overruns
                ),
                band,
                fired_at_ms: timestamp_ms,
                dwell_ticks: state.consecutive_overruns,
            });
        } else {
            state.consecutive_overruns = 0;
        }

        snapshot.alerts = alerts;

        let snap_arc = Arc::new(snapshot);
        state.last_snapshot = Some(Arc::clone(&snap_arc));

        // NI14: surface broadcast send failures via a throttled debug
        // log. SendError occurs only when the channel has zero live
        // receivers — common during startup (no SSE clients yet) and
        // benign once consumers attach. Logging on every tick spams.
        match self.tx.send(Arc::clone(&snap_arc)) {
            Ok(_) => {
                state.subscriberless_ticks = 0;
            }
            Err(_) => {
                state.subscriberless_ticks = state.subscriberless_ticks.saturating_add(1);
                if state.subscriberless_ticks >= SUBSCRIBERLESS_LOG_INTERVAL {
                    tracing::debug!(
                        ticks_with_no_subscribers = state.subscriberless_ticks,
                        "cold-window broadcast send had no subscribers"
                    );
                    state.subscriberless_ticks = 0;
                }
            }
        }
        drop(state);

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
        // the integration path, not the full benchmark.
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
        // Ramp tokens 0 → 65K (14 ticks * 5K). With budget 80K (so
        // warning fires at 64K and caution=50K < warning=64K satisfies
        // tier ordering — NI6) and min_dwell 1, Warning fires by t=13
        // (65K crosses the 64K warning floor).
        let engine = ColdWindowEngine::with_strategies(
            Box::new(LoadAwareCadence::new()),
            Box::new(LayeredAlertPolicy::new(80_000).with_min_dwell(1)),
            Box::new(DefaultHintScorer(MultiSignalScorer::new())),
            Box::new(FieldwiseDiff::new()),
        );
        let mut warning_fired = false;
        for snap in chaos_sequence(14) {
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
    fn malformed_plugin_yields_caution_alert_in_snapshot() {
        // FR11 EC5: malformed plugin health.toml → CAUTION alert
        // with stable fingerprint, deterministic (no min-dwell).
        let engine = ColdWindowEngine::with_defaults(100_000);
        let malformed = vec![MalformedPlugin {
            plugin_name: "broken-plugin".into(),
            error_message: "expected `=` at line 2".into(),
        }];
        let input = TickInput::empty()
            .with_timestamp_ms(1_700_000_000_000)
            .with_malformed_plugins(malformed);
        let snap = engine.tick(input);
        let cautions: Vec<_> = snap
            .alerts
            .iter()
            .filter(|a| matches!(a.severity, Severity::Caution))
            .filter(|a| a.fingerprint.starts_with("plugin-health-malformed::"))
            .collect();
        assert_eq!(
            cautions.len(),
            1,
            "exactly one CAUTION per malformed plugin"
        );
        assert_eq!(
            cautions[0].fingerprint,
            "plugin-health-malformed::broken-plugin"
        );
        assert!(cautions[0].title.contains("broken-plugin"));
        assert!(cautions[0].message.contains("expected `=`"));
        assert_eq!(cautions[0].fired_at_ms, 1_700_000_000_000);
        assert_eq!(cautions[0].dwell_ticks, 1, "deterministic alert, no dwell");
    }

    #[test]
    fn collector_output_routes_to_plugin_health_and_alerts() {
        // FR11 + EC5: CollectorOutput.healths populates snapshot,
        // CollectorOutput.malformed becomes CAUTION alerts. The two
        // streams are disjoint — a malformed plugin never appears in
        // plugin_health.
        use skrills_snapshot::HealthStatus;
        let engine = ColdWindowEngine::with_defaults(100_000);
        let collector_output = super::super::plugin_health::CollectorOutput {
            healths: vec![PluginHealth {
                plugin_name: "good".into(),
                overall: HealthStatus::Ok,
                checks: vec![],
            }],
            malformed: vec![MalformedPlugin {
                plugin_name: "bad".into(),
                error_message: "garbage".into(),
            }],
        };
        let input = TickInput::empty().with_plugin_collector_output(collector_output);
        let snap = engine.tick(input);
        assert_eq!(snap.plugin_health.len(), 1);
        assert_eq!(snap.plugin_health[0].plugin_name, "good");
        assert!(
            snap.alerts.iter().any(|a| {
                matches!(a.severity, Severity::Caution)
                    && a.fingerprint == "plugin-health-malformed::bad"
            }),
            "expected CAUTION alert for malformed plugin"
        );
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

    // ---------- NB1 + NB2: tick budget overrun ----------

    fn slow_input() -> TickInput {
        TickInput::empty().with_timestamp_ms(1_700_000_000_000)
    }

    /// Drive a tick that exceeds the configured tick budget. We do this
    /// via a custom alert policy that sleeps inside `evaluate` for
    /// longer than the budget, ensuring deterministic overrun without
    /// relying on machine load.
    struct SlowPolicy {
        sleep: Duration,
    }
    impl AlertPolicy for SlowPolicy {
        fn evaluate(
            &self,
            _prev: &WindowSnapshot,
            _curr: &WindowSnapshot,
            _history: &mut AlertHistory,
        ) -> Vec<skrills_snapshot::Alert> {
            std::thread::sleep(self.sleep);
            Vec::new()
        }
    }

    #[test]
    fn nb1_overrun_emits_status_alert() {
        // Tight 1-ms tick budget plus a 5-ms sleep inside the policy
        // forces an overrun on every tick. First overrun → STATUS.
        let engine = ColdWindowEngine::with_strategies(
            Box::new(LoadAwareCadence::new()),
            Box::new(SlowPolicy {
                sleep: Duration::from_millis(5),
            }),
            Box::new(DefaultHintScorer(MultiSignalScorer::new())),
            Box::new(FieldwiseDiff::new()),
        )
        .with_tick_budget(Duration::from_millis(1));

        let snap = engine.tick(slow_input());
        let overrun: Vec<_> = snap
            .alerts
            .iter()
            .filter(|a| a.fingerprint == "tick-budget-overrun")
            .collect();
        assert_eq!(overrun.len(), 1);
        assert!(matches!(overrun[0].severity, Severity::Status));
        assert!(overrun[0].message.contains("elapsed_ms"));
        assert!(overrun[0].message.contains("budget_ms"));
    }

    #[test]
    fn nb2_three_consecutive_overruns_escalate_to_advisory() {
        let engine = ColdWindowEngine::with_strategies(
            Box::new(LoadAwareCadence::new()),
            Box::new(SlowPolicy {
                sleep: Duration::from_millis(3),
            }),
            Box::new(DefaultHintScorer(MultiSignalScorer::new())),
            Box::new(FieldwiseDiff::new()),
        )
        .with_tick_budget(Duration::from_millis(1));

        let mut last_severity: Option<Severity> = None;
        for _ in 0..3 {
            let snap = engine.tick(slow_input());
            let overrun = snap
                .alerts
                .iter()
                .find(|a| a.fingerprint == "tick-budget-overrun")
                .expect("overrun alert");
            last_severity = Some(overrun.severity);
        }
        assert!(matches!(last_severity, Some(Severity::Advisory)));
    }

    #[test]
    fn nb1_under_budget_resets_overrun_counter() {
        // Use the no-sleep default policy with a generous tick budget;
        // counter must remain zero, no overrun alert appears.
        let engine =
            ColdWindowEngine::with_defaults(100_000).with_tick_budget(Duration::from_secs(10));
        let snap = engine.tick(TickInput::empty());
        let overrun = snap
            .alerts
            .iter()
            .find(|a| a.fingerprint == "tick-budget-overrun");
        assert!(
            overrun.is_none(),
            "no overrun expected on under-budget tick"
        );
    }

    // ---------- B4 engine-half: KillSwitch ----------

    #[test]
    fn b4_kill_switch_engages_on_budget_breach() {
        let engine = ColdWindowEngine::with_defaults(50_000);
        let switch = engine.kill_switch();
        assert!(!switch.is_engaged(), "switch starts disengaged");
        let input = TickInput::empty().with_token_ledger(TokenLedger {
            total: 50_000,
            ..Default::default()
        });
        let _ = engine.tick(input);
        assert!(
            switch.is_engaged(),
            "switch must engage at or above budget ceiling"
        );
    }

    #[test]
    fn b4_kill_switch_stays_disengaged_below_ceiling() {
        let engine = ColdWindowEngine::with_defaults(100_000);
        let switch = engine.kill_switch();
        let input = TickInput::empty().with_token_ledger(TokenLedger {
            total: 50_000,
            ..Default::default()
        });
        let _ = engine.tick(input);
        assert!(!switch.is_engaged());
    }

    #[test]
    fn b4_with_kill_switch_replaces_engine_switch() {
        let external = KillSwitch::new();
        let engine = ColdWindowEngine::with_defaults(50_000).with_kill_switch(external.clone());
        let input = TickInput::empty().with_token_ledger(TokenLedger {
            total: 60_000,
            ..Default::default()
        });
        let _ = engine.tick(input);
        assert!(
            external.is_engaged(),
            "externally-cloned switch must observe engagement"
        );
    }

    // ---------- NI14: broadcast send debug log (no subscribers) ----------

    #[test]
    fn ni14_no_subscribers_does_not_panic_on_repeated_ticks() {
        // Driver test: many ticks with zero subscribers must never
        // panic. The engine throttles its `tracing::debug!` to once
        // per SUBSCRIBERLESS_LOG_INTERVAL ticks (100). We validate
        // robustness by running >2x the interval.
        let engine = ColdWindowEngine::with_defaults(100_000);
        for _ in 0..(SUBSCRIBERLESS_LOG_INTERVAL * 2 + 5) {
            let _ = engine.tick(TickInput::empty());
        }
        // Reaching here means no panic, no allocator blowup. Snapshot
        // version counter advanced by full count.
        let last = engine.last_snapshot().expect("at least one tick");
        assert!(last.version >= u64::from(SUBSCRIBERLESS_LOG_INTERVAL * 2 + 5));
    }

    #[tokio::test]
    async fn ni14_attached_subscriber_keeps_send_succeeding() {
        // Sanity check: with a subscriber attached, broadcast send
        // succeeds and the engine never increments its counter.
        let engine = ColdWindowEngine::with_defaults(100_000);
        let mut rx = engine.subscribe();
        for _ in 0..10 {
            let _ = engine.tick(TickInput::empty());
            let _ = rx.recv().await;
        }
        // No assertion on internal counter (private), but the receive
        // side observed each tick — the send path must have succeeded.
    }

    #[test]
    fn b4_kill_switch_engagement_is_one_way() {
        // Once engaged, dropping back below the ceiling must NOT
        // release the switch (FR12: only daemon restart clears).
        let engine = ColdWindowEngine::with_defaults(50_000);
        let switch = engine.kill_switch();
        let _ = engine.tick(TickInput::empty().with_token_ledger(TokenLedger {
            total: 60_000,
            ..Default::default()
        }));
        assert!(switch.is_engaged());
        let _ = engine.tick(TickInput::empty().with_token_ledger(TokenLedger {
            total: 0,
            ..Default::default()
        }));
        assert!(switch.is_engaged(), "engagement must be one-way");
    }
}
