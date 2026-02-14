use crate::common::decrease_balance;
use crate::per_block_processing::errors::{BlockProcessingError, PayloadAttestationInvalid};
use crate::VerifySignatures;
use safe_arith::SafeArith;
use swap_or_not_shuffle::compute_shuffled_index;
use tree_hash::TreeHash;
use types::consts::gloas::{BUILDER_INDEX_FLAG, PTC_SIZE};
use types::{
    BeaconState, BeaconStateError, BuilderPendingPayment, BuilderPendingWithdrawal, ChainSpec,
    Domain, EthSpec, Hash256, IndexedPayloadAttestation, List, PayloadAttestation,
    PublicKey, SignedExecutionPayloadBid, SigningData, Slot, Unsigned, Withdrawal,
};

/// Processes an execution payload bid in Gloas ePBS.
///
/// This validates the builder's bid and updates the state with the chosen bid.
/// The proposer may choose the highest valid bid or self-build (value = 0).
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#new-process_execution_payload_bid
pub fn process_execution_payload_bid<E: EthSpec>(
    state: &mut BeaconState<E>,
    signed_bid: &SignedExecutionPayloadBid<E>,
    block_slot: Slot,
    block_parent_root: Hash256,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    let bid = &signed_bid.message;
    let builder_index = bid.builder_index;
    let amount = bid.value;

    // Self-build validation
    if builder_index == spec.builder_index_self_build {
        if amount != 0 {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: "self-build bid must have value = 0".into(),
            });
        }
        if !signed_bid.signature.is_infinity() {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: "self-build bid signature must be G2_POINT_AT_INFINITY".into(),
            });
        }
    } else {
        // Extract values needed for validation before taking mutable borrow
        let finalized_epoch = state.finalized_checkpoint().epoch;
        let fork = state.fork();
        let genesis_validators_root = state.genesis_validators_root();

        let state_gloas = state.as_gloas().map_err(|_| {
            BlockProcessingError::PayloadBidInvalid {
                reason: "state is not Gloas".into(),
            }
        })?;

        let builder = state_gloas
            .builders
            .get(builder_index as usize)
            .ok_or_else(|| BlockProcessingError::PayloadBidInvalid {
                reason: format!("builder index {} does not exist", builder_index),
            })?;

        // Verify that the builder is active
        if !builder.is_active_at_finalized_epoch(finalized_epoch, spec) {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: format!("builder {} is not active", builder_index),
            });
        }

        // Verify that the builder has funds to cover the bid (can_builder_cover_bid)
        // Calculate total pending payments for this builder
        let mut total_pending = 0u64;
        for payment in state_gloas.builder_pending_payments.iter() {
            if payment.withdrawal.builder_index == builder_index {
                total_pending = total_pending.saturating_add(payment.withdrawal.amount);
            }
        }
        // Also account for pending withdrawals
        for withdrawal in state_gloas.builder_pending_withdrawals.iter() {
            if withdrawal.builder_index == builder_index {
                total_pending = total_pending.saturating_add(withdrawal.amount);
            }
        }
        if builder.balance < amount.saturating_add(total_pending) {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: format!(
                    "builder balance {} insufficient for bid value {} + pending {}",
                    builder.balance, amount, total_pending
                ),
            });
        }

        // Verify signature if requested
        if verify_signatures.is_true() {
            let domain = spec.get_domain(
                bid.slot.epoch(E::slots_per_epoch()),
                Domain::BeaconBuilder,
                &fork,
                genesis_validators_root,
            );

            let signing_root = SigningData {
                object_root: bid.tree_hash_root(),
                domain,
            }
            .tree_hash_root();

            let pubkey = builder
                .pubkey
                .decompress()
                .map_err(|_| BlockProcessingError::PayloadBidInvalid {
                    reason: format!("failed to decompress builder {} pubkey", builder_index),
                })?;

            if !signed_bid.signature.verify(&pubkey, signing_root) {
                return Err(BlockProcessingError::PayloadBidInvalid {
                    reason: format!("invalid builder {} signature", builder_index),
                });
            }
        }
    }

    // Verify commitments are under limit
    let max_blobs = spec.max_blobs_per_block(state.current_epoch());
    if bid.blob_kzg_commitments.len() > max_blobs as usize {
        return Err(BlockProcessingError::PayloadBidInvalid {
            reason: format!(
                "blob_kzg_commitments length {} exceeds max {}",
                bid.blob_kzg_commitments.len(),
                max_blobs
            ),
        });
    }

    // Verify that the bid is for the current slot
    if bid.slot != block_slot {
        return Err(BlockProcessingError::PayloadBidInvalid {
            reason: format!(
                "bid slot {} does not match block slot {}",
                bid.slot, block_slot
            ),
        });
    }

    // Verify that the bid is for the right parent block
    if bid.parent_block_hash != state.as_gloas().map_err(|_| {
        BlockProcessingError::PayloadBidInvalid {
            reason: "state is not Gloas".into(),
        }
    })?.latest_block_hash {
        return Err(BlockProcessingError::PayloadBidInvalid {
            reason: "bid parent_block_hash does not match state latest_block_hash".into(),
        });
    }

    if bid.parent_block_root != block_parent_root {
        return Err(BlockProcessingError::PayloadBidInvalid {
            reason: "bid parent_block_root does not match block parent_root".into(),
        });
    }

    // Verify prev_randao
    let current_epoch = state.current_epoch();
    let randao_mix = *state.get_randao_mix(current_epoch).map_err(|_| {
        BlockProcessingError::PayloadBidInvalid {
            reason: "failed to get randao mix".into(),
        }
    })?;
    if bid.prev_randao != randao_mix {
        return Err(BlockProcessingError::PayloadBidInvalid {
            reason: "bid prev_randao does not match state randao mix".into(),
        });
    }

    // Record the pending payment if there is some payment
    if amount > 0 {
        let slot_index = (bid.slot.as_u64() % E::BuilderPendingPaymentsLimit::to_u64()) as usize;

        let pending_payment = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: bid.fee_recipient,
                amount,
                builder_index,
            },
        };

        let state_gloas = state.as_gloas_mut().map_err(|_| {
            BlockProcessingError::PayloadBidInvalid {
                reason: "state is not Gloas".into(),
            }
        })?;

        *state_gloas
            .builder_pending_payments
            .get_mut(slot_index)
            .ok_or(BlockProcessingError::PayloadBidInvalid {
                reason: format!("slot index {} out of bounds for builder_pending_payments", slot_index),
            })? = pending_payment;
    }

    // Cache the signed execution payload bid
    let state_gloas = state.as_gloas_mut().map_err(|_| {
        BlockProcessingError::PayloadBidInvalid {
            reason: "state is not Gloas".into(),
        }
    })?;
    state_gloas.latest_execution_payload_bid = bid.clone();

    Ok(())
}

/// Processes payload attestations from the PTC (Payload Timeliness Committee).
///
/// PTC members attest to whether the execution payload was revealed on time
/// and blob data is available. Once enough attestations are collected
/// (quorum threshold), the builder gets paid.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#new-process_payload_attestation
pub fn process_payload_attestation<E: EthSpec>(
    state: &mut BeaconState<E>,
    attestation: &PayloadAttestation<E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    let data = &attestation.data;

    // Verify attestation is for the current slot
    if data.slot != state.slot() {
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::WrongSlot {
                expected: state.slot(),
                actual: data.slot,
            },
        ));
    }

    // Verify beacon_block_root matches
    if data.beacon_block_root != state.latest_block_header().tree_hash_root() {
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::WrongBeaconBlockRoot,
        ));
    }

    // Convert to indexed form for validation
    let indexed_attestation = get_indexed_payload_attestation(state, attestation, spec)?;

    // Verify the attestation signature if requested
    if verify_signatures.is_true() {
        // Verify aggregate payload attestation signature
        let domain = spec.get_domain(
            data.slot.epoch(E::slots_per_epoch()),
            Domain::PtcAttester,
            &state.fork(),
            state.genesis_validators_root(),
        );

        let signing_root = SigningData {
            object_root: data.tree_hash_root(),
            domain,
        }
        .tree_hash_root();

        // Collect public keys from all attesting indices
        let mut pubkeys = Vec::new();
        for &validator_index in indexed_attestation.attesting_indices.iter() {
            let validator = state
                .validators()
                .get(validator_index as usize)
                .ok_or(BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::AttesterIndexOutOfBounds,
                ))?;

            let pubkey = validator
                .pubkey
                .decompress()
                .map_err(|_| BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::InvalidPubkey,
                ))?;

                    pubkeys.push(pubkey);
        }

        // Verify the aggregate signature  
        let pubkey_refs: Vec<&PublicKey> = pubkeys.iter().collect();
        if !attestation
            .signature
            .fast_aggregate_verify(signing_root, &pubkey_refs)
        {
            return Err(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::BadSignature,
            ));
        }
    }

    // Update execution_payload_availability based on attestation data
    let state_gloas = state.as_gloas_mut().map_err(|_| {
        BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::IncorrectStateVariant,
        )
    })?;

    let slot_index = data.slot.as_usize() % E::SlotsPerHistoricalRoot::to_usize();
    
    // Update availability flag if payload_present (any attestation can mark it)
    if data.payload_present {
        state_gloas
            .execution_payload_availability
            .set(slot_index, true)
            .map_err(|_| {
                BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::SlotOutOfBounds,
                )
            })?;
    }

    // Accumulate weight from this attestation
    let num_attesters = attestation.num_attesters() as u64;
    let payment_slot_index =
        (data.slot.as_u64() % E::BuilderPendingPaymentsLimit::to_u64()) as usize;
    let pending_payment = state_gloas.builder_pending_payments.get_mut(payment_slot_index)
        .ok_or(BlockProcessingError::InvalidSlotIndex(payment_slot_index))?;

    // Add weight from this attestation
    let new_weight = pending_payment.weight.saturating_add(num_attesters);
    pending_payment.weight = new_weight;

    // Check if we've reached quorum (60% of PTC)
    let quorum_threshold = (PTC_SIZE * spec.builder_payment_threshold_numerator)
        / spec.builder_payment_threshold_denominator;

    // Process builder payment if we've crossed quorum threshold AND payload was revealed
    if new_weight >= quorum_threshold && data.payload_present {
        let builder_index = pending_payment.withdrawal.builder_index as usize;
        let payment_amount = pending_payment.withdrawal.amount;

        // Only process payment if it hasn't been done yet (weight was below threshold before)
        if pending_payment.weight.saturating_sub(num_attesters) < quorum_threshold {
            // Decrease builder balance
            if let Some(builder) = state_gloas.builders.get_mut(builder_index) {
                if builder.balance < payment_amount {
                    return Err(BlockProcessingError::PayloadBidInvalid {
                        reason: format!(
                            "builder {} has insufficient balance {} for payment {}",
                            builder_index, builder.balance, payment_amount
                        ),
                    });
                }
                builder.balance = builder.balance.saturating_sub(payment_amount);
            }

            // TODO: Increase proposer balance
            // Need to get proposer index for the slot - requires ConsensusContext
        }
    }

    Ok(())
}

/// Converts a PayloadAttestation to IndexedPayloadAttestation.
///
/// This unpacks the aggregation bitfield into an explicit list of validator indices
/// for efficient signature verification.
fn get_indexed_payload_attestation<E: EthSpec>(
    state: &BeaconState<E>,
    attestation: &PayloadAttestation<E>,
    spec: &ChainSpec,
) -> Result<IndexedPayloadAttestation<E>, BlockProcessingError> {
    let ptc_indices = get_ptc_committee(state, attestation.data.slot, spec)?;

    // Convert aggregation bits to list of attesting indices
    let mut attesting_indices = Vec::new();
    for (i, &validator_index) in ptc_indices.iter().enumerate() {
        if attestation.aggregation_bits.get(i).map_err(|_| {
            BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::AttesterIndexOutOfBounds,
            )
        })? {
            attesting_indices.push(validator_index);
        }
    }

    // Verify indices are sorted (required by spec)
    if !attesting_indices.windows(2).all(|w| w[0] < w[1]) {
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::IndicesNotSorted,
        ));
    }

    Ok(IndexedPayloadAttestation {
        attesting_indices: attesting_indices.try_into().map_err(|_| {
            BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::AttesterIndexOutOfBounds,
            )
        })?,
        data: attestation.data.clone(),
        signature: attestation.signature.clone(),
    })
}

/// Computes the PTC (Payload Timeliness Committee) for a given slot.
///
/// The PTC is a subset of 512 validators selected per slot who attest to
/// payload delivery and blob availability. The selection is based on a
/// deterministic shuffle using the slot's seed.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#get_ptc_committee
fn get_ptc_committee<E: EthSpec>(
    state: &BeaconState<E>,
    slot: Slot,
    spec: &ChainSpec,
) -> Result<Vec<u64>, BlockProcessingError> {
    let epoch = slot.epoch(E::slots_per_epoch());
    let active_validator_indices = state.get_active_validator_indices(epoch, spec)?;
    let active_validator_count = active_validator_indices.len();

    if active_validator_count == 0 {
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::NoActiveValidators,
        ));
    }

    // Get seed for this slot using domain PTC_ATTESTER
    // TODO: The spec may define a specific domain for PTC. For now, use a slot-based seed.
    let seed = state.get_beacon_proposer_seed(slot, spec)?;

    let ptc_size = PTC_SIZE as usize;
    let mut ptc_committee = Vec::with_capacity(ptc_size);
    let mut i = 0;

    // Select PTC_SIZE validators using shuffled indices
    while ptc_committee.len() < ptc_size && i < active_validator_count * 10 {
        let shuffled_index = compute_shuffled_index(
            i % active_validator_count,
            active_validator_count,
            seed.as_slice(),
            spec.shuffle_round_count,
        )
        .ok_or(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::ShuffleError,
        ))?;

        let candidate_index = *active_validator_indices
            .get(shuffled_index)
            .ok_or(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::AttesterIndexOutOfBounds,
            ))?;

        // Add to committee (no duplicates check since shuffled_index is unique)
        ptc_committee.push(candidate_index as u64);

        i += 1;
    }

    if ptc_committee.len() < ptc_size {
        // Not enough validators to form a full PTC (edge case for testnets)
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::InsufficientValidators,
        ));
    }

    Ok(ptc_committee)
}

/// [Modified in Gloas:EIP7732] Check if the parent block had its payload delivered.
pub fn is_parent_block_full<E: EthSpec>(
    state: &BeaconState<E>,
) -> Result<bool, BlockProcessingError> {
    let state_gloas = state.as_gloas().map_err(|e| {
        BlockProcessingError::BeaconStateError(e)
    })?;
    Ok(state_gloas.latest_execution_payload_bid.block_hash == state_gloas.latest_block_hash)
}

/// [Modified in Gloas:EIP7732] Process withdrawals without execution payload.
///
/// In Gloas, withdrawals are computed by the CL and stored in `payload_expected_withdrawals`
/// for the EL to include. The function computes expected withdrawals from builder pending
/// withdrawals, partial validator withdrawals, builder sweep, and validator sweep.
pub fn process_withdrawals_gloas<E: EthSpec>(
    state: &mut BeaconState<E>,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    // Return early if the parent block is empty (payload not delivered)
    if !is_parent_block_full::<E>(state)? {
        return Ok(());
    }

    let epoch = state.current_epoch();
    let fork_name = state.fork_name_unchecked();
    let mut withdrawal_index = state.next_withdrawal_index()?;
    let mut withdrawals = Vec::<Withdrawal>::new();
    let withdrawals_limit = E::max_withdrawals_per_payload();

    // 1. Builder pending withdrawals
    let mut processed_builder_withdrawals_count: usize = 0;
    {
        let state_gloas = state.as_gloas().map_err(|e| {
            BlockProcessingError::BeaconStateError(e)
        })?;
        let builders_count = state_gloas.builders.len() as u64;
        
        for withdrawal in state_gloas.builder_pending_withdrawals.iter() {
            if withdrawals.len() >= withdrawals_limit {
                break;
            }
            let builder_index = withdrawal.builder_index;
            
            // Validate builder_index exists in builders list
            if builder_index >= builders_count {
                return Err(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                    builder_index,
                    builders_count,
                });
            }
            
            withdrawals.push(Withdrawal {
                index: withdrawal_index,
                validator_index: builder_index | BUILDER_INDEX_FLAG,
                address: withdrawal.fee_recipient,
                amount: withdrawal.amount,
            });
            withdrawal_index.safe_add_assign(1)?;
            processed_builder_withdrawals_count += 1;
        }
    }

    // 2. Pending partial withdrawals (validator)
    let mut processed_partial_withdrawals_count: usize = 0;
    {
        if let Ok(pending_partial_withdrawals) = state.pending_partial_withdrawals() {
            for withdrawal_req in pending_partial_withdrawals {
                if withdrawal_req.withdrawable_epoch > epoch
                    || withdrawals.len() >= spec.max_pending_partials_per_withdrawals_sweep as usize
                {
                    break;
                }

                let validator = state.get_validator(withdrawal_req.validator_index as usize)?;

                let has_sufficient_effective_balance =
                    validator.effective_balance >= spec.min_activation_balance;
                let total_withdrawn: u64 = withdrawals
                    .iter()
                    .filter(|w| w.validator_index == withdrawal_req.validator_index)
                    .map(|w| w.amount)
                    .sum();
                let balance = state
                    .get_balance(withdrawal_req.validator_index as usize)?
                    .saturating_sub(total_withdrawn);
                let has_excess_balance = balance > spec.min_activation_balance;

                if validator.exit_epoch == spec.far_future_epoch
                    && has_sufficient_effective_balance
                    && has_excess_balance
                {
                    let withdrawable_balance = std::cmp::min(
                        balance.saturating_sub(spec.min_activation_balance),
                        withdrawal_req.amount,
                    );
                    withdrawals.push(Withdrawal {
                        index: withdrawal_index,
                        validator_index: withdrawal_req.validator_index,
                        address: validator
                            .get_execution_withdrawal_address(spec)
                            .ok_or(BeaconStateError::NonExecutionAddressWithdrawalCredential)?,
                        amount: withdrawable_balance,
                    });
                    withdrawal_index.safe_add_assign(1)?;
                }
                processed_partial_withdrawals_count += 1;
            }
        }
    }

    // 3. Builder sweep (exiting builders with balance)
    let mut processed_builders_sweep_count: usize = 0;
    {
        let state_gloas = state.as_gloas().map_err(|e| {
            BlockProcessingError::BeaconStateError(e)
        })?;
        let builders_count = state_gloas.builders.len();
        
        // Validate next_withdrawal_builder_index is within bounds
        if builders_count > 0 && state_gloas.next_withdrawal_builder_index >= builders_count as u64 {
            return Err(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                builder_index: state_gloas.next_withdrawal_builder_index,
                builders_count: builders_count as u64,
            });
        }
        
        if builders_count > 0 {
            let builders_limit = std::cmp::min(
                builders_count,
                spec.max_builders_per_withdrawals_sweep as usize,
            );
            let mut builder_index = state_gloas.next_withdrawal_builder_index;
            for _ in 0..builders_limit {
                if withdrawals.len() >= withdrawals_limit {
                    break;
                }

                if let Some(builder) = state_gloas.builders.get(builder_index as usize) {
                    if builder.withdrawable_epoch <= epoch && builder.balance > 0 {
                        withdrawals.push(Withdrawal {
                            index: withdrawal_index,
                            validator_index: builder_index | BUILDER_INDEX_FLAG,
                            address: builder.execution_address,
                            amount: builder.balance,
                        });
                        withdrawal_index.safe_add_assign(1)?;
                    }
                }

                builder_index = (builder_index + 1) % builders_count as u64;
                processed_builders_sweep_count += 1;
            }
        }
    }

    // 4. Validator sweep
    {
        let mut validator_index = state.next_withdrawal_validator_index()?;
        let bound = std::cmp::min(
            state.validators().len() as u64,
            spec.max_validators_per_withdrawals_sweep,
        );
        for _ in 0..bound {
            if withdrawals.len() >= E::max_withdrawals_per_payload() {
                break;
            }

            let validator = state.get_validator(validator_index as usize)?;
            let partially_withdrawn_balance: u64 = withdrawals
                .iter()
                .filter(|w| w.validator_index == validator_index)
                .map(|w| w.amount)
                .sum();
            let balance = state
                .get_balance(validator_index as usize)?
                .saturating_sub(partially_withdrawn_balance);
            if validator.is_fully_withdrawable_validator(balance, epoch, spec, fork_name) {
                withdrawals.push(Withdrawal {
                    index: withdrawal_index,
                    validator_index,
                    address: validator
                        .get_execution_withdrawal_address(spec)
                        .ok_or(BlockProcessingError::WithdrawalCredentialsInvalid)?,
                    amount: balance,
                });
                withdrawal_index.safe_add_assign(1)?;
            } else if validator.is_partially_withdrawable_validator(balance, spec, fork_name) {
                withdrawals.push(Withdrawal {
                    index: withdrawal_index,
                    validator_index,
                    address: validator
                        .get_execution_withdrawal_address(spec)
                        .ok_or(BlockProcessingError::WithdrawalCredentialsInvalid)?,
                    amount: balance.saturating_sub(validator.get_max_effective_balance(spec, fork_name)),
                });
                withdrawal_index.safe_add_assign(1)?;
            }

            validator_index = validator_index
                .safe_add(1)?
                .safe_rem(state.validators().len() as u64)?;
        }
    }

    // Apply withdrawals: decrease balances
    for withdrawal in &withdrawals {
        if (withdrawal.validator_index & BUILDER_INDEX_FLAG) != 0 {
            // Builder withdrawal
            let builder_index = (withdrawal.validator_index & !BUILDER_INDEX_FLAG) as usize;
            let state_gloas = state.as_gloas_mut().map_err(|e| {
                BlockProcessingError::BeaconStateError(e)
            })?;
            if let Some(builder) = state_gloas.builders.get_mut(builder_index) {
                builder.balance = builder
                    .balance
                    .saturating_sub(std::cmp::min(withdrawal.amount, builder.balance));
            }
        } else {
            // Validator withdrawal
            decrease_balance(state, withdrawal.validator_index as usize, withdrawal.amount)?;
        }
    }

    // Update next_withdrawal_index
    if let Some(latest_withdrawal) = withdrawals.last() {
        *state.next_withdrawal_index_mut()? = latest_withdrawal.index.safe_add(1)?;
    }

    // Store payload_expected_withdrawals
    {
        let state_gloas = state.as_gloas_mut().map_err(|e| {
            BlockProcessingError::BeaconStateError(e)
        })?;
        state_gloas.payload_expected_withdrawals = List::new(withdrawals)
            .map_err(|e| BlockProcessingError::MilhouseError(e))?;
    }

    // Update builder_pending_withdrawals (remove processed)
    {
        let state_gloas = state.as_gloas_mut().map_err(|e| {
            BlockProcessingError::BeaconStateError(e)
        })?;
        let remaining: Vec<_> = state_gloas
            .builder_pending_withdrawals
            .iter()
            .skip(processed_builder_withdrawals_count)
            .cloned()
            .collect();
        state_gloas.builder_pending_withdrawals = List::new(remaining)
            .map_err(|e| BlockProcessingError::MilhouseError(e))?;
    }

    // Update pending_partial_withdrawals (remove processed)
    if processed_partial_withdrawals_count > 0 {
        state
            .pending_partial_withdrawals_mut()?
            .pop_front(processed_partial_withdrawals_count)?;
    }

    // Update next_withdrawal_builder_index
    {
        let state_gloas = state.as_gloas_mut().map_err(|e| {
            BlockProcessingError::BeaconStateError(e)
        })?;
        let builders_count = state_gloas.builders.len();
        if builders_count > 0 {
            let next_index = state_gloas
                .next_withdrawal_builder_index
                .saturating_add(processed_builders_sweep_count as u64);
            state_gloas.next_withdrawal_builder_index = next_index % builders_count as u64;
        }
    }

    // Update next_withdrawal_validator_index
    // The spec says to update based on the validator sweep results
    // For now, use the same logic as pre-Gloas
    {
        let validators_len = state.validators().len() as u64;
        if validators_len > 0 {
            let next_validator_index = state
                .next_withdrawal_validator_index()?
                .safe_add(spec.max_validators_per_withdrawals_sweep)?
                .safe_rem(validators_len)?;
            *state.next_withdrawal_validator_index_mut()? = next_validator_index;
        }
    }

    Ok(())
}
