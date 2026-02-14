use crate::per_block_processing::errors::{BlockProcessingError, PayloadAttestationInvalid};
use crate::VerifySignatures;
use std::borrow::Cow;
use swap_or_not_shuffle::compute_shuffled_index;
use tree_hash::TreeHash;
use types::consts::gloas::{PTC_SIZE, BUILDER_INDEX_SELF_BUILD};
use types::{
    AggregateSignature, BeaconState, BuilderPendingPayment, BuilderPendingWithdrawal, ChainSpec,
    Domain, EthSpec, IndexedPayloadAttestation, PayloadAttestation, PayloadAttestationData,
    PublicKey, SignedExecutionPayloadBid, SigningData, Slot, Unsigned,
};

/// Processes an execution payload bid in Gloas ePBS.
///
/// This validates the builder's bid and updates the state with the chosen bid.
/// The proposer may choose the highest valid bid or self-build (value = 0).
///
/// Reference: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md#modified-process_block
pub fn process_execution_payload_bid<E: EthSpec>(
    state: &mut BeaconState<E>,
    signed_bid: &SignedExecutionPayloadBid<E>,
    verify_signatures: VerifySignatures,
    spec: &ChainSpec,
) -> Result<(), BlockProcessingError> {
    let bid = &signed_bid.message;

    // Verify slot matches current slot
    if bid.slot != state.slot() {
        return Err(BlockProcessingError::PayloadBidInvalid {
            reason: format!(
                "bid slot {} does not match state slot {}",
                bid.slot,
                state.slot()
            ),
        });
    }

    // Verify parent block root matches
    if bid.parent_block_root != state.latest_block_header().parent_root {
        return Err(BlockProcessingError::PayloadBidInvalid {
            reason: "bid parent_block_root does not match state".into(),
        });
    }

    // Handle self-build case (builder_index == BUILDER_INDEX_SELF_BUILD)
    if bid.builder_index == spec.builder_index_self_build {
        // For self-builds:
        // - value must be 0
        // - signature must be infinity (empty)
        if bid.value != 0 {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: "self-build bid must have value = 0".into(),
            });
        }
        if !signed_bid.signature.is_empty() {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: "self-build bid must have empty signature".into(),
            });
        }
    } else {
        // External builder bid
        let builder_index = bid.builder_index as usize;

        // Verify builder exists and is active
        let state_gloas = state.as_gloas_mut().map_err(|_| {
            BlockProcessingError::PayloadBidInvalid {
                reason: "state is not Gloas".into(),
            }
        })?;

        let builder = state_gloas
            .builders
            .get(builder_index)
            .ok_or_else(|| BlockProcessingError::PayloadBidInvalid {
                reason: format!("builder index {} does not exist", builder_index),
            })?;

        // Check builder is active (registered before finalized_epoch and not withdrawn)
        if !builder.is_active_at_finalized_epoch(state.finalized_checkpoint().epoch, spec) {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: format!("builder {} is not active", builder_index),
            });
        }

        // Check builder has sufficient balance for the bid
        if builder.balance < bid.value {
            return Err(BlockProcessingError::PayloadBidInvalid {
                reason: format!(
                    "builder balance {} insufficient for bid value {}",
                    builder.balance, bid.value
                ),
            });
        }

        // Verify signature if requested
        if verify_signatures.is_true() {
            // Verify builder bid signature
            let domain = spec.get_domain(
                bid.slot.epoch(E::slots_per_epoch()),
                Domain::BeaconBuilder,
                &state.fork(),
                state.genesis_validators_root(),
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

    // Update state with the chosen bid
    let state_gloas = state.as_gloas_mut().map_err(|_| {
        BlockProcessingError::PayloadBidInvalid {
            reason: "state is not Gloas".into(),
        }
    })?;
    state_gloas.latest_execution_payload_bid = bid.clone();

    // If this is an external builder bid, set up pending payment
    if bid.builder_index != spec.builder_index_self_build {
        let slot_index = (bid.slot.as_u64() % E::BuilderPendingPaymentsLimit::to_u64()) as usize;
        
        // Record the pending payment with zero initial weight
        // Weight will be accumulated as PTC members attest
        let pending_payment = BuilderPendingPayment {
            weight: 0,
            withdrawal: BuilderPendingWithdrawal {
                fee_recipient: bid.fee_recipient,
                amount: bid.value,
                        builder_index: bid.builder_index,
            },
        };
        
        *state_gloas.builder_pending_payments.get_mut(slot_index).ok_or(BlockProcessingError::InvalidSlot(slot_index as u64))? = pending_payment;
    }

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

    // Check if we've reached quorum (60% of PTC = 6/10 * 512 = 307 attesters)
    let num_attesters = attestation.num_attesters();
    let quorum_threshold = (PTC_SIZE * spec.builder_payment_threshold_numerator)
        / spec.builder_payment_threshold_denominator;

    if num_attesters >= quorum_threshold as usize {
        // Quorum reached! Mark payload as available
        let state_gloas = state.as_gloas_mut().map_err(|_| {
            BlockProcessingError::PayloadAttestationInvalid(
                PayloadAttestationInvalid::IncorrectStateVariant,
            )
        })?;

        let slot_index = data.slot.as_usize() % E::SlotsPerHistoricalRoot::to_usize();
        state_gloas
            .execution_payload_availability
            .set(slot_index, data.payload_present)
            .map_err(|_| {
                BlockProcessingError::PayloadAttestationInvalid(
                    PayloadAttestationInvalid::SlotOutOfBounds,
                )
            })?;

                // If payload was revealed, process builder payment
        if data.payload_present {
            let payment_slot_index =
                (data.slot.as_u64() % E::BuilderPendingPaymentsLimit::to_u64()) as usize;
            let pending_payment = state_gloas.builder_pending_payments.get_mut(payment_slot_index)
                .ok_or(BlockProcessingError::InvalidSlotIndex(payment_slot_index))?;

            // Transfer payment from builder to proposer if not already processed
            if pending_payment.weight < quorum_threshold {
                pending_payment.weight = quorum_threshold; // Mark as processed

                let builder_index = pending_payment.withdrawal.builder_index as usize;
                let payment_amount = pending_payment.withdrawal.amount;

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
                // For now, this is a known TODO that will be addressed when integrating
                // with the full block processing pipeline
            }
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

    let mut ptc_committee = Vec::with_capacity(PTC_SIZE);
        let mut i = 0;

    // Select PTC_SIZE validators using shuffled indices
    while ptc_committee.len() < PTC_SIZE && i < active_validator_count * 10 {
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
        ptc_committee.push(candidate_index);

        i += 1;
    }

    if ptc_committee.len() < PTC_SIZE {
        // Not enough validators to form a full PTC (edge case for testnets)
        return Err(BlockProcessingError::PayloadAttestationInvalid(
            PayloadAttestationInvalid::InsufficientValidators,
        ));
    }

    Ok(ptc_committee)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::*;
    use types::test_utils::{TestRandom, XorShiftRng};

    fn get_gloas_state<E: EthSpec>(validator_count: usize) -> BeaconState<E> {
        let spec = E::default_spec();
        let mut state = BeaconState::new(0, Hash256::zero(), &spec);
        
        // Add validators
        let mut rng = XorShiftRng::from_seed([42; 16]);
        for _ in 0..validator_count {
            let validator = Validator::random_for_test(&mut rng);
            state.validators_mut().push(validator).unwrap();
            state.balances_mut().push(32_000_000_000).unwrap(); // 32 ETH in Gwei
        }
        
        // Upgrade to Gloas
        let epoch = Epoch::new(0);
        let mut state_gloas = match state {
            BeaconState::Base(mut base) => {
                // This is a hack for testing - in reality we'd go through proper upgrade
                // For now just create a minimal Gloas state
                todo!("Need proper test state builder for Gloas")
            }
            _ => unreachable!(),
        };
        
        state_gloas
    }

    #[test]
    fn test_process_execution_payload_bid_self_build() {
        // TODO: Test that self-build bids are accepted with value=0 and empty signature
    }

    #[test]
    fn test_process_execution_payload_bid_external_builder() {
        // TODO: Test that external builder bids are validated correctly
    }

    #[test]
    fn test_process_execution_payload_bid_insufficient_balance() {
        // TODO: Test rejection when builder balance < bid value
    }

    #[test]
    fn test_process_execution_payload_bid_inactive_builder() {
        // TODO: Test rejection when builder is not active
    }

    #[test]
    fn test_process_execution_payload_bid_wrong_slot() {
        // TODO: Test rejection when bid slot != state slot
    }

    #[test]
    fn test_process_payload_attestation_quorum_reached() {
        // TODO: Test that quorum triggers payload availability update
    }

    #[test]
    fn test_process_payload_attestation_quorum_not_reached() {
        // TODO: Test that sub-quorum attestations are accepted but don't trigger payment
    }

    #[test]
    fn test_process_payload_attestation_wrong_slot() {
        // TODO: Test rejection when attestation slot != state slot
    }

    #[test]
    fn test_get_ptc_committee_deterministic() {
        // TODO: Test that PTC committee is deterministic for a given slot/state
    }

    #[test]
    fn test_get_ptc_committee_size() {
        // TODO: Test that PTC committee has exactly 512 members (when enough validators)
    }

    #[test]
    fn test_get_indexed_payload_attestation() {
        // TODO: Test conversion from PayloadAttestation to IndexedPayloadAttestation
    }

    #[test]
    fn test_indexed_payload_attestation_sorted() {
        // TODO: Test that indices are sorted after conversion
    }
}
