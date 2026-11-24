//! Typed errors for the sync subsystem.
//!
//! The crate's public surface still uses `anyhow::Error` for ergonomic
//! propagation (`crate::Error` / `crate::Result`); typed variants are
//! defined here so callers can pattern-match on specific failure modes
//! after downcasting. Adapters refuse a mutating sync call with
//! [`SyncError::KillSwitchEngaged`] when the cold-window kill-switch is
//! engaged (FR12); [`SyncError::TokenBudgetExceeded`] carries the numbers
//! when the caller has them.

use thiserror::Error;

/// Typed sync-layer errors. Convertible into `anyhow::Error` via `?`.
///
/// Non-exhaustive: a match needs a `_` arm, so a new failure mode is not a
/// breaking change.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SyncError {
    /// The cold-window kill-switch is engaged: a previous tick observed
    /// cumulative token usage at or above the configured ceiling and
    /// instructed sync to refuse further mutating writes.
    ///
    /// `tokens` is the most recent observed total; `ceiling` is the
    /// configured budget. Both fields are advisory only, the gate is
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
    /// The kill-switch is engaged and the caller does not know the token
    /// numbers behind it. Adapters refuse with this: reporting
    /// `TokenBudgetExceeded { tokens: 0, ceiling: 0 }` read as "usage 0
    /// exceeded ceiling 0".
    #[error(
        "cold-window kill-switch engaged; sync writes refuse until the operator releases the switch"
    )]
    KillSwitchEngaged,
    /// The person answering an interactive sync cancelled the prompts. Every
    /// decision for a target is asked before its first write, so none of
    /// this target's items were written. A run over several targets (such
    /// as `sync-all --interactive`) has already written the earlier ones;
    /// `skrills sync-rollback` undoes them.
    #[error(
        "interactive sync cancelled before any item was written to this target; targets synced \
         earlier in the run keep their changes (`skrills sync-rollback` undoes them)"
    )]
    InteractiveAborted,
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

    /// `sync-all --interactive` writes targets one after another, so a cancel
    /// at the second target cannot promise that nothing was written.
    #[test]
    fn a_cancel_claims_only_the_current_target_and_names_rollback() {
        let msg = SyncError::InteractiveAborted.to_string();

        assert!(msg.contains("this target"), "{msg}");
        assert!(msg.contains("skrills sync-rollback"), "{msg}");
        assert!(!msg.contains("nothing was written"), "{msg}");
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
