use crate::upgrade::{
    upgrade_to_altair, upgrade_to_bellatrix, upgrade_to_capella, upgrade_to_deneb,
    upgrade_to_electra, upgrade_to_fulu, upgrade_to_gloas,
};
use crate::{per_epoch_processing::EpochProcessingSummary, *};
use safe_arith::{ArithError, SafeArith};
use ssz_types::typenum::Unsigned;
use tracing::instrument;
use types::*;

#[derive(Debug, PartialEq)]
pub enum Error {
    BeaconStateError(BeaconStateError),
    EpochProcessingError(EpochProcessingError),
    ArithError(ArithError),
    InconsistentStateFork(InconsistentFork),
}

impl From<ArithError> for Error {
    fn from(e: ArithError) -> Self {
        Self::ArithError(e)
    }
}

/// Advances a state forward by one slot, performing per-epoch processing if required.
///
/// If the root of the supplied `state` is known, then it can be passed as `state_root`. If
/// `state_root` is `None`, the root of `state` will be computed using a cached tree hash.
/// Providing the `state_root` makes this function several orders of magnitude faster.
#[instrument(level = "debug", skip_all)]
pub fn per_slot_processing<E: EthSpec>(
    state: &mut BeaconState<E>,
    state_root: Option<Hash256>,
    spec: &ChainSpec,
) -> Result<Option<EpochProcessingSummary<E>>, Error> {
    // Verify that the `BeaconState` instantiation matches the fork at `state.slot()`.
    state
        .fork_name(spec)
        .map_err(Error::InconsistentStateFork)?;

    cache_state(state, state_root)?;

    let summary = if state.slot() > spec.genesis_slot
        && state.slot().safe_add(1)?.safe_rem(E::slots_per_epoch())? == 0
    {
        Some(per_epoch_processing(state, spec)?)
    } else {
        None
    };

    state.slot_mut().safe_add_assign(1)?;

    // Process fork upgrades here. Note that multiple upgrades can potentially run
    // in sequence if they are scheduled in the same Epoch (common in testnets)
    if state.slot().safe_rem(E::slots_per_epoch())? == 0 {
        // If the Altair fork epoch is reached, perform an irregular state upgrade.
        if spec.altair_fork_epoch == Some(state.current_epoch()) {
            upgrade_to_altair(state, spec)?;
        }
        // If the Bellatrix fork epoch is reached, perform an irregular state upgrade.
        if spec.bellatrix_fork_epoch == Some(state.current_epoch()) {
            upgrade_to_bellatrix(state, spec)?;
        }
        // Capella.
        if spec.capella_fork_epoch == Some(state.current_epoch()) {
            upgrade_to_capella(state, spec)?;
        }
        // Deneb.
        if spec.deneb_fork_epoch == Some(state.current_epoch()) {
            upgrade_to_deneb(state, spec)?;
        }
        // Electra.
        if spec.electra_fork_epoch == Some(state.current_epoch()) {
            upgrade_to_electra(state, spec)?;
        }

        // Fulu.
        if spec.fulu_fork_epoch == Some(state.current_epoch()) {
            upgrade_to_fulu(state, spec)?;
        }

        // Gloas.
        if spec.gloas_fork_epoch == Some(state.current_epoch()) {
            upgrade_to_gloas(state, spec)?;
        }

        // Additionally build all caches so that all valid states that are advanced always have
        // committee caches built, and we don't have to worry about initialising them at higher
        // layers.
        state.build_caches(spec)?;
    }

    Ok(summary)
}

#[instrument(skip_all)]
fn cache_state<E: EthSpec>(
    state: &mut BeaconState<E>,
    state_root: Option<Hash256>,
) -> Result<(), Error> {
    let previous_state_root = if let Some(root) = state_root {
        root
    } else {
        state.update_tree_hash_cache()?
    };

    // Note: increment the state slot here to allow use of our `state_root` and `block_root`
    // getter/setter functions.
    //
    // This is a bit hacky, however it gets the job done safely without lots of code.
    let previous_slot = state.slot();
    state.slot_mut().safe_add_assign(1)?;

    // Store the previous slot's post state transition root.
    state.set_state_root(previous_slot, previous_state_root)?;

    // Cache latest block header state root
    if state.latest_block_header().state_root == Hash256::zero() {
        state.latest_block_header_mut().state_root = previous_state_root;
    }

    // Cache block root
    let latest_block_root = state.latest_block_header().canonical_root();
    state.set_block_root(previous_slot, latest_block_root)?;

    // [New in Gloas:EIP7732] Unset the next payload availability
    // spec: state.execution_payload_availability[(state.slot + 1) % SLOTS_PER_HISTORICAL_ROOT] = 0b0
    // Note: at this point state.slot has been temporarily incremented by 1,
    // so state.slot() already represents the "next slot" from the spec's perspective.
    if state.fork_name_unchecked().gloas_enabled() {
        let next_slot_index = state
            .slot()
            .as_usize()
            .safe_rem(E::SlotsPerHistoricalRoot::to_usize())?;
        if let Ok(gloas_state) = state.as_gloas_mut() {
            gloas_state
                .execution_payload_availability
                .set(next_slot_index, false)
                .map_err(|_| BeaconStateError::SlotOutOfBounds)?;
        }
    }

    // Set the state slot back to what it should be.
    state.slot_mut().safe_sub_assign(1)?;

    Ok(())
}

impl From<BeaconStateError> for Error {
    fn from(e: BeaconStateError) -> Error {
        Error::BeaconStateError(e)
    }
}

impl From<EpochProcessingError> for Error {
    fn from(e: EpochProcessingError) -> Error {
        Error::EpochProcessingError(e)
    }
}

#[cfg(test)]
mod gloas_per_slot_tests {
    use super::*;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::{
        BeaconBlockHeader, BeaconStateGloas, BuilderPendingPayment, CACHED_EPOCHS, Checkpoint,
        CommitteeCache, Epoch, ExecutionBlockHash, ExecutionPayloadBid, ExitCache, FixedVector,
        Fork, Hash256, List, MinimalEthSpec, ProgressiveBalancesCache, PubkeyCache, PublicKeyBytes,
        SlashingsCache, SyncCommittee, Unsigned, Vector,
    };

    type E = MinimalEthSpec;

    /// Build a minimal Gloas state at the given slot for per_slot_processing tests.
    /// The state has all availability bits set to true (0xFF) so we can verify clearing.
    fn make_gloas_state_at_slot(slot_num: u64) -> (BeaconState<E>, ChainSpec) {
        let mut spec = E::default_spec();
        // Set all forks at epoch 0 so any slot is valid as Gloas
        spec.altair_fork_epoch = Some(Epoch::new(0));
        spec.bellatrix_fork_epoch = Some(Epoch::new(0));
        spec.capella_fork_epoch = Some(Epoch::new(0));
        spec.deneb_fork_epoch = Some(Epoch::new(0));
        spec.electra_fork_epoch = Some(Epoch::new(0));
        spec.fulu_fork_epoch = Some(Epoch::new(0));
        spec.gloas_fork_epoch = Some(Epoch::new(0));

        let slot = Slot::new(slot_num);

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let block_roots_vec: Vec<Hash256> = (0..slots_per_hist)
            .map(|i| Hash256::repeat_byte((i % 255 + 1) as u8))
            .collect();

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        // All availability bits set to true
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
                body_root: Hash256::repeat_byte(0x01),
            },
            block_roots: Vector::new(block_roots_vec).unwrap(),
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
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec)
    }

    #[test]
    fn cache_state_clears_next_slot_availability_bit() {
        // State at slot 5 (not epoch boundary). After cache_state, slot 6's
        // availability bit should be cleared.
        let (mut state, _spec) = make_gloas_state_at_slot(5);
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();

        // Verify all bits start as true
        let next_slot_index = 6 % slots_per_hist;
        assert!(
            state
                .as_gloas()
                .unwrap()
                .execution_payload_availability
                .get(next_slot_index)
                .unwrap(),
            "bit should start as true"
        );

        let state_root = Hash256::repeat_byte(0xDD);
        cache_state(&mut state, Some(state_root)).unwrap();

        // The next slot's availability bit should now be false
        assert!(
            !state
                .as_gloas()
                .unwrap()
                .execution_payload_availability
                .get(next_slot_index)
                .unwrap(),
            "next slot availability bit should be cleared after cache_state"
        );
    }

    #[test]
    fn cache_state_clears_correct_bit_at_wraparound() {
        // Test that the modular index wraps correctly near the end of the historical root vector.
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        // State at slot (slots_per_hist - 1). Next slot index = 0 (wraparound).
        let slot_num = (slots_per_hist - 1) as u64;
        let (mut state, _spec) = make_gloas_state_at_slot(slot_num);

        // Next slot index after wraparound = slot_num + 1 = slots_per_hist, mod = 0
        let next_slot_index = 0;
        assert!(
            state
                .as_gloas()
                .unwrap()
                .execution_payload_availability
                .get(next_slot_index)
                .unwrap(),
            "bit 0 should start as true"
        );

        let state_root = Hash256::repeat_byte(0xDD);
        cache_state(&mut state, Some(state_root)).unwrap();

        assert!(
            !state
                .as_gloas()
                .unwrap()
                .execution_payload_availability
                .get(next_slot_index)
                .unwrap(),
            "bit 0 should be cleared at wraparound"
        );
    }

    #[test]
    fn cache_state_does_not_clear_other_bits() {
        // Verify that only the next slot's bit is cleared, others remain true.
        let (mut state, _spec) = make_gloas_state_at_slot(5);
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let next_slot_index = 6 % slots_per_hist;

        let state_root = Hash256::repeat_byte(0xDD);
        cache_state(&mut state, Some(state_root)).unwrap();

        // Check that all other bits are still true
        for i in 0..slots_per_hist {
            let bit = state
                .as_gloas()
                .unwrap()
                .execution_payload_availability
                .get(i)
                .unwrap();
            if i == next_slot_index {
                assert!(!bit, "next slot bit should be cleared");
            } else {
                assert!(bit, "bit {} should remain true", i);
            }
        }
    }

    #[test]
    fn cache_state_already_false_stays_false() {
        // If the bit is already false, clearing it is a no-op (no error).
        let (mut state, _spec) = make_gloas_state_at_slot(5);
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let next_slot_index = 6 % slots_per_hist;

        // Pre-clear the bit
        state
            .as_gloas_mut()
            .unwrap()
            .execution_payload_availability
            .set(next_slot_index, false)
            .unwrap();

        let state_root = Hash256::repeat_byte(0xDD);
        cache_state(&mut state, Some(state_root)).unwrap();

        assert!(
            !state
                .as_gloas()
                .unwrap()
                .execution_payload_availability
                .get(next_slot_index)
                .unwrap(),
            "already-false bit stays false"
        );
    }

    #[test]
    fn cache_state_preserves_state_root_and_block_root() {
        // Verify cache_state still performs its normal duties (state root caching,
        // block root caching) alongside the Gloas availability clearing.
        let (mut state, _spec) = make_gloas_state_at_slot(5);
        let state_root = Hash256::repeat_byte(0xDD);
        cache_state(&mut state, Some(state_root)).unwrap();

        // The state root for slot 5 should be stored in state_roots[5 % SlotsPerHistoricalRoot].
        // We read it directly from the vector because the slot-based getter requires slot < state.slot(),
        // but cache_state restores the slot to its original value.
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let state_root_index = 5 % slots_per_hist;
        assert_eq!(
            *state.state_roots().get(state_root_index).unwrap(),
            state_root,
            "state root for previous slot should be cached"
        );
    }

    #[test]
    fn per_slot_processing_clears_availability_and_advances_slot() {
        // End-to-end test: per_slot_processing should clear the next slot's
        // availability bit and advance the slot.
        // Use slot 1 (not epoch boundary, not genesis slot)
        let (mut state, spec) = make_gloas_state_at_slot(1);
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();

        // The slot will become 2 after processing; cache_state clears bit for slot 2
        let next_slot_index = 2 % slots_per_hist;

        let state_root = Hash256::repeat_byte(0xDD);
        per_slot_processing(&mut state, Some(state_root), &spec).unwrap();

        assert_eq!(state.slot(), Slot::new(2));
        assert!(
            !state
                .as_gloas()
                .unwrap()
                .execution_payload_availability
                .get(next_slot_index)
                .unwrap(),
            "per_slot_processing should clear next slot availability bit"
        );
    }
}
