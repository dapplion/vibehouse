use std::mem;
use types::{
    BeaconState, BeaconStateBellatrix, BeaconStateError as Error, BuilderPubkeyCache, ChainSpec,
    EpochCache, EthSpec, ExecutionPayloadHeaderBellatrix, Fork,
};

/// Transform a `Altair` state into an `Bellatrix` state.
pub fn upgrade_to_bellatrix<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let epoch = pre_state.current_epoch();
    let pre = pre_state.as_altair_mut()?;

    // Where possible, use something like `mem::take` to move fields from behind the &mut
    // reference. For other fields that don't have a good default value, use `clone`.
    //
    // Fixed size vectors get cloned because replacing them would require the same size
    // allocation as cloning.
    let post = BeaconState::Bellatrix(BeaconStateBellatrix {
        // Versioning
        genesis_time: pre.genesis_time,
        genesis_validators_root: pre.genesis_validators_root,
        slot: pre.slot,
        fork: Fork {
            previous_version: pre.fork.current_version,
            current_version: spec.bellatrix_fork_version,
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
        latest_execution_payload_header: <ExecutionPayloadHeaderBellatrix<E>>::default(),
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

    fn make_altair_state() -> (BeaconState<E>, ChainSpec) {
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

        let validators = vec![Validator {
            pubkey: PublicKeyBytes::empty(),
            withdrawal_credentials: Hash256::zero(),
            effective_balance: 32_000_000_000,
            slashed: false,
            activation_eligibility_epoch: Epoch::new(0),
            activation_epoch: Epoch::new(0),
            exit_epoch: spec.far_future_epoch,
            withdrawable_epoch: spec.far_future_epoch,
        }];

        let state = BeaconState::Altair(BeaconStateAltair {
            genesis_time: 1234,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.genesis_fork_version,
                current_version: spec.altair_fork_version,
                epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root: Hash256::repeat_byte(0x01),
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 42,
            validators: List::new(validators).unwrap(),
            balances: List::new(vec![32_000_000_000]).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint {
                epoch: Epoch::new(8),
                root: Hash256::repeat_byte(0xBB),
            },
            current_justified_checkpoint: Checkpoint {
                epoch: Epoch::new(9),
                root: Hash256::repeat_byte(0xCC),
            },
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(7),
                root: Hash256::repeat_byte(0xDD),
            },
            inactivity_scores: List::new(vec![5]).unwrap(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
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
        let (mut state, spec) = make_altair_state();
        upgrade_to_bellatrix(&mut state, &spec).unwrap();

        assert!(state.as_bellatrix().is_ok());
        let fork = state.fork();
        assert_eq!(fork.previous_version, spec.altair_fork_version);
        assert_eq!(fork.current_version, spec.bellatrix_fork_version);
        assert_eq!(fork.epoch, Epoch::new(10));
    }

    #[test]
    fn upgrade_preserves_versioning() {
        let (mut state, spec) = make_altair_state();
        upgrade_to_bellatrix(&mut state, &spec).unwrap();

        assert_eq!(state.genesis_time(), 1234);
        assert_eq!(state.genesis_validators_root(), Hash256::repeat_byte(0xAA));
    }

    #[test]
    fn upgrade_preserves_registry() {
        let (mut state, spec) = make_altair_state();
        upgrade_to_bellatrix(&mut state, &spec).unwrap();

        assert_eq!(state.validators().len(), 1);
        assert_eq!(*state.balances().get(0).unwrap(), 32_000_000_000);
        assert_eq!(state.eth1_deposit_index(), 42);
    }

    #[test]
    fn upgrade_preserves_finality() {
        let (mut state, spec) = make_altair_state();
        upgrade_to_bellatrix(&mut state, &spec).unwrap();

        assert_eq!(state.finalized_checkpoint().epoch, Epoch::new(7));
        assert_eq!(state.previous_justified_checkpoint().epoch, Epoch::new(8));
        assert_eq!(state.current_justified_checkpoint().epoch, Epoch::new(9));
    }

    #[test]
    fn upgrade_preserves_inactivity_scores() {
        let (mut state, spec) = make_altair_state();
        upgrade_to_bellatrix(&mut state, &spec).unwrap();

        assert_eq!(state.inactivity_scores().unwrap().len(), 1);
        assert_eq!(*state.inactivity_scores().unwrap().get(0).unwrap(), 5);
    }

    #[test]
    fn upgrade_initializes_default_execution_payload_header() {
        let (mut state, spec) = make_altair_state();
        upgrade_to_bellatrix(&mut state, &spec).unwrap();

        let bellatrix = state.as_bellatrix().unwrap();
        assert_eq!(
            bellatrix.latest_execution_payload_header.block_hash,
            ExecutionBlockHash::zero()
        );
    }

    #[test]
    fn upgrade_fails_on_wrong_variant() {
        let spec = E::default_spec();
        // Try to upgrade a Bellatrix state as if it were Altair — should fail
        let (mut state, _) = make_altair_state();
        upgrade_to_bellatrix(&mut state, &spec).unwrap();
        // Now try to upgrade again (it's Bellatrix, not Altair)
        assert!(upgrade_to_bellatrix(&mut state, &spec).is_err());
    }
}
