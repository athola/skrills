//! Wire-format types for the cold-window snapshot.
//!
//! Type design rules (proto-friendly, per `docs/cold-window-brief.md` § 5.8):
//!
//! - All enum variants use tagged unions where they carry payload (proto3
//!   `oneof`-compatible). Unit-only enums serialize as bare strings
//!   (proto3 `enum`-compatible).
//! - All `Option<T>` fields stay explicit; no `#[serde(default)]` shortcuts
//!   that would erase the optional/required distinction at the wire level.
//! - All collections are `Vec<T>` or maps with primitive keys.
//! - All timestamps are `u64` milliseconds since UNIX epoch (mappable to
//!   `google.protobuf.Timestamp` when the gRPC follow-up lands in v0.9.0).

use serde::{Deserialize, Serialize};

/// A single immutable snapshot of the cold-window's view of the
/// skrills ecosystem at one tick.
///
/// Producers create exactly one of these per tick. Both the TUI
/// (`skrills_dashboard`) and the browser SSE handler
/// (`skrills_server`) render this same artifact; drift between
/// surfaces is structurally impossible because the artifact is the
/// contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSnapshot {
    /// Monotonic snapshot version (incremented per tick).
    pub version: u64,
    /// Wall-clock timestamp at tick start (UNIX epoch milliseconds).
    pub timestamp_ms: u64,
    /// Per-source token attribution.
    pub token_ledger: TokenLedger,
    /// Active alerts in tier-then-recency order.
    pub alerts: Vec<Alert>,
    /// Ranked hints (highest-score first).
    pub hints: Vec<ScoredHint>,
    /// Research findings attached to this tick (may be empty).
    pub research_findings: Vec<ResearchFinding>,
    /// Plugin health reports (from `health.toml` participants).
    pub plugin_health: Vec<PluginHealth>,
    /// Load sample driving adaptive cadence at this tick.
    pub load_sample: LoadSample,
    /// Effective duration until the next tick fires (milliseconds).
    pub next_tick_ms: u64,
}

impl WindowSnapshot {
    /// Human-readable cadence label including the adaptive-state suffix.
    ///
    /// Format: `tick: {secs:.1}s [active edit | load N.NN | base]`. The
    /// suffix priority is `active edit` (last edit younger than 10s) >
    /// `load` (loadavg above zero) > `base` (default). Both the HTTP
    /// SSE handler and the TUI status bar render this exact string so
    /// surfaces stay byte-for-byte identical.
    #[must_use]
    pub fn cadence_label(&self) -> String {
        let secs = (self.next_tick_ms as f64) / 1_000.0;
        let suffix = match self.load_sample.last_edit_age_ms {
            Some(age) if age < 10_000 => "[active edit]".to_string(),
            _ => {
                if self.load_sample.loadavg_1min > 0.0 {
                    format!("[load {:.2}]", self.load_sample.loadavg_1min)
                } else {
                    "[base]".to_string()
                }
            }
        };
        format!("tick: {secs:.1}s {suffix}")
    }
}

/// 4-tier alert severity, mapped from FAA AC 25.1322-1 cockpit CAS.
///
/// User-facing behavior per `docs/cold-window-spec.md` § 3.4:
/// `Warning` interrupts; `Caution` and below are panel-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Hard limit breached; requires per-row dismissal; bell on first sight.
    Warning,
    /// Threshold crossed with hysteresis; panel-visible; amber visual cue.
    Caution,
    /// Awareness-only; cyan; cleared by master-ack.
    Advisory,
    /// Informational; auto-clears when condition resolves.
    Status,
}

/// Hysteresis-banded threshold for an alert: re-arming requires
/// re-crossing the matching `*_clear` value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlertBand {
    /// Lower fire threshold.
    pub low: f64,
    /// Lower re-arm threshold (must re-cross to fire again).
    pub low_clear: f64,
    /// Upper fire threshold.
    pub high: f64,
    /// Upper re-arm threshold.
    pub high_clear: f64,
}

/// A single alert raised by the alert policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    /// Stable identifier for grouping and dedup.
    pub fingerprint: String,
    /// Severity tier.
    pub severity: Severity,
    /// Short headline.
    pub title: String,
    /// Detailed message.
    pub message: String,
    /// Threshold band when the alert is value-driven.
    pub band: Option<AlertBand>,
    /// First-fire timestamp (UNIX epoch ms).
    pub fired_at_ms: u64,
    /// How many consecutive ticks the underlying condition has held.
    pub dwell_ticks: u32,
}

/// Categories for hints; mirrors the recommender's signal taxonomy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HintCategory {
    /// Token-cost reduction opportunity.
    Token,
    /// Validation regression or failure.
    Validation,
    /// Redundant capability across skills/plugins.
    Redundancy,
    /// Sync drift between sources.
    SyncDrift,
    /// Skill quality score deviation.
    Quality,
}

/// A raw hint produced by the intelligence crate's recommender.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hint {
    /// Skill or plugin URI the hint targets.
    pub uri: String,
    /// Hint category.
    pub category: HintCategory,
    /// Human-readable description.
    pub message: String,
    /// How often this hint surfaces (frequency signal).
    pub frequency: u32,
    /// Estimated impact magnitude (0.0–10.0).
    pub impact: f64,
    /// Estimated remediation ease (0.0–10.0; higher = easier).
    pub ease_score: f64,
    /// Age in days since the underlying signal first appeared.
    pub age_days: f64,
}

/// A hint with its computed score, ready for ranked display.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoredHint {
    /// The original hint.
    pub hint: Hint,
    /// Combined score from the active `HintScorer`.
    pub score: f64,
    /// User-pinned (sticks to top regardless of score).
    pub pinned: bool,
}

/// Source channel for a research finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResearchChannel {
    /// GitHub code search.
    GitHub,
    /// Hacker News discourse.
    HackerNews,
    /// Lobsters discourse.
    Lobsters,
    /// Academic paper (arXiv, Semantic Scholar).
    Paper,
    /// TRIZ cross-domain analogy.
    Triz,
}

/// A research finding surfaced asynchronously by the tome dispatcher.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResearchFinding {
    /// Topic fingerprint that triggered this fetch.
    pub fingerprint: String,
    /// Source channel.
    pub channel: ResearchChannel,
    /// Result title.
    pub title: String,
    /// Canonical URL.
    pub url: String,
    /// Channel-specific score (e.g. HN points, GitHub stars, semantic relevance).
    pub score: f64,
    /// When the dispatcher fetched this (UNIX epoch ms).
    pub fetched_at_ms: u64,
}

/// One row of the per-source token ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenEntry {
    /// Source identifier (skill URI, plugin name, MCP name, etc.).
    pub source: String,
    /// Estimated tokens attributed to this source.
    pub tokens: u64,
}

/// Itemized token attribution for the snapshot.
///
/// Per `docs/cold-window-spec.md` § 3.3 and § 3.4: alerts at 20K
/// (quadratic inflection), 50K (Willison MCP-overhead range), and
/// 100% of `--alert-budget`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenLedger {
    /// Per-skill attribution.
    pub per_skill: Vec<TokenEntry>,
    /// Per-plugin attribution (sum of skills + commands + agents).
    pub per_plugin: Vec<TokenEntry>,
    /// Per-MCP-server attribution (tool descriptions are the largest drain).
    pub per_mcp: Vec<TokenEntry>,
    /// Cumulative cache reads in active conversations.
    pub conversation_cache_reads: u64,
    /// Cumulative cache writes in active conversations.
    pub conversation_cache_writes: u64,
    /// Total tokens across all sources.
    pub total: u64,
}

/// Aggregate health status for a plugin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// All checks passing.
    #[default]
    Ok,
    /// At least one warn-tier check.
    Warn,
    /// At least one error-tier check.
    Error,
    /// Plugin participation declared but checks unreachable.
    Unknown,
}

/// One named check inside a plugin's `health.toml` report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HealthCheck {
    /// Check identifier.
    pub name: String,
    /// Status of this individual check.
    pub status: HealthStatus,
    /// Optional human-readable detail.
    pub message: Option<String>,
}

/// Health report for a single plugin participating via `health.toml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginHealth {
    /// Plugin name (matches discovery).
    pub plugin_name: String,
    /// Aggregate status across `checks`.
    pub overall: HealthStatus,
    /// Individual checks.
    pub checks: Vec<HealthCheck>,
}

/// Sampled load signal driving the adaptive cadence policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoadSample {
    /// 1-minute load average from `/proc/loadavg` (or 0.0 on platforms
    /// without it; in which case cadence falls back to the base value).
    pub loadavg_1min: f64,
    /// Time since the most recent skill / plugin source-file edit
    /// (milliseconds). `None` when no recent edit has been observed.
    pub last_edit_age_ms: Option<u64>,
}

/// Decide whether the research dispatcher should issue an external
/// fetch for a given topic fingerprint.
///
/// Defines the contract between the cold-window producer
/// (`skrills_analyze::cold_window`) and the dispatcher
/// implementation (`skrills_tome::dispatcher::BucketedBudget` in
/// production). Co-located with [`WindowSnapshot`] because the
/// trait's argument type lives here and the trait itself is part of
/// the producer–consumer contract this crate carries.
pub trait ResearchBudget: Send + Sync {
    /// Return true when an external fetch is permitted for the given
    /// fingerprint at this moment, false when the budget refuses.
    fn should_query(
        &self,
        snapshot: &WindowSnapshot,
        topic_fingerprint: &str,
        last_query: Option<std::time::Instant>,
    ) -> bool;
}
