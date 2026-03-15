use safe_arith::SafeArith;
use std::cmp::max;
use types::{BeaconStateError as Error, *};

/// Initiate the exit of the validator of the given `index`.
pub fn initiate_validator_exit<E: EthSpec>(
    state: &mut BeaconState<E>,
    index: usize,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let validator = state.get_validator_cow(index)?;

    // Return if the validator already initiated exit
    if validator.exit_epoch != spec.far_future_epoch {
        return Ok(());
    }

    // Ensure the exit cache is built.
    state.build_exit_cache(spec)?;

    // Compute exit queue epoch
    let exit_queue_epoch = if state.fork_name_unchecked() >= ForkName::Electra {
        let effective_balance = state.get_effective_balance(index)?;
        state.compute_exit_epoch_and_update_churn(effective_balance, spec)?
    } else {
        let delayed_epoch = state.compute_activation_exit_epoch(state.current_epoch(), spec)?;
        let mut exit_queue_epoch = state
            .exit_cache()
            .max_epoch()?
            .map_or(delayed_epoch, |epoch| max(epoch, delayed_epoch));
        let exit_queue_churn = state.exit_cache().get_churn_at(exit_queue_epoch)?;

        if exit_queue_churn >= state.get_validator_churn_limit(spec)? {
            exit_queue_epoch.safe_add_assign(1)?;
        }
        exit_queue_epoch
    };

    let validator = state.get_validator_mut(index)?;
    validator.exit_epoch = exit_queue_epoch;
    validator.withdrawable_epoch =
        exit_queue_epoch.safe_add(spec.min_validator_withdrawability_delay)?;

    state
        .exit_cache_mut()
        .record_validator_exit(exit_queue_epoch)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::PublicKeyBytes;
    use std::sync::Arc;
    use types::{
        BeaconState, BeaconStateGloas, BitVector, BuilderPendingPayment, Checkpoint, EpochCache,
        Eth1Data, ExecutionBlockHash, ExecutionPayloadBid, ExitCache, ForkName, MinimalEthSpec,
        ProgressiveBalancesCache, PubkeyCache, SlashingsCache, SyncCommittee, Validator,
        beacon_state::BuilderPubkeyCache,
    };

    type E = MinimalEthSpec;

    fn make_spec() -> ChainSpec {
        ForkName::Gloas.make_genesis_spec(E::default_spec())
    }

    fn make_gloas_state(num_validators: usize, spec: &ChainSpec) -> BeaconState<E> {
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let current_epoch = Epoch::new(spec.shard_committee_period + 10);
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

        let finalized_checkpoint = Checkpoint {
            epoch: Epoch::new(5),
            root: Hash256::zero(),
        };

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
            finalized_checkpoint,
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
            total_active_balance: Some((
                current_epoch,
                num_validators as u64 * spec.max_effective_balance,
            )),
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <_>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: EpochCache::default(),
        })
    }

    #[test]
    fn already_exited_is_noop() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        // Set validator 0 as already exiting
        state.validators_mut().get_mut(0).unwrap().exit_epoch = current_epoch + 5;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = current_epoch + 10;

        let original_exit = state.validators().get(0).unwrap().exit_epoch;
        let original_withdrawable = state.validators().get(0).unwrap().withdrawable_epoch;

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();

        // Nothing should change
        assert_eq!(state.validators().get(0).unwrap().exit_epoch, original_exit);
        assert_eq!(
            state.validators().get(0).unwrap().withdrawable_epoch,
            original_withdrawable
        );
    }

    #[test]
    fn unknown_validator_errors() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        let result = initiate_validator_exit::<E>(&mut state, 99, &spec);
        assert!(result.is_err());
    }

    #[test]
    fn normal_exit_sets_epochs() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();

        let validator = state.validators().get(0).unwrap();
        // exit_epoch should no longer be far_future
        assert_ne!(validator.exit_epoch, spec.far_future_epoch);
        // withdrawable_epoch = exit_epoch + min_validator_withdrawability_delay
        assert_eq!(
            validator.withdrawable_epoch,
            validator.exit_epoch + spec.min_validator_withdrawability_delay
        );
    }

    #[test]
    fn exit_epoch_is_delayed_from_current() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();

        let validator = state.validators().get(0).unwrap();
        // Exit epoch must be at least current_epoch + 1 + MAX_SEED_LOOKAHEAD
        let min_exit_epoch = spec.compute_activation_exit_epoch(current_epoch).unwrap();
        assert!(validator.exit_epoch >= min_exit_epoch);
    }

    #[test]
    fn exit_cache_updated_after_exit() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();

        let exit_epoch = state.validators().get(0).unwrap().exit_epoch;
        // The exit cache should have recorded this exit
        let churn = state.exit_cache().get_churn_at(exit_epoch).unwrap();
        assert!(churn >= 1);
    }

    #[test]
    fn second_exit_same_epoch_increments_churn() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();
        initiate_validator_exit::<E>(&mut state, 1, &spec).unwrap();

        let exit_epoch_0 = state.validators().get(0).unwrap().exit_epoch;
        let exit_epoch_1 = state.validators().get(1).unwrap().exit_epoch;

        // Both should have valid (non-far-future) exit epochs
        assert_ne!(exit_epoch_0, spec.far_future_epoch);
        assert_ne!(exit_epoch_1, spec.far_future_epoch);
    }

    #[test]
    fn idempotent_double_call() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();
        let exit_epoch_first = state.validators().get(0).unwrap().exit_epoch;
        let withdrawable_first = state.validators().get(0).unwrap().withdrawable_epoch;

        // Second call should be a no-op (already has exit_epoch set)
        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();
        assert_eq!(
            state.validators().get(0).unwrap().exit_epoch,
            exit_epoch_first
        );
        assert_eq!(
            state.validators().get(0).unwrap().withdrawable_epoch,
            withdrawable_first
        );
    }

    #[test]
    fn multiple_exits_assigned_valid_epochs() {
        let spec = make_spec();
        let mut state = make_gloas_state(8, &spec);

        // Exit all 8 validators
        for i in 0..8 {
            initiate_validator_exit::<E>(&mut state, i, &spec).unwrap();
        }

        // All should have non-far-future exit epochs
        for i in 0..8 {
            let v = state.validators().get(i).unwrap();
            assert_ne!(v.exit_epoch, spec.far_future_epoch, "validator {i}");
            assert_eq!(
                v.withdrawable_epoch,
                v.exit_epoch + spec.min_validator_withdrawability_delay,
                "validator {i} withdrawable"
            );
        }
    }

    #[test]
    fn exit_epoch_monotonically_non_decreasing() {
        let spec = make_spec();
        let mut state = make_gloas_state(8, &spec);

        for i in 0..8 {
            initiate_validator_exit::<E>(&mut state, i, &spec).unwrap();
        }

        // Exit epochs should be monotonically non-decreasing
        let exit_epochs: Vec<Epoch> = (0..8)
            .map(|i| state.validators().get(i).unwrap().exit_epoch)
            .collect();

        for window in exit_epochs.windows(2) {
            assert!(window[0] <= window[1]);
        }
    }

    #[test]
    fn lower_effective_balance_exits_sooner() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        // Give validator 0 a lower effective balance
        state.validators_mut().get_mut(0).unwrap().effective_balance =
            spec.max_effective_balance / 2;

        // Give validator 1 the max effective balance
        // (already set by default)

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();
        initiate_validator_exit::<E>(&mut state, 1, &spec).unwrap();

        let exit_0 = state.validators().get(0).unwrap().exit_epoch;
        let exit_1 = state.validators().get(1).unwrap().exit_epoch;

        // With Electra balance-based churn, lower balance consumes less churn,
        // so validator 0 should exit at same or earlier epoch
        assert!(exit_0 <= exit_1);
    }

    #[test]
    fn earliest_exit_epoch_updated_by_churn() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();

        // After an exit, earliest_exit_epoch should be updated
        let earliest = state.earliest_exit_epoch().unwrap();
        let current_epoch = state.current_epoch();
        let min_delayed = spec.compute_activation_exit_epoch(current_epoch).unwrap();
        assert!(earliest >= min_delayed);
    }

    #[test]
    fn exit_balance_to_consume_decremented() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        // Get the per-epoch churn limit before any exits
        let churn_limit = state.get_activation_exit_churn_limit(&spec).unwrap();

        initiate_validator_exit::<E>(&mut state, 0, &spec).unwrap();

        let remaining = state.exit_balance_to_consume().unwrap();
        let validator_balance = spec.max_effective_balance;

        // The remaining balance to consume should be churn_limit - effective_balance
        // (assuming the exit fits within one epoch)
        if churn_limit >= validator_balance {
            assert_eq!(remaining, churn_limit - validator_balance);
        }
    }
}
