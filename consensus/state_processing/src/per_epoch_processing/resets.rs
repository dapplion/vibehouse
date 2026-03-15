use super::errors::EpochProcessingError;
use safe_arith::SafeArith;
use types::beacon_state::BeaconState;
use types::eth_spec::EthSpec;
use types::{List, Unsigned};

pub fn process_eth1_data_reset<E: EthSpec>(
    state: &mut BeaconState<E>,
) -> Result<(), EpochProcessingError> {
    if state
        .slot()
        .safe_add(1)?
        .safe_rem(<E as EthSpec>::SlotsPerEth1VotingPeriod::to_u64())?
        == 0
    {
        *state.eth1_data_votes_mut() = List::empty();
    }
    Ok(())
}

pub fn process_slashings_reset<E: EthSpec>(
    state: &mut BeaconState<E>,
) -> Result<(), EpochProcessingError> {
    let next_epoch = state.next_epoch()?;
    state.set_slashings(next_epoch, 0)?;
    Ok(())
}

pub fn process_randao_mixes_reset<E: EthSpec>(
    state: &mut BeaconState<E>,
) -> Result<(), EpochProcessingError> {
    let current_epoch = state.current_epoch();
    let next_epoch = state.next_epoch()?;
    state.set_randao_mix(next_epoch, *state.get_randao_mix(current_epoch)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use types::beacon_state::BuilderPubkeyCache;
    use types::*;

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    fn make_gloas_state(spec: &ChainSpec) -> BeaconState<E> {
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
            total_active_balance: Some((current_epoch, 0)),
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <_>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: EpochCache::default(),
        })
    }

    // --- eth1_data_reset tests ---

    #[test]
    fn eth1_data_votes_cleared_at_period_boundary() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        // Set slot so that (slot + 1) % SlotsPerEth1VotingPeriod == 0
        let voting_period = <E as EthSpec>::SlotsPerEth1VotingPeriod::to_u64();
        let target_slot = voting_period - 1; // slot + 1 == voting_period
        *state.slot_mut() = Slot::new(target_slot);

        // Add some votes
        state
            .eth1_data_votes_mut()
            .push(Eth1Data::default())
            .unwrap();
        state
            .eth1_data_votes_mut()
            .push(Eth1Data::default())
            .unwrap();
        assert_eq!(state.eth1_data_votes().len(), 2);

        process_eth1_data_reset::<E>(&mut state).unwrap();

        assert_eq!(state.eth1_data_votes().len(), 0);
    }

    #[test]
    fn eth1_data_votes_not_cleared_mid_period() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        // Set slot in the middle of a voting period
        let voting_period = <E as EthSpec>::SlotsPerEth1VotingPeriod::to_u64();
        *state.slot_mut() = Slot::new(voting_period / 2);

        state
            .eth1_data_votes_mut()
            .push(Eth1Data::default())
            .unwrap();
        assert_eq!(state.eth1_data_votes().len(), 1);

        process_eth1_data_reset::<E>(&mut state).unwrap();

        assert_eq!(state.eth1_data_votes().len(), 1);
    }

    // --- slashings_reset tests ---

    #[test]
    fn slashings_reset_clears_next_epoch_slot() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        // Set a nonzero value in the next_epoch's slashings slot
        let next_epoch = state.next_epoch().unwrap();
        let next_idx = next_epoch.as_usize() % <E as EthSpec>::EpochsPerSlashingsVector::to_usize();
        state.set_slashings(next_epoch, 999).unwrap();
        assert_eq!(*state.get_all_slashings().get(next_idx).unwrap(), 999);

        process_slashings_reset::<E>(&mut state).unwrap();

        assert_eq!(*state.get_all_slashings().get(next_idx).unwrap(), 0);
    }

    #[test]
    fn slashings_reset_preserves_other_slots() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        let current_epoch = state.current_epoch();
        state.set_slashings(current_epoch, 42).unwrap();

        process_slashings_reset::<E>(&mut state).unwrap();

        // Current epoch's slashing value should be preserved
        assert_eq!(state.get_slashings(current_epoch).unwrap(), 42);
    }

    // --- randao_mixes_reset tests ---

    #[test]
    fn randao_mixes_copies_current_to_next() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        let current_epoch = state.current_epoch();
        let next_epoch = state.next_epoch().unwrap();
        let mix = Hash256::repeat_byte(0xBB);
        state.set_randao_mix(current_epoch, mix).unwrap();

        // Verify next_epoch slot is initially zero
        let next_idx = next_epoch.as_usize() % state.randao_mixes().len();
        assert_eq!(
            *state.randao_mixes().get(next_idx).unwrap(),
            Hash256::zero()
        );

        process_randao_mixes_reset::<E>(&mut state).unwrap();

        // After reset, next_epoch slot should have current epoch's mix
        assert_eq!(*state.randao_mixes().get(next_idx).unwrap(), mix);
    }

    #[test]
    fn randao_mixes_preserves_current_epoch() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        let current_epoch = state.current_epoch();
        let mix = Hash256::repeat_byte(0xCC);
        state.set_randao_mix(current_epoch, mix).unwrap();

        process_randao_mixes_reset::<E>(&mut state).unwrap();

        assert_eq!(*state.get_randao_mix(current_epoch).unwrap(), mix);
    }

    #[test]
    fn randao_mixes_zero_default() {
        let spec = make_spec();
        let mut state = make_gloas_state(&spec);

        // Default is all zeros
        process_randao_mixes_reset::<E>(&mut state).unwrap();

        let next_epoch = state.next_epoch().unwrap();
        let next_idx = next_epoch.as_usize() % state.randao_mixes().len();
        assert_eq!(
            *state.randao_mixes().get(next_idx).unwrap(),
            Hash256::zero()
        );
    }
}
