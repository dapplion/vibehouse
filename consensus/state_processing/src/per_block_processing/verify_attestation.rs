use super::VerifySignatures;
use super::errors::{AttestationInvalid as Invalid, BlockOperationError};
use crate::ConsensusContext;
use crate::per_block_processing::is_valid_indexed_attestation;
use safe_arith::SafeArith;
use types::*;

type Result<T> = std::result::Result<T, BlockOperationError<Invalid>>;

fn error(reason: Invalid) -> BlockOperationError<Invalid> {
    BlockOperationError::invalid(reason)
}

/// Returns `Ok(())` if the given `attestation` is valid to be included in a block that is applied
/// to `state`. Otherwise, returns a descriptive `Err`.
///
/// Optionally verifies the aggregate signature, depending on `verify_signatures`.
pub fn verify_attestation_for_block_inclusion<'ctxt, E: EthSpec>(
    state: &BeaconState<E>,
    attestation: AttestationRef<'ctxt, E>,
    ctxt: &'ctxt mut ConsensusContext<E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<IndexedAttestationRef<'ctxt, E>> {
    let data = attestation.data();

    verify!(
        data.slot.safe_add(spec.min_attestation_inclusion_delay)? <= state.slot(),
        Invalid::IncludedTooEarly {
            state: state.slot(),
            delay: spec.min_attestation_inclusion_delay,
            attestation: data.slot,
        }
    );
    if state.fork_name_unchecked().deneb_enabled() {
        // [Modified in Deneb:EIP7045]
    } else {
        verify!(
            state.slot() <= data.slot.safe_add(E::slots_per_epoch())?,
            Invalid::IncludedTooLate {
                state: state.slot(),
                attestation: data.slot,
            }
        );
    }

    verify_attestation_for_state(state, attestation, ctxt, verify_signatures, spec)
}

/// Returns `Ok(())` if `attestation` is a valid attestation to the chain that precedes the given
/// `state`.
///
/// Returns a descriptive `Err` if the attestation is malformed or does not accurately reflect the
/// prior blocks in `state`.
///
/// Spec v0.12.1
pub fn verify_attestation_for_state<'ctxt, E: EthSpec>(
    state: &BeaconState<E>,
    attestation: AttestationRef<'ctxt, E>,
    ctxt: &'ctxt mut ConsensusContext<E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<IndexedAttestationRef<'ctxt, E>> {
    let data = attestation.data();

    // NOTE: choosing a validation based on the attestation's fork
    // rather than the state's fork makes this simple, but technically the spec
    // defines this verification based on the state's fork.
    match attestation {
        AttestationRef::Base(_) => {
            verify!(
                data.index < state.get_committee_count_at_slot(data.slot)?,
                Invalid::BadCommitteeIndex
            );
        }
        AttestationRef::Electra(_) => {
            // [Modified in Gloas:EIP7732] data.index < 2 (0 or 1)
            if state.fork_name_unchecked().gloas_enabled() {
                verify!(data.index < 2, Invalid::BadCommitteeIndex);
            } else {
                verify!(data.index == 0, Invalid::BadCommitteeIndex);
            }
        }
    }

    // Verify the Casper FFG vote.
    verify_casper_ffg_vote(attestation, state)?;

    // Check signature and bitfields
    let indexed_attestation = ctxt.get_indexed_attestation(state, attestation)?;
    is_valid_indexed_attestation(state, indexed_attestation, verify_signatures, spec)?;

    Ok(indexed_attestation)
}

/// Check target epoch and source checkpoint.
///
/// Spec v0.12.1
fn verify_casper_ffg_vote<E: EthSpec>(
    attestation: AttestationRef<E>,
    state: &BeaconState<E>,
) -> Result<()> {
    let data = attestation.data();
    verify!(
        data.target.epoch == data.slot.epoch(E::slots_per_epoch()),
        Invalid::TargetEpochSlotMismatch {
            target_epoch: data.target.epoch,
            slot_epoch: data.slot.epoch(E::slots_per_epoch()),
        }
    );
    if data.target.epoch == state.current_epoch() {
        verify!(
            data.source == state.current_justified_checkpoint(),
            Invalid::WrongJustifiedCheckpoint {
                state: Box::new(state.current_justified_checkpoint()),
                attestation: Box::new(data.source),
                is_current: true,
            }
        );
        Ok(())
    } else if data.target.epoch == state.previous_epoch() {
        verify!(
            data.source == state.previous_justified_checkpoint(),
            Invalid::WrongJustifiedCheckpoint {
                state: Box::new(state.previous_justified_checkpoint()),
                attestation: Box::new(data.source),
                is_current: false,
            }
        );
        Ok(())
    } else {
        Err(error(Invalid::BadTargetEpoch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::per_block_processing::errors::AttestationInvalid;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::{
        BeaconBlockHeader, BeaconStateFulu, BeaconStateGloas, BuilderPendingPayment, CACHED_EPOCHS,
        Checkpoint, CommitteeCache, Epoch, ExecutionBlockHash, ExecutionPayloadBid,
        ExecutionPayloadHeaderFulu, ExitCache, FixedVector, Fork, Hash256, List, MinimalEthSpec,
        ProgressiveBalancesCache, PubkeyCache, PublicKeyBytes, SlashingsCache, SyncCommittee,
        Unsigned, Vector,
    };

    type E = MinimalEthSpec;

    /// Build a minimal Gloas state at the given slot.
    fn make_gloas_state(slot_num: u64) -> (BeaconState<E>, ChainSpec) {
        let spec = gloas_spec();
        let slot = Slot::new(slot_num);
        let epoch = slot.epoch(E::slots_per_epoch());

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        let source_checkpoint = Checkpoint {
            epoch: epoch.saturating_sub(1u64),
            root: Hash256::repeat_byte(0xCC),
        };

        let state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch: Epoch::new(0),
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
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::default(),
            balances: List::default(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: source_checkpoint,
            current_justified_checkpoint: source_checkpoint,
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
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec)
    }

    /// Build a minimal Fulu (pre-Gloas) state at the given slot.
    fn make_fulu_state(slot_num: u64) -> (BeaconState<E>, ChainSpec) {
        let mut spec = E::default_spec();
        spec.altair_fork_epoch = Some(Epoch::new(0));
        spec.bellatrix_fork_epoch = Some(Epoch::new(0));
        spec.capella_fork_epoch = Some(Epoch::new(0));
        spec.deneb_fork_epoch = Some(Epoch::new(0));
        spec.electra_fork_epoch = Some(Epoch::new(0));
        spec.fulu_fork_epoch = Some(Epoch::new(0));
        // gloas NOT set — this is a Fulu state

        let slot = Slot::new(slot_num);
        let epoch = slot.epoch(E::slots_per_epoch());

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        let source_checkpoint = Checkpoint {
            epoch: epoch.saturating_sub(1u64),
            root: Hash256::repeat_byte(0xCC),
        };

        let state = BeaconState::Fulu(BeaconStateFulu {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.electra_fork_version,
                current_version: spec.fulu_fork_version,
                epoch: Epoch::new(0),
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
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::default(),
            balances: List::default(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: source_checkpoint,
            current_justified_checkpoint: source_checkpoint,
            finalized_checkpoint: Checkpoint::default(),
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_header: ExecutionPayloadHeaderFulu::default(),
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
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec)
    }

    fn gloas_spec() -> ChainSpec {
        let mut spec = E::default_spec();
        spec.altair_fork_epoch = Some(Epoch::new(0));
        spec.bellatrix_fork_epoch = Some(Epoch::new(0));
        spec.capella_fork_epoch = Some(Epoch::new(0));
        spec.deneb_fork_epoch = Some(Epoch::new(0));
        spec.electra_fork_epoch = Some(Epoch::new(0));
        spec.fulu_fork_epoch = Some(Epoch::new(0));
        spec.gloas_fork_epoch = Some(Epoch::new(0));
        spec
    }

    /// Create an Electra attestation with the specified index.
    fn make_electra_attestation(att_slot: u64, index: u64) -> Attestation<E> {
        Attestation::Electra(types::AttestationElectra {
            aggregation_bits: ssz_types::BitList::with_capacity(1).unwrap(),
            data: AttestationData {
                slot: Slot::new(att_slot),
                index,
                beacon_block_root: Hash256::zero(),
                source: Checkpoint::default(),
                target: Checkpoint {
                    epoch: Slot::new(att_slot).epoch(E::slots_per_epoch()),
                    root: Hash256::zero(),
                },
            },
            signature: types::AggregateSignature::empty(),
            committee_bits: BitVector::new(),
        })
    }

    /// Extract the Err from a Result where Ok doesn't implement Debug.
    fn extract_err<T>(
        result: std::result::Result<T, BlockOperationError<AttestationInvalid>>,
    ) -> BlockOperationError<AttestationInvalid> {
        match result {
            Ok(_) => panic!("expected Err, got Ok"),
            Err(e) => e,
        }
    }

    /// Helper: check whether an error is BadCommitteeIndex.
    fn is_bad_committee_index(err: &BlockOperationError<AttestationInvalid>) -> bool {
        matches!(
            err,
            BlockOperationError::Invalid(AttestationInvalid::BadCommitteeIndex)
        )
    }

    // ── Gloas committee index rejection tests ─────────────────

    #[test]
    fn gloas_index_2_rejected() {
        let (state, spec) = make_gloas_state(9);
        let attestation = make_electra_attestation(8, 2);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_state(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            is_bad_committee_index(&err),
            "expected BadCommitteeIndex, got {err:?}"
        );
    }

    #[test]
    fn gloas_index_3_rejected() {
        let (state, spec) = make_gloas_state(9);
        let attestation = make_electra_attestation(8, 3);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_state(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            is_bad_committee_index(&err),
            "expected BadCommitteeIndex, got {err:?}"
        );
    }

    #[test]
    fn gloas_index_u64_max_rejected() {
        let (state, spec) = make_gloas_state(9);
        let attestation = make_electra_attestation(8, u64::MAX);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_state(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            is_bad_committee_index(&err),
            "expected BadCommitteeIndex, got {err:?}"
        );
    }

    // ── Gloas committee index acceptance tests ────────────────
    // These pass the index check but will fail later (FFG vote or committee).
    // We verify that the error is NOT BadCommitteeIndex.

    #[test]
    fn gloas_index_0_passes_index_check() {
        let (state, spec) = make_gloas_state(9);
        let attestation = make_electra_attestation(8, 0);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_state(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            !is_bad_committee_index(&err),
            "index 0 should pass the committee index check in Gloas, got BadCommitteeIndex"
        );
    }

    #[test]
    fn gloas_index_1_passes_index_check() {
        // This is the NEW behavior in Gloas: index 1 is allowed (was rejected in Electra/Fulu)
        let (state, spec) = make_gloas_state(9);
        let attestation = make_electra_attestation(8, 1);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_state(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            !is_bad_committee_index(&err),
            "index 1 should pass the committee index check in Gloas, got BadCommitteeIndex"
        );
    }

    // ── Fulu (pre-Gloas) committee index tests ───────────────

    #[test]
    fn fulu_index_0_passes_index_check() {
        let (state, spec) = make_fulu_state(9);
        let attestation = make_electra_attestation(8, 0);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_state(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            !is_bad_committee_index(&err),
            "index 0 should pass the committee index check in Fulu, got BadCommitteeIndex"
        );
    }

    #[test]
    fn fulu_index_1_rejected() {
        // In Fulu (pre-Gloas), Electra attestation index must be == 0
        let (state, spec) = make_fulu_state(9);
        let attestation = make_electra_attestation(8, 1);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_state(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            is_bad_committee_index(&err),
            "expected BadCommitteeIndex for index 1 in Fulu, got {err:?}"
        );
    }

    #[test]
    fn fulu_index_2_rejected() {
        let (state, spec) = make_fulu_state(9);
        let attestation = make_electra_attestation(8, 2);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_state(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            is_bad_committee_index(&err),
            "expected BadCommitteeIndex for index 2 in Fulu, got {err:?}"
        );
    }

    // ── Block inclusion timing tests ─────────────────────────

    #[test]
    fn gloas_inclusion_too_early_rejected() {
        // State at slot 8, attestation at slot 8 — same slot, too early
        let (state, spec) = make_gloas_state(8);
        let attestation = make_electra_attestation(8, 0);
        let mut ctxt = ConsensusContext::new(state.slot());

        let err = extract_err(verify_attestation_for_block_inclusion(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            matches!(
                err,
                BlockOperationError::Invalid(AttestationInvalid::IncludedTooEarly { .. })
            ),
            "expected IncludedTooEarly, got {err:?}"
        );
    }

    #[test]
    fn gloas_inclusion_delay_respected() {
        // State at slot 9, attestation at slot 8 — inclusion delay = 1, should pass timing check
        let (state, spec) = make_gloas_state(9);
        let attestation = make_electra_attestation(8, 0);
        let mut ctxt = ConsensusContext::new(state.slot());

        // This will fail later (FFG or committee), but should NOT fail on IncludedTooEarly
        let err = extract_err(verify_attestation_for_block_inclusion(
            &state,
            attestation.to_ref(),
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        ));
        assert!(
            !matches!(
                err,
                BlockOperationError::Invalid(AttestationInvalid::IncludedTooEarly { .. })
            ),
            "should not be IncludedTooEarly at slot 9 for att at slot 8"
        );
    }
}
