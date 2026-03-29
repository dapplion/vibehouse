use std::mem;
use types::{
    BeaconState, BeaconStateError as Error, BeaconStateHeze, BitVector, ChainSpec, EthSpec,
    ExecutionPayloadBidHeze, Fork,
};

/// Transform a `Gloas` state into a `Heze` state.
pub fn upgrade_to_heze<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let epoch = pre_state.current_epoch();
    let pre = pre_state.as_gloas_mut()?;

    let post = BeaconState::Heze(BeaconStateHeze {
        // Versioning
        genesis_time: pre.genesis_time,
        genesis_validators_root: pre.genesis_validators_root,
        slot: pre.slot,
        fork: Fork {
            previous_version: pre.fork.current_version,
            current_version: spec.heze_fork_version,
            epoch,
        },
        // History
        latest_block_header: pre.latest_block_header,
        block_roots: pre.block_roots.clone(),
        state_roots: pre.state_roots.clone(),
        historical_roots: mem::take(&mut pre.historical_roots),
        // Eth1
        eth1_data: pre.eth1_data,
        eth1_data_votes: mem::take(&mut pre.eth1_data_votes),
        eth1_deposit_index: pre.eth1_deposit_index,
        // Registry
        validators: mem::take(&mut pre.validators),
        balances: mem::take(&mut pre.balances),
        // Randomness
        randao_mixes: pre.randao_mixes.clone(),
        // Slashings
        slashings: pre.slashings.clone(),
        // Participation
        previous_epoch_participation: mem::take(&mut pre.previous_epoch_participation),
        current_epoch_participation: mem::take(&mut pre.current_epoch_participation),
        // Finality
        justification_bits: pre.justification_bits.clone(),
        previous_justified_checkpoint: pre.previous_justified_checkpoint,
        current_justified_checkpoint: pre.current_justified_checkpoint,
        finalized_checkpoint: pre.finalized_checkpoint,
        // Inactivity
        inactivity_scores: mem::take(&mut pre.inactivity_scores),
        // Sync committees
        current_sync_committee: pre.current_sync_committee.clone(),
        next_sync_committee: pre.next_sync_committee.clone(),
        // Execution Bid
        latest_execution_payload_bid: ExecutionPayloadBidHeze {
            parent_block_hash: pre.latest_execution_payload_bid.parent_block_hash,
            parent_block_root: pre.latest_execution_payload_bid.parent_block_root,
            block_hash: pre.latest_execution_payload_bid.block_hash,
            prev_randao: pre.latest_execution_payload_bid.prev_randao,
            fee_recipient: pre.latest_execution_payload_bid.fee_recipient,
            gas_limit: pre.latest_execution_payload_bid.gas_limit,
            builder_index: pre.latest_execution_payload_bid.builder_index,
            slot: pre.latest_execution_payload_bid.slot,
            value: pre.latest_execution_payload_bid.value,
            execution_payment: pre.latest_execution_payload_bid.execution_payment,
            blob_kzg_commitments: pre
                .latest_execution_payload_bid
                .blob_kzg_commitments
                .clone(),
            inclusion_list_bits: BitVector::default(),
        },
        // Capella
        next_withdrawal_index: pre.next_withdrawal_index,
        next_withdrawal_validator_index: pre.next_withdrawal_validator_index,
        historical_summaries: mem::take(&mut pre.historical_summaries),
        // Electra
        deposit_requests_start_index: pre.deposit_requests_start_index,
        deposit_balance_to_consume: pre.deposit_balance_to_consume,
        exit_balance_to_consume: pre.exit_balance_to_consume,
        earliest_exit_epoch: pre.earliest_exit_epoch,
        consolidation_balance_to_consume: pre.consolidation_balance_to_consume,
        earliest_consolidation_epoch: pre.earliest_consolidation_epoch,
        pending_deposits: mem::take(&mut pre.pending_deposits),
        pending_partial_withdrawals: mem::take(&mut pre.pending_partial_withdrawals),
        pending_consolidations: mem::take(&mut pre.pending_consolidations),
        proposer_lookahead: mem::take(&mut pre.proposer_lookahead),
        // Gloas (carried over)
        builders: mem::take(&mut pre.builders),
        next_withdrawal_builder_index: pre.next_withdrawal_builder_index,
        execution_payload_availability: pre.execution_payload_availability.clone(),
        builder_pending_payments: pre.builder_pending_payments.clone(),
        builder_pending_withdrawals: mem::take(&mut pre.builder_pending_withdrawals),
        latest_block_hash: pre.latest_block_hash,
        payload_expected_withdrawals: mem::take(&mut pre.payload_expected_withdrawals),
        ptc_window: pre.ptc_window.clone(),
        // Caches
        total_active_balance: pre.total_active_balance,
        progressive_balances_cache: mem::take(&mut pre.progressive_balances_cache),
        committee_caches: mem::take(&mut pre.committee_caches),
        pubkey_cache: mem::take(&mut pre.pubkey_cache),
        builder_pubkey_cache: mem::take(&mut pre.builder_pubkey_cache),
        exit_cache: mem::take(&mut pre.exit_cache),
        slashings_cache: mem::take(&mut pre.slashings_cache),
        epoch_cache: mem::take(&mut pre.epoch_cache),
    });

    *pre_state = post;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::FixedBytesExtended;
    use ssz_types::typenum::Unsigned;
    use std::sync::Arc;
    use types::test_utils::generate_deterministic_keypairs;
    use types::{
        BeaconBlockHeader, BeaconStateGloas, BuilderPendingPayment, BuilderPubkeyCache,
        CACHED_EPOCHS, Checkpoint, CommitteeCache, Epoch, ExecutionBlockHash,
        ExecutionPayloadBidGloas, ExitCache, FixedVector, Hash256, List, MinimalEthSpec,
        ProgressiveBalancesCache, PubkeyCache, PublicKeyBytes, SlashingsCache, Slot, SyncCommittee,
        Validator, Vector,
    };

    type E = MinimalEthSpec;

    const BALANCE: u64 = 32_000_000_000;
    const NUM_VALIDATORS: usize = 4;

    /// Create a Gloas BeaconState suitable for testing upgrade_to_heze.
    fn make_gloas_state() -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let slot = Slot::new(E::slots_per_epoch() * 2); // slot 16 = epoch 2
        let epoch = slot.epoch(E::slots_per_epoch());

        let keypairs = generate_deterministic_keypairs(NUM_VALIDATORS);
        let mut validators = Vec::with_capacity(NUM_VALIDATORS);
        let mut balances = Vec::with_capacity(NUM_VALIDATORS);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            validators.push(Validator {
                pubkey: kp.pk.compress(),
                effective_balance: BALANCE,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..Validator::default()
            });
            balances.push(BALANCE);
        }

        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        let block_hash = ExecutionBlockHash::repeat_byte(0x42);

        let state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 5678,
            genesis_validators_root: Hash256::repeat_byte(0xCC),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 1,
                parent_root: Hash256::repeat_byte(0x02),
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 99,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(1),
                root: Hash256::repeat_byte(0xDD),
            },
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBidGloas {
                block_hash,
                parent_block_hash: ExecutionBlockHash::repeat_byte(0x11),
                parent_block_root: Hash256::repeat_byte(0x22),
                builder_index: 7,
                slot: Slot::new(15),
                value: 1_000_000_000,
                execution_payment: 500_000_000,
                gas_limit: 30_000_000,
                ..Default::default()
            },
            next_withdrawal_index: 12,
            next_withdrawal_validator_index: 2,
            historical_summaries: List::default(),
            deposit_requests_start_index: u64::MAX,
            deposit_balance_to_consume: 100,
            exit_balance_to_consume: 200,
            earliest_exit_epoch: Epoch::new(5),
            consolidation_balance_to_consume: 300,
            earliest_consolidation_epoch: Epoch::new(6),
            pending_deposits: List::default(),
            pending_partial_withdrawals: List::default(),
            pending_consolidations: List::default(),
            proposer_lookahead: Vector::new(vec![
                0u64;
                <E as EthSpec>::ProposerLookaheadSlots::to_usize()
            ])
            .unwrap(),
            builders: List::default(),
            next_withdrawal_builder_index: 3,
            execution_payload_availability: BitVector::from_bytes(
                vec![0xFFu8; <E as EthSpec>::SlotsPerHistoricalRoot::to_usize() / 8].into(),
            )
            .unwrap(),
            builder_pending_payments: Vector::new(vec![
                BuilderPendingPayment::default();
                E::builder_pending_payments_limit()
            ])
            .unwrap(),
            builder_pending_withdrawals: List::default(),
            latest_block_hash: block_hash,
            payload_expected_withdrawals: List::default(),
            ptc_window: FixedVector::default(),
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

    #[test]
    fn upgrade_preserves_versioning_fields() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        assert!(state.as_heze().is_ok());
        assert_eq!(state.genesis_time(), 5678);
        assert_eq!(state.genesis_validators_root(), Hash256::repeat_byte(0xCC));
        assert_eq!(state.slot(), Slot::new(E::slots_per_epoch() * 2));
        let fork = state.fork();
        assert_eq!(fork.previous_version, spec.gloas_fork_version);
        assert_eq!(fork.current_version, spec.heze_fork_version);
        assert_eq!(fork.epoch, Epoch::new(2));
    }

    #[test]
    fn upgrade_preserves_registry() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        assert_eq!(state.validators().len(), NUM_VALIDATORS);
        assert_eq!(state.balances().len(), NUM_VALIDATORS);
        for i in 0..NUM_VALIDATORS {
            assert_eq!(*state.balances().get(i).unwrap(), BALANCE);
        }
    }

    #[test]
    fn upgrade_preserves_electra_fields() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        let heze = state.as_heze().unwrap();
        assert_eq!(heze.deposit_requests_start_index, u64::MAX);
        assert_eq!(heze.deposit_balance_to_consume, 100);
        assert_eq!(heze.exit_balance_to_consume, 200);
        assert_eq!(heze.earliest_exit_epoch, Epoch::new(5));
        assert_eq!(heze.consolidation_balance_to_consume, 300);
        assert_eq!(heze.earliest_consolidation_epoch, Epoch::new(6));
    }

    #[test]
    fn upgrade_preserves_capella_fields() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        let heze = state.as_heze().unwrap();
        assert_eq!(heze.next_withdrawal_index, 12);
        assert_eq!(heze.next_withdrawal_validator_index, 2);
    }

    #[test]
    fn upgrade_preserves_finality() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        assert_eq!(state.finalized_checkpoint().epoch, Epoch::new(1));
        assert_eq!(
            state.finalized_checkpoint().root,
            Hash256::repeat_byte(0xDD)
        );
    }

    #[test]
    fn upgrade_preserves_gloas_fields() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        let heze = state.as_heze().unwrap();
        assert_eq!(heze.next_withdrawal_builder_index, 3);
        assert_eq!(
            heze.latest_block_hash,
            ExecutionBlockHash::repeat_byte(0x42)
        );
        assert_eq!(heze.builders.len(), 0);
        assert_eq!(heze.builder_pending_withdrawals.len(), 0);
        assert_eq!(heze.payload_expected_withdrawals.len(), 0);
    }

    #[test]
    fn upgrade_preserves_bid_fields_and_adds_inclusion_list_bits() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        let heze = state.as_heze().unwrap();
        let bid = &heze.latest_execution_payload_bid;
        assert_eq!(bid.block_hash, ExecutionBlockHash::repeat_byte(0x42));
        assert_eq!(bid.parent_block_hash, ExecutionBlockHash::repeat_byte(0x11));
        assert_eq!(bid.parent_block_root, Hash256::repeat_byte(0x22));
        assert_eq!(bid.builder_index, 7);
        assert_eq!(bid.slot, Slot::new(15));
        assert_eq!(bid.value, 1_000_000_000);
        assert_eq!(bid.execution_payment, 500_000_000);
        assert_eq!(bid.gas_limit, 30_000_000);

        // New Heze field: inclusion_list_bits initialized to all zeros
        let il_bits = &bid.inclusion_list_bits;
        let il_size = <E as EthSpec>::InclusionListCommitteeSize::to_usize();
        for i in 0..il_size {
            assert!(
                !il_bits.get(i).unwrap(),
                "inclusion_list_bits[{i}] should be false after upgrade"
            );
        }
    }

    #[test]
    fn upgrade_preserves_execution_payload_availability() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        let heze = state.as_heze().unwrap();
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        for i in 0..slots_per_hist {
            assert!(
                heze.execution_payload_availability.get(i).unwrap(),
                "availability bit {i} should be preserved"
            );
        }
    }

    #[test]
    fn upgrade_preserves_eth1_deposit_index() {
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();

        assert_eq!(state.eth1_deposit_index(), 99);
    }

    #[test]
    fn upgrade_fails_on_non_gloas_state() {
        // Trying to upgrade a Heze state should fail
        let (mut state, spec) = make_gloas_state();
        upgrade_to_heze(&mut state, &spec).unwrap();
        // Now state is Heze — upgrading again should error
        assert!(upgrade_to_heze(&mut state, &spec).is_err());
    }
}
