use crate::common::decrease_balance;
use crate::per_epoch_processing::{
    Error,
    single_pass::{SinglePassConfig, process_epoch_single_pass},
};
use safe_arith::{SafeArith, SafeArithIter};
use types::{BeaconState, ChainSpec, EthSpec, Unsigned};

/// Process slashings.
pub fn process_slashings<E: EthSpec>(
    state: &mut BeaconState<E>,
    total_balance: u64,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let epoch = state.current_epoch();
    let sum_slashings = state.get_all_slashings().iter().copied().safe_sum()?;

    let adjusted_total_slashing_balance = std::cmp::min(
        sum_slashings.safe_mul(spec.proportional_slashing_multiplier_for_state(state))?,
        total_balance,
    );

    let target_withdrawable_epoch =
        epoch.safe_add(E::EpochsPerSlashingsVector::to_u64().safe_div(2)?)?;
    let indices = state
        .validators()
        .iter()
        .enumerate()
        .filter(|(_, validator)| {
            validator.slashed && target_withdrawable_epoch == validator.withdrawable_epoch
        })
        .map(|(index, validator)| (index, validator.effective_balance))
        .collect::<Vec<(usize, u64)>>();

    for (index, validator_effective_balance) in indices {
        let increment = spec.effective_balance_increment;
        let penalty_numerator = validator_effective_balance
            .safe_div(increment)?
            .safe_mul(adjusted_total_slashing_balance)?;
        let penalty = penalty_numerator
            .safe_div(total_balance)?
            .safe_mul(increment)?;

        decrease_balance(state, index, penalty)?;
    }

    Ok(())
}

pub fn process_slashings_slow<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    process_epoch_single_pass(
        state,
        spec,
        SinglePassConfig {
            slashings: true,
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
    /// Validator effective_balance = max_effective_balance, balance = max_effective_balance.
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

    /// Compute target_withdrawable_epoch for a given state epoch.
    fn target_withdrawable_epoch(state: &BeaconState<E>) -> Epoch {
        let epoch = state.current_epoch();
        epoch + <E as EthSpec>::EpochsPerSlashingsVector::to_u64() / 2
    }

    #[test]
    fn no_penalty_when_no_slashed_validators() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;

        let balance_before = *state.balances().get(0).unwrap();
        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();
        assert_eq!(*state.balances().get(0).unwrap(), balance_before);
    }

    #[test]
    fn penalty_applied_to_slashed_validator_at_target_epoch() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;

        // Slash validator 0 and set withdrawable_epoch to match target
        state.validators_mut().get_mut(0).unwrap().slashed = true;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_withdrawable_epoch(&state);

        // Record some slashings in the vector
        let epoch = state.current_epoch();
        state
            .set_slashings(epoch, spec.max_effective_balance)
            .unwrap();

        let balance_before = *state.balances().get(0).unwrap();
        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();

        // Balance should decrease
        assert!(*state.balances().get(0).unwrap() < balance_before);
    }

    #[test]
    fn no_penalty_if_withdrawable_epoch_mismatch() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;

        // Slash validator 0 but set wrong withdrawable_epoch
        state.validators_mut().get_mut(0).unwrap().slashed = true;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_withdrawable_epoch(&state) + 1;

        let epoch = state.current_epoch();
        state
            .set_slashings(epoch, spec.max_effective_balance)
            .unwrap();

        let balance_before = *state.balances().get(0).unwrap();
        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();

        // No penalty — withdrawable epoch doesn't match
        assert_eq!(*state.balances().get(0).unwrap(), balance_before);
    }

    #[test]
    fn not_slashed_validator_no_penalty() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;

        // Validator 0: not slashed but has matching withdrawable_epoch
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_withdrawable_epoch(&state);

        let epoch = state.current_epoch();
        state
            .set_slashings(epoch, spec.max_effective_balance)
            .unwrap();

        let balance_before = *state.balances().get(0).unwrap();
        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();

        assert_eq!(*state.balances().get(0).unwrap(), balance_before);
    }

    #[test]
    fn penalty_proportional_to_slashing_fraction() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;

        // Slash validator 0
        state.validators_mut().get_mut(0).unwrap().slashed = true;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_withdrawable_epoch(&state);

        let epoch = state.current_epoch();
        let eff_bal = spec.max_effective_balance;

        // Record a small amount of slashings
        state.set_slashings(epoch, eff_bal).unwrap();

        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();
        let penalty_small = eff_bal - *state.balances().get(0).unwrap();

        // Now try with larger slashings sum
        let mut state2 = make_gloas_state(4, &spec);
        state2.validators_mut().get_mut(0).unwrap().slashed = true;
        state2
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_withdrawable_epoch(&state2);

        let epoch2 = state2.current_epoch();
        state2.set_slashings(epoch2, eff_bal * 3).unwrap();

        process_slashings::<E>(&mut state2, total_balance, &spec).unwrap();
        let penalty_large = eff_bal - *state2.balances().get(0).unwrap();

        // Larger slashings sum → larger penalty
        assert!(penalty_large >= penalty_small);
    }

    #[test]
    fn adjusted_slashing_balance_capped_at_total_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;

        state.validators_mut().get_mut(0).unwrap().slashed = true;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_withdrawable_epoch(&state);

        // Set slashings extremely high (will be capped by min(sum*multiplier, total_balance))
        let epoch = state.current_epoch();
        state.set_slashings(epoch, total_balance * 10).unwrap();

        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();

        // Penalty should be: eff_bal / increment * total_balance / total_balance * increment = eff_bal
        // (when adjusted >= total, penalty = full effective balance)
        let penalty = spec.max_effective_balance - *state.balances().get(0).unwrap();
        assert_eq!(penalty, spec.max_effective_balance);
    }

    #[test]
    fn zero_slashings_sum_no_penalty() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;

        // Slashed validator with matching epoch but zero slashings sum
        state.validators_mut().get_mut(0).unwrap().slashed = true;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_withdrawable_epoch(&state);

        let balance_before = *state.balances().get(0).unwrap();
        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();

        // No slashings recorded → sum=0 → adjusted=0 → penalty=0
        assert_eq!(*state.balances().get(0).unwrap(), balance_before);
    }

    #[test]
    fn multiple_slashed_validators_each_penalized() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;
        let target = target_withdrawable_epoch(&state);

        // Slash validators 0 and 1
        for i in 0..2 {
            state.validators_mut().get_mut(i).unwrap().slashed = true;
            state
                .validators_mut()
                .get_mut(i)
                .unwrap()
                .withdrawable_epoch = target;
        }

        let epoch = state.current_epoch();
        state
            .set_slashings(epoch, 2 * spec.max_effective_balance)
            .unwrap();

        let bal0_before = *state.balances().get(0).unwrap();
        let bal1_before = *state.balances().get(1).unwrap();
        let bal2_before = *state.balances().get(2).unwrap();

        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();

        assert!(*state.balances().get(0).unwrap() < bal0_before);
        assert!(*state.balances().get(1).unwrap() < bal1_before);
        // Validator 2 not slashed — unchanged
        assert_eq!(*state.balances().get(2).unwrap(), bal2_before);
    }

    #[test]
    fn penalty_cannot_exceed_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;

        state.validators_mut().get_mut(0).unwrap().slashed = true;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_withdrawable_epoch(&state);

        // Set balance very low
        *state.balances_mut().get_mut(0).unwrap() = 1;

        let epoch = state.current_epoch();
        state.set_slashings(epoch, total_balance * 10).unwrap();

        // Should not panic/underflow — balance goes to 0
        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();
        assert_eq!(*state.balances().get(0).unwrap(), 0);
    }

    #[test]
    fn penalty_uses_effective_balance_not_actual() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let total_balance = 4 * spec.max_effective_balance;
        let target = target_withdrawable_epoch(&state);

        // Two slashed validators, same withdrawable epoch, different effective balances
        state.validators_mut().get_mut(0).unwrap().slashed = true;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target;
        // Validator 0: full effective balance

        state.validators_mut().get_mut(1).unwrap().slashed = true;
        state
            .validators_mut()
            .get_mut(1)
            .unwrap()
            .withdrawable_epoch = target;
        state.validators_mut().get_mut(1).unwrap().effective_balance =
            spec.max_effective_balance / 2;

        let epoch = state.current_epoch();
        state.set_slashings(epoch, total_balance).unwrap();

        process_slashings::<E>(&mut state, total_balance, &spec).unwrap();

        let penalty0 = spec.max_effective_balance - *state.balances().get(0).unwrap();
        let penalty1 = spec.max_effective_balance - *state.balances().get(1).unwrap();

        // Higher effective balance → higher penalty
        assert!(penalty0 > penalty1);
    }
}
