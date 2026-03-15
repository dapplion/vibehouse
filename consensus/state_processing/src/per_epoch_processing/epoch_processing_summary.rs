use super::base::{TotalBalances, ValidatorStatus, validator_statuses::InclusionInfo};
use crate::metrics;
use std::sync::Arc;
use types::{
    BeaconStateError, Epoch, EthSpec, List, ParticipationFlags, ProgressiveBalancesCache,
    SyncCommittee, Validator,
    consts::altair::{TIMELY_HEAD_FLAG_INDEX, TIMELY_SOURCE_FLAG_INDEX, TIMELY_TARGET_FLAG_INDEX},
};

/// Provides a summary of validator participation during the epoch.
#[derive(PartialEq, Debug)]
pub enum EpochProcessingSummary<E: EthSpec> {
    Base {
        total_balances: TotalBalances,
        statuses: Vec<ValidatorStatus>,
    },
    Altair {
        progressive_balances: ProgressiveBalancesCache,
        current_epoch_total_active_balance: u64,
        participation: Box<ParticipationEpochSummary<E>>,
        sync_committee: Arc<SyncCommittee<E>>,
    },
}

#[derive(PartialEq, Debug)]
pub struct ParticipationEpochSummary<E: EthSpec> {
    /// Copy of the validator registry prior to mutation.
    validators: List<Validator, E::ValidatorRegistryLimit>,
    /// Copy of the participation flags for the previous epoch.
    previous_epoch_participation: List<ParticipationFlags, E::ValidatorRegistryLimit>,
    /// Copy of the participation flags for the current epoch.
    current_epoch_participation: List<ParticipationFlags, E::ValidatorRegistryLimit>,
    previous_epoch: Epoch,
    current_epoch: Epoch,
}

impl<E: EthSpec> ParticipationEpochSummary<E> {
    pub fn new(
        validators: List<Validator, E::ValidatorRegistryLimit>,
        previous_epoch_participation: List<ParticipationFlags, E::ValidatorRegistryLimit>,
        current_epoch_participation: List<ParticipationFlags, E::ValidatorRegistryLimit>,
        previous_epoch: Epoch,
        current_epoch: Epoch,
    ) -> Self {
        Self {
            validators,
            previous_epoch_participation,
            current_epoch_participation,
            previous_epoch,
            current_epoch,
        }
    }

    pub fn is_active_and_unslashed(&self, val_index: usize, epoch: Epoch) -> bool {
        self.validators
            .get(val_index)
            .map(|validator| !validator.slashed && validator.is_active_at(epoch))
            .unwrap_or(false)
    }

    pub fn is_previous_epoch_unslashed_participating_index(
        &self,
        val_index: usize,
        flag_index: usize,
    ) -> Result<bool, BeaconStateError> {
        Ok(self.is_active_and_unslashed(val_index, self.previous_epoch)
            && self
                .previous_epoch_participation
                .get(val_index)
                .ok_or(BeaconStateError::UnknownValidator(val_index))?
                .has_flag(flag_index)?)
    }

    pub fn is_current_epoch_unslashed_participating_index(
        &self,
        val_index: usize,
        flag_index: usize,
    ) -> Result<bool, BeaconStateError> {
        Ok(self.is_active_and_unslashed(val_index, self.current_epoch)
            && self
                .current_epoch_participation
                .get(val_index)
                .ok_or(BeaconStateError::UnknownValidator(val_index))?
                .has_flag(flag_index)?)
    }
}

impl<E: EthSpec> EpochProcessingSummary<E> {
    /// Updates some Prometheus metrics with some values in `self`.
    pub fn observe_metrics(&self) -> Result<(), BeaconStateError> {
        metrics::set_gauge(
            &metrics::PARTICIPATION_PREV_EPOCH_HEAD_ATTESTING_GWEI_TOTAL,
            self.previous_epoch_head_attesting_balance()? as i64,
        );
        metrics::set_gauge(
            &metrics::PARTICIPATION_PREV_EPOCH_TARGET_ATTESTING_GWEI_TOTAL,
            self.previous_epoch_target_attesting_balance()? as i64,
        );
        metrics::set_gauge(
            &metrics::PARTICIPATION_PREV_EPOCH_SOURCE_ATTESTING_GWEI_TOTAL,
            self.previous_epoch_source_attesting_balance()? as i64,
        );
        metrics::set_gauge(
            &metrics::PARTICIPATION_CURRENT_EPOCH_TOTAL_ACTIVE_GWEI_TOTAL,
            self.current_epoch_total_active_balance() as i64,
        );

        Ok(())
    }

    /// Returns the sync committee indices for the current epoch for altair.
    pub fn sync_committee(&self) -> Option<&SyncCommittee<E>> {
        match self {
            EpochProcessingSummary::Altair { sync_committee, .. } => Some(sync_committee),
            EpochProcessingSummary::Base { .. } => None,
        }
    }

    /// Returns the sum of the effective balance of all validators in the current epoch.
    pub fn current_epoch_total_active_balance(&self) -> u64 {
        match self {
            EpochProcessingSummary::Base { total_balances, .. } => total_balances.current_epoch(),
            EpochProcessingSummary::Altair {
                current_epoch_total_active_balance,
                ..
            } => *current_epoch_total_active_balance,
        }
    }

    /// Returns the sum of the effective balance of all validators in the current epoch who
    /// included an attestation that matched the target.
    pub fn current_epoch_target_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        match self {
            EpochProcessingSummary::Base { total_balances, .. } => {
                Ok(total_balances.current_epoch_target_attesters())
            }
            EpochProcessingSummary::Altair {
                progressive_balances,
                ..
            } => progressive_balances.current_epoch_target_attesting_balance(),
        }
    }

    /// Returns `true` if `val_index` was included in the active validator indices in the current
    /// epoch *and* the validator is not slashed.
    ///
    /// ## Notes
    ///
    /// Always returns `false` for an unknown `val_index`.
    pub fn is_active_unslashed_in_current_epoch(&self, val_index: usize) -> bool {
        match self {
            EpochProcessingSummary::Base { statuses, .. } => statuses
                .get(val_index)
                .is_some_and(|s| s.is_active_in_current_epoch && !s.is_slashed),
            EpochProcessingSummary::Altair { participation, .. } => {
                participation.is_active_and_unslashed(val_index, participation.current_epoch)
            }
        }
    }

    /// Returns `true` if `val_index` had a target-matching attestation included on chain in the
    /// current epoch.
    ///
    /// ## Differences between Base and Altair
    ///
    /// - Base: active validators return `true`.
    /// - Altair: only active and *unslashed* validators return `true`.
    ///
    /// ## Notes
    ///
    /// Always returns `false` for an unknown `val_index`.
    pub fn is_current_epoch_target_attester(
        &self,
        val_index: usize,
    ) -> Result<bool, BeaconStateError> {
        match self {
            EpochProcessingSummary::Base { statuses, .. } => Ok(statuses
                .get(val_index)
                .is_some_and(|s| s.is_current_epoch_target_attester)),
            EpochProcessingSummary::Altair { participation, .. } => participation
                .is_current_epoch_unslashed_participating_index(
                    val_index,
                    TIMELY_TARGET_FLAG_INDEX,
                ),
        }
    }

    /// Returns the sum of the effective balance of all validators in the previous epoch who
    /// included an attestation that matched the target.
    pub fn previous_epoch_target_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        match self {
            EpochProcessingSummary::Base { total_balances, .. } => {
                Ok(total_balances.previous_epoch_target_attesters())
            }
            EpochProcessingSummary::Altair {
                progressive_balances,
                ..
            } => progressive_balances.previous_epoch_target_attesting_balance(),
        }
    }

    /// Returns the sum of the effective balance of all validators in the previous epoch who
    /// included an attestation that matched the head.
    ///
    /// ## Differences between Base and Altair
    ///
    /// - Base: any attestation can match the head.
    /// - Altair: only "timely" attestations can match the head.
    pub fn previous_epoch_head_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        match self {
            EpochProcessingSummary::Base { total_balances, .. } => {
                Ok(total_balances.previous_epoch_head_attesters())
            }
            EpochProcessingSummary::Altair {
                progressive_balances,
                ..
            } => progressive_balances.previous_epoch_head_attesting_balance(),
        }
    }

    /// Returns the sum of the effective balance of all validators in the previous epoch who
    /// included an attestation that matched the source.
    ///
    /// ## Differences between Base and Altair
    ///
    /// - Base: any attestation can match the source.
    /// - Altair: only "timely" attestations can match the source.
    pub fn previous_epoch_source_attesting_balance(&self) -> Result<u64, BeaconStateError> {
        match self {
            EpochProcessingSummary::Base { total_balances, .. } => {
                Ok(total_balances.previous_epoch_attesters())
            }
            EpochProcessingSummary::Altair {
                progressive_balances,
                ..
            } => progressive_balances.previous_epoch_source_attesting_balance(),
        }
    }

    /// Returns `true` if `val_index` was included in the active validator indices in the previous
    /// epoch *and* the validator is not slashed.
    ///
    /// ## Notes
    ///
    /// Always returns `false` for an unknown `val_index`.
    pub fn is_active_unslashed_in_previous_epoch(&self, val_index: usize) -> bool {
        match self {
            EpochProcessingSummary::Base { statuses, .. } => statuses
                .get(val_index)
                .is_some_and(|s| s.is_active_in_previous_epoch && !s.is_slashed),
            EpochProcessingSummary::Altair { participation, .. } => {
                participation.is_active_and_unslashed(val_index, participation.previous_epoch)
            }
        }
    }

    /// Returns `true` if `val_index` had a target-matching attestation included on chain in the
    /// previous epoch.
    ///
    /// ## Notes
    ///
    /// Always returns `false` for an unknown `val_index`.
    pub fn is_previous_epoch_target_attester(
        &self,
        val_index: usize,
    ) -> Result<bool, BeaconStateError> {
        match self {
            EpochProcessingSummary::Base { statuses, .. } => Ok(statuses
                .get(val_index)
                .is_some_and(|s| s.is_previous_epoch_target_attester)),
            EpochProcessingSummary::Altair { participation, .. } => participation
                .is_previous_epoch_unslashed_participating_index(
                    val_index,
                    TIMELY_TARGET_FLAG_INDEX,
                ),
        }
    }

    /// Returns `true` if `val_index` had a head-matching attestation included on chain in the
    /// previous epoch.
    ///
    /// ## Differences between Base and Altair
    ///
    /// - Base: any attestation can match the head.
    /// - Altair: only "timely" attestations can match the head.
    ///
    /// ## Notes
    ///
    /// Always returns `false` for an unknown `val_index`.
    pub fn is_previous_epoch_head_attester(
        &self,
        val_index: usize,
    ) -> Result<bool, BeaconStateError> {
        match self {
            EpochProcessingSummary::Base { statuses, .. } => Ok(statuses
                .get(val_index)
                .is_some_and(|s| s.is_previous_epoch_head_attester)),
            EpochProcessingSummary::Altair { participation, .. } => participation
                .is_previous_epoch_unslashed_participating_index(val_index, TIMELY_HEAD_FLAG_INDEX),
        }
    }

    /// Returns `true` if `val_index` had a source-matching attestation included on chain in the
    /// previous epoch.
    ///
    /// ## Differences between Base and Altair
    ///
    /// - Base: any attestation can match the head.
    /// - Altair: only "timely" attestations can match the source.
    ///
    /// ## Notes
    ///
    /// Always returns `false` for an unknown `val_index`.
    pub fn is_previous_epoch_source_attester(
        &self,
        val_index: usize,
    ) -> Result<bool, BeaconStateError> {
        match self {
            EpochProcessingSummary::Base { statuses, .. } => Ok(statuses
                .get(val_index)
                .is_some_and(|s| s.is_previous_epoch_attester)),
            EpochProcessingSummary::Altair { participation, .. } => participation
                .is_previous_epoch_unslashed_participating_index(
                    val_index,
                    TIMELY_SOURCE_FLAG_INDEX,
                ),
        }
    }

    /// Returns information about the inclusion distance for `val_index` for the previous epoch.
    ///
    /// ## Differences between Base and Altair
    ///
    /// - Base: always returns `Some` if the validator had an attestation included on-chain.
    /// - Altair: always returns `None`.
    ///
    /// ## Notes
    ///
    /// Always returns `false` for an unknown `val_index`.
    pub fn previous_epoch_inclusion_info(&self, val_index: usize) -> Option<InclusionInfo> {
        match self {
            EpochProcessingSummary::Base { statuses, .. } => {
                statuses.get(val_index).and_then(|s| s.inclusion_info)
            }
            EpochProcessingSummary::Altair { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::PublicKeyBytes;
    use types::{
        Epoch, Hash256, List, MinimalEthSpec, ParticipationFlags, Validator,
        consts::altair::{
            TIMELY_HEAD_FLAG_INDEX, TIMELY_SOURCE_FLAG_INDEX, TIMELY_TARGET_FLAG_INDEX,
        },
    };

    type E = MinimalEthSpec;

    /// Helper to create a validator active in [activation, exit).
    fn make_validator(activation: u64, exit: u64, slashed: bool) -> Validator {
        Validator {
            pubkey: PublicKeyBytes::empty(),
            withdrawal_credentials: Hash256::ZERO,
            effective_balance: 32_000_000_000,
            slashed,
            activation_eligibility_epoch: Epoch::new(0),
            activation_epoch: Epoch::new(activation),
            exit_epoch: Epoch::new(exit),
            withdrawable_epoch: Epoch::new(exit + 256),
        }
    }

    fn flags_with(indices: &[usize]) -> ParticipationFlags {
        let mut f = ParticipationFlags::default();
        for &i in indices {
            f.add_flag(i).unwrap();
        }
        f
    }

    fn make_summary(
        validators: Vec<Validator>,
        prev_participation: Vec<ParticipationFlags>,
        curr_participation: Vec<ParticipationFlags>,
        prev_epoch: u64,
        curr_epoch: u64,
    ) -> ParticipationEpochSummary<E> {
        ParticipationEpochSummary::new(
            List::new(validators).unwrap(),
            List::new(prev_participation).unwrap(),
            List::new(curr_participation).unwrap(),
            Epoch::new(prev_epoch),
            Epoch::new(curr_epoch),
        )
    }

    // --- is_active_and_unslashed ---

    #[test]
    fn active_unslashed_validator() {
        let summary = make_summary(
            vec![make_validator(0, 100, false)],
            vec![ParticipationFlags::default()],
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(summary.is_active_and_unslashed(0, Epoch::new(10)));
    }

    #[test]
    fn slashed_validator_returns_false() {
        let summary = make_summary(
            vec![make_validator(0, 100, true)],
            vec![ParticipationFlags::default()],
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(!summary.is_active_and_unslashed(0, Epoch::new(10)));
    }

    #[test]
    fn inactive_validator_returns_false() {
        // Validator activates at epoch 50, so not active at epoch 10
        let summary = make_summary(
            vec![make_validator(50, 100, false)],
            vec![ParticipationFlags::default()],
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(!summary.is_active_and_unslashed(0, Epoch::new(10)));
    }

    #[test]
    fn out_of_bounds_index_returns_false() {
        let summary = make_summary(
            vec![make_validator(0, 100, false)],
            vec![ParticipationFlags::default()],
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(!summary.is_active_and_unslashed(999, Epoch::new(10)));
    }

    // --- is_previous_epoch_unslashed_participating_index ---

    #[test]
    fn previous_epoch_participating_with_flag() {
        let summary = make_summary(
            vec![make_validator(0, 100, false)],
            vec![flags_with(&[TIMELY_TARGET_FLAG_INDEX])],
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(
            summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn previous_epoch_participating_without_flag() {
        let summary = make_summary(
            vec![make_validator(0, 100, false)],
            vec![flags_with(&[TIMELY_SOURCE_FLAG_INDEX])], // only source, not target
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(
            !summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn previous_epoch_slashed_not_participating() {
        let summary = make_summary(
            vec![make_validator(0, 100, true)], // slashed
            vec![flags_with(&[TIMELY_TARGET_FLAG_INDEX])],
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(
            !summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn previous_epoch_inactive_not_participating() {
        let summary = make_summary(
            vec![make_validator(50, 100, false)], // not active at epoch 9
            vec![flags_with(&[TIMELY_TARGET_FLAG_INDEX])],
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(
            !summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn unknown_validator_not_active() {
        // Empty validator list → is_active_and_unslashed returns false for any index
        let summary = make_summary(vec![], vec![], vec![], 9, 10);
        assert!(!summary.is_active_and_unslashed(0, Epoch::new(10)));
    }

    #[test]
    fn participation_missing_for_active_validator_errors() {
        // Validator exists and is active, but participation list is empty → Err
        let summary = make_summary(
            vec![make_validator(0, 100, false)],
            vec![], // no participation entry
            vec![],
            9,
            10,
        );
        assert!(
            summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .is_err()
        );
    }

    // --- is_current_epoch_unslashed_participating_index ---

    #[test]
    fn current_epoch_participating_with_flag() {
        let summary = make_summary(
            vec![make_validator(0, 100, false)],
            vec![ParticipationFlags::default()],
            vec![flags_with(&[TIMELY_HEAD_FLAG_INDEX])],
            9,
            10,
        );
        assert!(
            summary
                .is_current_epoch_unslashed_participating_index(0, TIMELY_HEAD_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn current_epoch_participating_wrong_flag() {
        let summary = make_summary(
            vec![make_validator(0, 100, false)],
            vec![ParticipationFlags::default()],
            vec![flags_with(&[TIMELY_HEAD_FLAG_INDEX])],
            9,
            10,
        );
        assert!(
            !summary
                .is_current_epoch_unslashed_participating_index(0, TIMELY_SOURCE_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn current_epoch_slashed_not_participating() {
        let summary = make_summary(
            vec![make_validator(0, 100, true)],
            vec![ParticipationFlags::default()],
            vec![flags_with(&[TIMELY_HEAD_FLAG_INDEX])],
            9,
            10,
        );
        assert!(
            !summary
                .is_current_epoch_unslashed_participating_index(0, TIMELY_HEAD_FLAG_INDEX)
                .unwrap()
        );
    }

    // --- Multiple validators ---

    #[test]
    fn multiple_validators_mixed() {
        let summary = make_summary(
            vec![
                make_validator(0, 100, false),  // active, unslashed
                make_validator(0, 100, true),   // active, slashed
                make_validator(50, 100, false), // inactive
            ],
            vec![
                flags_with(&[TIMELY_SOURCE_FLAG_INDEX, TIMELY_TARGET_FLAG_INDEX]),
                flags_with(&[TIMELY_SOURCE_FLAG_INDEX]),
                flags_with(&[TIMELY_TARGET_FLAG_INDEX]),
            ],
            vec![
                ParticipationFlags::default(),
                ParticipationFlags::default(),
                ParticipationFlags::default(),
            ],
            9,
            10,
        );

        // Validator 0: active, unslashed, has source+target → true for both
        assert!(
            summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_SOURCE_FLAG_INDEX)
                .unwrap()
        );
        assert!(
            summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );

        // Validator 1: slashed → false even though has source flag
        assert!(
            !summary
                .is_previous_epoch_unslashed_participating_index(1, TIMELY_SOURCE_FLAG_INDEX)
                .unwrap()
        );

        // Validator 2: inactive at epoch 9 → false even though has target flag
        assert!(
            !summary
                .is_previous_epoch_unslashed_participating_index(2, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn all_three_flags_independent() {
        let summary = make_summary(
            vec![make_validator(0, 100, false)],
            vec![flags_with(&[
                TIMELY_SOURCE_FLAG_INDEX,
                TIMELY_HEAD_FLAG_INDEX,
            ])],
            vec![ParticipationFlags::default()],
            9,
            10,
        );

        assert!(
            summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_SOURCE_FLAG_INDEX)
                .unwrap()
        );
        assert!(
            summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_HEAD_FLAG_INDEX)
                .unwrap()
        );
        assert!(
            !summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn exited_validator_not_active() {
        // Validator exited at epoch 5, checking at epoch 9
        let summary = make_summary(
            vec![make_validator(0, 5, false)],
            vec![flags_with(&[TIMELY_TARGET_FLAG_INDEX])],
            vec![ParticipationFlags::default()],
            9,
            10,
        );
        assert!(
            !summary
                .is_previous_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );
    }

    #[test]
    fn validator_active_at_boundary_epoch() {
        // Activation at epoch 10, exit at epoch 11 — active only at epoch 10
        let summary = make_summary(
            vec![make_validator(10, 11, false)],
            vec![ParticipationFlags::default()],
            vec![flags_with(&[TIMELY_TARGET_FLAG_INDEX])],
            9,
            10,
        );
        // Active at current epoch (10)
        assert!(
            summary
                .is_current_epoch_unslashed_participating_index(0, TIMELY_TARGET_FLAG_INDEX)
                .unwrap()
        );
        // Not active at previous epoch (9)
        assert!(!summary.is_active_and_unslashed(0, Epoch::new(9)));
    }
}
