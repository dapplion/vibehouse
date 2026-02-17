use crate::per_block_processing::is_valid_deposit_signature;
use ssz_types::BitVector;
use ssz_types::typenum::Unsigned;
use std::mem;
use types::{
    Address, BeaconState, BeaconStateError as Error, BeaconStateGloas, Builder,
    BuilderPendingPayment, ChainSpec, DepositData, EthSpec, ExecutionPayloadBid, Fork, List,
    PublicKeyBytes, Vector,
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

pub fn upgrade_state_to_gloas<E: EthSpec>(
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

        // Check if it's an existing builder or has builder credentials
        let state_gloas = state.as_gloas().map_err(|_| Error::IncorrectStateVariant)?;
        let is_existing_builder = state_gloas.builders.iter().any(|b| b.pubkey == deposit.pubkey);
        let has_builder_credentials = deposit
            .withdrawal_credentials
            .as_slice()
            .first()
            .copied()
            == Some(0x03); // BUILDER_WITHDRAWAL_PREFIX

        if is_existing_builder || has_builder_credentials {
            // Apply as builder deposit
            apply_builder_deposit::<E>(state, deposit.pubkey, deposit.withdrawal_credentials, deposit.amount, &deposit.signature, deposit.slot, spec)?;
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
    let state_gloas = state.as_gloas_mut().map_err(|_| Error::IncorrectStateVariant)?;

    // Check if builder already exists
    let builder_index = state_gloas
        .builders
        .iter()
        .position(|b| b.pubkey == pubkey);

    if let Some(index) = builder_index {
        // Top-up existing builder
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
            let address_bytes = cred_slice
                .get(12..)
                .ok_or(Error::IncorrectStateVariant)?;

            let builder = Builder {
                pubkey,
                version,
                execution_address: Address::from_slice(address_bytes),
                balance: amount,
                deposit_epoch: slot.epoch(E::slots_per_epoch()),
                withdrawable_epoch: spec.far_future_epoch,
            };

            if new_index < state_gloas.builders.len() {
                *state_gloas
                    .builders
                    .get_mut(new_index)
                    .ok_or(Error::UnknownValidator(new_index))? = builder;
            } else {
                state_gloas
                    .builders
                    .push(builder)
                    .map_err(Error::MilhouseError)?;
            }
        }
    }

    Ok(())
}
