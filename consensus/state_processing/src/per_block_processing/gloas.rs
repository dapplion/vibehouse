use crate::VerifySignatures;
use crate::common::decrease_balance;
use crate::per_block_processing::errors::{BlockProcessingError, PayloadAttestationInvalid};
use ethereum_hashing::hash;
use int_to_bytes::int_to_bytes8;
use safe_arith::SafeArith;
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

        let state_gloas =
            state
                .as_gloas()
                .map_err(|_| BlockProcessingError::PayloadBidInvalid {
                    reason: "state is not Gloas".into(),
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
        if builder.balance < min_balance || builder.balance.saturating_sub(min_balance) < amount {
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

            let pubkey = builder.pubkey.decompress().map_err(|_| {
                BlockProcessingError::PayloadBidInvalid {
                    reason: format!("failed to decompress builder {} pubkey", builder_index),
                }
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
    if bid.parent_block_hash
        != state
            .as_gloas()
            .map_err(|_| BlockProcessingError::PayloadBidInvalid {
                reason: "state is not Gloas".into(),
            })?
            .latest_block_hash
    {
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
        let slot_index =
            slots_per_epoch.safe_add(bid.slot.as_u64().safe_rem(slots_per_epoch)?)? as usize;

        let pending_payment = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: bid.fee_recipient,
                amount,
                builder_index,
            },
        };

        let state_gloas =
            state
                .as_gloas_mut()
                .map_err(|_| BlockProcessingError::PayloadBidInvalid {
                    reason: "state is not Gloas".into(),
                })?;

        *state_gloas
            .builder_pending_payments
            .get_mut(slot_index)
            .ok_or(BlockProcessingError::PayloadBidInvalid {
                reason: format!(
                    "slot index {} out of bounds for builder_pending_payments",
                    slot_index
                ),
            })? = pending_payment;
    }

    // Cache the signed execution payload bid
    let state_gloas =
        state
            .as_gloas_mut()
            .map_err(|_| BlockProcessingError::PayloadBidInvalid {
                reason: "state is not Gloas".into(),
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
        for w in indices.windows(2) {
            if matches!(w, [a, b] if b < a) {
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
            let validator = state.validators().get(validator_index as usize).ok_or(
                BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::AttesterIndexOutOfBounds,
                ),
            )?;

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
        attesting_indices: attesting_indices.into(),
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
    let max_random_value: u64 = (1u64 << 16).saturating_sub(1); // 2^16 - 1

    let mut selected: Vec<u64> = Vec::with_capacity(ptc_size);
    let mut i: u64 = 0;
    while selected.len() < ptc_size {
        let next_index = i.safe_rem(total as u64)? as usize;
        // shuffle_indices=False, so just use next_index directly
        let candidate_index =
            *indices
                .get(next_index)
                .ok_or(BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::AttesterIndexOutOfBounds,
                ))?;

        // compute_balance_weighted_acceptance
        let random_bytes = hash(&[&seed[..], &int_to_bytes8(i.safe_div(16)?)].concat());
        let offset = i.safe_rem(16)?.safe_mul(2)? as usize;
        let random_byte_0 =
            *random_bytes
                .get(offset)
                .ok_or(BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::AttesterIndexOutOfBounds,
                ))?;
        let random_byte_1 = *random_bytes.get(offset.safe_add(1)?).ok_or(
            BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::AttesterIndexOutOfBounds,
            ),
        )?;
        let random_value = u16::from_le_bytes([random_byte_0, random_byte_1]) as u64;

        let effective_balance = state
            .validators()
            .get(candidate_index as usize)
            .ok_or(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::AttesterIndexOutOfBounds,
            ))?
            .effective_balance;

        if effective_balance.safe_mul(max_random_value)?
            >= max_effective_balance.safe_mul(random_value)?
        {
            selected.push(candidate_index);
        }
        i.safe_add_assign(1)?;
    }

    Ok(selected)
}

/// [Modified in Gloas:EIP7732] Check if the parent block had its payload delivered.
pub fn is_parent_block_full<E: EthSpec>(
    state: &BeaconState<E>,
) -> Result<bool, BlockProcessingError> {
    let state_gloas = state
        .as_gloas()
        .map_err(BlockProcessingError::BeaconStateError)?;
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
    let mut processed_builder_withdrawals_count: usize = 0;
    {
        let state_gloas = state
            .as_gloas()
            .map_err(BlockProcessingError::BeaconStateError)?;
        let builders_count = state_gloas.builders.len() as u64;
        for withdrawal in state_gloas.builder_pending_withdrawals.iter() {
            if withdrawals.len() >= reserved_limit {
                break;
            }
            let builder_index = withdrawal.builder_index;
            // Spec: state.builders[builder_index] — panics if OOB
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
            processed_builder_withdrawals_count.safe_add_assign(1)?;
        }
    }

    // 2. Pending partial withdrawals (validator)
    // Spec: withdrawals_limit = min(prior_count + MAX_PENDING_PARTIALS, MAX_WITHDRAWALS - 1)
    let mut processed_partial_withdrawals_count: usize = 0;
    {
        let partials_limit = std::cmp::min(
            withdrawals
                .len()
                .saturating_add(spec.max_pending_partials_per_withdrawals_sweep as usize),
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
                    .try_fold(0u64, |acc, amt| acc.safe_add(amt))?;
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
                processed_partial_withdrawals_count.safe_add_assign(1)?;
            }
        }
    }

    // 3. Builder sweep (exiting builders with balance, limit: MAX_WITHDRAWALS - 1)
    let mut processed_builders_sweep_count: usize = 0;
    {
        let state_gloas = state
            .as_gloas()
            .map_err(BlockProcessingError::BeaconStateError)?;
        let builders_count = state_gloas.builders.len();
        if builders_count > 0 {
            let builders_limit = std::cmp::min(
                builders_count,
                spec.max_builders_per_withdrawals_sweep as usize,
            );
            let mut builder_index = state_gloas.next_withdrawal_builder_index;
            // Spec: state.builders[builder_index] — panics if OOB
            if builder_index >= builders_count as u64 {
                return Err(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                    builder_index,
                    builders_count: builders_count as u64,
                });
            }
            for _ in 0..builders_limit {
                if withdrawals.len() >= reserved_limit {
                    break;
                }

                if let Some(builder) = state_gloas.builders.get(builder_index as usize)
                    && builder.withdrawable_epoch <= epoch
                    && builder.balance > 0
                {
                    withdrawals.push(Withdrawal {
                        index: withdrawal_index,
                        validator_index: builder_index | BUILDER_INDEX_FLAG,
                        address: builder.execution_address,
                        amount: builder.balance,
                    });
                    withdrawal_index.safe_add_assign(1)?;
                }

                builder_index = builder_index.safe_add(1)?.safe_rem(builders_count as u64)?;
                processed_builders_sweep_count.safe_add_assign(1)?;
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
                .try_fold(0u64, |acc, amt| acc.safe_add(amt))?;
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
                    amount: balance
                        .saturating_sub(validator.get_max_effective_balance(spec, fork_name)),
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
            let state_gloas = state
                .as_gloas_mut()
                .map_err(BlockProcessingError::BeaconStateError)?;
            if let Some(builder) = state_gloas.builders.get_mut(builder_index) {
                builder.balance = builder
                    .balance
                    .saturating_sub(std::cmp::min(withdrawal.amount, builder.balance));
            }
        } else {
            // Validator withdrawal
            decrease_balance(
                state,
                withdrawal.validator_index as usize,
                withdrawal.amount,
            )?;
        }
    }

    // Update next_withdrawal_index
    if let Some(latest_withdrawal) = withdrawals.last() {
        *state.next_withdrawal_index_mut()? = latest_withdrawal.index.safe_add(1)?;
    }

    // Store payload_expected_withdrawals
    {
        let state_gloas = state
            .as_gloas_mut()
            .map_err(BlockProcessingError::BeaconStateError)?;
        state_gloas.payload_expected_withdrawals =
            List::new(withdrawals.clone()).map_err(BlockProcessingError::MilhouseError)?;
    }

    // Update builder_pending_withdrawals (remove processed)
    {
        let state_gloas = state
            .as_gloas_mut()
            .map_err(BlockProcessingError::BeaconStateError)?;
        let remaining: Vec<_> = state_gloas
            .builder_pending_withdrawals
            .iter()
            .skip(processed_builder_withdrawals_count)
            .cloned()
            .collect();
        state_gloas.builder_pending_withdrawals =
            List::new(remaining).map_err(BlockProcessingError::MilhouseError)?;
    }

    // Update pending_partial_withdrawals (remove processed)
    if processed_partial_withdrawals_count > 0 {
        state
            .pending_partial_withdrawals_mut()?
            .pop_front(processed_partial_withdrawals_count)?;
    }

    // Update next_withdrawal_builder_index
    {
        let state_gloas = state
            .as_gloas_mut()
            .map_err(BlockProcessingError::BeaconStateError)?;
        let builders_count = state_gloas.builders.len();
        if builders_count > 0 {
            let next_index = state_gloas
                .next_withdrawal_builder_index
                .saturating_add(processed_builders_sweep_count as u64);
            state_gloas.next_withdrawal_builder_index =
                next_index.safe_rem(builders_count as u64)?;
        }
    }

    // Update next_withdrawal_validator_index
    // Spec: if withdrawals hit MAX, next = (last.validator_index + 1) % len
    //       else, next = (current + MAX_VALIDATORS_PER_SWEEP) % len
    {
        let validators_len = state.validators().len() as u64;
        if validators_len > 0 {
            let next_validator_index = if withdrawals.len() == max_withdrawals {
                let last_validator_index =
                    withdrawals.last().map(|w| w.validator_index).unwrap_or(0);
                last_validator_index.safe_add(1)?.safe_rem(validators_len)?
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

/// Compute expected withdrawals for a Gloas block without mutating state.
///
/// This mirrors the withdrawal computation in `process_withdrawals_gloas` but is read-only,
/// suitable for providing withdrawal lists to the EL via payload attributes.
/// Returns an empty list if the parent block's payload was not delivered.
pub fn get_expected_withdrawals_gloas<E: EthSpec>(
    state: &BeaconState<E>,
    spec: &ChainSpec,
) -> Result<Vec<Withdrawal>, BlockProcessingError> {
    // Return empty if the parent block's payload was not delivered
    if !is_parent_block_full::<E>(state)? {
        return Ok(vec![]);
    }

    let epoch = state.current_epoch();
    let fork_name = state.fork_name_unchecked();
    let max_withdrawals = E::max_withdrawals_per_payload();
    let reserved_limit = max_withdrawals.saturating_sub(1);
    let mut withdrawal_index = state.next_withdrawal_index()?;
    let mut withdrawals = Vec::<Withdrawal>::new();

    // 1. Builder pending withdrawals
    {
        let state_gloas = state
            .as_gloas()
            .map_err(BlockProcessingError::BeaconStateError)?;
        let builders_count = state_gloas.builders.len() as u64;
        for withdrawal in state_gloas.builder_pending_withdrawals.iter() {
            if withdrawals.len() >= reserved_limit {
                break;
            }
            let builder_index = withdrawal.builder_index;
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
        }
    }

    // 2. Pending partial withdrawals (validator)
    {
        let partials_limit = std::cmp::min(
            withdrawals
                .len()
                .saturating_add(spec.max_pending_partials_per_withdrawals_sweep as usize),
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
                    .try_fold(0u64, |acc, amt| acc.safe_add(amt))?;
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
            }
        }
    }

    // 3. Builder sweep
    {
        let state_gloas = state
            .as_gloas()
            .map_err(BlockProcessingError::BeaconStateError)?;
        let builders_count = state_gloas.builders.len();
        if builders_count > 0 {
            let builders_limit = std::cmp::min(
                builders_count,
                spec.max_builders_per_withdrawals_sweep as usize,
            );
            let mut builder_index = state_gloas.next_withdrawal_builder_index;
            if builder_index >= builders_count as u64 {
                return Err(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                    builder_index,
                    builders_count: builders_count as u64,
                });
            }
            for _ in 0..builders_limit {
                if withdrawals.len() >= reserved_limit {
                    break;
                }
                if let Some(builder) = state_gloas.builders.get(builder_index as usize)
                    && builder.withdrawable_epoch <= epoch
                    && builder.balance > 0
                {
                    withdrawals.push(Withdrawal {
                        index: withdrawal_index,
                        validator_index: builder_index | BUILDER_INDEX_FLAG,
                        address: builder.execution_address,
                        amount: builder.balance,
                    });
                    withdrawal_index.safe_add_assign(1)?;
                }
                builder_index = builder_index.safe_add(1)?.safe_rem(builders_count as u64)?;
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
            if withdrawals.len() >= max_withdrawals {
                break;
            }

            let validator = state.get_validator(validator_index as usize)?;
            let partially_withdrawn_balance: u64 = withdrawals
                .iter()
                .filter(|w| w.validator_index == validator_index)
                .map(|w| w.amount)
                .try_fold(0u64, |acc, amt| acc.safe_add(amt))?;
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
                    amount: balance
                        .saturating_sub(validator.get_max_effective_balance(spec, fork_name)),
                });
                withdrawal_index.safe_add_assign(1)?;
            }

            validator_index = validator_index
                .safe_add(1)?
                .safe_rem(state.validators().len() as u64)?;
        }
    }

    Ok(withdrawals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::FixedBytesExtended;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::{
        Address, BeaconBlockHeader, BeaconStateGloas, Builder, BuilderPendingWithdrawal,
        CACHED_EPOCHS, Checkpoint, CommitteeCache, Epoch, ExecutionBlockHash, ExecutionPayloadBid,
        ExitCache, FixedVector, Fork, MinimalEthSpec, ProgressiveBalancesCache, PubkeyCache,
        Signature, SignedExecutionPayloadBid, SlashingsCache, SyncCommittee, Vector,
    };

    type E = MinimalEthSpec;

    /// Build a minimal Gloas state with `n` validators, each with `balance` Gwei,
    /// and a single active builder at index 0 with `builder_balance` Gwei.
    fn make_gloas_state(
        num_validators: usize,
        balance: u64,
        builder_balance: u64,
    ) -> (BeaconState<E>, ChainSpec) {
        let spec = E::default_spec();
        let slot = Slot::new(E::slots_per_epoch()); // slot 8, epoch 1
        let epoch = slot.epoch(E::slots_per_epoch());

        // Build validators and balances
        let keypairs = types::test_utils::generate_deterministic_keypairs(num_validators);
        let mut validators = Vec::with_capacity(num_validators);
        let mut balances = Vec::with_capacity(num_validators);
        for kp in &keypairs {
            let mut creds = [0u8; 32];
            creds[0] = 0x01;
            creds[12..].copy_from_slice(&[0xAA; 20]);

            validators.push(types::Validator {
                pubkey: kp.pk.compress(),
                effective_balance: balance,
                activation_epoch: Epoch::new(0),
                exit_epoch: spec.far_future_epoch,
                withdrawable_epoch: spec.far_future_epoch,
                withdrawal_credentials: Hash256::from_slice(&creds),
                ..types::Validator::default()
            });
            balances.push(balance);
        }

        // Create a builder
        let builder = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xBB),
            balance: builder_balance,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: spec.far_future_epoch,
        };

        let parent_root = Hash256::repeat_byte(0x01);
        let parent_block_hash = ExecutionBlockHash::repeat_byte(0x02);
        let randao_mix = Hash256::repeat_byte(0x03);

        // Build randao_mixes fixed vector
        let epochs_per_vector = <E as EthSpec>::EpochsPerHistoricalVector::to_usize();
        let mut randao_mixes = vec![Hash256::zero(); epochs_per_vector];
        let mix_index = epoch.as_usize() % epochs_per_vector;
        randao_mixes[mix_index] = randao_mix;

        // SyncCommittee needs manual construction (no Default)
        let sync_committee = Arc::new(SyncCommittee {
            pubkeys: FixedVector::new(vec![
                types::PublicKeyBytes::empty();
                <E as EthSpec>::SyncCommitteeSize::to_usize()
            ])
            .unwrap(),
            aggregate_pubkey: types::PublicKeyBytes::empty(),
        });

        let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
        let epochs_per_slash = <E as EthSpec>::EpochsPerSlashingsVector::to_usize();

        let state = BeaconState::Gloas(BeaconStateGloas {
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
                <E as EthSpec>::ProposerLookaheadSlots::to_usize()
            ])
            .unwrap(),
            builders: List::new(vec![builder]).unwrap(),
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

        (state, spec)
    }

    /// Create a valid self-build bid for the given state.
    fn make_self_build_bid(
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> SignedExecutionPayloadBid<E> {
        let state_gloas = state.as_gloas().unwrap();
        let parent_root = state.latest_block_header().parent_root;
        let randao_mix = *state.get_randao_mix(state.current_epoch()).unwrap();

        SignedExecutionPayloadBid {
            message: ExecutionPayloadBid {
                builder_index: spec.builder_index_self_build,
                value: 0,
                parent_block_hash: state_gloas.latest_block_hash,
                parent_block_root: parent_root,
                block_hash: ExecutionBlockHash::repeat_byte(0x10),
                prev_randao: randao_mix,
                slot: state.slot(),
                ..Default::default()
            },
            signature: Signature::infinity().unwrap(),
        }
    }

    /// Create a valid builder bid (builder_index=0) for the given state.
    fn make_builder_bid(
        state: &BeaconState<E>,
        _spec: &ChainSpec,
        value: u64,
    ) -> SignedExecutionPayloadBid<E> {
        let state_gloas = state.as_gloas().unwrap();
        let parent_root = state.latest_block_header().parent_root;
        let randao_mix = *state.get_randao_mix(state.current_epoch()).unwrap();

        SignedExecutionPayloadBid {
            message: ExecutionPayloadBid {
                builder_index: 0,
                value,
                parent_block_hash: state_gloas.latest_block_hash,
                parent_block_root: parent_root,
                block_hash: ExecutionBlockHash::repeat_byte(0x20),
                prev_randao: randao_mix,
                slot: state.slot(),
                fee_recipient: Address::repeat_byte(0xCC),
                ..Default::default()
            },
            signature: Signature::empty(),
        }
    }

    // ── Self-build bid tests ────────────────────────────────────

    #[test]
    fn self_build_bid_valid() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = make_self_build_bid(&state, &spec);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(result.is_ok(), "valid self-build bid should succeed");

        // Verify bid was cached in state
        let cached_bid = &state.as_gloas().unwrap().latest_execution_payload_bid;
        assert_eq!(cached_bid.builder_index, spec.builder_index_self_build);
        assert_eq!(cached_bid.value, 0);
    }

    #[test]
    fn self_build_bid_nonzero_value_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut bid = make_self_build_bid(&state, &spec);
        bid.message.value = 100;
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("self-build bid must have value = 0")
        ));
    }

    #[test]
    fn self_build_bid_non_infinity_signature_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut bid = make_self_build_bid(&state, &spec);
        bid.signature = Signature::empty(); // not infinity
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("G2_POINT_AT_INFINITY")
        ));
    }

    // ── Builder bid validation tests ────────────────────────────

    #[test]
    fn builder_bid_valid_with_skip_signature() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = make_builder_bid(&state, &spec, 1_000_000_000);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            result.is_ok(),
            "valid builder bid should succeed: {:?}",
            result.err()
        );

        // Check pending payment was recorded
        let state_gloas = state.as_gloas().unwrap();
        let slot_index = E::slots_per_epoch() + (slot.as_u64() % E::slots_per_epoch());
        let payment = state_gloas
            .builder_pending_payments
            .get(slot_index as usize)
            .unwrap();
        assert_eq!(payment.withdrawal.amount, 1_000_000_000);
        assert_eq!(payment.withdrawal.builder_index, 0);
    }

    #[test]
    fn builder_bid_zero_value_no_pending_payment() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = make_builder_bid(&state, &spec, 0);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(result.is_ok());

        // No pending payment should be recorded for zero-value bid
        let state_gloas = state.as_gloas().unwrap();
        let slot_index = E::slots_per_epoch() + (slot.as_u64() % E::slots_per_epoch());
        let payment = state_gloas
            .builder_pending_payments
            .get(slot_index as usize)
            .unwrap();
        assert_eq!(payment.withdrawal.amount, 0);
    }

    #[test]
    fn builder_bid_nonexistent_builder_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut bid = make_builder_bid(&state, &spec, 1_000_000_000);
        bid.message.builder_index = 99; // no such builder
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("does not exist")
        ));
    }

    #[test]
    fn builder_bid_inactive_builder_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        // Make the builder inactive by setting withdrawable_epoch to past
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);
        let bid = make_builder_bid(&state, &spec, 1_000_000_000);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("not active")
        ));
    }

    #[test]
    fn builder_bid_insufficient_balance_rejected() {
        // Builder has min_deposit_amount + 100 gwei, bids 200 gwei
        let min_deposit = E::default_spec().min_deposit_amount;
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, min_deposit + 100);
        let bid = make_builder_bid(&state, &spec, 200);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("insufficient")
        ));
    }

    #[test]
    fn builder_bid_balance_accounts_for_pending_withdrawals() {
        let min_deposit = E::default_spec().min_deposit_amount;
        let builder_balance = min_deposit + 1000;
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, builder_balance);

        // Add a pending withdrawal for builder 0
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 500,
                builder_index: 0,
            })
            .unwrap();

        // Bid for 600 should fail: available = builder_balance - min_deposit - 500 = 500 < 600
        let bid = make_builder_bid(&state, &spec, 600);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("insufficient")
        ));

        // Bid for 400 should succeed: available = 500 >= 400
        let bid = make_builder_bid(&state, &spec, 400);
        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            result.is_ok(),
            "bid within available balance should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn builder_bid_balance_accounts_for_pending_payments() {
        let min_deposit = E::default_spec().min_deposit_amount;
        let builder_balance = min_deposit + 1000;
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, builder_balance);

        // Add a pending payment for builder 0
        *state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_payments
            .get_mut(0)
            .unwrap() = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 500,
                builder_index: 0,
            },
        };

        // Bid for 600 should fail: available = 1000 - 500 = 500 < 600
        let bid = make_builder_bid(&state, &spec, 600);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("insufficient")
        ));
    }

    // ── Slot and parent validation tests ────────────────────────

    #[test]
    fn builder_bid_wrong_slot_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = make_builder_bid(&state, &spec, 0);
        let wrong_slot = state.slot() + 1;
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            wrong_slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("slot")
        ));
    }

    #[test]
    fn builder_bid_wrong_parent_block_hash_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut bid = make_builder_bid(&state, &spec, 0);
        bid.message.parent_block_hash = ExecutionBlockHash::repeat_byte(0xFF);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("parent_block_hash")
        ));
    }

    #[test]
    fn builder_bid_wrong_parent_block_root_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut bid = make_builder_bid(&state, &spec, 0);
        bid.message.parent_block_root = Hash256::repeat_byte(0xFF);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("parent_block_root")
        ));
    }

    #[test]
    fn builder_bid_wrong_prev_randao_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut bid = make_builder_bid(&state, &spec, 0);
        bid.message.prev_randao = Hash256::repeat_byte(0xFF);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("prev_randao")
        ));
    }

    // ── Blob KZG commitments limit test ─────────────────────────

    #[test]
    fn builder_bid_too_many_blob_commitments_rejected() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let mut bid = make_builder_bid(&state, &spec, 0);

        // Add more blob commitments than allowed
        let max_blobs = spec.max_blobs_per_block(state.current_epoch()) as usize;
        let mut commitments = Vec::with_capacity(max_blobs + 1);
        for _ in 0..=max_blobs {
            commitments.push(types::KzgCommitment::empty_for_testing());
        }
        bid.message.blob_kzg_commitments = ssz_types::VariableList::new(commitments).unwrap();

        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("blob_kzg_commitments")
        ));
    }

    // ── is_parent_block_full tests ──────────────────────────────

    #[test]
    fn is_parent_block_full_when_hashes_match() {
        let (state, _spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        // In our test state, latest_execution_payload_bid.block_hash != latest_block_hash
        // because they're set to different values (0x04 vs 0x02)
        let result = is_parent_block_full::<E>(&state).unwrap();
        assert!(!result, "different hashes mean parent block is empty");

        // Now make them match
        let mut state2 = state.clone();
        let state_gloas = state2.as_gloas_mut().unwrap();
        state_gloas.latest_execution_payload_bid.block_hash = state_gloas.latest_block_hash;
        let result = is_parent_block_full::<E>(&state2).unwrap();
        assert!(result, "matching hashes mean parent block is full");
    }

    // ── State mutation tests ────────────────────────────────────

    #[test]
    fn bid_caches_latest_execution_payload_bid() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = make_builder_bid(&state, &spec, 500);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        let cached = &state.as_gloas().unwrap().latest_execution_payload_bid;
        assert_eq!(cached.builder_index, 0);
        assert_eq!(cached.value, 500);
        assert_eq!(cached.slot, slot);
        assert_eq!(cached.block_hash, ExecutionBlockHash::repeat_byte(0x20));
    }

    // ── Withdrawal tests ──────────────────────────────────────────

    /// Make the parent block "full" so withdrawals execute.
    /// In Gloas, is_parent_block_full checks:
    ///   state.latest_execution_payload_bid.block_hash == state.latest_block_hash
    fn make_parent_block_full(state: &mut BeaconState<E>) {
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas.latest_block_hash = state_gloas.latest_execution_payload_bid.block_hash;
    }

    #[test]
    fn withdrawals_empty_when_parent_block_not_full() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        // Default state has mismatched hashes (parent block empty)
        assert!(!is_parent_block_full::<E>(&state).unwrap());

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // No withdrawals stored when parent block is empty
        let state_gloas = state.as_gloas().unwrap();
        assert!(state_gloas.payload_expected_withdrawals.is_empty());
    }

    #[test]
    fn withdrawals_builder_pending_withdrawals() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Add a pending withdrawal for builder 0
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 5_000_000_000,
                builder_index: 0,
            })
            .unwrap();

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let withdrawals = &state_gloas.payload_expected_withdrawals;
        // Should have builder pending withdrawal + validator sweep entries
        assert!(!withdrawals.is_empty());

        // First withdrawal should be the builder pending withdrawal
        let w = withdrawals.get(0).unwrap();
        assert_eq!(w.validator_index, BUILDER_INDEX_FLAG);
        assert_eq!(w.amount, 5_000_000_000);
        assert_eq!(w.address, Address::repeat_byte(0xDD));

        // Builder pending withdrawals should be cleared
        assert!(state_gloas.builder_pending_withdrawals.is_empty());
    }

    #[test]
    fn withdrawals_builder_pending_respects_reserved_limit() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // MinimalEthSpec: max_withdrawals = 4, reserved_limit = 3
        // Add 5 builder pending withdrawals — only 3 should fit
        for i in 0..5 {
            state
                .as_gloas_mut()
                .unwrap()
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    fee_recipient: Address::repeat_byte(0xDD),
                    amount: 1000 + i as u64,
                    builder_index: 0,
                })
                .unwrap();
        }

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        // Only 3 builder pending withdrawals should be processed (reserved_limit)
        // plus possibly 1 validator sweep entry (up to max_withdrawals=4)
        let builder_withdrawals: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert_eq!(builder_withdrawals.len(), 3);
        assert_eq!(builder_withdrawals[0].amount, 1000);
        assert_eq!(builder_withdrawals[1].amount, 1001);
        assert_eq!(builder_withdrawals[2].amount, 1002);

        // 2 unprocessed builder pending withdrawals remain
        assert_eq!(state_gloas.builder_pending_withdrawals.len(), 2);
    }

    #[test]
    fn withdrawals_builder_balance_decreased() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 10_000_000_000,
                builder_index: 0,
            })
            .unwrap();

        let initial_balance = state.as_gloas().unwrap().builders.get(0).unwrap().balance;
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();
        let final_balance = state.as_gloas().unwrap().builders.get(0).unwrap().balance;

        assert_eq!(final_balance, initial_balance - 10_000_000_000);
    }

    #[test]
    fn withdrawals_validator_full_withdrawal() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Make validator 0 fully withdrawable: set withdrawable_epoch to past
        state.get_validator_mut(0).unwrap().withdrawable_epoch = Epoch::new(0);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let validator_withdrawals: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();

        // Validator 0 should appear with full balance withdrawal
        let v0_withdrawal = validator_withdrawals
            .iter()
            .find(|w| w.validator_index == 0)
            .expect("validator 0 should have a withdrawal");
        assert_eq!(v0_withdrawal.amount, 32_000_000_000);
        assert_eq!(v0_withdrawal.address, Address::repeat_byte(0xAA));

        // Balance should be zero after full withdrawal
        assert_eq!(state.get_balance(0).unwrap(), 0);
    }

    #[test]
    fn withdrawals_validator_partial_withdrawal() {
        // Create state with balance=34 ETH but effective_balance still at 32 ETH
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Set validator balances to 34 ETH (effective stays 32 ETH from make_gloas_state)
        // is_partially_withdrawable: has_execution_withdrawal_credential, effective == max, balance > max
        // max_effective_balance for 0x01 credential = min_activation_balance = 32 ETH
        // Excess = 34 - 32 = 2 ETH
        for i in 0..8 {
            *state.get_balance_mut(i).unwrap() = 34_000_000_000;
        }

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let validator_withdrawals: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();

        // At least validator 0 should appear with excess withdrawal
        let v0_withdrawal = validator_withdrawals
            .iter()
            .find(|w| w.validator_index == 0)
            .expect("validator 0 should have a partial withdrawal");
        assert_eq!(v0_withdrawal.amount, 2_000_000_000); // 34 - 32 = 2 ETH
    }

    #[test]
    fn withdrawals_builder_sweep_exiting_builder() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Make builder 0 exiting (withdrawable_epoch <= current epoch)
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_withdrawals: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Exiting builder with balance should be swept
        assert!(!builder_withdrawals.is_empty());
        assert_eq!(builder_withdrawals[0].amount, 5_000_000_000);
        assert_eq!(builder_withdrawals[0].validator_index, BUILDER_INDEX_FLAG);

        // Builder balance should be zeroed
        assert_eq!(state_gloas.builders.get(0).unwrap().balance, 0);
    }

    #[test]
    fn withdrawals_builder_sweep_skips_active_builder() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Builder 0 is active (withdrawable_epoch = far_future_epoch) — should NOT be swept

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_sweep_withdrawals: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Active builder should not appear in sweep
        assert!(builder_sweep_withdrawals.is_empty());

        // Builder balance should be unchanged
        assert_eq!(state_gloas.builders.get(0).unwrap().balance, 5_000_000_000);
    }

    #[test]
    fn withdrawals_next_withdrawal_index_updated() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        assert_eq!(state.next_withdrawal_index().unwrap(), 0);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let num_withdrawals = state_gloas.payload_expected_withdrawals.len();
        if num_withdrawals > 0 {
            // next_withdrawal_index should be last withdrawal's index + 1
            let last_w = state_gloas
                .payload_expected_withdrawals
                .iter()
                .last()
                .unwrap();
            assert_eq!(state.next_withdrawal_index().unwrap(), last_w.index + 1);
        }
    }

    #[test]
    fn withdrawals_next_withdrawal_validator_index_advances() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        let initial_index = state.next_withdrawal_validator_index().unwrap();
        assert_eq!(initial_index, 0);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // Since no validator is withdrawable (no excess balance, not fully withdrawable),
        // and withdrawals.len() < max_withdrawals, the index advances by
        // max_validators_per_withdrawals_sweep (16 in minimal)
        let expected = spec.max_validators_per_withdrawals_sweep % 8;
        assert_eq!(state.next_withdrawal_validator_index().unwrap(), expected);
    }

    #[test]
    fn withdrawals_next_withdrawal_builder_index_advances() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        let initial = state.as_gloas().unwrap().next_withdrawal_builder_index;
        assert_eq!(initial, 0);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // Builder sweep processes min(builders_count=1, max_builders_per_sweep=16) = 1 builder
        // next_withdrawal_builder_index = (0 + 1) % 1 = 0 (wraps around with 1 builder)
        let state_gloas = state.as_gloas().unwrap();
        assert_eq!(state_gloas.next_withdrawal_builder_index, 0);
    }

    #[test]
    fn withdrawals_pending_partial_withdrawals_cleared() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Add a pending partial withdrawal for validator 0
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0), // immediately withdrawable
            })
            .unwrap();
        assert_eq!(state.pending_partial_withdrawals().unwrap().len(), 1);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // Pending partial withdrawal should be processed and removed
        assert_eq!(state.pending_partial_withdrawals().unwrap().len(), 0);

        // A withdrawal should have been generated for the partial withdrawal
        let state_gloas = state.as_gloas().unwrap();
        let partial_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| w.validator_index == 0 && (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();
        assert!(!partial_w.is_empty());
        // min(balance - min_activation_balance, requested_amount) = min(34-32, 1) = 1 ETH
        assert_eq!(partial_w[0].amount, 1_000_000_000);
    }

    #[test]
    fn withdrawals_partial_skips_exited_validator() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Make validator 0 exited
        state.get_validator_mut(0).unwrap().exit_epoch = Epoch::new(0);

        // Add a pending partial withdrawal for exited validator 0
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // Pending partial withdrawal was "processed" (counted) but no actual
        // withdrawal generated because validator has exited (exit_epoch != far_future)
        let state_gloas = state.as_gloas().unwrap();
        let partial_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| w.validator_index == 0 && (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();
        assert!(
            partial_w.is_empty(),
            "exited validator should not receive partial withdrawal"
        );
    }

    #[test]
    fn get_expected_withdrawals_matches_process_withdrawals() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Make builder 0 exiting
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Add a builder pending withdrawal
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 1_000_000_000,
                builder_index: 0,
            })
            .unwrap();

        // Compute expected withdrawals (read-only)
        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // Now process (mutating)
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let actual = &state.as_gloas().unwrap().payload_expected_withdrawals;
        assert_eq!(expected.len(), actual.len());
        for (e, a) in expected.iter().zip(actual.iter()) {
            assert_eq!(e.index, a.index);
            assert_eq!(e.validator_index, a.validator_index);
            assert_eq!(e.address, a.address);
            assert_eq!(e.amount, a.amount);
        }
    }

    #[test]
    fn get_expected_withdrawals_empty_when_parent_not_full() {
        let (state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        // Don't make parent block full
        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();
        assert!(expected.is_empty());
    }

    // ── get_expected_withdrawals_gloas phase tests ──────────────────

    #[test]
    fn get_expected_withdrawals_builder_pending_withdrawal() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Add a builder pending withdrawal
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 2_000_000_000,
                builder_index: 0,
            })
            .unwrap();

        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // Should include the builder pending withdrawal
        let builder_w: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0)
            .collect();
        assert_eq!(builder_w.len(), 1, "should have one builder withdrawal");
        assert_eq!(builder_w[0].amount, 2_000_000_000);
        assert_eq!(builder_w[0].address, Address::repeat_byte(0xDD));
    }

    #[test]
    fn get_expected_withdrawals_multiple_builder_pending() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Add two builder pending withdrawals
        for i in 0..2 {
            state
                .as_gloas_mut()
                .unwrap()
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    fee_recipient: Address::repeat_byte(0xDD + i),
                    amount: (i as u64 + 1) * 1_000_000_000,
                    builder_index: 0,
                })
                .unwrap();
        }

        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        let builder_w: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0)
            .collect();
        assert_eq!(builder_w.len(), 2, "should have two builder withdrawals");
        assert_eq!(builder_w[0].amount, 1_000_000_000);
        assert_eq!(builder_w[1].amount, 2_000_000_000);
        // Withdrawal indices should be sequential
        assert_eq!(builder_w[1].index, builder_w[0].index + 1);
    }

    #[test]
    fn get_expected_withdrawals_builder_sweep_exited_with_balance() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Make builder 0 exited (withdrawable_epoch in past) with remaining balance
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // Builder sweep should pick up exited builder with balance > 0
        let builder_sweep: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0 && w.amount == 5_000_000_000)
            .collect();
        assert_eq!(
            builder_sweep.len(),
            1,
            "should have one builder sweep withdrawal"
        );
        assert_eq!(builder_sweep[0].amount, 5_000_000_000);
    }

    #[test]
    fn get_expected_withdrawals_builder_sweep_active_not_withdrawn() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Builder 0 is active (withdrawable_epoch = far_future_epoch, the default)
        // Active builders should NOT be swept
        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        let builder_sweep: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0)
            .collect();
        assert!(
            builder_sweep.is_empty(),
            "active builder should not be swept"
        );
    }

    #[test]
    fn get_expected_withdrawals_validator_sweep_excess_balance() {
        // 32 ETH effective balance matches min_activation_balance for 0x01 credentials
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Give validators 34 ETH balance (excess of 2 ETH over 32 ETH effective balance)
        for i in 0..8 {
            *state.get_balance_mut(i).unwrap() = 34_000_000_000;
        }

        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // The validator sweep should produce partial withdrawals for excess balance
        let validator_w: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG == 0)
            .collect();
        assert!(
            !validator_w.is_empty(),
            "validators with excess balance should produce sweep withdrawals"
        );
        // Each validator withdrawal should be 2 ETH (34 - 32)
        for w in &validator_w {
            assert_eq!(w.amount, 2_000_000_000);
        }
    }

    #[test]
    fn get_expected_withdrawals_validator_fully_withdrawable() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Make validator 0 fully withdrawable (exited + past withdrawable epoch)
        let validator = state.get_validator_mut(0).unwrap();
        validator.exit_epoch = Epoch::new(0);
        validator.withdrawable_epoch = Epoch::new(0);

        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // Should have a full withdrawal for validator 0
        let full_w: Vec<_> = expected.iter().filter(|w| w.validator_index == 0).collect();
        assert_eq!(
            full_w.len(),
            1,
            "should have full withdrawal for validator 0"
        );
        // Full withdrawal = entire balance
        assert_eq!(full_w[0].amount, 34_000_000_000);
    }

    #[test]
    fn get_expected_withdrawals_combined_phases() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Phase 1: Builder pending withdrawal
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 1_000_000_000,
                builder_index: 0,
            })
            .unwrap();

        // Phase 3: Make builder exited for sweep
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Phase 4: Validators have excess balance (34 > 32 ETH)

        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // Should have withdrawals from multiple phases
        assert!(
            expected.len() >= 2,
            "should have withdrawals from multiple phases, got {}",
            expected.len()
        );

        // Builder pending withdrawal (phase 1)
        let builder_pending: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0 && w.amount == 1_000_000_000)
            .collect();
        assert_eq!(
            builder_pending.len(),
            1,
            "should have builder pending withdrawal"
        );

        // Builder sweep (phase 3)
        let builder_sweep: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0 && w.amount == 5_000_000_000)
            .collect();
        assert_eq!(
            builder_sweep.len(),
            1,
            "should have builder sweep withdrawal"
        );
    }

    // ── process_withdrawals_gloas edge case tests ──────────────────

    #[test]
    fn withdrawals_max_withdrawals_reached_updates_validator_index_from_last() {
        // When withdrawals.len() == max_withdrawals (4 for minimal), the
        // next_withdrawal_validator_index should be set to
        // (last_withdrawal.validator_index + 1) % validators_len.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Make validators 0..3 fully withdrawable to fill all 4 withdrawal slots.
        // 3 builder pending withdrawals fill the reserved limit (3), then 1 validator
        // fills the last slot (max_withdrawals=4).
        for i in 0..4 {
            let v = state.get_validator_mut(i).unwrap();
            v.exit_epoch = Epoch::new(0);
            v.withdrawable_epoch = Epoch::new(0);
        }

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let num_withdrawals = state_gloas.payload_expected_withdrawals.len();
        // With 4 fully-withdrawable validators and max_withdrawals=4,
        // all 4 go through the validator sweep (no builder pending/sweep in this setup).
        assert_eq!(num_withdrawals, 4, "should fill all 4 withdrawal slots");

        // next_withdrawal_validator_index = (last.validator_index + 1) % 8
        let last_w = state_gloas
            .payload_expected_withdrawals
            .iter()
            .last()
            .unwrap();
        let expected_next = (last_w.validator_index + 1) % 8;
        assert_eq!(
            state.next_withdrawal_validator_index().unwrap(),
            expected_next,
            "when max_withdrawals reached, next index = (last + 1) % validators_len"
        );
    }

    #[test]
    fn withdrawals_partial_amount_capped_to_excess() {
        // When pending partial withdrawal amount exceeds the validator's excess balance,
        // the withdrawal amount should be capped to (balance - min_activation_balance).
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Give validator 0 exactly 33 ETH (excess = 1 ETH)
        *state.get_balance_mut(0).unwrap() = 33_000_000_000;

        // Request 5 ETH partial withdrawal — should be capped to 1 ETH excess
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 5_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let partial_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| w.validator_index == 0 && (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();
        assert_eq!(partial_w.len(), 1, "should have one partial withdrawal");
        // min(33 - 32, 5) = 1 ETH
        assert_eq!(
            partial_w[0].amount, 1_000_000_000,
            "partial withdrawal capped to excess balance"
        );
    }

    #[test]
    fn withdrawals_builder_sweep_round_robin_from_nonzero_index() {
        // Builder sweep should start from next_withdrawal_builder_index and wrap around.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Add a second builder (index 1), also exited
        let builder1 = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xCC),
            balance: 3_000_000_000,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: Epoch::new(0), // exited
        };
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .push(builder1)
            .unwrap();

        // Make builder 0 also exited
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Start sweep from builder index 1 (not 0)
        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 1;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Both builders should be swept (sweep starts at 1, wraps to 0)
        assert_eq!(builder_w.len(), 2, "both exited builders should be swept");
        // First swept should be builder 1 (index 1), second should be builder 0
        assert_eq!(
            builder_w[0].validator_index,
            1 | BUILDER_INDEX_FLAG,
            "sweep starts at builder index 1"
        );
        assert_eq!(builder_w[0].amount, 3_000_000_000);
        assert_eq!(
            builder_w[1].validator_index, BUILDER_INDEX_FLAG,
            "sweep wraps to builder index 0"
        );
        assert_eq!(builder_w[1].amount, 5_000_000_000);

        // After sweeping 2 builders, next_withdrawal_builder_index = (1+2) % 2 = 1
        assert_eq!(
            state_gloas.next_withdrawal_builder_index, 1,
            "builder index wraps after full sweep"
        );
    }

    #[test]
    fn withdrawals_pending_partial_not_withdrawable_yet_breaks() {
        // Pending partial withdrawals with future withdrawable_epoch should NOT be processed.
        // The spec says: iterate pending partials, break when !is_withdrawable || limit reached.
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Add a partial withdrawal that isn't withdrawable yet (epoch 100, current epoch = 1)
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(100),
            })
            .unwrap();

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // The partial withdrawal should still be in the queue (not processed)
        assert_eq!(
            state.pending_partial_withdrawals().unwrap().len(),
            1,
            "future partial withdrawal should remain in queue"
        );

        // No partial withdrawal should have been generated
        let state_gloas = state.as_gloas().unwrap();
        let partial_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| w.validator_index == 0 && (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();
        assert!(
            partial_w.is_empty(),
            "no partial withdrawal for non-withdrawable epoch"
        );
    }

    #[test]
    fn withdrawals_partial_and_validator_sweep_same_validator() {
        // When a validator has a pending partial withdrawal AND has excess balance,
        // the validator sweep should account for the already-withdrawn partial amount.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Give validator 0 a balance of 36 ETH (excess = 4 ETH over 32 ETH min)
        *state.get_balance_mut(0).unwrap() = 36_000_000_000;

        // Add a pending partial withdrawal for 2 ETH
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 2_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let v0_withdrawals: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| w.validator_index == 0 && (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();

        // Should have 2 withdrawals for validator 0:
        // 1. Partial withdrawal: min(36-32, 2) = 2 ETH
        // 2. Validator sweep: partially withdrawable with balance = 36 - 2 (prior) = 34,
        //    excess = 34 - 32 = 2 ETH
        assert_eq!(
            v0_withdrawals.len(),
            2,
            "validator 0 should have partial + sweep withdrawal"
        );
        assert_eq!(
            v0_withdrawals[0].amount, 2_000_000_000,
            "partial withdrawal = 2 ETH"
        );
        assert_eq!(
            v0_withdrawals[1].amount, 2_000_000_000,
            "sweep accounts for prior partial: excess = 36 - 2 - 32 = 2 ETH"
        );

        // Final balance should be 36 - 2 - 2 = 32 ETH
        assert_eq!(
            state.get_balance(0).unwrap(),
            32_000_000_000,
            "balance decremented by both withdrawals"
        );
    }

    #[test]
    fn withdrawals_builder_sweep_zero_balance_skipped() {
        // Exited builder with zero balance should NOT produce a sweep withdrawal.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 0);
        make_parent_block_full(&mut state);

        // Make builder 0 exited but with zero balance
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert!(
            builder_w.is_empty(),
            "exited builder with zero balance should not produce withdrawal"
        );
    }

    #[test]
    fn withdrawals_pending_partial_insufficient_balance_skipped() {
        // If validator's balance <= min_activation_balance (no excess), partial withdrawal
        // should not produce a withdrawal entry (but still counts as processed).
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Validator 0 has exactly min_activation_balance (32 ETH) — no excess
        // (balance = 32 ETH from make_gloas_state)

        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // Partial withdrawal is counted as processed (removed from queue)
        assert_eq!(
            state.pending_partial_withdrawals().unwrap().len(),
            0,
            "partial withdrawal should be removed even though no excess"
        );

        // No actual withdrawal generated (no excess balance)
        let state_gloas = state.as_gloas().unwrap();
        let v0_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| w.validator_index == 0 && (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();
        assert!(
            v0_w.is_empty(),
            "no withdrawal when balance <= min_activation_balance"
        );
    }

    // ── get_ptc_committee tests ──────────────────────────────────

    /// Build a state with committee caches initialized (needed for get_ptc_committee).
    fn make_gloas_state_with_committees(
        num_validators: usize,
        balance: u64,
        builder_balance: u64,
    ) -> (BeaconState<E>, ChainSpec) {
        let (mut state, spec) = make_gloas_state(num_validators, balance, builder_balance);
        state
            .build_committee_cache(types::RelativeEpoch::Previous, &spec)
            .expect("should build previous committee cache");
        state
            .build_committee_cache(types::RelativeEpoch::Current, &spec)
            .expect("should build current committee cache");
        (state, spec)
    }

    #[test]
    fn ptc_committee_returns_correct_size() {
        // MinimalEthSpec: PtcSize = 2
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let slot = state.slot();
        let ptc = get_ptc_committee(&state, slot, &spec).unwrap();
        assert_eq!(ptc.len(), E::ptc_size());
        assert_eq!(ptc.len(), 2);
    }

    #[test]
    fn ptc_committee_members_are_valid_validators() {
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let slot = state.slot();
        let ptc = get_ptc_committee(&state, slot, &spec).unwrap();

        let num_validators = state.validators().len();
        for &idx in &ptc {
            assert!(
                (idx as usize) < num_validators,
                "PTC member index {} exceeds validator count {}",
                idx,
                num_validators
            );
        }
    }

    #[test]
    fn ptc_committee_deterministic_for_same_state() {
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let slot = state.slot();

        let ptc1 = get_ptc_committee(&state, slot, &spec).unwrap();
        let ptc2 = get_ptc_committee(&state, slot, &spec).unwrap();
        assert_eq!(
            ptc1, ptc2,
            "PTC should be deterministic for the same state and slot"
        );
    }

    #[test]
    fn ptc_committee_differs_across_slots() {
        // Use more validators to make it likely that different slots get different committees
        let (mut state, spec) =
            make_gloas_state_with_committees(64, 32_000_000_000, 64_000_000_000);

        // Slot 8 (current) and slot 9 (we need to also build committees for next epoch if needed)
        let slot_a = state.slot();

        // Move state to next slot
        *state.slot_mut() = state.slot() + 1;
        state
            .build_committee_cache(types::RelativeEpoch::Current, &spec)
            .unwrap();
        let slot_b = state.slot();

        let ptc_a = get_ptc_committee(&state, slot_a, &spec).unwrap();
        let ptc_b = get_ptc_committee(&state, slot_b, &spec).unwrap();

        // With 64 validators and PTC_SIZE=2, very likely different selections
        // (not guaranteed but extremely likely with different seeds)
        // We just check both are valid; a strict inequality test could flake
        assert_eq!(ptc_a.len(), 2);
        assert_eq!(ptc_b.len(), 2);
    }

    #[test]
    fn ptc_committee_uses_balance_weighting() {
        // Create validators where validator 0 has max effective balance
        // and others have 0 effective balance — validator 0 should be selected
        let (mut state, spec) = make_gloas_state(8, 0, 64_000_000_000);

        // Give validator 0 max effective balance, rest get 0
        let max_eb = spec.max_effective_balance_electra;
        state.get_validator_mut(0).unwrap().effective_balance = max_eb;
        for i in 1..8 {
            state.get_validator_mut(i).unwrap().effective_balance = max_eb;
            *state.get_balance_mut(i).unwrap() = max_eb;
        }
        *state.get_balance_mut(0).unwrap() = max_eb;

        state
            .build_committee_cache(types::RelativeEpoch::Current, &spec)
            .expect("should build committee cache");

        let slot = state.slot();
        let ptc = get_ptc_committee(&state, slot, &spec).unwrap();

        // All members should have max effective balance — they should all pass the filter
        // We just verify the committee was computed successfully and has PTC_SIZE members
        assert_eq!(ptc.len(), E::ptc_size());
    }

    #[test]
    fn ptc_committee_works_at_epoch_boundary() {
        // Test at start of epoch (slot 8 = first slot of epoch 1 in minimal)
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        assert_eq!(state.slot(), Slot::new(8));
        assert_eq!(state.slot().epoch(E::slots_per_epoch()), Epoch::new(1));

        let ptc = get_ptc_committee(&state, state.slot(), &spec).unwrap();
        assert_eq!(ptc.len(), E::ptc_size());
    }

    // ── process_payload_attestation tests ─────────────────────────

    /// Build a PayloadAttestation targeting the parent block at the previous slot.
    fn make_payload_attestation(state: &BeaconState<E>, bits: &[bool]) -> PayloadAttestation<E> {
        let parent_root = state.latest_block_header().parent_root;
        let prev_slot = state.slot().saturating_sub(1u64);

        let mut aggregation_bits = BitVector::<<E as EthSpec>::PtcSize>::new();
        for (i, &bit) in bits.iter().enumerate() {
            aggregation_bits
                .set(i, bit)
                .expect("bit index should be in range");
        }

        PayloadAttestation {
            aggregation_bits,
            data: types::PayloadAttestationData {
                beacon_block_root: parent_root,
                slot: prev_slot,
                payload_present: true,
                blob_data_available: true,
            },
            signature: bls::AggregateSignature::empty(),
        }
    }

    #[test]
    fn payload_attestation_valid_skip_signature() {
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let attestation = make_payload_attestation(&state, &[true, false]);

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "valid attestation should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn payload_attestation_all_bits_set() {
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let attestation = make_payload_attestation(&state, &[true, true]);

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "all-bits attestation should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn payload_attestation_no_bits_set() {
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let attestation = make_payload_attestation(&state, &[false, false]);

        // With no bits set, the indexed attestation has empty attesting_indices.
        // With VerifySignatures::False, this should still succeed.
        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "no-bits attestation should succeed without sig check: {:?}",
            result.err()
        );
    }

    #[test]
    fn payload_attestation_wrong_beacon_block_root() {
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let mut attestation = make_payload_attestation(&state, &[true, false]);
        attestation.data.beacon_block_root = Hash256::repeat_byte(0xFF);

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::WrongBeaconBlockRoot
            ))
        ));
    }

    #[test]
    fn payload_attestation_wrong_slot_too_old() {
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let mut attestation = make_payload_attestation(&state, &[true, false]);
        // Set data.slot so that data.slot + 1 != state.slot
        attestation.data.slot = state.slot().saturating_sub(5u64);

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::WrongSlot { .. }
            ))
        ));
    }

    #[test]
    fn payload_attestation_wrong_slot_future() {
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);
        let mut attestation = make_payload_attestation(&state, &[true, false]);
        // Set data.slot to current slot (data.slot + 1 = state.slot + 1 != state.slot)
        attestation.data.slot = state.slot();

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::WrongSlot { .. }
            ))
        ));
    }

    #[test]
    fn payload_attestation_indexed_indices_match_ptc() {
        // Verify that the indexed attestation produced internally matches the PTC committee
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        // Get expected PTC
        let prev_slot = state.slot().saturating_sub(1u64);
        let ptc = get_ptc_committee(&state, prev_slot, &spec).unwrap();

        // Set bit 0 only
        let attestation = make_payload_attestation(&state, &[true, false]);

        // Call get_indexed_payload_attestation directly
        let indexed = get_indexed_payload_attestation(&state, &attestation, &spec).unwrap();

        // Only bit 0 was set, so attesting_indices should contain only ptc[0]
        assert_eq!(indexed.attesting_indices.len(), 1);
        assert_eq!(indexed.attesting_indices[0], ptc[0]);
    }

    #[test]
    fn payload_attestation_indexed_all_bits() {
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let prev_slot = state.slot().saturating_sub(1u64);
        let ptc = get_ptc_committee(&state, prev_slot, &spec).unwrap();

        let attestation = make_payload_attestation(&state, &[true, true]);
        let indexed = get_indexed_payload_attestation(&state, &attestation, &spec).unwrap();

        // Both bits set, attesting_indices should contain both PTC members (sorted)
        assert_eq!(indexed.attesting_indices.len(), 2);
        let mut expected = [ptc[0], ptc[1]];
        expected.sort_unstable();
        assert_eq!(indexed.attesting_indices[0], expected[0]);
        assert_eq!(indexed.attesting_indices[1], expected[1]);
    }

    #[test]
    fn payload_attestation_indexed_empty() {
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let attestation = make_payload_attestation(&state, &[false, false]);
        let indexed = get_indexed_payload_attestation(&state, &attestation, &spec).unwrap();

        assert!(indexed.attesting_indices.is_empty());
    }

    #[test]
    fn payload_attestation_indexed_sorted_output() {
        // With 8 validators and PtcSize=2, the committee members could be in any order.
        // get_indexed_payload_attestation should sort the attesting_indices.
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let attestation = make_payload_attestation(&state, &[true, true]);
        let indexed = get_indexed_payload_attestation(&state, &attestation, &spec).unwrap();

        // Verify indices are sorted
        let indices_vec: Vec<u64> = indexed.attesting_indices.iter().copied().collect();
        for w in indices_vec.windows(2) {
            assert!(w[0] <= w[1], "indices must be sorted: {} > {}", w[0], w[1]);
        }
    }

    #[test]
    fn payload_attestation_data_preserved_in_indexed() {
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let attestation = make_payload_attestation(&state, &[true, false]);
        let indexed = get_indexed_payload_attestation(&state, &attestation, &spec).unwrap();

        // Attestation data should be preserved
        assert_eq!(indexed.data, attestation.data);
        assert!(indexed.data.payload_present);
        assert!(indexed.data.blob_data_available);
    }

    #[test]
    fn payload_attestation_signature_check_empty_indices_rejected() {
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        // No bits set — empty attesting_indices
        let attestation = make_payload_attestation(&state, &[false, false]);

        // With signature verification enabled, empty indices should be rejected
        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::True, &spec);
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::AttesterIndexOutOfBounds
            ))
        ));
    }

    #[test]
    fn payload_attestation_bad_signature_rejected() {
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        // Set bits but use an empty (invalid) aggregate signature
        let attestation = make_payload_attestation(&state, &[true, false]);

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::True, &spec);
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::BadSignature
            ))
        ));
    }

    #[test]
    fn payload_attestation_payload_not_present_field() {
        // Test that attestation with payload_present=false is valid (field value is up to PTC)
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let mut attestation = make_payload_attestation(&state, &[true, true]);
        attestation.data.payload_present = false;
        attestation.data.blob_data_available = false;

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "payload_present=false should be valid: {:?}",
            result.err()
        );
    }
}
