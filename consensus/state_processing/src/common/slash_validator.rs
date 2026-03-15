use crate::common::update_progressive_balances_cache::update_progressive_balances_on_slashing;
use crate::{
    ConsensusContext,
    common::{decrease_balance, increase_balance, initiate_validator_exit},
    per_block_processing::errors::BlockProcessingError,
};
use safe_arith::SafeArith;
use std::cmp;
use types::{
    consts::altair::{PROPOSER_WEIGHT, WEIGHT_DENOMINATOR},
    *,
};

/// Slash the validator with index `slashed_index`.
pub fn slash_validator<E: EthSpec>(
    state: &mut BeaconState<E>,
    slashed_index: usize,
    opt_whistleblower_index: Option<usize>,
    ctxt: &mut ConsensusContext<E>,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    let epoch = state.current_epoch();
    let latest_block_slot = state.latest_block_header().slot;

    initiate_validator_exit(state, slashed_index, spec)?;

    let validator = state.get_validator_mut(slashed_index)?;
    validator.slashed = true;
    validator.withdrawable_epoch = cmp::max(
        validator.withdrawable_epoch,
        epoch.safe_add(<E as EthSpec>::EpochsPerSlashingsVector::to_u64())?,
    );
    let validator_effective_balance = validator.effective_balance;
    state.set_slashings(
        epoch,
        state
            .get_slashings(epoch)?
            .safe_add(validator_effective_balance)?,
    )?;

    decrease_balance(
        state,
        slashed_index,
        validator_effective_balance
            .safe_div(spec.min_slashing_penalty_quotient_for_state(state))?,
    )?;

    update_progressive_balances_on_slashing(state, slashed_index, validator_effective_balance)?;
    state
        .slashings_cache_mut()
        .record_validator_slashing(latest_block_slot, slashed_index)?;

    // Apply proposer and whistleblower rewards
    let proposer_index = ctxt.get_proposer_index(state, spec)? as usize;
    let whistleblower_index = opt_whistleblower_index.unwrap_or(proposer_index);
    let whistleblower_reward = validator_effective_balance
        .safe_div(spec.whistleblower_reward_quotient_for_state(state))?;
    let proposer_reward = if state.fork_name_unchecked().altair_enabled() {
        whistleblower_reward
            .safe_mul(PROPOSER_WEIGHT)?
            .safe_div(WEIGHT_DENOMINATOR)?
    } else {
        whistleblower_reward.safe_div(spec.proposer_reward_quotient)?
    };

    // Ensure the whistleblower index is in the validator registry.
    if state.validators().get(whistleblower_index).is_none() {
        return Err(BeaconStateError::UnknownValidator(whistleblower_index).into());
    }

    increase_balance(state, proposer_index, proposer_reward)?;
    increase_balance(
        state,
        whistleblower_index,
        whistleblower_reward.safe_sub(proposer_reward)?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConsensusContext;
    use bls::PublicKeyBytes;
    use std::sync::Arc;
    use types::{
        BeaconState, BeaconStateGloas, BitVector, BuilderPendingPayment, Checkpoint, EpochCache,
        Eth1Data, ExecutionBlockHash, ExecutionPayloadBid, ExitCache, ForkName, MinimalEthSpec,
        ParticipationFlags, ProgressiveBalancesCache, PubkeyCache, SlashingsCache, SyncCommittee,
        Validator, beacon_state::BuilderPubkeyCache,
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
        let mut participation = Vec::with_capacity(num_validators);
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
            participation.push(ParticipationFlags::default());
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
            previous_epoch_participation: List::new(participation.clone()).unwrap(),
            current_epoch_participation: List::new(participation).unwrap(),
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

    fn make_context(state: &BeaconState<E>, proposer_index: u64) -> ConsensusContext<E> {
        ConsensusContext::new(state.slot()).set_proposer_index(proposer_index)
    }

    fn init_caches(state: &mut BeaconState<E>, spec: &ChainSpec) {
        crate::common::update_progressive_balances_cache::initialize_progressive_balances_cache(
            state, spec,
        )
        .unwrap();
        state.build_slashings_cache().unwrap();
    }

    // --- Basic slashing behavior ---

    #[test]
    fn slash_marks_validator_slashed() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let mut ctxt = make_context(&state, 1);

        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();

        assert!(state.validators().get(0).unwrap().slashed);
    }

    #[test]
    fn slash_initiates_exit() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let mut ctxt = make_context(&state, 1);

        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();

        let v = state.validators().get(0).unwrap();
        assert_ne!(v.exit_epoch, spec.far_future_epoch);
    }

    #[test]
    fn slash_sets_withdrawable_epoch_minimum() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let current_epoch = state.current_epoch();
        let mut ctxt = make_context(&state, 1);

        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();

        let v = state.validators().get(0).unwrap();
        // withdrawable_epoch >= current_epoch + EPOCHS_PER_SLASHINGS_VECTOR
        let min_withdrawable = current_epoch + <E as EthSpec>::EpochsPerSlashingsVector::to_u64();
        assert!(v.withdrawable_epoch >= min_withdrawable);
    }

    #[test]
    fn slash_updates_slashings_sum() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let epoch = state.current_epoch();
        let mut ctxt = make_context(&state, 1);

        let before = state.get_slashings(epoch).unwrap();
        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();
        let after = state.get_slashings(epoch).unwrap();

        assert_eq!(after, before + spec.max_effective_balance);
    }

    #[test]
    fn slash_decreases_slashed_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let mut ctxt = make_context(&state, 1);

        let before = *state.balances().get(0).unwrap();
        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();
        let after = *state.balances().get(0).unwrap();

        // Balance should decrease by effective_balance / min_slashing_penalty_quotient
        let penalty =
            spec.max_effective_balance / spec.min_slashing_penalty_quotient_for_state(&state);
        assert_eq!(after, before - penalty);
    }

    // --- Proposer and whistleblower rewards ---

    #[test]
    fn proposer_receives_reward_when_no_whistleblower() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let proposer_idx = 1;
        let mut ctxt = make_context(&state, proposer_idx as u64);

        let before = *state.balances().get(proposer_idx).unwrap();
        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();
        let after = *state.balances().get(proposer_idx).unwrap();

        // When no whistleblower, proposer gets the full whistleblower reward
        let whistleblower_reward =
            spec.max_effective_balance / spec.whistleblower_reward_quotient_for_state(&state);
        assert_eq!(after, before + whistleblower_reward);
    }

    #[test]
    fn separate_whistleblower_splits_reward() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let proposer_idx = 1usize;
        let whistleblower_idx = 2usize;
        let mut ctxt = make_context(&state, proposer_idx as u64);

        let proposer_before = *state.balances().get(proposer_idx).unwrap();
        let wb_before = *state.balances().get(whistleblower_idx).unwrap();

        slash_validator::<E>(&mut state, 0, Some(whistleblower_idx), &mut ctxt, &spec).unwrap();

        let proposer_after = *state.balances().get(proposer_idx).unwrap();
        let wb_after = *state.balances().get(whistleblower_idx).unwrap();

        let whistleblower_reward =
            spec.max_effective_balance / spec.whistleblower_reward_quotient_for_state(&state);
        // Altair+ proposer reward formula
        let proposer_reward = whistleblower_reward * PROPOSER_WEIGHT / WEIGHT_DENOMINATOR;

        assert_eq!(proposer_after, proposer_before + proposer_reward);
        assert_eq!(wb_after, wb_before + whistleblower_reward - proposer_reward);
    }

    #[test]
    fn unknown_whistleblower_errors() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let mut ctxt = make_context(&state, 1);

        let result = slash_validator::<E>(&mut state, 0, Some(99), &mut ctxt, &spec);
        assert!(result.is_err());
    }

    // --- Edge cases ---

    #[test]
    fn slash_already_exiting_validator() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let current_epoch = state.current_epoch();
        let mut ctxt = make_context(&state, 1);

        // Set validator as already exiting
        let exit_epoch = current_epoch + 5;
        state.validators_mut().get_mut(0).unwrap().exit_epoch = exit_epoch;
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = exit_epoch + 256;

        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();

        let v = state.validators().get(0).unwrap();
        assert!(v.slashed);
        // Exit epoch unchanged (initiate_validator_exit is a no-op for already-exiting)
        assert_eq!(v.exit_epoch, exit_epoch);
        // But withdrawable_epoch may be extended
        let min_withdrawable = current_epoch + <E as EthSpec>::EpochsPerSlashingsVector::to_u64();
        assert!(v.withdrawable_epoch >= min_withdrawable);
    }

    #[test]
    fn slash_proposer_slashes_self() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        // Proposer slashes themselves (index 0 is both proposer and slashed)
        let mut ctxt = make_context(&state, 0);

        let before = *state.balances().get(0).unwrap();
        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();
        let after = *state.balances().get(0).unwrap();

        assert!(state.validators().get(0).unwrap().slashed);
        // Balance = before - penalty + whistleblower_reward (since proposer == whistleblower)
        let penalty =
            spec.max_effective_balance / spec.min_slashing_penalty_quotient_for_state(&state);
        let whistleblower_reward =
            spec.max_effective_balance / spec.whistleblower_reward_quotient_for_state(&state);
        assert_eq!(after, before - penalty + whistleblower_reward);
    }

    #[test]
    fn double_slashing_accumulates() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        init_caches(&mut state, &spec);
        let epoch = state.current_epoch();
        let mut ctxt = make_context(&state, 2);

        slash_validator::<E>(&mut state, 0, None, &mut ctxt, &spec).unwrap();
        slash_validator::<E>(&mut state, 1, None, &mut ctxt, &spec).unwrap();

        let slashings = state.get_slashings(epoch).unwrap();
        assert_eq!(slashings, 2 * spec.max_effective_balance);
    }
}
