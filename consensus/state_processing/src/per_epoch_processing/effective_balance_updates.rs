use super::errors::EpochProcessingError;
use crate::per_epoch_processing::single_pass::{SinglePassConfig, process_epoch_single_pass};
use safe_arith::SafeArith;
use types::beacon_state::BeaconState;
use types::chain_spec::ChainSpec;
use types::{BeaconStateError, EthSpec};

/// This implementation is now only used in phase0. Later hard forks use single-pass.
pub fn process_effective_balance_updates<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), EpochProcessingError> {
    // Compute new total active balance for the next epoch as a side-effect of iterating the
    // effective balances.
    let next_epoch = state.next_epoch()?;
    let mut new_total_active_balance = 0;

    let hysteresis_increment = spec
        .effective_balance_increment
        .safe_div(spec.hysteresis_quotient)?;
    let downward_threshold = hysteresis_increment.safe_mul(spec.hysteresis_downward_multiplier)?;
    let upward_threshold = hysteresis_increment.safe_mul(spec.hysteresis_upward_multiplier)?;
    let (validators, balances, _) = state.validators_and_balances_and_progressive_balances_mut();
    let mut validators_iter = validators.iter_cow();

    while let Some((index, validator)) = validators_iter.next_cow() {
        let balance = balances
            .get(index)
            .copied()
            .ok_or(BeaconStateError::BalancesOutOfBounds(index))?;

        let new_effective_balance = if balance.safe_add(downward_threshold)?
            < validator.effective_balance
            || validator.effective_balance.safe_add(upward_threshold)? < balance
        {
            std::cmp::min(
                balance.safe_sub(balance.safe_rem(spec.effective_balance_increment)?)?,
                spec.max_effective_balance,
            )
        } else {
            validator.effective_balance
        };

        if validator.is_active_at(next_epoch) {
            new_total_active_balance.safe_add_assign(new_effective_balance)?;
        }

        if new_effective_balance != validator.effective_balance {
            validator.into_mut()?.effective_balance = new_effective_balance;
        }
    }

    state.set_total_active_balance(next_epoch, new_total_active_balance, spec);

    Ok(())
}

/// Only used to test the effective balance part of single-pass in isolation.
pub fn process_effective_balance_updates_slow<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), EpochProcessingError> {
    process_epoch_single_pass(
        state,
        spec,
        SinglePassConfig {
            effective_balance_updates: true,
            ..SinglePassConfig::disable_all()
        },
    )?;
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

    /// Build a Gloas state with `num_validators` active validators.
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
    fn no_change_when_balance_equals_effective() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        // All effective balances should remain unchanged
        for i in 0..4 {
            assert_eq!(
                state.validators().get(i).unwrap().effective_balance,
                spec.max_effective_balance,
            );
        }
    }

    #[test]
    fn no_change_within_hysteresis_downward() {
        let spec = make_spec();
        let mut state = make_gloas_state(1, &spec);

        // downward_threshold = effective_balance_increment / hysteresis_quotient * hysteresis_downward_multiplier
        // = 1_000_000_000 / 4 * 1 = 250_000_000
        // Balance must be < effective_balance - downward_threshold to trigger change
        // So balance = effective_balance - downward_threshold should NOT trigger
        let downward_threshold = spec.effective_balance_increment / spec.hysteresis_quotient
            * spec.hysteresis_downward_multiplier;
        *state.get_balance_mut(0).unwrap() = spec.max_effective_balance - downward_threshold;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        // Should NOT change — at the threshold boundary, not below
        assert_eq!(
            state.validators().get(0).unwrap().effective_balance,
            spec.max_effective_balance,
        );
    }

    #[test]
    fn decreases_when_balance_below_hysteresis() {
        let spec = make_spec();
        let mut state = make_gloas_state(1, &spec);

        let downward_threshold = spec.effective_balance_increment / spec.hysteresis_quotient
            * spec.hysteresis_downward_multiplier;
        // Balance just below the downward threshold
        *state.get_balance_mut(0).unwrap() = spec.max_effective_balance - downward_threshold - 1;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        // Effective balance should decrease by one increment
        assert_eq!(
            state.validators().get(0).unwrap().effective_balance,
            spec.max_effective_balance - spec.effective_balance_increment,
        );
    }

    #[test]
    fn no_change_within_hysteresis_upward() {
        let spec = make_spec();
        let mut state = make_gloas_state(1, &spec);

        // Set effective_balance lower, then set balance just at the upward threshold
        let lower_eff = spec.max_effective_balance - spec.effective_balance_increment;
        state.validators_mut().get_mut(0).unwrap().effective_balance = lower_eff;

        let upward_threshold = spec.effective_balance_increment / spec.hysteresis_quotient
            * spec.hysteresis_upward_multiplier;
        // Balance = effective_balance + upward_threshold should NOT trigger
        *state.get_balance_mut(0).unwrap() = lower_eff + upward_threshold;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(
            state.validators().get(0).unwrap().effective_balance,
            lower_eff,
        );
    }

    #[test]
    fn increases_when_balance_above_hysteresis() {
        let spec = make_spec();
        let mut state = make_gloas_state(1, &spec);

        let lower_eff = spec.max_effective_balance - spec.effective_balance_increment;
        state.validators_mut().get_mut(0).unwrap().effective_balance = lower_eff;

        let upward_threshold = spec.effective_balance_increment / spec.hysteresis_quotient
            * spec.hysteresis_upward_multiplier;
        // Balance just above the upward threshold
        *state.get_balance_mut(0).unwrap() = lower_eff + upward_threshold + 1;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(
            state.validators().get(0).unwrap().effective_balance,
            spec.max_effective_balance,
        );
    }

    #[test]
    fn capped_at_max_effective_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(1, &spec);

        // Set balance way above max
        *state.get_balance_mut(0).unwrap() = spec.max_effective_balance * 2;
        // Set effective_balance low to trigger update
        state.validators_mut().get_mut(0).unwrap().effective_balance = 0;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(
            state.validators().get(0).unwrap().effective_balance,
            spec.max_effective_balance,
        );
    }

    #[test]
    fn rounds_down_to_increment() {
        let spec = make_spec();
        let mut state = make_gloas_state(1, &spec);

        // Set balance to 1.5 increments and effective_balance to 0 to force recalc
        let balance = spec.effective_balance_increment + spec.effective_balance_increment / 2;
        *state.get_balance_mut(0).unwrap() = balance;
        state.validators_mut().get_mut(0).unwrap().effective_balance = 0;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        // Should round down to 1 increment
        assert_eq!(
            state.validators().get(0).unwrap().effective_balance,
            spec.effective_balance_increment,
        );
    }

    #[test]
    fn total_active_balance_updated() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        // Drop one validator's balance to trigger effective balance decrease
        let downward_threshold = spec.effective_balance_increment / spec.hysteresis_quotient
            * spec.hysteresis_downward_multiplier;
        *state.get_balance_mut(0).unwrap() = spec.max_effective_balance - downward_threshold - 1;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        let next_epoch = state.next_epoch().unwrap();
        let expected = 3 * spec.max_effective_balance
            + (spec.max_effective_balance - spec.effective_balance_increment);
        assert_eq!(
            state.get_total_active_balance_at_epoch(next_epoch).unwrap(),
            expected
        );
    }

    #[test]
    fn inactive_validator_excluded_from_total_active_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        // Make validator 0 inactive (not yet activated)
        let next_epoch = state.next_epoch().unwrap();
        state.validators_mut().get_mut(0).unwrap().activation_epoch = next_epoch + 1;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        // Only 3 validators should be in total active balance
        assert_eq!(
            state.get_total_active_balance_at_epoch(next_epoch).unwrap(),
            3 * spec.max_effective_balance,
        );
    }

    #[test]
    fn zero_balance_validator() {
        let spec = make_spec();
        let mut state = make_gloas_state(1, &spec);

        *state.get_balance_mut(0).unwrap() = 0;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(state.validators().get(0).unwrap().effective_balance, 0,);
    }

    #[test]
    fn multiple_validators_mixed_changes() {
        let spec = make_spec();
        let mut state = make_gloas_state(3, &spec);
        let downward_threshold = spec.effective_balance_increment / spec.hysteresis_quotient
            * spec.hysteresis_downward_multiplier;

        // Validator 0: stays the same (balance = effective)
        // Validator 1: decreases (balance dropped below threshold)
        *state.get_balance_mut(1).unwrap() = spec.max_effective_balance - downward_threshold - 1;
        // Validator 2: effective is 0, balance is 5 increments → should update to 5 increments
        state.validators_mut().get_mut(2).unwrap().effective_balance = 0;
        *state.get_balance_mut(2).unwrap() = 5 * spec.effective_balance_increment;

        process_effective_balance_updates::<E>(&mut state, &spec).unwrap();

        assert_eq!(
            state.validators().get(0).unwrap().effective_balance,
            spec.max_effective_balance,
        );
        assert_eq!(
            state.validators().get(1).unwrap().effective_balance,
            spec.max_effective_balance - spec.effective_balance_increment,
        );
        assert_eq!(
            state.validators().get(2).unwrap().effective_balance,
            5 * spec.effective_balance_increment,
        );
    }
}
