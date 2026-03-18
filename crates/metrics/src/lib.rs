//! SQLite-based metrics collection for skrills.
//!
//! This crate provides utilities for:
//! - Recording skill invocations with timing and token usage
//! - Tracking validation run results
//! - Logging sync events (push/pull operations)
//! - Querying historical metrics and statistics
//!
//! Data is stored in `~/.skrills/metrics.db` using WAL mode for concurrent access.
//! A 30-day retention policy can be enforced via `cleanup_old_data`.
//!
//! # Examples
//!
//! ```no_run
//! use skrills_metrics::MetricsCollector;
//!
//! let collector = MetricsCollector::new().unwrap();
//! collector.record_skill_invocation("my-skill", 150, true, Some(1024)).unwrap();
//! let stats = collector.get_skill_stats("my-skill").unwrap();
//! println!("Total invocations: {}", stats.total_invocations());
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod collector;
mod error;
mod schema;
mod types;

pub use collector::{MetricsCollector, StorageMode};
pub use error::{MetricsError, Result};
pub use types::{
    AnalyticsSummary, MetricEvent, RuleAnalyticsSummary, RuleEffectiveness, RuleOutcome,
    RuleTriggerDetail, SkillStats, SyncDetail, SyncOperation, SyncStatus, SyncSummary, TopSkill,
    ValidationDetail, ValidationSummary,
};
