use super::errors::{AttesterSlashingInvalid as Invalid, BlockOperationError};
use super::is_valid_indexed_attestation::is_valid_indexed_attestation;
use crate::per_block_processing::VerifySignatures;
use std::collections::BTreeSet;
use types::*;

type Result<T> = std::result::Result<T, BlockOperationError<Invalid>>;

fn error(reason: Invalid) -> BlockOperationError<Invalid> {
    BlockOperationError::invalid(reason)
}

/// Indicates if an `AttesterSlashing` is valid to be included in a block in the current epoch of
/// the given state.
///
/// Returns `Ok(indices)` with `indices` being a non-empty vec of validator indices in ascending
/// order if the `AttesterSlashing` is valid. Otherwise returns `Err(e)` with the reason for
/// invalidity.
pub fn verify_attester_slashing<E: EthSpec>(
    state: &BeaconState<E>,
    attester_slashing: AttesterSlashingRef<'_, E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<Vec<u64>> {
    let attestation_1 = attester_slashing.attestation_1();
    let attestation_2 = attester_slashing.attestation_2();

    // Spec: is_slashable_attestation_data
    verify!(
        attestation_1.is_double_vote(attestation_2)
            || attestation_1.is_surround_vote(attestation_2),
        Invalid::NotSlashable
    );

    is_valid_indexed_attestation(state, attestation_1, verify_signatures, spec)
        .map_err(|e| error(Invalid::IndexedAttestation1Invalid(e)))?;
    is_valid_indexed_attestation(state, attestation_2, verify_signatures, spec)
        .map_err(|e| error(Invalid::IndexedAttestation2Invalid(e)))?;

    get_slashable_indices(state, attester_slashing)
}

/// For a given attester slashing, return the indices able to be slashed in ascending order.
///
/// Returns Ok(indices) if `indices.len() > 0`
pub fn get_slashable_indices<E: EthSpec>(
    state: &BeaconState<E>,
    attester_slashing: AttesterSlashingRef<'_, E>,
) -> Result<Vec<u64>> {
    get_slashable_indices_modular(state, attester_slashing, |_, validator| {
        validator.is_slashable_at(state.current_epoch())
    })
}

/// Same as `gather_attester_slashing_indices` but allows the caller to specify the criteria
/// for determining whether a given validator should be considered slashable.
pub fn get_slashable_indices_modular<F, E: EthSpec>(
    state: &BeaconState<E>,
    attester_slashing: AttesterSlashingRef<'_, E>,
    is_slashable: F,
) -> Result<Vec<u64>>
where
    F: Fn(u64, &Validator) -> bool,
{
    let attestation_1 = attester_slashing.attestation_1();
    let attestation_2 = attester_slashing.attestation_2();

    let attesting_indices_1 = attestation_1
        .attesting_indices_iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    let attesting_indices_2 = attestation_2
        .attesting_indices_iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut slashable_indices = vec![];

    for index in &attesting_indices_1 & &attesting_indices_2 {
        let validator = state
            .validators()
            .get(index as usize)
            .ok_or_else(|| error(Invalid::UnknownValidator(index)))?;

        if is_slashable(index, validator) {
            slashable_indices.push(index);
        }
    }

    verify!(!slashable_indices.is_empty(), Invalid::NoSlashableIndices);

    Ok(slashable_indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::per_block_processing::VerifySignatures;
    use types::{
        AggregateSignature, AttestationData, AttesterSlashingElectra, Checkpoint, ForkName,
        IndexedAttestationElectra, MinimalEthSpec,
    };

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    /// Create a minimal Gloas state with `num_validators` active validators.
    fn make_state(num_validators: usize, spec: &ChainSpec) -> BeaconState<E> {
        // Reuse the verify_exit helper pattern — build a minimal Gloas state
        use bls::PublicKeyBytes;
        use std::sync::Arc;
        use types::{
            BeaconBlockHeader, BeaconStateGloas, BitVector, BuilderPendingPayment, Checkpoint,
            EpochCache, Eth1Data, ExecutionBlockHash, ExecutionPayloadBid, ExitCache, Fork,
            ProgressiveBalancesCache, PubkeyCache, SlashingsCache, SyncCommittee,
            beacon_state::BuilderPubkeyCache,
        };

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();
        let current_epoch = Epoch::new(10);
        let slot = current_epoch.start_slot(E::slots_per_epoch());

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        let mut validators = Vec::with_capacity(num_validators);
        let mut balances = Vec::with_capacity(num_validators);
        for _ in 0..num_validators {
            validators.push(Validator {
                pubkey: PublicKeyBytes::empty(),
                withdrawal_credentials: Hash256::zero(),
                effective_balance: spec.max_effective_balance,
                slashed: false,
                activation_eligibility_epoch: Epoch::new(0),
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
            });
            balances.push(spec.max_effective_balance);
        }

        BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch: current_epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root: Hash256::zero(),
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint::default(),
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid::default(),
            next_withdrawal_index: 0,
            next_withdrawal_validator_index: 0,
            historical_summaries: List::default(),
            deposit_requests_start_index: u64::MAX,
            deposit_balance_to_consume: 0,
            exit_balance_to_consume: 0,
            earliest_exit_epoch: Epoch::new(0),
            consolidation_balance_to_consume: 0,
            earliest_consolidation_epoch: Epoch::new(0),
            pending_deposits: List::default(),
            pending_partial_withdrawals: List::default(),
            pending_consolidations: List::default(),
            proposer_lookahead: Vector::new(vec![
                0u64;
                <E as EthSpec>::ProposerLookaheadSlots::to_usize()
            ])
            .unwrap(),
            builders: List::default(),
            next_withdrawal_builder_index: 0,
            execution_payload_availability: BitVector::from_bytes(
                vec![0xFFu8; slots_per_hist / 8].into(),
            )
            .unwrap(),
            builder_pending_payments: Vector::new(vec![
                BuilderPendingPayment::default();
                E::builder_pending_payments_limit()
            ])
            .unwrap(),
            builder_pending_withdrawals: List::default(),
            latest_block_hash: ExecutionBlockHash::zero(),
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <_>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: EpochCache::default(),
        })
    }

    /// Build an Electra-style indexed attestation with the given attesting indices and data.
    fn make_indexed_attestation(
        indices: Vec<u64>,
        data: AttestationData,
    ) -> IndexedAttestationElectra<E> {
        IndexedAttestationElectra {
            attesting_indices: VariableList::new(indices).unwrap(),
            data,
            signature: AggregateSignature::empty(),
        }
    }

    /// Build attestation data with specified source/target epochs.
    fn make_att_data(source_epoch: u64, target_epoch: u64) -> AttestationData {
        AttestationData {
            slot: Slot::new(0),
            index: 0,
            beacon_block_root: Hash256::zero(),
            source: Checkpoint {
                epoch: Epoch::new(source_epoch),
                root: Hash256::zero(),
            },
            target: Checkpoint {
                epoch: Epoch::new(target_epoch),
                root: Hash256::zero(),
            },
        }
    }

    /// Build a double-vote attester slashing (same target epoch, different data).
    fn make_double_vote_slashing(indices_1: Vec<u64>, indices_2: Vec<u64>) -> AttesterSlashing<E> {
        let data_1 = make_att_data(2, 5);
        let mut data_2 = make_att_data(2, 5);
        // Different beacon_block_root makes them different attestation data
        data_2.beacon_block_root = Hash256::repeat_byte(0x01);

        let att_1 = make_indexed_attestation(indices_1, data_1);
        let att_2 = make_indexed_attestation(indices_2, data_2);

        AttesterSlashing::Electra(AttesterSlashingElectra {
            attestation_1: att_1,
            attestation_2: att_2,
        })
    }

    // --- get_slashable_indices tests ---

    #[test]
    fn double_vote_returns_overlapping_indices() {
        let spec = make_spec();
        let state = make_state(8, &spec);
        let slashing = make_double_vote_slashing(vec![0, 1, 2, 3], vec![1, 2, 4]);
        let indices = get_slashable_indices(&state, slashing.to_ref()).unwrap();
        // Intersection of {0,1,2,3} and {1,2,4} = {1,2}
        assert_eq!(indices, vec![1, 2]);
    }

    #[test]
    fn slashable_indices_in_ascending_order() {
        let spec = make_spec();
        let state = make_state(8, &spec);
        let slashing = make_double_vote_slashing(vec![0, 3, 5, 7], vec![3, 5, 7]);
        let indices = get_slashable_indices(&state, slashing.to_ref()).unwrap();
        assert_eq!(indices, vec![3, 5, 7]);
        // Verify sorted
        for window in indices.windows(2) {
            assert!(window[0] < window[1]);
        }
    }

    #[test]
    fn no_overlapping_indices_returns_error() {
        let spec = make_spec();
        let state = make_state(8, &spec);
        let slashing = make_double_vote_slashing(vec![0, 1], vec![2, 3]);
        let err = get_slashable_indices(&state, slashing.to_ref()).unwrap_err();
        assert!(
            matches!(
                err,
                BlockOperationError::Invalid(Invalid::NoSlashableIndices)
            ),
            "expected NoSlashableIndices, got {:?}",
            err
        );
    }

    #[test]
    fn already_slashed_validator_excluded() {
        let spec = make_spec();
        let mut state = make_state(4, &spec);
        // Slash validator 1 — they're no longer slashable
        state.validators_mut().get_mut(1).unwrap().slashed = true;
        let slashing = make_double_vote_slashing(vec![0, 1, 2], vec![1, 2]);
        let indices = get_slashable_indices(&state, slashing.to_ref()).unwrap();
        // Validator 1 is already slashed, so only 2 remains
        assert_eq!(indices, vec![2]);
    }

    #[test]
    fn all_overlapping_already_slashed_returns_error() {
        let spec = make_spec();
        let mut state = make_state(4, &spec);
        state.validators_mut().get_mut(1).unwrap().slashed = true;
        state.validators_mut().get_mut(2).unwrap().slashed = true;
        let slashing = make_double_vote_slashing(vec![0, 1, 2], vec![1, 2]);
        let err = get_slashable_indices(&state, slashing.to_ref()).unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::NoSlashableIndices)
        ));
    }

    #[test]
    fn unknown_validator_in_intersection_returns_error() {
        let spec = make_spec();
        let state = make_state(4, &spec); // indices 0..3
        // Index 99 doesn't exist in state
        let slashing = make_double_vote_slashing(vec![0, 99], vec![99]);
        let err = get_slashable_indices(&state, slashing.to_ref()).unwrap_err();
        assert!(
            matches!(
                err,
                BlockOperationError::Invalid(Invalid::UnknownValidator(99))
            ),
            "expected UnknownValidator(99), got {:?}",
            err
        );
    }

    #[test]
    fn exited_validator_not_slashable() {
        let spec = make_spec();
        let mut state = make_state(4, &spec);
        // Validator 1 has exited and is withdrawable — not slashable
        let current_epoch = state.current_epoch();
        state.validators_mut().get_mut(1).unwrap().exit_epoch = current_epoch - 2;
        state
            .validators_mut()
            .get_mut(1)
            .unwrap()
            .withdrawable_epoch = current_epoch - 1;
        let slashing = make_double_vote_slashing(vec![0, 1, 2], vec![1, 2]);
        let indices = get_slashable_indices(&state, slashing.to_ref()).unwrap();
        // Only validator 2 (not 1, who is already withdrawable)
        assert_eq!(indices, vec![2]);
    }

    // --- get_slashable_indices_modular tests ---

    #[test]
    fn modular_custom_predicate() {
        let spec = make_spec();
        let state = make_state(8, &spec);
        let slashing = make_double_vote_slashing(vec![0, 1, 2, 3], vec![1, 2, 3]);
        // Only consider even indices slashable
        let indices =
            get_slashable_indices_modular(&state, slashing.to_ref(), |idx, _| idx % 2 == 0)
                .unwrap();
        assert_eq!(indices, vec![2]);
    }

    #[test]
    fn modular_all_rejected_returns_error() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        let slashing = make_double_vote_slashing(vec![0, 1], vec![0, 1]);
        let err =
            get_slashable_indices_modular(&state, slashing.to_ref(), |_, _| false).unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::NoSlashableIndices)
        ));
    }

    // --- verify_attester_slashing tests ---

    #[test]
    fn valid_double_vote_slashing() {
        let spec = make_spec();
        let state = make_state(8, &spec);
        let slashing = make_double_vote_slashing(vec![0, 1, 2], vec![1, 2, 3]);
        let result =
            verify_attester_slashing(&state, slashing.to_ref(), VerifySignatures::False, &spec);
        assert_eq!(result.unwrap(), vec![1, 2]);
    }

    #[test]
    fn valid_surround_vote_slashing() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        // Surround vote: att1 source < att2 source AND att2 target < att1 target
        // att1: source=1, target=8  (wider)
        // att2: source=3, target=5  (narrower, surrounded)
        let data_1 = make_att_data(1, 8);
        let data_2 = make_att_data(3, 5);
        let att_1 = make_indexed_attestation(vec![0, 1], data_1);
        let att_2 = make_indexed_attestation(vec![0, 1], data_2);
        let slashing = AttesterSlashing::Electra(AttesterSlashingElectra {
            attestation_1: att_1,
            attestation_2: att_2,
        });
        let result =
            verify_attester_slashing(&state, slashing.to_ref(), VerifySignatures::False, &spec);
        assert_eq!(result.unwrap(), vec![0, 1]);
    }

    #[test]
    fn not_slashable_same_data() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        // Same attestation data — not a double vote, not a surround vote
        let data = make_att_data(2, 5);
        let att_1 = make_indexed_attestation(vec![0, 1], data);
        let att_2 = make_indexed_attestation(vec![0, 1], data);
        let slashing = AttesterSlashing::Electra(AttesterSlashingElectra {
            attestation_1: att_1,
            attestation_2: att_2,
        });
        let err =
            verify_attester_slashing(&state, slashing.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        assert!(
            matches!(err, BlockOperationError::Invalid(Invalid::NotSlashable)),
            "expected NotSlashable, got {:?}",
            err
        );
    }

    #[test]
    fn not_slashable_different_target_epochs() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        // Different target epochs, no surround (att1 doesn't surround att2 or vice versa)
        let data_1 = make_att_data(2, 5);
        let data_2 = make_att_data(3, 7);
        let att_1 = make_indexed_attestation(vec![0, 1], data_1);
        let att_2 = make_indexed_attestation(vec![0, 1], data_2);
        let slashing = AttesterSlashing::Electra(AttesterSlashingElectra {
            attestation_1: att_1,
            attestation_2: att_2,
        });
        let err =
            verify_attester_slashing(&state, slashing.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::NotSlashable)
        ));
    }

    #[test]
    fn indexed_attestation_1_empty_indices() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        let data_1 = make_att_data(2, 5);
        let mut data_2 = make_att_data(2, 5);
        data_2.beacon_block_root = Hash256::repeat_byte(0x01);
        // Empty attesting indices in attestation 1
        let att_1 = make_indexed_attestation(vec![], data_1);
        let att_2 = make_indexed_attestation(vec![0, 1], data_2);
        let slashing = AttesterSlashing::Electra(AttesterSlashingElectra {
            attestation_1: att_1,
            attestation_2: att_2,
        });
        let err =
            verify_attester_slashing(&state, slashing.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        // Empty indices makes indexed attestation 1 invalid
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::IndexedAttestation1Invalid(_))
        ));
    }

    #[test]
    fn indexed_attestation_2_unsorted_indices() {
        let spec = make_spec();
        let state = make_state(4, &spec);
        let data_1 = make_att_data(2, 5);
        let mut data_2 = make_att_data(2, 5);
        data_2.beacon_block_root = Hash256::repeat_byte(0x01);
        let att_1 = make_indexed_attestation(vec![0, 1], data_1);
        // Unsorted indices in attestation 2
        let att_2 = make_indexed_attestation(vec![2, 0], data_2);
        let slashing = AttesterSlashing::Electra(AttesterSlashingElectra {
            attestation_1: att_1,
            attestation_2: att_2,
        });
        let err =
            verify_attester_slashing(&state, slashing.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::IndexedAttestation2Invalid(_))
        ));
    }

    #[test]
    fn double_vote_all_slashed_returns_no_slashable_indices() {
        let spec = make_spec();
        let mut state = make_state(4, &spec);
        // All overlapping validators already slashed
        state.validators_mut().get_mut(0).unwrap().slashed = true;
        state.validators_mut().get_mut(1).unwrap().slashed = true;
        let slashing = make_double_vote_slashing(vec![0, 1], vec![0, 1]);
        let err =
            verify_attester_slashing(&state, slashing.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::NoSlashableIndices)
        ));
    }
}
