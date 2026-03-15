use bls::Signature;
use itertools::Itertools;
use safe_arith::SafeArith;
use std::mem;
use types::{
    BeaconState, BeaconStateElectra, BeaconStateError as Error, BuilderPubkeyCache, ChainSpec,
    Epoch, EpochCache, EthSpec, Fork, PendingDeposit,
};

/// Transform a `Deneb` state into an `Electra` state.
pub fn upgrade_to_electra<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let epoch = pre_state.current_epoch();

    let activation_exit_epoch = spec.compute_activation_exit_epoch(epoch)?;
    let earliest_exit_epoch = pre_state
        .validators()
        .iter()
        .filter(|v| v.exit_epoch != spec.far_future_epoch)
        .map(|v| v.exit_epoch)
        .max()
        .unwrap_or(activation_exit_epoch)
        .max(activation_exit_epoch)
        .safe_add(1)?;

    // The total active balance cache must be built before the consolidation churn limit
    // is calculated.
    pre_state.build_total_active_balance_cache(spec)?;
    let earliest_consolidation_epoch = spec.compute_activation_exit_epoch(epoch)?;

    let mut post = upgrade_state_to_electra(
        pre_state,
        earliest_exit_epoch,
        earliest_consolidation_epoch,
        spec,
    )?;

    *post.exit_balance_to_consume_mut()? = post.get_activation_exit_churn_limit(spec)?;
    *post.consolidation_balance_to_consume_mut()? = post.get_consolidation_churn_limit(spec)?;

    // Add validators that are not yet active to pending balance deposits
    let pre_activation = post
        .validators()
        .iter()
        .enumerate()
        .filter(|(_, validator)| validator.activation_epoch == spec.far_future_epoch)
        .map(|(index, validator)| (index, validator.activation_eligibility_epoch))
        .sorted_by_key(|(index, epoch)| (*epoch, *index))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    // Process validators to queue entire balance and reset them
    for index in pre_activation {
        let balance = post
            .balances_mut()
            .get_mut(index)
            .ok_or(Error::UnknownValidator(index))?;
        let balance_copy = *balance;
        *balance = 0_u64;

        let validator = post
            .validators_mut()
            .get_mut(index)
            .ok_or(Error::UnknownValidator(index))?;
        validator.effective_balance = 0;
        validator.activation_eligibility_epoch = spec.far_future_epoch;
        let pubkey = validator.pubkey;
        let withdrawal_credentials = validator.withdrawal_credentials;

        post.pending_deposits_mut()?
            .push(PendingDeposit {
                pubkey,
                withdrawal_credentials,
                amount: balance_copy,
                signature: Signature::infinity()?.into(),
                slot: spec.genesis_slot,
            })
            .map_err(Error::MilhouseError)?;
    }

    // Ensure early adopters of compounding credentials go through the activation churn
    let compounding_indices: Vec<usize> = post
        .validators()
        .iter()
        .enumerate()
        .filter(|(_, validator)| validator.has_compounding_withdrawal_credential(spec))
        .map(|(index, _)| index)
        .collect();
    for index in compounding_indices {
        post.queue_excess_active_balance(index, spec)?;
    }

    *pre_state = post;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use types::*;

    type E = MinimalEthSpec;

    fn make_deneb_state(
        num_validators: usize,
        pre_activation: &[usize],
    ) -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let epoch = Epoch::new(10);
        let slot = epoch.start_slot(E::slots_per_epoch());

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

        let keypairs = types::test_utils::generate_deterministic_keypairs(num_validators);
        let mut validators = Vec::with_capacity(num_validators);
        let mut balances = Vec::with_capacity(num_validators);
        for (i, kp) in keypairs.iter().enumerate() {
            let (activation_epoch_val, activation_eligibility) = if pre_activation.contains(&i) {
                (spec.far_future_epoch, Epoch::new(5))
            } else {
                (Epoch::new(0), Epoch::new(0))
            };
            validators.push(Validator {
                pubkey: kp.pk.compress(),
                withdrawal_credentials: Hash256::zero(),
                effective_balance: 32_000_000_000,
                slashed: false,
                activation_eligibility_epoch: activation_eligibility,
                activation_epoch: activation_epoch_val,
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
            });
            balances.push(32_000_000_000);
        }

        let state = BeaconState::Deneb(BeaconStateDeneb {
            genesis_time: 5555,
            genesis_validators_root: Hash256::repeat_byte(0xEE),
            slot,
            fork: Fork {
                previous_version: spec.capella_fork_version,
                current_version: spec.deneb_fork_version,
                epoch,
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
            eth1_deposit_index: 300,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::new(vec![
                ParticipationFlags::default();
                num_validators
            ])
            .unwrap(),
            current_epoch_participation: List::new(vec![
                ParticipationFlags::default();
                num_validators
            ])
            .unwrap(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint {
                epoch: Epoch::new(8),
                root: Hash256::repeat_byte(0xAA),
            },
            current_justified_checkpoint: Checkpoint {
                epoch: Epoch::new(9),
                root: Hash256::repeat_byte(0xBB),
            },
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(7),
                root: Hash256::repeat_byte(0xCC),
            },
            inactivity_scores: List::new(vec![0; num_validators]).unwrap(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_header: ExecutionPayloadHeaderDeneb {
                block_hash: ExecutionBlockHash::repeat_byte(0x77),
                ..Default::default()
            },
            next_withdrawal_index: 99,
            next_withdrawal_validator_index: 3,
            historical_summaries: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <_>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: EpochCache::default(),
        });

        (state, spec)
    }

    #[test]
    fn upgrade_sets_fork_versions() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        assert!(state.as_electra().is_ok());
        let fork = state.fork();
        assert_eq!(fork.previous_version, spec.deneb_fork_version);
        assert_eq!(fork.current_version, spec.electra_fork_version);
        assert_eq!(fork.epoch, Epoch::new(10));
    }

    #[test]
    fn upgrade_preserves_versioning() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        assert_eq!(state.genesis_time(), 5555);
        assert_eq!(state.genesis_validators_root(), Hash256::repeat_byte(0xEE));
    }

    #[test]
    fn upgrade_preserves_registry_and_eth1() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        assert_eq!(state.validators().len(), 8);
        assert_eq!(state.eth1_deposit_index(), 300);
    }

    #[test]
    fn upgrade_preserves_finality() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        assert_eq!(state.finalized_checkpoint().epoch, Epoch::new(7));
        assert_eq!(state.previous_justified_checkpoint().epoch, Epoch::new(8));
        assert_eq!(state.current_justified_checkpoint().epoch, Epoch::new(9));
    }

    #[test]
    fn upgrade_preserves_capella_fields() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        let electra = state.as_electra().unwrap();
        assert_eq!(electra.next_withdrawal_index, 99);
        assert_eq!(electra.next_withdrawal_validator_index, 3);
    }

    #[test]
    fn upgrade_upgrades_execution_payload_header() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        let electra = state.as_electra().unwrap();
        assert_eq!(
            electra.latest_execution_payload_header.block_hash,
            ExecutionBlockHash::repeat_byte(0x77)
        );
    }

    #[test]
    fn upgrade_initializes_electra_fields() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        let electra = state.as_electra().unwrap();
        assert_eq!(
            electra.deposit_requests_start_index,
            spec.unset_deposit_requests_start_index
        );
        assert_eq!(electra.deposit_balance_to_consume, 0);
        // exit_balance_to_consume is set to the activation exit churn limit
        assert!(electra.exit_balance_to_consume > 0);
        // consolidation_balance_to_consume is set to the consolidation churn limit
        // (may be 0 if total active balance is small with minimal spec)
    }

    #[test]
    fn upgrade_queues_pre_activation_validators_as_pending_deposits() {
        // Validators 6 and 7 are not yet active (activation_epoch = far_future)
        let (mut state, spec) = make_deneb_state(8, &[6, 7]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        let electra = state.as_electra().unwrap();

        // Should have 2 pending deposits for the pre-activation validators
        assert_eq!(electra.pending_deposits.len(), 2);

        // Pre-activation validators should have balance zeroed
        assert_eq!(*state.balances().get(6).unwrap(), 0);
        assert_eq!(*state.balances().get(7).unwrap(), 0);

        // Their effective balance should also be zeroed
        assert_eq!(state.validators().get(6).unwrap().effective_balance, 0);
        assert_eq!(state.validators().get(7).unwrap().effective_balance, 0);

        // Their activation eligibility should be reset to far_future
        assert_eq!(
            state
                .validators()
                .get(6)
                .unwrap()
                .activation_eligibility_epoch,
            spec.far_future_epoch
        );

        // Active validators should be unaffected
        assert_eq!(*state.balances().get(0).unwrap(), 32_000_000_000);
    }

    #[test]
    fn upgrade_no_pre_activation_means_no_pending_deposits() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();

        let electra = state.as_electra().unwrap();
        assert_eq!(electra.pending_deposits.len(), 0);
    }

    #[test]
    fn upgrade_fails_on_wrong_variant() {
        let (mut state, spec) = make_deneb_state(8, &[]);
        upgrade_to_electra(&mut state, &spec).unwrap();
        // Now it's Electra — upgrading again should fail
        assert!(upgrade_to_electra(&mut state, &spec).is_err());
    }
}

pub fn upgrade_state_to_electra<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    earliest_exit_epoch: Epoch,
    earliest_consolidation_epoch: Epoch,
    spec: &ChainSpec,
) -> Result<BeaconState<E>, Error> {
    let epoch = pre_state.current_epoch();
    let pre = pre_state.as_deneb_mut()?;
    // Where possible, use something like `mem::take` to move fields from behind the &mut
    // reference. For other fields that don't have a good default value, use `clone`.
    //
    // Fixed size vectors get cloned because replacing them would require the same size
    // allocation as cloning.
    let post = BeaconState::Electra(BeaconStateElectra {
        // Versioning
        genesis_time: pre.genesis_time,
        genesis_validators_root: pre.genesis_validators_root,
        slot: pre.slot,
        fork: Fork {
            previous_version: pre.fork.current_version,
            current_version: spec.electra_fork_version,
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
        // `Participation
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
        // Execution
        latest_execution_payload_header: pre.latest_execution_payload_header.upgrade_to_electra(),
        // Capella
        next_withdrawal_index: pre.next_withdrawal_index,
        next_withdrawal_validator_index: pre.next_withdrawal_validator_index,
        historical_summaries: mem::take(&mut pre.historical_summaries),
        // Electra
        deposit_requests_start_index: spec.unset_deposit_requests_start_index,
        deposit_balance_to_consume: 0,
        exit_balance_to_consume: 0,
        earliest_exit_epoch,
        consolidation_balance_to_consume: 0,
        earliest_consolidation_epoch,
        pending_deposits: Default::default(),
        pending_partial_withdrawals: Default::default(),
        pending_consolidations: Default::default(),
        // Caches
        total_active_balance: pre.total_active_balance,
        progressive_balances_cache: mem::take(&mut pre.progressive_balances_cache),
        committee_caches: mem::take(&mut pre.committee_caches),
        pubkey_cache: mem::take(&mut pre.pubkey_cache),
        builder_pubkey_cache: BuilderPubkeyCache::default(),
        exit_cache: mem::take(&mut pre.exit_cache),
        slashings_cache: mem::take(&mut pre.slashings_cache),
        epoch_cache: EpochCache::default(),
    });
    Ok(post)
}
