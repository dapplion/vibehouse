use safe_arith::SafeArith;
use std::mem;
use types::{
    BeaconState, BeaconStateError as Error, BeaconStateFulu, BuilderPubkeyCache, ChainSpec,
    EthSpec, Fork, Vector,
};

/// Transform a `Electra` state into an `Fulu` state.
pub fn upgrade_to_fulu<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let _epoch = pre_state.current_epoch();

    let post = upgrade_state_to_fulu(pre_state, spec)?;

    *pre_state = post;

    Ok(())
}

fn initialize_proposer_lookahead<E: EthSpec>(
    state: &BeaconState<E>,
    spec: &ChainSpec,
) -> Result<Vector<u64, E::ProposerLookaheadSlots>, Error> {
    let current_epoch = state.current_epoch();
    let mut lookahead = Vec::with_capacity(E::proposer_lookahead_slots());
    for i in 0..(spec.min_seed_lookahead.safe_add(1)?.as_u64()) {
        let target_epoch = current_epoch.safe_add(i)?;
        lookahead.extend(
            state
                .get_beacon_proposer_indices(target_epoch, spec)
                .map(|vec| vec.into_iter().map(|x| x as u64))?,
        );
    }

    Vector::new(lookahead).map_err(|e| e.into())
}

pub fn upgrade_state_to_fulu<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<BeaconState<E>, Error> {
    let epoch = pre_state.current_epoch();
    let proposer_lookahead = initialize_proposer_lookahead(pre_state, spec)?;
    let pre = pre_state.as_electra_mut()?;
    // Where possible, use something like `mem::take` to move fields from behind the &mut
    // reference. For other fields that don't have a good default value, use `clone`.
    //
    // Fixed size vectors get cloned because replacing them would require the same size
    // allocation as cloning.
    let post = BeaconState::Fulu(BeaconStateFulu {
        // Versioning
        genesis_time: pre.genesis_time,
        genesis_validators_root: pre.genesis_validators_root,
        slot: pre.slot,
        fork: Fork {
            previous_version: pre.fork.current_version,
            current_version: spec.fulu_fork_version,
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
        latest_execution_payload_header: pre.latest_execution_payload_header.upgrade_to_fulu(),
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
        // Caches
        total_active_balance: pre.total_active_balance,
        progressive_balances_cache: mem::take(&mut pre.progressive_balances_cache),
        committee_caches: mem::take(&mut pre.committee_caches),
        pubkey_cache: mem::take(&mut pre.pubkey_cache),
        builder_pubkey_cache: BuilderPubkeyCache::default(),
        exit_cache: mem::take(&mut pre.exit_cache),
        slashings_cache: mem::take(&mut pre.slashings_cache),
        epoch_cache: mem::take(&mut pre.epoch_cache),
        proposer_lookahead,
    });
    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use types::*;

    type E = MinimalEthSpec;

    fn make_electra_state(num_validators: usize) -> (BeaconState<E>, ChainSpec) {
        let mut spec = E::default_spec();
        let epoch = Epoch::new(10);
        // Set fulu fork epoch so initialize_proposer_lookahead can compute next-epoch indices
        spec.fulu_fork_epoch = Some(epoch);
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
        for kp in &keypairs {
            validators.push(Validator {
                pubkey: kp.pk.compress(),
                withdrawal_credentials: Hash256::zero(),
                effective_balance: 32_000_000_000,
                slashed: false,
                activation_eligibility_epoch: Epoch::new(0),
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
            });
            balances.push(32_000_000_000);
        }

        let state = BeaconState::Electra(BeaconStateElectra {
            genesis_time: 1111,
            genesis_validators_root: Hash256::repeat_byte(0xDD),
            slot,
            fork: Fork {
                previous_version: spec.deneb_fork_version,
                current_version: spec.electra_fork_version,
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
            randao_mixes: Vector::new(vec![Hash256::repeat_byte(0x11); epochs_per_vector]).unwrap(),
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
            latest_execution_payload_header: ExecutionPayloadHeaderElectra {
                block_hash: ExecutionBlockHash::repeat_byte(0x77),
                ..Default::default()
            },
            next_withdrawal_index: 50,
            next_withdrawal_validator_index: 2,
            historical_summaries: List::default(),
            deposit_requests_start_index: u64::MAX,
            deposit_balance_to_consume: 1000,
            exit_balance_to_consume: 2000,
            earliest_exit_epoch: Epoch::new(5),
            consolidation_balance_to_consume: 3000,
            earliest_consolidation_epoch: Epoch::new(6),
            pending_deposits: List::default(),
            pending_partial_withdrawals: List::default(),
            pending_consolidations: List::default(),
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
        let (mut state, spec) = make_electra_state(8);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        state
            .build_committee_cache(RelativeEpoch::Next, &spec)
            .unwrap();
        upgrade_to_fulu(&mut state, &spec).unwrap();

        assert!(state.as_fulu().is_ok());
        let fork = state.fork();
        assert_eq!(fork.previous_version, spec.electra_fork_version);
        assert_eq!(fork.current_version, spec.fulu_fork_version);
        assert_eq!(fork.epoch, Epoch::new(10));
    }

    #[test]
    fn upgrade_preserves_versioning() {
        let (mut state, spec) = make_electra_state(8);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        state
            .build_committee_cache(RelativeEpoch::Next, &spec)
            .unwrap();
        upgrade_to_fulu(&mut state, &spec).unwrap();

        assert_eq!(state.genesis_time(), 1111);
        assert_eq!(state.genesis_validators_root(), Hash256::repeat_byte(0xDD));
    }

    #[test]
    fn upgrade_preserves_electra_fields() {
        let (mut state, spec) = make_electra_state(8);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        state
            .build_committee_cache(RelativeEpoch::Next, &spec)
            .unwrap();
        upgrade_to_fulu(&mut state, &spec).unwrap();

        let fulu = state.as_fulu().unwrap();
        assert_eq!(fulu.deposit_requests_start_index, u64::MAX);
        assert_eq!(fulu.deposit_balance_to_consume, 1000);
        assert_eq!(fulu.exit_balance_to_consume, 2000);
        assert_eq!(fulu.earliest_exit_epoch, Epoch::new(5));
        assert_eq!(fulu.consolidation_balance_to_consume, 3000);
        assert_eq!(fulu.earliest_consolidation_epoch, Epoch::new(6));
    }

    #[test]
    fn upgrade_preserves_capella_fields() {
        let (mut state, spec) = make_electra_state(8);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        state
            .build_committee_cache(RelativeEpoch::Next, &spec)
            .unwrap();
        upgrade_to_fulu(&mut state, &spec).unwrap();

        let fulu = state.as_fulu().unwrap();
        assert_eq!(fulu.next_withdrawal_index, 50);
        assert_eq!(fulu.next_withdrawal_validator_index, 2);
    }

    #[test]
    fn upgrade_preserves_registry() {
        let (mut state, spec) = make_electra_state(8);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        state
            .build_committee_cache(RelativeEpoch::Next, &spec)
            .unwrap();
        upgrade_to_fulu(&mut state, &spec).unwrap();

        assert_eq!(state.validators().len(), 8);
        assert_eq!(state.eth1_deposit_index(), 300);
    }

    #[test]
    fn upgrade_upgrades_execution_payload_header() {
        let (mut state, spec) = make_electra_state(8);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        state
            .build_committee_cache(RelativeEpoch::Next, &spec)
            .unwrap();
        upgrade_to_fulu(&mut state, &spec).unwrap();

        let fulu = state.as_fulu().unwrap();
        assert_eq!(
            fulu.latest_execution_payload_header.block_hash,
            ExecutionBlockHash::repeat_byte(0x77)
        );
    }

    #[test]
    fn upgrade_initializes_proposer_lookahead() {
        let (mut state, spec) = make_electra_state(8);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        state
            .build_committee_cache(RelativeEpoch::Next, &spec)
            .unwrap();
        upgrade_to_fulu(&mut state, &spec).unwrap();

        let fulu = state.as_fulu().unwrap();
        let lookahead = &fulu.proposer_lookahead;
        // Should have exactly ProposerLookaheadSlots entries
        assert_eq!(
            lookahead.len(),
            <E as EthSpec>::ProposerLookaheadSlots::to_usize()
        );

        // Each entry should be a valid validator index (< num_validators)
        for i in 0..lookahead.len() {
            let idx = *lookahead.get(i).unwrap();
            assert!(
                idx < 8,
                "proposer index {} at slot {} is out of range",
                idx,
                i
            );
        }
    }

    #[test]
    fn upgrade_proposer_lookahead_deterministic() {
        let (mut state1, spec) = make_electra_state(8);
        state1
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        let (mut state2, _) = make_electra_state(8);
        state2
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();

        upgrade_to_fulu(&mut state1, &spec).unwrap();
        upgrade_to_fulu(&mut state2, &spec).unwrap();

        let la1 = &state1.as_fulu().unwrap().proposer_lookahead;
        let la2 = &state2.as_fulu().unwrap().proposer_lookahead;
        for i in 0..la1.len() {
            assert_eq!(
                la1.get(i).unwrap(),
                la2.get(i).unwrap(),
                "proposer lookahead mismatch at index {}",
                i
            );
        }
    }

    #[test]
    fn upgrade_fails_on_wrong_variant() {
        let (mut state, spec) = make_electra_state(8);
        state
            .build_committee_cache(RelativeEpoch::Current, &spec)
            .unwrap();
        state
            .build_committee_cache(RelativeEpoch::Next, &spec)
            .unwrap();
        upgrade_to_fulu(&mut state, &spec).unwrap();
        // Now state is Fulu, calling upgrade again should fail (expects Electra)
        assert!(upgrade_to_fulu(&mut state, &spec).is_err());
    }
}
