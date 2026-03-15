use crate::common::update_progressive_balances_cache::initialize_progressive_balances_cache;
use crate::common::{
    attesting_indices_base::get_attesting_indices, get_attestation_participation_flag_indices,
};
use std::mem;
use std::sync::Arc;
use types::{
    BeaconState, BeaconStateAltair, BeaconStateError as Error, BuilderPubkeyCache, ChainSpec,
    EpochCache, EthSpec, Fork, List, ParticipationFlags, PendingAttestation, RelativeEpoch,
    SyncCommittee,
};

/// Translate the participation information from the epoch prior to the fork into Altair's format.
pub fn translate_participation<E: EthSpec>(
    state: &mut BeaconState<E>,
    pending_attestations: &List<PendingAttestation<E>, E::MaxPendingAttestations>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    // Previous epoch committee cache is required for `get_attesting_indices`.
    state.build_committee_cache(RelativeEpoch::Previous, spec)?;

    for attestation in pending_attestations {
        let data = &attestation.data;
        let inclusion_delay = attestation.inclusion_delay;

        // Translate attestation inclusion info to flag indices.
        let participation_flag_indices =
            get_attestation_participation_flag_indices(state, data, inclusion_delay, spec)?;

        // Apply flags to all attesting validators.
        let committee = state.get_beacon_committee(data.slot, data.index)?;
        let attesting_indices =
            get_attesting_indices::<E>(committee.committee, &attestation.aggregation_bits)?;
        let epoch_participation = state.previous_epoch_participation_mut()?;

        for index in attesting_indices {
            for flag_index in &participation_flag_indices {
                epoch_participation
                    .get_mut(index as usize)
                    .ok_or(Error::UnknownValidator(index as usize))?
                    .add_flag(*flag_index)?;
            }
        }
    }
    Ok(())
}

/// Transform a `Base` state into an `Altair` state.
pub fn upgrade_to_altair<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let epoch = pre_state.current_epoch();
    let pre = pre_state.as_base_mut()?;

    let default_epoch_participation =
        List::new(vec![ParticipationFlags::default(); pre.validators.len()])?;
    let inactivity_scores = List::new(vec![0; pre.validators.len()])?;

    let temp_sync_committee = Arc::new(SyncCommittee::temporary());

    // Where possible, use something like `mem::take` to move fields from behind the &mut
    // reference. For other fields that don't have a good default value, use `clone`.
    //
    // Fixed size vectors get cloned because replacing them would require the same size
    // allocation as cloning.
    let mut post = BeaconState::Altair(BeaconStateAltair {
        // Versioning
        genesis_time: pre.genesis_time,
        genesis_validators_root: pre.genesis_validators_root,
        slot: pre.slot,
        fork: Fork {
            previous_version: pre.fork.current_version,
            current_version: spec.altair_fork_version,
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
        previous_epoch_participation: default_epoch_participation.clone(),
        current_epoch_participation: default_epoch_participation,
        // Finality
        justification_bits: pre.justification_bits.clone(),
        previous_justified_checkpoint: pre.previous_justified_checkpoint,
        current_justified_checkpoint: pre.current_justified_checkpoint,
        finalized_checkpoint: pre.finalized_checkpoint,
        // Inactivity
        inactivity_scores,
        // Sync committees
        current_sync_committee: temp_sync_committee.clone(), // not read
        next_sync_committee: temp_sync_committee,            // not read
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

    // Fill in previous epoch participation from the pre state's pending attestations.
    translate_participation(&mut post, &pre.previous_epoch_attestations, spec)?;

    initialize_progressive_balances_cache(&mut post, spec)?;

    // Fill in sync committees
    // Note: A duplicate committee is assigned for the current and next committee at the fork
    // boundary
    let sync_committee = Arc::new(post.get_next_sync_committee(spec)?);
    *post.current_sync_committee_mut()? = sync_committee.clone();
    *post.next_sync_committee_mut()? = sync_committee;

    *pre_state = post;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::*;

    type E = MinimalEthSpec;

    fn make_base_state(num_validators: usize) -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let epoch = Epoch::new(10);
        let slot = epoch.start_slot(E::slots_per_epoch());

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

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

        let state = BeaconState::Base(BeaconStateBase {
            genesis_time: 1234,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.genesis_fork_version,
                current_version: spec.genesis_fork_version,
                epoch: E::genesis_epoch(),
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
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_attestations: List::default(),
            current_epoch_attestations: List::default(),
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
        let (mut state, spec) = make_base_state(8);
        upgrade_to_altair(&mut state, &spec).unwrap();

        assert!(state.as_altair().is_ok());
        let fork = state.fork();
        assert_eq!(fork.previous_version, spec.genesis_fork_version);
        assert_eq!(fork.current_version, spec.altair_fork_version);
        assert_eq!(fork.epoch, Epoch::new(10));
    }

    #[test]
    fn upgrade_preserves_versioning() {
        let (mut state, spec) = make_base_state(8);
        upgrade_to_altair(&mut state, &spec).unwrap();

        assert_eq!(state.genesis_time(), 1234);
        assert_eq!(state.genesis_validators_root(), Hash256::repeat_byte(0xAA));
    }

    #[test]
    fn upgrade_preserves_registry_and_eth1() {
        let (mut state, spec) = make_base_state(8);
        upgrade_to_altair(&mut state, &spec).unwrap();

        assert_eq!(state.validators().len(), 8);
        assert_eq!(*state.balances().get(0).unwrap(), 32_000_000_000);
        assert_eq!(state.eth1_deposit_index(), 42);
    }

    #[test]
    fn upgrade_preserves_finality() {
        let (mut state, spec) = make_base_state(8);
        upgrade_to_altair(&mut state, &spec).unwrap();

        assert_eq!(state.finalized_checkpoint().epoch, Epoch::new(7));
        assert_eq!(state.previous_justified_checkpoint().epoch, Epoch::new(8));
        assert_eq!(state.current_justified_checkpoint().epoch, Epoch::new(9));
    }

    #[test]
    fn upgrade_initializes_inactivity_scores() {
        let (mut state, spec) = make_base_state(8);
        upgrade_to_altair(&mut state, &spec).unwrap();

        let scores = state.inactivity_scores().unwrap();
        assert_eq!(scores.len(), 8);
        for i in 0..8 {
            assert_eq!(*scores.get(i).unwrap(), 0);
        }
    }

    #[test]
    fn upgrade_initializes_participation_flags() {
        let (mut state, spec) = make_base_state(8);
        upgrade_to_altair(&mut state, &spec).unwrap();

        // Current epoch participation should be default (no flags)
        let current = state.current_epoch_participation().unwrap();
        assert_eq!(current.len(), 8);
        for i in 0..8 {
            assert_eq!(*current.get(i).unwrap(), ParticipationFlags::default());
        }
    }

    #[test]
    fn upgrade_initializes_sync_committees() {
        let (mut state, spec) = make_base_state(8);
        upgrade_to_altair(&mut state, &spec).unwrap();

        let current = state.current_sync_committee().unwrap();
        let next = state.next_sync_committee().unwrap();
        // Both committees should be identical at the fork boundary
        assert_eq!(current, next);
        // The committees should have been computed (not temporary)
        assert_eq!(
            current.pubkeys.len(),
            <E as EthSpec>::SyncCommitteeSize::to_usize()
        );
    }

    #[test]
    fn upgrade_fails_on_wrong_variant() {
        let (mut state, spec) = make_base_state(8);
        upgrade_to_altair(&mut state, &spec).unwrap();
        // Now it's Altair — upgrading again should fail
        assert!(upgrade_to_altair(&mut state, &spec).is_err());
    }
}
