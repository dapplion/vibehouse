use crate::common::decrease_balance;
use crate::per_block_processing::errors::{BlockProcessingError, PayloadAttestationInvalid};
use crate::VerifySignatures;
use safe_arith::SafeArith;
use ethereum_hashing::hash;
use int_to_bytes::int_to_bytes8;
use tree_hash::TreeHash;
use types::consts::gloas::BUILDER_INDEX_FLAG;
use types::{
    BeaconState, BeaconStateError, BuilderPendingPayment, BuilderPendingWithdrawal, ChainSpec,
    Domain, EthSpec, Hash256, IndexedPayloadAttestation, List, PayloadAttestation, PublicKey,
    SignedExecutionPayloadBid, SigningData, Slot, Unsigned, Withdrawal,
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
        // Spec: get_pending_balance_to_withdraw_for_builder
        let mut pending_withdrawals_amount = 0u64;
        for withdrawal in state_gloas.builder_pending_withdrawals.iter() {
            if withdrawal.builder_index == builder_index {
                pending_withdrawals_amount =
                    pending_withdrawals_amount.saturating_add(withdrawal.amount);
            }
        }
        for payment in state_gloas.builder_pending_payments.iter() {
            if payment.withdrawal.builder_index == builder_index {
                pending_withdrawals_amount =
                    pending_withdrawals_amount.saturating_add(payment.withdrawal.amount);
            }
        }
        // Spec: min_balance = MIN_DEPOSIT_AMOUNT + pending_withdrawals_amount
        let min_balance = spec
            .min_deposit_amount
            .saturating_add(pending_withdrawals_amount);
        if builder.balance < min_balance
            || builder.balance.saturating_sub(min_balance) < amount
        {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: format!(
                    "builder balance {} insufficient for bid value {} (min_balance {})",
                    builder.balance, amount, min_balance
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
        // Spec: state.builder_pending_payments[SLOTS_PER_EPOCH + bid.slot % SLOTS_PER_EPOCH]
        let slots_per_epoch = E::slots_per_epoch();
        let slot_index = (slots_per_epoch + bid.slot.as_u64() % slots_per_epoch) as usize;

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

/// Processes a payload attestation from the PTC (Payload Timeliness Committee).
///
/// Spec: process_payload_attestation(state, payload_attestation)
/// Validates that the attestation targets the parent block at the previous slot
/// and verifies the aggregate BLS signature.
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#new-process_payload_attestation
pub fn process_payload_attestation<E: EthSpec>(
    state: &mut BeaconState<E>,
    attestation: &PayloadAttestation<E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    let data = &attestation.data;

    // Spec: assert data.beacon_block_root == state.latest_block_header.parent_root
    if data.beacon_block_root != state.latest_block_header().parent_root {
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::WrongBeaconBlockRoot,
        ));
    }

    // Spec: assert data.slot + 1 == state.slot
    let expected_slot = data.slot.safe_add(1u64).map_err(|_| {
        BlockProcessingError::PayloadAttestationInvalid(PayloadAttestationInvalid::WrongSlot {
            expected: state.slot(),
            actual: data.slot,
        })
    })?;
    if expected_slot != state.slot() {
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::WrongSlot {
                expected: state.slot(),
                actual: data.slot,
            },
        ));
    }

    // Spec: indexed_payload_attestation = get_indexed_payload_attestation(state, payload_attestation)
    // Spec: assert is_valid_indexed_payload_attestation(state, indexed_payload_attestation)
    let indexed_attestation = get_indexed_payload_attestation(state, attestation, spec)?;

    if verify_signatures.is_true() {
        let indices = &indexed_attestation.attesting_indices;
        if indices.is_empty() {
            return Err(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::AttesterIndexOutOfBounds,
            ));
        }

        // Verify indices are sorted (non-decreasing, duplicates allowed)
        for i in 1..indices.len() {
            if indices[i] < indices[i - 1] {
                return Err(BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::AttesterIndexOutOfBounds,
                ));
            }
        }

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

        let mut pubkeys = Vec::with_capacity(indices.len());
        for &validator_index in indices.iter() {
            let validator = state
                .validators()
                .get(validator_index as usize)
                .ok_or(BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::AttesterIndexOutOfBounds,
                ))?;

            let pubkey = validator.pubkey.decompress().map_err(|_| {
                BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::InvalidPubkey,
                )
            })?;

            pubkeys.push(pubkey);
        }

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
    // Spec: attesting_indices = [index for i, index in enumerate(ptc) if bits[i]]
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

    // Spec: attesting_indices = sorted(attesting_indices)
    attesting_indices.sort_unstable();

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
/// Spec: get_ptc(state, slot)
/// 1. Concatenate all beacon committees for the slot
/// 2. Use compute_balance_weighted_selection to pick PTC_SIZE validators
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#get_ptc
pub fn get_ptc_committee<E: EthSpec>(
    state: &BeaconState<E>,
    slot: Slot,
    spec: &ChainSpec,
) -> Result<Vec<u64>, BlockProcessingError> {
    let epoch = slot.epoch(E::slots_per_epoch());

    // Spec: seed = hash(get_seed(state, epoch, DOMAIN_PTC_ATTESTER) + uint_to_bytes(slot))
    let base_seed = state
        .get_seed(epoch, Domain::PtcAttester, spec)
        .map_err(BlockProcessingError::BeaconStateError)?;
    let slot_bytes = int_to_bytes8(slot.as_u64());
    let mut seed_input = [0u8; 40]; // 32 + 8
    seed_input[..32].copy_from_slice(base_seed.as_slice());
    seed_input[32..].copy_from_slice(&slot_bytes);
    let seed = hash(&seed_input);

    // Concatenate all committees for this slot in order
    let committees = state
        .get_beacon_committees_at_slot(slot)
        .map_err(BlockProcessingError::BeaconStateError)?;
    let mut indices: Vec<u64> = Vec::new();
    for committee in &committees {
        for &validator_index in committee.committee {
            indices.push(validator_index as u64);
        }
    }

    // compute_balance_weighted_selection(state, indices, seed, PTC_SIZE, shuffle_indices=False)
    let ptc_size = E::PtcSize::to_usize();
    let total = indices.len();
    if total == 0 {
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::NoActiveValidators,
        ));
    }

    let max_effective_balance = spec.max_effective_balance_electra;
    let max_random_value: u64 = (1u64 << 16) - 1; // 2^16 - 1

    let mut selected: Vec<u64> = Vec::with_capacity(ptc_size);
    let mut i: u64 = 0;
    while selected.len() < ptc_size {
        let next_index = (i % total as u64) as usize;
        // shuffle_indices=False, so just use next_index directly
        let candidate_index = indices[next_index];

        // compute_balance_weighted_acceptance
        let random_bytes = hash(
            &[&seed[..], &int_to_bytes8(i / 16)].concat(),
        );
        let offset = ((i % 16) * 2) as usize;
        let random_value =
            u16::from_le_bytes([random_bytes[offset], random_bytes[offset + 1]]) as u64;

        let effective_balance = state
            .validators()
            .get(candidate_index as usize)
            .ok_or(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::AttesterIndexOutOfBounds,
            ))?
            .effective_balance;

        if effective_balance * max_random_value >= max_effective_balance * random_value {
            selected.push(candidate_index);
        }
        i += 1;
    }

    Ok(selected)
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
    let max_withdrawals = E::max_withdrawals_per_payload();
    // Builders, partials, and builder sweep reserve 1 slot for the validator sweep
    let reserved_limit = max_withdrawals.saturating_sub(1);
    let mut withdrawal_index = state.next_withdrawal_index()?;
    let mut withdrawals = Vec::<Withdrawal>::new();

    // 1. Builder pending withdrawals (limit: MAX_WITHDRAWALS_PER_PAYLOAD - 1)
    // Spec: get_builder_withdrawals(state, withdrawal_index, withdrawals)
    // Note: The spec does NOT validate builder_index here. Invalid indices are
    // caught later in apply_withdrawals when accessing state.builders[builder_index].
    let mut processed_builder_withdrawals_count: usize = 0;
    {
        let state_gloas = state.as_gloas().map_err(|e| {
            BlockProcessingError::BeaconStateError(e)
        })?;
        for withdrawal in state_gloas.builder_pending_withdrawals.iter() {
            if withdrawals.len() >= reserved_limit {
                break;
            }
            let builder_index = withdrawal.builder_index;
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
    // Spec: withdrawals_limit = min(prior_count + MAX_PENDING_PARTIALS, MAX_WITHDRAWALS - 1)
    let mut processed_partial_withdrawals_count: usize = 0;
    {
        let partials_limit = std::cmp::min(
            withdrawals.len().saturating_add(spec.max_pending_partials_per_withdrawals_sweep as usize),
            reserved_limit,
        );
        if let Ok(pending_partial_withdrawals) = state.pending_partial_withdrawals() {
            for withdrawal_req in pending_partial_withdrawals {
                let is_withdrawable = withdrawal_req.withdrawable_epoch <= epoch;
                let has_reached_limit = withdrawals.len() >= partials_limit;
                if !is_withdrawable || has_reached_limit {
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

    // 3. Builder sweep (exiting builders with balance, limit: MAX_WITHDRAWALS - 1)
    // Spec: get_builders_sweep_withdrawals(state, withdrawal_index, withdrawals)
    let mut processed_builders_sweep_count: usize = 0;
    {
        let state_gloas = state.as_gloas().map_err(|e| {
            BlockProcessingError::BeaconStateError(e)
        })?;
        let builders_count = state_gloas.builders.len();
        if builders_count > 0 {
            let builders_limit = std::cmp::min(
                builders_count,
                spec.max_builders_per_withdrawals_sweep as usize,
            );
            let mut builder_index = state_gloas.next_withdrawal_builder_index;
            for _ in 0..builders_limit {
                if withdrawals.len() >= reserved_limit {
                    break;
                }

                // Spec accesses state.builders[builder_index] directly.
                // With wrapping modular arithmetic, this should always be in-bounds.
                let bi = (builder_index % builders_count as u64) as usize;
                let builder = state_gloas
                    .builders
                    .get(bi)
                    .ok_or(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                        builder_index,
                        builders_count: builders_count as u64,
                    })?;
                if builder.withdrawable_epoch <= epoch && builder.balance > 0 {
                    withdrawals.push(Withdrawal {
                        index: withdrawal_index,
                        validator_index: builder_index | BUILDER_INDEX_FLAG,
                        address: builder.execution_address,
                        amount: builder.balance,
                    });
                    withdrawal_index.safe_add_assign(1)?;
                }

                builder_index = (builder_index + 1) % builders_count as u64;
                processed_builders_sweep_count += 1;
            }
        }
    }

    // 4. Validator sweep (limit: MAX_WITHDRAWALS_PER_PAYLOAD — the full limit)
    {
        let mut validator_index = state.next_withdrawal_validator_index()?;
        let bound = std::cmp::min(
            state.validators().len() as u64,
            spec.max_validators_per_withdrawals_sweep,
        );
        for _ in 0..bound {
            if withdrawals.len() >= max_withdrawals {
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
    // Spec: apply_withdrawals(state, withdrawals)
    for withdrawal in &withdrawals {
        if (withdrawal.validator_index & BUILDER_INDEX_FLAG) != 0 {
            // Builder withdrawal
            let builder_index = (withdrawal.validator_index & !BUILDER_INDEX_FLAG) as usize;
            let state_gloas = state.as_gloas_mut().map_err(|e| {
                BlockProcessingError::BeaconStateError(e)
            })?;
            let builders_count = state_gloas.builders.len() as u64;
            let builder = state_gloas
                .builders
                .get_mut(builder_index)
                .ok_or(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                    builder_index: builder_index as u64,
                    builders_count,
                })?;
            builder.balance = builder
                .balance
                .saturating_sub(std::cmp::min(withdrawal.amount, builder.balance));
        } else {
            // Validator withdrawal
            decrease_balance(state, withdrawal.validator_index as usize, withdrawal.amount)?;
        }
    }

    // Update next_withdrawal_index
    if !withdrawals.is_empty() {
        let latest_withdrawal = withdrawals.last().unwrap();
        *state.next_withdrawal_index_mut()? = latest_withdrawal.index.safe_add(1)?;
    }

    // Store payload_expected_withdrawals
    {
        let state_gloas = state.as_gloas_mut().map_err(|e| {
            BlockProcessingError::BeaconStateError(e)
        })?;
        state_gloas.payload_expected_withdrawals = List::new(withdrawals.clone())
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
    // Spec: if withdrawals hit MAX, next = (last.validator_index + 1) % len
    //       else, next = (current + MAX_VALIDATORS_PER_SWEEP) % len
    {
        let validators_len = state.validators().len() as u64;
        if validators_len > 0 {
            let next_validator_index = if withdrawals.len() == max_withdrawals {
                let last_validator_index = withdrawals
                    .last()
                    .map(|w| w.validator_index)
                    .unwrap_or(0);
                last_validator_index
                    .safe_add(1)?
                    .safe_rem(validators_len)?
            } else {
                state
                    .next_withdrawal_validator_index()?
                    .safe_add(spec.max_validators_per_withdrawals_sweep)?
                    .safe_rem(validators_len)?
            };
            *state.next_withdrawal_validator_index_mut()? = next_validator_index;
        }
    }

    Ok(())
}
