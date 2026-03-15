use crate::per_epoch_processing::single_pass::{SinglePassConfig, process_epoch_single_pass};
use crate::{common::initiate_validator_exit, per_epoch_processing::Error};
use safe_arith::SafeArith;
use types::{BeaconState, ChainSpec, EthSpec, Validator};

/// Performs a validator registry update, if required.
///
/// NOTE: unchanged in Altair
pub fn process_registry_updates<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    // Process activation eligibility and ejections.
    // Collect eligible and exiting validators (we need to avoid mutating the state while iterating).
    // We assume it's safe to re-order the change in eligibility and `initiate_validator_exit`.
    // Rest assured exiting validators will still be exited in the same order as in the spec.
    let current_epoch = state.current_epoch();
    let is_ejectable = |validator: &Validator| {
        validator.is_active_at(current_epoch)
            && validator.effective_balance <= spec.ejection_balance
    };
    let fork_name = state.fork_name_unchecked();
    let indices_to_update: Vec<_> = state
        .validators()
        .iter()
        .enumerate()
        .filter(|(_, validator)| {
            validator.is_eligible_for_activation_queue(spec, fork_name) || is_ejectable(validator)
        })
        .map(|(idx, _)| idx)
        .collect();

    for index in indices_to_update {
        let validator = state.get_validator_mut(index)?;
        if validator.is_eligible_for_activation_queue(spec, fork_name) {
            validator.activation_eligibility_epoch = current_epoch.safe_add(1)?;
        }
        if is_ejectable(validator) {
            initiate_validator_exit(state, index, spec)?;
        }
    }

    // Queue validators eligible for activation and not dequeued for activation prior to finalized epoch
    // Dequeue validators for activation up to churn limit
    let churn_limit = state.get_activation_churn_limit(spec)? as usize;

    let epoch_cache = state.epoch_cache();
    let activation_queue = epoch_cache
        .activation_queue()?
        .get_validators_eligible_for_activation(state.finalized_checkpoint().epoch, churn_limit);

    let delayed_activation_epoch = state.compute_activation_exit_epoch(current_epoch, spec)?;
    for index in activation_queue {
        state.get_validator_mut(index)?.activation_epoch = delayed_activation_epoch;
    }

    Ok(())
}

/// Process registry updates using the direct (non-single-pass) implementation.
pub fn process_registry_updates_slow<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    process_epoch_single_pass(
        state,
        spec,
        SinglePassConfig {
            registry_updates: true,
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

    /// Build a Gloas state with `num_validators` active validators at a reasonable epoch.
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

    /// Initialize caches needed by process_registry_updates.
    fn init_caches(
        state: &mut BeaconState<E>,
        activation_queue: ActivationQueue,
        spec: &ChainSpec,
    ) {
        // Committee cache is needed for churn limit calculation
        state
            .build_committee_cache(RelativeEpoch::Current, spec)
            .unwrap();

        // EpochCache is needed for activation queue
        let current_epoch = state.current_epoch();
        let effective_balances: Vec<u64> = state
            .validators()
            .iter()
            .map(|v| v.effective_balance)
            .collect();
        let base_rewards = vec![0; 33];
        let key = EpochCacheKey {
            epoch: current_epoch,
            decision_block_root: Hash256::zero(),
        };
        *state.epoch_cache_mut() = EpochCache::new(
            key,
            effective_balances,
            base_rewards,
            activation_queue,
            spec,
        );
    }

    // ── Activation eligibility tests ──

    #[test]
    fn sets_activation_eligibility_for_new_deposit() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        // Validator 0: eligible for activation queue (far_future eligibility, sufficient balance)
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .activation_eligibility_epoch = spec.far_future_epoch;

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        assert_eq!(
            state
                .validators()
                .get(0)
                .unwrap()
                .activation_eligibility_epoch,
            current_epoch + 1
        );
    }

    #[test]
    fn no_eligibility_if_balance_too_low() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .activation_eligibility_epoch = spec.far_future_epoch;
        state.validators_mut().get_mut(0).unwrap().effective_balance =
            spec.min_activation_balance - 1;

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        assert_eq!(
            state
                .validators()
                .get(0)
                .unwrap()
                .activation_eligibility_epoch,
            spec.far_future_epoch
        );
    }

    #[test]
    fn already_eligible_not_touched() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .activation_eligibility_epoch = Epoch::new(3);

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        assert_eq!(
            state
                .validators()
                .get(0)
                .unwrap()
                .activation_eligibility_epoch,
            Epoch::new(3)
        );
    }

    #[test]
    fn eligibility_at_exact_min_activation_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .activation_eligibility_epoch = spec.far_future_epoch;
        state.validators_mut().get_mut(0).unwrap().effective_balance = spec.min_activation_balance;

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        assert_eq!(
            state
                .validators()
                .get(0)
                .unwrap()
                .activation_eligibility_epoch,
            current_epoch + 1
        );
    }

    // ── Ejection tests ──

    #[test]
    fn ejects_validator_at_ejection_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        state.validators_mut().get_mut(1).unwrap().effective_balance = spec.ejection_balance;

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        assert_ne!(
            state.validators().get(1).unwrap().exit_epoch,
            spec.far_future_epoch
        );
    }

    #[test]
    fn no_ejection_above_ejection_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        state.validators_mut().get_mut(1).unwrap().effective_balance = spec.ejection_balance + 1;

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        assert_eq!(
            state.validators().get(1).unwrap().exit_epoch,
            spec.far_future_epoch
        );
    }

    #[test]
    fn inactive_validator_not_ejected_even_if_low_balance() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        state.validators_mut().get_mut(1).unwrap().exit_epoch = current_epoch - 1;
        state.validators_mut().get_mut(1).unwrap().effective_balance = spec.ejection_balance;

        init_caches(&mut state, ActivationQueue::default(), &spec);

        let exit_before = state.validators().get(1).unwrap().exit_epoch;
        process_registry_updates(&mut state, &spec).unwrap();

        assert_eq!(state.validators().get(1).unwrap().exit_epoch, exit_before);
    }

    #[test]
    fn ejected_validator_gets_withdrawable_epoch() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        state.validators_mut().get_mut(0).unwrap().effective_balance = spec.ejection_balance;

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        let v = state.validators().get(0).unwrap();
        assert_ne!(v.exit_epoch, spec.far_future_epoch);
        assert_ne!(v.withdrawable_epoch, spec.far_future_epoch);
        assert!(v.withdrawable_epoch > v.exit_epoch);
    }

    // ── Activation queue tests ──

    #[test]
    fn activates_validator_from_queue() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        state
            .validators_mut()
            .get_mut(2)
            .unwrap()
            .activation_eligibility_epoch = Epoch::new(3);
        state.validators_mut().get_mut(2).unwrap().activation_epoch = spec.far_future_epoch;

        let mut queue = ActivationQueue::default();
        queue.add_if_could_be_eligible_for_activation(
            2,
            state.validators().get(2).unwrap(),
            current_epoch + 1,
            &spec,
        );

        init_caches(&mut state, queue, &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        let expected_activation = spec.compute_activation_exit_epoch(current_epoch).unwrap();
        assert_eq!(
            state.validators().get(2).unwrap().activation_epoch,
            expected_activation
        );
    }

    #[test]
    fn does_not_activate_if_eligibility_after_finalized() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        // Eligibility epoch 6 > finalized epoch 5 → not eligible for activation
        state
            .validators_mut()
            .get_mut(2)
            .unwrap()
            .activation_eligibility_epoch = Epoch::new(6);
        state.validators_mut().get_mut(2).unwrap().activation_epoch = spec.far_future_epoch;

        let mut queue = ActivationQueue::default();
        queue.add_if_could_be_eligible_for_activation(
            2,
            state.validators().get(2).unwrap(),
            current_epoch + 1,
            &spec,
        );

        init_caches(&mut state, queue, &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        assert_eq!(
            state.validators().get(2).unwrap().activation_epoch,
            spec.far_future_epoch
        );
    }

    #[test]
    fn activation_respects_churn_limit() {
        let spec = make_spec();
        // Use enough validators to get a meaningful churn limit
        let total = 8;
        let mut state = make_gloas_state(total, &spec);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        let churn_limit = state.get_activation_churn_limit(&spec).unwrap() as usize;
        let current_epoch = state.current_epoch();

        // Set up more pending activations than churn_limit allows
        let num_pending = churn_limit + 2;

        let mut queue = ActivationQueue::default();
        for i in 0..num_pending {
            state
                .validators_mut()
                .get_mut(i)
                .unwrap()
                .activation_eligibility_epoch = Epoch::new(3);
            state.validators_mut().get_mut(i).unwrap().activation_epoch = spec.far_future_epoch;

            queue.add_if_could_be_eligible_for_activation(
                i,
                state.validators().get(i).unwrap(),
                current_epoch + 1,
                &spec,
            );
        }

        init_caches(&mut state, queue, &spec);

        let churn_limit = state.get_activation_churn_limit(&spec).unwrap() as usize;
        process_registry_updates(&mut state, &spec).unwrap();

        let activated_count = (0..num_pending)
            .filter(|&i| {
                state.validators().get(i).unwrap().activation_epoch != spec.far_future_epoch
            })
            .count();

        assert_eq!(activated_count, churn_limit);
    }

    #[test]
    fn activation_epoch_is_delayed() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .activation_eligibility_epoch = Epoch::new(3);
        state.validators_mut().get_mut(0).unwrap().activation_epoch = spec.far_future_epoch;

        let mut queue = ActivationQueue::default();
        queue.add_if_could_be_eligible_for_activation(
            0,
            state.validators().get(0).unwrap(),
            current_epoch + 1,
            &spec,
        );

        init_caches(&mut state, queue, &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        let expected = spec.compute_activation_exit_epoch(current_epoch).unwrap();
        assert_eq!(
            state.validators().get(0).unwrap().activation_epoch,
            expected
        );
        assert!(expected > current_epoch);
    }

    // ── Combined tests ──

    #[test]
    fn eligibility_and_ejection_in_same_pass() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        // Validator 0: eligible for activation queue
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .activation_eligibility_epoch = spec.far_future_epoch;

        // Validator 1: should be ejected
        state.validators_mut().get_mut(1).unwrap().effective_balance = spec.ejection_balance;

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        assert_eq!(
            state
                .validators()
                .get(0)
                .unwrap()
                .activation_eligibility_epoch,
            current_epoch + 1
        );
        assert_ne!(
            state.validators().get(1).unwrap().exit_epoch,
            spec.far_future_epoch
        );
    }

    #[test]
    fn multiple_ejections_same_epoch() {
        let spec = make_spec();
        let mut state = make_gloas_state(8, &spec);

        for i in 0..4 {
            state.validators_mut().get_mut(i).unwrap().effective_balance = spec.ejection_balance;
        }

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        for i in 0..4 {
            assert_ne!(
                state.validators().get(i).unwrap().exit_epoch,
                spec.far_future_epoch,
                "validator {} should be ejected",
                i
            );
        }
        for i in 4..8 {
            assert_eq!(
                state.validators().get(i).unwrap().exit_epoch,
                spec.far_future_epoch,
                "validator {} should not be ejected",
                i
            );
        }
    }

    #[test]
    fn no_changes_when_all_validators_healthy() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        init_caches(&mut state, ActivationQueue::default(), &spec);

        let validators_before: Vec<_> = state
            .validators()
            .iter()
            .map(|v| {
                (
                    v.activation_eligibility_epoch,
                    v.exit_epoch,
                    v.activation_epoch,
                )
            })
            .collect();

        process_registry_updates(&mut state, &spec).unwrap();

        for (i, v) in state.validators().iter().enumerate() {
            let (elig, exit, act) = validators_before[i];
            assert_eq!(v.activation_eligibility_epoch, elig);
            assert_eq!(v.exit_epoch, exit);
            assert_eq!(v.activation_epoch, act);
        }
    }

    #[test]
    fn multiple_eligibility_in_same_epoch() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        for i in 0..3 {
            state
                .validators_mut()
                .get_mut(i)
                .unwrap()
                .activation_eligibility_epoch = spec.far_future_epoch;
        }

        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        for i in 0..3 {
            assert_eq!(
                state
                    .validators()
                    .get(i)
                    .unwrap()
                    .activation_eligibility_epoch,
                current_epoch + 1
            );
        }
    }

    #[test]
    fn activation_at_finalized_boundary() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);
        let current_epoch = state.current_epoch();

        // Eligibility epoch exactly equals finalized epoch (5) → should activate
        state
            .validators_mut()
            .get_mut(0)
            .unwrap()
            .activation_eligibility_epoch = Epoch::new(5);
        state.validators_mut().get_mut(0).unwrap().activation_epoch = spec.far_future_epoch;

        let mut queue = ActivationQueue::default();
        queue.add_if_could_be_eligible_for_activation(
            0,
            state.validators().get(0).unwrap(),
            current_epoch + 1,
            &spec,
        );

        init_caches(&mut state, queue, &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        let expected = spec.compute_activation_exit_epoch(current_epoch).unwrap();
        assert_eq!(
            state.validators().get(0).unwrap().activation_epoch,
            expected
        );
    }

    #[test]
    fn empty_activation_queue_no_activations() {
        let spec = make_spec();
        let mut state = make_gloas_state(4, &spec);

        // All validators already activated (activation_epoch = 0)
        init_caches(&mut state, ActivationQueue::default(), &spec);
        process_registry_updates(&mut state, &spec).unwrap();

        // No validator should change activation epoch
        for i in 0..4 {
            assert_eq!(
                state.validators().get(i).unwrap().activation_epoch,
                Epoch::new(0)
            );
        }
    }
}
