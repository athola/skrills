//! Context usage statistics tracking.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Atomic context statistics for thread-safe updates.
#[derive(Debug, Default)]
pub struct ContextStats {
    /// Estimated tokens saved by lazy loading.
    pub tokens_saved: AtomicU64,
    /// Total tool schemas loaded this session.
    pub schemas_loaded: AtomicU64,
    /// Total tool invocations this session.
    pub total_invocations: AtomicU64,
    /// Per-category token estimates.
    category_tokens: parking_lot::RwLock<HashMap<String, u64>>,
}

impl ContextStats {
    /// Create new context stats.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record tokens saved by not loading a schema.
    pub fn record_tokens_saved(&self, tokens: u64) {
        self.tokens_saved.fetch_add(tokens, Ordering::Relaxed);
    }

    /// Record a schema load.
    pub fn record_schema_load(&self) {
        self.schemas_loaded.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a tool invocation.
    pub fn record_invocation(&self) {
        self.total_invocations.fetch_add(1, Ordering::Relaxed);
    }

    /// Update category token estimate.
    pub fn set_category_tokens(&self, category: &str, tokens: u64) {
        self.category_tokens
            .write()
            .insert(category.to_string(), tokens);
    }

    /// Get current stats snapshot.
    pub fn snapshot(&self) -> ContextStatsSnapshot {
        ContextStatsSnapshot {
            tokens_saved: self.tokens_saved.load(Ordering::Relaxed),
            schemas_loaded: self.schemas_loaded.load(Ordering::Relaxed),
            total_invocations: self.total_invocations.load(Ordering::Relaxed),
            category_tokens: self.category_tokens.read().clone(),
        }
    }
}

/// Serializable snapshot of context stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextStatsSnapshot {
    /// Estimated tokens saved by lazy loading.
    pub tokens_saved: u64,
    /// Total tool schemas loaded this session.
    pub schemas_loaded: u64,
    /// Total tool invocations this session.
    pub total_invocations: u64,
    /// Per-category token estimates.
    pub category_tokens: HashMap<String, u64>,
}

impl ContextStatsSnapshot {
    /// Calculate efficiency ratio (invocations per schema load).
    pub fn efficiency_ratio(&self) -> f64 {
        if self.schemas_loaded == 0 {
            f64::INFINITY
        } else {
            self.total_invocations as f64 / self.schemas_loaded as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_stats() {
        let stats = ContextStats::new();
        stats.record_tokens_saved(100);
        stats.record_tokens_saved(50);
        stats.record_schema_load();
        stats.record_invocation();
        stats.record_invocation();

        let snapshot = stats.snapshot();
        assert_eq!(snapshot.tokens_saved, 150);
        assert_eq!(snapshot.schemas_loaded, 1);
        assert_eq!(snapshot.total_invocations, 2);
        assert_eq!(snapshot.efficiency_ratio(), 2.0);
    }
}
