//! Typed errors for the sync subsystem.
//!
//! The crate's public surface still uses `anyhow::Error` for ergonomic
//! propagation (`crate::Error` / `crate::Result`); typed variants are
//! defined here so callers can pattern-match on specific failure modes
//! after downcasting. Currently the only such variant is
//! [`SyncError::TokenBudgetExceeded`], emitted when the cold-window
//! kill-switch is engaged at the start of a mutating sync call (FR12).

use thiserror::Error;

/// Typed sync-layer errors. Convertible into `anyhow::Error` via `?`.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// The cold-window kill-switch is engaged: a previous tick observed
    /// cumulative token usage at or above the configured ceiling and
    /// instructed sync to refuse further mutating writes.
    ///
    /// `tokens` is the most recent observed total; `ceiling` is the
    /// configured budget. Both fields are advisory only — the gate is
    /// the boolean kill-switch, not the numeric comparison.
    #[error(
        "cold-window kill-switch engaged: token usage {tokens} exceeded ceiling {ceiling}; \
         subsequent sync writes refuse until the operator releases the switch"
    )]
    TokenBudgetExceeded {
        /// Cumulative tokens observed at the time of engagement.
        tokens: u64,
        /// Configured `--alert-budget` ceiling.
        ceiling: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_budget_exceeded_includes_both_numbers() {
        let err = SyncError::TokenBudgetExceeded {
            tokens: 150_000,
            ceiling: 100_000,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("150000"));
        assert!(msg.contains("100000"));
        assert!(msg.contains("kill-switch"));
    }

    #[test]
    fn round_trips_through_anyhow() {
        let err = SyncError::TokenBudgetExceeded {
            tokens: 1,
            ceiling: 0,
        };
        let any: anyhow::Error = err.into();
        let downcast: &SyncError = any.downcast_ref().expect("downcast");
        assert!(matches!(
            downcast,
            SyncError::TokenBudgetExceeded {
                tokens: 1,
                ceiling: 0
            }
        ));
    }
}
