use super::*;
use crate::VerifySignatures;
use crate::common::{
    get_attestation_participation_flag_indices, increase_balance, initiate_validator_exit,
    is_attestation_same_slot, slash_validator,
};
use crate::per_block_processing::errors::{BlockProcessingError, IntoWithIndex};
use types::BuilderPendingPayment;
use types::consts::altair::{PARTICIPATION_FLAG_WEIGHTS, PROPOSER_WEIGHT, WEIGHT_DENOMINATOR};
use types::typenum::U33;

pub fn process_operations<E: EthSpec, Payload: AbstractExecPayload<E>>(
    state: &mut BeaconState<E>,
    block_body: BeaconBlockBodyRef<E, Payload>,
    verify_signatures: VerifySignatures,
    ctxt: &mut ConsensusContext<E>,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    process_proposer_slashings(
        state,
        block_body.proposer_slashings(),
        verify_signatures,
        ctxt,
        spec,
    )?;
    process_attester_slashings(
        state,
        block_body.attester_slashings(),
        verify_signatures,
        ctxt,
        spec,
    )?;
    process_attestations(state, block_body, verify_signatures, ctxt, spec)?;
    process_deposits(state, block_body.deposits(), spec)?;
    process_exits(state, block_body.voluntary_exits(), verify_signatures, spec)?;

    if let Ok(bls_to_execution_changes) = block_body.bls_to_execution_changes() {
        process_bls_to_execution_changes(state, bls_to_execution_changes, verify_signatures, spec)?;
    }

    // Execution requests exist in Electra and Fulu, but not in Gloas (ePBS removes them
    // from the block body since there is no execution payload in the proposer's block).
    if let Ok(execution_requests) = block_body.execution_requests() {
        state.update_pubkey_cache()?;
        process_deposit_requests(state, &execution_requests.deposits, spec)?;
        process_withdrawal_requests(state, &execution_requests.withdrawals, spec)?;
        process_consolidation_requests(state, &execution_requests.consolidations, spec)?;
    }

    // Gloas ePBS operations
    // Note: process_execution_payload_bid is called from per_block_processing before
    // process_randao (not here) because the bid verification depends on the randao mix
    // from the previous block.
    if state.fork_name_unchecked().gloas_enabled() {
        // Process payload attestations
        if let Ok(attestations) = block_body.payload_attestations() {
            for attestation in attestations.iter() {
                gloas::process_payload_attestation(state, attestation, verify_signatures, spec)?;
            }
        }
    }

    Ok(())
}

pub mod base {
    use super::*;

    /// Validates each `Attestation` and updates the state, short-circuiting on an invalid object.
    ///
    /// Returns `Ok(())` if the validation and state updates completed successfully, otherwise returns
    /// an `Err` describing the invalid object or cause of failure.
    pub fn process_attestations<'a, E: EthSpec, I>(
        state: &mut BeaconState<E>,
        attestations: I,
        verify_signatures: VerifySignatures,
        ctxt: &mut ConsensusContext<E>,
        spec: &ChainSpec,
    ) -> Result<(), BlockProcessingError>
    where
        I: Iterator<Item = AttestationRef<'a, E>>,
    {
        // Ensure required caches are all built. These should be no-ops during regular operation.
        state.build_committee_cache(RelativeEpoch::Current, spec)?;
        state.build_committee_cache(RelativeEpoch::Previous, spec)?;
        initialize_epoch_cache(state, spec)?;
        initialize_progressive_balances_cache(state, spec)?;
        state.build_slashings_cache()?;

        let proposer_index = ctxt.get_proposer_index(state, spec)?;

        // Verify and apply each attestation.
        for (i, attestation) in attestations.enumerate() {
            verify_attestation_for_block_inclusion(
                state,
                attestation,
                ctxt,
                verify_signatures,
                spec,
            )
            .map_err(|e| e.into_with_index(i))?;

            let AttestationRef::Base(attestation) = attestation else {
                // Pending attestations have been deprecated in a altair, this branch should
                // never happen
                return Err(BlockProcessingError::PendingAttestationInElectra);
            };

            let pending_attestation = PendingAttestation {
                aggregation_bits: attestation.aggregation_bits.clone(),
                data: attestation.data.clone(),
                inclusion_delay: state.slot().safe_sub(attestation.data.slot)?.as_u64(),
                proposer_index,
            };

            if attestation.data.target.epoch == state.current_epoch() {
                state
                    .as_base_mut()?
                    .current_epoch_attestations
                    .push(pending_attestation)?;
            } else {
                state
                    .as_base_mut()?
                    .previous_epoch_attestations
                    .push(pending_attestation)?;
            }
        }

        Ok(())
    }
}

pub mod altair_deneb {
    use super::*;
    use crate::common::update_progressive_balances_cache::update_progressive_balances_on_attestation;

    pub fn process_attestations<'a, E: EthSpec, I>(
        state: &mut BeaconState<E>,
        attestations: I,
        verify_signatures: VerifySignatures,
        ctxt: &mut ConsensusContext<E>,
        spec: &ChainSpec,
    ) -> Result<(), BlockProcessingError>
    where
        I: Iterator<Item = AttestationRef<'a, E>>,
    {
        attestations.enumerate().try_for_each(|(i, attestation)| {
            process_attestation(state, attestation, i, ctxt, verify_signatures, spec)
        })
    }

    pub fn process_attestation<E: EthSpec>(
        state: &mut BeaconState<E>,
        attestation: AttestationRef<E>,
        att_index: usize,
        ctxt: &mut ConsensusContext<E>,
        verify_signatures: VerifySignatures,
        spec: &ChainSpec,
    ) -> Result<(), BlockProcessingError> {
        let proposer_index = ctxt.get_proposer_index(state, spec)?;
        let previous_epoch = ctxt.previous_epoch;
        let current_epoch = ctxt.current_epoch;

        let indexed_att = verify_attestation_for_block_inclusion(
            state,
            attestation,
            ctxt,
            verify_signatures,
            spec,
        )
        .map_err(|e| e.into_with_index(att_index))?;

        // Matching roots, participation flag indices
        let data = attestation.data();
        let inclusion_delay = state.slot().safe_sub(data.slot)?.as_u64();
        let participation_flag_indices =
            get_attestation_participation_flag_indices(state, data, inclusion_delay, spec)?;

        // [New in Gloas:EIP7732] Pre-compute whether this is a same-slot attestation
        let is_gloas = state.fork_name_unchecked().gloas_enabled();
        let same_slot = if is_gloas {
            is_attestation_same_slot(state, data)?
        } else {
            false
        };

        // Update epoch participation flags.
        let mut proposer_reward_numerator = 0;
        for index in indexed_att.attesting_indices_iter() {
            let index = *index as usize;

            let validator_effective_balance = state.epoch_cache().get_effective_balance(index)?;
            let validator_slashed = state.slashings_cache().is_slashed(index);

            // [New in Gloas:EIP7732] Track if any new flag is set for this validator
            let mut will_set_new_flag = false;

            for (flag_index, &weight) in PARTICIPATION_FLAG_WEIGHTS.iter().enumerate() {
                let epoch_participation = state.get_epoch_participation_mut(
                    data.target.epoch,
                    previous_epoch,
                    current_epoch,
                )?;

                if participation_flag_indices.contains(&flag_index) {
                    let validator_participation = epoch_participation
                        .get_mut(index)
                        .ok_or(BeaconStateError::ParticipationOutOfBounds(index))?;

                    if !validator_participation.has_flag(flag_index)? {
                        validator_participation.add_flag(flag_index)?;
                        proposer_reward_numerator
                            .safe_add_assign(state.get_base_reward(index)?.safe_mul(weight)?)?;

                        // [New in Gloas:EIP7732]
                        will_set_new_flag = true;

                        update_progressive_balances_on_attestation(
                            state,
                            data.target.epoch,
                            flag_index,
                            validator_effective_balance,
                            validator_slashed,
                        )?;
                    }
                }
            }

            // [New in Gloas:EIP7732] Add weight for same-slot attestations
            if is_gloas && will_set_new_flag && same_slot {
                let slots_per_epoch = E::slots_per_epoch();
                let slot_mod = data.slot.as_u64().safe_rem(slots_per_epoch)?;
                let payment_slot_index = if data.target.epoch == current_epoch {
                    slots_per_epoch.safe_add(slot_mod)? as usize
                } else {
                    slot_mod as usize
                };

                if let Ok(state_gloas) = state.as_gloas_mut()
                    && let Some(payment) = state_gloas
                        .builder_pending_payments
                        .get_mut(payment_slot_index)
                    && payment.withdrawal.amount > 0
                {
                    payment.weight = payment.weight.saturating_add(validator_effective_balance);
                }
            }
        }

        let proposer_reward_denominator = WEIGHT_DENOMINATOR
            .safe_sub(PROPOSER_WEIGHT)?
            .safe_mul(WEIGHT_DENOMINATOR)?
            .safe_div(PROPOSER_WEIGHT)?;
        let proposer_reward = proposer_reward_numerator.safe_div(proposer_reward_denominator)?;
        increase_balance(state, proposer_index as usize, proposer_reward)?;
        Ok(())
    }
}

/// Validates each `ProposerSlashing` and updates the state, short-circuiting on an invalid object.
///
/// Returns `Ok(())` if the validation and state updates completed successfully, otherwise returns
/// an `Err` describing the invalid object or cause of failure.
pub fn process_proposer_slashings<E: EthSpec>(
    state: &mut BeaconState<E>,
    proposer_slashings: &[ProposerSlashing],
    verify_signatures: VerifySignatures,
    ctxt: &mut ConsensusContext<E>,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    state.build_slashings_cache()?;

    // Verify and apply proposer slashings in series.
    // We have to verify in series because an invalid block may contain multiple slashings
    // for the same validator, and we need to correctly detect and reject that.
    proposer_slashings
        .iter()
        .enumerate()
        .try_for_each(|(i, proposer_slashing)| {
            verify_proposer_slashing(proposer_slashing, state, verify_signatures, spec)
                .map_err(|e| e.into_with_index(i))?;

            let proposer_index = proposer_slashing.signed_header_1.message.proposer_index as usize;

            slash_validator(state, proposer_index, None, ctxt, spec)?;

            // [New in Gloas:EIP7732] Remove the BuilderPendingPayment for this proposal
            // if it is still in the 2-epoch window.
            if state.fork_name_unchecked().gloas_enabled() {
                let slot = proposer_slashing.signed_header_1.message.slot;
                let slots_per_epoch = E::slots_per_epoch();
                let proposal_epoch = slot.epoch(slots_per_epoch);
                let current_epoch = state.current_epoch();
                let previous_epoch = current_epoch.saturating_sub(1u64);

                let slot_mod = slot.as_u64().safe_rem(slots_per_epoch)?;
                let payment_index = if proposal_epoch == current_epoch {
                    Some(slots_per_epoch.safe_add(slot_mod)? as usize)
                } else if proposal_epoch == previous_epoch {
                    Some(slot_mod as usize)
                } else {
                    None
                };

                if let Some(idx) = payment_index {
                    let state_gloas = state
                        .as_gloas_mut()
                        .map_err(BlockProcessingError::BeaconStateError)?;
                    if let Some(payment) = state_gloas.builder_pending_payments.get_mut(idx) {
                        *payment = BuilderPendingPayment::default();
                    }
                }
            }

            Ok(())
        })
}

/// Validates each `AttesterSlashing` and updates the state, short-circuiting on an invalid object.
///
/// Returns `Ok(())` if the validation and state updates completed successfully, otherwise returns
/// an `Err` describing the invalid object or cause of failure.
pub fn process_attester_slashings<'a, E: EthSpec, I>(
    state: &mut BeaconState<E>,
    attester_slashings: I,
    verify_signatures: VerifySignatures,
    ctxt: &mut ConsensusContext<E>,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError>
where
    I: Iterator<Item = AttesterSlashingRef<'a, E>>,
{
    state.build_slashings_cache()?;

    for (i, attester_slashing) in attester_slashings.enumerate() {
        let slashable_indices =
            verify_attester_slashing(state, attester_slashing, verify_signatures, spec)
                .map_err(|e| e.into_with_index(i))?;

        for i in slashable_indices {
            slash_validator(state, i as usize, None, ctxt, spec)?;
        }
    }

    Ok(())
}

/// Wrapper function to handle calling the correct version of `process_attestations` based on
/// the fork.
pub fn process_attestations<E: EthSpec, Payload: AbstractExecPayload<E>>(
    state: &mut BeaconState<E>,
    block_body: BeaconBlockBodyRef<E, Payload>,
    verify_signatures: VerifySignatures,
    ctxt: &mut ConsensusContext<E>,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    if state.fork_name_unchecked().altair_enabled() {
        altair_deneb::process_attestations(
            state,
            block_body.attestations(),
            verify_signatures,
            ctxt,
            spec,
        )?;
    } else {
        base::process_attestations(
            state,
            block_body.attestations(),
            verify_signatures,
            ctxt,
            spec,
        )?;
    }
    Ok(())
}

/// Validates each `Exit` and updates the state, short-circuiting on an invalid object.
///
/// Returns `Ok(())` if the validation and state updates completed successfully, otherwise returns
/// an `Err` describing the invalid object or cause of failure.
pub fn process_exits<E: EthSpec>(
    state: &mut BeaconState<E>,
    voluntary_exits: &[SignedVoluntaryExit],
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    // Verify and apply each exit in series. We iterate in series because higher-index exits may
    // become invalid due to the application of lower-index ones.
    for (i, exit) in voluntary_exits.iter().enumerate() {
        let is_builder_exit = verify_exit(state, None, exit, verify_signatures, spec)
            .map_err(|e| e.into_with_index(i))?;

        if is_builder_exit {
            // [New in Gloas:EIP7732] Builder exit path
            let builder_index =
                exit.message.validator_index & !types::consts::gloas::BUILDER_INDEX_FLAG;
            gloas::initiate_builder_exit(state, builder_index, spec)?;
        } else {
            initiate_validator_exit(state, exit.message.validator_index as usize, spec)?;
        }
    }
    Ok(())
}

/// Validates each `bls_to_execution_change` and updates the state
///
/// Returns `Ok(())` if the validation and state updates completed successfully. Otherwise returns
/// an `Err` describing the invalid object or cause of failure.
pub fn process_bls_to_execution_changes<E: EthSpec>(
    state: &mut BeaconState<E>,
    bls_to_execution_changes: &[SignedBlsToExecutionChange],
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    for (i, signed_address_change) in bls_to_execution_changes.iter().enumerate() {
        verify_bls_to_execution_change(state, signed_address_change, verify_signatures, spec)
            .map_err(|e| e.into_with_index(i))?;

        state
            .get_validator_mut(signed_address_change.message.validator_index as usize)?
            .change_withdrawal_credentials(
                &signed_address_change.message.to_execution_address,
                spec,
            );
    }

    Ok(())
}

/// Validates each `Deposit` and updates the state, short-circuiting on an invalid object.
///
/// Returns `Ok(())` if the validation and state updates completed successfully, otherwise returns
/// an `Err` describing the invalid object or cause of failure.
pub fn process_deposits<E: EthSpec>(
    state: &mut BeaconState<E>,
    deposits: &[Deposit],
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    // [Modified in Electra:EIP6110]
    // Disable former deposit mechanism once all prior deposits are processed
    let deposit_requests_start_index = state.deposit_requests_start_index().unwrap_or(u64::MAX);
    let eth1_deposit_index_limit = std::cmp::min(
        deposit_requests_start_index,
        state.eth1_data().deposit_count,
    );

    if state.eth1_deposit_index() < eth1_deposit_index_limit {
        let expected_deposit_len = std::cmp::min(
            E::MaxDeposits::to_u64(),
            eth1_deposit_index_limit.safe_sub(state.eth1_deposit_index())?,
        );
        block_verify!(
            deposits.len() as u64 == expected_deposit_len,
            BlockProcessingError::DepositCountInvalid {
                expected: expected_deposit_len as usize,
                found: deposits.len(),
            }
        );
    } else {
        block_verify!(
            deposits.len() as u64 == 0,
            BlockProcessingError::DepositCountInvalid {
                expected: 0,
                found: deposits.len(),
            }
        );
    }

    // Verify merkle proofs in parallel.
    deposits
        .par_iter()
        .enumerate()
        .try_for_each(|(i, deposit)| {
            verify_deposit_merkle_proof(
                state,
                deposit,
                state.eth1_deposit_index().safe_add(i as u64)?,
                spec,
            )
            .map_err(|e| e.into_with_index(i))
        })?;

    // Update the state in series.
    for deposit in deposits {
        apply_deposit(state, deposit.data.clone(), None, true, spec)?;
    }

    Ok(())
}

/// Process a single deposit, verifying its merkle proof if provided.
pub fn apply_deposit<E: EthSpec>(
    state: &mut BeaconState<E>,
    deposit_data: DepositData,
    proof: Option<FixedVector<Hash256, U33>>,
    increment_eth1_deposit_index: bool,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    let deposit_index = state.eth1_deposit_index() as usize;
    if let Some(proof) = proof {
        let deposit = Deposit {
            proof,
            data: deposit_data.clone(),
        };
        verify_deposit_merkle_proof(state, &deposit, state.eth1_deposit_index(), spec)
            .map_err(|e| e.into_with_index(deposit_index))?;
    }

    if increment_eth1_deposit_index {
        state.eth1_deposit_index_mut().safe_add_assign(1)?;
    }

    // Get an `Option<u64>` where `u64` is the validator index if this deposit public key
    // already exists in the beacon_state.
    let validator_index = get_existing_validator_index(state, &deposit_data.pubkey)
        .map_err(|e| e.into_with_index(deposit_index))?;

    let amount = deposit_data.amount;

    if let Some(index) = validator_index {
        // [Modified in Electra:EIP7251]
        if let Ok(pending_deposits) = state.pending_deposits_mut() {
            pending_deposits.push(PendingDeposit {
                pubkey: deposit_data.pubkey,
                withdrawal_credentials: deposit_data.withdrawal_credentials,
                amount,
                signature: deposit_data.signature,
                slot: spec.genesis_slot, // Use `genesis_slot` to distinguish from a pending deposit request
            })?;
        } else {
            // Update the existing validator balance.
            increase_balance(state, index as usize, amount)?;
        }
    }
    // New validator
    else {
        // The signature should be checked for new validators. Return early for a bad
        // signature.
        if is_valid_deposit_signature(&deposit_data, spec).is_err() {
            return Ok(());
        }

        state.add_validator_to_registry(
            deposit_data.pubkey,
            deposit_data.withdrawal_credentials,
            if state.fork_name_unchecked() >= ForkName::Electra {
                0
            } else {
                amount
            },
            spec,
        )?;

        // [New in Electra:EIP7251]
        if let Ok(pending_deposits) = state.pending_deposits_mut() {
            pending_deposits.push(PendingDeposit {
                pubkey: deposit_data.pubkey,
                withdrawal_credentials: deposit_data.withdrawal_credentials,
                amount,
                signature: deposit_data.signature,
                slot: spec.genesis_slot, // Use `genesis_slot` to distinguish from a pending deposit request
            })?;
        }
    }

    Ok(())
}

// Make sure to build the pubkey cache before calling this function
pub fn process_withdrawal_requests<E: EthSpec>(
    state: &mut BeaconState<E>,
    requests: &[WithdrawalRequest],
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    for request in requests {
        let amount = request.amount;
        let is_full_exit_request = amount == spec.full_exit_request_amount;

        // If partial withdrawal queue is full, only full exits are processed
        if state.pending_partial_withdrawals()?.len() == E::pending_partial_withdrawals_limit()
            && !is_full_exit_request
        {
            continue;
        }

        // Verify pubkey exists
        let Some(validator_index) = state.pubkey_cache().get(&request.validator_pubkey) else {
            continue;
        };

        let validator = state.get_validator(validator_index)?;
        // Verify withdrawal credentials
        let has_correct_credential = validator.has_execution_withdrawal_credential(spec);
        let is_correct_source_address = validator
            .get_execution_withdrawal_address(spec)
            .map(|addr| addr == request.source_address)
            .unwrap_or(false);

        if !(has_correct_credential && is_correct_source_address) {
            continue;
        }

        // Verify the validator is active
        if !validator.is_active_at(state.current_epoch()) {
            continue;
        }

        // Verify exit has not been initiated
        if validator.exit_epoch != spec.far_future_epoch {
            continue;
        }

        // Verify the validator has been active long enough
        if state.current_epoch()
            < validator
                .activation_epoch
                .safe_add(spec.shard_committee_period)?
        {
            continue;
        }

        let pending_balance_to_withdraw = state.get_pending_balance_to_withdraw(validator_index)?;
        if is_full_exit_request {
            // Only exit validator if it has no pending withdrawals in the queue
            if pending_balance_to_withdraw == 0 {
                initiate_validator_exit(state, validator_index, spec)?
            }
            continue;
        }

        let balance = state.get_balance(validator_index)?;
        let has_sufficient_effective_balance =
            validator.effective_balance >= spec.min_activation_balance;
        let has_excess_balance = balance
            > spec
                .min_activation_balance
                .safe_add(pending_balance_to_withdraw)?;

        // Only allow partial withdrawals with compounding withdrawal credentials
        if validator.has_compounding_withdrawal_credential(spec)
            && has_sufficient_effective_balance
            && has_excess_balance
        {
            let to_withdraw = std::cmp::min(
                balance
                    .safe_sub(spec.min_activation_balance)?
                    .safe_sub(pending_balance_to_withdraw)?,
                amount,
            );
            let exit_queue_epoch = state.compute_exit_epoch_and_update_churn(to_withdraw, spec)?;
            let withdrawable_epoch =
                exit_queue_epoch.safe_add(spec.min_validator_withdrawability_delay)?;
            state
                .pending_partial_withdrawals_mut()?
                .push(PendingPartialWithdrawal {
                    validator_index: validator_index as u64,
                    amount: to_withdraw,
                    withdrawable_epoch,
                })?;
        }
    }
    Ok(())
}

pub fn process_deposit_requests<E: EthSpec>(
    state: &mut BeaconState<E>,
    deposit_requests: &[DepositRequest],
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    for request in deposit_requests {
        // [Modified in Gloas:EIP7732] Route builder deposits
        if state.fork_name_unchecked().gloas_enabled() {
            process_deposit_request_gloas(state, request, spec)?;
        } else {
            // Set deposit receipt start index [New in Electra:EIP6110]
            if state.deposit_requests_start_index()? == spec.unset_deposit_requests_start_index {
                *state.deposit_requests_start_index_mut()? = request.index
            }

            let slot = state.slot();

            // [New in Electra:EIP7251]
            if let Ok(pending_deposits) = state.pending_deposits_mut() {
                pending_deposits.push(PendingDeposit {
                    pubkey: request.pubkey,
                    withdrawal_credentials: request.withdrawal_credentials,
                    amount: request.amount,
                    signature: request.signature.clone(),
                    slot,
                })?;
            }
        }
    }

    Ok(())
}

/// [New in Gloas:EIP7732] Process a single deposit request, routing builder deposits.
fn process_deposit_request_gloas<E: EthSpec>(
    state: &mut BeaconState<E>,
    request: &DepositRequest,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    let slot = state.slot();

    // Check if pubkey belongs to an existing builder
    let is_builder = state
        .as_gloas()
        .map(|s| s.builders.iter().any(|b| b.pubkey == request.pubkey))
        .unwrap_or(false);

    // Check if pubkey belongs to an existing validator (finalized)
    let is_validator = state.pubkey_cache().get(&request.pubkey).is_some();

    let is_builder_prefix = is_builder_withdrawal_credential(request.withdrawal_credentials);

    // Route to builder if: existing builder OR (builder prefix AND not existing/pending validator)
    // Spec: process_deposit_request — check pending_deposits for a validly signed deposit
    // with this pubkey to avoid routing a pending validator's deposit to a builder.
    //
    // Note: is_pending_validator iterates all pending_deposits and re-verifies signatures.
    // The spec suggests caching these results for performance.
    if is_builder
        || (is_builder_prefix
            && !is_validator
            && !is_pending_validator(state, &request.pubkey, spec))
    {
        apply_deposit_for_builder(state, request, slot, spec)?;
    } else {
        // Add to pending validator deposits
        state.pending_deposits_mut()?.push(PendingDeposit {
            pubkey: request.pubkey,
            withdrawal_credentials: request.withdrawal_credentials,
            amount: request.amount,
            signature: request.signature.clone(),
            slot,
        })?;
    }

    Ok(())
}

/// [New in Gloas:EIP7732] Check if withdrawal credentials have builder prefix (0x03).
fn is_builder_withdrawal_credential(withdrawal_credentials: Hash256) -> bool {
    withdrawal_credentials.as_slice().first().copied() == Some(0x03)
}

/// [New in Gloas:EIP7732] Check if a pending deposit with a valid signature exists for this pubkey.
///
/// Iterates `state.pending_deposits` looking for a deposit matching the pubkey with a valid
/// BLS signature. Returns true as soon as one is found.
///
/// Spec note: implementations SHOULD cache verification results to avoid repeated work.
fn is_pending_validator<E: EthSpec>(
    state: &BeaconState<E>,
    pubkey: &PublicKeyBytes,
    spec: &ChainSpec,
) -> bool {
    let Ok(pending_deposits) = state.pending_deposits() else {
        return false;
    };
    for pending_deposit in pending_deposits.iter() {
        if pending_deposit.pubkey != *pubkey {
            continue;
        }
        let deposit_data = DepositData {
            pubkey: pending_deposit.pubkey,
            withdrawal_credentials: pending_deposit.withdrawal_credentials,
            amount: pending_deposit.amount,
            signature: pending_deposit.signature.clone(),
        };
        if is_valid_deposit_signature(&deposit_data, spec).is_ok() {
            return true;
        }
    }
    false
}

/// [New in Gloas:EIP7732] Apply a deposit for a builder (new or top-up).
fn apply_deposit_for_builder<E: EthSpec>(
    state: &mut BeaconState<E>,
    request: &DepositRequest,
    slot: Slot,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    let state_gloas = state
        .as_gloas_mut()
        .map_err(BlockProcessingError::BeaconStateError)?;

    // Check if builder already exists
    let builder_index = state_gloas
        .builders
        .iter()
        .position(|b| b.pubkey == request.pubkey);

    if let Some(index) = builder_index {
        // Top-up existing builder
        let builder = state_gloas
            .builders
            .get_mut(index)
            .ok_or(BeaconStateError::UnknownValidator(index))?;
        builder.balance = builder.balance.saturating_add(request.amount);
    } else {
        // New builder - verify deposit signature
        let deposit_data = DepositData {
            pubkey: request.pubkey,
            withdrawal_credentials: request.withdrawal_credentials,
            amount: request.amount,
            signature: request.signature.clone(),
        };

        if is_valid_deposit_signature(&deposit_data, spec).is_err() {
            return Ok(());
        }

        // Find slot for new builder (reuse exited builder slot or append)
        let current_epoch = state_gloas.slot.epoch(E::slots_per_epoch());
        let new_index = get_index_for_new_builder::<E>(&state_gloas.builders, current_epoch, spec);

        let cred_slice = request.withdrawal_credentials.as_slice();
        let version = cred_slice
            .first()
            .copied()
            .ok_or(BlockProcessingError::InvalidBuilderCredentials)?;
        let address_bytes = cred_slice
            .get(12..)
            .ok_or(BlockProcessingError::InvalidBuilderCredentials)?;

        let builder = types::Builder {
            pubkey: request.pubkey,
            version,
            execution_address: Address::from_slice(address_bytes),
            balance: request.amount,
            deposit_epoch: slot.epoch(E::slots_per_epoch()),
            withdrawable_epoch: spec.far_future_epoch,
        };

        if new_index < state_gloas.builders.len() {
            *state_gloas
                .builders
                .get_mut(new_index)
                .ok_or(BeaconStateError::UnknownValidator(new_index))? = builder;
        } else {
            state_gloas
                .builders
                .push(builder)
                .map_err(BlockProcessingError::MilhouseError)?;
        }
    }

    Ok(())
}

/// [New in Gloas:EIP7732] Find index for a new builder (reuse exited slot or append).
fn get_index_for_new_builder<E: EthSpec>(
    builders: &List<Builder, E::BuilderRegistryLimit>,
    current_epoch: Epoch,
    _spec: &ChainSpec,
) -> usize {
    for (index, builder) in builders.iter().enumerate() {
        if builder.withdrawable_epoch <= current_epoch && builder.balance == 0 {
            return index;
        }
    }
    builders.len()
}

// Make sure to build the pubkey cache before calling this function
pub fn process_consolidation_requests<E: EthSpec>(
    state: &mut BeaconState<E>,
    consolidation_requests: &[ConsolidationRequest],
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    for request in consolidation_requests {
        process_consolidation_request(state, request, spec)?;
    }

    Ok(())
}

fn is_valid_switch_to_compounding_request<E: EthSpec>(
    state: &BeaconState<E>,
    consolidation_request: &ConsolidationRequest,
    spec: &ChainSpec,
) -> Result<bool, BlockProcessingError> {
    // Switch to compounding requires source and target be equal
    if consolidation_request.source_pubkey != consolidation_request.target_pubkey {
        return Ok(false);
    }

    // Verify pubkey exists
    let Some(source_index) = state
        .pubkey_cache()
        .get(&consolidation_request.source_pubkey)
    else {
        // source validator doesn't exist
        return Ok(false);
    };

    let source_validator = state.get_validator(source_index)?;
    // Verify the source withdrawal credentials
    // Note: We need to specifically check for eth1 withdrawal credentials here
    // If the validator is already compounding, the compounding request is not valid.
    if let Some(withdrawal_address) = source_validator
        .has_eth1_withdrawal_credential(spec)
        .then(|| {
            source_validator
                .withdrawal_credentials
                .as_slice()
                .get(12..)
                .map(Address::from_slice)
        })
        .flatten()
    {
        if withdrawal_address != consolidation_request.source_address {
            return Ok(false);
        }
    } else {
        // Source doesn't have eth1 withdrawal credentials
        return Ok(false);
    }

    // Verify the source is active
    let current_epoch = state.current_epoch();
    if !source_validator.is_active_at(current_epoch) {
        return Ok(false);
    }
    // Verify exits for source has not been initiated
    if source_validator.exit_epoch != spec.far_future_epoch {
        return Ok(false);
    }

    Ok(true)
}

pub fn process_consolidation_request<E: EthSpec>(
    state: &mut BeaconState<E>,
    consolidation_request: &ConsolidationRequest,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    if is_valid_switch_to_compounding_request(state, consolidation_request, spec)? {
        let Some(source_index) = state
            .pubkey_cache()
            .get(&consolidation_request.source_pubkey)
        else {
            // source validator doesn't exist. This is unreachable as `is_valid_switch_to_compounding_request`
            // will return false in that case.
            return Ok(());
        };
        state.switch_to_compounding_validator(source_index, spec)?;
        return Ok(());
    }

    // Verify that source != target, so a consolidation cannot be used as an exit.
    if consolidation_request.source_pubkey == consolidation_request.target_pubkey {
        return Ok(());
    }

    // If the pending consolidations queue is full, consolidation requests are ignored
    if state.pending_consolidations()?.len() == E::PendingConsolidationsLimit::to_usize() {
        return Ok(());
    }
    // If there is too little available consolidation churn limit, consolidation requests are ignored
    if state.get_consolidation_churn_limit(spec)? <= spec.min_activation_balance {
        return Ok(());
    }

    let Some(source_index) = state
        .pubkey_cache()
        .get(&consolidation_request.source_pubkey)
    else {
        // source validator doesn't exist
        return Ok(());
    };
    let Some(target_index) = state
        .pubkey_cache()
        .get(&consolidation_request.target_pubkey)
    else {
        // target validator doesn't exist
        return Ok(());
    };

    let source_validator = state.get_validator(source_index)?;
    // Verify the source withdrawal credentials
    if let Some(withdrawal_address) = source_validator.get_execution_withdrawal_address(spec) {
        if withdrawal_address != consolidation_request.source_address {
            return Ok(());
        }
    } else {
        // Source doen't have execution withdrawal credentials
        return Ok(());
    }

    let target_validator = state.get_validator(target_index)?;
    // Verify the target has compounding withdrawal credentials
    if !target_validator.has_compounding_withdrawal_credential(spec) {
        return Ok(());
    }

    // Verify the source and target are active
    let current_epoch = state.current_epoch();
    if !source_validator.is_active_at(current_epoch)
        || !target_validator.is_active_at(current_epoch)
    {
        return Ok(());
    }
    // Verify exits for source and target have not been initiated
    if source_validator.exit_epoch != spec.far_future_epoch
        || target_validator.exit_epoch != spec.far_future_epoch
    {
        return Ok(());
    }
    // Verify the source has been active long enough
    if current_epoch
        < source_validator
            .activation_epoch
            .safe_add(spec.shard_committee_period)?
    {
        return Ok(());
    }
    // Verify the source has no pending withdrawals in the queue
    if state.get_pending_balance_to_withdraw(source_index)? > 0 {
        return Ok(());
    }

    // Initiate source validator exit and append pending consolidation
    let source_exit_epoch = state
        .compute_consolidation_epoch_and_update_churn(source_validator.effective_balance, spec)?;
    let source_validator = state.get_validator_mut(source_index)?;
    source_validator.exit_epoch = source_exit_epoch;
    source_validator.withdrawable_epoch =
        source_exit_epoch.safe_add(spec.min_validator_withdrawability_delay)?;
    state
        .pending_consolidations_mut()?
        .push(PendingConsolidation {
            source_index: source_index as u64,
            target_index: target_index as u64,
        })?;

    Ok(())
}

#[cfg(test)]
mod builder_deposit_tests {
    use super::*;
    use bls::FixedBytesExtended;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::test_utils::generate_deterministic_keypairs;
    use types::{
        Address, BeaconBlockHeader, BeaconStateGloas, Builder, BuilderPendingPayment,
        CACHED_EPOCHS, Checkpoint, CommitteeCache, Epoch, ExecutionBlockHash, ExecutionPayloadBid,
        ExitCache, FixedVector, Fork, Hash256, MinimalEthSpec, PendingDeposit,
        ProgressiveBalancesCache, PubkeyCache, SignatureBytes, SlashingsCache, Slot, SyncCommittee,
        Vector,
    };

    type E = MinimalEthSpec;
    const NUM_VALIDATORS: usize = 8;

    /// Build a minimal Gloas state with validators and an optional pre-existing builder.
    fn make_gloas_state_for_deposits(include_builder: bool) -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let slot = Slot::new(E::slots_per_epoch()); // slot 8, epoch 1
        let epoch = slot.epoch(E::slots_per_epoch());

        let keypairs = generate_deterministic_keypairs(NUM_VALIDATORS);
        let mut validators = Vec::with_capacity(NUM_VALIDATORS);
        let mut balances = Vec::with_capacity(NUM_VALIDATORS);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);

            validators.push(types::Validator {
                pubkey: kp.pk.compress(),
                effective_balance: 32_000_000_000,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..types::Validator::default()
            });
            balances.push(32_000_000_000);
        }

        let builders = if include_builder {
            let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
            let builder_kp = &extra_kps[NUM_VALIDATORS];
            vec![Builder {
                pubkey: builder_kp.pk.compress(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xBB),
                balance: 64_000_000_000,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: spec.far_future_epoch,
            }]
        } else {
            vec![]
        };

        let parent_root = Hash256::repeat_byte(0x01);
        let parent_block_hash = ExecutionBlockHash::repeat_byte(0x02);
        let randao_mix = Hash256::repeat_byte(0x03);

        let epochs_per_vector = <E as types::EthSpec>::EpochsPerHistoricalVector::to_usize();
        let mut randao_mixes = vec![Hash256::zero(); epochs_per_vector];
        let mix_index = epoch.as_usize() % epochs_per_vector;
        randao_mixes[mix_index] = randao_mix;

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                types::PublicKeyBytes::empty();
                <E as types::EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: types::PublicKeyBytes::empty(),
        });

        let slots_per_hist = <E as types::EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_slash = <E as types::EthSpec>::EpochsPerSlashingsVector::to_usize();

        let mut state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch,
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root,
                state_root: Hash256::zero(),
                body_root: Hash256::zero(),
            },
            block_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(randao_mixes).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::default(),
            current_epoch_participation: List::default(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: Checkpoint::default(),
            current_justified_checkpoint: Checkpoint::default(),
            finalized_checkpoint: Checkpoint {
                epoch: Epoch::new(1),
                root: Hash256::zero(),
            },
            inactivity_scores: List::default(),
            current_sync_committee: sync_committee.clone(),
            next_sync_committee: sync_committee,
            latest_execution_payload_bid: ExecutionPayloadBid {
                parent_block_hash,
                parent_block_root: parent_root,
                block_hash: ExecutionBlockHash::repeat_byte(0x04),
                prev_randao: randao_mix,
                slot,
                ..Default::default()
            },
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
                <E as types::EthSpec>::ProposerLookaheadSlots::to_usize()
            ])
            .unwrap(),
            builders: List::new(builders).unwrap(),
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
            latest_block_hash: parent_block_hash,
            payload_expected_withdrawals: List::default(),
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        // Build the pubkey cache so lookups work
        state.update_pubkey_cache().unwrap();

        (state, spec)
    }

    /// Create a DepositRequest with builder credentials (0x03 prefix).
    fn make_builder_deposit_request(
        keypair: &bls::Keypair,
        amount: u64,
        spec: &ChainSpec,
    ) -> DepositRequest {
        let mut creds = [0u8; 32];
        creds[0] = 0x03;
        creds[12..].copy_from_slice(&[0xDD; 20]);
        let withdrawal_credentials = Hash256::from_slice(&creds);

        let deposit_data = types::DepositData {
            pubkey: keypair.pk.compress(),
            withdrawal_credentials,
            amount,
            signature: SignatureBytes::empty(),
        };
        let signature = deposit_data.create_signature(&keypair.sk, spec);

        DepositRequest {
            pubkey: keypair.pk.compress(),
            withdrawal_credentials,
            amount,
            signature,
            index: 0,
        }
    }

    /// Create a DepositRequest with validator credentials (0x01 prefix).
    fn make_validator_deposit_request(
        keypair: &bls::Keypair,
        amount: u64,
        spec: &ChainSpec,
    ) -> DepositRequest {
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

        DepositRequest {
            pubkey: keypair.pk.compress(),
            withdrawal_credentials,
            amount,
            signature,
            index: 0,
        }
    }

    // ── is_builder_withdrawal_credential tests ─────────────────

    #[test]
    fn builder_credential_prefix_0x03_is_builder() {
        let mut creds = [0u8; 32];
        creds[0] = 0x03;
        assert!(is_builder_withdrawal_credential(Hash256::from_slice(
            &creds
        )));
    }

    #[test]
    fn validator_credential_prefix_0x01_not_builder() {
        let mut creds = [0u8; 32];
        creds[0] = 0x01;
        assert!(!is_builder_withdrawal_credential(Hash256::from_slice(
            &creds
        )));
    }

    #[test]
    fn validator_credential_prefix_0x02_not_builder() {
        let mut creds = [0u8; 32];
        creds[0] = 0x02;
        assert!(!is_builder_withdrawal_credential(Hash256::from_slice(
            &creds
        )));
    }

    #[test]
    fn zero_credential_not_builder() {
        assert!(!is_builder_withdrawal_credential(Hash256::zero()));
    }

    // ── get_index_for_new_builder tests ────────────────────────

    #[test]
    fn new_builder_index_empty_list_returns_zero() {
        let spec = E::default_spec();
        let builders: List<Builder, <E as types::EthSpec>::BuilderRegistryLimit> = List::default();
        assert_eq!(
            get_index_for_new_builder::<E>(&builders, Epoch::new(1), &spec),
            0
        );
    }

    #[test]
    fn new_builder_index_no_exited_returns_len() {
        let spec = E::default_spec();
        let builder = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xBB),
            balance: 100,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: spec.far_future_epoch,
        };
        let builders: List<Builder, <E as types::EthSpec>::BuilderRegistryLimit> =
            List::new(vec![builder]).unwrap();
        assert_eq!(
            get_index_for_new_builder::<E>(&builders, Epoch::new(1), &spec),
            1
        );
    }

    #[test]
    fn new_builder_index_reuses_exited_slot() {
        let spec = E::default_spec();
        let active = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xBB),
            balance: 100,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: spec.far_future_epoch,
        };
        let exited = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xCC),
            balance: 0,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: Epoch::new(0), // already withdrawable
        };
        let builders: List<Builder, <E as types::EthSpec>::BuilderRegistryLimit> =
            List::new(vec![active, exited]).unwrap();
        // Should reuse index 1 (the exited builder)
        assert_eq!(
            get_index_for_new_builder::<E>(&builders, Epoch::new(1), &spec),
            1
        );
    }

    #[test]
    fn new_builder_index_exited_but_nonzero_balance_not_reused() {
        let spec = E::default_spec();
        let exited_with_balance = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xCC),
            balance: 50, // still has balance, not fully withdrawn yet
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: Epoch::new(0),
        };
        let builders: List<Builder, <E as types::EthSpec>::BuilderRegistryLimit> =
            List::new(vec![exited_with_balance]).unwrap();
        // Should NOT reuse (balance > 0), returns len
        assert_eq!(
            get_index_for_new_builder::<E>(&builders, Epoch::new(1), &spec),
            1
        );
    }

    #[test]
    fn new_builder_index_future_withdrawable_not_reused() {
        let spec = E::default_spec();
        let not_yet_withdrawable = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xCC),
            balance: 0,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: Epoch::new(10), // still in future
        };
        let builders: List<Builder, <E as types::EthSpec>::BuilderRegistryLimit> =
            List::new(vec![not_yet_withdrawable]).unwrap();
        // withdrawable_epoch (10) > current_epoch (1), not reusable
        assert_eq!(
            get_index_for_new_builder::<E>(&builders, Epoch::new(1), &spec),
            1
        );
    }

    // ── apply_deposit_for_builder tests ────────────────────────

    #[test]
    fn apply_deposit_tops_up_existing_builder() {
        let (mut state, spec) = make_gloas_state_for_deposits(true);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];

        let initial_balance = state.as_gloas().unwrap().builders.get(0).unwrap().balance;

        let request = make_builder_deposit_request(builder_kp, 5_000_000_000, &spec);
        let slot = state.slot();
        apply_deposit_for_builder(&mut state, &request, slot, &spec).unwrap();

        let builder = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(builder.balance, initial_balance + 5_000_000_000);
        // Builder count unchanged
        assert_eq!(state.as_gloas().unwrap().builders.len(), 1);
    }

    #[test]
    fn apply_deposit_creates_new_builder_with_valid_signature() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        // Use a keypair not in the validator set
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_builder_kp = &extra_kps[NUM_VALIDATORS];

        let request = make_builder_deposit_request(new_builder_kp, 10_000_000_000, &spec);
        let slot = state.slot();
        apply_deposit_for_builder(&mut state, &request, slot, &spec).unwrap();

        let builders = &state.as_gloas().unwrap().builders;
        assert_eq!(builders.len(), 1);
        let b = builders.get(0).unwrap();
        assert_eq!(b.pubkey, new_builder_kp.pk.compress());
        assert_eq!(b.balance, 10_000_000_000);
        assert_eq!(b.version, 0x03);
        assert_eq!(b.execution_address, Address::repeat_byte(0xDD));
        assert_eq!(b.deposit_epoch, slot.epoch(E::slots_per_epoch()));
        assert_eq!(b.withdrawable_epoch, spec.far_future_epoch);
    }

    #[test]
    fn apply_deposit_invalid_signature_silently_skipped() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_builder_kp = &extra_kps[NUM_VALIDATORS];

        let mut request = make_builder_deposit_request(new_builder_kp, 10_000_000_000, &spec);
        // Corrupt the signature
        request.signature = SignatureBytes::empty();

        let slot = state.slot();
        // Should succeed but NOT create a builder (bad signature)
        apply_deposit_for_builder(&mut state, &request, slot, &spec).unwrap();

        assert_eq!(state.as_gloas().unwrap().builders.len(), 0);
    }

    #[test]
    fn apply_deposit_new_builder_reuses_exited_slot() {
        let (mut state, spec) = make_gloas_state_for_deposits(true);

        // Make the existing builder exited with zero balance
        {
            let builder = state.as_gloas_mut().unwrap().builders.get_mut(0).unwrap();
            builder.withdrawable_epoch = Epoch::new(0);
            builder.balance = 0;
        }

        // Create a new builder that should reuse index 0
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 2);
        let new_builder_kp = &extra_kps[NUM_VALIDATORS + 1];

        let request = make_builder_deposit_request(new_builder_kp, 7_000_000_000, &spec);
        let slot = state.slot();
        apply_deposit_for_builder(&mut state, &request, slot, &spec).unwrap();

        let builders = &state.as_gloas().unwrap().builders;
        // Should still be 1 builder (reused slot 0)
        assert_eq!(builders.len(), 1);
        let b = builders.get(0).unwrap();
        assert_eq!(b.pubkey, new_builder_kp.pk.compress());
        assert_eq!(b.balance, 7_000_000_000);
    }

    #[test]
    fn apply_deposit_new_builder_appends_when_no_free_slot() {
        let (mut state, spec) = make_gloas_state_for_deposits(true);

        // Existing builder at index 0 is active (no free slot)
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 2);
        let new_builder_kp = &extra_kps[NUM_VALIDATORS + 1];

        let request = make_builder_deposit_request(new_builder_kp, 3_000_000_000, &spec);
        let slot = state.slot();
        apply_deposit_for_builder(&mut state, &request, slot, &spec).unwrap();

        let builders = &state.as_gloas().unwrap().builders;
        assert_eq!(builders.len(), 2);
        let b = builders.get(1).unwrap();
        assert_eq!(b.pubkey, new_builder_kp.pk.compress());
        assert_eq!(b.balance, 3_000_000_000);
    }

    // ── is_pending_validator tests ─────────────────────────────

    #[test]
    fn pending_validator_found_with_valid_signature() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_val_kp = &extra_kps[NUM_VALIDATORS];

        // Add a pending deposit with valid signature for a non-validator pubkey
        let request = make_validator_deposit_request(new_val_kp, 32_000_000_000, &spec);
        let deposit_data = types::DepositData {
            pubkey: request.pubkey,
            withdrawal_credentials: request.withdrawal_credentials,
            amount: request.amount,
            signature: request.signature.clone(),
        };
        let valid_sig = deposit_data.create_signature(&new_val_kp.sk, &spec);
        let slot = state.slot();

        state
            .pending_deposits_mut()
            .unwrap()
            .push(PendingDeposit {
                pubkey: new_val_kp.pk.compress(),
                withdrawal_credentials: request.withdrawal_credentials,
                amount: request.amount,
                signature: valid_sig,
                slot,
            })
            .unwrap();

        assert!(is_pending_validator(
            &state,
            &new_val_kp.pk.compress(),
            &spec
        ));
    }

    #[test]
    fn pending_validator_not_found_with_invalid_signature() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_val_kp = &extra_kps[NUM_VALIDATORS];

        // Add a pending deposit with INVALID signature
        let slot = state.slot();
        state
            .pending_deposits_mut()
            .unwrap()
            .push(PendingDeposit {
                pubkey: new_val_kp.pk.compress(),
                withdrawal_credentials: Hash256::repeat_byte(0x01),
                amount: 32_000_000_000,
                signature: SignatureBytes::empty(), // invalid
                slot,
            })
            .unwrap();

        assert!(!is_pending_validator(
            &state,
            &new_val_kp.pk.compress(),
            &spec
        ));
    }

    #[test]
    fn pending_validator_not_found_when_no_pending_deposits() {
        let (state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_val_kp = &extra_kps[NUM_VALIDATORS];

        assert!(!is_pending_validator(
            &state,
            &new_val_kp.pk.compress(),
            &spec
        ));
    }

    #[test]
    fn pending_validator_not_found_wrong_pubkey() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 2);
        let val_kp = &extra_kps[NUM_VALIDATORS];
        let other_kp = &extra_kps[NUM_VALIDATORS + 1];

        // Add a pending deposit for val_kp
        let request = make_validator_deposit_request(val_kp, 32_000_000_000, &spec);
        let slot = state.slot();
        state
            .pending_deposits_mut()
            .unwrap()
            .push(PendingDeposit {
                pubkey: val_kp.pk.compress(),
                withdrawal_credentials: request.withdrawal_credentials,
                amount: request.amount,
                signature: request.signature,
                slot,
            })
            .unwrap();

        // Search for a different pubkey
        assert!(!is_pending_validator(
            &state,
            &other_kp.pk.compress(),
            &spec
        ));
    }

    // ── process_deposit_request_gloas routing tests ────────────

    #[test]
    fn deposit_request_routes_existing_builder_to_topup() {
        let (mut state, spec) = make_gloas_state_for_deposits(true);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];

        let request = make_builder_deposit_request(builder_kp, 2_000_000_000, &spec);
        let initial_balance = state.as_gloas().unwrap().builders.get(0).unwrap().balance;

        process_deposit_request_gloas(&mut state, &request, &spec).unwrap();

        // Builder should be topped up
        let builder = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(builder.balance, initial_balance + 2_000_000_000);
        // No pending deposits added
        assert_eq!(state.pending_deposits().unwrap().len(), 0);
    }

    #[test]
    fn deposit_request_routes_existing_validator_to_pending() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let keypairs = generate_deterministic_keypairs(NUM_VALIDATORS);
        let val_kp = &keypairs[0]; // existing validator

        // Even with 0x03 prefix, existing validator deposits go to pending
        let request = make_validator_deposit_request(val_kp, 1_000_000_000, &spec);
        process_deposit_request_gloas(&mut state, &request, &spec).unwrap();

        assert_eq!(state.pending_deposits().unwrap().len(), 1);
        assert_eq!(state.as_gloas().unwrap().builders.len(), 0);
    }

    #[test]
    fn deposit_request_new_pubkey_with_builder_prefix_creates_builder() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_kp = &extra_kps[NUM_VALIDATORS]; // not a validator

        let request = make_builder_deposit_request(new_kp, 10_000_000_000, &spec);
        process_deposit_request_gloas(&mut state, &request, &spec).unwrap();

        // Should create a builder (0x03 prefix, not existing validator, no pending validator deposit)
        assert_eq!(state.as_gloas().unwrap().builders.len(), 1);
        assert_eq!(state.pending_deposits().unwrap().len(), 0);
    }

    #[test]
    fn deposit_request_new_pubkey_with_validator_prefix_to_pending() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_kp = &extra_kps[NUM_VALIDATORS];

        let request = make_validator_deposit_request(new_kp, 32_000_000_000, &spec);
        process_deposit_request_gloas(&mut state, &request, &spec).unwrap();

        // 0x01 prefix → always goes to pending deposits
        assert_eq!(state.pending_deposits().unwrap().len(), 1);
        assert_eq!(state.as_gloas().unwrap().builders.len(), 0);
    }

    #[test]
    fn deposit_request_builder_prefix_but_pending_validator_exists_goes_to_pending() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_kp = &extra_kps[NUM_VALIDATORS];

        // First, add a pending validator deposit with valid signature for this pubkey
        let val_request = make_validator_deposit_request(new_kp, 32_000_000_000, &spec);
        let slot = state.slot();
        state
            .pending_deposits_mut()
            .unwrap()
            .push(PendingDeposit {
                pubkey: new_kp.pk.compress(),
                withdrawal_credentials: val_request.withdrawal_credentials,
                amount: val_request.amount,
                signature: val_request.signature,
                slot,
            })
            .unwrap();

        // Now submit a builder-prefix deposit for the same pubkey
        let builder_request = make_builder_deposit_request(new_kp, 5_000_000_000, &spec);
        process_deposit_request_gloas(&mut state, &builder_request, &spec).unwrap();

        // Should go to pending deposits (not builder) because a pending validator
        // deposit with valid signature exists
        assert_eq!(state.pending_deposits().unwrap().len(), 2);
        assert_eq!(state.as_gloas().unwrap().builders.len(), 0);
    }

    #[test]
    fn deposit_request_builder_prefix_pending_with_bad_sig_still_creates_builder() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let new_kp = &extra_kps[NUM_VALIDATORS];

        // Add a pending deposit with INVALID signature for this pubkey
        let slot = state.slot();
        state
            .pending_deposits_mut()
            .unwrap()
            .push(PendingDeposit {
                pubkey: new_kp.pk.compress(),
                withdrawal_credentials: Hash256::repeat_byte(0x01),
                amount: 32_000_000_000,
                signature: SignatureBytes::empty(), // invalid
                slot,
            })
            .unwrap();

        // Builder-prefix deposit: pending validator has bad sig, so this is NOT
        // considered a pending validator → creates builder
        let builder_request = make_builder_deposit_request(new_kp, 5_000_000_000, &spec);
        process_deposit_request_gloas(&mut state, &builder_request, &spec).unwrap();

        assert_eq!(state.as_gloas().unwrap().builders.len(), 1);
        // The original invalid pending deposit is still there
        assert_eq!(state.pending_deposits().unwrap().len(), 1);
    }

    #[test]
    fn deposit_request_existing_builder_topup_no_signature_check() {
        let (mut state, spec) = make_gloas_state_for_deposits(true);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let builder_kp = &extra_kps[NUM_VALIDATORS];

        // Create request with invalid signature — topup should still work
        let mut request = make_builder_deposit_request(builder_kp, 3_000_000_000, &spec);
        request.signature = SignatureBytes::empty(); // bad sig

        let initial_balance = state.as_gloas().unwrap().builders.get(0).unwrap().balance;
        process_deposit_request_gloas(&mut state, &request, &spec).unwrap();

        let builder = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(builder.balance, initial_balance + 3_000_000_000);
    }

    #[test]
    fn deposit_request_multiple_builders_created() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 3);

        for i in 0..3 {
            let kp = &extra_kps[NUM_VALIDATORS + i];
            let request = make_builder_deposit_request(kp, (i as u64 + 1) * 1_000_000_000, &spec);
            process_deposit_request_gloas(&mut state, &request, &spec).unwrap();
        }

        let builders = &state.as_gloas().unwrap().builders;
        assert_eq!(builders.len(), 3);
        assert_eq!(builders.get(0).unwrap().balance, 1_000_000_000);
        assert_eq!(builders.get(1).unwrap().balance, 2_000_000_000);
        assert_eq!(builders.get(2).unwrap().balance, 3_000_000_000);
    }

    #[test]
    fn deposit_request_builder_credentials_parsed_correctly() {
        let (mut state, spec) = make_gloas_state_for_deposits(false);
        let extra_kps = generate_deterministic_keypairs(NUM_VALIDATORS + 1);
        let kp = &extra_kps[NUM_VALIDATORS];

        let request = make_builder_deposit_request(kp, 10_000_000_000, &spec);
        process_deposit_request_gloas(&mut state, &request, &spec).unwrap();

        let builder = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(builder.version, 0x03);
        // Address is extracted from bytes [12..32] of withdrawal_credentials
        assert_eq!(builder.execution_address, Address::repeat_byte(0xDD));
    }
}

/// Tests for Gloas-specific proposer slashing payment removal and same-slot attestation weight.
#[cfg(test)]
mod gloas_operations_tests {
    use super::*;
    use crate::ConsensusContext;
    use crate::VerifySignatures;
    use crate::common::update_progressive_balances_cache::initialize_progressive_balances_cache;
    use crate::epoch_cache::initialize_epoch_cache;
    use bls::FixedBytesExtended;
    use ssz_types::{BitList, BitVector};
    use std::sync::Arc;
    use types::test_utils::generate_deterministic_keypairs;
    use types::{
        Address, Attestation, AttestationData, BeaconBlockHeader, BeaconStateGloas,
        BuilderPendingPayment, BuilderPendingWithdrawal, CACHED_EPOCHS, Checkpoint, CommitteeCache,
        Epoch, ExecutionBlockHash, ExecutionPayloadBid, ExitCache, FixedVector, Fork, Hash256,
        List, MinimalEthSpec, ParticipationFlags, ProgressiveBalancesCache, PubkeyCache,
        SignedBeaconBlockHeader, SlashingsCache, SyncCommittee, Unsigned, Vector,
    };

    type E = MinimalEthSpec;
    const NUM_VALIDATORS: usize = 8;

    /// Build a Gloas state at the given slot with validators and caches built.
    /// The state is at the start of an epoch (not epoch boundary) so committee caches work.
    fn make_gloas_state_with_caches(
        slot_num: u64,
    ) -> (BeaconState<E>, ChainSpec, ConsensusContext<E>) {
        let mut spec = E::default_spec();
        // Set all forks at epoch 0
        spec.altair_fork_epoch = Some(Epoch::new(0));
        spec.bellatrix_fork_epoch = Some(Epoch::new(0));
        spec.capella_fork_epoch = Some(Epoch::new(0));
        spec.deneb_fork_epoch = Some(Epoch::new(0));
        spec.electra_fork_epoch = Some(Epoch::new(0));
        spec.fulu_fork_epoch = Some(Epoch::new(0));
        spec.gloas_fork_epoch = Some(Epoch::new(0));

        let slot = Slot::new(slot_num);
        let epoch = slot.epoch(E::slots_per_epoch());

        let keypairs = generate_deterministic_keypairs(NUM_VALIDATORS);
        let mut validators = Vec::with_capacity(NUM_VALIDATORS);
        let mut balances = Vec::with_capacity(NUM_VALIDATORS);
        let mut participation = Vec::with_capacity(NUM_VALIDATORS);
        let mut inactivity = Vec::with_capacity(NUM_VALIDATORS);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);

            validators.push(types::Validator {
                pubkey: kp.pk.compress(),
                effective_balance: 32_000_000_000,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..types::Validator::default()
            });
            balances.push(32_000_000_000);
            participation.push(ParticipationFlags::default());
            inactivity.push(0u64);
        }

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        // Unique block root per slot
        let block_roots_vec: Vec<Hash256> = (0..slots_per_hist)
            .map(|i| Hash256::repeat_byte((i % 255 + 1) as u8))
            .collect();

        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                types::PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: types::PublicKeyBytes::empty(),
        });

        let source_checkpoint = Checkpoint {
            epoch: epoch.saturating_sub(1u64),
            root: Hash256::repeat_byte(0xCC),
        };

        let mut state = BeaconState::Gloas(BeaconStateGloas {
            genesis_time: 0,
            genesis_validators_root: Hash256::repeat_byte(0xAA),
            slot,
            fork: Fork {
                previous_version: spec.fulu_fork_version,
                current_version: spec.gloas_fork_version,
                epoch: Epoch::new(0),
            },
            latest_block_header: BeaconBlockHeader {
                slot: slot.saturating_sub(1u64),
                proposer_index: 0,
                parent_root: Hash256::zero(),
                state_root: Hash256::zero(),
                body_root: Hash256::repeat_byte(0x01),
            },
            block_roots: Vector::new(block_roots_vec).unwrap(),
            state_roots: Vector::new(vec![Hash256::zero(); slots_per_hist]).unwrap(),
            historical_roots: List::default(),
            eth1_data: types::Eth1Data::default(),
            eth1_data_votes: List::default(),
            eth1_deposit_index: 0,
            validators: List::new(validators).unwrap(),
            balances: List::new(balances).unwrap(),
            randao_mixes: Vector::new(vec![Hash256::zero(); epochs_per_vector]).unwrap(),
            slashings: Vector::new(vec![0; epochs_per_slash]).unwrap(),
            previous_epoch_participation: List::new(participation.clone()).unwrap(),
            current_epoch_participation: List::new(participation).unwrap(),
            justification_bits: BitVector::new(),
            previous_justified_checkpoint: source_checkpoint,
            current_justified_checkpoint: source_checkpoint,
            finalized_checkpoint: Checkpoint::default(),
            inactivity_scores: List::new(inactivity).unwrap(),
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
            total_active_balance: None,
            progressive_balances_cache: ProgressiveBalancesCache::default(),
            committee_caches: <[Arc<CommitteeCache>; CACHED_EPOCHS]>::default(),
            pubkey_cache: PubkeyCache::default(),
            exit_cache: ExitCache::default(),
            slashings_cache: SlashingsCache::default(),
            epoch_cache: types::EpochCache::default(),
        });

        // Build all required caches
        state.build_caches(&spec).unwrap();
        initialize_epoch_cache(&mut state, &spec).unwrap();
        initialize_progressive_balances_cache(&mut state, &spec).unwrap();

        let ctxt = ConsensusContext::new(slot);

        (state, spec, ctxt)
    }

    /// Create a `ProposerSlashing` for the given proposer at the given slot.
    /// Uses `VerifySignatures::False` so signatures don't matter.
    fn make_proposer_slashing(proposer_index: u64, slot: Slot) -> ProposerSlashing {
        ProposerSlashing {
            signed_header_1: SignedBeaconBlockHeader {
                message: BeaconBlockHeader {
                    slot,
                    proposer_index,
                    parent_root: Hash256::repeat_byte(0x01),
                    state_root: Hash256::repeat_byte(0x02),
                    body_root: Hash256::repeat_byte(0x03),
                },
                signature: types::Signature::empty(),
            },
            signed_header_2: SignedBeaconBlockHeader {
                message: BeaconBlockHeader {
                    slot,
                    proposer_index,
                    parent_root: Hash256::repeat_byte(0x04),
                    state_root: Hash256::repeat_byte(0x05),
                    body_root: Hash256::repeat_byte(0x06),
                },
                signature: types::Signature::empty(),
            },
        }
    }

    /// Set a BuilderPendingPayment at the given index with the given amount.
    fn set_payment(state: &mut BeaconState<E>, index: usize, amount: u64) {
        let payment = state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_payments
            .get_mut(index)
            .unwrap();
        payment.weight = 0;
        payment.withdrawal = BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0xBB),
            amount,
            builder_index: 0,
        };
    }

    // ── Proposer slashing builder payment removal tests ──────

    #[test]
    fn slashing_current_epoch_clears_payment() {
        // State at slot 9 (epoch 1 for MinimalEthSpec with 8 slots/epoch).
        // Proposer slashing at slot 9 (current epoch).
        // payment_index = slots_per_epoch + (9 % 8) = 8 + 1 = 9
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(9);

        let payment_index = E::slots_per_epoch() as usize + (9 % E::slots_per_epoch() as usize);
        set_payment(&mut state, payment_index, 1_000_000_000);

        // Verify payment exists
        assert_eq!(
            state
                .as_gloas()
                .unwrap()
                .builder_pending_payments
                .get(payment_index)
                .unwrap()
                .withdrawal
                .amount,
            1_000_000_000
        );

        let slashing = make_proposer_slashing(1, Slot::new(9));
        process_proposer_slashings(
            &mut state,
            &[slashing],
            VerifySignatures::False,
            &mut ctxt,
            &spec,
        )
        .unwrap();

        // Payment should be cleared (zeroed)
        let payment = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap();
        assert_eq!(payment.withdrawal.amount, 0, "payment should be cleared");
        assert_eq!(payment.weight, 0, "weight should be cleared");
    }

    #[test]
    fn slashing_previous_epoch_clears_payment() {
        // State at slot 9 (epoch 1). Proposer slashing at slot 3 (epoch 0 = previous epoch).
        // payment_index = 3 % 8 = 3
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(9);

        let payment_index = 3 % E::slots_per_epoch() as usize;
        set_payment(&mut state, payment_index, 2_000_000_000);

        let slashing = make_proposer_slashing(2, Slot::new(3));
        process_proposer_slashings(
            &mut state,
            &[slashing],
            VerifySignatures::False,
            &mut ctxt,
            &spec,
        )
        .unwrap();

        let payment = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap();
        assert_eq!(
            payment.withdrawal.amount, 0,
            "previous epoch payment should be cleared"
        );
    }

    #[test]
    fn slashing_old_epoch_does_not_clear_payment() {
        // State at slot 17 (epoch 2). Proposer slashing at slot 3 (epoch 0 = 2 epochs ago).
        // proposal_epoch != current_epoch && proposal_epoch != previous_epoch → no clearing.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(17);

        // Set payment at index 3 (where slot 3 would map if it were previous epoch)
        let payment_index = 3 % E::slots_per_epoch() as usize;
        set_payment(&mut state, payment_index, 3_000_000_000);

        let slashing = make_proposer_slashing(3, Slot::new(3));
        process_proposer_slashings(
            &mut state,
            &[slashing],
            VerifySignatures::False,
            &mut ctxt,
            &spec,
        )
        .unwrap();

        // Payment should NOT be cleared because the slashing is from 2 epochs ago
        let payment = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap();
        assert_eq!(
            payment.withdrawal.amount, 3_000_000_000,
            "old epoch payment should not be cleared"
        );
    }

    #[test]
    fn slashing_clears_only_target_payment_index() {
        // Verify that only the specific payment index is cleared, not others.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(9);

        // Set payments at multiple indices
        set_payment(&mut state, 0, 1_000_000_000);
        set_payment(&mut state, 1, 2_000_000_000);
        let target_index = E::slots_per_epoch() as usize + (9 % E::slots_per_epoch() as usize);
        set_payment(&mut state, target_index, 3_000_000_000);

        let slashing = make_proposer_slashing(1, Slot::new(9));
        process_proposer_slashings(
            &mut state,
            &[slashing],
            VerifySignatures::False,
            &mut ctxt,
            &spec,
        )
        .unwrap();

        // Target should be cleared
        assert_eq!(
            state
                .as_gloas()
                .unwrap()
                .builder_pending_payments
                .get(target_index)
                .unwrap()
                .withdrawal
                .amount,
            0
        );
        // Others should remain
        assert_eq!(
            state
                .as_gloas()
                .unwrap()
                .builder_pending_payments
                .get(0)
                .unwrap()
                .withdrawal
                .amount,
            1_000_000_000
        );
        assert_eq!(
            state
                .as_gloas()
                .unwrap()
                .builder_pending_payments
                .get(1)
                .unwrap()
                .withdrawal
                .amount,
            2_000_000_000
        );
    }

    #[test]
    fn slashing_already_empty_payment_is_noop() {
        // Slashing a slot with no payment should not error.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(9);

        let slashing = make_proposer_slashing(1, Slot::new(9));
        // No payment set at this index — should succeed without error
        process_proposer_slashings(
            &mut state,
            &[slashing],
            VerifySignatures::False,
            &mut ctxt,
            &spec,
        )
        .unwrap();
    }

    #[test]
    fn slashing_epoch_boundary_slot_calculates_correct_index() {
        // State at slot 9 (epoch 1). Slashing at slot 8 (epoch boundary, epoch 1 current epoch).
        // payment_index = 8 + (8 % 8) = 8 + 0 = 8
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(9);

        let payment_index = E::slots_per_epoch() as usize; // 8 + 0 = 8
        set_payment(&mut state, payment_index, 5_000_000_000);

        let slashing = make_proposer_slashing(4, Slot::new(8));
        process_proposer_slashings(
            &mut state,
            &[slashing],
            VerifySignatures::False,
            &mut ctxt,
            &spec,
        )
        .unwrap();

        assert_eq!(
            state
                .as_gloas()
                .unwrap()
                .builder_pending_payments
                .get(payment_index)
                .unwrap()
                .withdrawal
                .amount,
            0,
            "epoch boundary payment should be cleared"
        );
    }

    // ── Same-slot attestation weight accumulation tests ──────

    /// Create an Electra attestation for a given slot with correct aggregation bits.
    fn make_attestation(state: &BeaconState<E>, att_slot: u64, att_index: u64) -> Attestation<E> {
        let epoch = state.current_epoch();
        let target_slot = epoch.start_slot(E::slots_per_epoch());
        let target_root = *state.get_block_root(target_slot).unwrap();
        let att_block_root = *state.get_block_root(Slot::new(att_slot)).unwrap();

        let att_data = AttestationData {
            slot: Slot::new(att_slot),
            index: att_index,
            beacon_block_root: att_block_root,
            source: state.current_justified_checkpoint(),
            target: Checkpoint {
                epoch,
                root: target_root,
            },
        };

        // Get the committee for this slot
        let committee = state.get_beacon_committee(Slot::new(att_slot), 0).unwrap();
        let committee_len = committee.committee.len();

        // Create aggregation bits with first member attesting
        let mut aggregation_bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerSlot>::with_capacity(committee_len).unwrap();
        aggregation_bits.set(0, true).unwrap();

        let mut committee_bits = BitVector::<<E as EthSpec>::MaxCommitteesPerSlot>::new();
        committee_bits.set(0, true).unwrap();

        Attestation::Electra(types::AttestationElectra {
            aggregation_bits,
            data: att_data,
            signature: types::AggregateSignature::empty(),
            committee_bits,
        })
    }

    #[test]
    fn same_slot_attestation_adds_weight_to_payment() {
        // State at slot 11 (epoch 1). Attestation at slot 10 (same-slot, current epoch).
        // The attestation is "same-slot" because block_roots[10] != block_roots[9].
        // payment_slot_index = slots_per_epoch + (10 % 8) = 8 + 2 = 10
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(11);

        let payment_index = E::slots_per_epoch() as usize + (10 % E::slots_per_epoch() as usize);
        set_payment(&mut state, payment_index, 1_000_000_000);

        let attestation = make_attestation(&state, 10, 0);

        let initial_weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;
        assert_eq!(initial_weight, 0);

        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // Weight should have increased by the attesting validator's effective_balance
        let updated_weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;
        assert!(
            updated_weight > 0,
            "same-slot attestation should add weight (got {})",
            updated_weight
        );
        assert_eq!(
            updated_weight, 32_000_000_000,
            "weight should equal the attesting validator's effective balance"
        );
    }

    #[test]
    fn same_slot_attestation_no_weight_when_payment_zero() {
        // If the payment amount is 0, no weight should be added.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(11);

        let attestation = make_attestation(&state, 10, 0);

        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let payment_index = E::slots_per_epoch() as usize + (10 % E::slots_per_epoch() as usize);
        let weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;
        assert_eq!(
            weight, 0,
            "no weight should be added when payment amount is 0"
        );
    }

    #[test]
    fn non_same_slot_attestation_no_weight_added() {
        // Attestation from a skipped slot should not add weight.
        // Make slot 9 a "skipped" slot (block_roots[9] == block_roots[8]).
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(11);

        // Make slot 9 look like a skipped slot
        let prev_root = *state.get_block_root(Slot::new(8)).unwrap();
        state.set_block_root(Slot::new(9), prev_root).unwrap();

        let payment_index = E::slots_per_epoch() as usize + (9 % E::slots_per_epoch() as usize);
        set_payment(&mut state, payment_index, 1_000_000_000);

        // Attestation at slot 9 with index=1 (matches availability bit=1 for historical)
        let attestation = make_attestation(&state, 9, 1);

        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;
        assert_eq!(weight, 0, "non-same-slot attestation should not add weight");
    }

    #[test]
    fn duplicate_attestation_does_not_add_weight_twice() {
        // If the same validator attests twice, the second attestation should not
        // set new flags (they're already set), so no additional weight.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(11);

        let payment_index = E::slots_per_epoch() as usize + (10 % E::slots_per_epoch() as usize);
        set_payment(&mut state, payment_index, 1_000_000_000);

        let attestation = make_attestation(&state, 10, 0);

        // First attestation
        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let weight_after_first = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;

        // Second attestation (same validator, same slot)
        let attestation2 = make_attestation(&state, 10, 0);
        altair_deneb::process_attestation(
            &mut state,
            attestation2.to_ref(),
            1,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let weight_after_second = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;

        assert_eq!(
            weight_after_first, weight_after_second,
            "duplicate attestation should not add weight again"
        );
    }

    /// Create an Electra attestation targeting the PREVIOUS epoch.
    /// State must be at an epoch > 0 so previous epoch exists.
    fn make_prev_epoch_attestation(
        state: &BeaconState<E>,
        att_slot: u64,
        att_index: u64,
    ) -> Attestation<E> {
        let prev_epoch = state.previous_epoch();
        let target_slot = prev_epoch.start_slot(E::slots_per_epoch());
        let target_root = *state.get_block_root(target_slot).unwrap();
        let att_block_root = *state.get_block_root(Slot::new(att_slot)).unwrap();

        let att_data = AttestationData {
            slot: Slot::new(att_slot),
            index: att_index,
            beacon_block_root: att_block_root,
            source: state.previous_justified_checkpoint(),
            target: Checkpoint {
                epoch: prev_epoch,
                root: target_root,
            },
        };

        let committee = state.get_beacon_committee(Slot::new(att_slot), 0).unwrap();
        let committee_len = committee.committee.len();

        let mut aggregation_bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerSlot>::with_capacity(committee_len).unwrap();
        aggregation_bits.set(0, true).unwrap();

        let mut committee_bits = BitVector::<<E as EthSpec>::MaxCommitteesPerSlot>::new();
        committee_bits.set(0, true).unwrap();

        Attestation::Electra(types::AttestationElectra {
            aggregation_bits,
            data: att_data,
            signature: types::AggregateSignature::empty(),
            committee_bits,
        })
    }

    /// Create an attestation where multiple committee members attest.
    fn make_multi_attester_attestation(
        state: &BeaconState<E>,
        att_slot: u64,
        att_index: u64,
        num_attesters: usize,
    ) -> Attestation<E> {
        let epoch = state.current_epoch();
        let target_slot = epoch.start_slot(E::slots_per_epoch());
        let target_root = *state.get_block_root(target_slot).unwrap();
        let att_block_root = *state.get_block_root(Slot::new(att_slot)).unwrap();

        let att_data = AttestationData {
            slot: Slot::new(att_slot),
            index: att_index,
            beacon_block_root: att_block_root,
            source: state.current_justified_checkpoint(),
            target: Checkpoint {
                epoch,
                root: target_root,
            },
        };

        let committee = state.get_beacon_committee(Slot::new(att_slot), 0).unwrap();
        let committee_len = committee.committee.len();

        let mut aggregation_bits =
            BitList::<<E as EthSpec>::MaxValidatorsPerSlot>::with_capacity(committee_len).unwrap();
        for i in 0..num_attesters.min(committee_len) {
            aggregation_bits.set(i, true).unwrap();
        }

        let mut committee_bits = BitVector::<<E as EthSpec>::MaxCommitteesPerSlot>::new();
        committee_bits.set(0, true).unwrap();

        Attestation::Electra(types::AttestationElectra {
            aggregation_bits,
            data: att_data,
            signature: types::AggregateSignature::empty(),
            committee_bits,
        })
    }

    #[test]
    fn previous_epoch_same_slot_attestation_uses_first_half_index() {
        // State at slot 17 (epoch 2). Attestation at slot 10 (epoch 1 = previous epoch).
        // Slot 10 is a same-slot attestation (block_roots[10] != block_roots[9]).
        // target.epoch = previous_epoch (1), so payment_slot_index = slot_mod = 10 % 8 = 2.
        // This tests the `else` branch at line 236: previous epoch → slot_mod as usize.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(17);

        // Payment in the first half (previous epoch): index = 10 % 8 = 2
        let slot_mod = 10 % E::slots_per_epoch() as usize; // 2
        set_payment(&mut state, slot_mod, 1_000_000_000);

        let attestation = make_prev_epoch_attestation(&state, 10, 0);

        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(slot_mod)
            .unwrap()
            .weight;
        assert_eq!(
            weight, 32_000_000_000,
            "previous-epoch same-slot attestation should add weight to first-half index"
        );
    }

    #[test]
    fn previous_epoch_attestation_does_not_touch_second_half() {
        // Same setup as above but verify the second-half slot (current epoch) is untouched.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(17);

        let slot_mod = 10 % E::slots_per_epoch() as usize; // 2
        let current_epoch_index = E::slots_per_epoch() as usize + slot_mod; // 10
        set_payment(&mut state, slot_mod, 500_000_000);
        set_payment(&mut state, current_epoch_index, 999_000_000);

        let attestation = make_prev_epoch_attestation(&state, 10, 0);
        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // First half should have weight added
        let first_half_weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(slot_mod)
            .unwrap()
            .weight;
        assert!(
            first_half_weight > 0,
            "previous-epoch payment should get weight"
        );

        // Second half should be untouched (weight stays 0)
        let second_half_weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(current_epoch_index)
            .unwrap()
            .weight;
        assert_eq!(
            second_half_weight, 0,
            "current-epoch payment at same slot_mod should NOT get weight from prev-epoch attestation"
        );
    }

    #[test]
    fn multiple_attesters_accumulate_combined_weight() {
        // An aggregate attestation from multiple committee members should add
        // the sum of their effective_balances to the payment weight.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(11);

        let payment_index = E::slots_per_epoch() as usize + (10 % E::slots_per_epoch() as usize);
        set_payment(&mut state, payment_index, 1_000_000_000);

        // Get how many members are in the committee for slot 10
        let committee = state.get_beacon_committee(Slot::new(10), 0).unwrap();
        let committee_len = committee.committee.len();
        assert!(committee_len >= 1, "need at least 1 committee member");

        // Create attestation with all committee members
        let attestation = make_multi_attester_attestation(&state, 10, 0, committee_len);

        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;

        // Each validator has effective_balance = 32_000_000_000
        let expected_weight = committee_len as u64 * 32_000_000_000;
        assert_eq!(
            weight, expected_weight,
            "weight should be sum of all attesting validators' effective_balances ({} validators)",
            committee_len
        );
    }

    #[test]
    fn epoch_boundary_slot_attestation_uses_correct_payment_index() {
        // Attestation at slot 8 (epoch 1 start, current epoch) at state slot 9.
        // payment_slot_index = 8 + (8 % 8) = 8 + 0 = 8
        // This tests that the epoch boundary slot maps to index 0 mod slots_per_epoch.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(9);

        let payment_index = E::slots_per_epoch() as usize; // 8 + 0 = 8
        set_payment(&mut state, payment_index, 2_000_000_000);

        let attestation = make_attestation(&state, 8, 0);

        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;
        assert_eq!(
            weight, 32_000_000_000,
            "epoch boundary attestation should use payment index SLOTS_PER_EPOCH + 0"
        );
    }

    #[test]
    fn weight_saturates_instead_of_overflowing() {
        // Set initial weight near u64::MAX to verify saturating_add behavior.
        let (mut state, spec, mut ctxt) = make_gloas_state_with_caches(11);

        let payment_index = E::slots_per_epoch() as usize + (10 % E::slots_per_epoch() as usize);
        set_payment(&mut state, payment_index, 1_000_000_000);

        // Set weight near max
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_payments
            .get_mut(payment_index)
            .unwrap()
            .weight = u64::MAX - 1;

        let attestation = make_attestation(&state, 10, 0);

        altair_deneb::process_attestation(
            &mut state,
            attestation.to_ref(),
            0,
            &mut ctxt,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let weight = state
            .as_gloas()
            .unwrap()
            .builder_pending_payments
            .get(payment_index)
            .unwrap()
            .weight;
        assert_eq!(
            weight,
            u64::MAX,
            "weight should saturate at u64::MAX instead of overflowing"
        );
    }

    // ── builder voluntary exit tests ────────────

    /// Create a state with builders and a finalized checkpoint so builders are active.
    /// Uses slot 512 (epoch 64 for minimal 8 slots/epoch) so validators pass
    /// shard_committee_period check too.
    fn make_state_with_builders() -> (BeaconState<E>, ChainSpec) {
        // slot 512 = epoch 64 for MinimalEthSpec (8 slots/epoch)
        let (mut state, spec, _ctxt) = make_gloas_state_with_caches(512);

        // Add an active builder (deposit_epoch=0, withdrawable_epoch=FAR_FUTURE)
        let builder = types::Builder {
            pubkey: PublicKeyBytes::empty(),
            version: 0,
            execution_address: Address::zero(),
            balance: 1_000_000,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: spec.far_future_epoch,
        };
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .push(builder)
            .unwrap();

        // Set finalized checkpoint epoch > deposit_epoch so builder is active
        state.as_gloas_mut().unwrap().finalized_checkpoint = Checkpoint {
            epoch: Epoch::new(1),
            root: Hash256::repeat_byte(0xDD),
        };

        (state, spec)
    }

    fn make_builder_exit(builder_index: u64, epoch: Epoch) -> SignedVoluntaryExit {
        use types::consts::gloas::BUILDER_INDEX_FLAG;
        SignedVoluntaryExit {
            message: VoluntaryExit {
                epoch,
                validator_index: builder_index | BUILDER_INDEX_FLAG,
            },
            signature: Signature::infinity().unwrap(),
        }
    }

    fn make_validator_exit_msg(validator_index: u64, epoch: Epoch) -> SignedVoluntaryExit {
        SignedVoluntaryExit {
            message: VoluntaryExit {
                epoch,
                validator_index,
            },
            signature: Signature::infinity().unwrap(),
        }
    }

    #[test]
    fn verify_exit_builder_returns_true() {
        let (state, spec) = make_state_with_builders();
        let exit = make_builder_exit(0, state.current_epoch());
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "builder exit should pass: {:?}",
            result.err()
        );
        assert!(
            result.unwrap(),
            "verify_exit should return true for builder exits"
        );
    }

    #[test]
    fn verify_exit_validator_returns_false() {
        let (state, spec) = make_state_with_builders();
        let exit = make_validator_exit_msg(1, state.current_epoch());
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "validator exit should pass: {:?}",
            result.err()
        );
        assert!(
            !result.unwrap(),
            "verify_exit should return false for validator exits"
        );
    }

    #[test]
    fn verify_exit_builder_unknown_index_rejected() {
        let (state, spec) = make_state_with_builders();
        let exit = make_builder_exit(999, state.current_epoch());
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(result.is_err(), "unknown builder index should be rejected");
    }

    #[test]
    fn verify_exit_builder_not_active_rejected() {
        let (mut state, spec) = make_state_with_builders();
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(50);
        let exit = make_builder_exit(0, state.current_epoch());
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(result.is_err(), "exiting builder should be rejected");
    }

    #[test]
    fn verify_exit_builder_with_pending_withdrawals_rejected() {
        let (mut state, spec) = make_state_with_builders();
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(types::BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 100,
                builder_index: 0,
            })
            .unwrap();
        let exit = make_builder_exit(0, state.current_epoch());
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(
            result.is_err(),
            "builder with pending withdrawals should be rejected"
        );
    }

    #[test]
    fn verify_exit_builder_future_epoch_rejected() {
        let (state, spec) = make_state_with_builders();
        let exit = make_builder_exit(0, Epoch::new(999));
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(result.is_err(), "future epoch should be rejected");
    }

    #[test]
    fn process_exits_builder_sets_withdrawable_epoch() {
        let (mut state, spec) = make_state_with_builders();
        let current_epoch = state.current_epoch();
        let exit = make_builder_exit(0, current_epoch);
        process_exits(&mut state, &[exit], VerifySignatures::False, &spec).unwrap();
        let builder = state.as_gloas().unwrap().builders.get(0).unwrap();
        assert_eq!(
            builder.withdrawable_epoch,
            current_epoch + spec.min_builder_withdrawability_delay,
        );
    }

    #[test]
    fn process_exits_mixed_builder_and_validator() {
        let (mut state, spec) = make_state_with_builders();
        let current_epoch = state.current_epoch();
        let builder_exit = make_builder_exit(0, current_epoch);
        let validator_exit = make_validator_exit_msg(1, current_epoch);
        process_exits(
            &mut state,
            &[builder_exit, validator_exit],
            VerifySignatures::False,
            &spec,
        )
        .unwrap();
        let builder = state.as_gloas().unwrap().builders.get(0).unwrap();
        assert_eq!(
            builder.withdrawable_epoch,
            current_epoch + spec.min_builder_withdrawability_delay,
        );
        let validator = state.get_validator(1).unwrap();
        assert_ne!(validator.exit_epoch, spec.far_future_epoch);
    }

    #[test]
    fn verify_exit_builder_with_pending_payment_rejected() {
        let (mut state, spec) = make_state_with_builders();
        // Set a pending payment for builder 0
        let state_gloas = state.as_gloas_mut().unwrap();
        *state_gloas.builder_pending_payments.get_mut(0).unwrap() = BuilderPendingPayment {
            weight: 0,
            withdrawal: types::BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 500,
                builder_index: 0,
            },
        };
        let exit = make_builder_exit(0, state.current_epoch());
        let result = verify_exit(&state, None, &exit, VerifySignatures::False, &spec);
        assert!(
            result.is_err(),
            "builder with pending payment should be rejected"
        );
    }
}
