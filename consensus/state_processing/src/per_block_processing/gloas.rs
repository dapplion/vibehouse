use crate::per_block_processing::errors::{BlockProcessingError, PayloadAttestationInvalid};
use crate::VerifySignatures;
use types::consts::gloas::{PTC_SIZE, BUILDER_INDEX_SELF_BUILD};
use types::{
    BeaconState, ChainSpec, EthSpec, IndexedPayloadAttestation, PayloadAttestation,
    PayloadAttestationData, SignedExecutionPayloadBid, Slot,
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
            // TODO: implement signature verification
            // Need to:
            // 1. Get signing root of ExecutionPayloadBid
            // 2. Get domain for DOMAIN_BEACON_BUILDER
            // 3. Verify signature against builder's pubkey
            // For now, we'll add a placeholder
            todo!("Signature verification for builder bids not yet implemented");
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
        let slot_index = (bid.slot % E::SlotsPerEpoch::to_u64()) as usize;
        // TODO: Add builder pending payment to builder_pending_payments[slot_index]
        // This tracks that the builder should be paid when payload is revealed
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
        // TODO: Implement signature verification
        // Need to verify aggregate signature from all PTC members
        todo!("Signature verification for payload attestations not yet implemented");
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

        // If payload was revealed, trigger builder payment
        if data.payload_present {
            // TODO: Process builder payment
            // Transfer bid value from builder balance to proposer balance
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
    // TODO: Implement PTC committee calculation
    // For now, return a placeholder
    todo!("PTC committee calculation not yet implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::*;

    // TODO: Add unit tests
    // - test_process_execution_payload_bid_self_build
    // - test_process_execution_payload_bid_external_builder
    // - test_process_execution_payload_bid_insufficient_balance
    // - test_process_execution_payload_bid_inactive_builder
    // - test_process_payload_attestation_quorum_reached
    // - test_process_payload_attestation_quorum_not_reached
    // - test_process_payload_attestation_wrong_slot
}
