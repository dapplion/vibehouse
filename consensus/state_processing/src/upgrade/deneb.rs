use std::mem;
use types::{
    BeaconState, BeaconStateDeneb, BeaconStateError as Error, BuilderPubkeyCache, ChainSpec,
    EpochCache, EthSpec, Fork,
};

/// Transform a `Capella` state into an `Deneb` state.
pub fn upgrade_to_deneb<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let epoch = pre_state.current_epoch();
    let pre = pre_state.as_capella_mut()?;

    let previous_fork_version = pre.fork.current_version;

    // Where possible, use something like `mem::take` to move fields from behind the &mut
    // reference. For other fields that don't have a good default value, use `clone`.
    //
    // Fixed size vectors get cloned because replacing them would require the same size
    // allocation as cloning.
    let post = BeaconState::Deneb(BeaconStateDeneb {
        // Versioning
        genesis_time: pre.genesis_time,
        genesis_validators_root: pre.genesis_validators_root,
        slot: pre.slot,
        fork: Fork {
            previous_version: previous_fork_version,
            current_version: spec.deneb_fork_version,
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
        latest_execution_payload_header: pre.latest_execution_payload_header.upgrade_to_deneb(),
        // Capella
        next_withdrawal_index: pre.next_withdrawal_index,
        next_withdrawal_validator_index: pre.next_withdrawal_validator_index,
        historical_summaries: mem::take(&mut pre.historical_summaries),
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

    *pre_state = post;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use types::*;

    type E = MinimalEthSpec;

    fn make_capella_state() -> (BeaconState<E>, ChainSpec) {
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

        let state = BeaconState::Capella(BeaconStateCapella {
            genesis_time: 9999,
            genesis_validators_root: Hash256::repeat_byte(0xCC),
            slot,
            fork: Fork {
                previous_version: spec.bellatrix_fork_version,
                current_version: spec.capella_fork_version,
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
            eth1_deposit_index: 200,
            validators: List::new(vec![Validator {
                pubkey: PublicKeyBytes::empty(),
                withdrawal_credentials: Hash256::zero(),
                effective_balance: 32_000_000_000,
                slashed: false,
                activation_eligibility_epoch: Epoch::new(0),
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
            }])
            .unwrap(),
            balances: List::new(vec![32_000_000_000]).unwrap(),
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
            latest_execution_payload_header: ExecutionPayloadHeaderCapella {
                block_hash: ExecutionBlockHash::repeat_byte(0x55),
                ..Default::default()
            },
            next_withdrawal_index: 42,
            next_withdrawal_validator_index: 7,
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
        let (mut state, spec) = make_capella_state();
        upgrade_to_deneb(&mut state, &spec).unwrap();

        assert!(state.as_deneb().is_ok());
        let fork = state.fork();
        assert_eq!(fork.previous_version, spec.capella_fork_version);
        assert_eq!(fork.current_version, spec.deneb_fork_version);
        assert_eq!(fork.epoch, Epoch::new(10));
    }

    #[test]
    fn upgrade_preserves_versioning() {
        let (mut state, spec) = make_capella_state();
        upgrade_to_deneb(&mut state, &spec).unwrap();

        assert_eq!(state.genesis_time(), 9999);
        assert_eq!(state.genesis_validators_root(), Hash256::repeat_byte(0xCC));
    }

    #[test]
    fn upgrade_preserves_capella_fields() {
        let (mut state, spec) = make_capella_state();
        upgrade_to_deneb(&mut state, &spec).unwrap();

        let deneb = state.as_deneb().unwrap();
        assert_eq!(deneb.next_withdrawal_index, 42);
        assert_eq!(deneb.next_withdrawal_validator_index, 7);
    }

    #[test]
    fn upgrade_preserves_registry() {
        let (mut state, spec) = make_capella_state();
        upgrade_to_deneb(&mut state, &spec).unwrap();

        assert_eq!(state.validators().len(), 1);
        assert_eq!(*state.balances().get(0).unwrap(), 32_000_000_000);
        assert_eq!(state.eth1_deposit_index(), 200);
    }

    #[test]
    fn upgrade_upgrades_execution_payload_header() {
        let (mut state, spec) = make_capella_state();
        upgrade_to_deneb(&mut state, &spec).unwrap();

        let deneb = state.as_deneb().unwrap();
        assert_eq!(
            deneb.latest_execution_payload_header.block_hash,
            ExecutionBlockHash::repeat_byte(0x55)
        );
    }

    #[test]
    fn upgrade_fails_on_wrong_variant() {
        let spec = E::default_spec();
        let (mut state, _) = make_capella_state();
        upgrade_to_deneb(&mut state, &spec).unwrap();
        assert!(upgrade_to_deneb(&mut state, &spec).is_err());
    }
}
