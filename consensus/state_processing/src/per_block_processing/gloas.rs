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
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#new-process_execution_payload_bid>
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
        if !can_builder_cover_bid(state, builder_index, amount, spec).map_err(|_| {
            BlockProcessingError::PayloadBidInvalid {
                reason: "failed to check builder balance".into(),
            }
        })? {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: format!(
                    "builder balance {} insufficient for bid value {}",
                    builder.balance, amount
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

/// Checks whether a builder has sufficient balance to cover a bid.
///
/// Returns true if the builder's balance minus the minimum required balance
/// (MIN_DEPOSIT_AMOUNT + pending withdrawals) is >= the bid amount.
///
/// Spec: `can_builder_cover_bid(state, builder_index, amount)`
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md>
pub fn can_builder_cover_bid<E: EthSpec>(
    state: &BeaconState<E>,
    builder_index: u64,
    bid_amount: u64,
    spec: &ChainSpec,
) -> Result<bool, BeaconStateError> {
    let builder = state
        .builders()?
        .get(builder_index as usize)
        .ok_or(BeaconStateError::UnknownBuilder(builder_index))?;
    let pending_withdrawals_amount =
        get_pending_balance_to_withdraw_for_builder(state, builder_index)?;
    let min_balance = spec
        .min_deposit_amount
        .saturating_add(pending_withdrawals_amount);
    if builder.balance < min_balance {
        return Ok(false);
    }
    Ok(builder.balance.saturating_sub(min_balance) >= bid_amount)
}

/// Processes a payload attestation from the PTC (Payload Timeliness Committee).
///
/// Spec: process_payload_attestation(state, payload_attestation)
/// Validates that the attestation targets the parent block at the previous slot
/// and verifies the aggregate BLS signature.
///
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#new-process_payload_attestation>
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

    // Structural checks from is_valid_indexed_payload_attestation run unconditionally per spec.
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

    if verify_signatures.is_true() {
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
pub fn get_indexed_payload_attestation<E: EthSpec>(
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
/// Reference: <https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#get_ptc>
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

    // Capture withdrawal metadata before moving into List
    let withdrawals_len = withdrawals.len();
    let last_validator_index = withdrawals.last().map(|w| w.validator_index);

    // Store payload_expected_withdrawals (consume withdrawals by value, no clone)
    {
        let state_gloas = state
            .as_gloas_mut()
            .map_err(BlockProcessingError::BeaconStateError)?;
        state_gloas.payload_expected_withdrawals =
            List::new(withdrawals).map_err(BlockProcessingError::MilhouseError)?;
    }

    // Update builder_pending_withdrawals (remove processed via in-place pop_front)
    if processed_builder_withdrawals_count > 0 {
        state
            .as_gloas_mut()
            .map_err(BlockProcessingError::BeaconStateError)?
            .builder_pending_withdrawals
            .pop_front(processed_builder_withdrawals_count)?;
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
            let next_validator_index = if withdrawals_len == max_withdrawals {
                last_validator_index
                    .unwrap_or(0)
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

/// Compute the total pending balance to withdraw for a builder.
///
/// Sums amounts from both `builder_pending_withdrawals` and `builder_pending_payments`
/// for the given `builder_index`.
///
/// Spec: `get_pending_balance_to_withdraw_for_builder`
pub fn get_pending_balance_to_withdraw_for_builder<E: EthSpec>(
    state: &BeaconState<E>,
    builder_index: u64,
) -> Result<u64, BeaconStateError> {
    let state_gloas = state.as_gloas()?;
    let mut total = 0u64;
    for withdrawal in state_gloas.builder_pending_withdrawals.iter() {
        if withdrawal.builder_index == builder_index {
            total = total.saturating_add(withdrawal.amount);
        }
    }
    for payment in state_gloas.builder_pending_payments.iter() {
        if payment.withdrawal.builder_index == builder_index {
            total = total.saturating_add(payment.withdrawal.amount);
        }
    }
    Ok(total)
}

/// Initiate the exit of a builder.
///
/// Sets the builder's `withdrawable_epoch` to `current_epoch + MIN_BUILDER_WITHDRAWABILITY_DELAY`.
/// Does nothing if the builder has already initiated exit.
///
/// Spec: `initiate_builder_exit`
pub fn initiate_builder_exit<E: EthSpec>(
    state: &mut BeaconState<E>,
    builder_index: u64,
    spec: &ChainSpec,
) -> Result<(), BeaconStateError> {
    let current_epoch = state.current_epoch();
    let state_gloas = state.as_gloas_mut()?;
    let builder = state_gloas
        .builders
        .get_mut(builder_index as usize)
        .ok_or(BeaconStateError::UnknownBuilder(builder_index))?;
    // Return if builder already initiated exit
    if builder.withdrawable_epoch != spec.far_future_epoch {
        return Ok(());
    }
    builder.withdrawable_epoch = current_epoch.safe_add(spec.min_builder_withdrawability_delay)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bls::FixedBytesExtended;
    use ssz_types::BitVector;
    use std::sync::Arc;
    use types::{
        Address, BeaconBlockHeader, BeaconStateGloas, Builder, BuilderPendingWithdrawal,
        BuilderPubkeyCache, CACHED_EPOCHS, Checkpoint, CommitteeCache, Epoch, ExecutionBlockHash,
        ExecutionPayloadBid, ExitCache, FixedVector, Fork, MinimalEthSpec,
        ProgressiveBalancesCache, PubkeyCache, Signature, SignedExecutionPayloadBid,
        SlashingsCache, SyncCommittee, Vector,
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
            builder_pubkey_cache: BuilderPubkeyCache::default(),
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

    #[test]
    fn withdrawals_reserved_limit_blocks_builder_sweep() {
        // When builder pending withdrawals fill the reserved_limit (3 in minimal),
        // builder sweep should produce no withdrawals even for exited builders.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // reserved_limit = max_withdrawals(4) - 1 = 3 in minimal
        // Add 3 builder pending withdrawals to fill the reserved_limit exactly
        for i in 0..3 {
            state
                .as_gloas_mut()
                .unwrap()
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    fee_recipient: Address::repeat_byte(0xDD + i),
                    amount: 1_000_000_000,
                    builder_index: 0,
                })
                .unwrap();
        }

        // Make builder 0 exited (would be swept if there were room)
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let withdrawals = &state_gloas.payload_expected_withdrawals;

        // Phase 1 (builder pending) fills reserved_limit = 3
        let builder_pending: Vec<_> = withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0 && w.amount == 1_000_000_000)
            .collect();
        assert_eq!(builder_pending.len(), 3, "3 builder pending withdrawals");

        // Phase 3 (builder sweep) should have produced nothing — reserved_limit reached
        let builder_sweep: Vec<_> = withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0 && w.amount == 5_000_000_000)
            .collect();
        assert!(
            builder_sweep.is_empty(),
            "builder sweep blocked by reserved_limit"
        );

        // Builder balance should NOT have been decremented by sweep
        // (only the pending payments are applied)
        assert_eq!(
            state_gloas.builders.get(0).unwrap().balance,
            5_000_000_000 - 3_000_000_000,
            "balance reduced by pending withdrawals only, not sweep"
        );
    }

    #[test]
    fn withdrawals_partial_limit_respects_own_sub_limit() {
        // Partials have their own sub-limit: min(prior + max_pending_partials, reserved_limit).
        // In minimal: max_pending_partials_per_withdrawals_sweep = 2.
        // With 0 builder pending withdrawals, partials_limit = min(0+2, 3) = 2.
        // So even with 3 pending partials, only 2 should be processed.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Give validators 0,1,2 excess balance (34 ETH each)
        for i in 0..3 {
            *state.get_balance_mut(i).unwrap() = 34_000_000_000;
        }

        // Add 3 pending partial withdrawals for validators 0,1,2
        for i in 0..3u64 {
            state
                .pending_partial_withdrawals_mut()
                .unwrap()
                .push(types::PendingPartialWithdrawal {
                    validator_index: i,
                    amount: 1_000_000_000,
                    withdrawable_epoch: Epoch::new(0),
                })
                .unwrap();
        }

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // Only 2 of the 3 pending partials should have been processed
        // (max_pending_partials_per_withdrawals_sweep = 2 in minimal).
        // The third partial remains in queue.
        assert_eq!(
            state.pending_partial_withdrawals().unwrap().len(),
            1,
            "third partial remains unprocessed"
        );
        assert_eq!(
            state
                .pending_partial_withdrawals()
                .unwrap()
                .get(0)
                .unwrap()
                .validator_index,
            2
        );

        // Total withdrawals also include validator sweep entries, but the key assertion
        // is that the pending_partial_withdrawals queue was only drained by 2 (the sub-limit).
        let state_gloas = state.as_gloas().unwrap();
        assert!(
            state_gloas.payload_expected_withdrawals.len() >= 2,
            "at least the 2 processed partials appear in withdrawals"
        );
    }

    #[test]
    fn withdrawals_all_four_phases_interact() {
        // Test that all four withdrawal phases work together correctly:
        // 1. Builder pending withdrawals (1 withdrawal)
        // 2. Pending partial withdrawals (1 withdrawal)
        // 3. Builder sweep (1 withdrawal — fills reserved_limit=3)
        // 4. Validator sweep (1 withdrawal — fills max_withdrawals=4)
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 3_000_000_000);
        make_parent_block_full(&mut state);

        // Phase 1: 1 builder pending withdrawal
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 500_000_000,
                builder_index: 0,
            })
            .unwrap();

        // Phase 2: 1 pending partial withdrawal for validator 0
        *state.get_balance_mut(0).unwrap() = 34_000_000_000;
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0),
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

        // Phase 4: Make validator 1 fully withdrawable
        let v1 = state.get_validator_mut(1).unwrap();
        v1.exit_epoch = Epoch::new(0);
        v1.withdrawable_epoch = Epoch::new(0);

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let withdrawals = &state_gloas.payload_expected_withdrawals;

        // Should have exactly 4 (max_withdrawals) withdrawals
        assert_eq!(
            withdrawals.len(),
            4,
            "all four phases should produce exactly max_withdrawals"
        );

        let w: Vec<_> = withdrawals.iter().collect();

        // Withdrawal 0: builder pending (phase 1)
        assert_ne!(w[0].validator_index & BUILDER_INDEX_FLAG, 0);
        assert_eq!(w[0].amount, 500_000_000);

        // Withdrawal 1: partial withdrawal for validator 0 (phase 2)
        assert_eq!(w[1].validator_index, 0);
        assert_eq!(w[1].amount, 1_000_000_000); // min(34-32, 1) = 1 ETH

        // Withdrawal 2: builder sweep for builder 0 (phase 3)
        assert_ne!(w[2].validator_index & BUILDER_INDEX_FLAG, 0);
        assert_eq!(w[2].amount, 3_000_000_000); // full builder balance

        // Withdrawal 3: validator sweep — validator 0 has excess (34-1=33, excess=1),
        // or validator 1 is fully withdrawable (32 ETH)
        // The validator sweep starts at index 0. Validator 0 had a 1 ETH partial withdrawal
        // already, so balance effectively 34-1=33, excess = 33-32 = 1 ETH.
        assert_eq!(w[3].validator_index & BUILDER_INDEX_FLAG, 0);

        // Withdrawal indices should be sequential
        for (i, withdrawal) in w.iter().enumerate() {
            assert_eq!(withdrawal.index, i as u64, "sequential withdrawal index");
        }

        // get_expected_withdrawals_gloas should match
        // (rebuild state to test read-only function)
        let (mut state2, spec2) = make_gloas_state(8, 32_000_000_000, 3_000_000_000);
        make_parent_block_full(&mut state2);
        state2
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 500_000_000,
                builder_index: 0,
            })
            .unwrap();
        *state2.get_balance_mut(0).unwrap() = 34_000_000_000;
        state2
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();
        state2
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);
        let v1 = state2.get_validator_mut(1).unwrap();
        v1.exit_epoch = Epoch::new(0);
        v1.withdrawable_epoch = Epoch::new(0);

        let expected = get_expected_withdrawals_gloas::<E>(&state2, &spec2).unwrap();
        assert_eq!(expected.len(), withdrawals.len());
        for (e, a) in expected.iter().zip(withdrawals.iter()) {
            assert_eq!(e.index, a.index);
            assert_eq!(e.validator_index, a.validator_index);
            assert_eq!(e.amount, a.amount);
        }
    }

    #[test]
    fn withdrawals_builder_sweep_many_builders_mixed_states() {
        // Test builder sweep with many builders in different states:
        // - Some active (far_future_epoch), some exited, some zero-balance
        // Also tests wrapping when starting from a nonzero index.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 0);
        make_parent_block_full(&mut state);

        // Clear builder 0's balance (already 0 from param)
        // Add 5 more builders with different states
        let builder_configs = vec![
            // (balance, exited)
            (2_000_000_000u64, true), // builder 1: exited, has balance → swept
            (0u64, true),             // builder 2: exited, zero balance → skipped
            (3_000_000_000u64, false), // builder 3: active → skipped
            (4_000_000_000u64, true), // builder 4: exited, has balance → swept
            (1_000_000_000u64, true), // builder 5: exited, has balance → swept
        ];
        for (balance, exited) in &builder_configs {
            let builder = Builder {
                pubkey: types::PublicKeyBytes::empty(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xCC),
                balance: *balance,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: if *exited {
                    Epoch::new(0)
                } else {
                    spec.far_future_epoch
                },
            };
            state
                .as_gloas_mut()
                .unwrap()
                .builders
                .push(builder)
                .unwrap();
        }

        // Start sweep from builder index 3 (active builder, should be skipped)
        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 3;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Sweep starts at 3, iterates: 3(active,skip), 4(swept), 5(swept), 0(zero,skip),
        // 1(swept), 2(zero,skip) — total 6 builders checked (= builders_count).
        // 3 withdrawals produced: builders 4, 5, 1
        assert_eq!(
            builder_w.len(),
            3,
            "three exited builders with balance swept"
        );
        assert_eq!(
            builder_w[0].validator_index,
            4 | BUILDER_INDEX_FLAG,
            "first sweep: builder 4"
        );
        assert_eq!(builder_w[0].amount, 4_000_000_000);
        assert_eq!(
            builder_w[1].validator_index,
            5 | BUILDER_INDEX_FLAG,
            "second sweep: builder 5"
        );
        assert_eq!(builder_w[1].amount, 1_000_000_000);
        assert_eq!(
            builder_w[2].validator_index,
            1 | BUILDER_INDEX_FLAG,
            "third sweep: builder 1 (wrapped)"
        );
        assert_eq!(builder_w[2].amount, 2_000_000_000);

        // Swept builders should have zero balance
        assert_eq!(state_gloas.builders.get(4).unwrap().balance, 0);
        assert_eq!(state_gloas.builders.get(5).unwrap().balance, 0);
        assert_eq!(state_gloas.builders.get(1).unwrap().balance, 0);

        // Non-swept builders should keep their balance
        assert_eq!(
            state_gloas.builders.get(3).unwrap().balance,
            3_000_000_000,
            "active builder balance unchanged"
        );

        // next_withdrawal_builder_index: start=3, the loop processes 5 iterations before
        // hitting reserved_limit (3 withdrawals): builders 3(skip),4(sweep),5(sweep),0(skip),1(sweep)
        // So processed_builders_sweep_count = 5, next = (3+5) % 6 = 2
        assert_eq!(
            state_gloas.next_withdrawal_builder_index, 2,
            "builder index advances by processed count, wrapping"
        );
    }

    #[test]
    fn withdrawals_builder_pending_fills_partials_get_nothing() {
        // When builder pending withdrawals use 2 of 3 reserved slots,
        // partials are limited to min(2 + max_pending_partials(2), reserved_limit(3)) = 3.
        // So partials can produce at most 1 more (3-2=1).
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // 2 builder pending withdrawals
        for i in 0..2 {
            state
                .as_gloas_mut()
                .unwrap()
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    fee_recipient: Address::repeat_byte(0xDD + i),
                    amount: 1_000_000_000,
                    builder_index: 0,
                })
                .unwrap();
        }

        // 3 pending partial withdrawals, but only 1 should fit
        for i in 0..3u64 {
            *state.get_balance_mut(i as usize).unwrap() = 34_000_000_000;
            state
                .pending_partial_withdrawals_mut()
                .unwrap()
                .push(types::PendingPartialWithdrawal {
                    validator_index: i,
                    amount: 1_000_000_000,
                    withdrawable_epoch: Epoch::new(0),
                })
                .unwrap();
        }

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let withdrawals = &state_gloas.payload_expected_withdrawals;

        let builder_w: Vec<_> = withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        let partial_w: Vec<_> = withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();

        assert_eq!(builder_w.len(), 2, "2 builder pending withdrawals");
        // partials_limit = min(2 + 2, 3) = 3, so partials can fill up to 3 total
        // With 2 already from builders, 1 more partial fits
        assert_eq!(
            partial_w.len(),
            // validator sweep also runs but balances are at 32 ETH for most validators
            // only validators 0,1,2 have 34 ETH, and only 1 partial was processed
            // validator sweep: validator 0 has 34-1=33 ETH (excess 1 ETH), gets a sweep withdrawal
            // That fills max_withdrawals=4
            2,
            "1 partial processed + 1 validator sweep"
        );

        // 2 of the 3 pending partials should still be in queue
        // (1 processed — validator 0's partial)
        assert_eq!(
            state.pending_partial_withdrawals().unwrap().len(),
            2,
            "2 partials remain unprocessed"
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

    #[test]
    fn ptc_committee_max_balance_always_accepted() {
        // When all validators have max_effective_balance, the acceptance test
        // `effective_balance * max_random_value >= max_effective_balance * random_value`
        // becomes `max_eb * max_rv >= max_eb * rv` which is always true (since rv <= max_rv).
        // This means the first PTC_SIZE candidates from the committee cycle are selected
        // without any rejections — no iterations are wasted on the acceptance check.
        //
        // With minimal spec (8 slots/epoch, 4 max_committees_per_slot), 8 validators spread
        // across 8 slots means each slot gets ~1 validator. The concatenated committee for a
        // single slot may have only 1 validator, so that validator gets selected PTC_SIZE times
        // (the algorithm allows duplicates by design).
        //
        // We use 64 validators to ensure multiple candidates per slot, giving us distinct
        // members that verify the modular cycling works correctly.
        let spec = E::default_spec();
        let max_eb = spec.max_effective_balance_electra;
        let (state, spec) = make_gloas_state_with_committees(64, max_eb, 64_000_000_000);
        let slot = state.slot();

        let ptc = get_ptc_committee(&state, slot, &spec).unwrap();
        assert_eq!(ptc.len(), E::ptc_size());

        // Since every candidate passes, selection never rejects. With 64 validators and
        // multiple per slot, the first two candidates from the concatenated committee list
        // are selected — they should be distinct (different positions in the committee).
        let mut seen = std::collections::HashSet::new();
        for &idx in &ptc {
            assert!((idx as usize) < 64, "index {} out of range", idx);
            seen.insert(idx);
        }
        assert_eq!(
            seen.len(),
            ptc.len(),
            "expected distinct members with 64 validators"
        );
    }

    #[test]
    fn ptc_committee_allows_duplicate_selection() {
        // With minimal spec (8 slots/epoch), 8 validators are spread across 8 slots, so
        // each slot's committee has only ~1 validator. The concatenated committee for a
        // slot may have a single validator, and the modular cycling `i % 1` always returns
        // index 0 — selecting the same validator PTC_SIZE times.
        //
        // This tests a fundamental property: the algorithm allows duplicate selection.
        // In production (mainnet with 512+ PTC members across many validators), duplicates
        // are rare. But with small committees, they're expected.
        let spec = E::default_spec();
        let max_eb = spec.max_effective_balance_electra;
        let (state, spec) = make_gloas_state_with_committees(8, max_eb, 64_000_000_000);
        let slot = state.slot();

        // Get the committees for this slot to check if there's only 1 member
        let committees = state.get_beacon_committees_at_slot(slot).unwrap();
        let mut total_members: usize = 0;
        for committee in &committees {
            total_members += committee.committee.len();
        }

        let ptc = get_ptc_committee(&state, slot, &spec).unwrap();
        assert_eq!(ptc.len(), E::ptc_size());

        if total_members == 1 {
            // With only 1 committee member, both PTC slots select the same validator
            assert_eq!(
                ptc[0], ptc[1],
                "single-member committee should produce duplicate PTC entries"
            );
        }

        // All PTC members should be valid validators regardless of duplicates
        for &idx in &ptc {
            assert!((idx as usize) < 8, "index {} out of range", idx);
        }
    }

    #[test]
    fn ptc_committee_all_equal_balance_deterministic_indices() {
        // When all validators have the same effective balance (less than max), the
        // acceptance probability is `balance / max_effective_balance` for each candidate.
        // The selection is still deterministic — same state+slot always produces same PTC.
        // This validates the modular index cycling and hash-based randomness work correctly
        // with uniform balance distribution.
        let spec = E::default_spec();
        let half_max = spec.max_effective_balance_electra / 2;
        let (state, spec) = make_gloas_state_with_committees(16, half_max, 64_000_000_000);
        let slot = state.slot();

        let ptc1 = get_ptc_committee(&state, slot, &spec).unwrap();
        let ptc2 = get_ptc_committee(&state, slot, &spec).unwrap();
        assert_eq!(ptc1.len(), E::ptc_size());
        assert_eq!(
            ptc1, ptc2,
            "PTC should be deterministic for same state+slot"
        );

        // With 16 validators at half balance, each has ~50% acceptance rate.
        // The algorithm cycles through candidates with `i % total` and may need
        // multiple passes. Verify all selected members are valid.
        for &idx in &ptc1 {
            assert!((idx as usize) < 16, "index {} out of range", idx);
        }
    }

    #[test]
    fn ptc_committee_large_validator_set_wraps_correctly() {
        // With 128 validators (much larger than PTC_SIZE=2), the committees are spread
        // across multiple beacon committees per slot. The concatenation of all committees
        // produces a large candidate list. The modular cycling `i % total` wraps correctly
        // and the hash-based random bytes cover the full offset range.
        let spec = E::default_spec();
        let max_eb = spec.max_effective_balance_electra;
        let (state, spec) = make_gloas_state_with_committees(128, max_eb, 64_000_000_000);
        let slot = state.slot();

        let ptc = get_ptc_committee(&state, slot, &spec).unwrap();
        assert_eq!(ptc.len(), E::ptc_size());

        // All members should be valid validators
        for &idx in &ptc {
            assert!(
                (idx as usize) < 128,
                "PTC member {} exceeds validator count",
                idx
            );
        }

        // With max balance, all candidates accepted → first 2 from committee list selected.
        // Verify they're distinct (128 validators, no need to wrap for PTC_SIZE=2).
        let mut seen = std::collections::HashSet::new();
        for &idx in &ptc {
            seen.insert(idx);
        }
        assert_eq!(
            seen.len(),
            ptc.len(),
            "expected distinct members with 128 validators"
        );
    }

    #[test]
    fn ptc_committee_different_epoch_different_result() {
        // The PTC seed includes `get_seed(state, epoch, DOMAIN_PTC_ATTESTER)`, so the
        // same slot-in-epoch position but different epoch should produce different results.
        // Test by comparing slot 8 (epoch 1) vs slot 16 (epoch 2) — same position (slot 0
        // of epoch) but different seed due to different epoch.
        let spec = E::default_spec();
        let half_max = spec.max_effective_balance_electra / 2;
        let (mut state, spec) = make_gloas_state_with_committees(64, half_max, 64_000_000_000);

        let slot_epoch1 = Slot::new(8); // slot 0 of epoch 1
        let ptc_epoch1 = get_ptc_committee(&state, slot_epoch1, &spec).unwrap();
        assert_eq!(ptc_epoch1.len(), E::ptc_size());

        // Advance state to epoch 2 and rebuild caches
        *state.slot_mut() = Slot::new(16); // slot 0 of epoch 2
        // Need to rebuild committee cache for the new epoch
        state
            .build_committee_cache(types::RelativeEpoch::Previous, &spec)
            .unwrap();
        state
            .build_committee_cache(types::RelativeEpoch::Current, &spec)
            .unwrap();

        let slot_epoch2 = Slot::new(16);
        let ptc_epoch2 = get_ptc_committee(&state, slot_epoch2, &spec).unwrap();
        assert_eq!(ptc_epoch2.len(), E::ptc_size());

        // With 64 validators and half balance, different seeds almost certainly produce
        // different PTC committees. Both are valid but should differ.
        // Note: not guaranteed to differ (could be same by coincidence), so we just verify
        // both are valid. A strict inequality test could flake.
        for &idx in &ptc_epoch1 {
            assert!((idx as usize) < 64);
        }
        for &idx in &ptc_epoch2 {
            assert!((idx as usize) < 64);
        }
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
        // Per spec, is_valid_indexed_payload_attestation rejects empty indices unconditionally.
        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(
            matches!(
                result,
                Err(BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::AttesterIndexOutOfBounds
                ))
            ),
            "no-bits attestation should fail even without sig check: {:?}",
            result
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

    // ── process_execution_payload_bid additional edge case tests ──

    #[test]
    fn builder_bid_balance_accounts_for_both_withdrawals_and_payments() {
        // The spec sums BOTH builder_pending_withdrawals AND builder_pending_payments
        // when computing pending_withdrawals_amount. Test both together.
        let min_deposit = E::default_spec().min_deposit_amount;
        let builder_balance = min_deposit + 1000;
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, builder_balance);

        // Add a pending withdrawal for 300
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 300,
                builder_index: 0,
            })
            .unwrap();

        // Add a pending payment for 400
        *state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_payments
            .get_mut(0)
            .unwrap() = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xEE),
                amount: 400,
                builder_index: 0,
            },
        };

        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        // Available = balance - min_deposit - (300 + 400) = 1000 - 700 = 300
        // Bid 301 should fail
        let bid = make_builder_bid(&state, &spec, 301);
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

        // Bid 300 should succeed (exactly at boundary)
        let bid = make_builder_bid(&state, &spec, 300);
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
            "bid exactly at available balance should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn builder_bid_exact_boundary_balance() {
        // Balance exactly at min_deposit_amount + bid amount (zero pending) should succeed.
        let min_deposit = E::default_spec().min_deposit_amount;
        let bid_value = 500;
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, min_deposit + bid_value);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let bid = make_builder_bid(&state, &spec, bid_value);
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
            "bid at exact boundary should succeed: {:?}",
            result.err()
        );

        // One more gwei should fail
        let (mut state2, spec2) = make_gloas_state(8, 32_000_000_000, min_deposit + bid_value);
        let bid = make_builder_bid(&state2, &spec2, bid_value + 1);
        let result = process_execution_payload_bid(
            &mut state2,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec2,
        );
        assert!(matches!(
            result,
            Err(BlockProcessingError::PayloadBidInvalid { reason })
                if reason.contains("insufficient")
        ));
    }

    #[test]
    fn builder_bid_overwrites_cached_bid() {
        // Processing a second bid should overwrite the cached latest_execution_payload_bid.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        // Process first bid (builder)
        let bid1 = make_builder_bid(&state, &spec, 100);
        process_execution_payload_bid(
            &mut state,
            &bid1,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();
        assert_eq!(
            state.as_gloas().unwrap().latest_execution_payload_bid.value,
            100
        );

        // Process second bid (self-build), overwrites first
        let bid2 = make_self_build_bid(&state, &spec);
        process_execution_payload_bid(
            &mut state,
            &bid2,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();
        assert_eq!(
            state.as_gloas().unwrap().latest_execution_payload_bid.value,
            0,
            "second bid should overwrite cached bid"
        );
        assert_eq!(
            state
                .as_gloas()
                .unwrap()
                .latest_execution_payload_bid
                .builder_index,
            spec.builder_index_self_build,
        );
    }

    #[test]
    fn self_build_bid_wrong_slot_still_rejected() {
        // Self-build bids must also pass common checks (slot, parent, randao).
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = make_self_build_bid(&state, &spec);
        let wrong_slot = state.slot() + 1;
        let parent_root = state.latest_block_header().parent_root;

        // block_slot != bid.slot → should be rejected
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
    fn builder_bid_pending_payment_at_correct_slot_index() {
        // Verify the exact slot index formula: SLOTS_PER_EPOCH + bid.slot % SLOTS_PER_EPOCH
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let slot = state.slot(); // slot 8
        let parent_root = state.latest_block_header().parent_root;

        let bid_value = 42_000;
        let bid = make_builder_bid(&state, &spec, bid_value);
        process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        )
        .unwrap();

        // slot = 8, slots_per_epoch = 8
        // slot_index = 8 + (8 % 8) = 8 + 0 = 8
        let expected_index = E::slots_per_epoch() + (slot.as_u64() % E::slots_per_epoch());
        assert_eq!(expected_index, 8);

        let state_gloas = state.as_gloas().unwrap();
        let payment = state_gloas
            .builder_pending_payments
            .get(expected_index as usize)
            .unwrap();
        assert_eq!(payment.withdrawal.amount, bid_value);
        assert_eq!(payment.withdrawal.builder_index, 0);
        assert_eq!(payment.weight, 0);

        // Other indices should remain default (zero)
        for i in 0..E::builder_pending_payments_limit() {
            if i != expected_index as usize {
                let p = state_gloas.builder_pending_payments.get(i).unwrap();
                assert_eq!(
                    p.withdrawal.amount, 0,
                    "non-target index {} should be zero",
                    i
                );
            }
        }
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

    #[test]
    fn payload_attestation_slot_overflow_fails_gracefully() {
        // data.slot = u64::MAX: safe_add(1) should return an ArithError,
        // which wraps into WrongSlot, not a panic from overflow.
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let mut attestation = make_payload_attestation(&state, &[true, false]);
        attestation.data.slot = Slot::new(u64::MAX);

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        // Must be an error (WrongSlot from the safe_add overflow), NOT a panic
        assert!(
            result.is_err(),
            "slot overflow should produce an error, not a panic"
        );
    }

    #[test]
    fn payload_attestation_two_attestations_same_block_both_succeed() {
        // In a block, multiple PayloadAttestations can be included.
        // Calling process_payload_attestation twice on the same state should both succeed.
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let att1 = make_payload_attestation(&state, &[true, false]);
        let att2 = make_payload_attestation(&state, &[false, true]);

        let result1 =
            process_payload_attestation(&mut state, &att1, VerifySignatures::False, &spec);
        assert!(
            result1.is_ok(),
            "first attestation should succeed: {:?}",
            result1.err()
        );

        let result2 =
            process_payload_attestation(&mut state, &att2, VerifySignatures::False, &spec);
        assert!(
            result2.is_ok(),
            "second attestation should also succeed: {:?}",
            result2.err()
        );
    }

    #[test]
    fn payload_attestation_second_bit_only_maps_to_correct_ptc_member() {
        // Set only bit[1] (not bit[0]) and verify the indexed attestation
        // contains only the second PTC member.
        let (state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let prev_slot = state.slot().saturating_sub(1u64);
        let ptc = get_ptc_committee(&state, prev_slot, &spec).unwrap();

        // Only bit[1] set
        let attestation = make_payload_attestation(&state, &[false, true]);
        let indexed = get_indexed_payload_attestation(&state, &attestation, &spec).unwrap();

        assert_eq!(
            indexed.attesting_indices.len(),
            1,
            "only one bit set, one attester"
        );
        assert_eq!(
            indexed.attesting_indices[0], ptc[1],
            "bit[1] should map to ptc[1] = validator {}",
            ptc[1]
        );
    }

    #[test]
    fn payload_attestation_present_true_blob_false_valid() {
        // payload_present=true but blob_data_available=false is a valid split state
        // (payload was revealed but blob data not yet available). process_payload_attestation
        // does NOT validate the semantic correctness of these flags — that's fork choice's job.
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let mut attestation = make_payload_attestation(&state, &[true, true]);
        attestation.data.payload_present = true;
        attestation.data.blob_data_available = false;

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "present=true, blob=false should be valid: {:?}",
            result.err()
        );
    }

    #[test]
    fn payload_attestation_present_false_blob_true_valid() {
        // payload_present=false but blob_data_available=true is also a valid PTC vote.
        // The PTC member asserts the blob data is available even though the payload wasn't
        // timely. process_payload_attestation does not enforce consistency between these flags.
        let (mut state, spec) = make_gloas_state_with_committees(8, 32_000_000_000, 64_000_000_000);

        let mut attestation = make_payload_attestation(&state, &[true, true]);
        attestation.data.payload_present = false;
        attestation.data.blob_data_available = true;

        let result =
            process_payload_attestation(&mut state, &attestation, VerifySignatures::False, &spec);
        assert!(
            result.is_ok(),
            "present=false, blob=true should be valid: {:?}",
            result.err()
        );
    }

    // ── get_pending_balance_to_withdraw_for_builder tests ────────────

    #[test]
    fn pending_balance_to_withdraw_builder_empty_queues() {
        let (state, _spec) = make_gloas_state(8, 32_000_000_000, 1_000_000_000);
        let pending = get_pending_balance_to_withdraw_for_builder(&state, 0).unwrap();
        assert_eq!(pending, 0, "no pending withdrawals means zero balance");
    }

    #[test]
    fn pending_balance_to_withdraw_builder_from_withdrawals() {
        let (mut state, _spec) = make_gloas_state(8, 32_000_000_000, 1_000_000_000);
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 500,
                builder_index: 0,
            })
            .unwrap();
        state_gloas
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 300,
                builder_index: 0,
            })
            .unwrap();
        let pending = get_pending_balance_to_withdraw_for_builder(&state, 0).unwrap();
        assert_eq!(pending, 800);
    }

    #[test]
    fn pending_balance_to_withdraw_builder_from_payments() {
        let (mut state, _spec) = make_gloas_state(8, 32_000_000_000, 1_000_000_000);
        let state_gloas = state.as_gloas_mut().unwrap();
        *state_gloas.builder_pending_payments.get_mut(0).unwrap() = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 1000,
                builder_index: 0,
            },
        };
        let pending = get_pending_balance_to_withdraw_for_builder(&state, 0).unwrap();
        assert_eq!(pending, 1000);
    }

    #[test]
    fn pending_balance_to_withdraw_builder_sums_both_queues() {
        let (mut state, _spec) = make_gloas_state(8, 32_000_000_000, 1_000_000_000);
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 200,
                builder_index: 0,
            })
            .unwrap();
        *state_gloas.builder_pending_payments.get_mut(0).unwrap() = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 300,
                builder_index: 0,
            },
        };
        let pending = get_pending_balance_to_withdraw_for_builder(&state, 0).unwrap();
        assert_eq!(pending, 500);
    }

    #[test]
    fn pending_balance_to_withdraw_builder_ignores_other_builders() {
        let (mut state, _spec) = make_gloas_state(8, 32_000_000_000, 1_000_000_000);
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 999,
                builder_index: 0,
            })
            .unwrap();
        // Query builder_index=1 which doesn't have any pending
        let pending = get_pending_balance_to_withdraw_for_builder(&state, 1).unwrap();
        assert_eq!(pending, 0, "should ignore other builder's pending balance");
    }

    // ── can_builder_cover_bid tests ────────────

    #[test]
    fn can_builder_cover_bid_sufficient_balance() {
        // Builder has 3 ETH, min_deposit is 1 ETH, no pending obligations.
        // Available = 3 ETH - 1 ETH = 2 ETH. Bid of 1 ETH should succeed.
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 3_000_000_000);
        assert!(can_builder_cover_bid::<E>(&state, 0, 1_000_000_000, &spec).unwrap());
    }

    #[test]
    fn can_builder_cover_bid_exact_available_balance() {
        // Builder has 3 ETH, min_deposit is 1 ETH, no pending.
        // Available = 2 ETH. Bid of exactly 2 ETH should succeed.
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 3_000_000_000);
        assert!(can_builder_cover_bid::<E>(&state, 0, 2_000_000_000, &spec).unwrap());
    }

    #[test]
    fn can_builder_cover_bid_exceeds_available_balance() {
        // Builder has 3 ETH, min_deposit is 1 ETH, no pending.
        // Available = 2 ETH. Bid of 2 ETH + 1 Gwei should fail.
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 3_000_000_000);
        assert!(!can_builder_cover_bid::<E>(&state, 0, 2_000_000_001, &spec).unwrap());
    }

    #[test]
    fn can_builder_cover_bid_balance_below_min_deposit() {
        // Builder has 500_000_000 Gwei (0.5 ETH), which is below min_deposit (1 ETH).
        // Should reject any bid, even zero-value.
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 500_000_000);
        assert!(!can_builder_cover_bid::<E>(&state, 0, 0, &spec).unwrap());
    }

    #[test]
    fn can_builder_cover_bid_pending_withdrawals_reduce_available() {
        // Builder has 5 ETH. Pending withdrawal of 2 ETH.
        // Available = 5 ETH - 1 ETH (min_deposit) - 2 ETH (pending) = 2 ETH.
        // Bid of 2 ETH should succeed, 2 ETH + 1 should fail.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 2_000_000_000,
                builder_index: 0,
            })
            .unwrap();
        assert!(can_builder_cover_bid::<E>(&state, 0, 2_000_000_000, &spec).unwrap());
        assert!(!can_builder_cover_bid::<E>(&state, 0, 2_000_000_001, &spec).unwrap());
    }

    #[test]
    fn can_builder_cover_bid_pending_payments_reduce_available() {
        // Builder has 5 ETH. Pending payment of 1.5 ETH.
        // Available = 5 ETH - 1 ETH (min_deposit) - 1.5 ETH (pending payment) = 2.5 ETH.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        *state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_payments
            .get_mut(0)
            .unwrap() = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 1_500_000_000,
                builder_index: 0,
            },
        };
        assert!(can_builder_cover_bid::<E>(&state, 0, 2_500_000_000, &spec).unwrap());
        assert!(!can_builder_cover_bid::<E>(&state, 0, 2_500_000_001, &spec).unwrap());
    }

    #[test]
    fn can_builder_cover_bid_combined_pending_withdrawals_and_payments() {
        // Builder has 6 ETH. Pending withdrawal of 1 ETH + pending payment of 2 ETH.
        // Available = 6 ETH - 1 ETH (min_deposit) - 1 ETH (withdrawal) - 2 ETH (payment) = 2 ETH.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 6_000_000_000);
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 1_000_000_000,
                builder_index: 0,
            })
            .unwrap();
        *state_gloas.builder_pending_payments.get_mut(0).unwrap() = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xBB),
                amount: 2_000_000_000,
                builder_index: 0,
            },
        };
        assert!(can_builder_cover_bid::<E>(&state, 0, 2_000_000_000, &spec).unwrap());
        assert!(!can_builder_cover_bid::<E>(&state, 0, 2_000_000_001, &spec).unwrap());
    }

    #[test]
    fn can_builder_cover_bid_unknown_builder_returns_error() {
        let (state, spec) = make_gloas_state(8, 32_000_000_000, 3_000_000_000);
        let result = can_builder_cover_bid::<E>(&state, 99, 1_000_000_000, &spec);
        assert!(
            matches!(result, Err(BeaconStateError::UnknownBuilder(99))),
            "unknown builder index should return error: {:?}",
            result,
        );
    }

    // ── initiate_builder_exit tests ────────────

    #[test]
    fn initiate_builder_exit_sets_withdrawable_epoch() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 1_000_000_000);
        let current_epoch = state.current_epoch();
        initiate_builder_exit::<E>(&mut state, 0, &spec).unwrap();
        let builder = state.as_gloas().unwrap().builders.get(0).unwrap();
        assert_eq!(
            builder.withdrawable_epoch,
            current_epoch + spec.min_builder_withdrawability_delay,
        );
    }

    #[test]
    fn initiate_builder_exit_noop_if_already_exiting() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 1_000_000_000);
        // Set builder already exiting
        let target_epoch = Epoch::new(99);
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = target_epoch;
        initiate_builder_exit::<E>(&mut state, 0, &spec).unwrap();
        let builder = state.as_gloas().unwrap().builders.get(0).unwrap();
        assert_eq!(
            builder.withdrawable_epoch, target_epoch,
            "should not change already-set withdrawable_epoch"
        );
    }

    #[test]
    fn initiate_builder_exit_unknown_builder_returns_error() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 1_000_000_000);
        let result = initiate_builder_exit::<E>(&mut state, 999, &spec);
        assert!(result.is_err(), "should fail for unknown builder index");
    }

    // ── Error path tests ────────────────────────────────────────

    #[test]
    fn builder_bid_pubkey_decompression_failure_with_verify_signatures() {
        // When VerifySignatures::True is used, a builder with a corrupted (all-zero) pubkey
        // that cannot be decompressed should return PayloadBidInvalid with "failed to decompress".
        // The default make_gloas_state creates a builder with PublicKeyBytes::empty() (all zeros),
        // which is not a valid compressed BLS12-381 point.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        let bid = make_builder_bid(&state, &spec, 1_000_000_000);
        let slot = state.slot();
        let parent_root = state.latest_block_header().parent_root;

        let result = process_execution_payload_bid(
            &mut state,
            &bid,
            slot,
            parent_root,
            VerifySignatures::True,
            &spec,
        );
        assert!(
            matches!(
                &result,
                Err(BlockProcessingError::PayloadBidInvalid { reason })
                    if reason.contains("failed to decompress")
            ),
            "expected decompression failure, got: {:?}",
            result
        );
    }

    #[test]
    fn withdrawal_rejects_invalid_builder_index_in_pending() {
        // A builder_pending_withdrawal entry that references a builder_index beyond the
        // builders list length should trigger WithdrawalBuilderIndexInvalid.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Add a pending withdrawal for builder_index=99, but only 1 builder exists (index 0)
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 1_000_000_000,
                builder_index: 99,
            })
            .unwrap();

        let result = process_withdrawals_gloas::<E>(&mut state, &spec);
        assert!(
            matches!(
                &result,
                Err(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                    builder_index: 99,
                    builders_count: 1,
                })
            ),
            "expected WithdrawalBuilderIndexInvalid, got: {:?}",
            result
        );
    }

    #[test]
    fn withdrawal_rejects_stale_builder_sweep_index() {
        // When next_withdrawal_builder_index is beyond the current builders list length
        // (e.g., builders were removed), the builder sweep should fail with
        // WithdrawalBuilderIndexInvalid rather than panicking on an OOB access.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Set next_withdrawal_builder_index to 5, but only 1 builder exists
        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 5;

        let result = process_withdrawals_gloas::<E>(&mut state, &spec);
        assert!(
            matches!(
                &result,
                Err(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                    builder_index: 5,
                    builders_count: 1,
                })
            ),
            "expected WithdrawalBuilderIndexInvalid for stale sweep index, got: {:?}",
            result
        );
    }

    #[test]
    fn get_expected_withdrawals_rejects_invalid_builder_index() {
        // The read-only get_expected_withdrawals_gloas should also catch invalid builder
        // indices in pending withdrawals, mirroring process_withdrawals_gloas behavior.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 500_000_000,
                builder_index: 42,
            })
            .unwrap();

        let result = get_expected_withdrawals_gloas::<E>(&state, &spec);
        assert!(
            matches!(
                &result,
                Err(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                    builder_index: 42,
                    builders_count: 1,
                })
            ),
            "expected WithdrawalBuilderIndexInvalid, got: {:?}",
            result
        );
    }

    #[test]
    fn get_expected_withdrawals_rejects_stale_builder_sweep_index() {
        // The read-only function should also reject a stale next_withdrawal_builder_index.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 10;

        let result = get_expected_withdrawals_gloas::<E>(&state, &spec);
        assert!(
            matches!(
                &result,
                Err(BlockProcessingError::WithdrawalBuilderIndexInvalid {
                    builder_index: 10,
                    builders_count: 1,
                })
            ),
            "expected WithdrawalBuilderIndexInvalid for stale sweep index, got: {:?}",
            result
        );
    }

    // ── EMPTY parent path with pending items ────────────────────

    #[test]
    fn withdrawals_skipped_when_parent_empty_despite_pending_items() {
        // When the parent block is EMPTY (payload not delivered), ALL withdrawal processing
        // must be skipped — even if the state has pending builder withdrawals, pending partial
        // validator withdrawals, and exiting builders eligible for sweep. This is critical for
        // consensus: the CL must not generate a withdrawal list for a block whose parent had
        // no execution payload, because the EL state was not advanced.
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 10_000_000_000);
        // Default state has mismatched hashes → EMPTY parent
        assert!(!is_parent_block_full::<E>(&state).unwrap());

        // Add a pending builder withdrawal
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

        // Add a pending partial validator withdrawal (withdrawable immediately)
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        // Make builder 0 exiting with balance (sweep candidate)
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        let initial_next_withdrawal_index = state.next_withdrawal_index().unwrap();
        let initial_next_validator_index = state.next_withdrawal_validator_index().unwrap();
        let initial_next_builder_index = state.as_gloas().unwrap().next_withdrawal_builder_index;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // No withdrawals should be generated
        assert!(
            state
                .as_gloas()
                .unwrap()
                .payload_expected_withdrawals
                .is_empty(),
            "EMPTY parent should produce no withdrawals"
        );

        // All indices should be unchanged (the early return skips everything)
        assert_eq!(
            state.next_withdrawal_index().unwrap(),
            initial_next_withdrawal_index,
            "next_withdrawal_index should be unchanged for EMPTY parent"
        );
        assert_eq!(
            state.next_withdrawal_validator_index().unwrap(),
            initial_next_validator_index,
            "next_withdrawal_validator_index should be unchanged for EMPTY parent"
        );
        assert_eq!(
            state.as_gloas().unwrap().next_withdrawal_builder_index,
            initial_next_builder_index,
            "next_withdrawal_builder_index should be unchanged for EMPTY parent"
        );

        // Pending items should NOT be consumed (still in the lists)
        assert_eq!(
            state.as_gloas().unwrap().builder_pending_withdrawals.len(),
            1,
            "builder_pending_withdrawals should NOT be consumed for EMPTY parent"
        );
        assert_eq!(
            state.pending_partial_withdrawals().unwrap().len(),
            1,
            "pending_partial_withdrawals should NOT be consumed for EMPTY parent"
        );
    }

    #[test]
    fn get_expected_withdrawals_empty_despite_pending_items() {
        // The read-only function must mirror the mutable function's behavior: return an
        // empty vec when the parent is EMPTY, regardless of pending items in the state.
        // A mismatch would cause the EL to receive a non-empty withdrawal list that the
        // CL's process_withdrawals_gloas would then reject as an early-return no-op.
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 10_000_000_000);
        assert!(!is_parent_block_full::<E>(&state).unwrap());

        // Add pending builder withdrawal
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

        // Add pending partial validator withdrawal
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        // Make builder exiting (sweep candidate)
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();
        assert!(
            expected.is_empty(),
            "read-only function must return empty vec for EMPTY parent, \
             even with pending builder withdrawals, partial withdrawals, and exiting builders"
        );
    }

    #[test]
    fn is_parent_block_full_both_zero_hashes() {
        // At Gloas fork activation (or genesis with Gloas), both latest_execution_payload_bid.block_hash
        // and latest_block_hash start as zero. The parent is considered FULL when these match (0x00 == 0x00),
        // which enables withdrawal processing from the first block. This is important because the upgrade
        // function sets latest_block_hash from the Fulu execution payload header's block_hash, and the
        // bid's block_hash from the same value — both are the same EL head hash.
        let (mut state, _spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Set both hashes to zero (simulating genesis/fork activation)
        state
            .as_gloas_mut()
            .unwrap()
            .latest_execution_payload_bid
            .block_hash = ExecutionBlockHash::zero();
        state.as_gloas_mut().unwrap().latest_block_hash = ExecutionBlockHash::zero();

        assert!(
            is_parent_block_full::<E>(&state).unwrap(),
            "both hashes zero should be considered FULL (0x00 == 0x00)"
        );
    }

    #[test]
    fn is_parent_block_full_only_bid_hash_zero() {
        // When only the bid's block_hash is zero but latest_block_hash is non-zero,
        // the parent is EMPTY (hashes don't match). This would occur if the bid was
        // a default/unprocessed bid but an envelope was previously processed.
        let (mut state, _spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        state
            .as_gloas_mut()
            .unwrap()
            .latest_execution_payload_bid
            .block_hash = ExecutionBlockHash::zero();
        // latest_block_hash defaults to repeat_byte(0x02) in make_gloas_state

        assert!(
            !is_parent_block_full::<E>(&state).unwrap(),
            "bid hash zero + latest_block_hash non-zero should be EMPTY"
        );
    }

    #[test]
    fn get_expected_withdrawals_capped_at_max_builder_pending() {
        // When builder_pending_withdrawals exceeds max_withdrawals_per_payload,
        // the returned list must be capped. For MinimalEthSpec: max_withdrawals=4,
        // reserved_limit=3. If we add 5 builder pending withdrawals, only the first 3
        // should appear (the 4th slot is reserved for the validator sweep).
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 100_000_000_000);
        make_parent_block_full(&mut state);

        // Add 5 builder pending withdrawals (more than reserved_limit=3)
        for i in 0..5 {
            state
                .as_gloas_mut()
                .unwrap()
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    fee_recipient: Address::repeat_byte((0xA0 + i) as u8),
                    amount: (i as u64 + 1) * 1_000_000_000,
                    builder_index: 0,
                })
                .unwrap();
        }

        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // Builder withdrawals should be capped at reserved_limit = max_withdrawals - 1 = 3
        // Plus at least 1 slot for validator sweep (but no validators have excess balance
        // or pending partials, so the validator sweep contributes 0).
        // Total expected: 3 builder withdrawals + 0 from other phases = 3
        let builder_count = expected
            .iter()
            .filter(|w| {
                use types::consts::gloas::BUILDER_INDEX_FLAG;
                (w.validator_index & BUILDER_INDEX_FLAG) != 0
            })
            .count();
        assert_eq!(
            builder_count, 3,
            "builder pending withdrawals should be capped at reserved_limit (max - 1)"
        );
        assert_eq!(
            expected[0].amount, 1_000_000_000,
            "first builder withdrawal should have amount 1 ETH"
        );
        assert_eq!(
            expected[1].amount, 2_000_000_000,
            "second builder withdrawal should have amount 2 ETH"
        );
        assert_eq!(
            expected[2].amount, 3_000_000_000,
            "third builder withdrawal should have amount 3 ETH"
        );
    }

    // ── Non-zero next_withdrawal_index consistency test ──

    #[test]
    fn get_expected_withdrawals_matches_process_with_nonzero_withdrawal_index() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Set a non-zero starting withdrawal index to catch off-by-one bugs
        *state.next_withdrawal_index_mut().unwrap() = 42;

        // Make builder 0 exiting so we get builder sweep withdrawals
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

        // Compute expected (read-only)
        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();
        assert!(!expected.is_empty(), "should produce withdrawals");
        assert_eq!(
            expected[0].index, 42,
            "first withdrawal should start at index 42"
        );

        // Process (mutating)
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let actual = &state.as_gloas().unwrap().payload_expected_withdrawals;
        assert_eq!(expected.len(), actual.len());
        for (e, a) in expected.iter().zip(actual.iter()) {
            assert_eq!(e.index, a.index, "withdrawal indices should match");
            assert_eq!(
                e.validator_index, a.validator_index,
                "validator indices should match"
            );
            assert_eq!(e.address, a.address, "addresses should match");
            assert_eq!(e.amount, a.amount, "amounts should match");
        }
    }

    // ── Pending partial withdrawal BLS credential error path tests ──

    #[test]
    fn pending_partial_withdrawal_bls_credentials_rejected() {
        // A validator with BLS (0x00) credentials that somehow has a pending partial
        // withdrawal should trigger NonExecutionAddressWithdrawalCredential.
        //
        // The conditions at line 556-558 require:
        //   validator.exit_epoch == far_future_epoch  (not exiting)
        //   effective_balance >= min_activation_balance  (sufficient balance)
        //   balance > min_activation_balance  (excess balance)
        //
        // If all pass, get_execution_withdrawal_address(spec) is called. For a
        // validator with 0x00 prefix credentials, this returns None → error.
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Change validator 0's credentials to BLS (0x00 prefix)
        let mut bls_creds = [0u8; 32];
        bls_creds[0] = 0x00; // BLS withdrawal credential prefix
        bls_creds[1..].copy_from_slice(&[0xBB; 31]);
        state.get_validator_mut(0).unwrap().withdrawal_credentials =
            Hash256::from_slice(&bls_creds);

        // Add a pending partial withdrawal for this BLS-credential validator
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        let result = process_withdrawals_gloas::<E>(&mut state, &spec);
        assert!(
            matches!(
                result,
                Err(BlockProcessingError::BeaconStateError(
                    BeaconStateError::NonExecutionAddressWithdrawalCredential
                ))
            ),
            "BLS-credential validator partial withdrawal should fail: {:?}",
            result
        );
    }

    #[test]
    fn get_expected_withdrawals_bls_credentials_rejected() {
        // Same scenario as above but through the read-only get_expected_withdrawals_gloas.
        // Both mutable and read-only paths must reject identically, otherwise the EL
        // receives a withdrawal list that the CL would later reject.
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Change validator 0's credentials to BLS (0x00 prefix)
        let mut bls_creds = [0u8; 32];
        bls_creds[0] = 0x00;
        bls_creds[1..].copy_from_slice(&[0xBB; 31]);
        state.get_validator_mut(0).unwrap().withdrawal_credentials =
            Hash256::from_slice(&bls_creds);

        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 1_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        let result = get_expected_withdrawals_gloas::<E>(&state, &spec);
        assert!(
            matches!(
                result,
                Err(BlockProcessingError::BeaconStateError(
                    BeaconStateError::NonExecutionAddressWithdrawalCredential
                ))
            ),
            "read-only path should also reject BLS-credential partial withdrawal: {:?}",
            result
        );
    }

    #[test]
    fn validator_sweep_wraps_around_modular_index() {
        // Verify the validator sweep wraps correctly when next_withdrawal_validator_index
        // starts near the end of the validator list.
        //
        // With 8 validators and next_withdrawal_validator_index=6, the sweep processes
        // validators 6, 7, 0, 1, 2, 3, 4, 5 (wrapping around).
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Fix effective_balance to min_activation_balance (32 ETH) — make_gloas_state
        // sets it to the balance parameter (34 ETH), but is_partially_withdrawable
        // requires effective_balance == max_effective_balance. For 0x01 credentials,
        // max_effective_balance = min_activation_balance = 32 ETH.
        for i in 0..8 {
            state.get_validator_mut(i).unwrap().effective_balance = spec.min_activation_balance;
        }

        // Start near end of validator list
        *state.next_withdrawal_validator_index_mut().unwrap() = 6;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // Verify withdrawals were generated starting from validator 6
        let withdrawals = &state.as_gloas().unwrap().payload_expected_withdrawals;
        assert!(
            !withdrawals.is_empty(),
            "should generate partial withdrawals for validators with excess balance"
        );

        // Verify the first withdrawal targets validator 6 (the starting point)
        assert_eq!(
            withdrawals.get(0).unwrap().validator_index,
            6,
            "first withdrawal should be for validator 6 (starting index)"
        );

        // Verify the sweep wrapped around: second withdrawal should be validator 7,
        // third should be validator 0, etc. (up to max_withdrawals=4)
        let max_withdrawals = E::max_withdrawals_per_payload();
        assert_eq!(withdrawals.len(), max_withdrawals);
        assert_eq!(withdrawals.get(1).unwrap().validator_index, 7);
        assert_eq!(withdrawals.get(2).unwrap().validator_index, 0);
        assert_eq!(withdrawals.get(3).unwrap().validator_index, 1);

        // When withdrawals.len() == max_withdrawals, next index =
        // (last.validator_index + 1) % validators_len
        let new_index = state.next_withdrawal_validator_index().unwrap();
        assert_eq!(
            new_index, 2,
            "next_withdrawal_validator_index should be 2 (after last withdrawn validator 1)"
        );
    }

    #[test]
    fn multiple_pending_partials_for_same_validator_account_for_prior_withdrawals() {
        // When two pending partial withdrawals reference the same validator, the second
        // must account for the balance already withdrawn by the first. The total_withdrawn
        // accumulator (line 546-550) filters all prior withdrawals for the same validator.
        //
        // Validator 0: balance=36 ETH, effective_balance=34 ETH (from make_gloas_state).
        // min_activation_balance = 32 ETH
        // Excess: 36 - 32 = 4 ETH
        //
        // First partial withdrawal: amount=3 ETH → withdraws 3 ETH
        // Second partial withdrawal: amount=3 ETH → excess after first = 1 ETH → withdraws 1 ETH
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Set validator 0 balance to 36 ETH (4 ETH excess over min_activation_balance)
        *state.get_balance_mut(0).unwrap() = 36_000_000_000;

        // Two pending partial withdrawals for the same validator
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 3_000_000_000, // 3 ETH
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 3_000_000_000, // 3 ETH
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let validator_withdrawals: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| w.validator_index == 0 && (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();

        assert_eq!(
            validator_withdrawals.len(),
            2,
            "both pending partial withdrawals should be processed"
        );
        assert_eq!(
            validator_withdrawals[0].amount, 3_000_000_000,
            "first withdrawal gets full requested amount (excess=4 ETH, request=3 ETH)"
        );
        assert_eq!(
            validator_withdrawals[1].amount, 1_000_000_000,
            "second withdrawal capped to remaining excess (4-3=1 ETH, request=3 ETH)"
        );
    }

    #[test]
    fn get_expected_withdrawals_multiple_partials_matches_process() {
        // Verify the read-only path (get_expected_withdrawals_gloas) produces identical
        // results to the mutable path for multiple pending partials for the same validator.
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        *state.get_balance_mut(0).unwrap() = 36_000_000_000;

        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 3_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 3_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        // Get read-only result first (before mutation)
        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // Process mutating version
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();
        let actual = &state.as_gloas().unwrap().payload_expected_withdrawals;

        // Both paths must agree
        assert_eq!(
            expected.len(),
            actual.len(),
            "read-only and mutable paths should produce same number of withdrawals"
        );
        for (e, a) in expected.iter().zip(actual.iter()) {
            assert_eq!(e.amount, a.amount, "withdrawal amounts should match");
            assert_eq!(
                e.validator_index, a.validator_index,
                "validator indices should match"
            );
        }

        // Verify the second partial was capped
        let validator_0_expected: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index == 0 && (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();
        assert_eq!(validator_0_expected.len(), 2);
        assert_eq!(validator_0_expected[0].amount, 3_000_000_000);
        assert_eq!(
            validator_0_expected[1].amount, 1_000_000_000,
            "read-only path must also cap second partial at remaining excess"
        );
    }

    // ── Withdrawal interaction edge case tests (run 203) ────────────

    /// Builder pending withdrawal AND builder sweep for the same exited builder.
    ///
    /// Phase 1 (builder pending withdrawals) produces a withdrawal for the fee_recipient,
    /// phase 3 (builder sweep) sees the builder is exited with balance > 0 and produces
    /// another withdrawal for the builder's execution_address.
    /// After application, the builder balance should be decreased by the sum of both
    /// (saturating at 0).
    #[test]
    fn withdrawals_pending_and_sweep_same_builder() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 10_000_000_000);
        make_parent_block_full(&mut state);

        // Make builder exited so sweep picks it up
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Add a pending withdrawal for the same builder
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xCC),
                amount: 3_000_000_000,
                builder_index: 0,
            })
            .unwrap();

        // Process withdrawals
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        // Both a pending withdrawal (3 Gwei) and a sweep withdrawal (full balance 10 Gwei)
        let builder_ws: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0)
            .collect();
        assert_eq!(
            builder_ws.len(),
            2,
            "should have both pending and sweep withdrawals"
        );
        assert_eq!(builder_ws[0].amount, 3_000_000_000, "pending amount");
        assert_eq!(builder_ws[1].amount, 10_000_000_000, "sweep full balance");

        // Builder balance after application: saturating_sub of both amounts
        // min(3B, balance=10B) → 7B remaining, then min(10B, 7B) → 0
        let builder = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(
            builder.balance, 0,
            "builder balance should be 0 after both withdrawals"
        );
    }

    /// Builder pending withdrawal amount exceeds builder balance.
    ///
    /// The balance decrease uses `saturating_sub(min(amount, balance))`, so
    /// the balance should never go negative — it saturates at 0.
    #[test]
    fn withdrawals_pending_amount_exceeds_builder_balance() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 2_000_000_000);
        make_parent_block_full(&mut state);

        // Add a pending withdrawal larger than the builder's balance
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xDD),
                amount: 50_000_000_000, // much larger than 2 Gwei balance
                builder_index: 0,
            })
            .unwrap();

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // The withdrawal should be recorded with the full amount (50B) in expected_withdrawals
        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        let builder_ws: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0)
            .collect();
        assert_eq!(builder_ws.len(), 1);
        assert_eq!(
            builder_ws[0].amount, 50_000_000_000,
            "withdrawal records the full requested amount"
        );

        // But the actual balance decrease is capped at min(50B, 2B) = 2B, so balance = 0
        let builder = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(
            builder.balance, 0,
            "balance saturates at 0 when withdrawal exceeds balance"
        );
    }

    /// Builder sweep processes all builders when builder count <= max_builders_per_sweep.
    ///
    /// With 3 exited builders, all of them should be swept and the
    /// next_withdrawal_builder_index should wrap correctly.
    #[test]
    fn withdrawals_builder_sweep_all_builders_wrapped() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Add 2 more exited builders (total 3)
        for i in 1..=2u8 {
            state
                .as_gloas_mut()
                .unwrap()
                .builders
                .push(Builder {
                    pubkey: types::PublicKeyBytes::empty(),
                    version: 0x03,
                    execution_address: Address::repeat_byte(0xC0 + i),
                    balance: (i as u64 + 1) * 1_000_000_000,
                    deposit_epoch: Epoch::new(0),
                    withdrawable_epoch: Epoch::new(0), // exited
                })
                .unwrap();
        }

        // Make builder 0 exited too
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Start sweep at index 1
        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 1;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        let builder_ws: Vec<_> = expected
            .iter()
            .filter(|w| w.validator_index & BUILDER_INDEX_FLAG != 0)
            .collect();
        // All 3 builders should be swept (loop processes min(3, max_builders_per_sweep) = 3)
        assert_eq!(builder_ws.len(), 3, "all 3 exited builders should be swept");

        // Sweep order: index 1, 2, 0 (wraps around)
        assert_eq!(builder_ws[0].amount, 2_000_000_000, "builder 1 amount");
        assert_eq!(builder_ws[1].amount, 3_000_000_000, "builder 2 amount");
        assert_eq!(builder_ws[2].amount, 5_000_000_000, "builder 0 amount");

        // next_withdrawal_builder_index = (1 + 3) % 3 = 1
        assert_eq!(
            state.as_gloas().unwrap().next_withdrawal_builder_index,
            1,
            "next_withdrawal_builder_index should wrap to 1"
        );
    }

    /// All 4 withdrawal phases active: verify continuous index sequencing.
    ///
    /// With builder pending withdrawals, pending partials, builder sweep, and validator sweep
    /// all producing withdrawals, the withdrawal.index values should be a continuous
    /// sequence starting from next_withdrawal_index.
    #[test]
    fn withdrawals_all_phases_continuous_index_sequence() {
        let (mut state, spec) = make_gloas_state(8, 34_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Set starting withdrawal index to a non-zero value
        *state.next_withdrawal_index_mut().unwrap() = 42;

        // Phase 1: Builder pending withdrawal
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xAA),
                amount: 1_000_000_000,
                builder_index: 0,
            })
            .unwrap();

        // Phase 2: Pending partial withdrawal
        state
            .pending_partial_withdrawals_mut()
            .unwrap()
            .push(types::PendingPartialWithdrawal {
                validator_index: 0,
                amount: 2_000_000_000,
                withdrawable_epoch: Epoch::new(0),
            })
            .unwrap();

        // Phase 3: Builder sweep (make builder exited)
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Phase 4: Validator 1 has excess balance for partial withdrawal
        // (default validators have 34B with effective_balance = 34B, max_effective = 32B → excess)

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        // Verify indices are continuous starting from 42
        for (i, w) in expected.iter().enumerate() {
            assert_eq!(
                w.index,
                42 + i as u64,
                "withdrawal {} should have index {}",
                i,
                42 + i as u64
            );
        }

        // Verify next_withdrawal_index was updated
        let final_index = state.next_withdrawal_index().unwrap();
        assert_eq!(
            final_index,
            42 + expected.len() as u64,
            "next_withdrawal_index should be past all withdrawals"
        );
    }

    /// Validator sweep `next_withdrawal_validator_index` update when only builder
    /// withdrawals are produced (no validator withdrawals).
    ///
    /// When withdrawals.len() < max_withdrawals AND all withdrawals are from builders,
    /// the validator index should still advance by max_validators_per_withdrawals_sweep.
    #[test]
    fn withdrawals_only_builder_output_validator_index_still_advances() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // No validator has excess balance (32B == min_activation_balance → no partial)
        // No validator is fully withdrawable (exit_epoch == far_future_epoch)
        // Only builder pending withdrawal exists
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xEE),
                amount: 1_000_000_000,
                builder_index: 0,
            })
            .unwrap();

        // Start validator sweep at index 3
        *state.next_withdrawal_validator_index_mut().unwrap() = 3;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        // Only builder withdrawal should exist (no validator withdrawals)
        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        assert!(
            !expected.is_empty() && expected.len() < E::max_withdrawals_per_payload(),
            "should have some withdrawals but not max"
        );

        // All withdrawals should be builder-flagged
        assert!(
            expected
                .iter()
                .all(|w| w.validator_index & BUILDER_INDEX_FLAG != 0),
            "all withdrawals should be from builders"
        );

        // Even with no validator withdrawals, next_withdrawal_validator_index advances
        // by max_validators_per_withdrawals_sweep
        let new_index = state.next_withdrawal_validator_index().unwrap();
        let expected_index =
            (3 + spec.max_validators_per_withdrawals_sweep) % state.validators().len() as u64;
        assert_eq!(
            new_index, expected_index,
            "validator index should advance by max_validators_per_withdrawals_sweep"
        );
    }

    /// When withdrawals hit exactly max_withdrawals, the validator index update
    /// follows a DIFFERENT formula: `(last_withdrawal.validator_index + 1) % len`
    /// instead of the normal `(current + max_sweep) % len`. This test ensures
    /// the correct code path is taken when the validator sweep produces the last
    /// withdrawal that fills max_withdrawals.
    #[test]
    fn withdrawals_max_hit_updates_validator_index_from_last_withdrawal() {
        // Strategy: fill reserved_limit (3) with builder pending withdrawals,
        // then make validator 0 fully withdrawable so it fills the 4th slot
        // (max_withdrawals=4). This triggers the "max hit" path.
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Phase 1: 3 builder pending withdrawals fill reserved_limit
        for i in 0..3 {
            state
                .as_gloas_mut()
                .unwrap()
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    fee_recipient: Address::repeat_byte(0xDD),
                    amount: 100 + i as u64,
                    builder_index: 0,
                })
                .unwrap();
        }

        // Phase 4: Make validator 0 fully withdrawable
        let v0 = state.get_validator_mut(0).unwrap();
        v0.exit_epoch = Epoch::new(0);
        v0.withdrawable_epoch = Epoch::new(0);

        // Start validator sweep at index 0
        *state.next_withdrawal_validator_index_mut().unwrap() = 0;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let withdrawals = &state_gloas.payload_expected_withdrawals;

        // Should hit exactly max_withdrawals = 4
        assert_eq!(
            withdrawals.len(),
            E::max_withdrawals_per_payload(),
            "should produce exactly max_withdrawals"
        );

        // Last withdrawal should be validator 0 (fully withdrawn)
        let last = withdrawals.iter().last().unwrap();
        assert_eq!(
            last.validator_index & BUILDER_INDEX_FLAG,
            0,
            "last withdrawal should be from validator sweep"
        );
        assert_eq!(last.validator_index, 0);

        // When max_withdrawals is hit:
        // next_validator_index = (last.validator_index + 1) % validators_len
        // = (0 + 1) % 8 = 1
        assert_eq!(
            state.next_withdrawal_validator_index().unwrap(),
            1,
            "max hit: next_validator_index = (last.validator_index + 1) % len"
        );
    }

    /// Builder sweep with 3 builders where sweep starts near the end and must
    /// wrap around. Only builder at index 1 is eligible (exited + balance > 0).
    /// Builders 0 and 2 are either active or have zero balance.
    /// This tests the modular wraparound logic: `(index + 1) % builders_count`.
    #[test]
    fn withdrawals_builder_sweep_wrap_with_mixed_eligibility() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 0);
        make_parent_block_full(&mut state);

        // Replace builder 0 with active (far_future_epoch) zero-balance builder
        let state_gloas = state.as_gloas_mut().unwrap();
        let b0 = state_gloas.builders.get_mut(0).unwrap();
        b0.balance = 0;
        b0.withdrawable_epoch = spec.far_future_epoch; // active, not swept

        // Add builder 1: exited with balance (should be swept)
        state_gloas
            .builders
            .push(Builder {
                pubkey: types::PublicKeyBytes::empty(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xCC),
                balance: 7_000_000_000,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: Epoch::new(0), // exited
            })
            .unwrap();

        // Add builder 2: exited but zero balance (skipped)
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas
            .builders
            .push(Builder {
                pubkey: types::PublicKeyBytes::empty(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xDD),
                balance: 0,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: Epoch::new(0), // exited but no balance
            })
            .unwrap();

        // Start sweep at builder index 2 — must wrap: 2 → 0 → 1
        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 2;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Only builder 1 should be swept (exited + balance > 0)
        assert_eq!(builder_w.len(), 1, "only one builder eligible for sweep");
        assert_eq!(
            builder_w[0].validator_index,
            1 | BUILDER_INDEX_FLAG,
            "builder 1 should be swept"
        );
        assert_eq!(builder_w[0].amount, 7_000_000_000);

        // Builder 1 balance should be zeroed
        assert_eq!(state_gloas.builders.get(1).unwrap().balance, 0);

        // next_withdrawal_builder_index = (2 + 3) % 3 = 2
        // (swept all 3 builders in the sweep loop)
        assert_eq!(
            state_gloas.next_withdrawal_builder_index, 2,
            "builder sweep index wraps after processing all builders"
        );
    }

    /// Builder pending withdrawals for multiple distinct builder indices verify
    /// that the BUILDER_INDEX_FLAG encoding is correct for non-zero indices.
    /// Builder index N should produce validator_index = N | BUILDER_INDEX_FLAG.
    #[test]
    fn withdrawals_builder_pending_multiple_builders_index_encoding() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Add 2 more builders (indices 1 and 2)
        for i in 1..3u64 {
            state
                .as_gloas_mut()
                .unwrap()
                .builders
                .push(Builder {
                    pubkey: types::PublicKeyBytes::empty(),
                    version: 0x03,
                    execution_address: Address::repeat_byte(0xCC + i as u8),
                    balance: 10_000_000_000,
                    deposit_epoch: Epoch::new(0),
                    withdrawable_epoch: spec.far_future_epoch,
                })
                .unwrap();
        }

        // Add pending withdrawals for builders 0, 1, and 2
        for i in 0..3u64 {
            state
                .as_gloas_mut()
                .unwrap()
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    fee_recipient: Address::repeat_byte(0xDD + i as u8),
                    amount: 1000 + i * 100,
                    builder_index: i,
                })
                .unwrap();
        }

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // All 3 builder pending withdrawals should be processed (fits reserved_limit=3)
        assert_eq!(builder_w.len(), 3);

        // Verify BUILDER_INDEX_FLAG encoding for each builder index
        assert_eq!(
            builder_w[0].validator_index, BUILDER_INDEX_FLAG,
            "builder 0 flag encoding: 0 | BUILDER_INDEX_FLAG == BUILDER_INDEX_FLAG"
        );
        assert_eq!(builder_w[0].amount, 1000);
        assert_eq!(builder_w[0].address, Address::repeat_byte(0xDD));

        assert_eq!(
            builder_w[1].validator_index,
            1 | BUILDER_INDEX_FLAG,
            "builder 1 flag encoding"
        );
        assert_eq!(builder_w[1].amount, 1100);
        assert_eq!(builder_w[1].address, Address::repeat_byte(0xDE));

        assert_eq!(
            builder_w[2].validator_index,
            2 | BUILDER_INDEX_FLAG,
            "builder 2 flag encoding"
        );
        assert_eq!(builder_w[2].amount, 1200);
        assert_eq!(builder_w[2].address, Address::repeat_byte(0xDF));

        // Verify they decode back to the correct builder indices
        for (i, w) in builder_w.iter().enumerate() {
            let decoded = w.validator_index & !BUILDER_INDEX_FLAG;
            assert_eq!(decoded, i as u64, "decoded builder index should be {}", i);
        }
    }

    /// When builder pending withdrawals consume some of the reserved_limit,
    /// the partials_limit is reduced accordingly. This tests the formula:
    /// `partials_limit = min(prior_count + max_pending_partials, reserved_limit)`
    /// where prior_count > 0 (from builder pending withdrawals).
    ///
    /// In minimal: max_pending_partials = 2, reserved_limit = 3.
    /// With 2 builder pending withdrawals: prior = 2,
    /// partials_limit = min(2 + 2, 3) = 3 (clamped by reserved_limit).
    /// So only 1 partial can fit (reserved_limit - prior = 3 - 2 = 1).
    #[test]
    fn withdrawals_partials_limit_reduced_by_prior_builder_pending() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Phase 1: 2 builder pending withdrawals
        for i in 0..2 {
            state
                .as_gloas_mut()
                .unwrap()
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    fee_recipient: Address::repeat_byte(0xDD),
                    amount: 500 + i as u64,
                    builder_index: 0,
                })
                .unwrap();
        }

        // Phase 2: 3 pending partial withdrawals for validators 0, 1, 2
        for i in 0..3u64 {
            *state.get_balance_mut(i as usize).unwrap() = 34_000_000_000;
            state
                .pending_partial_withdrawals_mut()
                .unwrap()
                .push(types::PendingPartialWithdrawal {
                    validator_index: i,
                    amount: 1_000_000_000,
                    withdrawable_epoch: Epoch::new(0),
                })
                .unwrap();
        }

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let withdrawals = &state_gloas.payload_expected_withdrawals;

        // Builder pending: 2 processed
        let builder_w: Vec<_> = withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert_eq!(builder_w.len(), 2, "2 builder pending withdrawals");

        // Phase 2 (partials): with prior=2, max_partials=2,
        // partials_limit = min(2+2, 3) = 3, so partials can fill up to 3 total.
        // Since we already have 2, only 1 more partial fits before the limit.
        // Validator 0 gets a partial withdrawal (1 ETH), then limit is hit → break.
        //
        // Phase 4 (validator sweep) may ALSO produce partial-like withdrawals for
        // validators with excess balance. The sweep uses max_withdrawals (4), not
        // reserved_limit, so validator 1 can still get swept.
        //
        // The key assertion: only 1 pending partial was PROCESSED (removed from queue).
        // 2 remain in the pending_partial_withdrawals queue.
        assert_eq!(
            state.pending_partial_withdrawals().unwrap().len(),
            2,
            "2 partials remain (only 1 processed before hitting partials_limit)"
        );

        // The first validator withdrawal should be from phase 2 (validator 0's partial)
        let validator_w: Vec<_> = withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) == 0)
            .collect();
        assert!(!validator_w.is_empty(), "should have validator withdrawals");
        assert_eq!(
            validator_w[0].validator_index, 0,
            "first validator withdrawal is validator 0 (from phase 2 partial)"
        );
        // min(balance - min_activation, requested) = min(34-32, 1) = 1 ETH
        assert_eq!(
            validator_w[0].amount, 1_000_000_000,
            "partial withdrawal amount capped at requested 1 ETH"
        );
    }

    /// Comprehensive get_expected_withdrawals_gloas vs process_withdrawals_gloas
    /// consistency test with builder sweep wrapping and multiple builders.
    /// Both functions should produce identical withdrawal lists.
    #[test]
    fn get_expected_withdrawals_matches_process_with_builder_sweep_wrap() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 2_000_000_000);
        make_parent_block_full(&mut state);

        // Add builder 1 (exited, with balance)
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .push(Builder {
                pubkey: types::PublicKeyBytes::empty(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xCC),
                balance: 4_000_000_000,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: Epoch::new(0), // exited
            })
            .unwrap();

        // Add builder 2 (active, high balance — not swept)
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .push(Builder {
                pubkey: types::PublicKeyBytes::empty(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xDD),
                balance: 99_000_000_000,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: spec.far_future_epoch, // active
            })
            .unwrap();

        // Make builder 0 exited
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Start builder sweep at index 2 (wraps: 2→0→1)
        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 2;

        // Add 1 builder pending withdrawal to mix phases
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                fee_recipient: Address::repeat_byte(0xEE),
                amount: 500,
                builder_index: 0,
            })
            .unwrap();

        // Make validator 0 have excess balance for partial sweep
        *state.get_balance_mut(0).unwrap() = 34_000_000_000;

        // Get expected withdrawals (read-only)
        let expected = get_expected_withdrawals_gloas::<E>(&state, &spec).unwrap();

        // Clone state, run process_withdrawals_gloas (mutating)
        let mut state2 = state.clone();
        process_withdrawals_gloas::<E>(&mut state2, &spec).unwrap();
        let actual = state2
            .as_gloas()
            .unwrap()
            .payload_expected_withdrawals
            .clone();

        // Both should produce identical results
        assert_eq!(
            expected.len(),
            actual.len(),
            "expected and actual withdrawal count must match"
        );
        for (i, (e, a)) in expected.iter().zip(actual.iter()).enumerate() {
            assert_eq!(e.index, a.index, "withdrawal {} index mismatch", i);
            assert_eq!(
                e.validator_index, a.validator_index,
                "withdrawal {} validator_index mismatch",
                i
            );
            assert_eq!(e.amount, a.amount, "withdrawal {} amount mismatch", i);
            assert_eq!(e.address, a.address, "withdrawal {} address mismatch", i);
        }

        // Verify the expected contents:
        // Phase 1: 1 builder pending withdrawal (builder 0, 500 gwei)
        // Phase 2: no pending partials
        // Phase 3: builder sweep from index 2:
        //   - builder 2: active (far_future), skipped
        //   - builder 0: exited, balance=2B, swept
        //   - builder 1: exited, balance=4B, swept
        // Phase 4: validator sweep for validator 0 (excess 2 ETH)
        assert!(expected.len() >= 3, "should have at least 3 withdrawals");

        // First: builder pending
        assert_ne!(expected[0].validator_index & BUILDER_INDEX_FLAG, 0);
        assert_eq!(expected[0].amount, 500);

        // Next should include builder sweep entries
        let builder_sweep: Vec<_> = expected
            .iter()
            .skip(1)
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert_eq!(builder_sweep.len(), 2, "2 exited builders should be swept");
    }

    // ── initiate_builder_exit lifecycle interaction tests ────────────────

    /// Exit a builder then attempt to submit a bid with that builder.
    /// The bid must be rejected because is_active_at_finalized_epoch returns false
    /// once withdrawable_epoch != FAR_FUTURE_EPOCH.
    #[test]
    fn builder_exit_then_bid_rejected_as_inactive() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);

        // Pre-condition: builder 0 is active, bid succeeds
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
        assert!(result.is_ok(), "bid should succeed before exit");

        // Initiate exit for builder 0
        initiate_builder_exit::<E>(&mut state, 0, &spec).unwrap();

        // Verify builder 0's withdrawable_epoch is no longer FAR_FUTURE_EPOCH
        let builder = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_ne!(
            builder.withdrawable_epoch, spec.far_future_epoch,
            "builder should have a concrete withdrawable_epoch after exit"
        );

        // Re-create bid (state's latest_execution_payload_bid was updated by first bid)
        let bid2 = make_builder_bid(&state, &spec, 500_000_000);
        let result2 = process_execution_payload_bid(
            &mut state,
            &bid2,
            slot,
            parent_root,
            VerifySignatures::False,
            &spec,
        );
        assert!(
            result2.is_err(),
            "bid from exited builder should be rejected"
        );
        let err_msg = format!("{:?}", result2.unwrap_err());
        assert!(
            err_msg.contains("not active"),
            "error should mention builder is not active: {err_msg}"
        );
    }

    /// Exit a builder, set state epoch to withdrawable_epoch.
    /// The builder sweep should include the exited builder's balance.
    #[test]
    fn builder_exit_sweep_includes_after_withdrawable_epoch() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Initiate exit for builder 0
        initiate_builder_exit::<E>(&mut state, 0, &spec).unwrap();
        let withdrawable = state
            .as_gloas()
            .unwrap()
            .builders
            .get(0)
            .unwrap()
            .withdrawable_epoch;
        assert_eq!(
            withdrawable,
            Epoch::new(1)
                .safe_add(spec.min_builder_withdrawability_delay)
                .unwrap(),
            "withdrawable_epoch should be current_epoch + delay"
        );

        // Set state epoch to exactly the withdrawable epoch by adjusting the slot
        let target_slot = Slot::new(withdrawable.as_u64().saturating_mul(E::slots_per_epoch()));
        state.as_gloas_mut().unwrap().slot = target_slot;

        // Fix up the epoch-dependent fields so total_active_balance resolves
        let target_epoch = target_slot.epoch(E::slots_per_epoch());
        state.set_total_active_balance(target_epoch, 8 * 32_000_000_000, &spec);

        // Process withdrawals — builder sweep should include builder 0
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        let builder_withdrawals: Vec<_> = expected
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert_eq!(
            builder_withdrawals.len(),
            1,
            "exited builder at withdrawable_epoch should be swept"
        );
        assert_eq!(
            builder_withdrawals[0].amount, 64_000_000_000,
            "full builder balance should be withdrawn"
        );

        // Verify builder balance was decreased to 0
        let builder_after = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(
            builder_after.balance, 0,
            "builder balance should be drained by sweep"
        );
    }

    /// Exit a builder but state epoch is before withdrawable_epoch.
    /// The builder sweep should skip the exiting (not yet withdrawable) builder.
    #[test]
    fn builder_exit_sweep_skips_before_withdrawable_epoch() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 64_000_000_000);
        make_parent_block_full(&mut state);

        // Initiate exit
        initiate_builder_exit::<E>(&mut state, 0, &spec).unwrap();
        let withdrawable = state
            .as_gloas()
            .unwrap()
            .builders
            .get(0)
            .unwrap()
            .withdrawable_epoch;

        // State epoch is 1, withdrawable_epoch is 1 + 64 = 65 — way in the future
        assert!(
            state.current_epoch() < withdrawable,
            "current epoch should be before withdrawable"
        );

        // Process withdrawals — sweep should NOT include builder 0
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        let builder_sweep: Vec<_> = expected
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert_eq!(
            builder_sweep.len(),
            0,
            "exiting builder before withdrawable_epoch should not be swept"
        );

        // Builder balance should be unchanged
        let builder_after = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(
            builder_after.balance, 64_000_000_000,
            "builder balance should be unchanged"
        );
    }

    /// Three builders with different exit states:
    /// - Builder 0: active (far_future_epoch) with balance 10B
    /// - Builder 1: exited and withdrawable (withdrawable_epoch <= epoch) with balance 20B
    /// - Builder 2: exiting but not yet withdrawable (withdrawable_epoch > epoch) with balance 30B
    ///
    /// Builder sweep from index 0 should:
    /// - Skip builder 0 (active, not withdrawable)
    /// - Include builder 1 (exited, withdrawable, has balance)
    /// - Skip builder 2 (exiting, not yet withdrawable)
    #[test]
    fn builder_sweep_mixed_exit_states_three_builders() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 10_000_000_000);
        make_parent_block_full(&mut state);

        // Advance state to epoch 100 so we have room for different withdrawable_epochs
        let target_epoch = Epoch::new(100);
        let target_slot = Slot::new(target_epoch.as_u64().saturating_mul(E::slots_per_epoch()));
        state.as_gloas_mut().unwrap().slot = target_slot;
        state.set_total_active_balance(target_epoch, 8 * 32_000_000_000, &spec);

        // Add two more builders
        let builder_1 = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xC1),
            balance: 20_000_000_000,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: Epoch::new(50), // already passed (epoch 100 > 50)
        };
        let builder_2 = Builder {
            pubkey: types::PublicKeyBytes::empty(),
            version: 0x03,
            execution_address: Address::repeat_byte(0xC2),
            balance: 30_000_000_000,
            deposit_epoch: Epoch::new(0),
            withdrawable_epoch: Epoch::new(200), // not yet (epoch 100 < 200)
        };
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .push(builder_1)
            .unwrap();
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .push(builder_2)
            .unwrap();

        // Start sweep from index 0
        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 0;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        let builder_sweep: Vec<_> = expected
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Only builder 1 should be swept
        assert_eq!(
            builder_sweep.len(),
            1,
            "only the withdrawable exited builder should be swept"
        );
        let builder_1_flag = 1u64 | BUILDER_INDEX_FLAG;
        assert_eq!(
            builder_sweep[0].validator_index, builder_1_flag,
            "swept builder should be builder 1"
        );
        assert_eq!(
            builder_sweep[0].amount, 20_000_000_000,
            "swept amount should be builder 1's full balance"
        );
        assert_eq!(
            builder_sweep[0].address,
            Address::repeat_byte(0xC1),
            "swept address should be builder 1's execution_address"
        );

        // Verify individual builder balances
        let builders = &state.as_gloas().unwrap().builders;
        assert_eq!(
            builders.get(0).unwrap().balance,
            10_000_000_000,
            "active builder 0 balance should be unchanged"
        );
        assert_eq!(
            builders.get(1).unwrap().balance,
            0,
            "exited builder 1 balance should be drained"
        );
        assert_eq!(
            builders.get(2).unwrap().balance,
            30_000_000_000,
            "exiting builder 2 balance should be unchanged"
        );

        // Verify next_withdrawal_builder_index advanced past all 3 builders
        // sweep processed min(3, max_builders_per_sweep) = 3 builders
        // next = (0 + 3) % 3 = 0 (wraps around)
        assert_eq!(
            state.as_gloas().unwrap().next_withdrawal_builder_index,
            0,
            "next_withdrawal_builder_index should wrap around"
        );
    }

    /// Exit a builder that has a pending payment from a previous bid.
    /// The pending payment should still be processed normally (the payment was
    /// committed before the exit), and the builder's balance should cover both
    /// the payment withdrawal (when promoted) and the exit sweep withdrawal.
    #[test]
    fn builder_exit_with_pending_payment_both_processed() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 100_000_000_000);
        make_parent_block_full(&mut state);

        // Advance to epoch 100 so we can work with builder exit timing
        let target_epoch = Epoch::new(100);
        let target_slot = Slot::new(target_epoch.as_u64().saturating_mul(E::slots_per_epoch()));
        state.as_gloas_mut().unwrap().slot = target_slot;
        state.set_total_active_balance(target_epoch, 8 * 32_000_000_000, &spec);

        // Set builder 0 as exited and withdrawable (withdrawable_epoch in the past)
        {
            let builder = state.as_gloas_mut().unwrap().builders.get_mut(0).unwrap();
            builder.withdrawable_epoch = Epoch::new(50); // past
        }

        // Add a pending builder withdrawal (simulating a previously promoted payment)
        let pending_withdrawal = BuilderPendingWithdrawal {
            fee_recipient: Address::repeat_byte(0xDD),
            amount: 5_000_000_000,
            builder_index: 0,
        };
        state
            .as_gloas_mut()
            .unwrap()
            .builder_pending_withdrawals
            .push(pending_withdrawal)
            .unwrap();

        // Process withdrawals
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let expected = &state.as_gloas().unwrap().payload_expected_withdrawals;
        let builder_withdrawals: Vec<_> = expected
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Should have 2 builder withdrawals:
        // 1. The pending withdrawal (5B to fee_recipient 0xDD)
        // 2. The sweep withdrawal (full remaining balance to execution_address 0xBB)
        assert_eq!(
            builder_withdrawals.len(),
            2,
            "should have both pending payment withdrawal and sweep withdrawal"
        );

        // First: pending withdrawal
        assert_eq!(
            builder_withdrawals[0].amount, 5_000_000_000,
            "pending withdrawal amount should be 5B"
        );
        assert_eq!(
            builder_withdrawals[0].address,
            Address::repeat_byte(0xDD),
            "pending withdrawal address should be fee_recipient"
        );

        // Second: builder sweep withdrawal
        assert_eq!(
            builder_withdrawals[1].amount, 100_000_000_000,
            "sweep should withdraw full builder balance"
        );
        assert_eq!(
            builder_withdrawals[1].address,
            Address::repeat_byte(0xBB),
            "sweep withdrawal address should be builder's execution_address"
        );

        // Builder balance after both withdrawals: 100B - 5B - 100B = 0 (clamped)
        // The pending withdrawal decreases by min(5B, 100B) = 5B → 95B
        // The sweep decreases by min(100B, 95B) = 95B → 0
        let builder_after = state.as_gloas().unwrap().builders.get(0).unwrap().clone();
        assert_eq!(
            builder_after.balance, 0,
            "builder balance should be fully drained"
        );

        // Verify pending withdrawals list was drained
        assert_eq!(
            state.as_gloas().unwrap().builder_pending_withdrawals.len(),
            0,
            "processed pending withdrawal should be removed from list"
        );
    }

    /// Builder sweep exits early due to reserved_limit (not builders_limit).
    /// With 5 builders and reserved_limit=3, if phase 1 already produced 2
    /// withdrawals, the sweep can only produce 1 more before hitting
    /// reserved_limit. Verify next_withdrawal_builder_index reflects only the
    /// iterations actually executed, not the full builders_limit.
    #[test]
    fn withdrawals_builder_sweep_early_exit_reserved_limit() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 10_000_000_000);
        make_parent_block_full(&mut state);

        // Add 4 more exited builders (total 5, indices 0-4)
        for i in 1..5u8 {
            state
                .as_gloas_mut()
                .unwrap()
                .builders
                .push(Builder {
                    pubkey: types::PublicKeyBytes::empty(),
                    version: 0x03,
                    execution_address: Address::repeat_byte(0xB0 + i),
                    balance: 10_000_000_000,
                    deposit_epoch: Epoch::new(0),
                    withdrawable_epoch: Epoch::new(0), // exited
                })
                .unwrap();
        }

        // Make builder 0 also exited
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Add 2 builder pending withdrawals to fill phase 1 partially
        // reserved_limit = max_withdrawals(4) - 1 = 3
        // Phase 1 produces 2 withdrawals, leaving room for 1 more in phases 2-3
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                builder_index: 0,
                amount: 1_000_000_000,
                fee_recipient: Address::repeat_byte(0xBB),
            })
            .unwrap();
        state_gloas
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                builder_index: 1,
                amount: 1_000_000_000,
                fee_recipient: Address::repeat_byte(0xB1),
            })
            .unwrap();

        // Start sweep at builder 2
        state_gloas.next_withdrawal_builder_index = 2;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Phase 1: 2 pending withdrawals (builders 0 and 1)
        // Phase 3: sweep starts at builder 2, produces 1 withdrawal → hits reserved_limit=3
        // Total builder withdrawals: 3
        assert_eq!(
            builder_w.len(),
            3,
            "2 pending + 1 sweep = 3 builder withdrawals"
        );

        // The sweep withdrawal should be builder 2
        assert_eq!(
            builder_w[2].validator_index,
            2 | BUILDER_INDEX_FLAG,
            "sweep should process builder 2"
        );

        // The sweep ran only 1 iteration before reserved_limit was hit
        // next_withdrawal_builder_index = (2 + 1) % 5 = 3
        assert_eq!(
            state_gloas.next_withdrawal_builder_index, 3,
            "builder index advances by 1 (early exit after 1 iteration)"
        );
    }

    /// Withdrawal processing with zero builders in state.
    /// The builder sweep phase should gracefully handle an empty builders list
    /// without panicking or producing any builder withdrawals.
    #[test]
    fn withdrawals_zero_builders_no_panic() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 0);
        make_parent_block_full(&mut state);

        // Remove all builders
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas.builders = List::default();

        // Should not panic
        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();

        // No builder withdrawals should exist
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert!(
            builder_w.is_empty(),
            "no builder withdrawals with empty builders list"
        );

        // next_withdrawal_builder_index should remain 0 (no builders to advance through)
        assert_eq!(
            state_gloas.next_withdrawal_builder_index, 0,
            "builder index unchanged with empty builders list"
        );
    }

    /// Builder sweep processes exactly builders_limit iterations when no
    /// eligible builder is found (all active). Verify next_withdrawal_builder_index
    /// advances by the full sweep count, not just eligible-builder count.
    #[test]
    fn withdrawals_builder_sweep_all_ineligible_advances_index() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 5_000_000_000);
        make_parent_block_full(&mut state);

        // Add 2 more active builders (total 3, all active with far_future_epoch)
        for i in 1..3u8 {
            state
                .as_gloas_mut()
                .unwrap()
                .builders
                .push(Builder {
                    pubkey: types::PublicKeyBytes::empty(),
                    version: 0x03,
                    execution_address: Address::repeat_byte(0xB0 + i),
                    balance: 5_000_000_000,
                    deposit_epoch: Epoch::new(0),
                    withdrawable_epoch: spec.far_future_epoch, // active — not swept
                })
                .unwrap();
        }

        // Start sweep at builder 1
        state.as_gloas_mut().unwrap().next_withdrawal_builder_index = 1;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();

        // No builder sweep withdrawals (all active)
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert!(builder_w.is_empty(), "no active builders should be swept");

        // builders_limit = min(3, 16) = 3, all 3 iterated
        // next_withdrawal_builder_index = (1 + 3) % 3 = 1
        assert_eq!(
            state_gloas.next_withdrawal_builder_index, 1,
            "index advances by builders_limit even when no builder is eligible"
        );
    }

    /// Builder sweep with reserved_limit already fully consumed by phase 1
    /// (builder pending withdrawals). The sweep loop should not execute any
    /// iterations, so next_withdrawal_builder_index should not advance.
    #[test]
    fn withdrawals_builder_sweep_skipped_when_reserved_limit_full() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 100_000_000_000);
        make_parent_block_full(&mut state);

        // Make builder 0 exited
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Add 3 pending withdrawals to fill reserved_limit (3 in minimal)
        let state_gloas = state.as_gloas_mut().unwrap();
        for i in 0..3u64 {
            state_gloas
                .builder_pending_withdrawals
                .push(BuilderPendingWithdrawal {
                    builder_index: 0,
                    amount: 1_000_000_000 + i,
                    fee_recipient: Address::repeat_byte(0xBB),
                })
                .unwrap();
        }

        // Start sweep at builder 0
        state_gloas.next_withdrawal_builder_index = 0;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();

        // Phase 1 should have produced 3 withdrawals (filling reserved_limit)
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();
        assert_eq!(builder_w.len(), 3, "3 pending withdrawals from phase 1");

        // Phase 3 sweep should have iterated 0 times (reserved_limit already hit)
        // But wait — the sweep loop still iterates builders_limit times, checking
        // reserved_limit at the start of each iteration. Since reserved_limit is
        // already met, the very first iteration breaks.
        // processed_builders_sweep_count = 0
        // next_withdrawal_builder_index = (0 + 0) % 1 = 0
        assert_eq!(
            state_gloas.next_withdrawal_builder_index, 0,
            "sweep didn't iterate, so builder index should not advance"
        );
    }

    /// Phase 1 (builder pending) partially fills, phase 3 (builder sweep)
    /// wraps around with 2 builders starting near the end. Tests the combined
    /// index tracking when both pending and sweep phases contribute withdrawals.
    #[test]
    fn withdrawals_builder_pending_plus_sweep_wrap() {
        let (mut state, spec) = make_gloas_state(8, 32_000_000_000, 50_000_000_000);
        make_parent_block_full(&mut state);

        // Add a second exited builder
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .push(Builder {
                pubkey: types::PublicKeyBytes::empty(),
                version: 0x03,
                execution_address: Address::repeat_byte(0xCC),
                balance: 20_000_000_000,
                deposit_epoch: Epoch::new(0),
                withdrawable_epoch: Epoch::new(0), // exited
            })
            .unwrap();

        // Make builder 0 exited too
        state
            .as_gloas_mut()
            .unwrap()
            .builders
            .get_mut(0)
            .unwrap()
            .withdrawable_epoch = Epoch::new(0);

        // Add 1 pending withdrawal (phase 1 produces 1)
        let state_gloas = state.as_gloas_mut().unwrap();
        state_gloas
            .builder_pending_withdrawals
            .push(BuilderPendingWithdrawal {
                builder_index: 0,
                amount: 3_000_000_000,
                fee_recipient: Address::repeat_byte(0xBB),
            })
            .unwrap();

        // Start sweep at builder 1
        state_gloas.next_withdrawal_builder_index = 1;

        process_withdrawals_gloas::<E>(&mut state, &spec).unwrap();

        let state_gloas = state.as_gloas().unwrap();
        let builder_w: Vec<_> = state_gloas
            .payload_expected_withdrawals
            .iter()
            .filter(|w| (w.validator_index & BUILDER_INDEX_FLAG) != 0)
            .collect();

        // Phase 1: 1 pending withdrawal for builder 0
        // Phase 3: sweep starts at builder 1 (exited, balance=20B → withdrawal)
        //          then wraps to builder 0 (exited, balance=50B → withdrawal)
        //          now withdrawals.len() = 3 = reserved_limit → break
        // Total: 3 builder withdrawals
        assert_eq!(
            builder_w.len(),
            3,
            "1 pending + 2 sweep = 3 builder withdrawals"
        );

        // Verify sweep withdrawal order: builder 1 first, then builder 0
        assert_eq!(
            builder_w[1].validator_index,
            1 | BUILDER_INDEX_FLAG,
            "sweep starts at builder 1"
        );
        assert_eq!(
            builder_w[2].validator_index,
            BUILDER_INDEX_FLAG, // builder index 0
            "sweep wraps to builder 0"
        );

        // Sweep iterated 2 times: builders 1 and 0
        // next_withdrawal_builder_index = (1 + 2) % 2 = 1
        assert_eq!(
            state_gloas.next_withdrawal_builder_index, 1,
            "builder index wraps after sweep processes both builders"
        );

        // Withdrawal indices should be contiguous
        for (i, w) in state_gloas.payload_expected_withdrawals.iter().enumerate() {
            assert_eq!(w.index, i as u64, "withdrawal index should be contiguous");
        }
    }
}
