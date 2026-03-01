use crate::per_block_processing::is_valid_deposit_signature;
use ssz_types::BitVector;
use ssz_types::typenum::Unsigned;
use std::mem;
use types::{
    Address, BeaconState, BeaconStateError as Error, BeaconStateGloas, Builder,
    BuilderPendingPayment, BuilderPubkeyCache, ChainSpec, DepositData, EthSpec,
    ExecutionPayloadBid, Fork, List, PublicKeyBytes, Vector,
};

/// Transform a `Fulu` state into a `Gloas` state.
pub fn upgrade_to_gloas<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    let mut post = upgrade_state_to_gloas(pre_state, spec)?;

    // [New in Gloas:EIP7732] Onboard builders from pending deposits
    onboard_builders_from_pending_deposits(&mut post, spec)?;

    *pre_state = post;

    Ok(())
}

pub(crate) fn upgrade_state_to_gloas<E: EthSpec>(
    pre_state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<BeaconState<E>, Error> {
    let epoch = pre_state.current_epoch();
    let pre = pre_state.as_fulu_mut()?;
    // Where possible, use something like `mem::take` to move fields from behind the &mut
    // reference. For other fields that don't have a good default value, use `clone`.
    //
    // Fixed size vectors get cloned because replacing them would require the same size
    // allocation as cloning.
    let post = BeaconState::Gloas(BeaconStateGloas {
        // Versioning
        genesis_time: pre.genesis_time,
        genesis_validators_root: pre.genesis_validators_root,
        slot: pre.slot,
        fork: Fork {
            previous_version: pre.fork.current_version,
            current_version: spec.gloas_fork_version,
            epoch,
        },
        // History
        latest_block_header: pre.latest_block_header.clone(),
        block_roots: pre.block_roots.clone(),
        state_roots: pre.state_roots.clone(),
        historical_roots: mem::take(&mut pre.historical_roots),
        // Eth1
        eth1_data: pre.eth1_data.clone(),
        eth1_data_votes: mem::take(&mut pre.eth1_data_votes),
        eth1_deposit_index: pre.eth1_deposit_index,
        // Registry
        validators: mem::take(&mut pre.validators),
        balances: mem::take(&mut pre.balances),
        // Randomness
        randao_mixes: pre.randao_mixes.clone(),
        // Slashings
        slashings: pre.slashings.clone(),
        // Participation
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
        // Execution Bid (replaces latest_execution_payload_header)
        latest_execution_payload_bid: ExecutionPayloadBid {
            block_hash: pre.latest_execution_payload_header.block_hash,
            ..Default::default()
        },
        // Capella
        next_withdrawal_index: pre.next_withdrawal_index,
        next_withdrawal_validator_index: pre.next_withdrawal_validator_index,
        historical_summaries: pre.historical_summaries.clone(),
        // Electra
        deposit_requests_start_index: pre.deposit_requests_start_index,
        deposit_balance_to_consume: pre.deposit_balance_to_consume,
        exit_balance_to_consume: pre.exit_balance_to_consume,
        earliest_exit_epoch: pre.earliest_exit_epoch,
        consolidation_balance_to_consume: pre.consolidation_balance_to_consume,
        earliest_consolidation_epoch: pre.earliest_consolidation_epoch,
        pending_deposits: pre.pending_deposits.clone(),
        pending_partial_withdrawals: pre.pending_partial_withdrawals.clone(),
        pending_consolidations: pre.pending_consolidations.clone(),
        proposer_lookahead: mem::take(&mut pre.proposer_lookahead),
        // Gloas
        builders: List::default(),
        next_withdrawal_builder_index: 0,
        // All bits set to true per spec:
        // execution_payload_availability = [0b1 for _ in range(SLOTS_PER_HISTORICAL_ROOT)]
        execution_payload_availability: BitVector::from_bytes(
            vec![0xFFu8; E::SlotsPerHistoricalRoot::to_usize() / 8].into(),
        )
        .map_err(|_| Error::InvalidBitfield)?,
        builder_pending_payments: Vector::new(vec![
            BuilderPendingPayment::default();
            E::builder_pending_payments_limit()
        ])?,
        builder_pending_withdrawals: List::default(),
        latest_block_hash: pre.latest_execution_payload_header.block_hash,
        payload_expected_withdrawals: List::default(),
        // Caches
        total_active_balance: pre.total_active_balance,
        progressive_balances_cache: mem::take(&mut pre.progressive_balances_cache),
        committee_caches: mem::take(&mut pre.committee_caches),
        pubkey_cache: mem::take(&mut pre.pubkey_cache),
        builder_pubkey_cache: BuilderPubkeyCache::default(),
        exit_cache: mem::take(&mut pre.exit_cache),
        slashings_cache: mem::take(&mut pre.slashings_cache),
        epoch_cache: mem::take(&mut pre.epoch_cache),
    });
    Ok(post)
}

/// [New in Gloas:EIP7732] Applies any pending deposit for builders, effectively
/// onboarding builders at the fork transition.
fn onboard_builders_from_pending_deposits<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), Error> {
    // Collect validator pubkeys for lookup
    let validator_pubkeys: Vec<PublicKeyBytes> =
        state.validators().iter().map(|v| v.pubkey).collect();

    let pending_deposits = state.pending_deposits()?.clone();
    let mut new_pending_deposits = Vec::new();
    let mut new_validator_pubkeys: Vec<PublicKeyBytes> = Vec::new();

    for deposit in pending_deposits.iter() {
        // If pubkey belongs to a validator, keep as validator deposit
        if validator_pubkeys.contains(&deposit.pubkey)
            || new_validator_pubkeys.contains(&deposit.pubkey)
        {
            new_pending_deposits.push(deposit.clone());
            continue;
        }

        // Check if it's an existing builder (O(1) via cache) or has builder credentials
        let is_existing_builder = state.builder_pubkey_cache().get(&deposit.pubkey).is_some();
        let has_builder_credentials =
            deposit.withdrawal_credentials.as_slice().first().copied() == Some(0x03); // BUILDER_WITHDRAWAL_PREFIX

        if is_existing_builder || has_builder_credentials {
            // Apply as builder deposit
            apply_builder_deposit::<E>(
                state,
                deposit.pubkey,
                deposit.withdrawal_credentials,
                deposit.amount,
                &deposit.signature,
                deposit.slot,
                spec,
            )?;
            continue;
        }

        // Check if this is a valid new validator deposit
        let deposit_data = DepositData {
            pubkey: deposit.pubkey,
            withdrawal_credentials: deposit.withdrawal_credentials,
            amount: deposit.amount,
            signature: deposit.signature.clone(),
        };
        if is_valid_deposit_signature(&deposit_data, spec).is_ok() {
            new_validator_pubkeys.push(deposit.pubkey);
            new_pending_deposits.push(deposit.clone());
        }
    }

    // Replace pending_deposits with filtered list
    *state.pending_deposits_mut()? =
        List::new(new_pending_deposits).map_err(Error::MilhouseError)?;

    Ok(())
}

/// Apply a deposit for a builder during fork upgrade.
fn apply_builder_deposit<E: EthSpec>(
    state: &mut BeaconState<E>,
    pubkey: PublicKeyBytes,
    withdrawal_credentials: types::Hash256,
    amount: u64,
    signature: &types::SignatureBytes,
    slot: types::Slot,
    spec: &ChainSpec,
) -> Result<(), Error> {
    // Use builder pubkey cache for O(1) lookup
    let builder_index = state.builder_pubkey_cache().get(&pubkey);

    if let Some(index) = builder_index {
        // Top-up existing builder
        let state_gloas = state
            .as_gloas_mut()
            .map_err(|_| Error::IncorrectStateVariant)?;
        let builder = state_gloas
            .builders
            .get_mut(index)
            .ok_or(Error::UnknownValidator(index))?;
        builder.balance = builder.balance.saturating_add(amount);
    } else {
        // New builder - verify deposit signature
        let deposit_data = DepositData {
            pubkey,
            withdrawal_credentials,
            amount,
            signature: signature.clone(),
        };

        if is_valid_deposit_signature(&deposit_data, spec).is_ok() {
            let state_gloas = state
                .as_gloas_mut()
                .map_err(|_| Error::IncorrectStateVariant)?;
            let current_epoch = state_gloas.slot.epoch(E::slots_per_epoch());

            // Find reusable index or append
            let new_index = state_gloas
                .builders
                .iter()
                .position(|b| b.withdrawable_epoch <= current_epoch && b.balance == 0)
                .unwrap_or(state_gloas.builders.len());

            let cred_slice = withdrawal_credentials.as_slice();
            let version = cred_slice
                .first()
                .copied()
                .ok_or(Error::IncorrectStateVariant)?;
            let address_bytes = cred_slice.get(12..).ok_or(Error::IncorrectStateVariant)?;

            let builder = Builder {
                pubkey,
                version,
                execution_address: Address::from_slice(address_bytes),
                balance: amount,
                deposit_epoch: slot.epoch(E::slots_per_epoch()),
                withdrawable_epoch: spec.far_future_epoch,
            };

            if new_index < state_gloas.builders.len() {
                // Reusing exited builder slot — update cache
                let old_pubkey = state_gloas.builders.get(new_index).map(|b| b.pubkey);
                *state_gloas
                    .builders
                    .get_mut(new_index)
                    .ok_or(Error::UnknownValidator(new_index))? = builder;
                if let Some(old_pk) = old_pubkey {
                    state_gloas.builder_pubkey_cache.remove(&old_pk);
                }
                state_gloas.builder_pubkey_cache.insert(pubkey, new_index);
            } else {
                let new_idx = state_gloas.builders.len();
                state_gloas
                    .builders
                    .push(builder)
                    .map_err(Error::MilhouseError)?;
                state_gloas.builder_pubkey_cache.insert(pubkey, new_idx);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::FixedBytesExtended;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::test_utils::generate_deterministic_keypairs;
    use types::{
        BeaconBlockHeader, BeaconStateFulu, CACHED_EPOCHS, Checkpoint, CommitteeCache, Epoch,
        ExecutionBlockHash, ExecutionPayloadHeaderFulu, ExitCache, FixedVector, Fork, Hash256,
        MinimalEthSpec, PendingDeposit, ProgressiveBalancesCache, PubkeyCache, SignatureBytes,
        SlashingsCache, Slot, SyncCommittee, Unsigned, Validator,
    };

    type E = MinimalEthSpec;

    const BALANCE: u64 = 32_000_000_000;
    const NUM_VALIDATORS: usize = 4;

    /// Create a Fulu BeaconState suitable for testing upgrade_to_gloas.
    fn make_fulu_state() -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let slot = Slot::new(E::slots_per_epoch()); // slot 8 = epoch 1
        let epoch = slot.epoch(E::slots_per_epoch());

        let keypairs = generate_deterministic_keypairs(NUM_VALIDATORS);
        let mut validators = Vec::with_capacity(NUM_VALIDATORS);
        let mut balances = Vec::with_capacity(NUM_VALIDATORS);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);
            validators.push(Validator {
                pubkey: kp.pk.compress(),
                effective_balance: BALANCE,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..Validator::default()
            });
            balances.push(BALANCE);
        }

        let block_hash = ExecutionBlockHash::repeat_byte(0x42);

        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: PublicKeyBytes::empty(),
        });

        let state = BeaconState::Fulu(BeaconStateFulu {
            genesis_time: 1234,
            genesis_validators_root: Hash256::repeat_byte(0xBB),
            slot,
            fork: Fork {
                previous_version: spec.electra_fork_version,
                current_version: spec.fulu_fork_version,
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
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 55,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(1),
                root: Hash256::repeat_byte(0xCC),
            },
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_header: ExecutionPayloadHeaderFulu {
                block_hash,
                ..Default::default()
            },
            next_withdrawal_index: 7,
            next_withdrawal_validator_index: 3,
            historical_summaries: List::default(),
            deposit_requests_start_index: u64::MAX,
            deposit_balance_to_consume: 500,
            exit_balance_to_consume: 600,
            earliest_exit_epoch: Epoch::new(2),
            consolidation_balance_to_consume: 700,
            earliest_consolidation_epoch: Epoch::new(3),
            pending_deposits: List::default(),
            pending_partial_withdrawals: List::default(),
            pending_consolidations: List::default(),
            proposer_lookahead: Vector::new(vec![
                0u64;
                <E as EthSpec>::ProposerLookaheadSlots::to_usize()
            ])
            .unwrap(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            builder_pubkey_cache: BuilderPubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        (state, spec)
    }

    /// Create a builder deposit with 0x03 prefix withdrawal credentials.
    fn make_builder_deposit(
        keypair: &bls::Keypair,
        amount: u64,
        slot: Slot,
        spec: &ChainSpec,
    ) -> PendingDeposit {
        let mut creds = [0u8; 32];
        creds[0] = 0x03; // BUILDER_WITHDRAWAL_PREFIX
        creds[12..].copy_from_slice(&[0xDD; 20]);
        let withdrawal_credentials = Hash256::from_slice(&creds);

        let deposit_data = types::DepositData {
            pubkey: keypair.pk.compress(),
            withdrawal_credentials,
            amount,
            signature: SignatureBytes::empty(),
        };
        let signature = deposit_data.create_signature(&keypair.sk, spec);

        PendingDeposit {
            pubkey: keypair.pk.compress(),
            withdrawal_credentials,
            amount,
            signature,
            slot,
        }
    }

    /// Create a validator deposit with 0x01 prefix withdrawal credentials.
    fn make_validator_deposit(
        keypair: &bls::Keypair,
        amount: u64,
        slot: Slot,
        spec: &ChainSpec,
    ) -> PendingDeposit {
        let mut creds = [0u8; 32];
        creds[0] = 0x01;
        creds[12..].copy_from_slice(&[0xEE; 20]);
        let withdrawal_credentials = Hash256::from_slice(&creds);

        let deposit_data = types::DepositData {
            pubkey: keypair.pk.compress(),
            withdrawal_credentials,
            amount,
            signature: SignatureBytes::empty(),
        };
        let signature = deposit_data.create_signature(&keypair.sk, spec);

        PendingDeposit {
            pubkey: keypair.pk.compress(),
            withdrawal_credentials,
            amount,
            signature,
            slot,
        }
    }

    // ========================================================================
    // upgrade_state_to_gloas: structural field migration
    // ========================================================================

    #[test]
    fn upgrade_preserves_versioning_fields() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        assert!(state.as_gloas().is_ok());
        assert_eq!(state.genesis_time(), 1234);
        assert_eq!(state.genesis_validators_root(), Hash256::repeat_byte(0xBB));
        assert_eq!(state.slot(), Slot::new(E::slots_per_epoch()));
        let fork = state.fork();
        assert_eq!(fork.previous_version, spec.fulu_fork_version);
        assert_eq!(fork.current_version, spec.gloas_fork_version);
        assert_eq!(fork.epoch, Epoch::new(1));
    }

    #[test]
    fn upgrade_preserves_registry() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        assert_eq!(state.validators().len(), NUM_VALIDATORS);
        assert_eq!(state.balances().len(), NUM_VALIDATORS);
        for i in 0..NUM_VALIDATORS {
            assert_eq!(*state.balances().get(i).unwrap(), BALANCE);
        }
    }

    #[test]
    fn upgrade_preserves_electra_fields() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.deposit_requests_start_index, u64::MAX);
        assert_eq!(gloas.deposit_balance_to_consume, 500);
        assert_eq!(gloas.exit_balance_to_consume, 600);
        assert_eq!(gloas.earliest_exit_epoch, Epoch::new(2));
        assert_eq!(gloas.consolidation_balance_to_consume, 700);
        assert_eq!(gloas.earliest_consolidation_epoch, Epoch::new(3));
    }

    #[test]
    fn upgrade_preserves_capella_fields() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.next_withdrawal_index, 7);
        assert_eq!(gloas.next_withdrawal_validator_index, 3);
    }

    #[test]
    fn upgrade_preserves_finality() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        assert_eq!(state.finalized_checkpoint().epoch, Epoch::new(1));
        assert_eq!(
            state.finalized_checkpoint().root,
            Hash256::repeat_byte(0xCC)
        );
    }

    #[test]
    fn upgrade_creates_execution_payload_bid_from_header() {
        let (mut state, spec) = make_fulu_state();
        let expected_block_hash = ExecutionBlockHash::repeat_byte(0x42);
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // block_hash from header should be preserved
        assert_eq!(
            gloas.latest_execution_payload_bid.block_hash,
            expected_block_hash
        );
        // All other bid fields should be default
        assert_eq!(
            gloas.latest_execution_payload_bid.parent_block_hash,
            ExecutionBlockHash::zero()
        );
        assert_eq!(gloas.latest_execution_payload_bid.slot, Slot::new(0));
    }

    #[test]
    fn upgrade_sets_latest_block_hash() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(
            gloas.latest_block_hash,
            ExecutionBlockHash::repeat_byte(0x42)
        );
    }

    // ========================================================================
    // upgrade_state_to_gloas: new Gloas fields initialization
    // ========================================================================

    #[test]
    fn upgrade_initializes_empty_builders() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builders.len(), 0);
    }

    #[test]
    fn upgrade_initializes_builder_withdrawal_index() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.next_withdrawal_builder_index, 0);
    }

    #[test]
    fn upgrade_initializes_execution_payload_availability_all_true() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        let bits = &gloas.execution_payload_availability;
        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        for i in 0..slots_per_hist {
            assert!(bits.get(i).unwrap(), "bit {} should be true", i);
        }
    }

    #[test]
    fn upgrade_initializes_builder_pending_payments_all_default() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        let limit = E::builder_pending_payments_limit();
        assert_eq!(gloas.builder_pending_payments.len(), limit);
        for i in 0..limit {
            let payment = gloas.builder_pending_payments.get(i).unwrap();
            assert_eq!(payment.weight, 0);
            assert_eq!(payment.withdrawal.amount, 0);
        }
    }

    #[test]
    fn upgrade_initializes_empty_builder_pending_withdrawals() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builder_pending_withdrawals.len(), 0);
    }

    #[test]
    fn upgrade_initializes_empty_payload_expected_withdrawals() {
        let (mut state, spec) = make_fulu_state();
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.payload_expected_withdrawals.len(), 0);
    }

    // ========================================================================
    // onboard_builders_from_pending_deposits
    // ========================================================================

    #[test]
    fn upgrade_no_pending_deposits_no_builders() {
        let (mut state, spec) = make_fulu_state();
        // No pending deposits set
        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builders.len(), 0);
        assert_eq!(gloas.pending_deposits.len(), 0);
    }

    #[test]
    fn upgrade_builder_deposit_creates_builder() {
        let (mut state, spec) = make_fulu_state();
        // Use a keypair NOT in the validator set
        let extra_keypairs = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_keypairs[NUM_VALIDATORS]; // index 4 = not a validator
        let slot = state.slot();

        let deposit = make_builder_deposit(builder_kp, 10_000_000_000, slot, &spec);
        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Builder should be created
        assert_eq!(gloas.builders.len(), 1);
        let b0 = gloas.builders.get(0).unwrap();
        assert_eq!(b0.pubkey, builder_kp.pk.compress());
        assert_eq!(b0.balance, 10_000_000_000);
        assert_eq!(b0.version, 0x03);
        assert_eq!(b0.execution_address, Address::repeat_byte(0xDD));
        // Deposit should be removed from pending
        assert_eq!(gloas.pending_deposits.len(), 0);
    }

    #[test]
    fn upgrade_validator_deposit_kept_in_pending() {
        let (mut state, spec) = make_fulu_state();
        // Use a keypair that IS a validator
        let keypairs = generate_deterministic_keypairs(NUM_VALIDATORS);
        let val_kp = &keypairs[0];
        let slot = state.slot();

        let deposit = PendingDeposit {
            pubkey: val_kp.pk.compress(),
            withdrawal_credentials: Hash256::repeat_byte(0x01),
            amount: 1_000_000_000,
            signature: SignatureBytes::empty(),
            slot,
        };
        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // No builders created
        assert_eq!(gloas.builders.len(), 0);
        // Deposit kept in pending
        assert_eq!(gloas.pending_deposits.len(), 1);
    }

    #[test]
    fn upgrade_mixed_deposits_separated_correctly() {
        let (mut state, spec) = make_fulu_state();
        let all_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 2);
        let val_kp = &all_kps[0]; // existing validator
        let builder_kp1 = &all_kps[NUM_VALIDATORS]; // new builder
        let builder_kp2 = &all_kps[NUM_VALIDATORS + 1]; // new builder
        let slot = state.slot();

        let val_deposit = PendingDeposit {
            pubkey: val_kp.pk.compress(),
            withdrawal_credentials: Hash256::repeat_byte(0x01),
            amount: 1_000_000_000,
            signature: SignatureBytes::empty(),
            slot,
        };
        let builder_deposit1 = make_builder_deposit(builder_kp1, 5_000_000_000, slot, &spec);
        let builder_deposit2 = make_builder_deposit(builder_kp2, 8_000_000_000, slot, &spec);

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits =
            List::new(vec![val_deposit, builder_deposit1, builder_deposit2]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Two builders created
        assert_eq!(gloas.builders.len(), 2);
        assert_eq!(gloas.builders.get(0).unwrap().balance, 5_000_000_000);
        assert_eq!(gloas.builders.get(1).unwrap().balance, 8_000_000_000);
        // Only validator deposit remains
        assert_eq!(gloas.pending_deposits.len(), 1);
        assert_eq!(
            gloas.pending_deposits.get(0).unwrap().pubkey,
            val_kp.pk.compress()
        );
    }

    #[test]
    fn upgrade_builder_topup_existing_builder() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        // Two deposits for the same builder pubkey
        let deposit1 = make_builder_deposit(builder_kp, 5_000_000_000, slot, &spec);
        let deposit2 = make_builder_deposit(builder_kp, 3_000_000_000, slot, &spec);

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit1, deposit2]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Only one builder, balance is sum
        assert_eq!(gloas.builders.len(), 1);
        assert_eq!(gloas.builders.get(0).unwrap().balance, 8_000_000_000);
    }

    #[test]
    fn upgrade_new_validator_deposit_with_valid_signature_kept() {
        let (mut state, spec) = make_fulu_state();
        // New pubkey not in validator set, 0x01 credentials (validator, not builder)
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_val_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        let deposit = make_validator_deposit(new_val_kp, 32_000_000_000, slot, &spec);
        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Not a builder deposit, valid signature → kept
        assert_eq!(gloas.builders.len(), 0);
        assert_eq!(gloas.pending_deposits.len(), 1);
    }

    #[test]
    fn upgrade_new_deposit_with_invalid_signature_dropped() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        // 0x01 credentials (not builder), but bad signature
        let mut creds = [0u8; 32];
        creds[0] = 0x01;
        let deposit = PendingDeposit {
            pubkey: new_kp.pk.compress(),
            withdrawal_credentials: Hash256::from_slice(&creds),
            amount: 32_000_000_000,
            signature: SignatureBytes::empty(), // invalid signature
            slot,
        };
        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Bad signature → dropped (not builder, not valid validator deposit)
        assert_eq!(gloas.builders.len(), 0);
        assert_eq!(gloas.pending_deposits.len(), 0);
    }

    #[test]
    fn upgrade_builder_deposit_epoch_set_from_slot() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        let deposit = make_builder_deposit(builder_kp, 10_000_000_000, slot, &spec);
        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        let b0 = gloas.builders.get(0).unwrap();
        assert_eq!(b0.deposit_epoch, slot.epoch(E::slots_per_epoch()));
        assert_eq!(b0.withdrawable_epoch, spec.far_future_epoch);
    }

    // ========================================================================
    // Builder pubkey cache consistency and deposit edge cases
    // ========================================================================

    /// After upgrade with builder deposits, the builder_pubkey_cache must map
    /// each builder's pubkey to its correct index. A stale or empty cache would
    /// cause top-up deposits to create duplicate builders instead of adding to
    /// the existing balance.
    #[test]
    fn upgrade_builder_pubkey_cache_populated_correctly() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 3);
        let builder_kp1 = &extra_kps[NUM_VALIDATORS];
        let builder_kp2 = &extra_kps[NUM_VALIDATORS + 1];
        let builder_kp3 = &extra_kps[NUM_VALIDATORS + 2];
        let slot = state.slot();

        let d1 = make_builder_deposit(builder_kp1, 5_000_000_000, slot, &spec);
        let d2 = make_builder_deposit(builder_kp2, 6_000_000_000, slot, &spec);
        let d3 = make_builder_deposit(builder_kp3, 7_000_000_000, slot, &spec);

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![d1, d2, d3]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builders.len(), 3);

        // Each builder's pubkey should be in the cache with the correct index
        assert_eq!(
            gloas.builder_pubkey_cache.get(&builder_kp1.pk.compress()),
            Some(0),
            "builder 0 pubkey should map to index 0"
        );
        assert_eq!(
            gloas.builder_pubkey_cache.get(&builder_kp2.pk.compress()),
            Some(1),
            "builder 1 pubkey should map to index 1"
        );
        assert_eq!(
            gloas.builder_pubkey_cache.get(&builder_kp3.pk.compress()),
            Some(2),
            "builder 2 pubkey should map to index 2"
        );
    }

    /// A builder deposit (0x03 credentials) with an invalid signature is silently
    /// dropped — no builder is created, no error is returned. This is important
    /// because during the fork upgrade we cannot reject the entire transition due
    /// to a single bad deposit; we must skip it and continue processing.
    #[test]
    fn upgrade_builder_deposit_invalid_signature_dropped() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        // Build a deposit with 0x03 credentials but an empty (invalid) signature.
        let mut creds = [0u8; 32];
        creds[0] = 0x03; // BUILDER_WITHDRAWAL_PREFIX
        creds[12..].copy_from_slice(&[0xDD; 20]);
        let deposit = PendingDeposit {
            pubkey: builder_kp.pk.compress(),
            withdrawal_credentials: Hash256::from_slice(&creds),
            amount: 10_000_000_000,
            signature: SignatureBytes::empty(), // invalid
            slot,
        };

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Bad signature → builder NOT created
        assert_eq!(gloas.builders.len(), 0);
        // Deposit consumed (not kept in pending — it was a builder deposit attempt)
        assert_eq!(gloas.pending_deposits.len(), 0);
    }

    /// Two pending deposits for the same NEW validator pubkey (0x01 credentials)
    /// should both be kept in pending_deposits. The second deposit must not be
    /// misclassified as a builder deposit because the first deposit added the
    /// pubkey to new_validator_pubkeys. Without this tracking, the second deposit
    /// would fall through to the builder/signature check and potentially be dropped.
    #[test]
    fn upgrade_two_deposits_same_new_validator_pubkey_both_kept() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_val_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        let d1 = make_validator_deposit(new_val_kp, 32_000_000_000, slot, &spec);
        let d2 = make_validator_deposit(new_val_kp, 1_000_000_000, slot, &spec);

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![d1, d2]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // No builders created
        assert_eq!(gloas.builders.len(), 0);
        // Both deposits kept (second recognized via new_validator_pubkeys)
        assert_eq!(gloas.pending_deposits.len(), 2);
    }

    /// When a builder's second deposit arrives during the same fork upgrade, the
    /// pubkey cache hit triggers the top-up path which does NOT re-verify the
    /// signature. This test deposits 5 ETH (valid sig) then 3 ETH (invalid sig)
    /// for the same builder pubkey: both should succeed, total balance = 8 ETH.
    /// The top-up path must not check the signature because deposit top-ups in
    /// the spec are unconditional once the builder exists.
    #[test]
    fn upgrade_builder_topup_skips_signature_verification() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        // First deposit: valid signature → creates builder
        let d1 = make_builder_deposit(builder_kp, 5_000_000_000, slot, &spec);

        // Second deposit: same pubkey, 0x03 credentials, but INVALID signature.
        // The top-up path should accept this because the builder already exists.
        let mut creds = [0u8; 32];
        creds[0] = 0x03;
        creds[12..].copy_from_slice(&[0xDD; 20]);
        let d2 = PendingDeposit {
            pubkey: builder_kp.pk.compress(),
            withdrawal_credentials: Hash256::from_slice(&creds),
            amount: 3_000_000_000,
            signature: SignatureBytes::empty(), // invalid, but top-up skips verification
            slot,
        };

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![d1, d2]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Only one builder, balance is the sum of both deposits
        assert_eq!(gloas.builders.len(), 1);
        assert_eq!(gloas.builders.get(0).unwrap().balance, 8_000_000_000);
    }

    /// A comprehensive ordering test: validator deposit, builder deposit, another
    /// validator deposit for a different pubkey, another builder deposit. Verify
    /// that builder indices are assigned in deposit order and validator deposits
    /// are preserved in their original order.
    #[test]
    fn upgrade_deposit_ordering_preserved() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 4);
        let new_val_kp1 = &extra_kps[NUM_VALIDATORS];
        let builder_kp1 = &extra_kps[NUM_VALIDATORS + 1];
        let new_val_kp2 = &extra_kps[NUM_VALIDATORS + 2];
        let builder_kp2 = &extra_kps[NUM_VALIDATORS + 3];
        let slot = state.slot();

        let val_d1 = make_validator_deposit(new_val_kp1, 32_000_000_000, slot, &spec);
        let builder_d1 = make_builder_deposit(builder_kp1, 5_000_000_000, slot, &spec);
        let val_d2 = make_validator_deposit(new_val_kp2, 32_000_000_000, slot, &spec);
        let builder_d2 = make_builder_deposit(builder_kp2, 8_000_000_000, slot, &spec);

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![val_d1, builder_d1, val_d2, builder_d2]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();

        // Two builders created, in deposit order
        assert_eq!(gloas.builders.len(), 2);
        assert_eq!(
            gloas.builders.get(0).unwrap().pubkey,
            builder_kp1.pk.compress()
        );
        assert_eq!(gloas.builders.get(0).unwrap().balance, 5_000_000_000);
        assert_eq!(
            gloas.builders.get(1).unwrap().pubkey,
            builder_kp2.pk.compress()
        );
        assert_eq!(gloas.builders.get(1).unwrap().balance, 8_000_000_000);

        // Two validator deposits preserved in original order
        assert_eq!(gloas.pending_deposits.len(), 2);
        assert_eq!(
            gloas.pending_deposits.get(0).unwrap().pubkey,
            new_val_kp1.pk.compress()
        );
        assert_eq!(
            gloas.pending_deposits.get(1).unwrap().pubkey,
            new_val_kp2.pk.compress()
        );
    }

    // ========================================================================
    // Edge cases: validator/builder routing boundaries
    // ========================================================================

    /// An existing validator's pubkey with 0x03 (builder) credentials should still
    /// be routed as a validator deposit — the `is_validator` check comes before
    /// the credentials check. Without this priority order, a validator could
    /// accidentally become a builder during upgrade.
    #[test]
    fn upgrade_existing_validator_with_builder_credentials_stays_pending() {
        let (mut state, spec) = make_fulu_state();
        let keypairs = generate_deterministic_keypairs(NUM_VALIDATORS);
        let val_kp = &keypairs[0]; // existing validator
        let slot = state.slot();

        // Deposit with validator's pubkey but builder (0x03) credentials
        let mut creds = [0u8; 32];
        creds[0] = 0x03; // BUILDER_WITHDRAWAL_PREFIX
        creds[12..].copy_from_slice(&[0xDD; 20]);
        let deposit = PendingDeposit {
            pubkey: val_kp.pk.compress(),
            withdrawal_credentials: Hash256::from_slice(&creds),
            amount: 5_000_000_000,
            signature: SignatureBytes::empty(),
            slot,
        };

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // Must NOT create a builder — this pubkey belongs to a validator
        assert_eq!(gloas.builders.len(), 0);
        // Deposit kept in pending as validator deposit
        assert_eq!(gloas.pending_deposits.len(), 1);
    }

    /// A new validator deposit (0x01 credentials, valid sig) followed by a builder
    /// deposit (0x03 credentials) with the SAME pubkey: the second deposit should
    /// be treated as a validator deposit because the pubkey was tracked in
    /// `new_validator_pubkeys` by the first deposit.
    #[test]
    fn upgrade_new_validator_then_builder_deposit_same_pubkey_both_pending() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        // First: valid validator deposit → adds pubkey to new_validator_pubkeys
        let val_d = make_validator_deposit(kp, 32_000_000_000, slot, &spec);

        // Second: same pubkey but with 0x03 credentials → should be routed as
        // validator because is_validator check (via new_validator_pubkeys) wins
        let mut creds = [0u8; 32];
        creds[0] = 0x03;
        creds[12..].copy_from_slice(&[0xDD; 20]);
        let builder_d = PendingDeposit {
            pubkey: kp.pk.compress(),
            withdrawal_credentials: Hash256::from_slice(&creds),
            amount: 5_000_000_000,
            signature: SignatureBytes::empty(),
            slot,
        };

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![val_d, builder_d]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        // No builders created — both deposits route as validator
        assert_eq!(gloas.builders.len(), 0);
        // Both deposits kept in pending
        assert_eq!(gloas.pending_deposits.len(), 2);
    }

    // ========================================================================
    // Edge cases: builder balance and slot reuse
    // ========================================================================

    /// Top-up balance should saturate at u64::MAX instead of wrapping.
    #[test]
    fn upgrade_builder_topup_balance_saturates_at_max() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        // First deposit: near-max balance
        let d1 = make_builder_deposit(builder_kp, u64::MAX - 1_000, slot, &spec);
        // Second deposit: would overflow without saturation
        let d2 = make_builder_deposit(builder_kp, 5_000, slot, &spec);

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![d1, d2]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builders.len(), 1);
        assert_eq!(gloas.builders.get(0).unwrap().balance, u64::MAX);
    }

    /// Top-up for a builder created earlier in the same upgrade batch with
    /// multiple builders present — ensures the pubkey cache lookup returns
    /// the correct index, not index 0 by default.
    #[test]
    fn upgrade_topup_targets_correct_builder_among_multiple() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 3);
        let builder_kp1 = &extra_kps[NUM_VALIDATORS];
        let builder_kp2 = &extra_kps[NUM_VALIDATORS + 1];
        let builder_kp3 = &extra_kps[NUM_VALIDATORS + 2];
        let slot = state.slot();

        // Create 3 builders, then top up the middle one
        let d1 = make_builder_deposit(builder_kp1, 5_000_000_000, slot, &spec);
        let d2 = make_builder_deposit(builder_kp2, 6_000_000_000, slot, &spec);
        let d3 = make_builder_deposit(builder_kp3, 7_000_000_000, slot, &spec);
        let topup = make_builder_deposit(builder_kp2, 1_000_000_000, slot, &spec);

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![d1, d2, d3, topup]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builders.len(), 3);
        // Builder 0 and 2 unchanged
        assert_eq!(gloas.builders.get(0).unwrap().balance, 5_000_000_000);
        assert_eq!(gloas.builders.get(2).unwrap().balance, 7_000_000_000);
        // Builder 1 got the top-up
        assert_eq!(
            gloas.builders.get(1).unwrap().balance,
            7_000_000_000, // 6B + 1B
        );
    }

    /// A zero-amount builder deposit with valid signature should still create
    /// a builder with zero balance. The spec does not require a minimum deposit
    /// for builder creation during upgrade.
    #[test]
    fn upgrade_zero_amount_builder_deposit_creates_builder() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];
        let slot = state.slot();

        let deposit = make_builder_deposit(builder_kp, 0, slot, &spec);
        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![deposit]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builders.len(), 1);
        assert_eq!(gloas.builders.get(0).unwrap().balance, 0);
        assert_eq!(
            gloas.builders.get(0).unwrap().pubkey,
            builder_kp.pk.compress()
        );
    }

    /// When only invalid deposits exist (bad signatures, no builder/validator
    /// match), all are dropped and both builders list and pending_deposits are
    /// empty after upgrade.
    #[test]
    fn upgrade_all_invalid_deposits_dropped() {
        let (mut state, spec) = make_fulu_state();
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 2);
        let kp1 = &extra_kps[NUM_VALIDATORS];
        let kp2 = &extra_kps[NUM_VALIDATORS + 1];
        let slot = state.slot();

        // Two deposits: both have 0x01 credentials (not builder) and invalid sigs
        let d1 = PendingDeposit {
            pubkey: kp1.pk.compress(),
            withdrawal_credentials: Hash256::repeat_byte(0x01),
            amount: 32_000_000_000,
            signature: SignatureBytes::empty(), // invalid
            slot,
        };
        let d2 = PendingDeposit {
            pubkey: kp2.pk.compress(),
            withdrawal_credentials: Hash256::repeat_byte(0x01),
            amount: 32_000_000_000,
            signature: SignatureBytes::empty(), // invalid
            slot,
        };

        let fulu = state.as_fulu_mut().unwrap();
        fulu.pending_deposits = List::new(vec![d1, d2]).unwrap();

        upgrade_to_gloas(&mut state, &spec).unwrap();

        let gloas = state.as_gloas().unwrap();
        assert_eq!(gloas.builders.len(), 0);
        assert_eq!(gloas.pending_deposits.len(), 0);
    }
}
