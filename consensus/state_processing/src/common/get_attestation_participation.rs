use integer_sqrt::IntegerSquareRoot;
use safe_arith::SafeArith;
use smallvec::SmallVec;
use types::{AttestationData, BeaconState, ChainSpec, EthSpec, Slot};
use types::{
    BeaconStateError as Error,
    consts::altair::{
        NUM_FLAG_INDICES, TIMELY_HEAD_FLAG_INDEX, TIMELY_SOURCE_FLAG_INDEX,
        TIMELY_TARGET_FLAG_INDEX,
    },
};

/// [New in Gloas:EIP7732]
/// Check if attestation targets the block proposed at the attestation slot.
pub fn is_attestation_same_slot<E: EthSpec>(
    state: &BeaconState<E>,
    data: &AttestationData,
) -> Result<bool, Error> {
    if data.slot == Slot::new(0) {
        return Ok(true);
    }
    let blockroot = data.beacon_block_root;
    let slot_blockroot = *state.get_block_root(data.slot)?;
    let prev_blockroot = *state.get_block_root(data.slot.safe_sub(1u64)?)?;
    Ok(blockroot == slot_blockroot && blockroot != prev_blockroot)
}

/// Get the participation flags for a valid attestation.
///
/// You should have called `verify_attestation_for_block_inclusion` or similar before
/// calling this function, in order to ensure that the attestation's source is correct.
///
/// This function will return an error if the source of the attestation doesn't match the
/// state's relevant justified checkpoint.
pub fn get_attestation_participation_flag_indices<E: EthSpec>(
    state: &BeaconState<E>,
    data: &AttestationData,
    inclusion_delay: u64,
    spec: &ChainSpec,
) -> Result<SmallVec<[usize; NUM_FLAG_INDICES]>, Error> {
    let justified_checkpoint = if data.target.epoch == state.current_epoch() {
        state.current_justified_checkpoint()
    } else {
        state.previous_justified_checkpoint()
    };

    // Matching roots.
    let is_matching_source = data.source == justified_checkpoint;
    let is_matching_target = is_matching_source
        && data.target.root == *state.get_block_root_at_epoch(data.target.epoch)?;

    let head_root_matches = data.beacon_block_root == *state.get_block_root(data.slot)?;

    // [Modified in Gloas:EIP7732] head flag also requires payload_matches
    let is_matching_head = if state.fork_name_unchecked().gloas_enabled() {
        let is_same_slot = is_attestation_same_slot(state, data)?;
        // [New in Gloas:EIP7732] Same-slot attestations must have data.index == 0
        if is_same_slot && data.index != 0 {
            return Err(Error::IncorrectAttestationIndex);
        }
        let payload_matches = if is_same_slot {
            // Same-slot attestations always match payload
            true
        } else {
            // Historical: check execution_payload_availability
            let slot_index = data
                .slot
                .as_usize()
                .safe_rem(E::slots_per_historical_root())?;
            let availability = state
                .as_gloas()
                .map(|s| {
                    s.execution_payload_availability
                        .get(slot_index)
                        .map(|b| b as u64)
                        .unwrap_or(0)
                })
                .unwrap_or(0);
            data.index == availability
        };
        is_matching_target && head_root_matches && payload_matches
    } else {
        is_matching_target && head_root_matches
    };

    if !is_matching_source {
        return Err(Error::IncorrectAttestationSource);
    }

    // Participation flag indices
    let mut participation_flag_indices = SmallVec::new();
    if is_matching_source && inclusion_delay <= E::slots_per_epoch().integer_sqrt() {
        participation_flag_indices.push(TIMELY_SOURCE_FLAG_INDEX);
    }
    if state.fork_name_unchecked().deneb_enabled() {
        if is_matching_target {
            // [Modified in Deneb:EIP7045]
            participation_flag_indices.push(TIMELY_TARGET_FLAG_INDEX);
        }
    } else if is_matching_target && inclusion_delay <= E::slots_per_epoch() {
        participation_flag_indices.push(TIMELY_TARGET_FLAG_INDEX);
    }

    if is_matching_head && inclusion_delay == spec.min_attestation_inclusion_delay {
        participation_flag_indices.push(TIMELY_HEAD_FLAG_INDEX);
    }
    Ok(participation_flag_indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::{
        BeaconBlockHeader, BeaconStateGloas, BuilderPendingPayment, CACHED_EPOCHS, Checkpoint,
        CommitteeCache, Epoch, ExecutionBlockHash, ExecutionPayloadBid, ExitCache,
        FixedBytesExtended, FixedVector, Fork, Hash256, List, MinimalEthSpec,
        ProgressiveBalancesCache, PubkeyCache, PublicKeyBytes, SlashingsCache, SyncCommittee,
        Unsigned, Vector,
    };

    type E = MinimalEthSpec;

    /// Build a minimal Gloas state at the given slot, with block roots and checkpoints
    /// configured for attestation participation testing.
    ///
    /// The slot must be > epoch start slot to allow get_block_root_at_epoch to work.
    fn make_gloas_state_for_attestation(slot_num: u64) -> (BeaconState<E>, ChainSpec) {
        assert!(
            !slot_num.is_multiple_of(E::slots_per_epoch()),
            "slot must not be at epoch boundary for get_block_root_at_epoch to work"
        );
        let spec = E::default_spec();
        let slot = Slot::new(slot_num);
        let epoch = slot.epoch(E::slots_per_epoch());

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        // Unique block root per slot (repeat_byte)
        let block_roots_vec: Vec<Hash256> = (0..slots_per_hist)
            .map(|i| Hash256::repeat_byte((i % 255 + 1) as u8))
            .collect();

        let randao_mixes = vec![Hash256::zero(); epochs_per_vector];

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

        // All availability bits set to true by default
        let state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root: Hash256::zero(),
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(block_roots_vec).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::default(),
            balances: List::default(),
            randao_mixes: Vector::new(randao_mixes).unwrap(),
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
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec)
    }

    /// Get the block root stored at `slot` in the state's block_roots vector.
    fn block_root_at(state: &BeaconState<E>, slot: u64) -> Hash256 {
        *state.get_block_root(Slot::new(slot)).unwrap()
    }

    // ========================
    // is_attestation_same_slot
    // ========================

    #[test]
    fn same_slot_at_slot_zero() {
        let (state, _) = make_gloas_state_for_attestation(17);
        let data = AttestationData {
            slot: Slot::new(0),
            index: 0,
            beacon_block_root: Hash256::zero(),
            source: Checkpoint::default(),
            target: Checkpoint::default(),
        };
        // Slot 0 always returns true
        assert!(is_attestation_same_slot(&state, &data).unwrap());
    }

    #[test]
    fn same_slot_when_block_matches_and_differs_from_prev() {
        let (state, _) = make_gloas_state_for_attestation(17);
        // Slot 10: block_roots[10] has a unique root, different from block_roots[9]
        let slot = 10u64;
        let root = block_root_at(&state, slot);
        let prev_root = block_root_at(&state, slot - 1);
        assert_ne!(root, prev_root); // our test setup gives unique roots

        let data = AttestationData {
            slot: Slot::new(slot),
            index: 0,
            beacon_block_root: root,
            source: Checkpoint::default(),
            target: Checkpoint::default(),
        };
        assert!(is_attestation_same_slot(&state, &data).unwrap());
    }

    #[test]
    fn not_same_slot_when_root_equals_prev() {
        // If block_roots[slot] == block_roots[slot-1], it means this slot was skipped
        let (mut state, _) = make_gloas_state_for_attestation(17);
        let slot = 10u64;
        let prev_root = block_root_at(&state, slot - 1);
        // Set block_roots[10] = block_roots[9] to simulate a skipped slot
        state.set_block_root(Slot::new(slot), prev_root).unwrap();

        let data = AttestationData {
            slot: Slot::new(slot),
            index: 0,
            beacon_block_root: prev_root,
            source: Checkpoint::default(),
            target: Checkpoint::default(),
        };
        // blockroot == slot_blockroot BUT blockroot == prev_blockroot → false
        assert!(!is_attestation_same_slot(&state, &data).unwrap());
    }

    #[test]
    fn not_same_slot_when_attestation_root_differs() {
        let (state, _) = make_gloas_state_for_attestation(17);
        let slot = 10u64;
        // Attestation references a different block root than what's at slot 10
        let data = AttestationData {
            slot: Slot::new(slot),
            index: 0,
            beacon_block_root: Hash256::repeat_byte(0xFF),
            source: Checkpoint::default(),
            target: Checkpoint::default(),
        };
        // blockroot != slot_blockroot → false
        assert!(!is_attestation_same_slot(&state, &data).unwrap());
    }

    // =============================================
    // get_attestation_participation_flag_indices
    // Gloas-specific head flag behavior
    // =============================================

    /// Make attestation data with matching source and target for current epoch
    fn make_matching_attestation(
        state: &BeaconState<E>,
        att_slot: u64,
        index: u64,
    ) -> AttestationData {
        let epoch = state.current_epoch();
        let target_slot = epoch.start_slot(E::slots_per_epoch());
        let target_root = *state.get_block_root(target_slot).unwrap();
        AttestationData {
            slot: Slot::new(att_slot),
            index,
            beacon_block_root: *state.get_block_root(Slot::new(att_slot)).unwrap(),
            source: state.current_justified_checkpoint(),
            target: Checkpoint {
                epoch,
                root: target_root,
            },
        }
    }

    #[test]
    fn gloas_same_slot_index_zero_gets_head_flag() {
        // State at slot 16 (epoch 2), attestation at slot 10 (epoch 1, same-slot)
        // With unique block roots, slot 10 is same-slot, and index=0 should work
        let (state, spec) = make_gloas_state_for_attestation(17);
        let data = make_matching_attestation(&state, 10, 0);
        let flags = get_attestation_participation_flag_indices(&state, &data, 1, &spec).unwrap();
        assert!(
            flags.contains(&TIMELY_HEAD_FLAG_INDEX),
            "same-slot attestation with index=0 should get head flag"
        );
    }

    #[test]
    fn gloas_same_slot_index_nonzero_errors() {
        // Same-slot attestation with index != 0 should return IncorrectAttestationIndex
        let (state, spec) = make_gloas_state_for_attestation(17);
        let data = make_matching_attestation(&state, 10, 1);
        let result = get_attestation_participation_flag_indices(&state, &data, 1, &spec);
        assert_eq!(result.unwrap_err(), Error::IncorrectAttestationIndex);
    }

    #[test]
    fn gloas_historical_attestation_index_matches_availability() {
        // Historical (not same-slot) attestation: head flag depends on index == availability bit
        let (mut state, spec) = make_gloas_state_for_attestation(17);

        // Make slot 10 a skipped slot (block_roots[10] == block_roots[9])
        let prev_root = block_root_at(&state, 9);
        state.set_block_root(Slot::new(10), prev_root).unwrap();

        // Attestation at slot 10, referencing the block at slot 9 (skip slot behavior)
        let epoch = state.current_epoch();
        let target_slot = epoch.start_slot(E::slots_per_epoch());
        let target_root = *state.get_block_root(target_slot).unwrap();

        // availability bit at index 10 is true (1) in our default setup
        let data = AttestationData {
            slot: Slot::new(10),
            index: 1, // matches availability bit = true (1)
            beacon_block_root: prev_root,
            source: state.current_justified_checkpoint(),
            target: Checkpoint {
                epoch,
                root: target_root,
            },
        };
        let flags = get_attestation_participation_flag_indices(&state, &data, 1, &spec).unwrap();
        assert!(
            flags.contains(&TIMELY_HEAD_FLAG_INDEX),
            "historical attestation with index matching availability should get head flag"
        );
    }

    #[test]
    fn gloas_historical_attestation_index_mismatches_availability() {
        // Historical attestation: index doesn't match availability → no head flag
        let (mut state, spec) = make_gloas_state_for_attestation(17);

        // Make slot 10 a skipped slot
        let prev_root = block_root_at(&state, 9);
        state.set_block_root(Slot::new(10), prev_root).unwrap();

        let epoch = state.current_epoch();
        let target_slot = epoch.start_slot(E::slots_per_epoch());
        let target_root = *state.get_block_root(target_slot).unwrap();

        // availability bit at index 10 is true (1), but attestation index = 0
        let data = AttestationData {
            slot: Slot::new(10),
            index: 0, // doesn't match availability=1
            beacon_block_root: prev_root,
            source: state.current_justified_checkpoint(),
            target: Checkpoint {
                epoch,
                root: target_root,
            },
        };
        let flags = get_attestation_participation_flag_indices(&state, &data, 1, &spec).unwrap();
        assert!(
            !flags.contains(&TIMELY_HEAD_FLAG_INDEX),
            "index != availability means no head flag"
        );
    }

    #[test]
    fn gloas_historical_availability_false_index_zero_gets_head() {
        // Clear the availability bit at slot 10 → availability = 0
        let (mut state, spec) = make_gloas_state_for_attestation(17);

        // Make slot 10 a skipped slot
        let prev_root = block_root_at(&state, 9);
        state.set_block_root(Slot::new(10), prev_root).unwrap();

        // Clear availability for slot 10
        let slot_index = 10 % <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        state
            .as_gloas_mut()
            .unwrap()
            .execution_payload_availability
            .set(slot_index, false)
            .unwrap();

        let epoch = state.current_epoch();
        let target_slot = epoch.start_slot(E::slots_per_epoch());
        let target_root = *state.get_block_root(target_slot).unwrap();

        let data = AttestationData {
            slot: Slot::new(10),
            index: 0, // matches availability=0
            beacon_block_root: prev_root,
            source: state.current_justified_checkpoint(),
            target: Checkpoint {
                epoch,
                root: target_root,
            },
        };
        let flags = get_attestation_participation_flag_indices(&state, &data, 1, &spec).unwrap();
        assert!(
            flags.contains(&TIMELY_HEAD_FLAG_INDEX),
            "index=0 matches availability=false(0), should get head flag"
        );
    }

    #[test]
    fn gloas_source_and_target_flags_still_work() {
        // Source and target flags should still work independently of the Gloas head logic
        let (state, spec) = make_gloas_state_for_attestation(17);
        let data = make_matching_attestation(&state, 10, 0);

        // inclusion_delay=1 (min) → source flag
        let flags = get_attestation_participation_flag_indices(&state, &data, 1, &spec).unwrap();
        assert!(flags.contains(&TIMELY_SOURCE_FLAG_INDEX));
        assert!(flags.contains(&TIMELY_TARGET_FLAG_INDEX));
        assert!(flags.contains(&TIMELY_HEAD_FLAG_INDEX));
    }

    #[test]
    fn gloas_high_inclusion_delay_no_source_flag() {
        let (state, spec) = make_gloas_state_for_attestation(17);
        let data = make_matching_attestation(&state, 10, 0);

        // inclusion_delay > sqrt(slots_per_epoch) → no source flag
        // MinimalEthSpec: slots_per_epoch=8, sqrt(8)=2
        let flags = get_attestation_participation_flag_indices(&state, &data, 3, &spec).unwrap();
        assert!(!flags.contains(&TIMELY_SOURCE_FLAG_INDEX));
        // Target still present (deneb+)
        assert!(flags.contains(&TIMELY_TARGET_FLAG_INDEX));
    }

    #[test]
    fn gloas_high_inclusion_delay_no_head_flag() {
        let (state, spec) = make_gloas_state_for_attestation(17);
        let data = make_matching_attestation(&state, 10, 0);

        // inclusion_delay > 1 → no head flag
        let flags = get_attestation_participation_flag_indices(&state, &data, 2, &spec).unwrap();
        assert!(!flags.contains(&TIMELY_HEAD_FLAG_INDEX));
    }

    #[test]
    fn gloas_wrong_source_errors() {
        let (state, spec) = make_gloas_state_for_attestation(17);
        let mut data = make_matching_attestation(&state, 10, 0);
        data.source = Checkpoint {
            epoch: Epoch::new(0),
            root: Hash256::repeat_byte(0xDE),
        };
        let result = get_attestation_participation_flag_indices(&state, &data, 1, &spec);
        assert_eq!(result.unwrap_err(), Error::IncorrectAttestationSource);
    }

    #[test]
    fn gloas_head_root_mismatch_no_head_flag() {
        let (state, spec) = make_gloas_state_for_attestation(17);
        let mut data = make_matching_attestation(&state, 10, 0);
        // Wrong beacon_block_root → head_root_matches = false → no head flag
        data.beacon_block_root = Hash256::repeat_byte(0xFE);
        let flags = get_attestation_participation_flag_indices(&state, &data, 1, &spec).unwrap();
        assert!(!flags.contains(&TIMELY_HEAD_FLAG_INDEX));
        // Source and target can still match
        assert!(flags.contains(&TIMELY_SOURCE_FLAG_INDEX));
    }

    #[test]
    fn gloas_wrong_target_root_no_target_or_head() {
        let (state, spec) = make_gloas_state_for_attestation(17);
        let mut data = make_matching_attestation(&state, 10, 0);
        data.target.root = Hash256::repeat_byte(0xFD);
        let flags = get_attestation_participation_flag_indices(&state, &data, 1, &spec).unwrap();
        assert!(flags.contains(&TIMELY_SOURCE_FLAG_INDEX));
        assert!(!flags.contains(&TIMELY_TARGET_FLAG_INDEX));
        assert!(!flags.contains(&TIMELY_HEAD_FLAG_INDEX));
    }
}
