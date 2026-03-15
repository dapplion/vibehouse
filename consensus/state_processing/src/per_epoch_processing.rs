#![deny(clippy::wildcard_imports)]

use crate::metrics;
pub use epoch_processing_summary::{EpochProcessingSummary, ParticipationEpochSummary};
use errors::EpochProcessingError as Error;
pub use justification_and_finalization_state::JustificationAndFinalizationState;
use safe_arith::SafeArith;
use tracing::instrument;
use types::{BeaconState, ChainSpec, EthSpec};

pub use registry_updates::{process_registry_updates, process_registry_updates_slow};
pub use slashings::{process_slashings, process_slashings_slow};
pub use weigh_justification_and_finalization::weigh_justification_and_finalization;

pub mod altair;
pub mod base;
pub mod capella;
pub mod effective_balance_updates;
pub mod epoch_processing_summary;
pub mod errors;
pub mod gloas;
pub mod historical_roots_update;
pub mod justification_and_finalization_state;
pub mod registry_updates;
pub mod resets;
pub mod single_pass;
pub mod slashings;
pub mod tests;
pub mod weigh_justification_and_finalization;

/// Performs per-epoch processing on some BeaconState.
///
/// Mutates the given `BeaconState`, returning early if an error is encountered. If an error is
/// returned, a state might be "half-processed" and therefore in an invalid state.
#[instrument(skip_all)]
pub fn process_epoch<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<EpochProcessingSummary<E>, Error> {
    let _timer = metrics::start_timer(&metrics::PROCESS_EPOCH_TIME);

    // Verify that the `BeaconState` instantiation matches the fork at `state.slot()`.
    state
        .fork_name(spec)
        .map_err(Error::InconsistentStateFork)?;

    if state.fork_name_unchecked().altair_enabled() {
        altair::process_epoch(state, spec)
    } else {
        base::process_epoch(state, spec)
    }
}

/// Used to track the changes to a validator's balance.
#[derive(Default, Clone)]
pub struct Delta {
    pub rewards: u64,
    pub penalties: u64,
}

impl Delta {
    /// Reward the validator with the `reward`.
    pub fn reward(&mut self, reward: u64) -> Result<(), Error> {
        self.rewards = self.rewards.safe_add(reward)?;
        Ok(())
    }

    /// Penalize the validator with the `penalty`.
    pub fn penalize(&mut self, penalty: u64) -> Result<(), Error> {
        self.penalties = self.penalties.safe_add(penalty)?;
        Ok(())
    }

    /// Combine two deltas.
    fn combine(&mut self, other: Delta) -> Result<(), Error> {
        self.reward(other.rewards)?;
        self.penalize(other.penalties)
    }
}

#[cfg(test)]
mod delta_tests {
    use super::*;

    #[test]
    fn default_is_zero() {
        let d = Delta::default();
        assert_eq!(d.rewards, 0);
        assert_eq!(d.penalties, 0);
    }

    #[test]
    fn reward_accumulates() {
        let mut d = Delta::default();
        d.reward(100).unwrap();
        d.reward(200).unwrap();
        assert_eq!(d.rewards, 300);
        assert_eq!(d.penalties, 0);
    }

    #[test]
    fn penalize_accumulates() {
        let mut d = Delta::default();
        d.penalize(50).unwrap();
        d.penalize(75).unwrap();
        assert_eq!(d.penalties, 125);
        assert_eq!(d.rewards, 0);
    }

    #[test]
    fn reward_and_penalize_independent() {
        let mut d = Delta::default();
        d.reward(100).unwrap();
        d.penalize(50).unwrap();
        assert_eq!(d.rewards, 100);
        assert_eq!(d.penalties, 50);
    }

    #[test]
    fn combine_merges_both() {
        let mut d1 = Delta::default();
        d1.reward(100).unwrap();
        d1.penalize(30).unwrap();

        let mut d2 = Delta::default();
        d2.reward(50).unwrap();
        d2.penalize(20).unwrap();

        d1.combine(d2).unwrap();
        assert_eq!(d1.rewards, 150);
        assert_eq!(d1.penalties, 50);
    }

    #[test]
    fn clone_is_independent() {
        let mut d = Delta::default();
        d.reward(100).unwrap();
        let mut d2 = d.clone();
        d2.reward(50).unwrap();
        assert_eq!(d.rewards, 100);
        assert_eq!(d2.rewards, 150);
    }

    #[test]
    fn reward_overflow_is_error() {
        let mut d = Delta::default();
        d.reward(u64::MAX).unwrap();
        assert!(d.reward(1).is_err());
    }

    #[test]
    fn penalize_overflow_is_error() {
        let mut d = Delta::default();
        d.penalize(u64::MAX).unwrap();
        assert!(d.penalize(1).is_err());
    }
}
