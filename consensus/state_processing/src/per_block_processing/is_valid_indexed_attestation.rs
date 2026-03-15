use super::errors::{BlockOperationError, IndexedAttestationInvalid as Invalid};
use super::signature_sets::{get_pubkey_from_state, indexed_attestation_signature_set};
use crate::VerifySignatures;
use itertools::Itertools;
use types::*;

type Result<T> = std::result::Result<T, BlockOperationError<Invalid>>;

fn error(reason: Invalid) -> BlockOperationError<Invalid> {
    BlockOperationError::invalid(reason)
}

/// Verify an `IndexedAttestation`.
pub fn is_valid_indexed_attestation<E: EthSpec>(
    state: &BeaconState<E>,
    indexed_attestation: IndexedAttestationRef<E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<()> {
    // Verify that indices aren't empty
    verify!(
        !indexed_attestation.attesting_indices_is_empty(),
        Invalid::IndicesEmpty
    );

    // Check that indices are sorted and unique (using iterator, no Vec allocation)
    indexed_attestation
        .attesting_indices_iter()
        .tuple_windows()
        .enumerate()
        .try_for_each(|(i, (x, y))| {
            if x < y {
                Ok(())
            } else {
                Err(error(Invalid::BadValidatorIndicesOrdering(i)))
            }
        })?;

    if verify_signatures.is_true() {
        verify!(
            indexed_attestation_signature_set(
                state,
                |i| get_pubkey_from_state(state, i),
                indexed_attestation.signature(),
                indexed_attestation,
                spec
            )?
            .verify(),
            Invalid::BadSignature
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{
        AggregateSignature, AttestationData, Checkpoint, ForkName, IndexedAttestationElectra,
        MinimalEthSpec,
    };

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    /// Minimal state — we only need it for signature verification (which we skip).
    fn make_state(spec: &ChainSpec) -> BeaconState<E> {
        use bls::PublicKeyBytes;
        use std::sync::Arc;
        use types::{
            BeaconBlockHeader, BeaconStateGloas, BitVector, BuilderPendingPayment, Checkpoint,
            EpochCache, Eth1Data, ExecutionBlockHash, ExecutionPayloadBid, ExitCache, Fork,
            ProgressiveBalancesCache, PubkeyCache, SlashingsCache, SyncCommittee, Validator,
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
            validators: List::new(vec![
                Validator {
                    pubkey: PublicKeyBytes::empty(),
                    withdrawal_credentials: Hash256::zero(),
                    effective_balance: spec.max_effective_balance,
                    slashed: false,
                    activation_eligibility_epoch: Epoch::new(0),
                    activation_epoch: Epoch::new(0),
                    exit_epoch: spec.far_future_epoch,
                    withdrawable_epoch: spec.far_future_epoch,
                };
                4
            ])
            .unwrap(),
            balances: List::new(vec![spec.max_effective_balance; 4]).unwrap(),
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

    fn make_att_data() -> AttestationData {
        AttestationData {
            slot: Slot::new(0),
            index: 0,
            beacon_block_root: Hash256::zero(),
            source: Checkpoint {
                epoch: Epoch::new(2),
                root: Hash256::zero(),
            },
            target: Checkpoint {
                epoch: Epoch::new(5),
                root: Hash256::zero(),
            },
        }
    }

    fn make_indexed_att(indices: Vec<u64>) -> IndexedAttestation<E> {
        IndexedAttestation::Electra(IndexedAttestationElectra {
            attesting_indices: VariableList::new(indices).unwrap(),
            data: make_att_data(),
            signature: AggregateSignature::empty(),
        })
    }

    #[test]
    fn valid_sorted_indices() {
        let spec = make_spec();
        let state = make_state(&spec);
        let att = make_indexed_att(vec![0, 1, 2]);
        let result =
            is_valid_indexed_attestation(&state, att.to_ref(), VerifySignatures::False, &spec);
        assert!(result.is_ok());
    }

    #[test]
    fn single_index_valid() {
        let spec = make_spec();
        let state = make_state(&spec);
        let att = make_indexed_att(vec![3]);
        let result =
            is_valid_indexed_attestation(&state, att.to_ref(), VerifySignatures::False, &spec);
        assert!(result.is_ok());
    }

    #[test]
    fn empty_indices_returns_error() {
        let spec = make_spec();
        let state = make_state(&spec);
        let att = make_indexed_att(vec![]);
        let err =
            is_valid_indexed_attestation(&state, att.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::IndicesEmpty)
        ));
    }

    #[test]
    fn unsorted_indices_returns_error() {
        let spec = make_spec();
        let state = make_state(&spec);
        let att = make_indexed_att(vec![2, 0, 1]);
        let err =
            is_valid_indexed_attestation(&state, att.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        assert!(
            matches!(
                err,
                BlockOperationError::Invalid(Invalid::BadValidatorIndicesOrdering(0))
            ),
            "expected BadValidatorIndicesOrdering(0) for first out-of-order pair, got {:?}",
            err
        );
    }

    #[test]
    fn duplicate_indices_returns_error() {
        let spec = make_spec();
        let state = make_state(&spec);
        let att = make_indexed_att(vec![0, 1, 1, 2]);
        let err =
            is_valid_indexed_attestation(&state, att.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        // Duplicate at position 1 (indices[1]=1 is not < indices[2]=1)
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::BadValidatorIndicesOrdering(1))
        ));
    }

    #[test]
    fn ordering_error_reports_correct_position() {
        let spec = make_spec();
        let state = make_state(&spec);
        // Sorted until the last pair: [0, 1, 3, 2]
        let att = make_indexed_att(vec![0, 1, 3, 2]);
        let err =
            is_valid_indexed_attestation(&state, att.to_ref(), VerifySignatures::False, &spec)
                .unwrap_err();
        // The bad pair is at window index 2 (between indices[2]=3 and indices[3]=2)
        assert!(matches!(
            err,
            BlockOperationError::Invalid(Invalid::BadValidatorIndicesOrdering(2))
        ));
    }
}
